//! Native private creation, durable object identities, and scoped command supervision.
use super::*;
use std::fs::{File, Metadata, OpenOptions};
use unicode_normalization::UnicodeNormalization;

/// Install handlers before the first password operation. Cancellation drops staging
/// guards before returning; command execution has its own process-tree supervision.
pub(crate) async fn prepare<T>(future: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    #[cfg(unix)]
    let mut signals = [
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?,
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?,
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())?,
    ];
    #[cfg(windows)]
    let mut signals = (
        tokio::signal::windows::ctrl_c()?,
        tokio::signal::windows::ctrl_break()?,
    );
    let cancelled = async {
        #[cfg(unix)]
        {
            let [interrupt, terminate, hangup] = &mut signals;
            tokio::select! { _ = interrupt.recv() => (), _ = terminate.recv() => (), _ = hangup.recv() => () }
        }
        #[cfg(windows)]
        {
            let (interrupt, ctrl_break) = &mut signals;
            tokio::select! { _ = interrupt.recv() => (), _ = ctrl_break.recv() => () }
        }
    };
    tokio::select! {
        result = future => result,
        _ = cancelled => bail!("file-group preparation interrupted; inspect any reported cleanup failures before retrying"),
    }
}

#[cfg(windows)]
#[path = "platform/windows.rs"]
mod windows;
#[cfg(windows)]
pub(crate) use windows::{
    Identity, command_status, create_private, directory_identity, identity, is_link,
    lock_directory, lock_file, open_directory, open_read, path_identity, private_directory,
    publish_new, sync_cipher_parents, verify_acl,
};

#[cfg(unix)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Identity(u64, u64);

#[cfg(unix)]
pub(crate) fn is_link(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(unix)]
pub(crate) fn directory_identity(file: &File) -> Result<Identity> {
    use std::os::unix::fs::MetadataExt;
    let m = file.metadata()?;
    Ok(Identity(m.dev(), m.ino()))
}

#[cfg(unix)]
pub(crate) fn identity(file: &File) -> Result<Identity> {
    metadata_identity(&file.metadata()?)
}

#[cfg(unix)]
fn metadata_identity(m: &Metadata) -> Result<Identity> {
    use std::os::unix::fs::MetadataExt;
    ensure!(
        m.is_file() && m.nlink() == 1,
        "mapped file must be regular with exactly one hard link"
    );
    Ok(Identity(m.dev(), m.ino()))
}

/// Observe identity without requiring plaintext read access; never follow links.
#[cfg(unix)]
pub(crate) fn path_identity(path: &Path) -> Result<Identity> {
    metadata_identity(&fs::symlink_metadata(path)?)
}

#[cfg(unix)]
pub(crate) fn open_read(path: &Path) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    Ok(OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?)
}

#[cfg(unix)]
pub(crate) fn open_directory(path: &Path) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    Ok(OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
        .open(path)?)
}

#[cfg(unix)]
pub(crate) fn create_private(path: &Path) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    Ok(OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?)
}

#[cfg(unix)]
pub(crate) fn private_directory(path: &Path) -> Result<bool> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    let created = match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(error) => return Err(error.into()),
    };
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_dir()
            && !is_link(&meta)
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.mode() & 0o077 == 0,
        "private directory is not owner-only: {}",
        path.display()
    );
    Ok(created)
}

#[cfg(unix)]
pub(crate) fn lock_directory() -> Result<String> {
    Ok(format!("shine-file-group-locks-{}", unsafe {
        libc::geteuid()
    }))
}

#[cfg(unix)]
pub(crate) fn lock_file(path: &Path) -> Result<File> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    identity(&file)?;
    let meta = file.metadata()?;
    ensure!(
        meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o077 == 0,
        "unsafe file-group lock"
    );
    Ok(file)
}

pub(crate) fn path_key(path: &Path) -> Vec<u8> {
    // Normalize before and after case conversion so canonically equivalent
    // APFS names share both overlap checks and path locks on every platform.
    path.to_string_lossy()
        .replace('\\', "/")
        .nfd()
        .flat_map(char::to_lowercase)
        .nfd()
        .collect::<String>()
        .into_bytes()
}

