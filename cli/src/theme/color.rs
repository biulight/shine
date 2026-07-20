//! Pure color-classification logic: OSC 11 RGB parsing, luminance-based
//! light/dark classification, and `COLORFGBG` parsing. No I/O, fully
//! cross-platform, and unit-testable without a terminal.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    pub fn as_str(self) -> &'static str {
        match self {
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }
}

/// Parses `value` (`"light"` or `"dark"`) as a [`Theme`]. Any other value —
/// including case variants — is rejected: `SHINE_TERMINAL_THEME` is a
/// display-only signal (PRD §10), not something to interpret loosely.
pub fn parse_theme_str(value: &str) -> Option<Theme> {
    match value {
        "light" => Some(Theme::Light),
        "dark" => Some(Theme::Dark),
        _ => None,
    }
}

/// Parses an OSC 11 response body of the form `rgb:RRRR/GGGG/BBBB` (each
/// component 1-4 hex digits, matching XParseColor's `rgb:` device format)
/// into 8-bit RGB, scaling each component from its source bit depth to
/// 0-255.
#[cfg(unix)]
pub fn parse_osc_rgb(body: &str) -> Option<(u8, u8, u8)> {
    let rest = body.strip_prefix("rgb:")?;
    let mut parts = rest.splitn(3, '/');
    let r = parts.next()?;
    let g = parts.next()?;
    let b = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    Some((
        scale_hex_component(r)?,
        scale_hex_component(g)?,
        scale_hex_component(b)?,
    ))
}

#[cfg(unix)]
fn scale_hex_component(hex: &str) -> Option<u8> {
    if hex.is_empty() || hex.len() > 4 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    let max = 16u32.pow(hex.len() as u32) - 1;
    Some(((value * 255) / max) as u8)
}

/// Classifies an RGB triple as light/dark using the same weighted-luma
/// threshold as the (now-superseded) shell implementation
/// (`299R + 587G + 114B >= 128000` over 0-255-scaled components), so the
/// visible light/dark boundary doesn't shift for existing users — only the
/// read-timing bug (docs/kb/lessons.md, 2026-07-14) is fixed, not the
/// classification itself.
#[cfg(unix)]
pub fn theme_from_rgb(r: u8, g: u8, b: u8) -> Theme {
    let luma = 299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b);
    if luma >= 128_000 {
        Theme::Light
    } else {
        Theme::Dark
    }
}

/// Parses `COLORFGBG` (`"fg;bg"`, or `"fg;dark_bg;bg"` on some terminals —
/// only the last field is ever the background) into a [`Theme`] using the
/// conventional xterm 16-color palette: indices 0-6 and 8 are dark, 7 and
/// 9-15 are light.
pub fn parse_colorfgbg(value: &str) -> Option<Theme> {
    let bg = value.rsplit(';').next()?.trim();
    let index: u8 = bg.parse().ok()?;
    Some(if matches!(index, 7 | 9..=15) {
        Theme::Light
    } else {
        Theme::Dark
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_theme_str_accepts_only_exact_light_or_dark() {
        assert_eq!(parse_theme_str("light"), Some(Theme::Light));
        assert_eq!(parse_theme_str("dark"), Some(Theme::Dark));
        assert_eq!(parse_theme_str("Light"), None);
        assert_eq!(parse_theme_str("DARK"), None);
        assert_eq!(parse_theme_str(""), None);
        assert_eq!(parse_theme_str("light "), None);
    }

    #[cfg(unix)]
    #[test]
    fn parse_osc_rgb_scales_16_bit_components_to_8_bit() {
        // Full-scale white: 0xffff/0xffff/0xffff -> 255/255/255.
        assert_eq!(parse_osc_rgb("rgb:ffff/ffff/ffff"), Some((255, 255, 255)));
        // Full-scale black.
        assert_eq!(parse_osc_rgb("rgb:0000/0000/0000"), Some((0, 0, 0)));
    }

    #[cfg(unix)]
    #[test]
    fn parse_osc_rgb_scales_differing_component_widths() {
        // A 2-digit component (0-255 native) must scale identically to a
        // 4-digit component representing the same fraction.
        assert_eq!(parse_osc_rgb("rgb:ff/ff/ff"), Some((255, 255, 255)));
        assert_eq!(parse_osc_rgb("rgb:00/00/00"), Some((0, 0, 0)));
    }

    #[cfg(unix)]
    #[test]
    fn parse_osc_rgb_rejects_malformed_bodies() {
        assert_eq!(parse_osc_rgb(""), None);
        assert_eq!(parse_osc_rgb("rgb:ffff/ffff"), None); // missing component
        assert_eq!(parse_osc_rgb("rgb:ffff/ffff/ffff/ffff"), None); // extra component
        assert_eq!(parse_osc_rgb("rgb:gggg/ffff/ffff"), None); // non-hex
        assert_eq!(parse_osc_rgb("rgb:fffff/ffff/ffff"), None); // too many digits
        assert_eq!(parse_osc_rgb("rgb:/ffff/ffff"), None); // empty component
        assert_eq!(parse_osc_rgb("not-rgb-at-all"), None);
    }

    #[cfg(unix)]
    #[test]
    fn theme_from_rgb_matches_shell_luma_threshold() {
        // White: clearly light.
        assert_eq!(theme_from_rgb(255, 255, 255), Theme::Light);
        // Black: clearly dark.
        assert_eq!(theme_from_rgb(0, 0, 0), Theme::Dark);
        // Right at the shell's `>= 128000` threshold in 0-255-scaled terms:
        // 299*128 + 587*128 + 114*128 = 1000*128 = 128000 -> light.
        assert_eq!(theme_from_rgb(128, 128, 128), Theme::Light);
        assert_eq!(theme_from_rgb(127, 127, 127), Theme::Dark);
    }

    #[test]
    fn parse_colorfgbg_reads_only_the_last_field_as_background() {
        assert_eq!(parse_colorfgbg("15;0"), Some(Theme::Dark));
        assert_eq!(parse_colorfgbg("0;15"), Some(Theme::Light));
        // Three-field variant some terminals emit: still only the last field.
        assert_eq!(parse_colorfgbg("15;default;0"), Some(Theme::Dark));
    }

    #[test]
    fn parse_colorfgbg_classifies_full_xterm_16_color_palette() {
        for dark_index in [0, 1, 2, 3, 4, 5, 6, 8] {
            assert_eq!(
                parse_colorfgbg(&format!("15;{dark_index}")),
                Some(Theme::Dark),
                "index {dark_index} should be dark"
            );
        }
        for light_index in [7, 9, 10, 11, 12, 13, 14, 15] {
            assert_eq!(
                parse_colorfgbg(&format!("0;{light_index}")),
                Some(Theme::Light),
                "index {light_index} should be light"
            );
        }
    }

    #[test]
    fn parse_colorfgbg_rejects_malformed_values() {
        assert_eq!(parse_colorfgbg(""), None);
        assert_eq!(parse_colorfgbg("not-a-number"), None);
        assert_eq!(parse_colorfgbg(";"), None);
    }
}
