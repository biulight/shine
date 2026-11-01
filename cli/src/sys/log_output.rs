//! Stateful, bounded filtering of installer terminal styles. Never forward terminal commands.
use super::execution::installer_color_enabled;
use crate::{colors, output::escape_controls};
use std::{
    io::{self, IsTerminal, Write},
    time::{Duration, Instant},
};

const MAX_ROW_BYTES: usize = 64 * 1024;
const REFRESH_INTERVAL: Duration = Duration::from_millis(80);

#[derive(Default)]
struct StreamOutput {
    log: StyledLog,
    pending: String,
    replace: bool,
}

#[derive(Default)]
pub(super) struct InstallerOutput {
    item: String,
    streams: [StreamOutput; 2],
    active: Option<usize>,
    dirty: bool,
    last_draw: Option<Instant>,
}

struct Surface {
    terminal: bool,
    dynamic: bool,
    color: bool,
    columns: usize,
}

impl Surface {
    fn current() -> Self {
        Self {
            terminal: std::io::stdout().is_terminal(),
            dynamic: std::io::stdout().is_terminal()
                && std::env::var("TERM").as_deref() != Ok("dumb"),
            color: installer_color_enabled(),
            columns: console::Term::stdout().size().1.into(),
        }
    }
}

pub(super) struct Update<'a> {
    pub item: &'a str,
    pub stream: &'a str,
    pub text: &'a str,
    pub code: &'a str,
}

impl InstallerOutput {
    pub(super) fn write(&mut self, writer: &mut impl Write, update: Update<'_>) -> io::Result<()> {
        self.update(writer, update, &Surface::current())
    }

    fn update(
        &mut self,
        writer: &mut impl Write,
        update: Update<'_>,
        surface: &Surface,
    ) -> io::Result<()> {
        let surface = &Surface {
            terminal: surface.terminal,
            dynamic: surface.dynamic
                && surface.columns > console::measure_text_width(update.item) + 8,
            color: surface.color,
            columns: surface.columns,
        };
        if self.item != update.item {
            self.finish_on(writer, surface)?;
            self.item = update.item.into();
        }
        if !surface.dynamic
            && let Some(active) = self.active
        {
            self.commit(writer, active, surface)?;
        }
        if update.code == "sys_bootstrap_installer_tick" {
            if self.dirty
                && self
                    .last_draw
                    .is_none_or(|time| time.elapsed() >= REFRESH_INTERVAL)
            {
                self.draw(writer, surface)?;
            }
            return Ok(());
        }
        let index = usize::from(update.stream == "stderr");
        let stream = &mut self.streams[index];
        let body = stream.log.render(update.text, surface.color);
        if stream.replace && !body.is_empty() {
            stream.pending.clear();
            stream.replace = false;
        }
        if update.code == "sys_bootstrap_installer" {
            // CRLF/EOF or a control-only suffix commits the last visible update.
            stream.replace = false;
        }
        // Flush a pathological unterminated line in bounded fragments, as before.
        if self.streams[index].pending.len() + body.len() > MAX_ROW_BYTES {
            self.commit(writer, index, surface)?;
        }
        self.streams[index].pending.push_str(&body);
        if update.code == "sys_bootstrap_installer" {
            self.commit(writer, index, surface)?;
        } else if surface.dynamic {
            if self.active.is_some_and(|active| active != index) {
                self.commit(writer, self.active.unwrap(), surface)?;
            }
            if !self.streams[index].pending.is_empty() {
                self.active = Some(index);
                self.dirty = true;
                if self
                    .last_draw
                    .is_none_or(|time| time.elapsed() >= REFRESH_INTERVAL)
                {
                    self.draw(writer, surface)?;
                }
            }
        } else {
            self.commit(writer, index, surface)?;
        }
        // Invisible partial fragments (e.g. Cargo's split erase/SGR) must not
        // consume a preceding CR. Visible text or completion consumes it above.
        if update.code == "sys_bootstrap_installer_progress" {
            self.streams[index].replace = true;
        }
        Ok(())
    }