#[cfg(unix)]
pub(crate) fn publish_new(temp: &Path, path: &Path) -> Result<()> {
    // Atomic no-clobber publication, followed by durable namespace synchronization.
    fs::hard_link(temp, path)?;
    fs::remove_file(temp)?;
    Ok(())
}

#[cfg(unix)]
pub(crate) fn sync_cipher_parents(root: &Path, path: &Path) -> Result<()> {
    // Newly created nested directories must also be durable before plaintext deletion.
    for parent in path
        .parent()
        .context("ciphertext parent required")?
        .ancestors()
    {
        if !parent.starts_with(root) {
            break;
        }
        open_directory(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) async fn command_status(
    command: &[OsString],
    values: &BTreeMap<String, String>,
    override_env: bool,
    explicit: &BTreeMap<String, String>,
) -> Result<std::process::ExitStatus> {
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut hangup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())?;
    let mut builder = command_builder(command, values, override_env, explicit)?;
    builder.process_group(0).kill_on_drop(true);
    let mut child = builder.spawn().context("starting file-group command")?;
    let pid = child.id().context("child PID required")? as libc::pid_t;
    // The new foreground group keeps interactive stdin usable. SIGTTOU is blocked only
    // on this thread while changing terminal ownership, not ignored process-wide.
    let terminal = Foreground::take(pid);
    if terminal.0.is_some() {
        unsafe {
            libc::kill(-pid, libc::SIGCONT);
        }
    }
    let result = tokio::select! {
        status = child.wait() => status,
        _ = interrupt.recv() => stop(&mut child, pid, libc::SIGINT).await,
        _ = terminate.recv() => stop(&mut child, pid, libc::SIGTERM).await,
        _ = hangup.recv() => stop(&mut child, pid, libc::SIGHUP).await,
    };
    // Kill any remaining non-detached descendants before removing their credential files.
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
    drop(terminal);
    result.context("waiting for file-group command")
}

#[cfg(unix)]
async fn stop(
    child: &mut tokio::process::Child,
    pid: libc::pid_t,
    signal: i32,
) -> std::io::Result<std::process::ExitStatus> {
    unsafe {
        libc::kill(-pid, signal);
    }
    match tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await {
        Ok(status) => status,
        Err(_) => {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
            child.wait().await
        }
    }
}

#[cfg(unix)]
struct Foreground(Option<libc::pid_t>);
#[cfg(unix)]
impl Foreground {
    fn take(pid: libc::pid_t) -> Self {
        let old = unsafe { libc::tcgetpgrp(libc::STDIN_FILENO) };
        let active = old > 0 && old == unsafe { libc::getpgrp() };
        if active && Self::set(pid) {
            Self(Some(old))
        } else {
            Self(None)
        }
    }
    fn set(pid: libc::pid_t) -> bool {
        unsafe {
            let mut blocked = std::mem::zeroed();
            let mut old = std::mem::zeroed();
            libc::sigemptyset(&mut blocked);
            libc::sigaddset(&mut blocked, libc::SIGTTOU);
            if libc::pthread_sigmask(libc::SIG_BLOCK, &blocked, &mut old) != 0 {
                return false;
            }
            let success = libc::tcsetpgrp(libc::STDIN_FILENO, pid) == 0;
            libc::pthread_sigmask(libc::SIG_SETMASK, &old, std::ptr::null_mut());
            success
        }
    }
}
#[cfg(unix)]
impl Drop for Foreground {
    fn drop(&mut self) {
        if let Some(pid) = self.0 {
            Self::set(pid);
        }
    }
}

#[cfg(test)]
mod path_key_tests {
    use super::*;

    #[test]
    fn unicode_and_case_aliases_share_overlap_and_lock_keys() {
        assert_eq!(
            path_key(Path::new("root/CAF\u{c9}/key")),
            path_key(Path::new("root/cafe\u{301}/key")),
        );
        assert_ne!(
            path_key(Path::new("root/cafe/key")),
            path_key(Path::new("root/caf\u{e9}/key")),
        );
    }
}
