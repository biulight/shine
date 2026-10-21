//! Pure path text conventions. These helpers do not validate or resolve filesystem paths.

use std::path::Path;

/// Join native components for preset snapshot keys; keep the historical component spelling.
pub(super) fn preset_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Recognize author-specific home paths for lint and synthetic fixture policy.
pub(super) fn private_absolute_path(value: &str) -> bool {
    let normalized = value.replace('\\', "/");
    normalized.starts_with("/Users/")
        || normalized.starts_with("/home/")
        || normalized.to_ascii_lowercase().starts_with("c:/users/")
}

/// Strip only the Windows verbatim prefix before launcher/profile quoting.
pub(super) fn strip_windows_verbatim_prefix(value: &str) -> String {
    value
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .or_else(|| value.strip_prefix(r"\\?\").map(str::to_string))
        .unwrap_or_else(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_keys_preserve_component_spelling() {
        for (path, expected) in [
            ("", ""),
            ("./app//demo/./config/", "./app/demo/config"),
            ("app/demo/../config", "app/demo/../config"),
            ("app/中文/e\u{301}", "app/中文/e\u{301}"),
        ] {
            assert_eq!(preset_path(Path::new(path)), expected);
        }
        // This is the preset component convention, not the security review encoder.
        #[cfg(unix)]
        {
            assert_eq!(preset_path(Path::new("/etc/config")), "//etc/config");
            assert_eq!(preset_path(Path::new(r"a\b")), r"a\b");
            use std::os::unix::ffi::OsStrExt;
            let path = Path::new(std::ffi::OsStr::from_bytes(b"app/\xff"));
            assert_eq!(preset_path(path), "app/\u{fffd}");
        }
    }

    #[test]
    fn private_path_detection_retains_case_and_prefix_policy() {
        for path in [
            "/Users/alice/a",
            "/home/alice/a",
            r"C:\Users\Alice\a",
            "c:/USERS/Alice/a",
        ] {
            assert!(private_absolute_path(path), "{path}");
        }
        for path in [
            "/Users",
            "/users/alice/a",
            "/Home/alice/a",
            "/homeish/a",
            "D:/Users/alice/a",
            "~/a",
            "/etc/a",
        ] {
            assert!(!private_absolute_path(path), "{path}");
        }
    }

    #[test]
    fn windows_prefix_removal_preserves_unc_and_literal_contents() {
        for (path, expected) in [
            (r"\\?\C:\O'Brien\a", r"C:\O'Brien\a"),
            (r"\\?\UNC\server\share\a", r"\\server\share\a"),
            (r"\\?\unc\server\share", r"unc\server\share"),
            (r"\\server\share", r"\\server\share"),
            (r"\\.\pipe\name", r"\\.\pipe\name"),
            ("/tmp/中文", "/tmp/中文"),
            ("", ""),
        ] {
            assert_eq!(strip_windows_verbatim_prefix(path), expected);
        }
    }
}
