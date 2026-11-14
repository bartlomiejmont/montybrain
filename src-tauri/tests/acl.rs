//! The capabilities (`capabilities/*.json` + the app manifest in `build.rs`)
//! let only the main window reach the agent-run and project-update commands. Runs the real
//! context (config + capabilities) on Tauri's mock runtime.

use montybrain_desktop::commands;
use tauri::ipc::{CallbackFn, InvokeBody, InvokeResponseBody};
use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder};
use tauri::webview::InvokeRequest;
use tauri::{App, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

fn app() -> App<MockRuntime> {
    mock_builder()
        .invoke_handler(tauri::generate_handler![
            commands::list_recipes,
            commands::list_runs,
            commands::start_run,
            commands::list_updates,
            commands::apply_update,
            commands::undo_update,
            commands::dismiss_update,
            commands::edit_update,
            commands::join_update,
            commands::list_drift,
            commands::link_branch,
            commands::list_waiting,
            commands::confirm_waiting,
            commands::set_auto_update,
            commands::set_update_sources,
            commands::quick_resize,
            commands::inbox_suggestions,
            commands::inbox_agent,
            commands::inbox_agent_answer,
            commands::inbox_agent_check,
            commands::inbox_agent_paste,
            commands::agent_session_start,
            commands::agent_session_get,
            commands::agent_session_send,
            commands::agent_session_answer,
            commands::agent_session_stop,
            commands::command_preview,
        ])
        .build(tauri::generate_context!(test = true))
        .expect("mock app")
}

fn window(app: &App<MockRuntime>, label: &str) -> WebviewWindow<MockRuntime> {
    app.get_webview_window(label).unwrap_or_else(|| {
        WebviewWindowBuilder::new(app, label, WebviewUrl::default())
            .build()
            .expect("window")
    })
}

fn invoke(
    app: &App<MockRuntime>,
    label: &str,
    cmd: &str,
    body: serde_json::Value,
) -> Result<InvokeResponseBody, serde_json::Value> {
    get_ipc_response(
        &window(app, label),
        InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(body),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
}

#[test]
fn run_commands_are_main_window_only() {
    let app = app();
    let ok = invoke(&app, "main", "list_recipes", serde_json::json!({})).expect("main may list");
    let InvokeResponseBody::Json(json) = ok else {
        panic!("json expected")
    };
    assert!(json.contains("\"postep\""), "{json}");

    for (cmd, body) in [
        ("list_recipes", serde_json::json!({})),
        ("list_runs", serde_json::json!({})),
        (
            "start_run",
            serde_json::json!({"request":{"recipe":"raport","provider":"claude","mode":"background","params":{}}}),
        ),
        ("list_updates", serde_json::json!({})),
        (
            "apply_update",
            serde_json::json!({"runId": "r1", "indices": [0]}),
        ),
        ("dismiss_update", serde_json::json!({"runId": "r1"})),
        (
            "join_update",
            serde_json::json!({"runId": "r1", "index": 0, "topic": "t"}),
        ),
        ("list_drift", serde_json::json!({})),
        (
            "link_branch",
            serde_json::json!({"project": "p", "repo": "r", "branch": "b", "topic": "t"}),
        ),
        ("list_waiting", serde_json::json!({})),
        ("confirm_waiting", serde_json::json!({"key": "topic:p/t"})),
        (
            "edit_update",
            serde_json::json!({"runId": "r1", "index": 0, "edit": {"kind": "new_task", "title": "x"}}),
        ),
        (
            "undo_update",
            serde_json::json!({"runId": "r1", "token": "r1#1"}),
        ),
        (
            "set_update_sources",
            serde_json::json!({"project": "p", "sources": {"history": true, "history_days": 14}}),
        ),
        (
            "set_auto_update",
            serde_json::json!({"project": "p", "config": {"schedule": {"kind": "every", "hours": 1}}}),
        ),
        ("inbox_suggestions", serde_json::json!({})),
        (
            "inbox_agent",
            serde_json::json!({"request": {"text": "", "selection": [], "mode": "proposals", "provider": "claude"}}),
        ),
        ("inbox_agent_answer", serde_json::json!({"runId": "r1"})),
        ("inbox_agent_check", serde_json::json!({"runId": "r1"})),
        (
            "inbox_agent_paste",
            serde_json::json!({"runId": "r1", "text": "x"}),
        ),
        (
            "agent_session_start",
            serde_json::json!({"request": {"scope": "inbox", "provider": "claude", "text": "x"}}),
        ),
        ("agent_session_get", serde_json::json!({})),
        ("agent_session_send", serde_json::json!({"text": "x"})),
        (
            "agent_session_answer",
            serde_json::json!({"entry": 1, "answer": "allow"}),
        ),
        ("agent_session_stop", serde_json::json!({})),
    ] {
        let e = invoke(&app, "quick", cmd, body).expect_err("quick must be refused");
        let msg = e.to_string();
        // Refused by the ACL, before the command's own window check runs.
        assert!(
            msg.contains("not allowed") && !msg.contains("głównym oknie"),
            "{cmd}: {msg}"
        );
    }
    // A window with no capability at all gets nothing either.
    let e = invoke(&app, "other", "list_recipes", serde_json::json!({})).unwrap_err();
    assert!(e.to_string().contains("not allowed"), "{e}");
    // The quick window keeps its own commands.
    invoke(
        &app,
        "quick",
        "quick_resize",
        serde_json::json!({"height": 100.0}),
    )
    .expect("quick keeps quick_resize");
    // The command field works in the quick window (the ACL lets it through;
    // without a vault in this mock app the command itself then fails).
    if let Err(e) = invoke(
        &app,
        "quick",
        "command_preview",
        serde_json::json!({"input": {"text": "x"}}),
    ) {
        assert!(!e.to_string().contains("not allowed"), "{e}");
    }
}
