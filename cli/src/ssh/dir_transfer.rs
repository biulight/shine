//! Directory transfer support (PRD phase 2): a directory is staged as an
//! uncompressed tar archive in a local temp file — never buffered fully in
//! memory — then transferred using the same byte-streaming primitive as
//! single files, and extracted from that temp file on the other end.
//!
//! Extraction is defense-in-depth per docs/ssh-local-transfer-prd.md
//! section 6.2: `tar::Entry::unpack_in` already rejects absolute paths and
//! `..` traversal in an entry's own path (the same logic cargo/rustup rely
//! on), but it does **not** validate where a *symlink's target* points, so
//! that check is added here to enforce the chosen policy: accept symlinks
//! that stay relative and resolve inside the transferred tree, reject
//! absolute or escaping ones. Non-file/dir/symlink entry types (hard
//! links, device nodes, FIFOs) are rejected outright.

use anyhow::{Context, Result, bail};
use std::path::{Component, Path, PathBuf};

/// Builds an uncompressed tar archive of `source_dir`'s contents into a
/// fresh temp file, on a blocking thread (directory walking and tar I/O are
/// synchronous). Returns the temp file's path and byte size.
pub async fn build_tar_to_temp_file(source_dir: PathBuf) -> Result<(PathBuf, u64)> {
    tokio::task::spawn_blocking(move || build_tar_sync(&source_dir))
        .await
        .context("tar-build task panicked")?
}

fn build_tar_sync(source_dir: &Path) -> Result<(PathBuf, u64)> {
    let temp_path =
        std::env::temp_dir().join(format!("shine-ssh-dir-{}.tar", uuid::Uuid::new_v4()));
    let file = std::fs::File::create(&temp_path)
        .with_context(|| format!("creating {}", temp_path.display()))?;
    let mut builder = tar::Builder::new(file);
    // Preserve symlinks as symlink entries instead of copying through the
    // files they point to; extraction policy (below) then decides which
    // symlinks are safe to recreate.
    builder.follow_symlinks(false);
    builder
        .append_dir_all(".", source_dir)
        .with_context(|| format!("archiving {}", source_dir.display()))?;
    builder.finish().context("finishing tar archive")?;
    drop(builder);

    let size = std::fs::metadata(&temp_path)
        .with_context(|| format!("stat {}", temp_path.display()))?
        .len();
    Ok((temp_path, size))
}

/// Extracts the tar archive at `tar_path` into `dest_dir` (created if
/// missing), on a blocking thread. Rejects the whole transfer on the first
/// unsafe or unsupported entry.
pub async fn extract_tar_from_file(tar_path: PathBuf, dest_dir: PathBuf) -> Result<()> {
    tokio::task::spawn_blocking(move || extract_tar_sync(&tar_path, &dest_dir))
        .await
        .context("tar-extract task panicked")?
}

fn extract_tar_sync(tar_path: &Path, dest_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dest_dir)
        .with_context(|| format!("creating {}", dest_dir.display()))?;

    let file =
        std::fs::File::open(tar_path).with_context(|| format!("opening {}", tar_path.display()))?;
    let mut archive = tar::Archive::new(file);

    for entry in archive.entries().context("reading tar archive entries")? {
        let mut entry = entry.context("reading tar entry")?;
        let entry_type = entry.header().entry_type();
        let entry_path = entry
            .path()
            .context("invalid path in tar entry")?
            .into_owned();

        if entry_type.is_symlink() {
            let link_target = entry
                .link_name()
                .context("invalid link name in tar entry")?
                .with_context(|| format!("symlink entry {} has no target", entry_path.display()))?
                .into_owned();
            if !symlink_target_is_safe(&entry_path, &link_target) {
                bail!(
                    "refusing to extract {}: symlink target {} escapes the transferred directory",
                    entry_path.display(),
                    link_target.display()
                );
            }
        } else if !(entry_type.is_file() || entry_type.is_contiguous() || entry_type.is_dir()) {
            bail!(
                "refusing to extract {}: unsupported entry type {:?} (only files, directories, and safe symlinks are allowed)",
                entry_path.display(),
                entry_type
            );
        }

        let unpacked = entry
            .unpack_in(dest_dir)
            .with_context(|| format!("extracting {}", entry_path.display()))?;
        if !unpacked {
            bail!(
                "refusing to extract {}: unsafe path (absolute or contains `..`)",
                entry_path.display()
            );
        }
    }

    Ok(())
}

