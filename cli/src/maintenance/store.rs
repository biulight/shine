use super::{Cache, Scope};
use anyhow::{Context, Result, ensure};
use std::{
    fs::{File, OpenOptions, TryLockError},
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
    file.try_lock()?;
    Ok(file)
}

pub(super) async fn lock_wait(path: &Path) -> Result<File> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match lock(path) {
            Ok(file) => return Ok(file),
            Err(error)
                if matches!(error.downcast_ref(), Some(TryLockError::WouldBlock))
                    && tokio::time::Instant::now() < deadline =>
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn contention_waits_for_release_and_preserves_the_lock_file() {
        let root = crate::test_support::make_temp_dir("shine-maintenance-lock").await;
        let path = root.join("state.lock");
        let holder = lock(&path).unwrap();
        let waiter = lock_wait(&path);
        tokio::pin!(waiter);
        assert!(
            tokio::time::timeout(Duration::from_millis(30), &mut waiter)
                .await
                .is_err()
        );
        drop(holder);
        let guard = waiter.await.unwrap();
        assert!(matches!(
            lock(&path).unwrap_err().downcast_ref(),
            Some(TryLockError::WouldBlock)
        ));
        drop(guard);
        assert!(path.is_file());
        drop(lock(&path).unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn contention_timeout_and_io_failure_remain_distinct() {
        let root = crate::test_support::make_temp_dir("shine-maintenance-lock-errors").await;
        let path = root.join("state.lock");
        let holder = lock(&path).unwrap();
        let error = lock_wait(&path).await.unwrap_err();
        assert!(matches!(
            error.downcast_ref(),
            Some(TryLockError::WouldBlock)
        ));
        assert!(path.is_file());
        let error = lock_wait(&root).await.unwrap_err();
        assert!(error.downcast_ref::<std::io::Error>().is_some());
        drop(holder);
        std::fs::remove_dir_all(root).unwrap();
    }
}
