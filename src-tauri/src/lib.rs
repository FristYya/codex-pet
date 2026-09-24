use account::{AccountMachine, AccountStatus, LoginNotification};
use codex_adapter::{AdapterError, CodexClient, ServerNotification};
use quota::{QuotaRefreshCoordinator, QuotaSnapshot, spawn_notification_bridge};
use serde_json::Value;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, Sender, channel},
    },
    thread,
    time::Duration,
};
use tauri::{
    Emitter, Manager,
    menu::{CheckMenuItem, Menu, MenuItem},
    tray::TrayIconBuilder,
};
use tauri_plugin_autostart::{AutoLaunchManager, ManagerExt as AutostartManagerExt};
use tauri_plugin_opener::OpenerExt;
use window_state::{
    AUTOSTART_MENU_ID, AutostartControl, LOCKED_MENU_ID, MonitorContext, MonitorProvider,
    NativeWindow, RuntimeWindowState, TOPMOST_MENU_ID, TrayState, WindowOperationError,
    WindowOperationResult, WindowStateController, resolve_monitor, resolve_startup_monitor_for,
};

pub mod account;

const WINDOW_LOCKED_EVENT: &str = "window://locked";
const ACCOUNT_UPDATED_EVENT: &str = "account://updated";

pub mod codex_adapter;
pub mod quota;
pub mod runtime;
pub mod settings;
pub mod window_state;

pub(crate) fn diagnostic(message: impl std::fmt::Display) {
    if std::env::var_os("CODEX_PET_DIAGNOSTICS").is_some() {
        eprintln!("[codex-pet] {message}");
    }
}

pub(crate) fn adapter_error_kind(error: &AdapterError) -> &'static str {
    match error {
        AdapterError::ExecutableNotFound => "executable_not_found",
        AdapterError::VersionCheckFailed { .. } => "version_check_failed",
        AdapterError::SpawnFailed { .. } => "spawn_failed",
        AdapterError::Io { .. } => "io",
        AdapterError::Protocol { .. } => "protocol",
        AdapterError::MalformedJson => "malformed_json",
        AdapterError::Rpc { .. } => "rpc",
        AdapterError::Timeout { .. } => "timeout",
        AdapterError::UnexpectedChildExit => "unexpected_child_exit",
        AdapterError::StderrFlooding { .. } => "stderr_flooding",
        AdapterError::AlreadyShutdown => "already_shutdown",
    }
}

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

impl<R: tauri::Runtime> NativeWindow for tauri::Window<R> {
    fn show_without_activation(&self) -> WindowOperationResult {
        // Keep Tauri/Tao's visibility state synchronized with the HWND. Tao's
        // Windows implementation applies visibility without activation; calling
        // `set_focus` here would be the only explicit focus request.
        tauri::Window::show(self).map_err(|error| WindowOperationError::new(error.to_string()))
    }

    fn hide(&self) -> WindowOperationResult {
        tauri::Window::hide(self).map_err(|error| WindowOperationError::new(error.to_string()))
    }

    fn set_always_on_top(&self, enabled: bool) -> WindowOperationResult {
        tauri::Window::set_always_on_top(self, enabled)
            .map_err(|error| WindowOperationError::new(error.to_string()))
    }

    fn set_ignore_cursor_events(&self, enabled: bool) -> WindowOperationResult {
        tauri::Window::set_ignore_cursor_events(self, enabled)
            .map_err(|error| WindowOperationError::new(error.to_string()))
    }
}

impl<R: tauri::Runtime> NativeWindow for tauri::WebviewWindow<R> {
    fn show_without_activation(&self) -> WindowOperationResult {
        // See `Window` above: use Tauri's dispatcher so later hide/cursor
        // operations observe the same visibility state as the native HWND.
        tauri::WebviewWindow::show(self)
            .map_err(|error| WindowOperationError::new(error.to_string()))
    }

    fn hide(&self) -> WindowOperationResult {
        tauri::WebviewWindow::hide(self)
            .map_err(|error| WindowOperationError::new(error.to_string()))
    }

    fn set_always_on_top(&self, enabled: bool) -> WindowOperationResult {
        tauri::WebviewWindow::set_always_on_top(self, enabled)
            .map_err(|error| WindowOperationError::new(error.to_string()))
    }

    fn set_ignore_cursor_events(&self, enabled: bool) -> WindowOperationResult {
        tauri::WebviewWindow::set_ignore_cursor_events(self, enabled)
            .map_err(|error| WindowOperationError::new(error.to_string()))
    }
}

struct TauriTrayState<R: tauri::Runtime> {
    locked: CheckMenuItem<R>,
    topmost: CheckMenuItem<R>,
    autostart: CheckMenuItem<R>,
}

impl<R: tauri::Runtime> Clone for TauriTrayState<R> {
    fn clone(&self) -> Self {
        Self {
            locked: self.locked.clone(),
            topmost: self.topmost.clone(),
            autostart: self.autostart.clone(),
        }
    }
}

impl<R: tauri::Runtime> TauriTrayState<R> {
    fn item(&self, id: &str) -> WindowOperationResult<&CheckMenuItem<R>> {
        match id {
            LOCKED_MENU_ID => Ok(&self.locked),
            TOPMOST_MENU_ID => Ok(&self.topmost),
            AUTOSTART_MENU_ID => Ok(&self.autostart),
            _ => Err(WindowOperationError::new(format!(
                "unknown checkbox menu id: {id}"
            ))),
        }
    }
}