    fn row(&self, index: usize) -> String {
        format!(
            "    {} {}",
            colors::dim(&format!("[{}]", escape_controls(&self.item))),
            self.streams[index].pending
        )
    }

    fn draw(&mut self, writer: &mut impl Write, surface: &Surface) -> io::Result<()> {
        if let Some(index) = self.active {
            // Reserve the final column so the terminal never wraps a live row.
            let row = self.row(index);
            let row = console::truncate_str(&row, surface.columns.saturating_sub(1), "");
            write!(writer, "\r\x1b[2K{row}")?;
            if surface.color {
                write!(writer, "\x1b[0m")?;
            }
            writer.flush()?;
            self.last_draw = Some(Instant::now());
            self.dirty = false;
        }
        Ok(())
    }

    fn commit(
        &mut self,
        writer: &mut impl Write,
        index: usize,
        surface: &Surface,
    ) -> io::Result<()> {
        if self.streams[index].pending.is_empty() {
            return Ok(());
        }
        if self.active.is_some() {
            write!(writer, "\r\x1b[2K")?;
        }
        // Installers can disable OPOST on our shared terminal through stdin.
        // Keep append-only terminals aligned too; redirected logs retain LF.
        let ending = if surface.terminal { "\r\n" } else { "\n" };
        write!(writer, "{}{ending}", self.row(index))?;
        self.streams[index].pending.clear();
        if self.active == Some(index) {
            self.active = None;
            self.dirty = false;
            self.last_draw = None;
        } else {
            // A completed log from the other pipe must not overwrite the live progress.
            self.draw(writer, surface)?;
        }
        writer.flush()
    }

    pub(super) fn finish(&mut self, writer: &mut impl Write) -> io::Result<()> {
        self.finish_on(writer, &Surface::current())
    }

    fn finish_on(&mut self, writer: &mut impl Write, surface: &Surface) -> io::Result<()> {
        if let Some(active) = self.active {
            self.commit(writer, active, surface)?;
        }
        self.commit(writer, 0, surface)?;
        self.commit(writer, 1, surface)?;
        *self = Self::default();
        Ok(())
    }
}

#[derive(Clone, Default)]
struct Style {
    bold: bool,
    dim: bool,
    foreground: Option<String>,
    background: Option<String>,
}

impl Style {
    fn ansi(&self) -> String {
        let mut params = Vec::new();
        if self.bold {
            params.push("1");
        }
        if self.dim {
            params.push("2");
        }
        if let Some(value) = &self.foreground {
            params.push(value);
        }
        if let Some(value) = &self.background {
            params.push(value);
        }
        if params.is_empty() {
            String::new()
        } else {
            format!("\x1b[{}m", params.join(";"))
        }
    }

    fn apply(&mut self, params: &str) -> bool {
        let values = if params.is_empty() {
            vec![0]
        } else {
            let Ok(values) = params
                .split(';')
                .map(str::parse::<u16>)
                .collect::<Result<Vec<_>, _>>()
            else {
                return false;
            };
            values
        };
        let mut next = self.clone();
        let mut values = values.into_iter();
        while let Some(value) = values.next() {
            match value {
                0 => next = Self::default(),
                1 => next.bold = true,
                2 => next.dim = true,
                22 => {
                    next.bold = false;
                    next.dim = false;
                }
                30..=37 | 90..=97 => next.foreground = Some(value.to_string()),
                40..=47 | 100..=107 => next.background = Some(value.to_string()),
                39 => next.foreground = None,
                49 => next.background = None,
                38 | 48 => {
                    let mut color = vec![value];
                    let Some(mode) = values.next() else {
                        return false;
                    };
                    color.push(mode);
                    let count = match mode {
                        5 => 1,
                        2 => 3,
                        _ => return false,
                    };
                    for _ in 0..count {
                        let Some(component @ 0..=255) = values.next() else {
                            return false;
                        };
                        color.push(component);
                    }
                    let color = color
                        .iter()
                        .map(u16::to_string)
                        .collect::<Vec<_>>()
                        .join(";");
                    if value == 38 {
                        next.foreground = Some(color);
                    } else {
                        next.background = Some(color);
                    }
                }
                // No conceal, blink, inverse, links, cursor control or other terminal actions.
                _ => return false,
            }
        }
        *self = next;
        true
    }
}

