//! Line-ending-agnostic comparison helpers.
//!
//! Preset templates are embedded and written LF (see the repo `.gitattributes`),
//! but a user's on-disk copy of an installed file may be CRLF — e.g. a Windows
//! editor re-saving a PowerShell profile. Comparing those byte-exact would treat
//! a pure CRLF↔LF difference as a real change, producing spurious "update"
//! reports (and silent whole-file rewrites) on re-install. These helpers let the
//! reconciliation logic compare content while ignoring line-ending style.

/// Returns a copy of `bytes` with every `\r\n` and lone `\r` reduced to `\n`.
///
/// For input that is already LF-only this returns an identical byte sequence, so
/// callers that compare via [`eol_eq`] see no behavior change for LF content.
pub fn normalize_eol(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' {
            out.push(b'\n');
            // Collapse a `\r\n` pair into the single `\n` just pushed.
            if bytes.get(i + 1) == Some(&b'\n') {
                i += 1;
            }
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    out
}

/// Compares two byte slices for equality, ignoring line-ending style
/// (`\r\n`, lone `\r`, and `\n` are all treated as equivalent line breaks).
pub fn eol_eq(a: &[u8], b: &[u8]) -> bool {
    normalize_eol(a) == normalize_eol(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_eol_converts_crlf_to_lf() {
        assert_eq!(normalize_eol(b"a\r\nb\r\n"), b"a\nb\n");
    }

    #[test]
    fn normalize_eol_converts_lone_cr_to_lf() {
        assert_eq!(normalize_eol(b"a\rb\r"), b"a\nb\n");
    }

    #[test]
    fn normalize_eol_handles_mixed_endings() {
        assert_eq!(normalize_eol(b"a\r\nb\rc\nd"), b"a\nb\nc\nd");
    }

    #[test]
    fn normalize_eol_is_noop_for_lf_only() {
        let input = b"a\nb\nc";
        assert_eq!(normalize_eol(input), input);
    }

    #[test]
    fn eol_eq_ignores_line_ending_style() {
        assert!(eol_eq(b"a\r\nb\r\n", b"a\nb\n"));
        assert!(eol_eq(b"a\rb", b"a\nb"));
        assert!(!eol_eq(b"a\r\nb", b"a\r\nc"));
    }
}
