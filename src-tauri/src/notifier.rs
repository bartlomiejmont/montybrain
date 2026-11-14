//! Showing a system notification so that the scheduler learns about failures.
//!
//! `tauri-plugin-notification` 2.4 (desktop) spawns the delivery on the async
//! runtime, drops the `notify-rust` handle (which is what actually sends, and
//! swallows its error) and returns `Ok(())` whatever happens. Through it a
//! failure can never reach the reminder tracker. On macOS we therefore call the
//! backend (`mac-notification-sys`, the crate under `notify-rust`) ourselves,
//! synchronously on the calling (scheduler) thread:
//!
//! * the notification is attributed to MontyBrain by binding the bundle id
//!   (`tauri.conf.json` identifier; `com.apple.Terminal` in dev, like the
//!   plugin does) via `set_application`;
//! * it is sent with `asynchronous(true)` and no buttons, so nothing waits for
//!   user interaction; the call only waits (bounded, 2 s, on a condvar when
//!   not on the main thread) for the delivery acknowledgement.
//!
//! What is **detected** (returned as `Err`, so the tracker retries with
//! backoff): the bundle id cannot be bound (`set_application` returns an
//! error when LaunchServices does not know the id, e.g. an unbundled binary)
//! and a notification the backend rejects up front.
//!
//! What is **not detectable at all** on macOS: `NSUserNotificationCenter`'s
//! `deliverNotification:` returns nothing, and `mac-notification-sys` does not
//! report the outcome of the delivery acknowledgement (its 2 s timeout is
//! silent). So a denied notification permission, Do Not Disturb / Focus,
//! "None" alert style in System Settings, or a failed XPC hand-over look
//! exactly like success and the reminder is recorded as fired. Retrying could
//! not change a denied permission anyway.
//!
//! Other platforms keep going through the plugin, whose result is likewise
//! only "the request was queued".

use std::sync::OnceLock;

/// The two macOS backend calls we depend on, as a seam for tests.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub trait MacBackend {
    fn set_application(&self, bundle_id: &str) -> Result<(), String>;
    fn send(&self, title: &str, body: &str) -> Result<(), String>;
}

/// Binds the bundle id once (the backend only allows that once per process and
/// afterwards reports "already set" even when the first attempt failed, so the
/// first outcome is remembered here) and then sends.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub struct MacNotifier<B> {
    backend: B,
    bundle_id: String,
    bound: OnceLock<Result<(), String>>,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
impl<B: MacBackend> MacNotifier<B> {
    pub fn new(backend: B, bundle_id: String) -> Self {
        Self {
            backend,
            bundle_id,
            bound: OnceLock::new(),
        }
    }

    pub fn deliver(&self, title: &str, body: &str) -> Result<(), String> {
        self.bound
            .get_or_init(|| self.backend.set_application(&self.bundle_id))
            .clone()?;
        self.backend.send(title, body)
    }
}

#[cfg(target_os = "macos")]
pub struct SysBackend;

#[cfg(target_os = "macos")]
impl MacBackend for SysBackend {
    fn set_application(&self, bundle_id: &str) -> Result<(), String> {
        mac_notification_sys::set_application(bundle_id)
            .map_err(|e| format!("bundle id {bundle_id}: {e}"))
    }

    fn send(&self, title: &str, body: &str) -> Result<(), String> {
        let mut n = mac_notification_sys::Notification::new();
        n.title(title).message(body).asynchronous(true);
        n.send().map(|_| ()).map_err(|e| e.to_string())
    }
}

/// Shows one notification; `Err` only for the failures described above.
pub fn deliver<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    title: &str,
    body: &str,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        static MAC: OnceLock<MacNotifier<SysBackend>> = OnceLock::new();
        MAC.get_or_init(|| {
            let id = if tauri::is_dev() {
                "com.apple.Terminal".to_string()
            } else {
                app.config().identifier.clone()
            };
            MacNotifier::new(SysBackend, id)
        })
        .deliver(title, body)
    }
    #[cfg(not(target_os = "macos"))]
    {
        use tauri_plugin_notification::NotificationExt;
        app.notification()
            .builder()
            .title(title)
            .body(body)
            .show()
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Fake {
        bind_calls: Mutex<Vec<String>>,
        sent: Mutex<Vec<(String, String)>>,
        bind_error: Option<String>,
        send_error: Option<String>,
    }

    impl MacBackend for &Fake {
        fn set_application(&self, id: &str) -> Result<(), String> {
            self.bind_calls.lock().unwrap().push(id.into());
            self.bind_error.clone().map_or(Ok(()), Err)
        }
        fn send(&self, t: &str, b: &str) -> Result<(), String> {
            if let Some(e) = &self.send_error {
                return Err(e.clone());
            }
            self.sent.lock().unwrap().push((t.into(), b.into()));
            Ok(())
        }
    }

    #[test]
    fn binds_bundle_id_once_then_sends_each() {
        let f = Fake::default();
        let n = MacNotifier::new(&f, "pl.montybrain.desktop".into());
        n.deliver("a", "1").unwrap();
        n.deliver("b", "2").unwrap();
        assert_eq!(*f.bind_calls.lock().unwrap(), ["pl.montybrain.desktop"]);
        assert_eq!(f.sent.lock().unwrap().len(), 2);
    }

    #[test]
    fn bind_failure_is_reported_every_time_and_nothing_is_sent() {
        let f = Fake {
            bind_error: Some("unknown".into()),
            ..Default::default()
        };
        let n = MacNotifier::new(&f, "x".into());
        assert_eq!(n.deliver("a", "1"), Err("unknown".into()));
        assert_eq!(n.deliver("a", "1"), Err("unknown".into()));
        assert_eq!(f.bind_calls.lock().unwrap().len(), 1, "bound only once");
        assert!(f.sent.lock().unwrap().is_empty());
    }

    #[test]
    fn send_failure_is_reported() {
        let f = Fake {
            send_error: Some("rejected".into()),
            ..Default::default()
        };
        let n = MacNotifier::new(&f, "x".into());
        assert_eq!(n.deliver("a", "1"), Err("rejected".into()));
    }

    /// The real backend, without sending anything: an id LaunchServices does
    /// not know must surface as an error (this is the production-detectable
    /// failure). Only this one test touches the process-wide binding.
    #[cfg(target_os = "macos")]
    #[test]
    fn real_backend_reports_an_unknown_bundle_id() {
        let n = MacNotifier::new(SysBackend, "pl.montybrain.no-such-app.test".into());
        let err = n.deliver("never shown", "never shown").unwrap_err();
        assert!(err.contains("pl.montybrain.no-such-app.test"), "{err}");
    }
}