impl<R: tauri::Runtime> TrayState for TauriTrayState<R> {
    fn is_checked(&self, id: &str) -> WindowOperationResult<bool> {
        self.item(id)?
            .is_checked()
            .map_err(|error| WindowOperationError::new(error.to_string()))
    }

    fn set_checked(&self, id: &str, checked: bool) -> WindowOperationResult {
        self.item(id)?
            .set_checked(checked)
            .map_err(|error| WindowOperationError::new(error.to_string()))
    }
}

impl AutostartControl for AutoLaunchManager {
    fn is_enabled(&self) -> WindowOperationResult<bool> {
        AutoLaunchManager::is_enabled(self)
            .map_err(|error| WindowOperationError::new(error.to_string()))
    }

    fn enable(&self) -> WindowOperationResult {
        AutoLaunchManager::enable(self)
            .map_err(|error| WindowOperationError::new(error.to_string()))
    }

    fn disable(&self) -> WindowOperationResult {
        AutoLaunchManager::disable(self)
            .map_err(|error| WindowOperationError::new(error.to_string()))
    }
}

struct WindowPersistenceState {
    runtime: Mutex<RuntimeWindowState>,
    writer: Option<Arc<Mutex<settings::DebouncedSettingsWriter<settings::WindowsReplace>>>>,
    scheduler: Mutex<Option<Sender<u64>>>,
    tray_ready: AtomicBool,
}

impl WindowPersistenceState {
    fn new(runtime: RuntimeWindowState, target: std::path::PathBuf, writable: bool) -> Self {
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
            tray_ready: AtomicBool::new(false),
        }
    }

    fn transition<T>(
        &self,
        transition: impl FnOnce(&mut RuntimeWindowState) -> WindowOperationResult<T>,
    ) -> WindowOperationResult<T> {
        let (result, changed) = {
            let mut runtime = self
                .runtime
                .lock()
                .map_err(|_| WindowOperationError::new("window state mutex poisoned"))?;
            let before = runtime.persisted_settings().clone();
            let result = transition(&mut runtime);
            let after = runtime.persisted_settings().clone();
            (result, (before != after).then_some(after))
        };
        if let Some(snapshot) = changed {
            self.schedule(snapshot);
        }
        result
    }

    fn set_visible(
        &self,
        window: &impl NativeWindow,
        visible: bool,
    ) -> WindowOperationResult<settings::UiSettings> {
        self.transition(|runtime| WindowStateController::new(runtime).set_visible(window, visible))
    }

    fn set_locked_from_tray(
        &self,
        window: &impl NativeWindow,
        tray: &impl TrayState,
    ) -> WindowOperationResult<settings::UiSettings> {
        self.transition(|runtime| {
            WindowStateController::new(runtime).set_locked_from_tray(window, tray)
        })
    }

    fn set_always_on_top_from_tray(
        &self,
        window: &impl NativeWindow,
        tray: &impl TrayState,
    ) -> WindowOperationResult<settings::UiSettings> {
        self.transition(|runtime| {
            WindowStateController::new(runtime).set_always_on_top_from_tray(window, tray)
        })
    }

    fn set_autostart_from_tray(
        &self,
        autostart: &impl AutostartControl,
        tray: &impl TrayState,
    ) -> WindowOperationResult<settings::UiSettings> {
        self.transition(|runtime| {
            WindowStateController::new(runtime).set_autostart_from_tray(autostart, tray)
        })
    }

    fn reconcile_autostart(
        &self,
        autostart: &impl AutostartControl,
        tray: Option<&impl TrayState>,
    ) -> WindowOperationResult<settings::UiSettings> {
        self.transition(|runtime| {
            WindowStateController::new(runtime).reconcile_autostart(autostart, tray)
        })
    }

    fn restore_locked<W: NativeWindow, T: TrayState>(
        &self,
        window: &W,
        tray: Option<&T>,
    ) -> WindowOperationResult<settings::UiSettings> {
        self.transition(|runtime| WindowStateController::new(runtime).restore_locked(window, tray))
    }

    fn restore_always_on_top(
        &self,
        window: &impl NativeWindow,
        tray: &impl TrayState,
        always_on_top: bool,
    ) -> WindowOperationResult<settings::UiSettings> {
        self.transition(|runtime| {
            WindowStateController::new(runtime).set_always_on_top(window, tray, always_on_top)
        })
    }

    fn force_unlocked(
        &self,
        window: &impl NativeWindow,
    ) -> WindowOperationResult<settings::UiSettings> {
        self.transition(|runtime| WindowStateController::new(runtime).force_unlocked(window))
    }

    fn close_requested(
        &self,
        window: &impl NativeWindow,
        prevent_close: impl FnOnce(),
    ) -> WindowOperationResult<settings::UiSettings> {
        self.transition(|runtime| {
            WindowStateController::new(runtime).close_requested(window, prevent_close)
        })
    }

    fn mark_tray_ready(&self) {
        self.tray_ready.store(true, Ordering::Release);
    }

    fn is_tray_ready(&self) -> bool {
        self.tray_ready.load(Ordering::Acquire)
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
    reset: Box<dyn Fn() + Send + Sync>,
}

