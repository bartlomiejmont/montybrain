//! Native/system integration: settings, the global quick-capture shortcut and
//! window, main-window show/hide, autostart. The tray lives in [`crate::tray`],
//! reminders in [`crate::reminders`].

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Mutex;

use montybrain_core::TaskLocation;
use serde::Serialize;
use tauri::{
    AppHandle, Emitter, LogicalPosition, Manager, PhysicalSize, Runtime, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

use crate::reminders;
use crate::settings::{self, Settings, SettingsPatch};

pub const MAIN: &str = "main";
pub const QUICK: &str = "quick";
/// Emitted with a [`SettingsView`] whenever settings or their status change.
pub const SETTINGS_CHANGED: &str = "settings-changed";
/// Emitted when the quick-capture window is shown (it resets + refocuses).
pub const QUICK_SHOW: &str = "quick-show";
/// Command-line flag used by the login item: start hidden in the menu bar.
pub const HIDDEN_ARG: &str = "--hidden";

const QUICK_WIDTH: f64 = 580.0;
const QUICK_MIN_HEIGHT: f64 = 60.0;
const QUICK_MAX_HEIGHT: f64 = 460.0;

#[derive(Debug, Default)]
struct ShortcutStatus {
    /// The registered shortcut (canonical string), if any.
    active: Option<String>,
    /// Why the saved shortcut is not registered.
    error: Option<String>,
    /// Temporarily unregistered while Settings records a new one.
    paused: bool,
    /// OS registrations that are not the active shortcut and that we failed to
    /// remove. They still trigger quick capture, so they stay tracked, are
    /// retried on the next transition and on quit, and are reported.
    strays: BTreeSet<String>,
}

impl ShortcutStatus {
    /// What Settings shows: the registration error, else a pending cleanup.
    fn view_error(&self) -> Option<String> {
        self.error.clone().or_else(|| {
            (!self.strays.is_empty()).then(|| {
                let list: Vec<_> = self
                    .strays
                    .iter()
                    .map(|c| crate::shortcut::pretty(c))
                    .collect();
                format!(
                    "Nie udało się zwolnić skrótu {}; próba zostanie ponowiona.",
                    list.join(", ")
                )
            })
        })
    }
}

/// Managed state of the native layer.
pub struct Native {
    config_dir: PathBuf,
    settings: Mutex<Settings>,
    /// Serializes settings writes (clone → save → publish).
    settings_op: Mutex<()>,
    shortcut: ShortcutCtl,
    /// Where focus goes when the quick window closes.
    return_to: Mutex<ReturnTo>,
}

impl Native {
    fn settings_path(&self) -> PathBuf {
        self.config_dir.join(settings::SETTINGS_FILE)
    }

    pub fn settings(&self) -> Settings {
        crate::lock(&self.settings).clone()
    }
}

/// What the Settings view shows (`get_settings` and friends).
#[derive(Debug, Clone, Serialize)]
pub struct SettingsView {
    pub shortcut: String,
    /// Set when the saved shortcut could not be registered.
    pub shortcut_error: Option<String>,
    pub reminders: bool,
    pub digest_hour: u8,
    pub autostart: bool,
    pub autostart_error: Option<String>,
    pub quick_destination: TaskLocation,
    /// Automatic project update per project slug (only entries that differ
    /// from the default "off").
    pub auto_update: std::collections::BTreeMap<String, crate::updates::schedule::AutoUpdate>,
    /// Update sources per project slug (entries that differ from the default).
    pub update_sources: std::collections::BTreeMap<String, crate::updates::schedule::UpdateSources>,
    /// Model and effort per agent.
    pub agents: crate::runs::agent::Agents,
    /// Default days between checks of a waiting item.
    pub waiting_days: u8,
    /// Model names to suggest per agent (Codex: from its own model catalog).
    pub agent_suggestions: AgentSuggestions,
    pub vault_path: String,
    pub config_path: String,
    pub version: String,
}

pub fn settings_view<R: Runtime>(app: &AppHandle<R>) -> SettingsView {
    let native = app.state::<Native>();
    let s = native.settings();
    let sc = crate::lock(&native.shortcut.status);
    let (autostart, autostart_error) = match app.autolaunch().is_enabled() {
        Ok(on) => (on, None),
        Err(e) => (false, Some(e.to_string())),
    };
    SettingsView {
        shortcut: s.shortcut,
        shortcut_error: sc.view_error(),
        reminders: s.reminders,
        digest_hour: s.digest_hour,
        autostart,
        autostart_error,
        quick_destination: s.quick_destination,
        auto_update: s.auto_update,
        update_sources: s.update_sources,
        agents: s.agents,
        waiting_days: s.waiting_days,
        agent_suggestions: agent_suggestions(app),
        vault_path: app
            .state::<crate::state::VaultState>()
            .vault()
            .root()
            .display()
            .to_string(),
        config_path: native.config_dir.display().to_string(),
        version: app.package_info().version.to_string(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentSuggestions {
    pub claude: Vec<String>,
    pub codex: Vec<String>,
}

/// Claude Code aliases from `claude --help`; Codex models listed in the
/// CLI's own catalog (`~/.codex/models_cache.json`, visible ones), best effort.
fn agent_suggestions<R: Runtime>(app: &AppHandle<R>) -> AgentSuggestions {
    let home = app.path().home_dir().ok();
    let codex = home
        .map(|h| codex_models(&h.join(".codex/models_cache.json")))
        .unwrap_or_default();
    AgentSuggestions {
        claude: ["opus", "sonnet", "fable"].map(String::from).to_vec(),
        codex,
    }
}

/// The visible model slugs of Codex's model catalog (only names that pass
/// the model-name check).
pub fn codex_models(path: &std::path::Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    v.get("models")
        .and_then(|m| m.as_array())
        .map(|ms| {
            ms.iter()
                .filter(|m| m.get("visibility").and_then(|x| x.as_str()) != Some("hide"))
                .filter_map(|m| m.get("slug").and_then(|x| x.as_str()))
                .filter_map(|s| {
                    crate::runs::agent::check_model(crate::runs::ProviderId::Codex, s)
                        .ok()
                        .flatten()
                })
                .take(12)
                .collect()
        })
        .unwrap_or_default()
}

/// Current reminder settings, for the scheduler.
pub fn reminder_config<R: Runtime>(app: &AppHandle<R>) -> reminders::Config {
    let s = app.state::<Native>().settings();
    reminders::Config {
        enabled: s.reminders,
        digest_hour: s.digest_hour,
    }
}

/// The registered shortcut (for the tray menu), if any.
pub fn active_shortcut<R: Runtime>(app: &AppHandle<R>) -> Option<String> {
    let native = app.try_state::<Native>()?;
    crate::lock(&native.shortcut.status).active.clone()
}

// --- setup ------------------------------------------------------------------

/// Called once from `setup`, after the vault state is managed.
pub fn init<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let config_dir = match settings::config_dir_override() {
        Some(dir) => dir,
        None => {
            let dir = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("montybrain-config"));
            // First start after the rename: copy the old app's state once.
            // Only for the platform dir; an env override is never migrated.
            if let Some(old) = settings::legacy_config_dir(&dir) {
                match settings::migrate_config(&dir, &old) {
                    Ok(copied) if !copied.is_empty() => crate::log(&format!(
                        "config: copied {} from {}",
                        copied.join(", "),
                        old.display()
                    )),
                    Ok(_) => {}
                    Err(e) => crate::log(&format!(
                        "config: migration from {} failed: {e}",
                        old.display()
                    )),
                }
            }
            dir
        }
    };
    crate::log(&format!("config: {}", config_dir.display()));
    let s = Settings::load(&config_dir.join(settings::SETTINGS_FILE));
    let shortcut = s.shortcut.clone();
    app.manage(Native {
        config_dir: config_dir.clone(),
        settings: Mutex::new(s),
        settings_op: Mutex::new(()),
        shortcut: ShortcutCtl::default(),
        return_to: Mutex::new(ReturnTo::Nothing),
    });

    // A taken shortcut must not stop the app: record the error for Settings.
    let status = match register(app, &shortcut) {
        Ok(canonical) => ShortcutStatus {
            active: Some(canonical),
            ..Default::default()
        },
        Err(e) => {
            crate::log(&format!("shortcut {shortcut}: {e}"));
            ShortcutStatus {
                error: Some(e),
                ..Default::default()
            }
        }
    };
    *crate::lock(&app.state::<Native>().shortcut.status) = status;

    if let Err(e) = create_quick_window(app) {
        crate::log(&format!("quick window failed: {e}"));
    }
    if let Err(e) = crate::tray::create(app) {
        crate::log(&format!("tray failed: {e}"));
    }
    carry_over_autostart(app);
    crate::waiting::start(app, &config_dir);
    reminders::start(app, config_dir.clone());
    crate::updates::start(app, &config_dir);
    crate::drift::start(app, &config_dir);

    let hidden = std::env::args().any(|a| a == HIDDEN_ARG);
    if hidden {
        crate::log("started hidden (login item)");
    } else {
        show_main(app);
    }
    Ok(())
}

/// Global shortcut handler (plugin-wide).
pub fn on_shortcut<R: Runtime>(app: &AppHandle<R>, _shortcut: &Shortcut, event: ShortcutEvent) {
    if event.state() == ShortcutState::Pressed {
        toggle_quick(app);
    }
}

/// Window events for all windows (`Builder::on_window_event`).
pub fn on_window_event<R: Runtime>(window: &tauri::Window<R>, event: &WindowEvent) {
    match (window.label(), event) {
        // Closing the main window only hides it: the app keeps running in the
        // menu bar so reminders and the shortcut keep working.
        (MAIN, WindowEvent::CloseRequested { api, .. }) => {
            api.prevent_close();
            let _ = window.hide();
        }
        (QUICK, WindowEvent::CloseRequested { api, .. }) => {
            api.prevent_close();
            let _ = window.hide();
        }
        // Clicking anywhere else dismisses the quick window.
        (QUICK, WindowEvent::Focused(false)) if window.is_visible().unwrap_or(false) => {
            let _ = window.hide();
        }
        _ => {}
    }
}

/// After every snapshot change: tray count + reminder check.
pub fn vault_changed<R: Runtime>(app: &AppHandle<R>) {
    crate::tray::refresh(app);
    if let Some(s) = app.try_state::<reminders::Scheduler>() {
        s.poke();
    }
}

// --- main window ------------------------------------------------------------

pub fn show_main<R: Runtime>(app: &AppHandle<R>) {
    #[cfg(target_os = "macos")]
    let _ = app.show();
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// Switches the main window to a view (`#/p/slug?temat=x`).
pub fn navigate_main<R: Runtime>(app: &AppHandle<R>, hash: &str) {
    let Some(w) = app.get_webview_window(MAIN) else {
        return;
    };
    let js = format!(
        "window.location.hash = {};",
        serde_json::to_string(hash).unwrap_or_else(|_| "\"#/dzis\"".into())
    );
    if let Err(e) = w.eval(js) {
        crate::log(&format!("navigate failed: {e}"));
    }
}

/// Hash route of the view that shows tasks at `loc` (mirrors `links` in
/// src/router/router.tsx).
pub fn location_hash(loc: &TaskLocation) -> String {
    match loc {
        TaskLocation::Inbox => "#/skrzynka".into(),
        TaskLocation::Project { project } => format!("#/p/{}", encode(project)),
        TaskLocation::Topic { project, topic } => {
            format!("#/p/{}?temat={}", encode(project), encode(topic))
        }
    }
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

// --- quick-capture window ---------------------------------------------------

fn create_quick_window<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<WebviewWindow<R>> {
    let builder = WebviewWindowBuilder::new(app, QUICK, WebviewUrl::App("quick.html".into()))
        .title("Szybkie zadanie")
        .inner_size(QUICK_WIDTH, QUICK_MIN_HEIGHT)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible_on_all_workspaces(true)
        .shadow(true)
        .visible(false)
        .focused(false);
    #[cfg(target_os = "macos")]
    let builder = builder.transparent(true);
    builder.build()
}

fn quick_window<R: Runtime>(app: &AppHandle<R>) -> Option<WebviewWindow<R>> {
    app.get_webview_window(QUICK).or_else(|| {
        create_quick_window(app)
            .map_err(|e| crate::log(&format!("quick window failed: {e}")))
            .ok()
    })
}

pub fn toggle_quick<R: Runtime>(app: &AppHandle<R>) {
    let open = app
        .get_webview_window(QUICK)
        .is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false));
    if open {
        hide_quick(app);
    } else {
        show_quick(app);
    }
}

pub fn show_quick<R: Runtime>(app: &AppHandle<R>) {
    let Some(w) = quick_window(app) else {
        return;
    };
    let main_focused = app
        .get_webview_window(MAIN)
        .is_some_and(|m| m.is_visible().unwrap_or(false) && m.is_focused().unwrap_or(false));
    let return_to = if main_focused {
        ReturnTo::Main
    } else {
        previous_app().map_or(ReturnTo::Nothing, ReturnTo::App)
    };
    *crate::lock(&app.state::<Native>().return_to) = return_to;
    place_quick(app, &w);
    let _ = w.show();
    let _ = w.set_focus();
    let _ = app.emit_to(QUICK, QUICK_SHOW, ());
    crate::log("quick window shown");
}

/// Centres the window horizontally on the monitor under the cursor, a bit
/// above the middle (like Spotlight). Works in logical points so mixed-DPI
/// setups (Retina laptop + external screen) place it correctly.
fn place_quick<R: Runtime>(app: &AppHandle<R>, w: &WebviewWindow<R>) {
    let Some(m) = cursor_monitor(app).or_else(|| app.primary_monitor().ok().flatten()) else {
        let _ = w.center();
        return;
    };
    let s = m.scale_factor();
    let area = m.work_area();
    let (ax, ay) = (
        f64::from(area.position.x) / s,
        f64::from(area.position.y) / s,
    );
    let (aw, ah) = (
        f64::from(area.size.width) / s,
        f64::from(area.size.height) / s,
    );
    let x = ax + (aw - QUICK_WIDTH) / 2.0;
    let y = ay + ah * 0.24;
    let _ = w.set_position(LogicalPosition::new(x, y));
}

/// The monitor under the mouse pointer.
///
/// tao 0.37 (macOS) builds the cursor's y from the main display's *pixel*
/// height minus a *point* coordinate, then scales by the primary monitor's
/// factor; `fixed_y` undoes that. The naive reading is kept as a fallback in
/// case a later tao fixes it.
fn cursor_monitor<R: Runtime>(app: &AppHandle<R>) -> Option<tauri::Monitor> {
    let monitors = app.available_monitors().ok()?;
    let primary = app.primary_monitor().ok().flatten()?;
    let s = primary.scale_factor();
    let p = app.cursor_position().ok()?;
    let x = p.x / s;
    let naive_y = p.y / s;
    let main_h = f64::from(primary.size().height) / s;
    let fixed_y = naive_y - main_h * (s - 1.0);
    let contains = |m: &tauri::Monitor, y: f64| {
        let ms = m.scale_factor();
        let (mx, my) = (
            f64::from(m.position().x) / ms,
            f64::from(m.position().y) / ms,
        );
        let (mw, mh) = (
            f64::from(m.size().width) / ms,
            f64::from(m.size().height) / ms,
        );
        x >= mx && x < mx + mw && y >= my && y < my + mh
    };
    [fixed_y, naive_y]
        .into_iter()
        .find_map(|y| monitors.iter().find(|m| contains(m, y)).cloned())
}

/// Hides the quick window and gives focus back to where it came from: the
/// main window if it had focus, else the previously active app.
pub fn hide_quick<R: Runtime>(app: &AppHandle<R>) {
    let Some(w) = app.get_webview_window(QUICK) else {
        return;
    };
    // Already dismissed (e.g. by clicking another app): leave focus alone.
    let had_focus = w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false);
    let _ = w.hide();
    let return_to = std::mem::replace(
        &mut *crate::lock(&app.state::<Native>().return_to),
        ReturnTo::Nothing,
    );
    if !had_focus {
        return;
    }
    match return_to {
        ReturnTo::Main => {
            if let Some(m) = app.get_webview_window(MAIN) {
                let _ = m.set_focus();
            }
        }
        // Re-activate the app that was in front; the main window (if open)
        // stays where it was in the window order.
        ReturnTo::App(pid) if activate_app(pid) => {}
        _ => {
            // Fallback: deactivate (hides our windows until the app is
            // reopened, but never brings the main window forward).
            #[cfg(target_os = "macos")]
            let _ = app.hide();
        }
    }
}

/// Where focus goes back to when the quick window closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReturnTo {
    Nothing,
    Main,
    /// pid of the app that was frontmost before the shortcut.
    App(i32),
}

