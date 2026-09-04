use super::manifest::hash_content;
use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};

pub(super) fn managed_hash(source_bytes: &[u8], managed_keys: &[String]) -> Result<u64> {
    let managed = managed_payload_from_source(source_bytes, managed_keys)?;
    Ok(hash_content(&serialize_object(&managed)?))
}

pub(super) fn installed_hash(current_bytes: &[u8], managed_keys: &[String]) -> Result<Option<u64>> {
    let current = parse_json_object(
        current_bytes,
        "json-merge: destination must be a JSON object",
    )?;
    let managed = collect_managed_subset(&current, managed_keys);
    if managed.is_empty() {
        return Ok(None);
    }
    Ok(Some(hash_content(&serialize_object(&managed)?)))
}

fn managed_payload_from_source(
    source_bytes: &[u8],
    managed_keys: &[String],
) -> Result<Map<String, Value>> {
    let source = parse_json_object(source_bytes, "json-merge: source must be a JSON object")?;
    let mut managed = Map::new();
    for key in managed_keys {
        let value = source
            .get(key)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("json-merge: source missing managed key `{key}`"))?;
        managed.insert(key.clone(), value);
    }
    Ok(managed)
}

fn collect_managed_subset(
    source: &Map<String, Value>,
    managed_keys: &[String],
) -> Map<String, Value> {
    let mut managed = Map::new();
    for key in managed_keys {
        if let Some(value) = source.get(key).cloned() {
            managed.insert(key.clone(), value);
        }
    }
    managed
}

fn parse_json_object(bytes: &[u8], context: &'static str) -> Result<Map<String, Value>> {
    let value: Value = serde_json::from_slice(bytes).context(context)?;
    let Value::Object(object) = value else {
        bail!("{context}");
    };
    Ok(object)
}

fn serialize_object(object: &Map<String, Value>) -> Result<Vec<u8>> {
    let mut out = serde_json::to_vec_pretty(object).context("json-merge: serialization failed")?;
    if out.last() != Some(&b'\n') {
        out.push(b'\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_hash_ignores_unmanaged_keys() {
        let hash = installed_hash(
            br#"{
  "proxy": { "mode": "manual" },
  "containersProxy": { "mode": "system" },
  "theme": "dark"
}"#,
            &["proxy".to_string(), "containersProxy".to_string()],
        )
        .unwrap();

        assert!(hash.is_some());
    }

    #[test]
    fn installed_hash_returns_none_when_managed_keys_missing() {
        let hash = installed_hash(
            br#"{ "theme": "dark" }"#,
            &["proxy".to_string(), "containersProxy".to_string()],
        )
        .unwrap();

        assert!(hash.is_none());
    }
}
