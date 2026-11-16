//! Menu-bar (tray) item: a monochrome template icon, the number of "today"
//! items that need attention next to it, and a small menu.
//!
//! [`refresh`] is cheap to call often (vault change, scheduler tick): the menu is
//! rebuilt only when what it shows changed.

use std::sync::Mutex;

use montybrain_core::chrono::NaiveDate;
use montybrain_core::{TodayItem, TodayReason, local_today};
use tauri::image::Image;
use tauri::menu::{IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Manager, Runtime};

use crate::state::VaultState;
use crate::settings::Language;

const TRAY_ID: &str = "montybrain";
/// How many tasks the menu lists.
const MENU_TASKS: usize = 6;
const MAX_LABEL: usize = 46;

/// Today items that count towards the number in the menu bar: overdue, due
/// today and in progress (blocked tasks wait on someone else).
pub fn attention(items: &[TodayItem]) -> Vec<&TodayItem> {
    items
        .iter()
        .filter(|i| i.reason != TodayReason::Blocked)
        .collect()
}

/// What the tray shows; the menu is rebuilt only when this changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayModel {
    pub language: Language,
    pub count: usize,
    /// (task id, label)
    pub tasks: Vec<(String, String)>,
    pub shortcut: Option<String>,
}

pub fn model(items: &[TodayItem], today: NaiveDate, shortcut: Option<String>) -> TrayModel {
    let att = attention(items);
    TrayModel {
        language: Language::Pl,
        count: att.len(),
        tasks: att
            .iter()
            .filter_map(|i| Some((i.task.id.clone()?, label(i, today))))
            .take(MENU_TASKS)
            .collect(),
        shortcut,
    }
}

fn label(item: &TodayItem, today: NaiveDate) -> String {
    let mut title: String = item.task.title.chars().take(MAX_LABEL).collect();
    if item.task.title.chars().count() > MAX_LABEL {
        title = title.trim_end().to_string() + "…";
    }
    let suffix = match item.reason {
        TodayReason::Overdue => Some("po terminie".to_string()),
        TodayReason::DueToday => item
            .task
            .due
            .and_then(|d| d.time)
            .filter(|_| item.task.due.is_some_and(|d| d.date == today))
            .map(|t| t.format("%H:%M").to_string()),
        TodayReason::Doing => Some("w toku".to_string()),
        TodayReason::Blocked => None,
    };
    match suffix {
        Some(s) => format!("{title}  ·  {s}"),
        None => title,
    }
}

/// Last model shown, to skip no-op rebuilds.
#[derive(Default)]
pub struct TrayState(Mutex<Option<TrayModel>>);

/// 18 pt template icon (36 px @2x): the app's "list" motif — three rows of a
/// dot and a bar, fading downwards. Black + alpha; macOS tints it.
pub fn icon_rgba(size: u32) -> Vec<u8> {
    let s = size as f32 / 36.0;
    let mut px = vec![0u8; (size * size * 4) as usize];
    // (row centre y, filled dot?, bar end x, alpha)
    let rows = [
        (10.0, true, 30.0, 1.0),
        (18.0, false, 26.0, 0.8),
        (26.0, false, 28.0, 0.6),
    ];
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = ((x as f32 + 0.5) / s, (y as f32 + 0.5) / s);
            let mut a: f32 = 0.0;
            for (cy, filled, bar_end, alpha) in rows {
                let dc = ((fx - 8.0).powi(2) + (fy - cy).powi(2)).sqrt();
                let d_dot = if filled {
                    dc - 3.4
                } else {
                    (dc - 2.7).abs() - 0.95
                };
                // Capsule from x=14 to bar_end, half-height 2.1.
                let qx = fx.clamp(16.1, bar_end - 2.1);
                let d_bar = ((fx - qx).powi(2) + (fy - cy).powi(2)).sqrt() - 2.1;
                let cover = (0.5 - d_dot.min(d_bar) * s).clamp(0.0, 1.0);
                a = a.max(cover * alpha);
            }
            let i = ((y * size + x) * 4) as usize;
            px[i + 3] = (a * 255.0).round() as u8;
        }
    }
    px
}

pub fn create<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    app.manage(TrayState::default());
    let size = 36;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::new_owned(icon_rgba(size), size, size))
        .icon_as_template(true)
        .tooltip("MontyBrain")
        .show_menu_on_left_click(true)
        .on_menu_event(on_menu_event)
        .build(app)?;
    refresh(app);
    Ok(())
}

/// Recomputes the count + menu from the current snapshot; rebuilds only on change.
pub fn refresh<R: Runtime>(app: &AppHandle<R>) {
    let (Some(tray), Some(state), Some(vault)) = (
        app.tray_by_id(TRAY_ID),
        app.try_state::<TrayState>(),
        app.try_state::<VaultState>(),
    ) else {
        return;
    };
    let today = local_today();
    let items = vault.snapshot().today(today);
    let mut next = model(&items, today, crate::native::active_shortcut(app));
    next.language = app.state::<crate::native::Native>().settings().language;
    if next.language == Language::En {
        for (_, label) in &mut next.tasks {
            if let Some(title) = label.strip_suffix("  ·  po terminie") { *label = format!("{title}  ·  overdue"); }
            else if let Some(title) = label.strip_suffix("  ·  w toku") { *label = format!("{title}  ·  in progress"); }
        }
    }
    let mut last = crate::lock(&state.0);
    if last.as_ref() == Some(&next) {
        return;
    }
    if let Err(e) = apply(app, &tray, &next) {
        crate::log(&format!("tray update failed: {e}"));
        return;
    }
    crate::log(&format!("tray: {} item(s) need attention", next.count));
    *last = Some(next);
}