/// pid of the frontmost app, when it is not MontyBrain itself.
#[cfg(target_os = "macos")]
fn previous_app() -> Option<i32> {
    use objc2_app_kit::NSWorkspace;
    let front = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    let pid = front.processIdentifier();
    (i64::from(pid) != i64::from(std::process::id())).then_some(pid)
}

#[cfg(not(target_os = "macos"))]
fn previous_app() -> Option<i32> {
    None
}

#[cfg(target_os = "macos")]
fn activate_app(pid: i32) -> bool {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .is_some_and(|a| a.activateWithOptions(NSApplicationActivationOptions::empty()))
}

#[cfg(not(target_os = "macos"))]
fn activate_app(_pid: i32) -> bool {
    false
}

pub fn resize_quick<R: Runtime>(app: &AppHandle<R>, height: f64) {
    if let Some(w) = app.get_webview_window(QUICK) {
        let h = height.clamp(QUICK_MIN_HEIGHT, QUICK_MAX_HEIGHT);
        let scale = w.scale_factor().unwrap_or(1.0);
        let size = PhysicalSize::new(
            (QUICK_WIDTH * scale).round() as u32,
            (h * scale).round() as u32,
        );
        let _ = w.set_size(size);
    }
}

// --- shortcut ---------------------------------------------------------------

