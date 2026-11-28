//! Instrument the real in-memory host; no parallel file/backup implementation.
use super::{ReadFuture, poll_once};
use crate::runtime::*;
use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(crate) struct LockedHost {
    pub inner: InMemoryHost,
    held: Arc<AtomicBool>,
    backup: PathBuf,
    pause_restore: bool,
    paused: AtomicBool,
    resume: tokio::sync::Notify,
}
struct TrackedGuard {
    _inner: PrivilegedOperationGuard,
    held: Arc<AtomicBool>,
}
impl Drop for TrackedGuard {
    fn drop(&mut self) {
        self.held.store(false, Ordering::SeqCst);
    }
}
impl LockedHost {
    fn new(backup: PathBuf, pause_restore: bool) -> Self {
        Self {
            inner: InMemoryHost::new(),
            held: Arc::new(AtomicBool::new(false)),
            backup,
            pause_restore,
            paused: AtomicBool::new(false),
            resume: tokio::sync::Notify::new(),
        }
    }
    fn assert_locked(&self) {
        assert!(
            self.held.load(Ordering::SeqCst),
            "filesystem operation escaped privileged guard"
        );
    }
}

impl FileSystemObservationHost for LockedHost {
    fn canonicalize<'a>(&'a self, path: &'a Path) -> ReadFuture<'a, PathBuf> {
        self.inner.canonicalize(path)
    }
    fn read<'a>(&'a self, path: &'a Path) -> ReadFuture<'a, Vec<u8>> {
        Box::pin(async move {
            self.assert_locked();
            self.inner.read(path).await
        })
    }
    fn metadata<'a>(&'a self, path: &'a Path) -> ReadFuture<'a, FileMetadata> {
        Box::pin(async move {
            self.assert_locked();
            self.inner.metadata(path).await
        })
    }
    fn read_dir<'a>(&'a self, path: &'a Path) -> ReadFuture<'a, Vec<PathBuf>> {
        self.inner.read_dir(path)
    }
    fn read_link<'a>(&'a self, path: &'a Path) -> ReadFuture<'a, PathBuf> {
        self.inner.read_link(path)
    }
}

impl FileSystemHost for LockedHost {
    fn acquire_operation_lock<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<PrivilegedOperationGuard>> + Send + 'a>> {
        self.inner.acquire_operation_lock(path)
    }
    fn write_atomic<'a>(
        &'a self,
        path: &'a Path,
        bytes: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        self.inner.write_atomic(path, bytes)
    }
    fn write<'a>(
        &'a self,
        path: &'a Path,
        bytes: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        self.inner.write(path, bytes)
    }
    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        self.inner.create_dir_all(path)
    }
    fn remove_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        self.inner.remove_file(path)
    }
    fn remove_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        self.inner.remove_dir_all(path)
    }
    fn set_executable<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        self.inner.set_executable(path)
    }
    fn set_mode<'a>(
        &'a self,
        path: &'a Path,
        mode: u32,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        self.inner.set_mode(path, mode)
    }
    fn rename<'a>(
        &'a self,
        from: &'a Path,
        to: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        self.inner.rename(from, to)
    }
    fn symlink<'a>(
        &'a self,
        target: &'a Path,
        link: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        self.inner.symlink(target, link)
    }
}

