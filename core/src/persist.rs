//! Shared atomic-write primitives.
//!
//! Several modules (config, app/sys/task manifests, ssh transfer, workspace
//! env files, self-install) each hand-rolled their own "write to a temp file,
//! then rename over the destination" sequence. This module is the single
//! place that logic lives.

use anyhow::{Context, Result};
use std::path::Path;
use tokio::io::AsyncWriteExt;

/// Durably writes `contents` to `path`.
///
/// Creates the parent directory if missing, writes to a uniquely-named temp
/// file in the same directory with existing regular-file permissions, fsyncs it,
/// then replaces `path` and synchronizes the directory entry. If
/// anything fails after the temp file is created, the temp file is removed.
pub async fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    tokio::fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;
    let temp = parent.join(format!(".shine-write-{}", uuid::Uuid::new_v4()));

    let permissions = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.is_file() => Some(metadata.permissions()),
        Ok(_) => None,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    if let Err(error) = write_temp(&temp, contents, permissions).await {
        let _ = tokio::fs::remove_file(&temp).await;
        return Err(error);
    }

    finalize_temp(&temp, path).await
}

/// Durably writes a private file with owner-only permissions on Unix.
///
/// The temporary file receives the restrictive mode before any content is
/// written, so plaintext never has a wider visibility window before rename.
pub async fn atomic_write_private(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    tokio::fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;
    let temp = parent.join(format!(".shine-write-{}", uuid::Uuid::new_v4()));

    if let Err(error) = write_private_temp(&temp, contents).await {
        let _ = tokio::fs::remove_file(&temp).await;
        return Err(error);
    }

    finalize_temp(&temp, path).await
}

#[cfg(unix)]
async fn write_private_temp(temp: &Path, contents: &[u8]) -> Result<()> {
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(temp)
        .await
        .with_context(|| format!("creating {}", temp.display()))?;
    file.write_all(contents)
        .await
        .with_context(|| format!("writing {}", temp.display()))?;
    file.sync_all()
        .await
        .with_context(|| format!("syncing {}", temp.display()))?;
    Ok(())
}

#[cfg(not(unix))]
async fn write_private_temp(temp: &Path, contents: &[u8]) -> Result<()> {
    write_temp(temp, contents, None).await
}

async fn write_temp(
    temp: &Path,
    contents: &[u8],
    permissions: Option<std::fs::Permissions>,
) -> Result<()> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if let Some(permissions) = &permissions {
        use std::os::unix::fs::PermissionsExt;
        options.mode(permissions.mode() & 0o7777);
    }
    let mut file = options
        .open(temp)
        .await
        .with_context(|| format!("creating {}", temp.display()))?;
    if let Some(permissions) = permissions {
        file.set_permissions(permissions)
            .await
            .with_context(|| format!("setting permissions for {}", temp.display()))?;
    }
    file.write_all(contents)
        .await
        .with_context(|| format!("writing {}", temp.display()))?;
    file.sync_all()
        .await
        .with_context(|| format!("syncing {}", temp.display()))?;
    Ok(())
}

/// Renames `temp` over `dest`, for callers that already wrote/streamed their
/// own temp file (e.g. large-file copies) and only need the finalize step.
///
/// Rename replaces an existing file on both Unix and Windows. Never unlink
/// `dest` first: a failed replacement must retain the previous contents.
/// On failure, only `temp` is removed. A synchronization error after replacement
/// is reported even though the new destination may already be visible.
pub async fn finalize_temp(temp: &Path, dest: &Path) -> Result<()> {
    if let Err(error) = replace_synced(temp, dest).await {
        let _ = tokio::fs::remove_file(temp).await;
        return Err(error).with_context(|| format!("replacing {}", dest.display()));
    }
    Ok(())
}

// Keep blocking filesystem synchronization off the async runtime workers.
async fn replace_synced(temp: &Path, dest: &Path) -> std::io::Result<()> {
    let temp = temp.to_path_buf();
    let dest = dest.to_path_buf();
    tokio::task::spawn_blocking(move || {
        #[cfg(unix)]
        {
            std::fs::rename(&temp, &dest)?;
            let parent = dest
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            std::fs::File::open(parent)?.sync_all()?;
            let source_parent = temp
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            if source_parent != parent {
                std::fs::File::open(source_parent)?.sync_all()?;
            }
            Ok(())
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
            }
            let encode = |path: &Path| -> std::io::Result<Vec<u16>> {
                let mut value: Vec<u16> = path.as_os_str().encode_wide().collect();
                if value.contains(&0) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "path contains NUL",
                    ));
                }
                value.push(0);
                Ok(value)
            };
            // std canonicalization supplies verbatim paths, retaining long-path support.
            let existing = encode(&std::fs::canonicalize(&temp)?)?;
            let parent = dest
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            let name = dest.file_name().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "destination has no filename",
                )
            })?;
            let new = encode(&std::fs::canonicalize(parent)?.join(name))?;
            const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
            const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
            // Never unlink the destination; request completion of the move on disk.
            // SAFETY: both buffers are NUL-terminated and live for this call.
            if unsafe {
                MoveFileExW(
                    existing.as_ptr(),
                    new.as_ptr(),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            } == 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        }
    })
    .await
    .map_err(std::io::Error::other)?
}

