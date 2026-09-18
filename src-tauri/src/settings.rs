use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
};

pub const CURRENT_SETTINGS_VERSION: u32 = 1;
pub const CURRENT_COLLAPSED_WIDTH: f64 = 164.0;
pub const CURRENT_COLLAPSED_HEIGHT: f64 = 154.0;
pub const DEFAULT_EDGE_MARGIN_LOGICAL: f64 = 16.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysicalRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl PhysicalRect {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
    pub const fn origin(self) -> (f64, f64) {
        (self.x, self.y)
    }
}

#[derive(Clone, Debug)]
pub struct CollapsedRectState {
    rect: PhysicalRect,
    monitor_key: Option<String>,
    monitor_name: Option<String>,
    scale_factor: f64,
}

impl CollapsedRectState {
    pub fn new(
        rect: PhysicalRect,
        monitor_key: Option<String>,
        monitor_name: Option<String>,
        scale_factor: f64,
    ) -> Self {
        Self {
            rect,
            monitor_key,
            monitor_name,
            scale_factor,
        }
    }
    pub fn rect(&self) -> PhysicalRect {
        self.rect
    }
    pub fn saved_window(&self) -> SavedWindow {
        SavedWindow {
            x: self.rect.x,
            y: self.rect.y,
            width: CURRENT_COLLAPSED_WIDTH,
            height: CURRENT_COLLAPSED_HEIGHT,
            monitor_key: self.monitor_key.clone(),
            monitor_name: self.monitor_name.clone(),
            scale_factor: self.scale_factor,
        }
    }
    fn replace_rect(&mut self, rect: PhysicalRect) {
        self.rect = rect;
    }
}

pub struct MoveTracker {
    collapsed: CollapsedRectState,
    expected_programmatic_move: Option<PhysicalRect>,
}