#[derive(Default)]
enum Sequence {
    #[default]
    Text,
    Escape,
    Intermediate,
    Csi(String),
    DiscardCsi,
    ControlString {
        escape: bool,
    },
}

#[derive(Default)]
struct StyledLog {
    sequence: Sequence,
    style: Style,
}

impl StyledLog {
    fn render(&mut self, text: &str, color: bool) -> String {
        let mut output = String::new();
        let mut started = false;
        for ch in text.chars() {
            match &mut self.sequence {
                Sequence::Text => match ch {
                    '\x1b' => self.sequence = Sequence::Escape,
                    '\u{9b}' => self.sequence = Sequence::Csi(String::new()),
                    '\u{90}' | '\u{98}' | '\u{9d}' | '\u{9e}' | '\u{9f}' => {
                        self.sequence = Sequence::ControlString { escape: false }
                    }
                    _ => {
                        if color && !started {
                            output.push_str(&self.style.ansi());
                        }
                        started = true;
                        if ch.is_control() {
                            output.push_str(&ch.escape_debug().to_string());
                        } else {
                            output.push(ch);
                        }
                    }
                },
                Sequence::Escape => {
                    self.sequence = match ch {
                        '[' => Sequence::Csi(String::new()),
                        'P' | 'X' | ']' | '^' | '_' => Sequence::ControlString { escape: false },
                        '\x1b' => Sequence::Escape,
                        '\u{20}'..='/' => Sequence::Intermediate,
                        _ => Sequence::Text,
                    }
                }
                Sequence::Intermediate => {
                    if ('0'..='~').contains(&ch) {
                        self.sequence = Sequence::Text;
                    } else if ch == '\x1b' {
                        self.sequence = Sequence::Escape;
                    }
                }
                Sequence::Csi(params) => {
                    if ('@'..='~').contains(&ch) {
                        if ch == 'm' && self.style.apply(params) && color && started {
                            output.push_str("\x1b[0m");
                            output.push_str(&self.style.ansi());
                        }
                        self.sequence = Sequence::Text;
                    } else if ch == '\x1b' {
                        self.sequence = Sequence::Escape;
                    } else if params.len() + ch.len_utf8() <= 128 {
                        params.push(ch);
                    } else {
                        self.sequence = Sequence::DiscardCsi;
                    }
                }
                Sequence::DiscardCsi => {
                    if ('@'..='~').contains(&ch) {
                        self.sequence = Sequence::Text;
                    } else if ch == '\x1b' {
                        self.sequence = Sequence::Escape;
                    }
                }
                Sequence::ControlString { escape } => {
                    if ch == '\x07' || ch == '\u{9c}' || (*escape && ch == '\\') {
                        self.sequence = Sequence::Text;
                    } else {
                        *escape = ch == '\x1b';
                    }
                }
            }
        }
        // Styles belong to the child body only, never to the next tag or Shine result.
        if color && started {
            output.push_str("\x1b[0m");
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event<'a>(stream: &'a str, text: &'a str, code: &'a str) -> Update<'a> {
        Update {
            item: "yazi",
            stream,
            text,
            code,
        }
    }