#[derive(Clone)]
struct AccountState {
    inner: Arc<AccountInner>,
}

struct AccountInner {
    machine: Mutex<AccountMachine>,
    client: Mutex<Option<CodexClient>>,
    connect: Box<dyn Fn() -> Result<CodexClient, AdapterError> + Send + Sync>,
}

trait AccountReader {
    fn has_chatgpt_login(&self) -> Result<bool, AdapterError>;
}

impl AccountReader for CodexClient {
    fn has_chatgpt_login(&self) -> Result<bool, AdapterError> {
        CodexClient::has_chatgpt_login(self)
    }
}

fn read_account_login_with_reconnect<C: AccountReader>(
    client: &mut Option<C>,
    connect: &dyn Fn() -> Result<C, AdapterError>,
    reconnected: &mut bool,
) -> Result<bool, AdapterError> {
    for attempt in 0..2 {
        if client.is_none() {
            *reconnected = true;
            match connect() {
                Ok(connected) => {
                    diagnostic("account client connected");
                    *client = Some(connected);
                }
                Err(error) if attempt == 0 => {
                    diagnostic(format!(
                        "account connect retry error={}",
                        adapter_error_kind(&error)
                    ));
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
        match client
            .as_ref()
            .expect("connected account client")
            .has_chatgpt_login()
        {
            Ok(logged_in) => return Ok(logged_in),
            Err(error) if account_error_requires_reconnect(&error) => {
                diagnostic(format!(
                    "account client reconnect after account/read error={}",
                    adapter_error_kind(&error)
                ));
                *reconnected = true;
                client.take();
                if attempt == 1 {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("two account reconnect attempts are bounded")
}

fn account_error_requires_reconnect(error: &AdapterError) -> bool {
    matches!(
        error,
        AdapterError::Io { .. }
            | AdapterError::Protocol { .. }
            | AdapterError::MalformedJson
            | AdapterError::UnexpectedChildExit
            | AdapterError::StderrFlooding { .. }
            | AdapterError::AlreadyShutdown
    )
}

fn account_updated_read_result(
    machine: &mut AccountMachine,
    account_read: Result<bool, AdapterError>,
    reconnected: bool,
) -> Option<(AccountStatus, bool)> {
    match account_read {
        Ok(false) if reconnected => {
            machine.active_login_id()?;
            machine.unavailable();
            Some((AccountStatus::Unavailable, false))
        }
        Ok(false) => None,
        Ok(true) => {
            machine.active_login_id()?;
            machine.reconcile(true);
            diagnostic("account transition LoggingIn->LoggedIn via account/updated");
            Some((AccountStatus::LoggedIn, true))
        }
        Err(error) if reconnected || account_error_requires_reconnect(&error) => {
            machine.active_login_id()?;
            diagnostic(format!(
                "account updated read unavailable error={}",
                adapter_error_kind(&error)
            ));
            machine.unavailable();
            Some((AccountStatus::Unavailable, false))
        }
        Err(error) => {
            diagnostic(format!(
                "account updated read temporarily failed error={}",
                adapter_error_kind(&error)
            ));
            None
        }
    }
}

fn login_completed_read_result(
    machine: &mut AccountMachine,
    account_read: Result<bool, AdapterError>,
) -> (AccountStatus, bool) {
    match account_read {
        Ok(true) => machine.reconcile(true),
        Ok(false) => machine.fail(),
        Err(error) => {
            diagnostic(format!(
                "account login confirmation read unavailable error={}",
                adapter_error_kind(&error)
            ));
            machine.unavailable();
        }
    }
    let status = machine.status();
    diagnostic(format!("account login notification transition={status:?}"));
    (status, status == AccountStatus::LoggedIn)
}

impl AccountState {
    fn new(
        connect: impl Fn() -> Result<CodexClient, AdapterError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            inner: Arc::new(AccountInner {
                machine: Mutex::new(AccountMachine::new()),
                client: Mutex::new(None),
                connect: Box::new(connect),
            }),
        }
    }
    fn status(&self) -> (AccountStatus, bool) {
        let mut client = self
            .inner
            .client
            .lock()
            .expect("account client mutex poisoned");
        let mut reconnected = false;
        let account_read =
            read_account_login_with_reconnect(&mut client, &*self.inner.connect, &mut reconnected);
        let mut machine = self
            .inner
            .machine
            .lock()
            .expect("account machine mutex poisoned");
        let previous = machine.status();
        match account_read {
            Ok(logged_in) if logged_in || previous != AccountStatus::LoggingIn => {
                machine.reconcile(logged_in)
            }
            Ok(false) if reconnected => machine.unavailable(),
            Ok(_) => {}
            Err(error) if previous != AccountStatus::LoggingIn || reconnected => {
                diagnostic(format!(
                    "account status unavailable error={}",
                    adapter_error_kind(&error)
                ));
                machine.unavailable();
            }
            Err(error) => diagnostic(format!(
                "account status unavailable during login error={}",
                adapter_error_kind(&error)
            )),
        }
        let status = machine.status();
        diagnostic(format!("account transition {:?}->{:?}", previous, status));
        (
            status,
            previous != AccountStatus::LoggedIn && status == AccountStatus::LoggedIn,
        )
    }
    fn start<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
    ) -> Result<(AccountStatus, Option<Receiver<ServerNotification>>), String> {
        let mut client = self
            .inner
            .client
            .lock()
            .map_err(|_| "account client mutex poisoned")?;
        if client.is_none() {
            *client = Some((self.inner.connect)().map_err(|error| error.to_string())?);
        }
        let login = client
            .as_ref()
            .expect("account client")
            .start_chatgpt_login()
            .map_err(|error| error.to_string())?;
        self.inner
            .machine
            .lock()
            .map_err(|_| "account machine mutex poisoned")?
            .begin(login.login_id.clone());
        if let Err(error) = app.opener().open_url(login.auth_url, None::<String>) {
            let _ = client
                .as_ref()
                .expect("account client")
                .cancel_chatgpt_login(&login.login_id);
            self.inner
                .machine
                .lock()
                .map_err(|_| "account machine mutex poisoned")?
                .fail();
            return Err(error.to_string());
        }
        let notifications = client
            .as_ref()
            .expect("account client")
            .take_notification_receiver();
        Ok((AccountStatus::LoggingIn, notifications))
    }
    fn cancel(&self) -> Result<AccountStatus, String> {
        let client = self
            .inner
            .client
            .lock()
            .map_err(|_| "account client mutex poisoned")?;
        let mut machine = self
            .inner
            .machine
            .lock()
            .map_err(|_| "account machine mutex poisoned")?;
        let Some(login_id) = machine.active_login_id().map(str::to_owned) else {
            return Ok(machine.status());
        };
        if let Some(client) = client.as_ref() {
            client
                .cancel_chatgpt_login(&login_id)
                .map_err(|error| error.to_string())?;
        }
        machine.cancel(&login_id);
        Ok(machine.status())
    }
    fn handle_login_notification(
        &self,
        notification: LoginNotification,
    ) -> Option<(AccountStatus, bool)> {
        if notification == LoginNotification::AccountUpdated {
            let active = self
                .inner
                .machine
                .lock()
                .expect("account machine mutex poisoned")
                .active_login_id()
                .is_some();
            if !active {
                return None;
            }
            let mut reconnected = false;
            let account_read = read_account_login_with_reconnect(
                &mut self
                    .inner
                    .client
                    .lock()
                    .expect("account client mutex poisoned"),
                &*self.inner.connect,
                &mut reconnected,
            );
            let mut machine = self
                .inner
                .machine
                .lock()
                .expect("account machine mutex poisoned");
            return account_updated_read_result(&mut machine, account_read, reconnected);
        }
        let login_id = notification.login_id()?;
        let accepted = {
            let mut machine = self
                .inner
                .machine
                .lock()
                .expect("account machine mutex poisoned");
            match &notification {
                LoginNotification::Completed { .. } => machine.complete(login_id, true),
                LoginNotification::Failed { .. } => machine.complete(login_id, false),
                LoginNotification::Cancelled { .. } => machine.cancel(login_id),
                LoginNotification::AccountUpdated => {
                    unreachable!("handled before login ID matching")
                }
            }
        };
        if !accepted {
            return None;
        }

        if !matches!(notification, LoginNotification::Completed { .. }) {
            return Some((
                self.inner
                    .machine
                    .lock()
                    .expect("account machine mutex poisoned")
                    .status(),
                false,
            ));
        }

        let mut reconnected = false;
        let account_read = read_account_login_with_reconnect(
            &mut self
                .inner
                .client
                .lock()
                .expect("account client mutex poisoned"),
            &*self.inner.connect,
            &mut reconnected,
        );
        let mut machine = self
            .inner
            .machine
            .lock()
            .expect("account machine mutex poisoned");
        Some(login_completed_read_result(&mut machine, account_read))
    }
}

struct ClientSession<C> {
    client: Option<C>,
    stopped: bool,
    generation: u64,
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
            generation: 0,
        }));
        let reset_session = Arc::clone(&session);
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
                    for attempt in 0..2 {
                        if session.client.is_none() {
                            let connected = match connect() {
                                Ok(connected) => connected,
                                Err(error) if attempt == 0 => {
                                    diagnostic(format!(
                                        "quota connect retry error={}",
                                        adapter_error_kind(&error)
                                    ));
                                    continue;
                                }
                                Err(error) => return Err(error),
                            };
                            session.generation += 1;
                            diagnostic(format!(
                                "quota client connected generation={}",
                                session.generation
                            ));
                            if let Some(notifications) = connected.take_notification_receiver()
                                && let Some(coordinator) = coordinator.upgrade()
                            {
                                diagnostic(format!(
                                    "quota notifications subscribed generation={}",
                                    session.generation
                                ));
                                spawn_notification_bridge(notifications, coordinator);
                            }
                            session.client = Some(connected);
                        }
                        diagnostic(format!("quota refresh generation={}", session.generation));
                        let result = session
                            .client
                            .as_ref()
                            .expect("connected client")
                            .read_rate_limits();
                        // A closed app-server cannot serve the current refresh.
                        // Reconnect once within the same coordinated flight.
                        if matches!(
                            &result,
                            Err(AdapterError::Io { .. }
                                | AdapterError::Protocol { .. }
                                | AdapterError::MalformedJson
                                | AdapterError::UnexpectedChildExit
                                | AdapterError::StderrFlooding { .. }
                                | AdapterError::AlreadyShutdown)
                        ) {
                            diagnostic(format!(
                                "quota client disconnected generation={} error={}",
                                session.generation,
                                adapter_error_kind(result.as_ref().unwrap_err())
                            ));
                            session.client.take();
                            if attempt == 0 {
                                continue;
                            }
                        }
                        diagnostic(format!(
                            "quota read generation={} ok={}",
                            session.generation,
                            result.is_ok()
                        ));
                        return result;
                    }
                    unreachable!("two reconnect attempts are bounded")
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
            reset: Box::new(move || {
                let mut session = reset_session.lock().expect("client mutex poisoned");
                if !session.stopped {
                    diagnostic(format!(
                        "quota client reset generation={}",
                        session.generation
                    ));
                    session.client.take();
                }
            }),
        }
    }
}

