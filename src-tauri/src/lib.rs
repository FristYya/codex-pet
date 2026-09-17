use serde::Serialize;
use serde_json::Value;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{
    Manager,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};

pub mod codex_adapter;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct QuotaWindow {
    id: String,
    name: String,
    used_percent: f64,
    remaining_percent: f64,
    window_duration_mins: Option<f64>,
    resets_at: Option<f64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct QuotaSnapshot {
    availability: String,
    windows: Vec<QuotaWindow>,
    plan_type: Option<String>,
    fetched_at: i64,
    stale: bool,
    message: Option<String>,
}

struct AppState {
    client: Mutex<Option<codex_adapter::CodexClient>>,
    last: Mutex<Option<QuotaSnapshot>>,
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn as_number(value: Option<&Value>) -> Option<f64> {
    value.and_then(Value::as_f64)
}

fn snapshot_from_response(response: &Value) -> QuotaSnapshot {
    let bucket = response
        .get("rateLimitsByLimitId")
        .and_then(|v| v.get("codex"))
        .or_else(|| response.get("rateLimits"));
    let mut windows = Vec::new();
    for (id, key) in [("primary", "primary"), ("secondary", "secondary")] {
        if let Some(window) = bucket.and_then(|v| v.get(key)).and_then(Value::as_object) {
            let used = as_number(window.get("usedPercent"))
                .unwrap_or(0.0)
                .clamp(0.0, 100.0);
            let duration = as_number(window.get("windowDurationMins"));
            let name = match duration {
                Some(10080.0) => "Weekly".to_string(),
                Some(value) if value > 0.0 && value % 1440.0 == 0.0 => {
                    format!("{}D", value / 1440.0)
                }
                Some(value) if value > 0.0 && value % 60.0 == 0.0 => format!("{}H", value / 60.0),
                Some(value) => format!("{}m", value),
                None => "Unknown".to_string(),
            };
            windows.push(QuotaWindow {
                id: id.into(),
                name,
                used_percent: used,
                remaining_percent: 100.0 - used,
                window_duration_mins: duration,
                resets_at: as_number(window.get("resetsAt")),
            });
        }
    }
    let allowed = response
        .get("ordinaryUsageAllowed")
        .or_else(|| bucket.and_then(|v| v.get("ordinaryUsageAllowed")));
    let availability = match allowed.and_then(Value::as_bool) {
        Some(true) => "allowed",
        Some(false) => "blocked",
        None => "unknown",
    }
    .into();
    QuotaSnapshot {
        availability,
        windows,
        plan_type: response
            .get("planType")
            .or_else(|| bucket.and_then(|v| v.get("planType")))
            .and_then(Value::as_str)
            .map(str::to_owned),
        fetched_at: now_seconds(),
        stale: false,
        message: None,
    }
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
