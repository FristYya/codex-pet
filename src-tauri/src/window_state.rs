use crate::settings::{
    CollapsedRectState, MoveTracker, PhysicalRect, SavedWindow, UiSettings,
    current_collapsed_physical_size, save_collapsed_rect,
};
use std::{error::Error, fmt};

pub const LOCKED_MENU_ID: &str = "locked";
pub const TOPMOST_MENU_ID: &str = "topmost";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowOperationError {
    message: String,
    requires_shutdown: bool,
}

impl WindowOperationError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            requires_shutdown: false,
        }
    }

    pub fn fatal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            requires_shutdown: true,
        }
    }

    pub fn requires_shutdown(&self) -> bool {
        self.requires_shutdown
    }

    fn after_compensation(
        primary: Self,
        failures: impl IntoIterator<Item = (&'static str, Self)>,
    ) -> Self {
        let failures = failures
            .into_iter()
            .map(|(operation, error)| format!("{operation}: {error}"))
            .collect::<Vec<_>>();
        if failures.is_empty() {
            primary
        } else {
            Self::fatal(format!(
                "{primary}; compensation failed ({})",
                failures.join("; ")
            ))
        }
    }
}

impl fmt::Display for WindowOperationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for WindowOperationError {}

pub type WindowOperationResult<T = ()> = Result<T, WindowOperationError>;

pub trait NativeWindow {
    fn show_without_activation(&self) -> WindowOperationResult;
    fn hide(&self) -> WindowOperationResult;
    fn set_always_on_top(&self, enabled: bool) -> WindowOperationResult;
    fn set_ignore_cursor_events(&self, enabled: bool) -> WindowOperationResult;
}

pub trait TrayState {
    fn is_checked(&self, id: &str) -> WindowOperationResult<bool>;
    fn set_checked(&self, id: &str, checked: bool) -> WindowOperationResult;
}

#[derive(Clone, Debug, PartialEq)]
pub struct MonitorContext {
    pub name: Option<String>,
    pub work_area: PhysicalRect,
    pub scale_factor: f64,
}

impl MonitorContext {
    pub fn new(name: Option<String>, work_area: PhysicalRect, scale_factor: f64) -> Self {
        Self {
            name,
            work_area,
            scale_factor,
        }
    }
}

pub trait MonitorProvider {
    type Error;
    fn current_monitor(&self) -> Result<Option<MonitorContext>, Self::Error>;
    fn primary_monitor(&self) -> Result<Option<MonitorContext>, Self::Error>;
    fn available_monitors(&self) -> Result<Vec<MonitorContext>, Self::Error>;
}

pub fn resolve_monitor(provider: &impl MonitorProvider) -> Option<MonitorContext> {
    provider
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| provider.primary_monitor().ok().flatten())
        .or_else(|| {
            provider
                .available_monitors()
                .ok()
                .and_then(|monitors| monitors.into_iter().next())
        })
}

pub fn resolve_startup_monitor(provider: &impl MonitorProvider) -> Option<MonitorContext> {
    provider.primary_monitor().ok().flatten().or_else(|| {
        provider
            .available_monitors()
            .ok()
            .and_then(|monitors| monitors.into_iter().next())
    })
}