impl PrivilegedFileSystemHost for LockedHost {
    fn acquire_privileged_operation<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<PrivilegedOperationGuard>> + Send + 'a>> {
        Box::pin(async move {
            let guard = self.inner.acquire_privileged_operation().await?;
            assert!(!self.held.swap(true, Ordering::SeqCst));
            Ok(Box::new(TrackedGuard {
                _inner: guard,
                held: self.held.clone(),
            }) as PrivilegedOperationGuard)
        })
    }
    fn write_privileged<'a>(
        &'a self,
        path: &'a Path,
        bytes: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            self.assert_locked();
            self.inner.write_privileged(path, bytes).await
        })
    }
    fn set_mode_privileged<'a>(
        &'a self,
        path: &'a Path,
        mode: u32,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            self.assert_locked();
            self.inner.set_mode_privileged(path, mode).await
        })
    }
    fn move_privileged<'a>(
        &'a self,
        from: &'a Path,
        to: &'a Path,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            self.assert_locked();
            if self.pause_restore && from == self.backup {
                self.paused.store(true, Ordering::SeqCst);
                self.resume.notified().await;
                self.assert_locked();
            }
            self.inner.move_privileged(from, to).await
        })
    }
    fn remove_privileged<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            self.assert_locked();
            self.inner.remove_privileged(path).await
        })
    }
}

pub(crate) async fn check_backup_boundaries(
    install: impl AsyncFn(&LockedHost, &Path) -> anyhow::Result<crate::install::InstallOutcome>,
    uninstall: impl AsyncFn(
        &LockedHost,
        &crate::install::AppEntry,
    ) -> anyhow::Result<crate::install::UninstallOutcome>,
) {
    use crate::install::{AppEntry, AppInstallStrategy, hash_content};
    let destination = std::env::temp_dir().join("shine-privileged-boundary");
    let backup = crate::install::backup_path(&destination);
    let failing_host = |pause_restore| {
        let host = LockedHost::new(backup.clone(), pause_restore);
        host.inner.put_file(&destination, b"original".to_vec());
        host.inner.fail_write_after(&destination, 0);
        host
    };
    let assert_backup_retained = async |host: &LockedHost| {
        assert!(host.inner.read(&destination).await.is_err());
        assert_eq!(host.inner.read(&backup).await.unwrap(), b"original");
    };
    // Pause inside backup restoration: prove ownership persists across the await,
    // and that cancellation releases the lock without claiming to restore bytes.
    for cancel in [false, true] {
        let host = failing_host(true);
        let mut operation = Box::pin(install(&host, &destination));
        assert!(poll_once(operation.as_mut()).await.is_pending());
        assert!(host.paused.load(Ordering::SeqCst));
        host.assert_locked();
        let mut competitor = Box::pin(host.inner.acquire_privileged_operation());
        assert!(poll_once(competitor.as_mut()).await.is_pending());
        if cancel {
            drop(operation);
        } else {
            host.resume.notify_one();
            assert_eq!(operation.await.unwrap_err().to_string(), "privileged write");
        }
        assert!(!host.held.load(Ordering::SeqCst));
        let guard = competitor.await.unwrap();
        if cancel {
            assert_backup_retained(&host).await;
        } else {
            assert_eq!(host.inner.read(&destination).await.unwrap(), b"original");
            assert!(host.inner.read(&backup).await.is_err());
        }
        drop(guard);
    }
    // A failed rollback retains the backup and the original write error.
    let host = failing_host(false);
    host.inner.fail_rename_after(&backup, &destination, 0);
    assert_eq!(
        install(&host, &destination).await.unwrap_err().to_string(),
        "privileged write"
    );
    assert!(!host.held.load(Ordering::SeqCst));
    assert_backup_retained(&host).await;

    // Uninstall restoration errors propagate after removal, preserving backup bytes.
    host.inner.put_file(&destination, b"managed".to_vec());
    host.inner.fail_rename_after(&backup, &destination, 0);
    let entry = AppEntry {
        source: "demo/config".into(),
        destination: destination.clone(),
        backup: Some(backup.clone()),
        content_hash: hash_content(b"managed"),
        install_strategy: AppInstallStrategy::Copy,
        uses_env: false,
        requires_admin: true,
    };
    assert_eq!(
        uninstall(&host, &entry).await.unwrap_err().to_string(),
        "privileged move"
    );
    assert!(!host.held.load(Ordering::SeqCst));
    assert_backup_retained(&host).await;
    let _guard = host.inner.acquire_privileged_operation().await.unwrap();
}
