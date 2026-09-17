use codex_adapter::{AdapterError, CodexClient, ServerNotification};
use quota::{QuotaRefreshCoordinator, QuotaSnapshot, spawn_notification_bridge};
use serde_json::Value;
use std::sync::{Arc, Mutex, mpsc::Receiver};
use tauri::{
    Emitter, Manager,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};

pub mod codex_adapter;
pub mod quota;

struct AppState {
    coordinator: Arc<QuotaRefreshCoordinator>,
}

trait QuotaClient: Send + 'static {
    fn read_rate_limits(&self) -> Result<Value, AdapterError>;
    fn take_notification_receiver(&self) -> Option<Receiver<ServerNotification>>;
}

impl QuotaClient for CodexClient {
    fn read_rate_limits(&self) -> Result<Value, AdapterError> {
        CodexClient::read_rate_limits(self)
    }

    fn take_notification_receiver(&self) -> Option<Receiver<ServerNotification>> {
        CodexClient::take_notification_receiver(self)
    }
}

impl AppState {
    fn new<C: QuotaClient>(
        connect: impl Fn() -> Result<C, AdapterError> + Send + Sync + 'static,
        emit: impl Fn(&str, &QuotaSnapshot) + Send + Sync + 'static,
    ) -> Self {
        let client = Mutex::new(None::<C>);
        let coordinator = Arc::new_cyclic(|coordinator| {
            let coordinator = coordinator.clone();
            QuotaRefreshCoordinator::new(
                move || {
                    let mut client = client.lock().expect("client mutex poisoned");
                    if client.is_none() {
                        let connected = connect()?;
                        if let Some(notifications) = connected.take_notification_receiver()
                            && let Some(coordinator) = coordinator.upgrade()
                        {
                            spawn_notification_bridge(notifications, coordinator);
                        }
                        *client = Some(connected);
                    }
                    let result = client
                        .as_ref()
                        .expect("connected client")
                        .read_rate_limits();
                    // RPC errors and request deadlines leave the adapter usable.
                    // Transport/protocol failures are latched by the adapter and
                    // require a new client on the next coordinated refresh.
                    if matches!(
                        &result,
                        Err(AdapterError::Io { .. }
                            | AdapterError::Protocol { .. }
                            | AdapterError::MalformedJson
                            | AdapterError::UnexpectedChildExit
                            | AdapterError::StderrFlooding { .. }
                            | AdapterError::AlreadyShutdown)
                    ) {
                        client.take();
                    }
                    result
                },
                emit,
            )
        });
        Self { coordinator }
    }
}

#[tauri::command]
fn read_quota(state: tauri::State<'_, AppState>) -> QuotaSnapshot {
    state.coordinator.refresh()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![read_quota])
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let handle = app.handle().clone();
            app.manage(AppState::new(
                || CodexClient::connect(Default::default()),
                move |event, snapshot| {
                    let _ = handle.emit(event, snapshot);
                },
            ));
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

#[cfg(test)]
mod wiring_tests {
    use super::*;
    use crate::codex_adapter::{AdapterError, ServerNotification};
    use serde_json::{Value, json};
    use std::{
        collections::VecDeque,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread,
        time::{Duration, Instant},
    };

    const TIMEOUT: Duration = Duration::from_secs(3);

    struct FakeClient {
        results: Mutex<VecDeque<Result<Value, AdapterError>>>,
        notifications: Mutex<Option<mpsc::Receiver<ServerNotification>>>,
        takes: Arc<AtomicUsize>,
        drops: Arc<AtomicUsize>,
    }

    struct Probe {
        notifications: mpsc::Sender<ServerNotification>,
        takes: Arc<AtomicUsize>,
        drops: Arc<AtomicUsize>,
    }

    impl Drop for FakeClient {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl QuotaClient for FakeClient {
        fn read_rate_limits(&self) -> Result<Value, AdapterError> {
            self.results
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected extra RPC")
        }

        fn take_notification_receiver(&self) -> Option<mpsc::Receiver<ServerNotification>> {
            self.takes.fetch_add(1, Ordering::SeqCst);
            self.notifications.lock().unwrap().take()
        }
    }

