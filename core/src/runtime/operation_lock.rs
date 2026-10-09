//! Persistent OS locks: waiting or dropping a guard never unlinks another lock.

use anyhow::{Context, Result, bail};
use fs2::FileExt;
use std::{path::Path, time::Duration};

impl<H: super::FileSystemHost> super::CoreRuntime<H> {
    pub(super) async fn acquire_app_lifecycle_operation(
        &self,
    ) -> Result<super::PrivilegedOperationGuard> {
        self.host()
            .acquire_operation_lock(&self.context().shine_dir.join("app-lifecycle.lock"))
            .await
    }
}

pub(super) async fn acquire(path: &Path) -> Result<super::PrivilegedOperationGuard> {
    acquire_with_timeout(path, Duration::from_secs(30)).await
}

async fn acquire_with_timeout(
    path: &Path,
    timeout: Duration,
) -> Result<super::PrivilegedOperationGuard> {
    let path = path.to_path_buf();
    let file = tokio::task::spawn_blocking(move || -> Result<std::fs::File> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("creating operation lock directory")?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = options.open(path).context("opening operation lock")?;
        if !file.metadata()?.is_file() {
            bail!("operation lock must be a regular file");
        }
        Ok(file)
    })
    .await??;
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(Box::new(file)),
            Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                if tokio::time::Instant::now() >= deadline {
                    bail!("timed out waiting for operation lock");
                }
                tokio::time::sleep_until(
                    deadline.min(tokio::time::Instant::now() + Duration::from_millis(50)),
                )
                .await;
            }
            Err(error) => return Err(error).context("acquiring operation lock"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn timeout_preserves_the_holder_and_release_keeps_the_same_lock_file() {
        let root = std::env::temp_dir().join(format!("shine-lock-test-{}", uuid::Uuid::new_v4()));
        let path = root.join("operation.lock");
        let first = acquire(&path).await.unwrap();
        assert!(acquire_with_timeout(&path, Duration::ZERO).await.is_err());
        assert!(path.is_file());
        drop(first);
        let second = acquire(&path).await.unwrap();
        assert!(acquire_with_timeout(&path, Duration::ZERO).await.is_err());
        drop(second);
        assert!(path.is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn process_exit_releases_the_lock_without_stale_reclamation() {
        const CHILD: &str = "SHINE_TEST_LOCK_CHILD";
        const CONTENDER: &str = "SHINE_TEST_LOCK_CONTENDER";
        if let Some(path) = std::env::var_os(CHILD) {
            if std::env::var_os(CONTENDER).is_some() {
                assert!(
                    acquire_with_timeout(Path::new(&path), Duration::ZERO)
                        .await
                        .is_err()
                );
                return;
            }
            let _guard = acquire(Path::new(&path)).await.unwrap();
            // Deliberately skip Rust destructors to model a terminated worker.
            std::process::exit(0);
        }
        let root = std::env::temp_dir().join(format!("shine-lock-exit-{}", uuid::Uuid::new_v4()));
        let path = root.join("operation.lock");
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "runtime::operation_lock::tests::process_exit_releases_the_lock_without_stale_reclamation"])
            .env(CHILD, &path)
            .status().unwrap();
        assert!(status.success());
        let guard = acquire_with_timeout(&path, Duration::ZERO).await.unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "runtime::operation_lock::tests::process_exit_releases_the_lock_without_stale_reclamation"])
            .env(CHILD, &path)
            .env(CONTENDER, "1")
            .status().unwrap();
        assert!(
            status.success(),
            "a child process stole the live parent's lock"
        );
        drop(guard);
        std::fs::remove_dir_all(root).unwrap();
    }
}
