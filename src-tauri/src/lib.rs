use quota::{QuotaSnapshot, now_seconds, snapshot_from_response};
use std::sync::Mutex;
use tauri::{
    Manager,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};

pub mod codex_adapter;
pub mod quota;

struct AppState {
    client: Mutex<Option<codex_adapter::CodexClient>>,
    last: Mutex<Option<QuotaSnapshot>>,
}

#[tauri::command]
fn read_quota(state: tauri::State<'_, AppState>) -> QuotaSnapshot {
    let mut client = state.client.lock().expect("client mutex poisoned");
    if client.is_none()
        && let Ok(value) = codex_adapter::CodexClient::connect(Default::default())
    {
        *client = Some(value);
    }
    let result = client.as_ref().map(|value| value.read_rate_limits());
    match result {
        Some(Ok(value)) => {
            let snapshot = snapshot_from_response(&value);
            *state.last.lock().expect("snapshot mutex poisoned") = Some(snapshot.clone());
            snapshot
        }
        Some(Err(error)) => state
            .last
            .lock()
            .expect("snapshot mutex poisoned")
            .clone()
            .map(|mut snapshot| {
                snapshot.stale = true;
                snapshot.message = Some(format!("更新失败：{error}"));
                snapshot
            })
            .unwrap_or(QuotaSnapshot {
                availability: "unavailable".into(),
                windows: vec![],
                plan_type: None,
                fetched_at: now_seconds(),
                stale: false,
                message: Some("额度暂不可用，请确认 Codex CLI 已登录".into()),
            }),
        None => QuotaSnapshot {
            availability: "unavailable".into(),
            windows: vec![],
            plan_type: None,
            fetched_at: now_seconds(),
            stale: false,
            message: Some("未检测到 Codex CLI".into()),
        },
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            client: Mutex::new(None),
            last: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![read_quota])
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let show = MenuItem::with_id(app, "show", "显示 Codex Pet", true, None::<&str>)?;
            let hide = MenuItem::with_id(app, "hide", "隐藏", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &hide, &quit])?;

            TrayIconBuilder::new()
                .icon(
                    app.default_window_icon()
                        .expect("应用图标应在 Tauri 配置中提供")
                        .clone(),
                )
                .tooltip("Codex Pet")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "hide" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.hide();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Codex Pet");
}
