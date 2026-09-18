use codex_adapter::{AdapterError, CodexClient, ServerNotification};
use quota::{QuotaRefreshCoordinator, QuotaSnapshot, spawn_notification_bridge};
use serde_json::Value;
use std::{
    sync::{Arc, Mutex, mpsc::{Receiver, Sender, channel}},
    thread,
    time::Duration,
};
use tauri::{
    Emitter, Manager,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};
use window_state::{
    MonitorContext, MonitorProvider, RuntimeWindowState, resolve_monitor, resolve_startup_monitor_for,
};

pub mod codex_adapter;
pub mod quota;
pub mod settings;
pub mod window_state;

struct TauriMonitorProvider<'a, R: tauri::Runtime> {
    window: &'a tauri::Window<R>,
}

struct TauriWebviewMonitorProvider<'a, R: tauri::Runtime> {
    window: &'a tauri::WebviewWindow<R>,
}

fn monitor_context(monitor: tauri::Monitor) -> MonitorContext {
    let work_area = monitor.work_area();
    MonitorContext::new(
        monitor.name().cloned(),
        settings::PhysicalRect::new(
            f64::from(work_area.position.x),
            f64::from(work_area.position.y),
            f64::from(work_area.size.width),
            f64::from(work_area.size.height),
        ),
        monitor.scale_factor(),
    )
}

impl<R: tauri::Runtime> MonitorProvider for TauriMonitorProvider<'_, R> {
    type Error = tauri::Error;

    fn current_monitor(&self) -> Result<Option<MonitorContext>, Self::Error> {
        self.window
            .current_monitor()
            .map(|monitor| monitor.map(monitor_context))
    }

    fn primary_monitor(&self) -> Result<Option<MonitorContext>, Self::Error> {
        self.window
            .primary_monitor()
            .map(|monitor| monitor.map(monitor_context))
    }

    fn available_monitors(&self) -> Result<Vec<MonitorContext>, Self::Error> {
        self.window
            .available_monitors()
            .map(|monitors| monitors.into_iter().map(monitor_context).collect())
    }
}

impl<R: tauri::Runtime> MonitorProvider for TauriWebviewMonitorProvider<'_, R> {
    type Error = tauri::Error;
    fn current_monitor(&self) -> Result<Option<MonitorContext>, Self::Error> {
        self.window
            .current_monitor()
            .map(|monitor| monitor.map(monitor_context))
    }
    fn primary_monitor(&self) -> Result<Option<MonitorContext>, Self::Error> {
        self.window
            .primary_monitor()
            .map(|monitor| monitor.map(monitor_context))
    }
    fn available_monitors(&self) -> Result<Vec<MonitorContext>, Self::Error> {
        self.window
            .available_monitors()
            .map(|monitors| monitors.into_iter().map(monitor_context).collect())
    }
}

struct WindowPersistenceState {
    runtime: Mutex<RuntimeWindowState>,
    writer: Option<Arc<Mutex<settings::DebouncedSettingsWriter<settings::WindowsReplace>>>>,
    scheduler: Mutex<Option<Sender<u64>>>,
}

impl WindowPersistenceState {
    fn new(
        runtime: RuntimeWindowState,
        target: std::path::PathBuf,
        writable: bool,
    ) -> Self {
        let writer = writable.then(|| {
            Arc::new(Mutex::new(settings::DebouncedSettingsWriter::new(
                settings::WindowsReplace,
                target,
            )))
        });
        let scheduler = writer.as_ref().map(|writer| {
            let (sender, receiver) = channel();
            spawn_persistence_scheduler(Arc::clone(writer), receiver);
            sender
        });
        Self {
            runtime: Mutex::new(runtime),
            writer,
            scheduler: Mutex::new(scheduler),
        }
    }

    fn schedule(&self, snapshot: settings::UiSettings) {
        let (Some(writer), Ok(scheduler)) = (&self.writer, self.scheduler.lock()) else {
            return;
        };
        let Some(scheduler) = scheduler.as_ref() else {
            return;
        };
        let token = match writer.lock() {
            Ok(mut writer) => writer.schedule(snapshot),
            Err(_) => return,
        };
        let _ = scheduler.send(token);
    }

    fn moved<R: tauri::Runtime>(
        &self,
        window: &tauri::Window<R>,
        position: tauri::PhysicalPosition<i32>,
    ) {
        let monitor = resolve_monitor(&TauriMonitorProvider { window });
        let snapshot = self.runtime.lock().ok().and_then(|mut runtime| {
            runtime.handle_moved_position(position.x, position.y, monitor.as_ref())
        });
        let Some(snapshot) = snapshot else {
            return;
        };
        self.schedule(snapshot);
    }
    fn shutdown(&self) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.coordinator_mut().begin_shutdown();
        }
        if let Ok(mut scheduler) = self.scheduler.lock() {
            scheduler.take();
        }
        if let Some(writer) = &self.writer
            && let Ok(mut writer) = writer.lock()
        {
            let _ = writer.shutdown();
        }
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.coordinator_mut().mark_stopped();
        }
    }
}

