/// A command failed after its complete diagnostic and result were rendered locally.
/// The binary uses this typed marker to retain a nonzero exit without printing another error.
#[derive(Debug)]
pub struct ReportedCommandFailure;

impl std::fmt::Display for ReportedCommandFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("command incomplete; see reported results")
    }
}
impl std::error::Error for ReportedCommandFailure {}

use crate::colors;
use console::measure_text_width;
use std::io::IsTerminal;

const SUMMARY_LABEL_WIDTH: usize = 15;
const DETAIL_LABEL_WIDTH: usize = 15;
const DETAIL_STATUS_WIDTH: usize = 13;
const COLUMN_GAP: usize = 2;

/// Keep untrusted labels on one terminal row without changing their stored values.
pub(crate) fn escape_controls(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_control() {
                ch.escape_debug().to_string()
            } else {
                ch.to_string()
            }
        })
        .collect()
}

/// Build a complete plain diff, splitting only on LF so bare CR remains content.
/// Callers choose comparison/display bytes and render the body with their stdout or TTY encoder.
pub(crate) fn unified_text_diff(old: &str, new: &str, labels: Option<(&str, &str)>) -> String {
    use std::fmt::Write as _;

    let old_lines = old.split_inclusive('\n').collect::<Vec<_>>();
    let new_lines = new.split_inclusive('\n').collect::<Vec<_>>();
    let diff = similar::TextDiff::from_slices(&old_lines, &new_lines);
    let mut unified = String::new();
    for hunk in diff.unified_diff().context_radius(3).iter_hunks() {
        if unified.is_empty()
            && let Some((old, new)) = labels
        {
            let _ = writeln!(
                unified,
                "--- {}\n+++ {}",
                escape_controls(old),
                escape_controls(new)
            );
        }
        let _ = writeln!(unified, "{}", hunk.header());
        for change in hunk.iter_changes() {
            let line = change.value();
            let _ = writeln!(
                unified,
                "{}{}",
                change.tag(),
                line.strip_suffix('\n').unwrap_or(line)
            );
            // similar also counts a final bare CR as a newline; only LF terminates our lines.
            if !line.ends_with('\n') {
                let _ = writeln!(unified, "\\ No newline at end of file");
            }
        }
    }
    unified
}

