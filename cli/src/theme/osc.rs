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

/// RAII guard: puts the tty on `fd` into a minimal query mode — `ECHO` **and**
/// `ICANON` (canonical/line mode) disabled — for the guard's lifetime, and
/// restores the original termios settings on drop, including on an early return
/// or panic-unwind (PRD §10: "tty 状态必须使用 guard/清理路径恢复").
///
/// Clearing `ICANON` is essential, not cosmetic: an OSC 11 reply
/// (`ESC ] 11 ; rgb:… ESC \`) carries **no newline**, and in canonical mode the
/// line discipline withholds a newline-less line from `read`/`poll` until a
/// newline arrives — so the reply is never delivered to the read loop, which
/// then times out having read nothing, restores the tty, and lets the buffered
/// reply leak into the next prompt. Disabling `ECHO` alone (the prior behavior)
/// does not help: the reply is withheld regardless of echo. Reproduced against a
/// real pty in `read_loop_reads_newline_free_response_through_pty`; observed in
/// production on Ghostty, whose OSC 11 reply never surfaced under canonical mode.
struct TtyQueryGuard {
    fd: RawFd,
    original: libc::termios,
}

impl TtyQueryGuard {
    /// Returns `None` if `fd` is not a tty or termios can't be read/set;
    /// callers treat that as "can't query, skip OSC" (PRD §6.2).
    fn enter(fd: RawFd) -> Option<Self> {
        // SAFETY: `original` is a plain-old-data struct; zero-initializing it
        // is valid before `tcgetattr` fully populates it.
        let mut original: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: `fd` is caller-provided and assumed open for the duration
        // of this call; `original` is a valid out-pointer sized for `termios`.
        if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
            return None;
        }
        let mut raw = original;
        raw.c_lflag &= !(libc::ECHO | libc::ICANON);
        // In non-canonical mode `c_cc[VMIN]`/`[VTIME]` alias the same array
        // indices as the canonical `VEOF`/`VEOL` control chars, so they hold
        // meaningless values unless set explicitly. VMIN=1, VTIME=0 = "return as
        // soon as ≥1 byte is available", matching the poll-then-read(1) loop;
        // leaving them would inherit VEOF (0x04 = 4) as VMIN and stall a
        // fragmented reply until 4 bytes accrued.
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        // SAFETY: `fd` is open; `raw` is a valid in-pointer sized for `termios`.
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
            return None;
        }
        Some(Self { fd, original })
    }
}

impl Drop for TtyQueryGuard {
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
    let _tty_guard = TtyQueryGuard::enter(fd)?;

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
    fn tty_query_guard_fails_closed_on_a_non_tty_fd() {
        let (a, _b) = UnixStream::pair().expect("unix socket pair");
        let fd = a.as_raw_fd();
        // A UnixStream is not a tty, so TtyQueryGuard::enter must fail closed
        // (tcgetattr returns ENOTTY) rather than silently doing nothing.
        assert!(TtyQueryGuard::enter(fd).is_none());
    }

    /// Regression test for the Ghostty leak: opens a *real* pty (which — unlike
    /// a `UnixStream` — has a line discipline, so canonical mode actually
    /// applies) in its default canonical+echo state, then verifies the guard
    /// puts it in a mode where a newline-free OSC 11 reply written to the master
    /// is delivered to the read loop in full. Before the ICANON fix this read
    /// nothing and returned empty at the deadline — the exact production leak.
    #[test]
    fn read_loop_reads_newline_free_response_through_pty() {
        let mut master: RawFd = -1;
        let mut slave: RawFd = -1;
        // SAFETY: `openpty` writes two valid fds into the out-params; the
        // termios/winsize in-params are null, so the kernel picks its defaults
        // (canonical mode + echo — the state that caused the leak).
        let rc = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(rc, 0, "openpty failed: {}", std::io::Error::last_os_error());

        // Confirm the fixture reproduces the buggy precondition.
        let mut before: libc::termios = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { libc::tcgetattr(slave, &mut before) }, 0);
        assert!(
            before.c_lflag & libc::ICANON != 0,
            "a fresh pty slave must start in canonical mode"
        );

        let reply = b"\x1b]11;rgb:ffff/ffff/ffff\x1b\\";
        let got = {
            let _guard = TtyQueryGuard::enter(slave).expect("guard on a real tty");

            let mut during: libc::termios = unsafe { std::mem::zeroed() };
            assert_eq!(unsafe { libc::tcgetattr(slave, &mut during) }, 0);
            assert_eq!(during.c_lflag & libc::ICANON, 0, "ICANON must be cleared");
            assert_eq!(during.c_lflag & libc::ECHO, 0, "ECHO must be cleared");

            // Deliver the newline-free reply to the slave via the master.
            // SAFETY: `master` is an open fd; `reply` is a valid byte buffer.
            let n = unsafe { libc::write(master, reply.as_ptr().cast(), reply.len()) };
            assert_eq!(n, reply.len() as isize, "short write to pty master");

            let deadline = Instant::now() + Duration::from_millis(500);
            read_until_terminator_or_deadline(slave, deadline)
        };

        assert_eq!(
            got, reply,
            "newline-free OSC reply must be read in full once ICANON is cleared"
        );

        // The guard restored canonical mode on drop.
        let mut after: libc::termios = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { libc::tcgetattr(slave, &mut after) }, 0);
        assert!(
            after.c_lflag & libc::ICANON != 0,
            "ICANON must be restored after the guard drops"
        );

        // SAFETY: both fds were opened by `openpty` above and not yet closed.
        unsafe {
            libc::close(master);
            libc::close(slave);
        }
    }
}