/// The OS side of the global shortcut, so the state machine can be tested
/// with a fake that fails on demand.
trait ShortcutOs {
    fn register(&self, canonical: &str) -> Result<(), String>;
    fn unregister(&self, canonical: &str) -> Result<(), String>;
}

struct GlobalShortcuts<'a, R: Runtime>(&'a AppHandle<R>);

impl<R: Runtime> ShortcutOs for GlobalShortcuts<'_, R> {
    fn register(&self, canonical: &str) -> Result<(), String> {
        let sc: Shortcut = canonical
            .parse()
            .map_err(|e| format!("Nieprawidłowy skrót {canonical}: {e}"))?;
        self.0.global_shortcut().register(sc).map_err(|e| {
            crate::log(&format!("register {canonical} failed: {e}"));
            format!(
                "Nie udało się ustawić skrótu {}: jest zajęty przez inną aplikację lub system.",
                crate::shortcut::pretty(canonical)
            )
        })?;
        crate::log(&format!("shortcut registered: {canonical}"));
        Ok(())
    }

    fn unregister(&self, canonical: &str) -> Result<(), String> {
        let sc: Shortcut = canonical.parse().map_err(|e| format!("{canonical}: {e}"))?;
        self.0.global_shortcut().unregister(sc).map_err(|e| {
            crate::log(&format!("unregister {canonical} failed: {e}"));
            format!(
                "Nie udało się zwolnić skrótu {}.",
                crate::shortcut::pretty(canonical)
            )
        })
    }
}

