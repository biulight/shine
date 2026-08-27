use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OperatingSystem {
    Macos,
    Linux,
    Windows,
}

impl OperatingSystem {
    pub(crate) const ALL: [Self; 3] = [Self::Macos, Self::Linux, Self::Windows];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Macos => "macos",
            Self::Linux => "linux",
            Self::Windows => "windows",
        }
    }

    pub(crate) const fn is_unix(self) -> bool {
        matches!(self, Self::Macos | Self::Linux)
    }
}

pub fn executable_name_for_os(os: &str) -> &'static str {
    if os == "windows" {
        "shine.exe"
    } else {
        "shine"
    }
}

pub fn current_executable_name() -> &'static str {
    executable_name_for_os(std::env::consts::OS)
}

pub(crate) fn current_platform() -> OperatingSystem {
    match std::env::consts::OS {
        "macos" => OperatingSystem::Macos,
        "linux" => OperatingSystem::Linux,
        "windows" => OperatingSystem::Windows,
        other => panic!("unsupported operating system: {other}"),
    }
}

pub fn release_target(os: &str, arch: &str) -> Result<String> {
    let normalized_os = match os {
        "macos" => "darwin",
        "linux" => "linux",
        "windows" => "windows",
        other => bail!("unsupported operating system: {other}"),
    };

    let normalized_arch = match arch {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => bail!("unsupported architecture: {other}"),
    };

    Ok(format!("{normalized_os}-{normalized_arch}"))
}

pub fn default_self_install_dest() -> Result<PathBuf> {
    default_self_install_dest_for(
        std::env::consts::OS,
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
    )
}

pub fn default_self_install_dest_for(os: &str, local_appdata: Option<PathBuf>) -> Result<PathBuf> {
    match os {
        "windows" => {
            let Some(local_appdata) = local_appdata else {
                bail!("LOCALAPPDATA is not set; pass --dest <PATH> to choose an install location");
            };
            Ok(local_appdata
                .join("Programs")
                .join("shine")
                .join(executable_name_for_os(os)))
        }
        _ => Ok(PathBuf::from("/usr/local/bin").join(executable_name_for_os(os))),
    }
}

/// True when `command` resolves to a file on the current `PATH`. On Windows the
/// common executable extensions (`.exe`, `.cmd`, `.bat`) are also probed. Used for
/// diagnostic "is this runtime available?" hints, not as an execution gate.
pub fn command_exists_on_path(command: &str) -> bool {
    let var_name = if cfg!(windows) { "Path" } else { "PATH" };
    let Some(path_value) = std::env::var_os(var_name) else {
        return false;
    };
    let candidates: Vec<String> = if cfg!(windows) {
        vec![
            command.to_string(),
            format!("{command}.exe"),
            format!("{command}.cmd"),
            format!("{command}.bat"),
        ]
    } else {
        vec![command.to_string()]
    };
    std::env::split_paths(&path_value)
        .any(|dir| candidates.iter().any(|name| dir.join(name).is_file()))
}

pub fn current_path_contains_dir(dir: &Path) -> bool {
    let var_name = if cfg!(windows) { "Path" } else { "PATH" };
    let Some(path_value) = std::env::var_os(var_name) else {
        return false;
    };
    std::env::split_paths(&path_value).any(|entry| paths_equivalent(&entry, dir))
}

pub fn path_install_hint(dir: &Path) -> String {
    if cfg!(windows) {
        let escaped = dir.display().to_string().replace('\'', "''");
        format!(
            "Add it to your user PATH, for example: [Environment]::SetEnvironmentVariable('Path', [Environment]::GetEnvironmentVariable('Path', 'User') + ';{escaped}', 'User')"
        )
    } else {
        format!(
            "Add it to PATH, for example: export PATH=\"{}:$PATH\"",
            dir.display()
        )
    }
}

fn paths_equivalent(left: &Path, right: &Path) -> bool {
    let left = normalize_for_compare(left);
    let right = normalize_for_compare(right);
    if cfg!(windows) {
        left.eq_ignore_ascii_case(&right)
    } else {
        left == right
    }
}

fn normalize_for_compare(path: &Path) -> String {
    path.to_string_lossy()
        .trim_end_matches(std::path::MAIN_SEPARATOR)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_target_maps_supported_platforms() {
        assert_eq!(
            release_target("macos", "aarch64").unwrap(),
            "darwin-aarch64"
        );
        assert_eq!(release_target("linux", "x86_64").unwrap(), "linux-x86_64");
        assert_eq!(
            release_target("windows", "x86_64").unwrap(),
            "windows-x86_64"
        );
        assert_eq!(
            release_target("windows", "aarch64").unwrap(),
            "windows-aarch64"
        );
    }

    #[test]
    fn operating_system_names_and_unix_group_are_stable() {
        assert_eq!(OperatingSystem::Macos.as_str(), "macos");
        assert_eq!(OperatingSystem::Linux.as_str(), "linux");
        assert_eq!(OperatingSystem::Windows.as_str(), "windows");
        assert!(OperatingSystem::Macos.is_unix());
        assert!(OperatingSystem::Linux.is_unix());
        assert!(!OperatingSystem::Windows.is_unix());
    }

    #[test]
    fn executable_name_is_platform_specific() {
        assert_eq!(executable_name_for_os("linux"), "shine");
        assert_eq!(executable_name_for_os("macos"), "shine");
        assert_eq!(executable_name_for_os("windows"), "shine.exe");
    }

    #[test]
    fn default_self_install_dest_is_platform_specific() {
        assert_eq!(
            default_self_install_dest_for("linux", None).unwrap(),
            PathBuf::from("/usr/local/bin/shine")
        );
        assert_eq!(
            default_self_install_dest_for(
                "windows",
                Some(PathBuf::from(r"C:\Users\me\AppData\Local"))
            )
            .unwrap(),
            PathBuf::from(r"C:\Users\me\AppData\Local")
                .join("Programs")
                .join("shine")
                .join("shine.exe")
        );
    }

    #[test]
    fn windows_default_requires_local_appdata() {
        let err = default_self_install_dest_for("windows", None).unwrap_err();
        assert!(err.to_string().contains("LOCALAPPDATA"));
    }
}
