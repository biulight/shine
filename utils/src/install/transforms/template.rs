use std::collections::BTreeMap;

/// Apply `@@VAR_NAME@@` substitution using `vars`.
///
/// Placeholders must match `@@[A-Za-z_][A-Za-z0-9_]*@@`. Any placeholder
/// whose name is not in `vars` is collected and returned as an error after
/// scanning the entire input, so all missing variables are reported at once.
///
/// Non-matching `@@` sequences (e.g. the closing `@@` is absent, or the name
/// contains invalid characters) are passed through unchanged.
pub(super) fn apply(input: &[u8], vars: &BTreeMap<String, String>) -> anyhow::Result<Vec<u8>> {
    let text = std::str::from_utf8(input)
        .map_err(|_| anyhow::anyhow!("template: input is not valid UTF-8"))?;

    let mut result = String::with_capacity(text.len());
    let mut missing: Vec<String> = Vec::new();
    let mut remaining = text;

    while let Some(open) = remaining.find("@@") {
        result.push_str(&remaining[..open]);
        let after_open = &remaining[open + 2..];

        if let Some(close) = after_open.find("@@") {
            let var_name = &after_open[..close];
            if is_valid_var_name(var_name) {
                match vars.get(var_name) {
                    Some(value) => result.push_str(value),
                    None => {
                        missing.push(var_name.to_string());
                        result.push_str("@@");
                        result.push_str(var_name);
                        result.push_str("@@");
                    }
                }
                remaining = &after_open[close + 2..];
                continue;
            }
        }
        // Not a valid placeholder — emit the opening @@ literally.
        result.push_str("@@");
        remaining = after_open;
    }

    result.push_str(remaining);

    if !missing.is_empty() {
        anyhow::bail!("undefined template variable(s): {}", missing.join(", "));
    }

    Ok(result.into_bytes())
}

fn is_valid_var_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn substitutes_known_variable() {
        let v = vars(&[("PORT", "6152")]);
        let out = apply(b"http://127.0.0.1:@@PORT@@", &v).unwrap();
        assert_eq!(out, b"http://127.0.0.1:6152");
    }

    #[test]
    fn substitutes_multiple_occurrences() {
        let v = vars(&[("PORT", "7890")]);
        let out = apply(b"@@PORT@@ and @@PORT@@", &v).unwrap();
        assert_eq!(out, b"7890 and 7890");
    }

    #[test]
    fn substitutes_multiple_distinct_variables() {
        let v = vars(&[("HTTP", "6152"), ("SOCKS5", "6153")]);
        let out = apply(b"@@HTTP@@ @@SOCKS5@@", &v).unwrap();
        assert_eq!(out, b"6152 6153");
    }

    #[test]
    fn errors_on_undefined_variable() {
        let v = vars(&[]);
        let err = apply(b"@@MISSING@@", &v).unwrap_err();
        assert!(err.to_string().contains("MISSING"), "{err}");
    }

    #[test]
    fn reports_all_missing_variables() {
        let v = vars(&[]);
        let err = apply(b"@@A@@ @@B@@", &v).unwrap_err();
        assert!(err.to_string().contains("A"), "{err}");
        assert!(err.to_string().contains("B"), "{err}");
    }

    #[test]
    fn passthrough_when_no_closing_at_at() {
        let v = vars(&[]);
        let out = apply(b"hello @@ world", &v).unwrap();
        assert_eq!(out, b"hello @@ world");
    }

    #[test]
    fn passthrough_for_invalid_var_name_with_space() {
        let v = vars(&[]);
        let out = apply(b"@@has space@@", &v).unwrap();
        assert_eq!(out, b"@@has space@@");
    }

    #[test]
    fn passthrough_for_empty_placeholder() {
        let v = vars(&[]);
        let out = apply(b"@@@@", &v).unwrap();
        assert_eq!(out, b"@@@@");
    }

    #[test]
    fn no_placeholders_passthrough() {
        let v = vars(&[("X", "1")]);
        let out = apply(b"plain text", &v).unwrap();
        assert_eq!(out, b"plain text");
    }

    #[test]
    fn empty_input_returns_empty() {
        let v = vars(&[]);
        let out = apply(b"", &v).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn errors_on_non_utf8_input() {
        let v = vars(&[]);
        assert!(apply(&[0xFF, 0xFE], &v).is_err());
    }
}