/// Shortcut state plus the lock that serializes every transition: OS
/// (un)registration, the tracked status and rollback happen under `op`, so two
/// concurrent changes can never leave an untracked registration behind. Every
/// OS registration is accounted for: the active one (unless paused) and
/// `status.strays`. Events are emitted by the callers after `op` is released.
#[derive(Debug, Default)]
struct ShortcutCtl {
    op: Mutex<()>,
    status: Mutex<ShortcutStatus>,
}

impl ShortcutCtl {
    /// Removes `c` from the OS; when that fails the registration is kept as a
    /// tracked stray instead of being forgotten.
    fn release(&self, os: &dyn ShortcutOs, c: &str) -> Result<(), String> {
        let r = os.unregister(c);
        if r.is_err() {
            crate::lock(&self.status).strays.insert(c.to_string());
        }
        r
    }

    /// Retries removing every stray. Called with `op` held.
    fn sweep(&self, os: &dyn ShortcutOs) {
        let strays: Vec<String> = crate::lock(&self.status).strays.iter().cloned().collect();
        for c in strays {
            if os.unregister(&c).is_ok() {
                crate::lock(&self.status).strays.remove(&c);
            }
        }
    }

    /// Registers `canonical` and persists it through `persist`; on any failure
    /// the previous shortcut is back in place (OS and status), and whatever
    /// could not be cleaned up stays tracked.
    fn set(
        &self,
        os: &dyn ShortcutOs,
        canonical: &str,
        persist: &mut dyn FnMut(&str) -> Result<(), String>,
    ) -> Result<(), String> {
        let _op = crate::lock(&self.op);
        self.sweep(os);
        let (previous, was_paused) = {
            let st = crate::lock(&self.status);
            (st.active.clone(), st.paused)
        };
        // A paused shortcut is already unregistered.
        if let Some(prev) = &previous
            && !was_paused
        {
            os.unregister(prev)?;
        }
        // A stray of the very combination we want is already registered.
        let already = crate::lock(&self.status).strays.remove(canonical);
        let result = if already {
            Ok(())
        } else {
            os.register(canonical)
        }
        .and_then(|()| {
            persist(canonical).inspect_err(|_| {
                // Saved nowhere, so it must not stay registered either.
                let _ = self.release(os, canonical);
            })
        });
        match result {
            Ok(()) => {
                let mut st = crate::lock(&self.status);
                st.active = Some(canonical.to_string());
                st.error = None;
                st.paused = false;
                Ok(())
            }
            Err(e) => {
                crate::lock(&self.status).paused = false;
                let restored = previous.as_deref().map(|prev| os.register(prev));
                let mut st = crate::lock(&self.status);
                match (previous, restored) {
                    (Some(prev), Some(Ok(()))) => st.active = Some(prev),
                    (Some(_), Some(Err(e2))) => {
                        st.active = None;
                        st.error = Some(e2);
                    }
                    _ => st.active = None,
                }
                Err(e)
            }
        }
    }

    /// Unregisters / re-registers the active shortcut. Pausing flips the flag
    /// only when the OS call worked. A failed resume leaves the shortcut
    /// inactive with `error` set and is returned as `Err`.
    fn pause(&self, os: &dyn ShortcutOs, paused: bool) -> Result<(), String> {
        let _op = crate::lock(&self.op);
        self.sweep(os);
        let (active, was_paused) = {
            let st = crate::lock(&self.status);
            (st.active.clone(), st.paused)
        };
        if was_paused == paused {
            return Ok(());
        }
        let Some(c) = active else {
            crate::lock(&self.status).paused = paused;
            return Ok(());
        };
        if paused {
            os.unregister(&c)?;
            crate::lock(&self.status).paused = true;
            Ok(())
        } else {
            let already = crate::lock(&self.status).strays.remove(&c);
            let r = if already { Ok(()) } else { os.register(&c) };
            let mut st = crate::lock(&self.status);
            st.paused = false;
            if let Err(e) = r {
                st.active = None;
                st.error = Some(e.clone());
                return Err(e);
            }
            Ok(())
        }
    }