/// Returns whether a symlink entry at `entry_path` pointing at
/// `link_target` stays within the transferred directory tree: `link_target`
/// must not be absolute, and walking it (relative to `entry_path`'s
/// containing directory) must never need to pop above the archive root.
/// Purely lexical — the target need not exist yet during extraction.
fn symlink_target_is_safe(entry_path: &Path, link_target: &Path) -> bool {
    if link_target.is_absolute() {
        return false;
    }

    let mut stack: Vec<&std::ffi::OsStr> = entry_path
        .parent()
        .into_iter()
        .flat_map(Path::components)
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name),
            _ => None,
        })
        .collect();

    for component in link_target.components() {
        match component {
            Component::Normal(name) => stack.push(name),
            Component::ParentDir => {
                if stack.pop().is_none() {
                    return false;
                }
            }
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_symlink_within_the_tree_is_safe() {
        assert!(symlink_target_is_safe(
            Path::new("subdir/link"),
            Path::new("../sibling/file.txt")
        ));
        assert!(symlink_target_is_safe(
            Path::new("link"),
            Path::new("file.txt")
        ));
    }

    #[test]
    fn absolute_symlink_target_is_unsafe() {
        assert!(!symlink_target_is_safe(
            Path::new("subdir/link"),
            Path::new("/etc/passwd")
        ));
    }

    #[test]
    fn symlink_target_escaping_the_root_is_unsafe() {
        assert!(!symlink_target_is_safe(
            Path::new("link"),
            Path::new("../outside.txt")
        ));
        assert!(!symlink_target_is_safe(
            Path::new("subdir/link"),
            Path::new("../../../etc/passwd")
        ));
    }

    #[test]
    fn build_then_extract_round_trip_preserves_nested_files_and_safe_symlinks() {
        let source = TestDir::new();
        std::fs::create_dir_all(source.path().join("nested")).unwrap();
        std::fs::write(source.path().join("nested/file.txt"), b"hello").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("nested/file.txt", source.path().join("link.txt")).unwrap();

        let (tar_path, size) =
            block_on(build_tar_to_temp_file(source.path().to_path_buf())).unwrap();
        assert!(size > 0);

        let dest = TestDir::new();
        block_on(extract_tar_from_file(tar_path, dest.path().to_path_buf())).unwrap();

        assert_eq!(
            std::fs::read(dest.path().join("nested/file.txt")).unwrap(),
            b"hello"
        );
        #[cfg(unix)]
        {
            let link_target = std::fs::read_link(dest.path().join("link.txt")).unwrap();
            assert_eq!(link_target, Path::new("nested/file.txt"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn build_then_extract_rejects_a_symlink_that_escapes_the_tree() {
        let source = TestDir::new();
        std::fs::write(source.path().join("real.txt"), b"data").unwrap();
        std::os::unix::fs::symlink("../../../etc/passwd", source.path().join("evil")).unwrap();

        let (tar_path, _size) =
            block_on(build_tar_to_temp_file(source.path().to_path_buf())).unwrap();

        let dest = TestDir::new();
        let result = block_on(extract_tar_from_file(tar_path, dest.path().to_path_buf()));
        assert!(result.is_err());
    }

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("shine-ssh-dt-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Runs a future to completion on a throwaway current-thread runtime,
    /// so these tests can stay plain `#[test]` (the functions under test
    /// spawn onto a blocking pool, which needs a runtime to exist).
    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }
}
