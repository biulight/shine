//! OSC 11 terminal query over `/dev/tty`, unix-only (mirrors the
//! `#[cfg(unix)]` split used by `ssh::remote_client` — Windows has no
//! `/dev/tty` or POSIX termios, and this path is a compatibility fallback
//! that `shine ssh` (§6.1) never needs on Windows).
//!
//! The read loop is the fix for the bug documented in
//! `docs/kb/lessons.md` (2026-07-14): the superseded shell implementation
//! gave the first byte 150ms but the *inter-byte* gap only 10ms, so a
//! response split across two network reads (common over SSH) was abandoned
//! mid-read, echo was restored, and the tail arrived at the now-echoing tty.
//! This implementation tracks a single **total deadline** with `poll(2)`,
//! recomputing the remaining time on every iteration — there is no
//! per-byte timeout to violate.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::fd::{AsRawFd, RawFd};
use std::time::{Duration, Instant};

use super::color::{Theme, parse_osc_rgb, theme_from_rgb};

/// OSC 11 "report background color" query.
const OSC_QUERY: &[u8] = b"\x1b]11;?\x1b\\";
/// String (ST) terminator shared by the query and its response.
const TERMINATOR: &[u8] = b"\x1b\\";
/// Bounds the response buffer regardless of deadline — a well-formed
/// `rgb:RRRR/GGGG/BBBB` response is at most ~24 bytes; 64 leaves generous
/// headroom without letting a misbehaving terminal grow the buffer unbounded.
const MAX_RESPONSE_LEN: usize = 64;

/// RAII guard: disables tty `ECHO` on `fd` for the guard's lifetime and
/// restores the original termios settings on drop — including on an early
/// return or panic-unwind, unlike a shell script's manual restore-before-
/// every-`return`, which is exactly the class of bug this replaces (PRD §10:
/// "tty 状态必须使用 guard/清理路径恢复").
struct EchoGuard {
    fd: RawFd,
    original: libc::termios,
}

impl EchoGuard {
    /// Returns `None` if `fd` is not a tty or termios can't be read/set;
    /// callers treat that as "can't query, skip OSC" (PRD §6.2).
    fn disable(fd: RawFd) -> Option<Self> {
        // SAFETY: `original` is a plain-old-data struct; zero-initializing it
        // is valid before `tcgetattr` fully populates it.
        let mut original: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: `fd` is caller-provided and assumed open for the duration
        // of this call; `original` is a valid out-pointer sized for `termios`.
        if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
            return None;
        }
        let mut raw = original;
        raw.c_lflag &= !libc::ECHO;
        // SAFETY: `fd` is open; `raw` is a valid in-pointer sized for `termios`.
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
            return None;
        }
        Some(Self { fd, original })
    }
}

impl Drop for EchoGuard {
    fn drop(&mut self) {
        // SAFETY: `fd` was validated open when this guard was constructed in
        // `disable`. Best-effort restore: a failure here is unrecoverable and
        // must not panic inside `Drop`, so the result is intentionally
        // discarded (PRD §10: never leave the tty with echo off).
        unsafe {
            let _ = libc::tcsetattr(self.fd, libc::TCSANOW, &self.original);
        }
    }
}

/// Blocks up to `timeout` for `fd` to become readable. Returns `Ok(false)`
/// on timeout (not an error — the deadline loop treats it as "stop here").
fn poll_readable(fd: RawFd, timeout: Duration) -> std::io::Result<bool> {
    let mut fds = [libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    }];
    let timeout_ms = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
    // SAFETY: `fds` points to a valid one-element array for the duration of
    // this call; `poll` does not retain the pointer afterward.
    let ret = unsafe { libc::poll(fds.as_mut_ptr(), 1, timeout_ms) };
    if ret < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(ret > 0 && fds[0].revents & libc::POLLIN != 0)
}