    fn response(used: f64) -> Value {
        json!({"rateLimitsByLimitId": {"codex": {
            "primary": {"usedPercent": used, "windowDurationMins": 300, "resetsAt": 1900000000},
            "secondary": {"usedPercent": 42, "windowDurationMins": 10080, "resetsAt": 1900600000},
            "planType": "pro", "ordinaryUsageAllowed": true
        }}})
    }

    fn client(results: Vec<Result<Value, AdapterError>>) -> (FakeClient, Probe) {
        let (tx, rx) = mpsc::channel();
        let takes = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        (
            FakeClient {
                results: Mutex::new(results.into()),
                notifications: Mutex::new(Some(rx)),
                takes: Arc::clone(&takes),
                drops: Arc::clone(&drops),
            },
            Probe {
                notifications: tx,
                takes,
                drops,
            },
        )
    }

    fn state(
        connections: Vec<Result<FakeClient, AdapterError>>,
    ) -> (AppState, Arc<AtomicUsize>, mpsc::Receiver<(String, Value)>) {
        let attempts = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&attempts);
        let connections = Mutex::new(VecDeque::from(connections));
        let (tx, rx) = mpsc::channel();
        let state = AppState::new(
            move || {
                count.fetch_add(1, Ordering::SeqCst);
                connections
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("unexpected reconnect")
            },
            move |event, snapshot| {
                let _ = tx.send((event.to_owned(), serde_json::to_value(snapshot).unwrap()));
            },
        );
        (state, attempts, rx)
    }

    fn notify(probe: &Probe) {
        probe
            .notifications
            .send(ServerNotification {
                method: "account/rateLimits/updated".into(),
                params: json!({"primary": {"usedPercent": 99}}),
            })
            .unwrap();
    }

