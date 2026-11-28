use anyhow::{Context, Result};

/// Convert supported JSONC/JSON5 syntax to standard JSON.
///
/// Output always ends with a newline so the hash is stable across installs.
pub(super) fn apply(input: &[u8]) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(input).context("jsonc-to-json: input must be valid UTF-8")?;
    let value = jsonc_parser::parse_to_serde_value::<serde_json::Value>(
        text,
        // Keep the accepted syntax independent of upstream JSON5 defaults.
        &jsonc_parser::ParseOptions {
            allow_comments: true,
            allow_loose_object_property_names: true,
            allow_trailing_commas: true,
            allow_missing_commas: true,
            allow_single_quoted_strings: true,
            allow_hexadecimal_numbers: true,
            allow_unary_plus_numbers: true,
            allow_bare_decimal_point_numbers: true,
            allow_non_finite_numbers: false,
            allow_extended_string_escapes: true,
        },
    )
    .map_err(|e| anyhow::anyhow!("jsonc-to-json: {e}"))?;
    let mut out =
        serde_json::to_vec_pretty(&value).context("jsonc-to-json: serialization failed")?;
    if out.last() != Some(&b'\n') {
        out.push(b'\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(input: &[u8]) -> serde_json::Value {
        let out = apply(input).unwrap();
        serde_json::from_slice(&out).unwrap()
    }

    #[test]
    fn strips_line_and_block_comments() {
        for input in [
            "{\n  \"key\": \"value\" // comment\n}",
            "{ /* block */ \"key\": \"value\" }",
        ] {
            assert_eq!(
                round_trip(input.as_bytes()),
                serde_json::json!({"key": "value"}),
                "{input}"
            );
        }
    }

    #[test]
    fn preserves_nested_mirror_urls_with_and_without_comments() {
        for (input, key, url) in [
            (
                r#"{ "mirrors": ["https://example.com"] }"#,
                "mirrors",
                "https://example.com",
            ),
            (
                r#"{ "registry-mirrors": [
                // tencent mirror
                "https://mirror.ccs.tencentyun.com"
            ] }"#,
                "registry-mirrors",
                "https://mirror.ccs.tencentyun.com",
            ),
        ] {
            assert_eq!(round_trip(input.as_bytes())[key][0], url);
        }
    }

    #[test]
    fn output_bytes_are_deterministic_and_end_with_a_newline() {
        for input in [r#"{"x": 1}"#, r#"{ "b": 2, "a": 1 }"#] {
            let bytes = apply(input.as_bytes()).unwrap();
            assert_eq!(bytes.last(), Some(&b'\n'));
            assert_eq!(apply(input.as_bytes()).unwrap(), bytes);
        }
    }

    #[test]
    fn retains_existing_permissive_syntax() {
        assert_eq!(round_trip(br#"{ "a": 1, }"#), serde_json::json!({"a": 1}));
        assert_eq!(
            round_trip(b"{unquoted: 'value', hex: 0x10, plus: +2, items: [1 2],}"),
            serde_json::json!({"unquoted": "value", "hex": 16, "plus": 2, "items": [1, 2]})
        );
    }

    #[test]
    fn normalizes_json5_decimal_numbers() {
        assert_eq!(
            round_trip(b"{values: [.5, -.5, +.5, 5., -5., +5., 5.e3, .5e-2]}"),
            serde_json::json!({"values": [0.5, -0.5, 0.5, 5.0, -5.0, 5.0, 5000.0, 0.005]})
        );
    }

    #[test]
    fn converts_json5_escapes_to_json_without_losing_characters() {
        let input = br#"{'\x6bey': '\x41\xE9\v\0\a', quoted: "it\'s", slash: '\\'}"#;
        let out = apply(input).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&out).unwrap(),
            serde_json::json!({"key": "Aé\u{000b}\0a", "quoted": "it's", "slash": "\\"})
        );
        assert!(!out.contains(&0));
        assert!(!out.contains(&0x0b));
        assert_eq!(out.last(), Some(&b'\n'));
        assert_eq!(apply(&out).unwrap(), out);
    }

    #[test]
    fn removes_json5_line_continuations_without_adding_newlines() {
        for ending in ["\n", "\r", "\r\n", "\u{2028}", "\u{2029}"] {
            for quote in ['\'', '"'] {
                let input = format!("{{value: {quote}first\\{ending}  second{quote}}}");
                assert_eq!(
                    round_trip(input.as_bytes()),
                    serde_json::json!({"value": "first  second"})
                );
            }
        }
    }

    #[test]
    fn rejects_non_finite_json5_numbers() {
        for number in ["Infinity", "-Infinity", "+Infinity", "NaN", "+NaN", "-NaN"] {
            let input = format!("{{values: [.5, {{value: {number}}}]}}");
            assert!(apply(input.as_bytes()).is_err(), "accepted {number}");
        }
        assert_eq!(round_trip(br#""NaN""#), serde_json::json!("NaN"));
    }

    #[test]
    fn rejects_malformed_objects_escapes_and_numbers() {
        for input in [
            "{ invalid }",
            r#""\x4""#,
            r#""\xGG""#,
            r#""\01""#,
            r#""\1""#,
            ".",
            "+.",
            "-.e2",
            "1.e",
            "0x",
            "1..2",
        ] {
            assert!(apply(input.as_bytes()).is_err(), "accepted {input}");
        }
    }

    #[test]
    fn rejects_unrepresentable_numbers_instead_of_writing_null() {
        for input in [r#"{"value": 1e999}"#, r#"[1, {"value": -1e999}]"#] {
            assert!(apply(input.as_bytes()).is_err(), "accepted {input}");
        }
    }
}
