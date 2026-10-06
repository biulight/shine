//! One path identity encoder shared by lifecycle planners and approved executors.
use super::RuntimeContext;
use crate::permission::PermissionPathBaseV1;
use std::path::{Component, Path};

pub(super) fn review_path(context: &RuntimeContext, path: &Path) -> String {
    for (base, root) in [
        (PermissionPathBaseV1::Shine, &context.shine_dir),
        (PermissionPathBaseV1::DataDir, &context.data_dir),
        (PermissionPathBaseV1::Home, &context.home_dir),
    ] {
        if let Ok(relative) = path.strip_prefix(root) {
            let value = if relative.as_os_str().is_empty() {
                ".".into()
            } else {
                logical_path(relative)
            };
            return format!("{}:{value}", base.as_str());
        }
    }
    format!("absolute:{}", logical_path(path))
}

pub(super) fn logical_path(path: &Path) -> String {
    let mut value = String::new();
    let mut separator = false;
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                value.push_str(&prefix.as_os_str().to_string_lossy().replace('\\', "/"));
                separator = false;
            }
            Component::RootDir => {
                if !value.ends_with('/') {
                    value.push('/');
                }
                separator = false;
            }
            _ => {
                if separator {
                    value.push('/');
                }
                value.push_str(&component.as_os_str().to_string_lossy());
                separator = true;
            }
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_identities_keep_their_existing_shape() {
        assert_eq!(
            logical_path(Path::new("app/docker/daemon.json")),
            "app/docker/daemon.json"
        );
        assert_eq!(logical_path(Path::new("./config")), "./config");
        assert_eq!(logical_path(Path::new("../config")), "../config");
    }

    #[cfg(unix)]
    #[test]
    fn unix_root_is_not_joined_as_an_extra_component() {
        assert_eq!(logical_path(Path::new("/")), "/");
        assert_eq!(
            logical_path(Path::new("/etc/docker/daemon.json")),
            "/etc/docker/daemon.json"
        );
        let context = RuntimeContext::isolated(
            "/home/test".into(),
            "/home/test/.shine".into(),
            "/home/test/.shine/presets".into(),
            "/home/test/.shine/bin".into(),
            super::super::RuntimePlatform::Linux,
        );
        assert_eq!(
            review_path(&context, Path::new("/etc/docker/daemon.json")),
            "absolute:/etc/docker/daemon.json"
        );
        assert_eq!(
            review_path(&context, Path::new("/home/test/.shine")),
            "shine:."
        );
        assert_eq!(
            review_path(&context, Path::new("/home/test/.shine/app-manifest.toml")),
            "shine:app-manifest.toml"
        );
        assert_eq!(
            review_path(&context, Path::new("/home/test/.config/app")),
            "home:.config/app"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_prefixes_keep_drive_root_and_unc_identity() {
        for (path, expected) in [
            (r"C:\", "C:/"),
            (
                r"C:\ProgramData\Docker\daemon.json",
                "C:/ProgramData/Docker/daemon.json",
            ),
            (r"C:relative", "C:relative"),
            (r"\\server\share\config", "//server/share/config"),
            (r"\\?\C:\config", "//?/C:/config"),
            (
                r"\\?\UNC\server\share\config",
                "//?/UNC/server/share/config",
            ),
        ] {
            assert_eq!(logical_path(Path::new(path)), expected);
        }
    }
}
