//! Pure top-level App JSON ownership rules shared by assessment, execution and recovery.
//! File observations, approval, journals and receipt commits remain with their callers.
use crate::install::hash_content;
use anyhow::{Context, Result, bail};
use serde_json::{Map as JsonMap, Value as JsonValue};

pub(crate) fn managed_json_hash(source: &[u8], managed_keys: &[String]) -> Result<u64> {
    Ok(hash_content(&serialize_json_object(
        &managed_json_payload(source, managed_keys)?,
    )?))
}

pub(crate) fn merge_managed_json_bytes(
    current: Option<&[u8]>,
    source: &[u8],
    managed_keys: &[String],
) -> Result<Vec<u8>> {
    let managed = managed_json_payload(source, managed_keys)?;
    let mut root = current
        .map(|bytes| parse_json_object(bytes, "json-merge: destination must be a JSON object"))
        .transpose()?
        .unwrap_or_default();
    for (key, value) in managed {
        root.insert(key, value);
    }
    serialize_json_object(&root)
}

pub(crate) fn remove_managed_json_bytes(
    current: &[u8],
    managed_keys: &[String],
) -> Result<Vec<u8>> {
    let mut root = parse_json_object(current, "json-merge: destination must be a JSON object")?;
    for key in managed_keys {
        root.remove(key);
    }
    serialize_json_object(&root)
}

pub(crate) fn restore_managed_json_bytes(
    current: &[u8],
    original: &[u8],
    managed_keys: &[String],
) -> Result<Vec<u8>> {
    let mut root = parse_json_object(current, "json-merge: destination must be a JSON object")?;
    let original = parse_json_object(original, "json-merge: rollback must be a JSON object")?;
    for key in managed_keys {
        if let Some(value) = original.get(key) {
            root.insert(key.clone(), value.clone());
        } else {
            root.remove(key);
        }
    }
    serialize_json_object(&root)
}

pub(crate) fn managed_json_keys_match(
    left: &[u8],
    right: &[u8],
    managed_keys: &[String],
) -> Result<bool> {
    let left = parse_json_object(left, "json-merge: destination must be a JSON object")?;
    let right = parse_json_object(right, "json-merge: comparison must be a JSON object")?;
    Ok(managed_keys
        .iter()
        .all(|key| left.get(key) == right.get(key)))
}

pub(crate) fn managed_json_keys_absent(bytes: &[u8], managed_keys: &[String]) -> Result<bool> {
    let root = parse_json_object(bytes, "json-merge: destination must be a JSON object")?;
    Ok(managed_keys.iter().all(|key| !root.contains_key(key)))
}

pub(crate) fn managed_json_payload(
    source: &[u8],
    managed_keys: &[String],
) -> Result<JsonMap<String, JsonValue>> {
    let source = parse_json_object(source, "json-merge: source must be a JSON object")?;
    managed_keys
        .iter()
        .map(|key| {
            source
                .get(key)
                .cloned()
                .map(|value| (key.clone(), value))
                .with_context(|| format!("json-merge: source missing managed key `{key}`"))
        })
        .collect()
}

pub(crate) fn installed_json_hash(bytes: &[u8], managed_keys: &[String]) -> Result<Option<u64>> {
    let current = parse_json_object(bytes, "json-merge: destination must be a JSON object")?;
    let managed = managed_keys
        .iter()
        .filter_map(|key| current.get(key).cloned().map(|value| (key.clone(), value)))
        .collect::<JsonMap<_, _>>();
    if managed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(hash_content(&serialize_json_object(&managed)?)))
    }
}

pub(crate) fn parse_json_object(
    bytes: &[u8],
    context: &'static str,
) -> Result<JsonMap<String, JsonValue>> {
    let value: JsonValue = serde_json::from_slice(bytes).context(context)?;
    let JsonValue::Object(object) = value else {
        bail!("{context}");
    };
    Ok(object)
}

pub(crate) fn serialize_json_object(object: &JsonMap<String, JsonValue>) -> Result<Vec<u8>> {
    let mut bytes =
        serde_json::to_vec_pretty(object).context("json-merge: serialization failed")?;
    if bytes.last() != Some(&b'\n') {
        bytes.push(b'\n');
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests;