    #[test]
    fn connection_is_lazy_and_notification_receiver_is_taken_once() {
        let (client, probe) = client(vec![
            Ok(response(25.0)),
            Ok(response(30.0)),
            Ok(response(35.0)),
        ]);
        let (state, attempts, emitted) = state(vec![Ok(client)]);
        assert_eq!(attempts.load(Ordering::SeqCst), 0);
        assert_eq!(state.coordinator.refresh().windows[0].used_percent, 25.0);
        emitted.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(state.coordinator.refresh().windows[0].used_percent, 30.0);
        emitted.recv_timeout(TIMEOUT).unwrap();
        notify(&probe);
        let (event, snapshot) = emitted.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(event, "quota://updated");
        assert_eq!(snapshot["windows"][0]["usedPercent"], 35.0);
        assert_eq!(snapshot["windows"][0]["remainingPercent"], 65.0);
        assert_eq!(snapshot["windows"].as_array().unwrap().len(), 2);
        assert_eq!(snapshot["availability"], "allowed");
        assert_eq!(snapshot["planType"], "pro");
        assert!(snapshot["fetchedAt"].as_i64().unwrap() > 0);
        assert_eq!(snapshot["stale"], false);
        assert!(snapshot["message"].is_null());
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        assert_eq!(probe.takes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn notification_failure_uses_the_command_coordinators_last_success() {
        let (client, probe) = client(vec![
            Ok(response(25.0)),
            Err(AdapterError::UnexpectedChildExit),
        ]);
        let (state, _, emitted) = state(vec![Ok(client)]);
        let initial = serde_json::to_value(state.coordinator.refresh()).unwrap();
        emitted.recv_timeout(TIMEOUT).unwrap();
        notify(&probe);
        let (_, stale) = emitted.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(stale["windows"], initial["windows"]);
        assert_eq!(stale["fetchedAt"], initial["fetchedAt"]);
        assert_eq!(stale["stale"], true);
        assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn connection_failure_is_unavailable_and_the_next_refresh_retries() {
        let (client, probe) = client(vec![Ok(response(25.0))]);
        let (state, attempts, _) = state(vec![Err(AdapterError::ExecutableNotFound), Ok(client)]);
        let unavailable = state.coordinator.refresh();
        assert_eq!(unavailable.availability, "unavailable");
        assert!(unavailable.windows.is_empty());
        assert!(!unavailable.stale);
        let recovered = state.coordinator.refresh();
        assert_eq!(recovered.windows[0].used_percent, 25.0);
        assert!(!recovered.stale);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(probe.takes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn terminal_read_errors_drop_the_old_client_and_reconnect() {
        let errors = [
            AdapterError::UnexpectedChildExit,
            AdapterError::AlreadyShutdown,
            AdapterError::MalformedJson,
            AdapterError::Protocol {
                reason: "bad envelope".into(),
            },
            AdapterError::Io {
                operation: "read child stdout",
                reason: "broken pipe".into(),
            },
            AdapterError::StderrFlooding { limit: 32 },
        ];
        for error in errors {
            let (first, first_probe) = client(vec![Ok(response(25.0)), Err(error.clone())]);
            let (second, second_probe) = client(vec![Ok(response(40.0))]);
            let (state, attempts, _) = state(vec![Ok(first), Ok(second)]);
            let initial = state.coordinator.refresh();
            let stale = state.coordinator.refresh();
            assert!(stale.stale, "{error:?}");
            assert_eq!(stale.fetched_at, initial.fetched_at);
            assert_eq!(stale.windows[0].used_percent, 25.0);
            assert_eq!(first_probe.drops.load(Ordering::SeqCst), 1, "{error:?}");
            let recovered = state.coordinator.refresh();
            assert_eq!(recovered.windows[0].used_percent, 40.0);
            assert!(!recovered.stale);
            assert!(recovered.message.is_none());
            assert_eq!(attempts.load(Ordering::SeqCst), 2);
            assert_eq!(first_probe.takes.load(Ordering::SeqCst), 1);
            assert_eq!(second_probe.takes.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn reconnected_client_notifications_refresh_through_the_same_coordinator() {
        let (first, _) = client(vec![Err(AdapterError::UnexpectedChildExit)]);
        let (second, probe) = client(vec![Ok(response(40.0)), Ok(response(45.0))]);
        let (state, attempts, emitted) = state(vec![Ok(first), Ok(second)]);
        assert_eq!(state.coordinator.refresh().availability, "unavailable");
        emitted.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(state.coordinator.refresh().windows[0].used_percent, 40.0);
        emitted.recv_timeout(TIMEOUT).unwrap();
        notify(&probe);
        let (_, refreshed) = emitted.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(refreshed["windows"][0]["usedPercent"], 45.0);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(probe.takes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn rpc_errors_and_timeouts_keep_the_live_client() {
        for error in [
            AdapterError::Rpc {
                code: -32000,
                message: "temporarily unavailable".into(),
                data: None,
            },
            AdapterError::Timeout {
                method: "account/rateLimits/read".into(),
            },
        ] {
            let (client, probe) = client(vec![Ok(response(25.0)), Err(error), Ok(response(30.0))]);
            let (state, attempts, _) = state(vec![Ok(client)]);
            state.coordinator.refresh();
            assert!(state.coordinator.refresh().stale);
            assert_eq!(probe.drops.load(Ordering::SeqCst), 0);
            assert_eq!(state.coordinator.refresh().windows[0].used_percent, 30.0);
            assert_eq!(attempts.load(Ordering::SeqCst), 1);
            assert_eq!(probe.takes.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn dropping_app_state_releases_the_client_despite_the_notification_bridge() {
        let (client, probe) = client(vec![Ok(response(25.0))]);
        let (state, _, _) = state(vec![Ok(client)]);
        state.coordinator.refresh();
        let owner = Arc::downgrade(&state.coordinator);
        drop(state);
        assert!(owner.upgrade().is_none());
        let deadline = Instant::now() + TIMEOUT;
        while probe.drops.load(Ordering::SeqCst) == 0 {
            assert!(
                Instant::now() < deadline,
                "client leaked after application state dropped"
            );
            thread::yield_now();
        }
    }
}
