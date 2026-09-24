use std::path::{Path, PathBuf};

pub fn private_codex_home(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("codex-runtime-home")
}

pub fn bundled_executable_for(current_exe: &Path) -> PathBuf {
    current_exe
        .with_file_name("resources")
        .join("codex-runtime")
        .join("bin")
        .join("codex.exe")
}

pub const MINIMUM_COMPATIBLE_VERSION: (u16, u16, u16) = (0, 155, 0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeSource {
    System,
    Bundled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedRuntime {
    pub source: RuntimeSource,
    pub executable: PathBuf,
    pub version: String,
    pub logged_in: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeLaunch {
    pub source: RuntimeSource,
    pub executable: PathBuf,
    pub version: String,
    pub codex_home: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    MissingCompatibleRuntime,
}

pub struct RuntimeManager;

impl RuntimeManager {
    pub fn select(
        system: Option<DetectedRuntime>,
        bundled: Option<DetectedRuntime>,
        app_runtime_home: PathBuf,
    ) -> Result<RuntimeLaunch, RuntimeError> {
        if let Some(system) = system.filter(|runtime| {
            runtime.source == RuntimeSource::System
                && runtime.logged_in
                && is_compatible(&runtime.version)
        }) {
            return Ok(RuntimeLaunch {
                source: system.source,
                executable: system.executable,
                version: system.version,
                codex_home: None,
            });
        }
        if let Some(bundled) = bundled.filter(|runtime| {
            runtime.source == RuntimeSource::Bundled && is_compatible(&runtime.version)
        }) {
            return Ok(RuntimeLaunch {
                source: bundled.source,
                executable: bundled.executable,
                version: bundled.version,
                codex_home: Some(app_runtime_home),
            });
        }
        Err(RuntimeError::MissingCompatibleRuntime)
    }
}

pub fn is_compatible(version: &str) -> bool {
    let Some(version) = parse_version(version) else {
        return false;
    };
    version >= MINIMUM_COMPATIBLE_VERSION
}

fn parse_version(value: &str) -> Option<(u16, u16, u16)> {
    let token = value.split_whitespace().find(|token| {
        token
            .trim_start_matches('v')
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_digit())
    })?;
    let mut parts = token.trim_start_matches('v').split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.split('-').next()?.parse().ok()?;
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn detected(source: RuntimeSource, version: &str, logged_in: bool) -> DetectedRuntime {
        DetectedRuntime {
            source,
            executable: PathBuf::from("C:/runtime/codex.exe"),
            version: version.into(),
            logged_in,
        }
    }

    #[test]
    fn selects_an_existing_compatible_logged_in_system_runtime() {
        let selected = RuntimeManager::select(
            Some(detected(RuntimeSource::System, "0.156.1", true)),
            Some(detected(RuntimeSource::Bundled, "0.156.1", false)),
            PathBuf::from("C:/CodexPet/runtime-home"),
        )
        .unwrap();

        assert_eq!(selected.source, RuntimeSource::System);
        assert_eq!(selected.codex_home, None);
    }

    #[test]
    fn uses_private_bundled_runtime_when_system_runtime_is_logged_out() {
        let selected = RuntimeManager::select(
            Some(detected(RuntimeSource::System, "0.156.1", false)),
            Some(detected(RuntimeSource::Bundled, "0.156.1", false)),
            PathBuf::from("C:/CodexPet/runtime-home"),
        )
        .unwrap();

        assert_eq!(selected.source, RuntimeSource::Bundled);
        assert_eq!(
            selected.codex_home,
            Some(PathBuf::from("C:/CodexPet/runtime-home"))
        );
    }

    #[test]
    fn falls_back_when_system_runtime_is_too_old() {
        let selected = RuntimeManager::select(
            Some(detected(RuntimeSource::System, "0.100.0", true)),
            Some(detected(RuntimeSource::Bundled, "0.156.1", false)),
            PathBuf::from("C:/CodexPet/runtime-home"),
        )
        .unwrap();

        assert_eq!(selected.source, RuntimeSource::Bundled);
    }

    #[test]
    fn reports_missing_when_no_compatible_runtime_exists() {
        let error = RuntimeManager::select(
            Some(detected(RuntimeSource::System, "0.100.0", true)),
            None,
            PathBuf::from("C:/CodexPet/runtime-home"),
        )
        .unwrap_err();

        assert_eq!(error, RuntimeError::MissingCompatibleRuntime);
    }

    #[test]
    fn bundled_private_home_is_stable_across_launches() {
        let app_data = PathBuf::from("C:/Users/test/AppData/Roaming/io.github.fristyya.codex-pet");
        let first = private_codex_home(&app_data);
        let restarted = private_codex_home(&app_data);
        assert_eq!(first, restarted);
        assert_eq!(first, app_data.join("codex-runtime-home"));
    }

    #[test]
    fn bundled_runtime_is_resolved_from_the_running_exe_directory() {
        let exe = PathBuf::from("C:/Program Files/Codex Pet/codex-pet.exe");
        assert_eq!(
            bundled_executable_for(&exe),
            PathBuf::from("C:/Program Files/Codex Pet/resources/codex-runtime/bin/codex.exe")
        );
    }
}
