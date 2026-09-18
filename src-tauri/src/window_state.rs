use crate::settings::{
    CollapsedRectState, MoveTracker, PhysicalRect, SavedWindow, UiSettings,
    current_collapsed_physical_size, save_collapsed_rect,
};

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
    provider
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| {
            provider
                .available_monitors()
                .ok()
                .and_then(|monitors| monitors.into_iter().next())
        })
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
}
