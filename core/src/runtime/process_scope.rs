//! Cancellation and terminal ownership for an isolated process group.

pub(super) struct ProcessScope {
    group: Option<u32>,
    #[cfg(unix)]
    terminal: Option<TerminalForeground>,
}

impl ProcessScope {
    pub(super) fn new(group: Option<u32>) -> Self {
        Self {
            group,
            #[cfg(unix)]
            terminal: None,
        }
    }

    pub(super) fn inherit_terminal(&mut self) -> std::io::Result<()> {
        #[cfg(unix)]
        if let Some(group) = self.group {
            self.terminal = TerminalForeground::transfer(group as libc::pid_t)?;
        }
        Ok(())
    }

    pub(super) fn finish(&mut self) {
        self.group = None;
    }

    pub(super) fn stop(&mut self) {
        #[cfg(unix)]
        if let Some(group) = self.group.take() {
            // Retain the group identity even after wait() reaps the direct child.
            // Descendants may still own pipes or perform effects.
            unsafe { libc::kill(-(group as libc::pid_t), libc::SIGKILL) };
        }
    }
}

impl Drop for ProcessScope {
    fn drop(&mut self) {
        self.stop();
        // TerminalForeground restores the original foreground group afterwards.
    }
}

#[cfg(unix)]
struct TerminalForeground {
    original: libc::pid_t,
}

#[cfg(unix)]
impl TerminalForeground {
    fn transfer(group: libc::pid_t) -> std::io::Result<Option<Self>> {
        // A pipe has no foreground group. Do not steal a terminal from another
        // foreground owner when Shine itself was started as a background job.
        let original = unsafe { libc::tcgetpgrp(libc::STDIN_FILENO) };
        if original < 0 || original != unsafe { libc::getpgrp() } {
            return Ok(None);
        }
        set_foreground(group)?;
        let guard = Self { original };
        // The child can race the handoff and stop on SIGTTIN before it completes.
        unsafe { libc::kill(-group, libc::SIGCONT) };
        Ok(Some(guard))
    }
}

#[cfg(unix)]
impl Drop for TerminalForeground {
    fn drop(&mut self) {
        let _ = set_foreground(self.original);
    }
}

#[cfg(unix)]
fn set_foreground(group: libc::pid_t) -> std::io::Result<()> {
    // tcsetpgrp while in the background normally sends SIGTTOU. Block it only
    // on this thread for the synchronous handoff, restoring the exact mask.
    unsafe {
        let mut blocked = std::mem::zeroed();
        let mut previous = std::mem::zeroed();
        libc::sigemptyset(&mut blocked);
        libc::sigaddset(&mut blocked, libc::SIGTTOU);
        let error = libc::pthread_sigmask(libc::SIG_BLOCK, &blocked, &mut previous);
        if error != 0 {
            return Err(std::io::Error::from_raw_os_error(error));
        }
        let result = libc::tcsetpgrp(libc::STDIN_FILENO, group);
        let error = std::io::Error::last_os_error();
        libc::pthread_sigmask(libc::SIG_SETMASK, &previous, std::ptr::null_mut());
        if result == 0 { Ok(()) } else { Err(error) }
    }
}
