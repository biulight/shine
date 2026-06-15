use super::file_ops::{InstallOutcome, UninstallOutcome};
use super::manifest::{AppEntry, hash_content};
use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};
use std::path::Path;
use tokio::fs;

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

pub(super) async fn install(
    source_bytes: &[u8],
    destination: &Path,
    dry_run: bool,
    managed_keys: &[String],
) -> Result<InstallOutcome> {
    if dry_run {
        return Ok(InstallOutcome::DryRun);
    }

    let managed = managed_payload_from_source(source_bytes, managed_keys)?;
    let new_hash = hash_content(&serialize_object(&managed)?);

    let mut destination_object = if destination.exists() {
        let existing = fs::read(destination)
            .await
            .with_context(|| format!("reading destination JSON: {}", destination.display()))?;
        let existing_hash = installed_hash(&existing, managed_keys)?;
        if existing_hash == Some(new_hash) {
            return Ok(InstallOutcome::AlreadyManaged);
        }
        parse_json_object(&existing, "json-merge: destination must be a JSON object")?
    } else {
        Map::new()
    };

    for (key, value) in managed {
        destination_object.insert(key, value);
    }

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create directory: {}", parent.display()))?;
    }
    fs::write(destination, serialize_object(&destination_object)?)
        .await
        .with_context(|| format!("failed to write merged JSON: {}", destination.display()))?;
    Ok(InstallOutcome::Installed { hash: new_hash })
}

pub(super) async fn uninstall(
    entry: &AppEntry,
    dry_run: bool,
    force: bool,
    managed_keys: &[String],
) -> Result<UninstallOutcome> {
    if dry_run {
        return Ok(UninstallOutcome::DryRun);
    }
    if !entry.destination.exists() {
        return Ok(UninstallOutcome::NotFound);
    }

    let existing = fs::read(&entry.destination)
        .await
        .with_context(|| format!("reading: {}", entry.destination.display()))?;
    let Some(current_hash) = installed_hash(&existing, managed_keys)? else {
        return Ok(UninstallOutcome::NotFound);
    };
    let user_modified = current_hash != entry.content_hash;
    if user_modified && !force {
        return Ok(UninstallOutcome::UserModified);
    }

    let mut root = parse_json_object(&existing, "json-merge: destination must be a JSON object")?;
    for key in managed_keys {
        root.remove(key);
    }
    fs::write(&entry.destination, serialize_object(&root)?)
        .await
        .with_context(|| format!("writing: {}", entry.destination.display()))?;
    Ok(if user_modified {
        UninstallOutcome::ForceRemoved
    } else {
        UninstallOutcome::Removed
    })
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