fn apply<R: Runtime>(app: &AppHandle<R>, tray: &TrayIcon<R>, m: &TrayModel) -> tauri::Result<()> {
    tray.set_title(if m.count > 0 {
        Some(m.count.to_string())
    } else {
        None
    })?;
    tray.set_tooltip(Some(match m.count {
        0 => "MontyBrain".to_string(),
        n => format!("MontyBrain — {}: {}", m.language.text("today", "dziś"), crate::reminders::count_tasks_in(n, m.language)),
    }))?;

    let mut owned: Vec<Box<dyn IsMenuItem<R>>> = vec![
        Box::new(MenuItem::with_id(
            app,
            "open",
            m.language.text("Open MontyBrain", "Otwórz MontyBrain"),
            true,
            None::<&str>,
        )?),
        Box::new(MenuItem::with_id(
            app,
            "quick",
            m.language.text("Quick task…", "Szybkie zadanie…"),
            true,
            m.shortcut.as_deref(),
        )?),
        Box::new(PredefinedMenuItem::separator(app)?),
    ];
    if m.tasks.is_empty() {
        owned.push(Box::new(MenuItem::with_id(
            app,
            "none",
            m.language.text("Nothing urgent today", "Nic pilnego na dziś"),
            false,
            None::<&str>,
        )?));
    } else {
        owned.push(Box::new(MenuItem::with_id(
            app,
            "head",
            format!("{} · {}", m.language.text("Today", "Dziś"), m.count),
            false,
            None::<&str>,
        )?));
        for (id, label) in &m.tasks {
            owned.push(Box::new(MenuItem::with_id(
                app,
                format!("task:{id}"),
                label,
                true,
                None::<&str>,
            )?));
        }
    }
    owned.push(Box::new(PredefinedMenuItem::separator(app)?));
    owned.push(Box::new(MenuItem::with_id(
        app,
        "settings",
        m.language.text("Settings…", "Ustawienia…"),
        true,
        None::<&str>,
    )?));
    owned.push(Box::new(MenuItem::with_id(
        app,
        "quit",
        m.language.text("Quit", "Zakończ"),
        true,
        None::<&str>,
    )?));
    let refs: Vec<&dyn IsMenuItem<R>> = owned.iter().map(|b| b.as_ref()).collect();
    let menu = Menu::with_items(app, &refs)?;
    tray.set_menu(Some(menu))
}

fn on_menu_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    let id = event.id().as_ref();
    crate::log(&format!("tray menu: {id}"));
    match id {
        "open" => crate::native::show_main(app),
        "quick" => crate::native::show_quick(app),
        "settings" => {
            crate::native::show_main(app);
            crate::native::navigate_main(app, "#/ustawienia");
        }
        "quit" => app.exit(0),
        _ => {
            if let Some(task_id) = id.strip_prefix("task:") {
                let snap = app.state::<VaultState>().snapshot();
                let hash = snap.task(task_id).map_or_else(
                    || "#/dzis".to_string(),
                    |t| crate::native::location_hash(&t.location),
                );
                crate::native::show_main(app);
                crate::native::navigate_main(app, &hash);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use montybrain_core::{Due, Priority, Task, TaskLocation, TaskStatus};

    fn item(reason: TodayReason, title: &str, due: Option<&str>) -> TodayItem {
        TodayItem {
            reason,
            task: Task {
                id: Some(format!("{:0<6}", title.len())),
                title: title.into(),
                status: TaskStatus::Todo,
                priority: Priority::Normal,
                due: due.map(|d| d.parse::<Due>().unwrap()),
                done: None,
                links: vec![],
                notes: String::new(),
                location: TaskLocation::Inbox,
                line: 1,
            },
        }
    }

    #[test]
    fn count_skips_blocked_and_labels_read_well() {
        let today: NaiveDate = "2026-11-13".parse().unwrap();
        let items = vec![
            item(TodayReason::Overdue, "Stare", Some("2026-11-10")),
            item(TodayReason::DueToday, "MR", Some("2026-11-13T14:00")),
            item(TodayReason::DueToday, "Cały dzień", Some("2026-11-13")),
            item(TodayReason::Doing, "Robię", None),
            item(TodayReason::Blocked, "Czekam", None),
        ];
        let m = model(&items, today, None);
        assert_eq!(m.count, 4);
        let labels: Vec<&str> = m.tasks.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "Stare  ·  po terminie",
                "MR  ·  14:00",
                "Cały dzień",
                "Robię  ·  w toku"
            ]
        );
        assert_eq!(model(&[], today, None).count, 0);
    }

    #[test]
    fn long_titles_are_cut() {
        let today: NaiveDate = "2026-11-13".parse().unwrap();
        let long = "ż".repeat(80);
        let m = model(&[item(TodayReason::Doing, &long, None)], today, None);
        assert!(m.tasks[0].1.starts_with(&"ż".repeat(MAX_LABEL)));
        assert!(m.tasks[0].1.contains('…'));
    }

    #[test]
    fn icon_has_ink_and_transparency() {
        let px = icon_rgba(36);
        assert_eq!(px.len(), 36 * 36 * 4);
        let alphas: Vec<u8> = px.chunks(4).map(|p| p[3]).collect();
        assert!(alphas.contains(&255));
        assert_eq!(alphas[0], 0, "corner is transparent");
        assert!(px.chunks(4).all(|p| p[0] == 0 && p[1] == 0 && p[2] == 0));
    }
}