    /// Best-effort removal of every registration on quit.
    fn release_all(&self, os: &dyn ShortcutOs) {
        let _op = crate::lock(&self.op);
        self.sweep(os);
        let (active, paused) = {
            let st = crate::lock(&self.status);
            (st.active.clone(), st.paused)
        };
        if let Some(c) = active
            && !paused
        {
            let _ = self.release(os, &c);
        }
    }
}

/// Registers `accel` (normalized first); returns the canonical string.
fn register<R: Runtime>(app: &AppHandle<R>, accel: &str) -> Result<String, String> {
    let canonical = crate::shortcut::canonical(accel)?;
    GlobalShortcuts(app).register(&canonical)?;
    Ok(canonical)
}

/// Saves + registers a new shortcut. On failure (registration or saving) the
/// previous one stays registered and the saved settings are untouched.
pub fn set_shortcut<R: Runtime>(app: &AppHandle<R>, accel: &str) -> Result<SettingsView, String> {
    let native = app.state::<Native>();
    let canonical = crate::shortcut::canonical(accel)?;
    let result = native
        .shortcut
        .set(&GlobalShortcuts(app), &canonical, &mut |c| {
            commit_settings(
                &native.settings,
                &native.settings_op,
                &mut |s| {
                    let changed = s.shortcut != c;
                    s.shortcut = c.to_string();
                    Ok(changed)
                },
                &mut |s| s.save(&native.settings_path()),
            )
            .map(|_| ())
        });
    // The lock is released: tell the UI whatever the outcome (status may have changed).
    changed(app);
    result?;
    Ok(settings_view(app))
}

/// Unregisters the shortcut while Settings records a new one (so pressing the
/// current combination reaches the page), and re-registers afterwards.
pub fn pause_shortcut<R: Runtime>(
    app: &AppHandle<R>,
    paused: bool,
) -> Result<SettingsView, String> {
    let native = app.state::<Native>();
    let result = native.shortcut.pause(&GlobalShortcuts(app), paused);
    // Locks are released. A failed resume deactivated the shortcut: tell every
    // view, not just the caller.
    if result.is_err() || !paused {
        changed(app);
    }
    result?;
    Ok(settings_view(app))
}

/// On quit: unregister the shortcut and any registration that could not be
/// removed earlier.
pub fn release_shortcuts<R: Runtime>(app: &AppHandle<R>) {
    if let Some(native) = app.try_state::<Native>() {
        native.shortcut.release_all(&GlobalShortcuts(app));
    }
}

// --- settings ---------------------------------------------------------------

/// Applies `mutate` to a clone of the settings, saves the clone, and only then
/// publishes it to memory. A failed save leaves memory untouched, so a retry
/// sees the change again. Returns whether anything changed.
fn commit_settings(
    slot: &Mutex<Settings>,
    op: &Mutex<()>,
    mutate: &mut dyn FnMut(&mut Settings) -> Result<bool, String>,
    save: &mut dyn FnMut(&Settings) -> std::io::Result<()>,
) -> Result<bool, String> {
    let _op = crate::lock(op);
    let mut next = crate::lock(slot).clone();
    let before = next.clone();
    let changed = mutate(&mut next)?;
    if !changed && next == before {
        return Ok(false);
    }
    save(&next).map_err(|e| format!("Nie udało się zapisać ustawień: {e}"))?;
    *crate::lock(slot) = next;
    Ok(true)
}

pub fn update_settings<R: Runtime>(
    app: &AppHandle<R>,
    patch: SettingsPatch,
) -> Result<SettingsView, String> {
    let native = app.state::<Native>();
    let changed_now = commit_settings(
        &native.settings,
        &native.settings_op,
        &mut |s| s.apply(patch.clone()),
        &mut |s| s.save(&native.settings_path()),
    )?;
    if changed_now {
        changed(app);
    }
    Ok(settings_view(app))
}

/// Sets the automatic update of one project (validated by the caller too).
pub fn set_auto_update<R: Runtime>(
    app: &AppHandle<R>,
    project: &str,
    cfg: crate::updates::schedule::AutoUpdate,
) -> Result<SettingsView, String> {
    let native = app.state::<Native>();
    let changed_now = commit_settings(
        &native.settings,
        &native.settings_op,
        &mut |s| s.set_auto_update(project, cfg),
        &mut |s| s.save(&native.settings_path()),
    )?;
    if changed_now {
        changed(app);
    }
    Ok(settings_view(app))
}

/// Sets the update sources of one project.
pub fn set_update_sources<R: Runtime>(
    app: &AppHandle<R>,
    project: &str,
    src: crate::updates::schedule::UpdateSources,
) -> Result<SettingsView, String> {
    let native = app.state::<Native>();
    let changed_now = commit_settings(
        &native.settings,
        &native.settings_op,
        &mut |s| s.set_update_sources(project, src),
        &mut |s| s.save(&native.settings_path()),
    )?;
    if changed_now {
        changed(app);
    }
    Ok(settings_view(app))
}

pub fn set_autostart<R: Runtime>(
    app: &AppHandle<R>,
    enabled: bool,
) -> Result<SettingsView, String> {
    let al = app.autolaunch();
    let res = if enabled { al.enable() } else { al.disable() };
    res.map_err(|e| format!("Nie udało się zmienić uruchamiania przy logowaniu: {e}"))?;
    crate::log(&format!("autostart: {enabled}"));
    // Either way the pre-rename login item must not keep starting the old app.
    remove_legacy_autostart(app);
    changed(app);
    Ok(settings_view(app))
}