pub fn resolve_startup_monitor_for(
    provider: &impl MonitorProvider,
    saved_window: &SavedWindow,
) -> Option<MonitorContext> {
    let saved_key = saved_window.monitor_key.as_deref();
    let saved_name = saved_window.monitor_name.as_deref();
    if (saved_key.is_some() || saved_name.is_some())
        && let Ok(monitors) = provider.available_monitors()
        && let Some(monitor) = monitors.into_iter().find(|monitor| {
            let name = monitor.name.as_deref();
            saved_key == name || saved_name == name
        })
    {
        return Some(monitor);
    }
    resolve_startup_monitor(provider)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WindowMode {
    Collapsed,
    Expanded,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lifecycle {
    Running,
    ShuttingDown,
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MoveAction {
    Ignore,
    ConsumedProgrammaticMove,
    SchedulePersist,
}

pub struct WindowStateCoordinator {
    mode: WindowMode,
    lifecycle: Lifecycle,
    tracker: MoveTracker,
    collapsed: SavedWindow,
}

pub struct RuntimeWindowState {
    coordinator: WindowStateCoordinator,
    settings: UiSettings,
}

impl RuntimeWindowState {
    pub fn new(initial: PhysicalRect) -> Self {
        Self {
            coordinator: WindowStateCoordinator::new(initial),
            settings: UiSettings::default(),
        }
    }
    pub fn from_settings(initial: PhysicalRect, settings: UiSettings) -> Self {
        Self {
            coordinator: WindowStateCoordinator::new(initial),
            settings,
        }
    }
    pub fn from_restored_settings(
        initial: PhysicalRect,
        mut settings: UiSettings,
        monitor: Option<&MonitorContext>,
    ) -> Self {
        let coordinator = WindowStateCoordinator::from_restored(initial, monitor);
        settings.window = coordinator.collapsed().clone();
        Self {
            coordinator,
            settings,
        }
    }
    pub fn coordinator_mut(&mut self) -> &mut WindowStateCoordinator {
        &mut self.coordinator
    }
    pub fn persisted_settings(&self) -> &UiSettings {
        &self.settings
    }
    pub fn set_locked(&mut self, locked: bool) -> UiSettings {
        self.settings.locked = locked;
        self.settings.clone()
    }
    pub fn set_visible(&mut self, visible: bool) -> UiSettings {
        self.settings.visible = visible;
        self.settings.clone()
    }
    pub fn set_always_on_top(&mut self, always_on_top: bool) -> UiSettings {
        self.settings.always_on_top = always_on_top;
        self.settings.clone()
    }
    pub fn handle_moved(
        &mut self,
        rect: PhysicalRect,
        monitor: Option<&MonitorContext>,
    ) -> Option<UiSettings> {
        if self.coordinator.handle_moved(rect, monitor) != MoveAction::SchedulePersist {
            return None;
        }
        self.settings.window = self.coordinator.collapsed().clone();
        Some(self.settings.clone())
    }
    pub fn handle_moved_position(
        &mut self,
        x: i32,
        y: i32,
        monitor: Option<&MonitorContext>,
    ) -> Option<UiSettings> {
        if self.coordinator.handle_moved_position(x, y, monitor) != MoveAction::SchedulePersist {
            return None;
        }
        self.settings.window = self.coordinator.collapsed().clone();
        Some(self.settings.clone())
    }
}

pub struct WindowStateController<'a> {
    state: &'a mut RuntimeWindowState,
}

impl<'a> WindowStateController<'a> {
    pub fn new(state: &'a mut RuntimeWindowState) -> Self {
        Self { state }
    }

    pub fn set_locked(
        &mut self,
        window: &impl NativeWindow,
        tray: &impl TrayState,
        locked: bool,
    ) -> WindowOperationResult<UiSettings> {
        let previous = self.state.settings.locked;
        if let Err(error) = window.set_ignore_cursor_events(locked) {
            let failures = tray
                .set_checked(LOCKED_MENU_ID, previous)
                .err()
                .map(|error| ("restore lock checkbox", error));
            return Err(WindowOperationError::after_compensation(error, failures));
        }
        if let Err(error) = tray.set_checked(LOCKED_MENU_ID, locked) {
            let failures = [
                window
                    .set_ignore_cursor_events(previous)
                    .err()
                    .map(|error| ("restore cursor passthrough", error)),
                tray.set_checked(LOCKED_MENU_ID, previous)
                    .err()
                    .map(|error| ("restore lock checkbox", error)),
            ]
            .into_iter()
            .flatten();
            return Err(WindowOperationError::after_compensation(error, failures));
        }
        Ok(self.state.set_locked(locked))
    }

    pub fn set_locked_from_tray(
        &mut self,
        window: &impl NativeWindow,
        tray: &impl TrayState,
    ) -> WindowOperationResult<UiSettings> {
        let locked = match tray.is_checked(LOCKED_MENU_ID) {
            Ok(locked) => locked,
            Err(error) => {
                let failures = tray
                    .set_checked(LOCKED_MENU_ID, self.state.settings.locked)
                    .err()
                    .map(|error| ("restore lock checkbox", error));
                return Err(WindowOperationError::after_compensation(error, failures));
            }
        };
        self.set_locked(window, tray, locked)
    }

    pub fn restore_locked<W: NativeWindow, T: TrayState>(
        &mut self,
        window: &W,
        tray: Option<&T>,
    ) -> WindowOperationResult<UiSettings> {
        let saved_locked = self.state.settings.locked;
        let Some(tray) = tray else {
            window.set_ignore_cursor_events(false).map_err(|error| {
                WindowOperationError::fatal(format!(
                    "cannot restore lock before the tray is ready; unlock failed: {error}"
                ))
            })?;
            self.state.set_locked(false);
            return Err(WindowOperationError::new(
                "cannot restore lock before the tray is ready",
            ));
        };

        if !saved_locked {
            window.set_ignore_cursor_events(false)?;
            tray.set_checked(LOCKED_MENU_ID, false)?;
            return Ok(self.state.settings.clone());
        }

        if let Err(error) = tray.set_checked(LOCKED_MENU_ID, true) {
            let failures = [
                window
                    .set_ignore_cursor_events(false)
                    .err()
                    .map(|error| ("disable cursor passthrough", error)),
                tray.set_checked(LOCKED_MENU_ID, false)
                    .err()
                    .map(|error| ("clear lock checkbox", error)),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
            if failures.is_empty() {
                self.state.set_locked(false);
            }
            return Err(WindowOperationError::after_compensation(error, failures));
        }
        if let Err(error) = window.set_ignore_cursor_events(true) {
            let failures = [
                window
                    .set_ignore_cursor_events(false)
                    .err()
                    .map(|error| ("disable cursor passthrough", error)),
                tray.set_checked(LOCKED_MENU_ID, false)
                    .err()
                    .map(|error| ("clear lock checkbox", error)),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
            if failures.is_empty() {
                self.state.set_locked(false);
            }
            return Err(WindowOperationError::after_compensation(error, failures));
        }
        Ok(self.state.settings.clone())
    }

    pub fn force_unlocked(
        &mut self,
        window: &impl NativeWindow,
    ) -> WindowOperationResult<UiSettings> {
        window.set_ignore_cursor_events(false)?;
        Ok(self.state.set_locked(false))
    }

    pub fn set_always_on_top(
        &mut self,
        window: &impl NativeWindow,
        tray: &impl TrayState,
        always_on_top: bool,
    ) -> WindowOperationResult<UiSettings> {
        let previous = self.state.settings.always_on_top;
        if let Err(error) = window.set_always_on_top(always_on_top) {
            let failures = tray
                .set_checked(TOPMOST_MENU_ID, previous)
                .err()
                .map(|error| ("restore topmost checkbox", error));
            return Err(WindowOperationError::after_compensation(error, failures));
        }
        if let Err(error) = tray.set_checked(TOPMOST_MENU_ID, always_on_top) {
            let failures = [
                window
                    .set_always_on_top(previous)
                    .err()
                    .map(|error| ("restore native topmost", error)),
                tray.set_checked(TOPMOST_MENU_ID, previous)
                    .err()
                    .map(|error| ("restore topmost checkbox", error)),
            ]
            .into_iter()
            .flatten();
            return Err(WindowOperationError::after_compensation(error, failures));
        }
        Ok(self.state.set_always_on_top(always_on_top))
    }

    pub fn set_always_on_top_from_tray(
        &mut self,
        window: &impl NativeWindow,
        tray: &impl TrayState,
    ) -> WindowOperationResult<UiSettings> {
        let always_on_top = match tray.is_checked(TOPMOST_MENU_ID) {
            Ok(always_on_top) => always_on_top,
            Err(error) => {
                let failures = tray
                    .set_checked(TOPMOST_MENU_ID, self.state.settings.always_on_top)
                    .err()
                    .map(|error| ("restore topmost checkbox", error));
                return Err(WindowOperationError::after_compensation(error, failures));
            }
        };
        self.set_always_on_top(window, tray, always_on_top)
    }

    pub fn set_visible(
        &mut self,
        window: &impl NativeWindow,
        visible: bool,
    ) -> WindowOperationResult<UiSettings> {
        if visible {
            window.show_without_activation()?;
        } else {
            window.hide()?;
        }
        Ok(self.state.set_visible(visible))
    }

    pub fn close_requested(
        &mut self,
        window: &impl NativeWindow,
        prevent_close: impl FnOnce(),
    ) -> WindowOperationResult<UiSettings> {
        prevent_close();
        window.hide()?;
        Ok(self.state.set_visible(false))
    }
}

impl WindowStateCoordinator {
    pub fn new(initial: PhysicalRect) -> Self {
        let state = CollapsedRectState::new(initial, None, None, 1.0);
        let collapsed = state.saved_window();
        let tracker = MoveTracker::new(state);
        Self {
            mode: WindowMode::Collapsed,
            lifecycle: Lifecycle::Running,
            collapsed,
            tracker,
        }
    }
    pub fn from_restored(initial: PhysicalRect, monitor: Option<&MonitorContext>) -> Self {
        let mut coordinator = Self::new(initial);
        if let Some(monitor) = monitor {
            coordinator.collapsed = saved_window_for_monitor(initial, monitor);
        }
        coordinator
    }
    pub fn set_mode(&mut self, mode: WindowMode) {
        self.mode = mode;
    }
    pub fn expect_programmatic_move(&mut self, target: PhysicalRect) {
        self.tracker.expect_programmatic_move(target);
    }
    pub fn begin_shutdown(&mut self) {
        if self.lifecycle == Lifecycle::Running {
            self.lifecycle = Lifecycle::ShuttingDown;
        }
    }
    pub fn mark_stopped(&mut self) {
        self.lifecycle = Lifecycle::Stopped;
    }
    pub fn collapsed(&self) -> &SavedWindow {
        &self.collapsed
    }
    pub fn handle_moved_position(
        &mut self,
        x: i32,
        y: i32,
        monitor: Option<&MonitorContext>,
    ) -> MoveAction {
        let Some(monitor) = monitor else {
            return MoveAction::Ignore;
        };
        let size = current_collapsed_physical_size(monitor.scale_factor);
        self.handle_moved(
            PhysicalRect::new(f64::from(x), f64::from(y), size.0, size.1),
            Some(monitor),
        )
    }
    pub fn handle_moved(
        &mut self,
        rect: PhysicalRect,
        monitor: Option<&MonitorContext>,
    ) -> MoveAction {
        if self.lifecycle != Lifecycle::Running || self.mode == WindowMode::Expanded {
            return MoveAction::Ignore;
        }
        if !self.tracker.on_moved(rect, false) {
            return MoveAction::ConsumedProgrammaticMove;
        }
        let Some(monitor) = monitor else {
            return MoveAction::Ignore;
        };
        self.collapsed = saved_window_for_monitor(rect, monitor);
        MoveAction::SchedulePersist
    }
}

fn saved_window_for_monitor(rect: PhysicalRect, monitor: &MonitorContext) -> SavedWindow {
    let mut saved = save_collapsed_rect(rect, monitor.work_area, monitor.scale_factor);
    saved.monitor_key = monitor.name.clone();
    saved.monitor_name = monitor.name.clone();
    saved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::PhysicalRect;
    use std::{
        cell::{Cell, RefCell},
        collections::HashMap,
        rc::Rc,
    };

    #[derive(Default)]
    struct FakeWindow {
        calls: Rc<RefCell<Vec<&'static str>>>,
        fail_ignore: Cell<bool>,
        fail_ignore_on_call: Cell<usize>,
        ignore_calls: Cell<usize>,
        fail_topmost: Cell<bool>,
        fail_hide: Cell<bool>,
        ignored: Cell<bool>,
        topmost: Cell<bool>,
    }

    impl NativeWindow for FakeWindow {
        fn show_without_activation(&self) -> WindowOperationResult {
            self.calls.borrow_mut().push("show");
            Ok(())
        }

        fn hide(&self) -> WindowOperationResult {
            self.calls.borrow_mut().push("hide");
            if self.fail_hide.get() {
                Err(WindowOperationError::new("hide failed"))
            } else {
                Ok(())
            }
        }

        fn set_always_on_top(&self, enabled: bool) -> WindowOperationResult {
            self.calls.borrow_mut().push(if enabled {
                "topmost:true"
            } else {
                "topmost:false"
            });
            if self.fail_topmost.replace(false) {
                Err(WindowOperationError::new("topmost failed"))
            } else {
                self.topmost.set(enabled);
                Ok(())
            }
        }

        fn set_ignore_cursor_events(&self, enabled: bool) -> WindowOperationResult {
            let call = self.ignore_calls.get() + 1;
            self.ignore_calls.set(call);
            self.calls.borrow_mut().push(if enabled {
                "ignore:true"
            } else {
                "ignore:false"
            });
            if self.fail_ignore.replace(false) || self.fail_ignore_on_call.get() == call {
                Err(WindowOperationError::new("ignore failed"))
            } else {
                self.ignored.set(enabled);
                Ok(())
            }
        }
    }

    #[derive(Default)]
    struct FakeTray {
        checked: RefCell<HashMap<String, bool>>,
        fail_read: Cell<bool>,
        fail_next_write: Cell<bool>,
    }

    impl FakeTray {
        fn with_checked(id: &str, checked: bool) -> Self {
            let tray = Self::default();
            tray.checked.borrow_mut().insert(id.into(), checked);
            tray
        }
    }

    impl TrayState for FakeTray {
        fn is_checked(&self, id: &str) -> WindowOperationResult<bool> {
            if self.fail_read.get() {
                Err(WindowOperationError::new("checkbox read failed"))
            } else {
                Ok(*self.checked.borrow().get(id).unwrap_or(&false))
            }
        }

        fn set_checked(&self, id: &str, checked: bool) -> WindowOperationResult {
            if self.fail_next_write.replace(false) {
                Err(WindowOperationError::new("checkbox write failed"))
            } else {
                self.checked.borrow_mut().insert(id.into(), checked);
                Ok(())
            }
        }
    }

    struct FakeMonitors {
        current: Result<Option<MonitorContext>, ()>,
        primary: Result<Option<MonitorContext>, ()>,
        available: Result<Vec<MonitorContext>, ()>,
    }

    impl MonitorProvider for FakeMonitors {
        type Error = ();
        fn current_monitor(&self) -> Result<Option<MonitorContext>, Self::Error> {
            self.current.clone()
        }
        fn primary_monitor(&self) -> Result<Option<MonitorContext>, Self::Error> {
            self.primary.clone()
        }
        fn available_monitors(&self) -> Result<Vec<MonitorContext>, Self::Error> {
            self.available.clone()
        }
    }

    #[test]
    fn resolver_falls_back_from_current_to_primary_then_first_without_panicking() {
        let primary = MonitorContext::new(
            Some("primary".into()),
            PhysicalRect::new(0.0, 0.0, 1920.0, 1040.0),
            1.0,
        );
        let first = MonitorContext::new(
            Some("first".into()),
            PhysicalRect::new(-1920.0, 0.0, 1920.0, 1040.0),
            1.25,
        );
        let provider = FakeMonitors {
            current: Err(()),
            primary: Ok(Some(primary.clone())),
            available: Ok(vec![first]),
        };
        assert_eq!(resolve_monitor(&provider), Some(primary));
    }

    #[test]
    fn resolver_uses_first_available_or_returns_unavailable_after_errors() {
        let first = MonitorContext::new(
            Some("first".into()),
            PhysicalRect::new(-1920.0, 0.0, 1920.0, 1040.0),
            1.25,
        );
        let provider = FakeMonitors {
            current: Ok(None),
            primary: Ok(None),
            available: Ok(vec![first.clone()]),
        };
        assert_eq!(resolve_monitor(&provider), Some(first));

        let unavailable = FakeMonitors {
            current: Err(()),
            primary: Err(()),
            available: Err(()),
        };
        assert_eq!(resolve_monitor(&unavailable), None);
    }

    #[test]
    fn startup_resolver_prefers_the_primary_monitor_over_the_windows_current_monitor() {
        let current = MonitorContext::new(
            Some("secondary".into()),
            PhysicalRect::new(-1920.0, 0.0, 1920.0, 1040.0),
            1.25,
        );
        let primary = MonitorContext::new(
            Some("primary".into()),
            PhysicalRect::new(0.0, 0.0, 1920.0, 1040.0),
            1.0,
        );
        let provider = FakeMonitors {
            current: Ok(Some(current)),
            primary: Ok(Some(primary.clone())),
            available: Ok(vec![]),
        };

        assert_eq!(resolve_startup_monitor(&provider), Some(primary));
    }

    #[test]
    fn startup_resolver_restores_the_saved_monitor_before_falling_back_to_primary() {
        let primary = MonitorContext::new(
            Some("primary".into()),
            PhysicalRect::new(0.0, 0.0, 1920.0, 1040.0),
            1.0,
        );
        let saved = MonitorContext::new(
            Some("secondary".into()),
            PhysicalRect::new(-1920.0, 0.0, 1920.0, 1040.0),
            1.25,
        );
        let provider = FakeMonitors {
            current: Ok(None),
            primary: Ok(Some(primary)),
            available: Ok(vec![saved.clone()]),
        };
        let saved_window = SavedWindow {
            monitor_key: Some("secondary".into()),
            ..UiSettings::default().window
        };

        assert_eq!(
            resolve_startup_monitor_for(&provider, &saved_window),
            Some(saved)
        );
    }

    #[test]
    fn coordinator_schedules_only_running_collapsed_user_moves() {
        let initial = PhysicalRect::new(0.0, 0.0, 164.0, 154.0);
        let monitor = MonitorContext::new(
            Some("primary".into()),
            PhysicalRect::new(-1920.0, 40.0, 1920.0, 1040.0),
            1.25,
        );
        let mut coordinator = WindowStateCoordinator::new(initial);

        let action = coordinator.handle_moved(
            PhysicalRect::new(-1800.0, 140.0, 205.0, 193.0),
            Some(&monitor),
        );

        assert_eq!(action, MoveAction::SchedulePersist);
        assert_eq!(coordinator.collapsed().x, 96.0);
        coordinator.set_mode(WindowMode::Expanded);
        assert_eq!(
            coordinator.handle_moved(
                PhysicalRect::new(-1700.0, 140.0, 205.0, 193.0),
                Some(&monitor)
            ),
            MoveAction::Ignore
        );
        coordinator.begin_shutdown();
        assert_eq!(
            coordinator.handle_moved(
                PhysicalRect::new(-1600.0, 140.0, 205.0, 193.0),
                Some(&monitor)
            ),
            MoveAction::Ignore
        );
    }

    #[test]
    fn coordinator_consumes_expected_programmatic_move_once_then_accepts_user_move() {
        let monitor = MonitorContext::new(None, PhysicalRect::new(0.0, 0.0, 1920.0, 1040.0), 1.0);
        let mut coordinator =
            WindowStateCoordinator::new(PhysicalRect::new(0.0, 0.0, 164.0, 154.0));
        coordinator.expect_programmatic_move(PhysicalRect::new(100.0, 100.0, 164.0, 154.0));

        assert_eq!(
            coordinator.handle_moved(
                PhysicalRect::new(100.0, 100.0, 164.0, 154.0),
                Some(&monitor)
            ),
            MoveAction::ConsumedProgrammaticMove
        );
        assert_eq!(
            coordinator.handle_moved(
                PhysicalRect::new(200.0, 100.0, 164.0, 154.0),
                Some(&monitor)
            ),
            MoveAction::SchedulePersist
        );
    }

    #[test]
    fn moved_position_uses_monitor_scale_to_build_the_physical_collapsed_rect() {
        let monitor = MonitorContext::new(None, PhysicalRect::new(0.0, 0.0, 1920.0, 1040.0), 1.25);
        let mut coordinator =
            WindowStateCoordinator::new(PhysicalRect::new(0.0, 0.0, 164.0, 154.0));
        assert_eq!(
            coordinator.handle_moved_position(100, 200, Some(&monitor)),
            MoveAction::SchedulePersist
        );
        assert_eq!(coordinator.collapsed().x, 80.0);
        assert_eq!(coordinator.collapsed().y, 160.0);
    }

    #[test]
    fn move_after_a_scale_change_saves_the_new_monitor_scale_and_relative_offset() {
        let monitor = MonitorContext::new(
            Some("secondary".into()),
            PhysicalRect::new(-1920.0, 40.0, 1920.0, 1040.0),
            1.5,
        );
        let mut coordinator =
            WindowStateCoordinator::new(PhysicalRect::new(0.0, 0.0, 164.0, 154.0));

        assert_eq!(
            coordinator.handle_moved_position(-1770, 160, Some(&monitor)),
            MoveAction::SchedulePersist
        );
        assert_eq!(coordinator.collapsed().x, 100.0);
        assert_eq!(coordinator.collapsed().y, 80.0);
        assert_eq!(coordinator.collapsed().scale_factor, 1.5);
        assert_eq!(
            coordinator.collapsed().monitor_key.as_deref(),
            Some("secondary")
        );
    }

    #[test]
    fn runtime_state_returns_a_persisted_snapshot_only_for_user_collapsed_moves() {
        let monitor = MonitorContext::new(None, PhysicalRect::new(0.0, 0.0, 1920.0, 1040.0), 1.0);
        let mut state = RuntimeWindowState::new(PhysicalRect::new(0.0, 0.0, 164.0, 154.0));

        let saved = state.handle_moved(
            PhysicalRect::new(100.0, 200.0, 164.0, 154.0),
            Some(&monitor),
        );
        assert_eq!(saved.unwrap().window.x, 100.0);
        state.coordinator_mut().set_mode(WindowMode::Expanded);
        assert!(
            state
                .handle_moved(
                    PhysicalRect::new(200.0, 200.0, 164.0, 154.0),
                    Some(&monitor)
                )
                .is_none()
        );
    }

    #[test]
    fn restored_runtime_uses_the_clamped_rect_as_its_persisted_snapshot() {
        let monitor = MonitorContext::new(
            Some("primary".into()),
            PhysicalRect::new(-1920.0, 40.0, 1920.0, 1040.0),
            1.25,
        );
        let restored = PhysicalRect::new(-225.0, 867.0, 205.0, 193.0);
        let mut loaded = UiSettings::default();
        loaded.window.x = 9_999.0;
        loaded.window.y = 9_999.0;

        let state = RuntimeWindowState::from_restored_settings(restored, loaded, Some(&monitor));

        assert_eq!(state.persisted_settings().window.x, 1356.0);
        assert_eq!(state.persisted_settings().window.y, 661.6);
        assert_eq!(state.persisted_settings().window.width, 164.0);
        assert_eq!(state.persisted_settings().window.height, 154.0);
    }

    #[test]
    fn locked_state_changes_only_after_native_success() {
        let mut state = RuntimeWindowState::new(PhysicalRect::new(0.0, 0.0, 164.0, 154.0));
        let window = FakeWindow::default();
        window.fail_ignore.set(true);
        let tray = FakeTray::with_checked(LOCKED_MENU_ID, true);

        let result = WindowStateController::new(&mut state).set_locked(&window, &tray, true);

        assert!(result.is_err());
        assert!(!state.persisted_settings().locked);
        assert!(!tray.is_checked(LOCKED_MENU_ID).unwrap());
        assert!(!window.ignored.get());
    }

    #[test]
    fn tray_failure_rolls_checkbox_back() {
        let mut state = RuntimeWindowState::new(PhysicalRect::new(0.0, 0.0, 164.0, 154.0));
        let window = FakeWindow::default();
        window.topmost.set(true);
        let tray = FakeTray::with_checked(TOPMOST_MENU_ID, false);
        tray.fail_next_write.set(true);

        let result =
            WindowStateController::new(&mut state).set_always_on_top(&window, &tray, false);

        assert!(result.is_err());
        assert!(state.persisted_settings().always_on_top);
        assert!(tray.is_checked(TOPMOST_MENU_ID).unwrap());
        assert!(window.topmost.get());
    }

    #[test]
    fn checkbox_read_failure_is_not_treated_as_a_default_value() {
        let mut state = RuntimeWindowState::new(PhysicalRect::new(0.0, 0.0, 164.0, 154.0));
        let window = FakeWindow::default();
        let tray = FakeTray::with_checked(LOCKED_MENU_ID, true);
        tray.fail_read.set(true);

        let result = WindowStateController::new(&mut state).set_locked_from_tray(&window, &tray);

        assert!(result.is_err());
        assert!(!state.persisted_settings().locked);
        assert!(!window.ignored.get());
        assert!(window.calls.borrow().is_empty());
    }

    #[test]
    fn close_requested_hides_and_marks_invisible() {
        let mut state = RuntimeWindowState::new(PhysicalRect::new(0.0, 0.0, 164.0, 154.0));
        let calls = Rc::new(RefCell::new(Vec::new()));
        let window = FakeWindow {
            calls: Rc::clone(&calls),
            ..FakeWindow::default()
        };

        WindowStateController::new(&mut state)
            .close_requested(&window, || calls.borrow_mut().push("prevent"))
            .unwrap();

        assert_eq!(&*calls.borrow(), &["prevent", "hide"]);
        assert!(!state.persisted_settings().visible);
    }

    #[test]
    fn visible_state_changes_only_after_native_success() {
        let mut state = RuntimeWindowState::new(PhysicalRect::new(0.0, 0.0, 164.0, 154.0));
        let window = FakeWindow {
            fail_hide: Cell::new(true),
            ..FakeWindow::default()
        };

        let result = WindowStateController::new(&mut state).set_visible(&window, false);

        assert!(result.is_err());
        assert!(state.persisted_settings().visible);
    }

    #[test]
    fn close_hide_failure_prevents_close_but_keeps_visible_state() {
        let mut state = RuntimeWindowState::new(PhysicalRect::new(0.0, 0.0, 164.0, 154.0));
        let calls = Rc::new(RefCell::new(Vec::new()));
        let window = FakeWindow {
            calls: Rc::clone(&calls),
            fail_hide: Cell::new(true),
            ..FakeWindow::default()
        };

        let result = WindowStateController::new(&mut state)
            .close_requested(&window, || calls.borrow_mut().push("prevent"));

        assert!(result.is_err());
        assert_eq!(&*calls.borrow(), &["prevent", "hide"]);
        assert!(state.persisted_settings().visible);
    }

    #[test]
    fn restored_lock_requires_ready_tray() {
        let mut settings = UiSettings {
            locked: true,
            ..UiSettings::default()
        };
        let mut state = RuntimeWindowState::from_settings(
            PhysicalRect::new(0.0, 0.0, 164.0, 154.0),
            settings.clone(),
        );
        let window = FakeWindow::default();

        let result = WindowStateController::new(&mut state)
            .restore_locked::<FakeWindow, FakeTray>(&window, None);

        assert!(result.is_err());
        assert!(!state.persisted_settings().locked);
        assert!(!window.ignored.get());

        settings.locked = true;
        let mut state =
            RuntimeWindowState::from_settings(PhysicalRect::new(0.0, 0.0, 164.0, 154.0), settings);
        let tray = FakeTray::with_checked(LOCKED_MENU_ID, false);
        tray.fail_next_write.set(true);

        let result = WindowStateController::new(&mut state).restore_locked(&window, Some(&tray));

        assert!(result.is_err());
        assert!(!state.persisted_settings().locked);
        assert!(!window.ignored.get());
    }

    #[test]
    fn failed_native_rollback_requires_shutdown_instead_of_hiding_state_divergence() {
        let mut state = RuntimeWindowState::new(PhysicalRect::new(0.0, 0.0, 164.0, 154.0));
        let window = FakeWindow::default();
        window.fail_ignore_on_call.set(2);
        let tray = FakeTray::with_checked(LOCKED_MENU_ID, true);
        tray.fail_next_write.set(true);

        let error = WindowStateController::new(&mut state)
            .set_locked(&window, &tray, true)
            .unwrap_err();

        assert!(error.requires_shutdown());
        assert!(!state.persisted_settings().locked);
        assert!(window.ignored.get());
    }

    #[test]
    fn restored_lock_compensation_failure_does_not_claim_unlocked() {
        let settings = UiSettings {
            locked: true,
            ..UiSettings::default()
        };
        let mut state =
            RuntimeWindowState::from_settings(PhysicalRect::new(0.0, 0.0, 164.0, 154.0), settings);
        let window = FakeWindow::default();
        window.fail_ignore.set(true);
        let tray = FakeTray::with_checked(LOCKED_MENU_ID, false);
        tray.fail_next_write.set(true);

        let error = WindowStateController::new(&mut state)
            .restore_locked(&window, Some(&tray))
            .unwrap_err();

        assert!(error.requires_shutdown());
        assert!(state.persisted_settings().locked);
    }
}