impl MoveTracker {
    pub fn new(collapsed: CollapsedRectState) -> Self {
        Self {
            collapsed,
            expected_programmatic_move: None,
        }
    }
    pub fn expect_programmatic_move(&mut self, rect: PhysicalRect) {
        self.expected_programmatic_move = Some(rect);
    }
    pub fn collapsed_rect(&self) -> PhysicalRect {
        self.collapsed.rect()
    }
    pub fn on_moved(&mut self, rect: PhysicalRect, expanded: bool) -> bool {
        if self.expected_programmatic_move.take() == Some(rect) || expanded {
            return false;
        }
        self.collapsed.replace_rect(rect);
        true
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedWindow {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub monitor_key: Option<String>,
    pub monitor_name: Option<String>,
    pub scale_factor: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiSettings {
    pub version: u32,
    pub window: SavedWindow,
    pub locked: bool,
    pub always_on_top: bool,
    pub visible: bool,
    pub autostart: bool,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            version: CURRENT_SETTINGS_VERSION,
            window: SavedWindow {
                x: 0.0,
                y: 0.0,
                width: CURRENT_COLLAPSED_WIDTH,
                height: CURRENT_COLLAPSED_HEIGHT,
                monitor_key: None,
                monitor_name: None,
                scale_factor: 1.0,
            },
            locked: false,
            always_on_top: true,
            visible: true,
            autostart: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedSettings {
    pub settings: UiSettings,
    pub may_overwrite_source: bool,
    pub has_valid_source: bool,
}

pub fn parse_settings(source: &str) -> ParsedSettings {
    let Ok(mut settings) = serde_json::from_str::<UiSettings>(source) else {
        return ParsedSettings {
            settings: UiSettings::default(),
            may_overwrite_source: true,
            has_valid_source: false,
        };
    };
    if settings.version > CURRENT_SETTINGS_VERSION {
        return ParsedSettings {
            settings: UiSettings::default(),
            may_overwrite_source: false,
            has_valid_source: false,
        };
    }
    settings.version = CURRENT_SETTINGS_VERSION;
    settings.window.width = CURRENT_COLLAPSED_WIDTH;
    settings.window.height = CURRENT_COLLAPSED_HEIGHT;
    ParsedSettings {
        settings,
        may_overwrite_source: true,
        has_valid_source: true,
    }
}

pub fn load_settings(path: &Path) -> ParsedSettings {
    match fs::read_to_string(path) {
        Ok(source) => {
            let parsed = parse_settings(&source);
            if !parsed.has_valid_source && parsed.may_overwrite_source {
                let backup = backup_path(path);
                if !backup.exists() {
                    let _ = fs::copy(path, backup);
                }
            }
            parsed
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => ParsedSettings {
            settings: UiSettings::default(),
            may_overwrite_source: true,
            has_valid_source: false,
        },
        Err(_) => ParsedSettings {
            settings: UiSettings::default(),
            may_overwrite_source: true,
            has_valid_source: false,
        },
    }
}

fn backup_path(target: &Path) -> PathBuf {
    let filename = target.file_name().unwrap_or_default().to_string_lossy();
    target.with_file_name(format!("{filename}.bak"))
}

pub fn save_collapsed_rect(
    rect: PhysicalRect,
    work_area: PhysicalRect,
    scale_factor: f64,
) -> SavedWindow {
    SavedWindow {
        x: (rect.x - work_area.x) / scale_factor,
        y: (rect.y - work_area.y) / scale_factor,
        width: CURRENT_COLLAPSED_WIDTH,
        height: CURRENT_COLLAPSED_HEIGHT,
        monitor_key: None,
        monitor_name: None,
        scale_factor,
    }
}

pub fn restore_collapsed_rect(
    saved: &SavedWindow,
    work_area: PhysicalRect,
    target_scale_factor: f64,
) -> PhysicalRect {
    let size = current_collapsed_physical_size(target_scale_factor);
    clamp_to_work_area(
        PhysicalRect::new(
            work_area.x + logical_to_physical(saved.x, target_scale_factor),
            work_area.y + logical_to_physical(saved.y, target_scale_factor),
            size.0,
            size.1,
        ),
        work_area,
    )
}

pub fn initial_collapsed_rect(
    loaded: &ParsedSettings,
    work_area: PhysicalRect,
    target_scale_factor: f64,
) -> PhysicalRect {
    if loaded.has_valid_source {
        restore_collapsed_rect(&loaded.settings.window, work_area, target_scale_factor)
    } else {
        default_collapsed_rect(work_area, target_scale_factor)
    }
}

pub fn current_collapsed_physical_size(scale_factor: f64) -> (f64, f64) {
    (
        logical_to_physical(CURRENT_COLLAPSED_WIDTH, scale_factor),
        logical_to_physical(CURRENT_COLLAPSED_HEIGHT, scale_factor),
    )
}

pub fn default_collapsed_rect(work_area: PhysicalRect, scale_factor: f64) -> PhysicalRect {
    let size = current_collapsed_physical_size(scale_factor);
    let margin = logical_to_physical(DEFAULT_EDGE_MARGIN_LOGICAL, scale_factor);
    clamp_to_work_area(
        PhysicalRect::new(
            work_area.x + work_area.width - size.0 - margin,
            work_area.y + work_area.height - size.1 - margin,
            size.0,
            size.1,
        ),
        work_area,
    )
}

fn logical_to_physical(value: f64, scale_factor: f64) -> f64 {
    (value * scale_factor).round()
}

pub fn clamp_to_work_area(rect: PhysicalRect, work_area: PhysicalRect) -> PhysicalRect {
    let max_x = work_area.x + (work_area.width - rect.width).max(0.0);
    let max_y = work_area.y + (work_area.height - rect.height).max(0.0);
    PhysicalRect::new(
        rect.x.clamp(work_area.x, max_x),
        rect.y.clamp(work_area.y, max_y),
        rect.width,
        rect.height,
    )
}

pub trait AtomicReplace {
    fn replace_existing(&self, temporary: &Path, target: &Path) -> io::Result<()>;
}

#[cfg(windows)]
pub struct WindowsReplace;

#[cfg(windows)]
impl AtomicReplace for WindowsReplace {
    fn replace_existing(&self, temporary: &Path, target: &Path) -> io::Result<()> {
        use std::{iter, os::windows::ffi::OsStrExt};
        use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;

        if !target.exists() {
            return fs::rename(temporary, target);
        }
        let target_wide: Vec<u16> = target
            .as_os_str()
            .encode_wide()
            .chain(iter::once(0))
            .collect();
        let temporary_wide: Vec<u16> = temporary
            .as_os_str()
            .encode_wide()
            .chain(iter::once(0))
            .collect();
        let replaced = unsafe {
            ReplaceFileW(
                target_wide.as_ptr(),
                temporary_wide.as_ptr(),
                std::ptr::null(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if replaced == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

pub fn save_atomic_with(
    replacer: &impl AtomicReplace,
    target: &Path,
    settings: &UiSettings,
) -> io::Result<()> {
    let temporary = temporary_path(target);
    let serialized =
        serde_json::to_vec_pretty(settings).map_err(|error| io::Error::other(error.to_string()))?;
    let write_result = (|| {
        {
            let mut file = File::create(&temporary)?;
            file.write_all(&serialized)?;
            file.flush()?;
            file.sync_all()?;
        }
        replacer.replace_existing(&temporary, target)
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}

fn temporary_path(target: &Path) -> PathBuf {
    let filename = target.file_name().unwrap_or_default().to_string_lossy();
    target.with_file_name(format!(".{filename}.tmp"))
}

pub struct DebouncedSettingsWriter<R> {
    replacer: R,
    target: PathBuf,
    pending: Option<(u64, UiSettings)>,
    next_token: u64,
    stopped: bool,
}

impl<R: AtomicReplace> DebouncedSettingsWriter<R> {
    pub fn new(replacer: R, target: PathBuf) -> Self {
        Self {
            replacer,
            target,
            pending: None,
            next_token: 0,
            stopped: false,
        }
    }

    pub fn schedule(&mut self, settings: UiSettings) -> u64 {
        self.next_token = self.next_token.wrapping_add(1);
        if !self.stopped {
            self.pending = Some((self.next_token, settings));
        }
        self.next_token
    }

    pub fn flush_if_current(&mut self, token: u64) -> io::Result<()> {
        let Some((pending_token, _)) = self.pending.as_ref() else {
            return Ok(());
        };
        if *pending_token != token {
            return Ok(());
        }
        let (_, settings) = self.pending.take().expect("pending settings checked above");
        save_atomic_with(&self.replacer, &self.target, &settings)
    }

    pub fn shutdown(&mut self) -> io::Result<()> {
        self.stopped = true;
        if let Some((_, settings)) = self.pending.take() {
            save_atomic_with(&self.replacer, &self.target, &settings)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailingReplace;

    impl AtomicReplace for FailingReplace {
        fn replace_existing(&self, _: &Path, _: &Path) -> io::Result<()> {
            Err(io::Error::other("replace failed"))
        }
    }

    #[test]
    fn default_settings_use_the_current_collapsed_size() {
        let settings = UiSettings::default();

        assert_eq!(settings.version, CURRENT_SETTINGS_VERSION);
        assert_eq!(settings.window.width, CURRENT_COLLAPSED_WIDTH);
        assert_eq!(settings.window.height, CURRENT_COLLAPSED_HEIGHT);
        assert!(settings.always_on_top);
        assert!(!settings.locked);
        assert!(settings.visible);
        assert!(!settings.autostart);
    }

    #[test]
    fn malformed_or_future_settings_fall_back_without_becoming_writable() {
        assert_eq!(parse_settings("not-json").settings, UiSettings::default());

        let future = r#"{
          "version": 2,
          "window": { "x": 1, "y": 2, "width": 3, "height": 4, "scaleFactor": 1 },
          "locked": true,
          "alwaysOnTop": false,
          "visible": false,
          "autostart": true
        }"#;
        let result = parse_settings(future);
        assert_eq!(result.settings, UiSettings::default());
        assert!(!result.may_overwrite_source);
    }

    #[test]
    fn startup_uses_the_first_launch_position_when_the_loaded_configuration_is_invalid() {
        let work_area = PhysicalRect::new(-1920.0, 40.0, 1920.0, 1040.0);
        let invalid = parse_settings("not-json");

        assert_eq!(
            initial_collapsed_rect(&invalid, work_area, 1.25),
            default_collapsed_rect(work_area, 1.25)
        );
    }

    #[test]
    fn startup_restores_a_valid_configuration_using_the_current_monitor_scale() {
        let work_area = PhysicalRect::new(0.0, 0.0, 1920.0, 1040.0);
        let valid = parse_settings(
            r#"{
              "version": 1,
              "window": { "x": 100, "y": 80, "width": 164, "height": 154, "scaleFactor": 1 },
              "locked": false,
              "alwaysOnTop": true,
              "visible": true,
              "autostart": false
            }"#,
        );

        assert_eq!(
            initial_collapsed_rect(&valid, work_area, 1.25),
            PhysicalRect::new(125.0, 100.0, 205.0, 193.0)
        );
    }

    #[test]
    fn malformed_settings_are_backed_up_before_a_recoverable_source_is_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("ui-settings.json");
        fs::write(&target, "not-json").unwrap();

        let loaded = load_settings(&target);

        assert!(!loaded.has_valid_source);
        assert!(loaded.may_overwrite_source);
        assert_eq!(
            fs::read_to_string(target.with_file_name("ui-settings.json.bak")).unwrap(),
            "not-json"
        );
    }

    #[test]
    fn replace_failure_keeps_the_previous_valid_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("ui-settings.json");
        std::fs::write(&target, "{\"version\":1}").unwrap();

        let result = save_atomic_with(&FailingReplace, &target, &UiSettings::default());

        assert!(result.is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "{\"version\":1}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_replace_existing_replaces_target_and_removes_temporary_file() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("ui-settings.json");
        std::fs::write(&target, "old").unwrap();

        save_atomic_with(&WindowsReplace, &target, &UiSettings::default()).unwrap();

        assert_ne!(std::fs::read_to_string(&target).unwrap(), "old");
        assert!(!temporary_path(&target).exists());
    }

    struct RenameReplace;

    impl AtomicReplace for RenameReplace {
        fn replace_existing(&self, temporary: &Path, target: &Path) -> io::Result<()> {
            fs::rename(temporary, target)
        }
    }

    #[test]
    fn debounce_persists_only_the_final_pending_settings() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("ui-settings.json");
        let mut writer = DebouncedSettingsWriter::new(RenameReplace, target.clone());
        let mut a = UiSettings::default();
        let mut b = UiSettings::default();
        let mut c = UiSettings::default();
        a.window.x = 10.0;
        b.window.x = 20.0;
        c.window.x = 30.0;

        let stale_a = writer.schedule(a);
        let stale_b = writer.schedule(b);
        let current_c = writer.schedule(c);
        writer.flush_if_current(stale_a).unwrap();
        writer.flush_if_current(stale_b).unwrap();
        writer.flush_if_current(current_c).unwrap();

        assert_eq!(
            parse_settings(&fs::read_to_string(target).unwrap())
                .settings
                .window
                .x,
            30.0
        );
    }

    #[test]
    fn shutdown_flushes_the_latest_pending_settings_and_rejects_old_timer() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("ui-settings.json");
        let mut writer = DebouncedSettingsWriter::new(RenameReplace, target.clone());
        let mut latest = UiSettings::default();
        latest.window.y = 456.0;
        let stale = writer.schedule(UiSettings::default());
        writer.schedule(latest);

        writer.shutdown().unwrap();
        writer.flush_if_current(stale).unwrap();

        assert_eq!(
            parse_settings(&fs::read_to_string(target).unwrap())
                .settings
                .window
                .y,
            456.0
        );
    }

    #[test]
    fn single_monitor_round_trip_and_clamp_use_work_area_relative_coordinates() {
        let work_area = PhysicalRect::new(-1920.0, 40.0, 1920.0, 1040.0);
        let collapsed = PhysicalRect::new(-1800.0, 140.0, 205.0, 193.0);
        let saved = save_collapsed_rect(collapsed, work_area, 1.25);

        assert_eq!((saved.x, saved.y), (96.0, 80.0));
        assert_eq!(restore_collapsed_rect(&saved, work_area, 1.25), collapsed);
        assert_eq!(
            clamp_to_work_area(PhysicalRect::new(-3000.0, 2000.0, 164.0, 154.0), work_area)
                .origin(),
            (-1920.0, 926.0)
        );
    }

    #[test]
    fn expanded_rect_never_replaces_the_persisted_collapsed_rect() {
        let collapsed = PhysicalRect::new(
            20.0,
            30.0,
            CURRENT_COLLAPSED_WIDTH,
            CURRENT_COLLAPSED_HEIGHT,
        );
        let expanded = PhysicalRect::new(-120.0, -200.0, 340.0, 390.0);
        let state = CollapsedRectState::new(collapsed, None, None, 1.0);

        assert_ne!(expanded, state.rect());
        assert_eq!(state.saved_window().x, 20.0);
        assert_eq!(state.saved_window().y, 30.0);
    }

    #[test]
    fn restoration_converts_runtime_logical_size_using_target_monitor_scale() {
        let work_area = PhysicalRect::new(-1920.0, 40.0, 1920.0, 1040.0);
        let saved = SavedWindow {
            x: 100.0,
            y: 80.0,
            ..UiSettings::default().window
        };

        assert_eq!(
            restore_collapsed_rect(&saved, work_area, 1.0),
            PhysicalRect::new(-1820.0, 120.0, 164.0, 154.0)
        );
        assert_eq!(
            restore_collapsed_rect(&saved, work_area, 1.25),
            PhysicalRect::new(-1795.0, 140.0, 205.0, 193.0)
        );
        assert_eq!(
            restore_collapsed_rect(&saved, work_area, 1.5),
            PhysicalRect::new(-1770.0, 160.0, 246.0, 231.0)
        );
    }

    #[test]
    fn restoration_uses_the_target_dpi_after_saved_display_dpi_changes() {
        let work_area = PhysicalRect::new(-1920.0, 40.0, 1920.0, 1040.0);
        let saved = SavedWindow {
            x: 100.0,
            y: 80.0,
            scale_factor: 1.25,
            ..UiSettings::default().window
        };

        assert_eq!(
            restore_collapsed_rect(&saved, work_area, 1.5),
            PhysicalRect::new(-1770.0, 160.0, 246.0, 231.0)
        );
        let saved_at_150 = SavedWindow { scale_factor: 1.5, ..saved };
        assert_eq!(
            restore_collapsed_rect(&saved_at_150, work_area, 1.0),
            PhysicalRect::new(-1820.0, 120.0, 164.0, 154.0)
        );
    }

    #[test]
    fn default_position_uses_work_area_bottom_right_with_physical_margin() {
        let work_area = PhysicalRect::new(-1920.0, 40.0, 1920.0, 1040.0);

        assert_eq!(
            default_collapsed_rect(work_area, 1.25),
            PhysicalRect::new(-225.0, 867.0, 205.0, 193.0)
        );
    }

    #[test]
    fn programmatic_move_event_is_ignored_but_next_user_move_updates_collapsed_rect() {
        let initial = PhysicalRect::new(0.0, 0.0, 164.0, 154.0);
        let mut tracker = MoveTracker::new(CollapsedRectState::new(initial, None, None, 1.0));
        tracker.expect_programmatic_move(PhysicalRect::new(100.0, 100.0, 164.0, 154.0));

        assert!(!tracker.on_moved(PhysicalRect::new(100.0, 100.0, 164.0, 154.0), false));
        assert_eq!(tracker.collapsed_rect(), initial);
        assert!(tracker.on_moved(PhysicalRect::new(200.0, 200.0, 164.0, 154.0), false));
        assert_eq!(tracker.collapsed_rect().origin(), (200.0, 200.0));
    }
}
