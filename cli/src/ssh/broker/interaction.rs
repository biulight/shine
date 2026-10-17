//! Local terminal interaction; authorization and nonce ownership remain in BrokerSession.
#[cfg(unix)]
use anyhow::Context;
use anyhow::{Result, bail};
use std::{future::Future, pin::Pin, sync::Mutex};

type InteractionFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

pub(super) trait BrokerInteraction: Send + Sync {
    fn set_ssh_pid(&self, pid: Option<u32>);
    fn confirm_prompt(&self, prompt: String) -> InteractionFuture<'_, LocalConfirmation>;
    fn display_local(&self, contents: String) -> InteractionFuture<'_, ()>;
}

/// Ownership keeps SSH paused and its TTY restored through decryption or enrollment persistence.
#[must_use]
pub(super) struct LocalConfirmation {
    _restore: Box<dyn Send>,
}
impl LocalConfirmation {
    #[cfg(any(unix, test))]
    pub(super) fn hold(restore: impl Send + 'static) -> Self {
        Self {
            _restore: Box::new(restore),
        }
    }
}

pub(super) struct TtyInteraction {
    ssh_pid: Mutex<Option<u32>>,
    #[cfg(unix)]
    original_termios: Option<libc::termios>,
}
impl TtyInteraction {
    pub(super) fn capture() -> Self {
        Self {
            ssh_pid: Mutex::new(None),
            #[cfg(unix)]
            original_termios: capture_tty_termios(),
        }
    }
}
impl BrokerInteraction for TtyInteraction {
    fn set_ssh_pid(&self, pid: Option<u32>) {
        *self.ssh_pid.lock().expect("ssh pid mutex poisoned") = pid;
    }
    fn confirm_prompt(&self, prompt: String) -> InteractionFuture<'_, LocalConfirmation> {
        Box::pin(async move {
            let pid = *self.ssh_pid.lock().expect("ssh pid mutex poisoned");
            #[cfg(unix)]
            {
                let original = self
                .original_termios
                .context("local TTY confirmation is unavailable; use a TTY or a matching workspace policy with --trust-remote-session")?;
                tokio::task::spawn_blocking(move || confirm_on_tty(pid, original, &prompt))
                    .await
                    .context("joining local broker confirmation")?
            }
            #[cfg(not(unix))]
            {
                let _ = (pid, prompt);
                bail!("interactive SSH secret confirmation is not supported on this platform")
            }
        })
    }

    fn display_local(&self, contents: String) -> InteractionFuture<'_, ()> {
        Box::pin(async move {
            let pid = *self.ssh_pid.lock().expect("ssh pid mutex poisoned");
            #[cfg(unix)]
            {
                let original = self.original_termios.context(
                    "local TTY display is unavailable; run inspect from an interactive terminal",
                )?;
                tokio::task::spawn_blocking(move || display_on_tty(pid, original, &contents))
                    .await
                    .context("joining local broker display")?
            }
            #[cfg(not(unix))]
            {
                let _ = (pid, contents);
                bail!("interactive SSH broker display is not supported on this platform")
            }
        })
    }
}

#[cfg(unix)]
struct TtyRestore {
    tty: std::fs::File,
    pid: u32,
    termios: libc::termios,
}

#[cfg(unix)]
impl Drop for TtyRestore {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        unsafe {
            libc::tcsetattr(self.tty.as_raw_fd(), libc::TCSANOW, &self.termios);
            libc::kill(self.pid as i32, libc::SIGCONT);
        }
    }
}

#[cfg(unix)]
fn capture_tty_termios() -> Option<libc::termios> {
    use std::os::fd::AsRawFd;
    let tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()?;
    let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
    let rc = unsafe { libc::tcgetattr(tty.as_raw_fd(), termios.as_mut_ptr()) };
    (rc == 0).then(|| unsafe { termios.assume_init() })
}

#[cfg(unix)]
fn confirm_on_tty(
    pid: Option<u32>,
    original: libc::termios,
    prompt: &str,
) -> Result<LocalConfirmation> {
    use std::io::{BufRead, Write};
    let mut restore = enter_local_tty(pid, original)?;
    restore.tty.write_all(prompt.as_bytes())?;
    restore.tty.flush()?;
    let mut answer = String::new();
    std::io::BufReader::new(restore.tty.try_clone()?).read_line(&mut answer)?;
    if !confirmation_approved(&answer) {
        bail!("secret request rejected by the local user");
    }
    Ok(LocalConfirmation::hold(restore))
}

#[cfg(unix)]
fn display_on_tty(pid: Option<u32>, original: libc::termios, contents: &str) -> Result<()> {
    use std::io::Write;
    let mut restore = enter_local_tty(pid, original)?;
    restore.tty.write_all(contents.as_bytes())?;
    restore.tty.flush()?;
    Ok(())
}

#[cfg(unix)]
fn enter_local_tty(pid: Option<u32>, original: libc::termios) -> Result<TtyRestore> {
    use std::os::fd::AsRawFd;
    let pid = pid.context("SSH child is not running")?;
    let tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .context("opening /dev/tty for local broker interaction")?;
    let fd = tty.as_raw_fd();
    let mut ssh_termios = std::mem::MaybeUninit::<libc::termios>::uninit();
    if unsafe { libc::tcgetattr(fd, ssh_termios.as_mut_ptr()) } != 0 {
        bail!("reading current TTY state");
    }
    let ssh_termios = unsafe { ssh_termios.assume_init() };
    if unsafe { libc::kill(pid as i32, libc::SIGSTOP) } != 0 {
        bail!("pausing SSH for local broker interaction");
    }
    let restore = TtyRestore {
        tty,
        pid,
        termios: ssh_termios,
    };
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &original) } != 0 {
        bail!("restoring canonical TTY state for local broker interaction");
    }
    Ok(restore)
}

#[cfg(any(unix, test))]
pub(super) fn confirmation_approved(answer: &str) -> bool {
    const PASTE_START: &str = "\u{1b}[200~";
    const PASTE_END: &str = "\u{1b}[201~";

    let answer = answer.trim();
    let answer = if let Some(pasted) = answer.strip_prefix(PASTE_START) {
        let Some(pasted) = pasted.strip_suffix(PASTE_END) else {
            return false;
        };
        pasted.trim()
    } else {
        answer
    };
    matches!(answer.to_ascii_lowercase().as_str(), "y" | "yes")
}

pub(super) fn raw_tty_safe_log(message: &str) {
    use std::io::{IsTerminal, Write};
    let mut stderr = std::io::stderr();
    if stderr.is_terminal() {
        let _ = write!(stderr, "\r{message}\r\n");
    } else {
        let _ = writeln!(stderr, "{message}");
    }
}