#[tauri::command]
fn read_quota(state: tauri::State<'_, AppState>, source: Option<String>) -> QuotaSnapshot {
    let source = match source.as_deref() {
        Some("manual") => "manual",
        _ => "automatic",
    };
    diagnostic(format!("quota command source={source} started"));
    let snapshot = state.coordinator.refresh();
    diagnostic(format!(
        "quota command source={source} completed stale={}",
        snapshot.stale
    ));
    snapshot
}

#[tauri::command]
fn read_account_status<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    account: tauri::State<'_, AccountState>,
    quota: tauri::State<'_, AppState>,
) -> AccountStatus {
    let (status, became_logged_in) = account.status();
    if became_logged_in {
        (quota.reset)();
    }
    let _ = app.emit(ACCOUNT_UPDATED_EVENT, status);
    status
}

#[tauri::command]
fn start_chatgpt_login<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    account: tauri::State<'_, AccountState>,
) -> Result<AccountStatus, String> {
    let (status, notifications) = account.start(&app)?;
    let account = (*account).clone();
    let notification_app = app.clone();
    if let Some(notifications) = notifications {
        thread::spawn(move || {
            while let Ok(notification) = notifications.recv() {
                let Some(notification) = LoginNotification::from_server_event(
                    &notification.method,
                    &notification.params,
                ) else {
                    continue;
                };
                let Some((status, became_logged_in)) =
                    account.handle_login_notification(notification)
                else {
                    continue;
                };
                if became_logged_in {
                    (notification_app.state::<AppState>().reset)();
                }
                let _ = notification_app.emit(ACCOUNT_UPDATED_EVENT, status);
            }
        });
    }
    let _ = app.emit(ACCOUNT_UPDATED_EVENT, status);
    Ok(status)
}