    #[test]
    fn control_only_fragments_preserve_progress_replacement() {
        let surface = Surface {
            terminal: true,
            dynamic: true,
            color: true,
            columns: 289,
        };
        // Cargo writes a padded progress row + CR, then erase-line and a status.
        // Timer flushes may split the erase or SGR sequence into empty fragments.
        for fragments in [vec!["\x1b[K"], vec!["\x1b[", "K", "\x1b[1m", "\x1b[92m"]] {
            let mut output = InstallerOutput::default();
            let mut bytes = Vec::new();
            output
                .update(
                    &mut bytes,
                    event(
                        "stderr",
                        "Building 139/686: futures-sink    ",
                        "sys_bootstrap_installer_progress",
                    ),
                    &surface,
                )
                .unwrap();
            for fragment in fragments {
                output
                    .update(
                        &mut bytes,
                        event("stderr", fragment, "sys_bootstrap_installer_partial"),
                        &surface,
                    )
                    .unwrap();
            }
            bytes.clear();
            output
                .update(
                    &mut bytes,
                    event(
                        "stderr",
                        "Compiling powerfmt v0.2.0",
                        "sys_bootstrap_installer",
                    ),
                    &surface,
                )
                .unwrap();
            let text = String::from_utf8(bytes).unwrap();
            assert!(!text.contains("Building"), "{text:?}");
            assert!(text.contains("Compiling powerfmt v0.2.0"));
        }
    }