fn spawn_persistence_scheduler(
    writer: Arc<Mutex<settings::DebouncedSettingsWriter<settings::WindowsReplace>>>,
    receiver: Receiver<u64>,
) {
    thread::spawn(move || {
        while let Ok(mut token) = receiver.recv() {
            loop {
                match receiver.recv_timeout(Duration::from_millis(650)) {
                    Ok(newer_token) => token = newer_token,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        if let Ok(mut writer) = writer.lock() {
                            let _ = writer.flush_if_current(token);
                        }
                        break;
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        }
    });
}

struct AppState {
    coordinator: Arc<QuotaRefreshCoordinator>,
    shutdown: Box<dyn Fn() + Send + Sync>,
}

struct ClientSession<C> {
    client: Option<C>,
    stopped: bool,
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
    fn on_run_event(&self, event: &tauri::RunEvent) {
        if matches!(event, tauri::RunEvent::Exit) {
            (self.shutdown)();
        }
    }

    fn new<C: QuotaClient>(
        connect: impl Fn() -> Result<C, AdapterError> + Send + Sync + 'static,
        emit: impl Fn(&str, &QuotaSnapshot) + Send + Sync + 'static,
    ) -> Self {
        let session = Arc::new(Mutex::new(ClientSession::<C> {
            client: None,
            stopped: false,
        }));
        let emitter = Arc::new(Mutex::new(Some(emit)));
        let read_session = Arc::clone(&session);
        let active_emitter = Arc::clone(&emitter);
        let coordinator = Arc::new_cyclic(|coordinator| {
            let coordinator = coordinator.clone();
            QuotaRefreshCoordinator::new(
                move || {
                    let mut session = read_session.lock().expect("client mutex poisoned");
                    if session.stopped {
                        return Err(AdapterError::AlreadyShutdown);
                    }
                    let client = &mut session.client;
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
                move |event, snapshot| {
                    if let Some(emit) = active_emitter
                        .lock()
                        .expect("emitter mutex poisoned")
                        .as_ref()
                    {
                        emit(event, snapshot);
                    }
                },
            )
        });
        Self {
            coordinator,
            shutdown: Box::new(move || {
                // Tauri does not drop managed state before process::exit. Release
                // the captured AppHandle and wait for any in-flight read before
                // synchronously dropping the client and disabling reconnection.
                emitter.lock().expect("emitter mutex poisoned").take();
                let mut session = session.lock().expect("client mutex poisoned");
                session.stopped = true;
                session.client.take();
            }),
        }
    }
}

#[tauri::command]
fn read_quota(state: tauri::State<'_, AppState>) -> QuotaSnapshot {
    state.coordinator.refresh()
}

fn handle_run_event<R: tauri::Runtime>(app: &tauri::AppHandle<R>, event: tauri::RunEvent) {
    if matches!(event, tauri::RunEvent::Exit)
        && let Some(state) = app.try_state::<WindowPersistenceState>()
    {
        state.shutdown();
    }
    if let Some(state) = app.try_state::<AppState>() {
        state.on_run_event(&event);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![read_quota])
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let settings_dir = app.path().app_config_dir()?;
            std::fs::create_dir_all(&settings_dir)?;
            let settings_path = settings_dir.join("ui-settings.json");
            let loaded = settings::load_settings(&settings_path);
            let window = app
                .get_webview_window("main")
                .ok_or_else(|| std::io::Error::other("missing main window"))?;
            let monitor = resolve_startup_monitor_for(
                &TauriWebviewMonitorProvider { window: &window },
                &loaded.settings.window,
            );
            let initial_rect = monitor
                .as_ref()
                .map(|monitor| settings::initial_collapsed_rect(
                    &loaded,
                    monitor.work_area,
                    monitor.scale_factor,
                ))
                .unwrap_or(settings::PhysicalRect::new(
                    0.0,
                    0.0,
                    settings::CURRENT_COLLAPSED_WIDTH,
                    settings::CURRENT_COLLAPSED_HEIGHT,
                ));
            let saved_before_restore = loaded.settings.clone();
            let writable_settings = loaded.may_overwrite_source;
            let mut runtime = RuntimeWindowState::from_restored_settings(
                initial_rect,
                loaded.settings,
                monitor.as_ref(),
            );
            runtime
                .coordinator_mut()
                .expect_programmatic_move(initial_rect);
            let persist_startup_state = writable_settings
                && runtime.persisted_settings() != &saved_before_restore;
            let startup_snapshot = persist_startup_state.then(|| runtime.persisted_settings().clone());
            let persistence = WindowPersistenceState::new(runtime, settings_path, writable_settings);
            if let Some(snapshot) = startup_snapshot {
                persistence.schedule(snapshot);
            }
            app.manage(persistence);
            let _ = window.set_position(tauri::PhysicalPosition::new(
                initial_rect.x as i32,
                initial_rect.y as i32,
            ));
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

            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
            }

            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::Moved(position) => {
                if let Some(state) = window.app_handle().try_state::<WindowPersistenceState>() {
                    state.moved(window, *position);
                }
            }
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("error while building Codex Pet")
        .run(handle_run_event);
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

    #[test]
    fn exit_releases_the_client_and_breaks_the_managed_owner_emitter_cycle() {
        // AppHandle strongly owns AppManager, whose managed state owns AppState.
        // Model that exact cycle without creating a native Windows runtime.
        let manager = Arc::new(std::sync::OnceLock::<AppState>::new());
        let handle = Arc::clone(&manager);
        let manager_owner = Arc::downgrade(&manager);
        let (client, probe) = client(vec![Ok(response(25.0))]);
        let clients = Mutex::new(Some(client));
        let attempts = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&attempts);
        let app_state = AppState::new(
            move || {
                count.fetch_add(1, Ordering::SeqCst);
                clients
                    .lock()
                    .unwrap()
                    .take()
                    .ok_or(AdapterError::AlreadyShutdown)
            },
            move |_, _| {
                let _keep_manager_alive = &handle;
            },
        );
        assert!(manager.set(app_state).is_ok());
        let state = manager.get().unwrap();
        let coordinator = Arc::downgrade(&state.coordinator);
        assert_eq!(state.coordinator.refresh().windows[0].used_percent, 25.0);

        state.on_run_event(&tauri::RunEvent::Ready);
        assert_eq!(probe.drops.load(Ordering::SeqCst), 0);
        assert_eq!(Arc::strong_count(&manager), 2);

        state.on_run_event(&tauri::RunEvent::Exit);

        assert_eq!(
            probe.drops.load(Ordering::SeqCst),
            1,
            "the client must be dropped before the process exits"
        );
        assert_eq!(
            Arc::strong_count(&manager),
            1,
            "the emitter must release its captured owner handle"
        );
        assert!(state.coordinator.refresh().stale);
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "shutdown must prevent reconnects"
        );
        state.on_run_event(&tauri::RunEvent::Exit);
        assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
        drop(manager);
        assert!(manager_owner.upgrade().is_none());
        assert!(
            coordinator.upgrade().is_none(),
            "the managed-state/AppHandle cycle must be broken"
        );
    }

    #[test]
    fn exit_waits_for_an_active_read_before_returning_after_client_drop() {
        struct BlockingClient {
            inner: FakeClient,
            entered: mpsc::Sender<()>,
            release: Mutex<mpsc::Receiver<()>>,
        }
        impl QuotaClient for BlockingClient {
            fn read_rate_limits(&self) -> Result<Value, AdapterError> {
                self.entered.send(()).unwrap();
                self.release.lock().unwrap().recv_timeout(TIMEOUT).unwrap();
                self.inner.read_rate_limits()
            }
            fn take_notification_receiver(&self) -> Option<Receiver<ServerNotification>> {
                self.inner.take_notification_receiver()
            }
        }
        let (client, probe) = client(vec![Ok(response(25.0))]);
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let client = Mutex::new(Some(BlockingClient {
            inner: client,
            entered: entered_tx,
            release: Mutex::new(release_rx),
        }));
        let state = Arc::new(AppState::new(
            move || Ok(client.lock().unwrap().take().unwrap()),
            |_, _| {},
        ));
        let refresh_state = Arc::clone(&state);
        let read = thread::spawn(move || refresh_state.coordinator.refresh());
        entered_rx.recv_timeout(TIMEOUT).unwrap();
        let (exit_started_tx, exit_started_rx) = mpsc::channel();
        let (exited_tx, exited_rx) = mpsc::channel();
        let exit_state = Arc::clone(&state);
        let exit = thread::spawn(move || {
            exit_started_tx.send(()).unwrap();
            exit_state.on_run_event(&tauri::RunEvent::Exit);
            exited_tx.send(()).unwrap();
        });
        exit_started_rx.recv_timeout(TIMEOUT).unwrap();
        let early_exit = exited_rx.recv_timeout(Duration::from_millis(100));
        release_tx.send(()).unwrap();
        read.join().unwrap();
        exit.join().unwrap();
        assert!(
            early_exit.is_err(),
            "exit returned while the client was still reading"
        );
        assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
    }
}