/// Render a single-file unified diff for stdout, escaping source controls before styling.
pub(crate) fn colorize_unified_diff(diff: &str) -> String {
    let mut out = String::with_capacity(diff.len());
    let mut in_hunk = false;
    for line in diff.split_inclusive('\n') {
        let escaped = escape_controls(line.trim_end_matches('\n'));
        let colored = if line.starts_with("@@ ") {
            in_hunk = true;
            colors::cyan(&escaped)
        } else if !in_hunk && (line.starts_with("--- ") || line.starts_with("+++ ")) {
            colors::cyan(&escaped)
        } else if line.starts_with('+') {
            colors::green(&escaped)
        } else if line.starts_with('-') {
            colors::red(&escaped)
        } else {
            escaped
        };
        out.push_str(&colored);
        if line.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// Tracks whether any upgrade section has printed content yet, so a run
/// covering several optional sections (shell presets, app configs, managed
/// system configs) can emit exactly one blank-line separator between two
/// sections that both printed something — never a leading separator before
/// the first section, and never a stray blank line when a section had
/// nothing to report.
#[derive(Default)]
pub struct SectionSeparator {
    printed: bool,
    preamble: Option<String>,
}

pub(crate) enum SectionPrefix {
    None,
    BlankLine,
    Preamble(String),
}

impl SectionSeparator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a separator that prints `preamble` immediately before its first
    /// visible section. If no section begins, the preamble stays hidden.
    pub fn with_preamble(preamble: impl Into<String>) -> Self {
        Self {
            printed: false,
            preamble: Some(preamble.into()),
        }
    }

    /// Call immediately before printing a section's header, only on the path
    /// where the section is actually about to print something.
    pub fn begin(&mut self) {
        match self.next_prefix() {
            SectionPrefix::None => {}
            SectionPrefix::BlankLine => println!(),
            SectionPrefix::Preamble(preamble) => println!("{preamble}"),
        }
    }

    pub(crate) fn next_prefix(&mut self) -> SectionPrefix {
        let prefix = if self.printed {
            SectionPrefix::BlankLine
        } else if let Some(preamble) = self.preamble.take() {
            SectionPrefix::Preamble(preamble)
        } else {
            SectionPrefix::None
        };
        self.printed = true;
        prefix
    }

    pub fn has_printed(&self) -> bool {
        self.printed
    }
}

fn join_summary(parts: &[String]) -> String {
    if parts.is_empty() {
        colors::dim("nothing changed")
    } else {
        parts.join(&colors::dim(" · "))
    }
}

pub(crate) fn summary_line_text(label: &str, parts: &[String]) -> String {
    format!(
        "{}{}{}",
        colors::bold(label),
        pad(label, SUMMARY_LABEL_WIDTH),
        join_summary(parts)
    )
}

/// Appends `"{count} {label}"` (styled with `color`) to `parts` when `count`
/// is nonzero. Collapses the repeated `if count > 0 { parts.push(color(...))
/// }` idiom used when building summary footers from several counters.
pub fn push_count(parts: &mut Vec<String>, count: usize, color: fn(&str) -> String, label: &str) {
    if count > 0 {
        parts.push(color(&format!("{count} {label}")));
    }
}

pub fn summary_line(label: &str, parts: &[String]) {
    println!("{}", summary_line_text(label, parts));
}

pub fn footer(label: &str, parts: &[String]) {
    println!();
    summary_line(label, parts);
}

/// Prints names in Homebrew-style columns when stdout is an interactive
/// terminal, falling back to one item per line for redirected output.
pub fn print_columns(items: &[String]) {
    let terminal_width = std::io::stdout().is_terminal().then(|| {
        let width = usize::from(console::Term::stdout().size().1);
        width.max(1)
    });
    print!("{}", format_columns(items, terminal_width));
}

/// Formats names in top-to-bottom, then left-to-right columns.
///
/// `terminal_width = None` represents non-interactive output and intentionally
/// produces one item per line, matching Homebrew's pipe-friendly fallback.
fn format_columns(items: &[String], terminal_width: Option<usize>) -> String {
    if items.is_empty() {
        return String::new();
    }

    let Some(terminal_width) = terminal_width else {
        return items.join("\n") + "\n";
    };
    let max_width = items
        .iter()
        .map(|item| measure_text_width(item))
        .max()
        .unwrap_or(0);
    let mut columns = (terminal_width + COLUMN_GAP) / (max_width + COLUMN_GAP);
    if columns < 2 {
        return items.join("\n") + "\n";
    }

    columns = columns.min(items.len());
    let rows = items.len().div_ceil(columns);
    columns = items.len().div_ceil(rows);
    let column_width = ((terminal_width + COLUMN_GAP) / columns) - COLUMN_GAP;
    let mut output = String::new();

    for row in 0..rows {
        let indices: Vec<usize> = (row..items.len()).step_by(rows).collect();
        for (position, index) in indices.iter().enumerate() {
            let item = &items[*index];
            output.push_str(item);
            if position + 1 < indices.len() {
                let padding = column_width.saturating_sub(measure_text_width(item)) + COLUMN_GAP;
                output.push_str(&" ".repeat(padding));
            }
        }
        output.push('\n');
    }

    output
}

pub fn detail_line(label: &str, status: &str, detail: Option<String>) {
    println!("{}", detail_line_text(label, status, detail));
}

pub(crate) fn detail_line_text(label: &str, status: &str, detail: Option<String>) -> String {
    // Details are plain data; callers may style the status separately.
    let detail = detail
        .filter(|value| !value.is_empty())
        .map(|value| format!("  {}", colors::dim(&escape_controls(&value))))
        .unwrap_or_default();

    format!(
        "{}{}{}{}{}",
        colors::bold(label),
        pad(label, DETAIL_LABEL_WIDTH),
        status,
        pad_plain(measure_text_width(status), DETAIL_STATUS_WIDTH),
        detail
    )
}

pub fn hint_line(label: &str, detail: &str) {
    println!("{}", hint_line_text(label, detail));
}

pub(crate) fn hint_line_text(label: &str, detail: &str) -> String {
    format!(
        "{}{}{}",
        colors::bold(label),
        pad(label, DETAIL_LABEL_WIDTH),
        colors::dim(detail)
    )
}

fn pad(label: &str, width: usize) -> String {
    pad_plain(measure_text_width(label), width)
}

fn pad_plain(visible_len: usize, width: usize) -> String {
    " ".repeat(width.saturating_sub(visible_len) + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_diff_uses_lf_lines_and_escapes_headers_before_construction() {
        let diff = unified_text_diff(
            "head\nold\rtail\nend\n",
            "head\nnew\rtail\nend\n",
            Some(("current\n--- forged\t", "expected\u{1b}")),
        );
        assert_eq!(
            diff,
            "--- current\\n--- forged\\t\n+++ expected\\u{1b}\n@@ -1,3 +1,3 @@\n head\n-old\rtail\n+new\rtail\n end\n"
        );
        let rendered = colorize_unified_diff(&diff);
        let rendered = console::strip_ansi_codes(&rendered);
        assert!(rendered.contains("-old\\rtail\n+new\\rtail\n"));
        assert!(rendered.chars().all(|ch| !ch.is_control() || ch == '\n'));
    }

    #[test]
    fn text_diff_preserves_final_newline_changes_and_bare_cr_at_eof() {
        for (old, new, expected) in [
            ("", "x", "@@ -0,0 +1 @@\n+x\n\\ No newline at end of file\n"),
            ("x\n", "", "@@ -1 +0,0 @@\n-x\n"),
            (
                "x",
                "x\n",
                "@@ -1 +1 @@\n-x\n\\ No newline at end of file\n+x\n",
            ),
            (
                "old\r",
                "new\r",
                "@@ -1 +1 @@\n-old\r\n\\ No newline at end of file\n+new\r\n\\ No newline at end of file\n",
            ),
        ] {
            assert_eq!(unified_text_diff(old, new, None), expected);
        }
        assert!(unified_text_diff("same\r\n", "same\r\n", Some(("a", "b"))).is_empty());
    }

    #[test]
    fn text_diff_keeps_all_hunks_and_long_changed_suffixes() {
        let middle = (0..20)
            .map(|i| format!("context {i}\n"))
            .collect::<String>();
        let prefix = "x".repeat(8192);
        let old = format!("old\n{middle}{prefix}OLD");
        let new = format!("new\n{middle}{prefix}NEW");
        let diff = unified_text_diff(&old, &new, Some(("a", "b")));
        assert_eq!(diff.matches("--- a\n+++ b\n").count(), 1);
        assert_eq!(diff.matches("@@ -").count(), 2);
        for (sign, suffix) in [('-', "OLD"), ('+', "NEW")] {
            assert!(diff.contains(&format!(
                "{sign}{prefix}{suffix}\n\\ No newline at end of file\n"
            )));
        }
    }

    #[test]
    fn unified_diff_styles_headers_and_hunk_content_without_trusting_source_controls() {
        let diff =
            "--- a/file\n+++ b/file\n@@ -1,2 +1,2 @@\n--- old\t\u{1b}[2J\n+++ new\r\n context";
        let expected = [
            colors::cyan("--- a/file"),
            colors::cyan("+++ b/file"),
            colors::cyan("@@ -1,2 +1,2 @@"),
            colors::red("--- old\\t\\u{1b}[2J"),
            colors::green("+++ new\\r"),
            " context".into(),
        ]
        .join("\n");
        assert_eq!(colorize_unified_diff(diff), expected);
    }

    #[test]
    fn join_summary_uses_default_for_empty_parts() {
        assert_eq!(join_summary(&[]), "nothing changed");
    }

    #[test]
    fn push_count_appends_only_for_nonzero_counts() {
        let mut parts = Vec::new();
        push_count(&mut parts, 0, colors::green, "up-to-date");
        push_count(&mut parts, 3, colors::green, "up-to-date");
        assert_eq!(parts, vec!["3 up-to-date".to_string()]);
    }

    #[test]
    fn detail_columns_use_terminal_width_for_unicode_and_styles() {
        let rows = ["cat", "猫", "e\u{301}", "\x1b[32m猫\x1b[0m"]
            .map(|status| detail_line_text("Link Conflict", status, Some("detail".into())));
        let widths = rows.map(|row| measure_text_width(row.split_once("detail").unwrap().0));
        assert_eq!(widths, [widths[0]; 4]);
    }

    #[test]
    fn detail_data_escapes_controls_preserving_status_styles_and_literal_paths() {
        let status = "\u{1b}[32mupdated\u{1b}[0m";
        let row = detail_line_text(
            "Shell Profile",
            status,
            Some("/tmp/a\nFORGED\r\t\u{1b}\\literal中文".into()),
        );
        assert!(row.contains(status), "{row:?}");
        assert!(
            console::strip_ansi_codes(&row).ends_with("/tmp/a\\nFORGED\\r\\t\\u{1b}\\literal中文"),
            "{row:?}"
        );
        assert_eq!(row.lines().count(), 1);
        assert!(!row.contains(['\r', '\t']));
    }

    #[test]
    fn columns_fall_back_to_one_item_per_line_without_a_terminal() {
        let items = vec!["alpha".to_string(), "beta".to_string()];

        assert_eq!(format_columns(&items, None), "alpha\nbeta\n");
    }

    #[test]
    fn columns_fill_top_to_bottom_then_left_to_right() {
        let items = ["alpha", "beta", "gamma", "delta"]
            .map(str::to_string)
            .to_vec();

        assert_eq!(
            format_columns(&items, Some(20)),
            "alpha      gamma\nbeta       delta\n"
        );
    }

    #[test]
    fn columns_fall_back_when_the_terminal_is_too_narrow() {
        let items = vec!["alpha".to_string(), "beta".to_string()];

        assert_eq!(format_columns(&items, Some(7)), "alpha\nbeta\n");
    }

    #[test]
    fn columns_measure_unicode_display_width_and_omit_trailing_spaces() {
        let items = ["猫", "dog", "鸟", "fox"].map(str::to_string).to_vec();
        let rendered = format_columns(&items, Some(12));

        assert_eq!(rendered, "猫     鸟\ndog    fox\n");
        assert!(rendered.lines().all(|line| !line.ends_with(' ')));
    }

    #[test]
    fn preamble_stays_pending_until_the_first_section() {
        let mut separator = SectionSeparator::with_preamble("Upgrade");
        assert!(!separator.has_printed());
        separator.begin();
        assert!(separator.has_printed());
    }
}
