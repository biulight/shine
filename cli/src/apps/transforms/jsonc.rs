use anyhow::{Context, Result};

/// Strip JSONC comments and trailing commas, then re-serialize as canonical JSON.
///
/// Output always ends with a newline so the hash is stable across installs.
pub(super) fn apply(input: &[u8]) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(input).context("jsonc-to-json: input must be valid UTF-8")?;
    let value = jsonc_parser::parse_to_serde_value(text, &jsonc_parser::ParseOptions::default())
        .map_err(|e| anyhow::anyhow!("jsonc-to-json: {e}"))?
        .unwrap_or(serde_json::Value::Null);
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
    fn strips_line_comments() {
        let input = b"{\n  \"key\": \"value\" // comment\n}";
        let out = apply(input).unwrap();
        assert!(!std::str::from_utf8(&out).unwrap().contains("//"));
        let _: serde_json::Value = serde_json::from_slice(&out).unwrap();
    }

    #[test]
    fn strips_block_comments() {
        let input = b"{ /* block */ \"key\": \"value\" }";
        let out = apply(input).unwrap();
        assert!(!std::str::from_utf8(&out).unwrap().contains("/*"));
        let _: serde_json::Value = serde_json::from_slice(&out).unwrap();
    }

    #[test]
    fn strips_trailing_commas() {
        let input = b"{ \"a\": 1, }";
        let out = apply(input).unwrap();
        let _: serde_json::Value = serde_json::from_slice(&out).unwrap();
    }

    #[test]
    fn preserves_nested_content() {
        let input = b"{ \"mirrors\": [\"https://example.com\"] }";
        let v = round_trip(input);
        assert_eq!(v["mirrors"][0], "https://example.com");
    }

    #[test]
    fn docker_daemon_example() {
        let input = br#"{
  "registry-mirrors": [
    // tencent mirror
    "https://mirror.ccs.tencentyun.com"
  ]
}"#;
        let v = round_trip(input);
        assert_eq!(
            v["registry-mirrors"][0],
            "https://mirror.ccs.tencentyun.com"
        );
    }

    #[test]
    fn output_ends_with_newline() {
        let out = apply(b"{\"x\": 1}").unwrap();
        assert_eq!(out.last(), Some(&b'\n'));
    }

    #[test]
    fn deterministic_output() {
        let input = b"{ \"b\": 2, \"a\": 1 }";
        let out1 = apply(input).unwrap();
        let out2 = apply(input).unwrap();
        assert_eq!(out1, out2);
    }

    #[test]
    fn invalid_json_after_strip_errors() {
        assert!(apply(b"{ invalid }").is_err());
    }
}
