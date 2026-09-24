use std::{
    collections::HashSet,
    fs::OpenOptions,
    io::Write,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum StartupStage {
    StartupStarted,
    SetupStarted,
    WindowHandleAvailable,
    SettingsLoaded,
    WindowPositionRestored,
    TrayReady,
    AutostartChecked,
    WindowStateRestored,
    WindowShown,
    ReactFirstFrame,
    AccountRuntimeSelected,
    AccountRuntimeSelectionStarted,
    AccountRuntimeReady,
    AccountCheckComplete,
    QuotaRuntimeSelected,
    QuotaRuntimeSelectionStarted,
    QuotaRuntimeReady,
    QuotaRefreshStarted,
    QuotaDataFresh,
    QuotaRefreshComplete,
}

impl StartupStage {
    fn label(self) -> &'static str {
        match self {
            Self::StartupStarted => "startup_started",
            Self::SetupStarted => "setup_started",
            Self::WindowHandleAvailable => "window_handle_available",
            Self::SettingsLoaded => "settings_loaded",
            Self::WindowPositionRestored => "window_position_restored",
            Self::TrayReady => "tray_ready",
            Self::AutostartChecked => "autostart_checked",
            Self::WindowStateRestored => "window_state_restored",
            Self::WindowShown => "window_shown",
            Self::ReactFirstFrame => "react_first_frame",
            Self::AccountRuntimeSelected => "account_runtime_selected",
            Self::AccountRuntimeSelectionStarted => "account_runtime_selection_started",
            Self::AccountRuntimeReady => "account_runtime_ready",
            Self::AccountCheckComplete => "account_check_complete",
            Self::QuotaRuntimeSelected => "quota_runtime_selected",
            Self::QuotaRuntimeSelectionStarted => "quota_runtime_selection_started",
            Self::QuotaRuntimeReady => "quota_runtime_ready",
            Self::QuotaRefreshStarted => "quota_refresh_started",
            Self::QuotaDataFresh => "quota_data_fresh",
            Self::QuotaRefreshComplete => "quota_refresh_complete",
        }
    }
}

struct StartupMetrics {
    started: Instant,
    recorded: Mutex<HashSet<StartupStage>>,
}

impl StartupMetrics {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            recorded: Mutex::new(HashSet::new()),
        }
    }

    fn record(&self, stage: StartupStage) -> Option<Duration> {
        let mut recorded = self.recorded.lock().ok()?;
        if !recorded.insert(stage) {
            return None;
        }

        let elapsed = self.started.elapsed();
        if let Some(path) = std::env::var_os("CODEX_PET_STARTUP_LOG")
            && let Ok(mut log) = OpenOptions::new().create(true).append(true).open(path)
        {
            let _ = writeln!(
                log,
                "stage={} elapsed_ms={}",
                stage.label(),
                elapsed.as_millis()
            );
        }
        Some(elapsed)
    }
}

static STARTUP_METRICS: OnceLock<StartupMetrics> = OnceLock::new();

pub(crate) fn record(stage: StartupStage) {
    let metrics = STARTUP_METRICS.get_or_init(StartupMetrics::new);
    let _ = metrics.record(stage);
}

#[cfg(test)]
mod tests {
    use super::{StartupMetrics, StartupStage};

    #[test]
    fn records_each_stage_only_once() {
        let metrics = StartupMetrics::new();

        assert!(metrics.record(StartupStage::StartupStarted).is_some());
        assert!(
            metrics
                .record(StartupStage::WindowHandleAvailable)
                .is_some()
        );
        assert!(
            metrics
                .record(StartupStage::WindowHandleAvailable)
                .is_none()
        );
    }

    #[test]
    fn react_first_frame_has_a_stable_local_metric_name() {
        assert_eq!(StartupStage::ReactFirstFrame.label(), "react_first_frame");
    }
}