#[tauri::command]
fn cancel_chatgpt_login<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    account: tauri::State<'_, AccountState>,
) -> Result<AccountStatus, String> {
    let status = account.cancel()?;
    let _ = app.emit(ACCOUNT_UPDATED_EVENT, status);
    Ok(status)
}

#[tauri::command]
fn read_window_locked(state: tauri::State<'_, WindowPersistenceState>) -> Result<bool, String> {
    state
        .runtime
        .lock()
        .map(|runtime| runtime.persisted_settings().locked)
        .map_err(|_| "window state mutex poisoned".to_string())
}

fn handle_window_operation_error<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    context: &str,
    error: WindowOperationError,
) {
    eprintln!("{context}: {error}");
    if error.requires_shutdown() {
        app.exit(1);
    }
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
        .invoke_handler(tauri::generate_handler![read_quota, read_window_locked, read_account_status, start_chatgpt_login, cancel_chatgpt_login])
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name("Codex Pet")
                .build(),
        )
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
                .map(|monitor| {
                    settings::initial_collapsed_rect(
                        &loaded,
                        monitor.work_area,
                        monitor.scale_factor,
                    )
                })
                .unwrap_or(settings::PhysicalRect::new(
                    0.0,
                    0.0,
                    settings::CURRENT_COLLAPSED_WIDTH,
                    settings::CURRENT_COLLAPSED_HEIGHT,
                ));
            let saved_before_restore = loaded.settings.clone();
            let saved_locked = loaded.settings.locked;
            let saved_topmost = loaded.settings.always_on_top;
            let saved_visible = loaded.settings.visible;
            let saved_autostart = loaded.settings.autostart;
            let writable_settings = loaded.may_overwrite_source;
            let mut runtime = RuntimeWindowState::from_restored_settings(
                initial_rect,
                loaded.settings,
                monitor.as_ref(),
            );
            runtime
                .coordinator_mut()
                .expect_programmatic_move(initial_rect);
            let persist_startup_state =
                writable_settings && runtime.persisted_settings() != &saved_before_restore;
            let startup_snapshot =
                persist_startup_state.then(|| runtime.persisted_settings().clone());
            let persistence =
                WindowPersistenceState::new(runtime, settings_path, writable_settings);
            if let Some(snapshot) = startup_snapshot {
                persistence.schedule(snapshot);
            }
            app.manage(persistence);
            let _ = window.set_position(tauri::PhysicalPosition::new(
                initial_rect.x as i32,
                initial_rect.y as i32,
            ));
            let handle = app.handle().clone();
            let bundled_runtime = runtime::bundled_executable_for(&std::env::current_exe()?);
            let private_runtime_home = runtime::private_codex_home(&app.path().app_data_dir()?);
            diagnostic(format!("startup bundled_exe={} CODEX_HOME={}", bundled_runtime.display(), private_runtime_home.display()));
            let account_bundled_runtime = bundled_runtime.clone();
            let account_private_runtime_home = private_runtime_home.clone();
            app.manage(AppState::new(
                move || match CodexClient::connect(Default::default()) {
                    Ok(client) if client.has_chatgpt_login().unwrap_or(false) => {
                        diagnostic("quota selected runtime=system");
                        Ok(client)
                    },
                    Ok(client) => {
                        drop(client);
                        diagnostic(format!("quota selected runtime=bundled exe={} CODEX_HOME={}", bundled_runtime.display(), private_runtime_home.display()));
                        CodexClient::connect_with_runtime(
                            Default::default(),
                            bundled_runtime.clone(),
                            Some(private_runtime_home.clone()),
                        )
                    }
                    Err(error) => {
                        diagnostic(format!("quota system runtime unavailable error={}; selecting bundled exe={} CODEX_HOME={}", adapter_error_kind(&error), bundled_runtime.display(), private_runtime_home.display()));
                        CodexClient::connect_with_runtime(
                        Default::default(),
                        bundled_runtime.clone(),
                        Some(private_runtime_home.clone()),
                    )},
                },
                move |event, snapshot| {
                    let _ = handle.emit(event, snapshot);
                },
            ));
            app.manage(AccountState::new(move || {
                match CodexClient::connect(Default::default()) {
                    Ok(client) if client.has_chatgpt_login().unwrap_or(false) => {
                        diagnostic("account selected runtime=system");
                        Ok(client)
                    },
                    Ok(client) => {
                        drop(client);
                        diagnostic(format!("account selected runtime=bundled exe={} CODEX_HOME={}", account_bundled_runtime.display(), account_private_runtime_home.display()));
                        CodexClient::connect_with_runtime(Default::default(), account_bundled_runtime.clone(), Some(account_private_runtime_home.clone()))
                    }
                    Err(error) => {
                        diagnostic(format!("account system runtime unavailable error={}; selecting bundled exe={} CODEX_HOME={}", adapter_error_kind(&error), account_bundled_runtime.display(), account_private_runtime_home.display()));
                        CodexClient::connect_with_runtime(Default::default(), account_bundled_runtime.clone(), Some(account_private_runtime_home.clone()))
                    },
                }
            }));
            let tray_result = (|| {
                let show = MenuItem::with_id(app, "show", "显示 Codex Pet", true, None::<&str>)?;
                let hide = MenuItem::with_id(app, "hide", "隐藏", true, None::<&str>)?;
                let locked = CheckMenuItem::with_id(
                    app,
                    LOCKED_MENU_ID,
                    "锁定位置",
                    true,
                    saved_locked,
                    None::<&str>,
                )?;
                let topmost = CheckMenuItem::with_id(
                    app,
                    TOPMOST_MENU_ID,
                    "始终置顶",
                    true,
                    saved_topmost,
                    None::<&str>,
                )?;
                let autostart = CheckMenuItem::with_id(
                    app,
                    AUTOSTART_MENU_ID,
                    "开机自动启动",
                    true,
                    saved_autostart,
                    None::<&str>,
                )?;
                let refresh = MenuItem::with_id(app, "refresh", "刷新额度", true, None::<&str>)?;
                let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
                let menu = Menu::with_items(
                    app,
                    &[&show, &hide, &locked, &topmost, &autostart, &refresh, &quit],
                )?;
                let tray = TauriTrayState {
                    locked,
                    topmost,
                    autostart,
                };
                let tray_for_event = tray.clone();

                TrayIconBuilder::new()
                    .icon(
                        app.default_window_icon()
                            .expect("应用图标应在 Tauri 配置中提供")
                            .clone(),
                    )
                    .tooltip("Codex Pet")
                    .menu(&menu)
                    .on_menu_event(move |app, event| match event.id.as_ref() {
                        "show" => {
                            if let (Some(window), Some(state)) = (
                                app.get_webview_window("main"),
                                app.try_state::<WindowPersistenceState>(),
                            ) && let Err(error) = state.set_visible(&window, true)
                            {
                                handle_window_operation_error(app, "window show failed", error);
                            }
                        }
                        "hide" => {
                            if let (Some(window), Some(state)) = (
                                app.get_webview_window("main"),
                                app.try_state::<WindowPersistenceState>(),
                            ) && let Err(error) = state.set_visible(&window, false)
                            {
                                handle_window_operation_error(app, "window hide failed", error);
                            }
                        }
                        LOCKED_MENU_ID => {
                            if let (Some(window), Some(state)) = (
                                app.get_webview_window("main"),
                                app.try_state::<WindowPersistenceState>(),
                            ) {
                                match state.set_locked_from_tray(&window, &tray_for_event) {
                                    Ok(settings) => {
                                        let _ = app.emit(WINDOW_LOCKED_EVENT, settings.locked);
                                    }
                                    Err(error) => {
                                        handle_window_operation_error(
                                            app,
                                            "window lock transition failed",
                                            error,
                                        );
                                    }
                                }
                            }
                        }
                        TOPMOST_MENU_ID => {
                            if let (Some(window), Some(state)) = (
                                app.get_webview_window("main"),
                                app.try_state::<WindowPersistenceState>(),
                            ) && let Err(error) =
                                state.set_always_on_top_from_tray(&window, &tray_for_event)
                            {
                                handle_window_operation_error(
                                    app,
                                    "window topmost transition failed",
                                    error,
                                );
                            }
                        }
                        AUTOSTART_MENU_ID => {
                            if let Some(state) = app.try_state::<WindowPersistenceState>() {
                                let autostart = app.autolaunch();
                                if let Err(error) =
                                    state.set_autostart_from_tray(&*autostart, &tray_for_event)
                                {
                                    handle_window_operation_error(
                                        app,
                                        "autostart transition failed",
                                        error,
                                    );
                                }
                            }
                        }
                        "refresh" => {
                            if let Some(state) = app.try_state::<AppState>() {
                                state.coordinator.refresh();
                            }
                        }
                        "quit" => app.exit(0),
                        _ => {}
                    })
                    .build(app)?;

                Ok::<_, tauri::Error>(tray)
            })();

            let persistence = app.state::<WindowPersistenceState>();
            let autostart = app.autolaunch();
            if let Err(error) =
                persistence.reconcile_autostart(&*autostart, tray_result.as_ref().ok())
            {
                eprintln!("autostart state restore failed: {error}");
                if error.requires_shutdown() {
                    return Err(Box::new(error));
                }
            }
            match tray_result {
                Ok(tray) => {
                    persistence.mark_tray_ready();
                    if let Err(error) =
                        persistence.restore_always_on_top(&window, &tray, saved_topmost)
                    {
                        eprintln!("window topmost restore failed: {error}");
                        if error.requires_shutdown() {
                            return Err(Box::new(error));
                        }
                    }
                    match persistence.restore_locked(&window, Some(&tray)) {
                        Ok(settings) => {
                            let _ = app.emit(WINDOW_LOCKED_EVENT, settings.locked);
                        }
                        Err(error) => {
                            eprintln!("window lock restore failed: {error}");
                            if error.requires_shutdown() {
                                return Err(Box::new(error));
                            }
                            let _ = app.emit(WINDOW_LOCKED_EVENT, false);
                        }
                    }
                    if let Err(error) = persistence.set_visible(&window, saved_visible) {
                        eprintln!("window visibility restore failed: {error}");
                    }
                }
                Err(error) => {
                    eprintln!("tray initialization failed; keeping the window unlocked: {error}");
                    if let Err(unlock_error) = persistence.force_unlocked(&window) {
                        eprintln!("window unlock fallback failed: {unlock_error}");
                        return Err(Box::new(unlock_error));
                    }
                    if let Err(show_error) = persistence.set_visible(&window, true) {
                        eprintln!("window show fallback failed: {show_error}");
                        return Err(Box::new(show_error));
                    }
                }
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
                if let Some(state) = window.app_handle().try_state::<WindowPersistenceState>() {
                    if !state.is_tray_ready() {
                        return;
                    }
                    if let Err(error) = state.close_requested(window, || api.prevent_close()) {
                        eprintln!("window close-to-tray failed: {error}");
                    }
                }
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

    struct FakeAccountReader(Mutex<VecDeque<Result<bool, AdapterError>>>);

    impl AccountReader for FakeAccountReader {
        fn has_chatgpt_login(&self) -> Result<bool, AdapterError> {
            self.0
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected account/read")
        }
    }

    #[test]
    fn account_read_reconnects_after_a_stale_client_without_reporting_logged_out() {
        let old = FakeAccountReader(Mutex::new(VecDeque::from([Err(
            AdapterError::UnexpectedChildExit,
        )])));
        let fresh = FakeAccountReader(Mutex::new(VecDeque::from([Ok(true)])));
        let pool = Mutex::new(VecDeque::from([Ok(fresh)]));
        let mut client = Some(old);
        let mut reconnected = false;
        let logged_in = read_account_login_with_reconnect(
            &mut client,
            &|| pool.lock().unwrap().pop_front().unwrap(),
            &mut reconnected,
        );
        assert!(logged_in.unwrap());
        assert!(reconnected);
    }

    #[test]
    fn account_read_rpc_error_keeps_the_live_client_instead_of_reconnecting() {
        let old = FakeAccountReader(Mutex::new(VecDeque::from([Err(AdapterError::Rpc {
            code: -32000,
            message: "temporary account/read failure".into(),
            data: None,
        })])));
        let fresh = FakeAccountReader(Mutex::new(VecDeque::from([Ok(true)])));
        let pool = Mutex::new(VecDeque::from([Ok(fresh)]));
        let reconnect_calls = AtomicUsize::new(0);
        let mut client = Some(old);

        let mut reconnected = false;
        let result = read_account_login_with_reconnect(
            &mut client,
            &|| {
                reconnect_calls.fetch_add(1, Ordering::SeqCst);
                pool.lock().unwrap().pop_front().unwrap()
            },
            &mut reconnected,
        );

        assert!(matches!(result, Err(AdapterError::Rpc { .. })));
        assert_eq!(reconnect_calls.load(Ordering::SeqCst), 0);
        assert!(!reconnected);
        assert!(client.is_some());
    }

    #[test]
    fn account_connect_retries_a_transient_failure_and_preserves_login() {
        let fresh = FakeAccountReader(Mutex::new(VecDeque::from([Ok(true)])));
        let pool = Mutex::new(VecDeque::from([
            Err(AdapterError::UnexpectedChildExit),
            Ok(fresh),
        ]));
        let mut client = None;
        let mut reconnected = false;
        let logged_in = read_account_login_with_reconnect(
            &mut client,
            &|| pool.lock().unwrap().pop_front().unwrap(),
            &mut reconnected,
        );
        assert!(logged_in.unwrap());
        assert!(reconnected);
    }

    #[test]
    fn account_read_failure_is_an_error_not_a_logged_out_result() {
        let pool = Mutex::new(VecDeque::<Result<FakeAccountReader, AdapterError>>::from([
            Err(AdapterError::UnexpectedChildExit),
            Err(AdapterError::UnexpectedChildExit),
        ]));
        let mut client = None;
        let mut reconnected = false;
        let result = read_account_login_with_reconnect(
            &mut client,
            &|| pool.lock().unwrap().pop_front().unwrap(),
            &mut reconnected,
        );
        assert!(matches!(result, Err(AdapterError::UnexpectedChildExit)));
        assert!(reconnected);
    }

    #[test]
    fn account_runtime_unavailable_is_not_reported_as_a_login_failure() {
        let state = AccountState::new(|| Err(AdapterError::ExecutableNotFound));

        let (status, became_logged_in) = state.status();

        assert_eq!(status, AccountStatus::Unavailable);
        assert!(!became_logged_in);
    }

    #[test]
    fn account_updated_read_error_ends_login_wait_as_unavailable() {
        let mut machine = AccountMachine::new();
        machine.begin("login-1".into());

        let outcome = account_updated_read_result(
            &mut machine,
            Err(AdapterError::UnexpectedChildExit),
            false,
        );

        assert_eq!(outcome, Some((AccountStatus::Unavailable, false)));
        assert_eq!(machine.status(), AccountStatus::Unavailable);
    }

    #[test]
    fn account_updated_read_after_reconnect_ends_the_abandoned_login_wait() {
        let mut machine = AccountMachine::new();
        machine.begin("login-1".into());

        let outcome = account_updated_read_result(&mut machine, Ok(false), true);

        assert_eq!(outcome, Some((AccountStatus::Unavailable, false)));
        assert_eq!(machine.active_login_id(), None);
        assert_eq!(machine.status(), AccountStatus::Unavailable);
    }

    #[test]
    fn account_updated_transient_read_error_keeps_the_pending_login() {
        let mut machine = AccountMachine::new();
        machine.begin("login-1".into());

        let outcome = account_updated_read_result(
            &mut machine,
            Err(AdapterError::Timeout {
                method: "account/read".into(),
            }),
            false,
        );

        assert_eq!(outcome, None);
        assert_eq!(machine.active_login_id(), Some("login-1"));
        assert_eq!(machine.status(), AccountStatus::LoggingIn);
    }

    #[test]
    fn login_completion_read_error_is_not_reported_as_failed_authentication() {
        let mut machine = AccountMachine::new();
        machine.begin("login-1".into());
        assert!(machine.complete("login-1", true));

        let outcome =
            login_completed_read_result(&mut machine, Err(AdapterError::UnexpectedChildExit));

        assert_eq!(outcome, (AccountStatus::Unavailable, false));
        assert_eq!(machine.status(), AccountStatus::Unavailable);
    }

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
    fn login_success_resets_the_old_quota_client_and_restores_notifications() {
        let (old, old_probe) = client(vec![Ok(response(25.0))]);
        let (fresh, fresh_probe) = client(vec![Ok(response(40.0)), Ok(response(45.0))]);
        let (state, attempts, emitted) = state(vec![Ok(old), Ok(fresh)]);
        assert_eq!(state.coordinator.refresh().windows[0].used_percent, 25.0);
        emitted.recv_timeout(TIMEOUT).unwrap();

        (state.reset)();
        assert_eq!(old_probe.drops.load(Ordering::SeqCst), 1);
        assert_eq!(state.coordinator.refresh().windows[0].used_percent, 40.0);
        emitted.recv_timeout(TIMEOUT).unwrap();
        notify(&fresh_probe);
        let (_, updated) = emitted.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(updated["windows"][0]["usedPercent"], 45.0);
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
        assert_eq!(fresh_probe.takes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn notification_failure_uses_the_command_coordinators_last_success() {
        let (client, probe) = client(vec![
            Ok(response(25.0)),
            Err(AdapterError::UnexpectedChildExit),
        ]);
        let (state, _, emitted) = state(vec![Ok(client), Err(AdapterError::ExecutableNotFound)]);
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
    fn connection_failure_retries_once_within_the_same_quota_refresh() {
        let (client, probe) = client(vec![Ok(response(25.0))]);
        let (state, attempts, _) = state(vec![Err(AdapterError::ExecutableNotFound), Ok(client)]);
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
            let recovered = state.coordinator.refresh();
            assert!(!recovered.stale, "{error:?}");
            assert_eq!(recovered.windows[0].used_percent, 40.0);
            assert_eq!(first_probe.drops.load(Ordering::SeqCst), 1, "{error:?}");
            assert!(initial.windows[0].used_percent == 25.0);
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