    #[test]
    fn terminal_lines_return_to_column_zero_without_output_processing() {
        // Cover capable, narrow and TERM=dumb-like terminal surfaces.
        for (dynamic, columns) in [(true, 80), (true, 5), (false, 80)] {
            let surface = Surface {
                terminal: true,
                dynamic,
                color: false,
                columns,
            };
            let mut output = InstallerOutput::default();
            let mut bytes = Vec::new();
            for text in ["Selecting package", "Unpacking package"] {
                output
                    .update(
                        &mut bytes,
                        event("stdout", text, "sys_bootstrap_installer"),
                        &surface,
                    )
                    .unwrap();
            }
            assert_eq!(
                bytes,
                b"    [yazi] Selecting package\r\n    [yazi] Unpacking package\r\n"
            );
            #[cfg(unix)]
            {
                use std::{
                    fs::File,
                    io::Read,
                    os::fd::{AsRawFd, FromRawFd},
                };
                let (mut master, mut slave) = (-1, -1);
                // SAFETY: valid output pointers; null arguments request default PTY settings.
                assert_eq!(
                    unsafe {
                        libc::openpty(
                            &mut master,
                            &mut slave,
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                            std::ptr::null_mut(),
                        )
                    },
                    0
                );
                // SAFETY: openpty returned distinct owned file descriptors.
                let (mut master, mut slave) =
                    unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) };
                let mut mode = std::mem::MaybeUninit::uninit();
                // SAFETY: tcgetattr initializes mode on success.
                assert_eq!(
                    unsafe { libc::tcgetattr(slave.as_raw_fd(), mode.as_mut_ptr()) },
                    0
                );
                let mut mode = unsafe { mode.assume_init() };
                mode.c_oflag &= !libc::OPOST;
                // SAFETY: mode is initialized and the slave fd remains open.
                assert_eq!(
                    unsafe { libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &mode) },
                    0
                );
                slave.write_all(&bytes).unwrap();
                let mut received = vec![0; bytes.len()];
                master.read_exact(&mut received).unwrap();
                assert_eq!(received, bytes);
            }
        }
    }

    #[test]
    fn progress_rewrites_current_row_and_commits_before_results() {
        let surface = Surface {
            terminal: true,
            dynamic: true,
            color: false,
            columns: 40,
        };
        let mut output = InstallerOutput::default();
        let mut bytes = Vec::new();
        output
            .update(
                &mut bytes,
                event("stdout", "10%", "sys_bootstrap_installer_progress"),
                &surface,
            )
            .unwrap();
        output
            .update(
                &mut bytes,
                event("stdout", "20%", "sys_bootstrap_installer_progress"),
                &surface,
            )
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&bytes), "\r\x1b[2K    [yazi] 10%");
        // The periodic tick renders the newest coalesced value, even with no more child output.
        output.last_draw = None;
        output
            .update(
                &mut bytes,
                event("stdout", "", "sys_bootstrap_installer_tick"),
                &surface,
            )
            .unwrap();
        output.finish_on(&mut bytes, &surface).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("\r\x1b[2K    [yazi] 20%"));
        assert!(text.ends_with("    [yazi] 20%\r\n"));
        assert_eq!(text.matches('\n').count(), 1);
    }

    #[test]
    fn control_only_suffix_does_not_lose_the_last_progress_or_its_newline() {
        let surface = Surface {
            terminal: true,
            dynamic: true,
            color: true,
            columns: 80,
        };
        let mut output = InstallerOutput::default();
        let mut bytes = Vec::new();
        output
            .update(
                &mut bytes,
                event("stdout", "10%", "sys_bootstrap_installer_progress"),
                &surface,
            )
            .unwrap();
        output
            .update(
                &mut bytes,
                event("stdout", "\x1b[0m", "sys_bootstrap_installer_partial"),
                &surface,
            )
            .unwrap();
        output
            .update(
                &mut bytes,
                event("stdout", "", "sys_bootstrap_installer"),
                &surface,
            )
            .unwrap();
        assert!(
            String::from_utf8(bytes)
                .unwrap()
                .ends_with("10%\x1b[0m\r\n")
        );
        assert!(output.active.is_none());
    }

    #[test]
    fn normal_stderr_logs_do_not_overwrite_stdout_progress() {
        let surface = Surface {
            terminal: true,
            dynamic: true,
            color: true,
            columns: 60,
        };
        let mut output = InstallerOutput::default();
        let mut bytes = Vec::new();
        output
            .update(
                &mut bytes,
                event(
                    "stdout",
                    "\x1b[31mDownloading",
                    "sys_bootstrap_installer_progress",
                ),
                &surface,
            )
            .unwrap();
        output
            .update(
                &mut bytes,
                event("stderr", "warning", "sys_bootstrap_installer"),
                &surface,
            )
            .unwrap();
        output
            .update(
                &mut bytes,
                event("stdout", "done\x1b[0m", "sys_bootstrap_installer"),
                &surface,
            )
            .unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(
            text.contains("    [yazi] warning\x1b[0m\r\n\r\x1b[2K    [yazi] \x1b[31mDownloading")
        );
        assert!(text.ends_with("    [yazi] \x1b[31mdone\x1b[0m\x1b[0m\r\n"));
        assert!(output.active.is_none());
    }

    #[test]
    fn redirected_output_is_plain_append_only_and_large_prompts_are_bounded() {
        let surface = Surface {
            terminal: false,
            dynamic: false,
            color: false,
            columns: 10,
        };
        let mut output = InstallerOutput::default();
        let mut bytes = Vec::new();
        for value in ["\x1b[31m10%", "20%\x1b[0m"] {
            output
                .update(
                    &mut bytes,
                    event("stdout", value, "sys_bootstrap_installer_progress"),
                    &surface,
                )
                .unwrap();
        }
        output.finish_on(&mut bytes, &surface).unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "    [yazi] 10%\n    [yazi] 20%\n"
        );
        let surface = Surface {
            terminal: true,
            dynamic: true,
            color: false,
            columns: 80,
        };
        let mut bytes = Vec::new();
        for _ in 0..20 {
            output
                .update(
                    &mut bytes,
                    event(
                        "stdout",
                        &"x".repeat(8192),
                        "sys_bootstrap_installer_partial",
                    ),
                    &surface,
                )
                .unwrap();
            assert!(output.streams[0].pending.len() <= MAX_ROW_BYTES);
        }
        output.finish_on(&mut bytes, &surface).unwrap();
    }

    #[test]
    fn live_rows_clip_wide_text_and_colors_on_resize_and_preserve_crlf_completion() {
        let mut surface = Surface {
            terminal: true,
            dynamic: true,
            color: true,
            columns: 22,
        };
        let mut output = InstallerOutput::default();
        let mut bytes = Vec::new();
        output
            .update(
                &mut bytes,
                event(
                    "stdout",
                    "\x1b[32m中文中文中文中文中文",
                    "sys_bootstrap_installer_progress",
                ),
                &surface,
            )
            .unwrap();
        let row = String::from_utf8(bytes.clone()).unwrap();
        assert!(console::measure_text_width(&row.replace("\r\x1b[2K", "")) < surface.columns);
        surface.columns = 16;
        output.last_draw = None;
        output.dirty = true;
        bytes.clear();
        output
            .update(
                &mut bytes,
                event("stdout", "", "sys_bootstrap_installer_tick"),
                &surface,
            )
            .unwrap();
        assert!(
            console::measure_text_width(
                &String::from_utf8(bytes.clone())
                    .unwrap()
                    .replace("\r\x1b[2K", "")
            ) < surface.columns
        );
        output
            .update(
                &mut bytes,
                event("stdout", "", "sys_bootstrap_installer"),
                &surface,
            )
            .unwrap();
        assert!(String::from_utf8(bytes).unwrap().ends_with('\n'));
        assert!(output.active.is_none());
    }

    #[test]
    fn styles_survive_split_sequences_and_rows_without_leaking() {
        let mut log = StyledLog::default();
        assert_eq!(log.render("\x1b[1;3", true), "");
        assert_eq!(
            log.render("2mCompiling 中文", true),
            "\x1b[1;32mCompiling 中文\x1b[0m"
        );
        assert_eq!(log.render("next", true), "\x1b[1;32mnext\x1b[0m");
        assert_eq!(log.render("\x1b[0mplain", true), "plain\x1b[0m");
        assert_eq!(
            log.render("\x1b[38;2;12;34;56mRGB", true),
            "\x1b[38;2;12;34;56mRGB\x1b[0m"
        );
        assert_eq!(
            log.render("\x1b[48;5;123mBG", true),
            "\x1b[38;2;12;34;56;48;5;123mBG\x1b[0m"
        );
    }

    #[test]
    fn plain_output_strips_styles_and_terminal_actions_even_across_fragments() {
        let mut log = StyledLog::default();
        assert_eq!(log.render("\x1b[1;32mCompiling\x1b[0m", false), "Compiling");
        assert_eq!(log.render("\x1b[2J\x1b[Hsafe\x1b]52;c;", false), "safe");
        assert_eq!(
            log.render("clipboard\x07visible\x1b]8;;https://example.com", false),
            "visible"
        );
        assert_eq!(log.render("\x1b\\link\x1b]8;;\x1b\\", false), "link");
        assert_eq!(log.render("\x1bPdevice\x1b\\done\r\n", false), "done\\r\\n");
    }

    #[test]
    fn malformed_sequences_are_bounded_and_unsupported_styles_cannot_hide_text() {
        let mut log = StyledLog::default();
        assert_eq!(
            log.render(&format!("\x1b[{}", "1;".repeat(100_000)), true),
            ""
        );
        assert!(matches!(log.sequence, Sequence::DiscardCsi));
        assert_eq!(
            log.render("m\x1b[8mvisible\x1b[38;2;999;1;1m!", true),
            "visible!\x1b[0m"
        );
    }

    #[test]
    fn stream_and_item_styles_are_independent_and_tags_stay_aligned() {
        let mut output = InstallerOutput::default();
        output.streams[0].log.render("\x1b[31m", true);
        assert!(output.streams[1].log.style.ansi().is_empty());
        output.item = "rust".into();
        let mut bytes = Vec::new();
        output
            .update(
                &mut bytes,
                Update {
                    item: "yazi",
                    stream: "stderr",
                    text: "Compiling 中文",
                    code: "sys_bootstrap_installer",
                },
                &Surface {
                    terminal: false,
                    dynamic: false,
                    color: false,
                    columns: 80,
                },
            )
            .unwrap();
        let row = String::from_utf8(bytes).unwrap();
        assert!(output.streams[0].log.style.ansi().is_empty());
        let plain = console::strip_ansi_codes(&row);
        assert_eq!(plain, "    [yazi] Compiling 中文\n");
        assert_eq!(
            console::measure_text_width(plain.split_once('[').unwrap().0),
            4
        );
    }
}
