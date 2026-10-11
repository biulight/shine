use super::{Cache, Scope};
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

/// Nonblocking OS locks. Never unlink a lock file: that would split concurrent users.
pub(super) fn lock(path: &Path) -> Result<File> {
    std::fs::create_dir_all(path.parent().context("missing maintenance directory")?)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    ensure!(
        file.metadata()?.is_file(),
        "maintenance lock is not a regular file"
    );
    file.try_lock_exclusive()?;
    Ok(file)
}

pub(super) async fn lock_wait(path: &Path) -> Result<File> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match lock(path) {
            Ok(file) => return Ok(file),
            Err(error)
                if error.downcast_ref::<std::io::Error>().is_some_and(|error| {
                    error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
                }) && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await
            }
            Err(error) => return Err(error),
        }
    }
}

pub(super) async fn load(scope: &Scope) -> Cache {
    // Missing, corrupt, foreign and old-build caches are advisory misses, never gates.
    let path = scope.root.join("checks.json");
    if let Ok(metadata) = tokio::fs::symlink_metadata(&path).await
        && metadata.is_file()
        && metadata.len() <= 1024 * 1024
        && let Ok(bytes) = tokio::fs::read(&path).await
        && let Ok(cache) = serde_json::from_slice::<Cache>(&bytes)
        && cache.schema == 1
        && cache.scope == scope.key
        && cache.build == crate::version::display()
    {
        return cache;
    }
    Cache::new(scope)
}

pub(super) async fn save(scope: &Scope, cache: &Cache) -> Result<()> {
    crate::persist::atomic_write_private(
        &scope.root.join("checks.json"),
        &serde_json::to_vec(cache)?,
    )
    .await
}
