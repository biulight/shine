use std::collections::BTreeMap;

/// Typed failure so planners can report missing inputs without exposing their names.
#[derive(Debug)]
pub(crate) struct MissingTemplateVariables(Vec<String>);

impl std::fmt::Display for MissingTemplateVariables {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "undefined template variable(s): {}", self.0.join(", "))
    }
}

impl std::error::Error for MissingTemplateVariables {}

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
        return Err(MissingTemplateVariables(missing).into());
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

    #[test]
    fn substitutes_known_variables_and_preserves_plain_text() {
        let render = |input: &[u8], bindings: &[(&str, &str)]| {
            let vars = bindings
                .iter()
                .map(|&(k, v)| (k.into(), v.into()))
                .collect();
            apply(input, &vars).unwrap()
        };
        assert_eq!(
            render(b"http://127.0.0.1:@@PORT@@", &[("PORT", "6152")]),
            b"http://127.0.0.1:6152"
        );
        assert_eq!(
            render(b"@@PORT@@ and @@PORT@@", &[("PORT", "7890")]),
            b"7890 and 7890"
        );
        assert_eq!(
            render(
                b"@@HTTP@@ @@SOCKS5@@",
                &[("HTTP", "6152"), ("SOCKS5", "6153")]
            ),
            b"6152 6153"
        );
        assert_eq!(render(b"plain text", &[("X", "1")]), b"plain text");
    }

    #[test]
    fn reports_all_missing_variables() {
        for (input, missing) in [
            ("@@MISSING@@", &["MISSING"][..]),
            ("@@A@@ @@B@@", &["A", "B"]),
        ] {
            let err = apply(input.as_bytes(), &BTreeMap::new()).unwrap_err();
            assert!(err.is::<MissingTemplateVariables>(), "{err}");
            for name in missing {
                assert!(err.to_string().contains(name), "{err}");
            }
        }
    }

    #[test]
    fn preserves_empty_input_and_invalid_placeholders() {
        for input in [b"".as_slice(), b"hello @@ world", b"@@has space@@", b"@@@@"] {
            assert_eq!(apply(input, &BTreeMap::new()).unwrap(), input, "{input:?}");
        }
    }

    #[test]
    fn errors_on_non_utf8_input() {
        assert!(apply(&[0xFF, 0xFE], &BTreeMap::new()).is_err());
    }
}