/// Removes the pre-rename app's login item when it is ours (logged, never
/// fatal).
fn remove_legacy_autostart<R: Runtime>(app: &AppHandle<R>) {
    let Ok(home) = app.path().home_dir() else {
        return;
    };
    match crate::legacy_autostart::remove(&home) {
        Ok(true) => crate::log("autostart: removed the old Morwisko login item"),
        Ok(false) => {}
        Err(e) => crate::log(&format!("autostart: cannot remove the old login item: {e}")),
    }
}

/// First start of the bundled app after the rename: an old login item that
/// is ours becomes the new one (autostart stays on, pointing at this app).
/// Dev builds leave login items alone.
fn carry_over_autostart<R: Runtime>(app: &AppHandle<R>) {
    let Ok(home) = app.path().home_dir() else {
        return;
    };
    if !crate::legacy_autostart::present(&home)
        || !std::env::current_exe().is_ok_and(|e| crate::legacy_autostart::runs_from_bundle(&e))
    {
        return;
    }
    let al = app.autolaunch();
    if !al.is_enabled().unwrap_or(false)
        && let Err(e) = al.enable()
    {
        crate::log(&format!(
            "autostart: cannot carry the old login item over: {e}"
        ));
        return;
    }
    crate::log("autostart: carried over from the old Morwisko login item");
    remove_legacy_autostart(app);
}