/// Reads bytes from `fd` until the OSC terminator (`ESC \`) is seen or
/// `deadline` passes — see the module docs for why this must be a single
/// total deadline rather than a per-byte timeout. Silently truncates at
/// [`MAX_RESPONSE_LEN`] or on any read error; the caller treats a
/// non-terminated response as invalid (PRD §6.2: "半包...均静默失败").
pub(super) fn read_until_terminator_or_deadline(fd: RawFd, deadline: Instant) -> Vec<u8> {
    let mut response = Vec::with_capacity(32);
    loop {
        if response.len() >= MAX_RESPONSE_LEN {
            break;
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        match poll_readable(fd, remaining) {
            Ok(true) => {}
            Ok(false) | Err(_) => break,
        }
        let mut byte = [0u8; 1];
        // SAFETY: `fd` is open for the duration of this call (caller
        // contract); `byte` is a valid one-byte buffer for `read` to fill.
        let n = unsafe { libc::read(fd, byte.as_mut_ptr().cast(), 1) };
        if n <= 0 {
            break; // EOF or error: stop, do not retry.
        }
        response.push(byte[0]);
        if response.ends_with(TERMINATOR) {
            break;
        }
    }
    response
}

fn parse_osc_response(response: &[u8]) -> Option<Theme> {
    let text = std::str::from_utf8(response).ok()?;
    let body = text
        .strip_prefix("\x1b]")?
        .strip_prefix("11;")?
        .strip_suffix("\x1b\\")?;
    let (r, g, b) = parse_osc_rgb(body)?;
    Some(theme_from_rgb(r, g, b))
}

/// `TERM` values under which OSC queries are skipped outright — terminal
/// multiplexers require DCS passthrough for OSC to reach the real terminal,
/// which this implementation does not attempt (PRD §4 non-goal).
fn is_multiplexer_term() -> bool {
    std::env::var("TERM")
        .map(|term| term.starts_with("screen") || term.starts_with("tmux"))
        .unwrap_or(false)
}

/// Queries the terminal on `/dev/tty` for its background color via OSC 11
/// and classifies it light/dark. `budget` is the *total* time allowed for
/// the whole exchange (write + read), enforced as a single deadline.
/// Returns `None` on any failure — not a tty, closed pty, malformed or
/// partial response, or timeout — all silent per PRD §6.2/§10. Never prints
/// the raw response anywhere.
pub fn query_terminal_theme(budget: Duration) -> Option<Theme> {
    if is_multiplexer_term() {
        return None;
    }
    let mut tty = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()?;
    let fd = tty.as_raw_fd();
    let _echo_guard = EchoGuard::disable(fd)?;

    let deadline = Instant::now() + budget;
    tty.write_all(OSC_QUERY).ok()?;
    tty.flush().ok()?;

    let response = read_until_terminator_or_deadline(fd, deadline);
    parse_osc_response(&response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;
    use std::thread;

    #[test]
    fn parse_osc_response_extracts_theme_from_well_formed_reply() {
        let response = b"\x1b]11;rgb:ffff/ffff/ffff\x1b\\";
        assert_eq!(parse_osc_response(response), Some(Theme::Light));
    }

    #[test]
    fn parse_osc_response_rejects_missing_terminator_or_prefix() {
        assert_eq!(parse_osc_response(b"11;rgb:ffff/ffff/ffff\x1b\\"), None);
        assert_eq!(parse_osc_response(b"\x1b]11;rgb:ffff/ffff/ffff"), None);
        assert_eq!(parse_osc_response(b""), None);
    }

    /// Reproduces the exact PRD §2.1 matrix — a duplex byte stream (not a
    /// real pty) driving the same `poll`+`read` deadline logic that runs
    /// against `/dev/tty` in production. This is faithful for what actually
    /// matters here (the total-deadline read policy, which doesn't care
    /// whether the fd is a socket or a tty) while staying deterministic and
    /// CI-safe. tty-specific echo control is covered separately by
    /// `EchoGuard`; real end-to-end tty behavior was verified manually
    /// against a live terminal during the PRD's own root-cause investigation.
    fn run_matrix_case(
        chunks: Vec<(&'static [u8], Duration)>,
        deadline_budget: Duration,
    ) -> Vec<u8> {
        let (rx, tx) = UnixStream::pair().expect("unix socket pair");
        let writer = thread::spawn(move || {
            let mut tx = tx;
            for (chunk, gap) in chunks {
                thread::sleep(gap);
                let _ = tx.write_all(chunk);
            }
        });

        let fd = rx.as_raw_fd();
        let deadline = Instant::now() + deadline_budget;
        let result = read_until_terminator_or_deadline(fd, deadline);
        writer.join().unwrap();
        result
    }

    const FULL_REPLY: &[u8] = b"\x1b]11;rgb:ffff/ffff/ffff\x1b\\";

    #[test]
    fn matrix_whole_packet_reads_complete_response() {
        let result = run_matrix_case(
            vec![(FULL_REPLY, Duration::ZERO)],
            Duration::from_millis(200),
        );
        assert_eq!(result, FULL_REPLY);
    }

    #[test]
    fn matrix_fragmented_50ms_gap_still_reads_complete_response() {
        // This is the case the superseded shell implementation got wrong:
        // a fragment arriving well past any *inter-byte* timeout (its 10ms)
        // must still be captured because the deadline here is total, not
        // per-byte.
        let result = run_matrix_case(
            vec![
                (&FULL_REPLY[..2], Duration::ZERO), // "\x1b]"
                (&FULL_REPLY[2..], Duration::from_millis(50)),
            ],
            Duration::from_millis(200),
        );
        assert_eq!(
            result, FULL_REPLY,
            "a 50ms-fragmented reply must be read in full under a 200ms total deadline"
        );
    }

    #[test]
    fn matrix_fragmented_5ms_gap_reads_complete_response() {
        let result = run_matrix_case(
            vec![
                (&FULL_REPLY[..2], Duration::ZERO),
                (&FULL_REPLY[2..], Duration::from_millis(5)),
            ],
            Duration::from_millis(200),
        );
        assert_eq!(result, FULL_REPLY);
    }

    #[test]
    fn matrix_no_reply_returns_empty_at_deadline() {
        let start = Instant::now();
        let result = run_matrix_case(vec![], Duration::from_millis(80));
        assert!(result.is_empty());
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "no-reply path must not hang past its deadline"
        );
    }

    #[test]
    fn matrix_reply_fragmented_beyond_total_deadline_is_truncated_not_leaked() {
        // A fragment arriving *after* the total deadline must simply not be
        // read (the function returns at the deadline) — it must never be
        // echoed by a caller that has already restored tty state, which is
        // exactly the bug this module fixes.
        let result = run_matrix_case(
            vec![
                (&FULL_REPLY[..2], Duration::ZERO),
                (&FULL_REPLY[2..], Duration::from_millis(300)),
            ],
            Duration::from_millis(50),
        );
        assert_eq!(result, &FULL_REPLY[..2]);
    }

    #[test]
    fn echo_guard_fails_closed_on_a_non_tty_fd() {
        let (a, _b) = UnixStream::pair().expect("unix socket pair");
        let fd = a.as_raw_fd();
        // A UnixStream is not a tty, so EchoGuard::disable must fail closed
        // (tcgetattr returns ENOTTY) rather than silently doing nothing.
        assert!(EchoGuard::disable(fd).is_none());
    }
}
