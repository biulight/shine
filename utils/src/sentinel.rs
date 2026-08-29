//! Shared sentinel-block primitives used by `shells::profile` and
//! `sys::profile` to find, extract, remove, and insert the
//! `# >>> ... >>>` / `# <<< ... <<<` blocks shine writes into user-owned
//! shell profile files.
//!
//! **Two removal styles are kept deliberately separate** ([`remove_block_bytewise`]
//! vs [`remove_block_linewise`]): they differ in preceding-blank-line
//! handling and CRLF behavior (see each function's docs), and canonicalizing
//! them without golden-output proof that neither caller depends on the
//! difference would risk a silent formatting regression in a file shine does
//! not own. Do not merge them without characterization tests confirming both
//! callers' current byte-for-byte output is preserved.

/// A sentinel marker pair, e.g. `("# >>> shine >>>", "# <<< shine <<<")`.
#[derive(Clone, Copy)]
pub struct Sentinel<'a> {
    pub start: &'a str,
    pub end: &'a str,
}

/// Returns the substring from the start marker through the end marker
/// (inclusive), or `None` if either marker is missing. Does not include any
/// trailing newline after the end marker.
pub fn find_block<'a>(content: &'a str, sentinel: &Sentinel) -> Option<&'a str> {
    let start = content.find(sentinel.start)?;
    let end = content[start..].find(sentinel.end)? + start + sentinel.end.len();
    Some(&content[start..end])
}

/// Like [`find_block`], but also includes one trailing `'\n'` immediately
/// after the end marker if present. The end marker is searched for only
/// after the start marker, so an end marker that appears *before* the start
/// marker is not matched.
pub fn extract_block_with_newline<'a>(content: &'a str, sentinel: &Sentinel) -> Option<&'a str> {
    let start = content.find(sentinel.start)?;
    let after_start = &content[start..];
    let end = after_start.find(sentinel.end)? + sentinel.end.len();
    let end = if after_start[end..].starts_with('\n') {
        end + 1
    } else {
        end
    };
    Some(&after_start[..end])
}

/// Byte-offset block removal (`shells::profile`'s semantics).
///
/// No-op if either marker is missing. When present, consumes one preceding
/// blank line (a literal `"\n\n"` tail immediately before the start marker)
/// and the trailing LF or CRLF immediately after the end marker, if present.
///
/// On CRLF input the preceding blank line is not consumed because the check
/// deliberately looks for a literal `"\n\n"` tail. CRLF bytes elsewhere in
/// `content` are left untouched (this function never rewrites line endings).
pub fn remove_block_bytewise(content: &str, sentinel: &Sentinel) -> String {
    let start = match content.find(sentinel.start) {
        Some(i) => i,
        None => return content.to_string(),
    };
    let end_marker = match content.find(sentinel.end) {
        Some(i) => i + sentinel.end.len(),
        None => return content.to_string(),
    };
    let end = if content[end_marker..].starts_with("\r\n") {
        end_marker + 2
    } else if content[end_marker..].starts_with('\n') {
        end_marker + 1
    } else {
        end_marker
    };
    let block_start = if start > 0 && content[..start].ends_with("\n\n") {
        start - 1
    } else {
        start
    };
    format!("{}{}", &content[..block_start], &content[end..])
}

/// Line-based block removal (`sys::profile`'s semantics).
///
/// Drops only the lines from the start marker through the end marker
/// (inclusive); never consumes a preceding blank line separator, unlike
/// [`remove_block_bytewise`].
///
/// Because this iterates via [`str::lines`], CRLF input is normalized to LF
/// **unconditionally** — even when the sentinel isn't present at all, since
/// `lines()` always strips a trailing `'\r'` from each line. The presence or
/// absence of a trailing newline on the original `content` is preserved on
/// the result.
pub fn remove_block_linewise(content: &str, sentinel: &Sentinel) -> String {
    let mut output = Vec::new();
    let mut skip = false;
    for line in content.lines() {
        if line == sentinel.start {
            skip = true;
            continue;
        }
        if line == sentinel.end {
            skip = false;
            continue;
        }
        if !skip {
            output.push(line);
        }
    }
    let mut result = output.join("\n");
    if content.ends_with('\n') && !result.is_empty() {
        result.push('\n');
    }
    result
}

/// Where to insert a block relative to existing content, for [`insert_block`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InsertAt {
    Start,
    End,
}

/// Inserts `block` into `content`, separated from any existing content by
/// exactly one blank line (regardless of whether `block` or `content`
/// already end/start with a newline). When `content` is empty, no
/// leading/trailing blank line is added — the result is just `block`.
pub fn insert_block(content: &str, block: &str, at: InsertAt) -> String {
    match at {
        InsertAt::Start => {
            let mut updated = String::new();
            updated.push_str(block);
            if !content.is_empty() {
                if !block.ends_with('\n') {
                    updated.push('\n');
                }
                updated.push('\n');
                updated.push_str(content);
            }
            updated
        }
        InsertAt::End => {
            let mut updated = content.to_string();
            if !updated.ends_with('\n') && !updated.is_empty() {
                updated.push('\n');
            }
            if !updated.is_empty() {
                updated.push('\n');
            }
            updated.push_str(block);
            updated
        }
    }
}

/// Trims all leading and trailing `'\n'` characters (not just one).
pub fn trim_outer_blank_lines(content: &str) -> String {
    content.trim_matches('\n').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SENTINEL: Sentinel<'static> = Sentinel {
        start: "START",
        end: "END",
    };

    #[test]
    fn find_block_returns_start_through_end_inclusive() {
        let content = "pre\nSTART\nbody\nEND\npost";
        assert_eq!(find_block(content, &SENTINEL), Some("START\nbody\nEND"));
    }

    #[test]
    fn find_block_none_when_either_marker_missing() {
        assert_eq!(find_block("no markers", &SENTINEL), None);
        assert_eq!(find_block("STARTonly", &SENTINEL), None);
    }

    #[test]
    fn remove_block_bytewise_and_linewise_agree_on_the_simple_case() {
        // Both styles remove an isolated block identically when there's no
        // preceding blank line or CRLF involved — they only diverge on
        // those specific edge cases (see each function's own module docs
        // and the dedicated tests in shells::profile / sys::profile).
        let content = "before\nSTART\nbody\nEND\nafter\n";
        assert_eq!(
            remove_block_bytewise(content, &SENTINEL),
            remove_block_linewise(content, &SENTINEL)
        );
    }

    #[test]
    fn insert_block_start_and_end_are_symmetric_on_empty_content() {
        assert_eq!(
            insert_block("", "BLOCK\n", InsertAt::Start),
            insert_block("", "BLOCK\n", InsertAt::End)
        );
    }

    #[test]
    fn trim_outer_blank_lines_is_idempotent() {
        let once = trim_outer_blank_lines("\n\nfoo\n\n");
        let twice = trim_outer_blank_lines(&once);
        assert_eq!(once, twice);
    }
}