/// Settings changed: tell the UI, refresh the tray, re-check reminders.
fn changed<R: Runtime>(app: &AppHandle<R>) {
    let _ = app.emit(SETTINGS_CHANGED, settings_view(app));
    vault_changed(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_match_the_router() {
        assert_eq!(location_hash(&TaskLocation::Inbox), "#/skrzynka");
        assert_eq!(
            location_hash(&TaskLocation::Project {
                project: "wub".into()
            }),
            "#/p/wub"
        );
        assert_eq!(
            location_hash(&TaskLocation::Topic {
                project: "wub".into(),
                topic: "a b".into()
            }),
            "#/p/wub?temat=a%20b"
        );
    }

    // --- settings commit -------------------------------------------------------

    #[test]
    fn failed_settings_save_changes_nothing_and_retry_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(settings::SETTINGS_FILE);
        let slot = Mutex::new(Settings::default());
        let op = Mutex::new(());
        let patch = || SettingsPatch {
            digest_hour: Some(7),
            ..Default::default()
        };
        let mut apply = |s: &mut Settings| s.apply(patch());

        let err = commit_settings(&slot, &op, &mut apply, &mut |_| {
            Err(std::io::Error::other("disk full"))
        });
        assert!(err.is_err());
        assert_eq!(
            crate::lock(&slot).digest_hour,
            settings::DEFAULT_DIGEST_HOUR
        );
        assert!(!path.exists());

        // Identical retry must really persist, not report a no-op success.
        let ok = commit_settings(&slot, &op, &mut apply, &mut |s| s.save(&path));
        assert_eq!(ok, Ok(true));
        assert_eq!(crate::lock(&slot).digest_hour, 7);
        assert_eq!(Settings::load(&path).digest_hour, 7);

        // A no-op patch does not write.
        std::fs::remove_file(&path).unwrap();
        let noop = commit_settings(&slot, &op, &mut apply, &mut |s| s.save(&path));
        assert_eq!(noop, Ok(false));
        assert!(!path.exists());
    }

    // --- shortcut state machine --------------------------------------------------

    use std::sync::mpsc;

    /// Parks the first `register(combo)` until released, so a test can force
    /// another operation to run while a transition is mid-flight.
    struct Gate {
        combo: String,
        entered: mpsc::Sender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    /// A fake OS: tracks what is registered, fails calls on demand.
    #[derive(Default)]
    struct FakeOs {
        registered: Mutex<BTreeSet<String>>,
        /// `register` of these fails (taken by another app).
        taken: Mutex<BTreeSet<String>>,
        /// `unregister` of these fails.
        stuck: Mutex<BTreeSet<String>>,
        gate: Mutex<Option<Gate>>,
    }

    impl ShortcutOs for FakeOs {
        fn register(&self, c: &str) -> Result<(), String> {
            let gate = {
                let mut g = self.gate.lock().unwrap();
                if g.as_ref().is_some_and(|g| g.combo == c) {
                    g.take()
                } else {
                    None
                }
            };
            if let Some(g) = gate {
                g.entered.send(()).unwrap();
                g.release.lock().unwrap().recv().unwrap();
            }
            if self.taken.lock().unwrap().contains(c) {
                return Err(format!("{c} zajęty"));
            }
            if !self.registered.lock().unwrap().insert(c.to_string()) {
                return Err(format!("{c} już zarejestrowany"));
            }
            Ok(())
        }
        fn unregister(&self, c: &str) -> Result<(), String> {
            if self.stuck.lock().unwrap().contains(c) {
                return Err("nie da się".into());
            }
            self.registered.lock().unwrap().remove(c);
            Ok(())
        }
    }

    fn ctl_with(os: &FakeOs, active: &str) -> ShortcutCtl {
        os.register(active).unwrap();
        let ctl = ShortcutCtl::default();
        ctl.status.lock().unwrap().active = Some(active.into());
        ctl
    }

    fn registered(os: &FakeOs) -> Vec<String> {
        os.registered.lock().unwrap().iter().cloned().collect()
    }

    fn set(os: &str) -> BTreeSet<String> {
        BTreeSet::from([os.to_string()])
    }

    /// Every OS registration is the active one (unless paused) or a tracked stray.
    fn assert_accounted(os: &FakeOs, ctl: &ShortcutCtl) {
        let st = ctl.status.lock().unwrap();
        let mut expect = st.strays.clone();
        if !st.paused {
            expect.extend(st.active.clone());
        }
        assert_eq!(*os.registered.lock().unwrap(), expect);
    }

    /// Runs `a` and, while it is parked inside `register(gate_combo)`, `b`.
    /// Returns whether `b` managed to finish before `a` was released (it must
    /// not: both are serialized by the transition lock).
    fn contested(
        os: &FakeOs,
        gate_combo: &str,
        a: impl FnOnce() + Send,
        b: impl FnOnce() + Send,
    ) -> bool {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (b_done_tx, b_done_rx) = mpsc::channel();
        *os.gate.lock().unwrap() = Some(Gate {
            combo: gate_combo.into(),
            entered: entered_tx,
            release: Mutex::new(release_rx),
        });
        let mut b_early = false;
        std::thread::scope(|sc| {
            sc.spawn(a);
            entered_rx.recv().unwrap(); // `a` is now inside the OS call
            sc.spawn(move || {
                b();
                let _ = b_done_tx.send(());
            });
            // Without the lock `b` finishes right away; with it, it is blocked.
            b_early = b_done_rx
                .recv_timeout(std::time::Duration::from_millis(500))
                .is_ok();
            release_tx.send(()).unwrap();
        });
        b_early
    }

    #[test]
    fn a_change_cannot_run_while_another_change_is_in_the_os_call() {
        let os = FakeOs::default();
        let ctl = ctl_with(&os, "Ctrl+Alt+Space");
        let early = contested(
            &os,
            "Ctrl+Alt+A",
            || ctl.set(&os, "Ctrl+Alt+A", &mut |_| Ok(())).unwrap(),
            || ctl.set(&os, "Ctrl+Alt+B", &mut |_| Ok(())).unwrap(),
        );
        assert!(!early, "second change ran inside the first one's OS call");
        assert_eq!(
            ctl.status.lock().unwrap().active.as_deref(),
            Some("Ctrl+Alt+B")
        );
        assert_accounted(&os, &ctl);
        assert_eq!(registered(&os), vec!["Ctrl+Alt+B".to_string()]);
    }

    #[test]
    fn resume_cannot_interleave_with_a_change() {
        // Paused with `old` unregistered; a change to `new` is mid-register
        // when a resume arrives. Without the lock the resume re-registers `old`
        // and `old` + `new` end up registered while only `new` is tracked.
        let os = FakeOs::default();
        let ctl = ctl_with(&os, "Ctrl+Alt+Space");
        ctl.pause(&os, true).unwrap();
        let early = contested(
            &os,
            "Ctrl+Alt+A",
            || ctl.set(&os, "Ctrl+Alt+A", &mut |_| Ok(())).unwrap(),
            || ctl.pause(&os, false).unwrap(),
        );
        assert!(!early, "resume ran inside the change's OS call");
        assert_eq!(registered(&os), vec!["Ctrl+Alt+A".to_string()]);
        assert_accounted(&os, &ctl);
    }

    #[test]
    fn pause_cannot_interleave_with_a_change() {
        let os = FakeOs::default();
        let ctl = ctl_with(&os, "Ctrl+Alt+Space");
        let early = contested(
            &os,
            "Ctrl+Alt+A",
            || ctl.set(&os, "Ctrl+Alt+A", &mut |_| Ok(())).unwrap(),
            || ctl.pause(&os, true).unwrap(),
        );
        assert!(!early, "pause ran inside the change's OS call");
        assert_accounted(&os, &ctl);
    }

    // Failures at every step of a transition, including during rollback.

    #[test]
    fn rollback_cleanup_failure_keeps_the_stray_tracked_and_retries_it() {
        // register new ok -> persist fails -> unregister new fails -> restore old ok
        let os = FakeOs::default();
        let ctl = ctl_with(&os, "Ctrl+Alt+Space");
        os.stuck.lock().unwrap().insert("Ctrl+Alt+A".into());
        let r = ctl.set(&os, "Ctrl+Alt+A", &mut |_| Err("zapis".into()));
        assert_eq!(r, Err("zapis".into()));
        {
            let st = ctl.status.lock().unwrap();
            assert_eq!(st.active.as_deref(), Some("Ctrl+Alt+Space"));
            assert_eq!(st.strays, set("Ctrl+Alt+A"));
            assert!(st.view_error().unwrap().contains("zwolnić"));
        }
        assert_eq!(registered(&os).len(), 2);
        assert_accounted(&os, &ctl);

        // Still stuck on the next transition: stays tracked.
        let _ = ctl.set(&os, "Ctrl+Alt+B", &mut |_| Err("zapis".into()));
        assert!(ctl.status.lock().unwrap().strays.contains("Ctrl+Alt+A"));
        assert_accounted(&os, &ctl);

        // Once the OS lets go, the next transition removes it and the error clears.
        os.stuck.lock().unwrap().clear();
        ctl.set(&os, "Ctrl+Alt+C", &mut |_| Ok(())).unwrap();
        assert_eq!(registered(&os), vec!["Ctrl+Alt+C".to_string()]);
        let st = ctl.status.lock().unwrap();
        assert!(st.strays.is_empty());
        assert_eq!(st.view_error(), None);
    }

    #[test]
    fn rollback_cleanup_and_restore_both_failing_is_fully_tracked() {
        // persist fails -> unregister new fails -> restore old fails
        let os = FakeOs::default();
        let ctl = ctl_with(&os, "Ctrl+Alt+Space");
        os.stuck.lock().unwrap().insert("Ctrl+Alt+A".into());
        os.taken.lock().unwrap().insert("Ctrl+Alt+Space".into());
        let r = ctl.set(&os, "Ctrl+Alt+A", &mut |_| Err("zapis".into()));
        assert_eq!(r, Err("zapis".into()));
        let st = ctl.status.lock().unwrap();
        assert_eq!(st.active, None);
        assert!(st.error.as_deref().unwrap().contains("zajęty"));
        assert_eq!(st.strays, set("Ctrl+Alt+A"));
        drop(st);
        assert_eq!(registered(&os), vec!["Ctrl+Alt+A".to_string()]);
        assert_accounted(&os, &ctl);
    }

    #[test]
    fn register_failure_then_restore_failure_leaves_nothing_registered() {
        let os = FakeOs::default();
        let ctl = ctl_with(&os, "Ctrl+Alt+Space");
        os.taken.lock().unwrap().extend([
            "Ctrl+Alt+A".to_string(),
            // Another app grabs the released old combination.
            "Ctrl+Alt+Space".to_string(),
        ]);
        assert!(ctl.set(&os, "Ctrl+Alt+A", &mut |_| Ok(())).is_err());
        assert!(registered(&os).is_empty());
        let st = ctl.status.lock().unwrap();
        assert_eq!(st.active, None);
        assert!(st.error.is_some());
    }

    #[test]
    fn a_stray_of_the_requested_combination_is_adopted() {
        let os = FakeOs::default();
        let ctl = ctl_with(&os, "Ctrl+Alt+Space");
        os.stuck.lock().unwrap().insert("Ctrl+Alt+A".into());
        let _ = ctl.set(&os, "Ctrl+Alt+A", &mut |_| Err("zapis".into()));
        // Stuck unregister, but the user now wants exactly that combination.
        ctl.set(&os, "Ctrl+Alt+A", &mut |_| Ok(())).unwrap();
        assert_eq!(registered(&os), vec!["Ctrl+Alt+A".to_string()]);
        assert!(ctl.status.lock().unwrap().strays.is_empty());
        assert_accounted(&os, &ctl);
    }

    #[test]
    fn quit_releases_active_and_strays_and_keeps_what_it_cannot() {
        let os = FakeOs::default();
        let ctl = ctl_with(&os, "Ctrl+Alt+Space");
        os.stuck.lock().unwrap().insert("Ctrl+Alt+A".into());
        let _ = ctl.set(&os, "Ctrl+Alt+A", &mut |_| Err("zapis".into()));
        os.stuck.lock().unwrap().clear();
        ctl.release_all(&os);
        assert!(registered(&os).is_empty());
        assert!(ctl.status.lock().unwrap().strays.is_empty());
    }

    #[test]
    fn failed_resume_is_reported_and_deactivates_the_shortcut() {
        let os = FakeOs::default();
        let ctl = ctl_with(&os, "Ctrl+Alt+Space");
        ctl.pause(&os, true).unwrap();
        os.taken.lock().unwrap().insert("Ctrl+Alt+Space".into());
        let r = ctl.pause(&os, false);
        assert!(r.unwrap_err().contains("zajęty"));
        let st = ctl.status.lock().unwrap();
        assert_eq!(st.active, None);
        assert!(!st.paused);
        assert!(st.view_error().is_some());
        drop(st);
        assert!(registered(&os).is_empty());
    }

    #[test]
    fn pause_sweeps_strays() {
        let os = FakeOs::default();
        let ctl = ctl_with(&os, "Ctrl+Alt+Space");
        os.stuck.lock().unwrap().insert("Ctrl+Alt+A".into());
        let _ = ctl.set(&os, "Ctrl+Alt+A", &mut |_| Err("zapis".into()));
        os.stuck.lock().unwrap().clear();
        ctl.pause(&os, true).unwrap();
        assert!(registered(&os).is_empty());
        assert_accounted(&os, &ctl);
    }

    #[test]
    fn failed_persist_rolls_back_the_registration() {
        let os = FakeOs::default();
        let ctl = ctl_with(&os, "Ctrl+Alt+Space");
        let r = ctl.set(&os, "Ctrl+Alt+A", &mut |_| Err("zapis".into()));
        assert_eq!(r, Err("zapis".into()));
        assert_eq!(registered(&os), vec!["Ctrl+Alt+Space".to_string()]);
        assert_eq!(
            ctl.status.lock().unwrap().active.as_deref(),
            Some("Ctrl+Alt+Space")
        );
    }

    #[test]
    fn taken_shortcut_keeps_the_previous_one() {
        let os = FakeOs::default();
        let ctl = ctl_with(&os, "Ctrl+Alt+Space");
        os.taken.lock().unwrap().insert("Ctrl+Alt+A".into());
        assert!(ctl.set(&os, "Ctrl+Alt+A", &mut |_| Ok(())).is_err());
        assert_eq!(registered(&os), vec!["Ctrl+Alt+Space".to_string()]);
    }

    #[test]
    fn failed_unregister_aborts_without_a_second_registration() {
        let os = FakeOs::default();
        let ctl = ctl_with(&os, "Ctrl+Alt+Space");
        os.stuck.lock().unwrap().insert("Ctrl+Alt+Space".into());
        assert!(ctl.set(&os, "Ctrl+Alt+A", &mut |_| Ok(())).is_err());
        assert_eq!(registered(&os), vec!["Ctrl+Alt+Space".to_string()]);
        // Pausing cannot claim success either.
        assert!(ctl.pause(&os, true).is_err());
        assert!(!ctl.status.lock().unwrap().paused);
    }

    #[test]
    fn pause_and_resume_round_trip() {
        let os = FakeOs::default();
        let ctl = ctl_with(&os, "Ctrl+Alt+Space");
        ctl.pause(&os, true).unwrap();
        assert!(registered(&os).is_empty());
        // Changing while paused does not unregister twice and ends unpaused.
        ctl.set(&os, "Ctrl+Alt+A", &mut |_| Ok(())).unwrap();
        assert_eq!(registered(&os), vec!["Ctrl+Alt+A".to_string()]);
        assert!(!ctl.status.lock().unwrap().paused);
        ctl.pause(&os, true).unwrap();
        ctl.pause(&os, false).unwrap();
        assert_eq!(registered(&os), vec!["Ctrl+Alt+A".to_string()]);
    }
}