/// Loads and parses a TOML file at `path`, or returns `T::default()` if it
/// doesn't exist yet. `what` is a human-readable label used in error
/// messages (e.g. `"app manifest"`).
pub async fn load_toml_or_default<T>(path: &Path, what: &str) -> Result<T>
where
    T: serde::de::DeserializeOwned + Default,
{
    match tokio::fs::read_to_string(path).await {
        Ok(content) => toml::from_str(&content).with_context(|| format!("failed to parse {what}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e).with_context(|| format!("failed to read {what}")),
    }
}

/// Serializes `value` as pretty TOML and atomically writes it to `path`.
/// `what` is a human-readable label used in error messages.
pub async fn save_toml_atomic<T: serde::Serialize>(
    value: &T,
    path: &Path,
    what: &str,
) -> Result<()> {
    let content =
        toml::to_string_pretty(value).with_context(|| format!("failed to serialize {what}"))?;
    atomic_write(path, content.as_bytes())
        .await
        .with_context(|| format!("failed to write {what}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn make_temp_dir(label: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("{label}-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&path).await.unwrap();
        path
    }

    #[tokio::test]
    async fn atomic_write_relative_file_syncs_the_current_directory() {
        const CHILD: &str = "SHINE_TEST_RELATIVE_PERSIST_CHILD";
        if std::env::var_os(CHILD).is_some() {
            atomic_write(Path::new("receipt.toml"), b"committed")
                .await
                .unwrap();
            assert_eq!(tokio::fs::read("receipt.toml").await.unwrap(), b"committed");
            return;
        }
        let dir = make_temp_dir("shine-relative-persist").await;
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "persist::tests::atomic_write_relative_file_syncs_the_current_directory",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn atomic_write_creates_missing_parent_directories() {
        let dir = make_temp_dir("shine-persist").await;
        let path = dir.join("nested/deep/file.txt");

        atomic_write(&path, b"hello").await.unwrap();

        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"hello");
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn atomic_write_replaces_existing_file() {
        let dir = make_temp_dir("shine-persist").await;
        let path = dir.join("file.txt");
        tokio::fs::write(&path, b"old").await.unwrap();

        atomic_write(&path, b"new").await.unwrap();

        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"new");
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn atomic_write_leaves_no_temp_file_behind_on_success() {
        let dir = make_temp_dir("shine-persist").await;
        let path = dir.join("file.txt");

        atomic_write(&path, b"content").await.unwrap();

        let mut entries = tokio::fs::read_dir(&dir).await.unwrap();
        let mut names = Vec::new();
        while let Some(entry) = entries.next_entry().await.unwrap() {
            names.push(entry.file_name());
        }
        assert_eq!(names, vec![std::ffi::OsString::from("file.txt")]);
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn atomic_write_private_uses_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = make_temp_dir("shine-persist-private").await;
        let path = dir.join("secret.env");
        tokio::fs::write(&path, b"old\n").await.unwrap();
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .await
            .unwrap();

        atomic_write_private(&path, b"TOKEN=secret\n")
            .await
            .unwrap();

        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"TOKEN=secret\n");
        let mode = tokio::fs::metadata(&path)
            .await
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn finalize_temp_removes_temp_on_rename_failure() {
        let dir = make_temp_dir("shine-persist").await;
        let temp = dir.join(".shine-write-test");
        tokio::fs::write(&temp, b"content").await.unwrap();
        // A destination inside a nonexistent directory makes rename fail.
        let dest = dir.join("missing-dir").join("dest.txt");

        let result = finalize_temp(&temp, &dest).await;

        assert!(result.is_err());
        assert!(!temp.exists(), "temp file should be cleaned up on failure");
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn failed_replacement_preserves_existing_destination() {
        let dir = make_temp_dir("shine-persist-failure").await;
        let dest = dir.join("config.toml");
        tokio::fs::write(&dest, b"retained configuration")
            .await
            .unwrap();
        // A missing staged file forces rename to fail even with a writable
        // destination. In particular, Windows must not unlink dest beforehand.
        assert!(
            finalize_temp(&dir.join("missing-stage"), &dest)
                .await
                .is_err()
        );
        assert_eq!(
            tokio::fs::read(&dest).await.unwrap(),
            b"retained configuration"
        );
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
    struct SampleToml {
        #[serde(default)]
        name: String,
        #[serde(default)]
        count: u32,
    }

    #[tokio::test]
    async fn load_toml_or_default_returns_default_when_file_missing() {
        let dir = make_temp_dir("shine-persist").await;
        let path = dir.join("sample.toml");

        let value: SampleToml = load_toml_or_default(&path, "sample").await.unwrap();

        assert_eq!(value, SampleToml::default());
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_then_load_toml_round_trips() {
        let dir = make_temp_dir("shine-persist").await;
        let path = dir.join("sample.toml");
        let value = SampleToml {
            name: "hi".to_string(),
            count: 3,
        };

        save_toml_atomic(&value, &path, "sample").await.unwrap();
        let loaded: SampleToml = load_toml_or_default(&path, "sample").await.unwrap();

        assert_eq!(loaded, value);
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_toml_atomic_creates_missing_parent_directory() {
        let dir = make_temp_dir("shine-persist").await;
        let path = dir.join("nested/sample.toml");
        let value = SampleToml::default();

        save_toml_atomic(&value, &path, "sample").await.unwrap();

        assert!(path.exists());
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
