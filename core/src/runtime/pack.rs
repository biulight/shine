//! Deterministic, policy-gated Preset bundle construction.

use super::validation::{load_preset_source_scope, validate_preset_source_scope};
use super::{FileKind, FileSystemObservationHost};
use flate2::{Compression, GzBuilder};
use schemars::JsonSchema;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const PRESET_BUNDLE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct PresetPackReportV1 {
    pub schema_version: u32,
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub files: usize,
    pub archive_bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
}

pub struct PresetPackArtifactV1 {
    pub report: PresetPackReportV1,
    pub bytes: Vec<u8>,
}

#[derive(JsonSchema, Serialize)]
pub(crate) struct BundleManifestV1 {
    schema_version: u32,
    target: String,
    files: Vec<BundleFileV1>,
}

#[derive(JsonSchema, Serialize)]
pub(crate) struct BundleFileV1 {
    path: String,
    sha256: String,
    mode: u32,
}

pub async fn pack_preset_path(
    source_host: &impl FileSystemObservationHost,
    cwd: &Path,
    path: &Path,
) -> PresetPackArtifactV1 {
    let scope = match load_preset_source_scope(source_host, cwd, path).await {
        Ok(scope) => scope,
        Err(_) => return invalid(None, "invalid_input"),
    };
    if scope.categories.len() != 1
        || (scope.canonical != scope.categories[0].root
            && scope.canonical != scope.categories[0].root.join("shine.toml"))
    {
        return invalid(None, "single_category_required");
    }
    let validation = validate_preset_source_scope(&scope).await;
    let category = &scope.categories[0];
    let target = format!("{}/{}", category.kind, category.name);
    if !validation.valid {
        return invalid(Some(target), "preset_validation_failed");
    }
    let physical = match scan_tree(source_host, &category.root).await {
        Ok(physical) => physical,
        Err(code) => return invalid(Some(target), code),
    };
    let prefix = format!("{}/{}/", category.kind, category.name);
    let manifest_bytes = scope
        .snapshot
        .get(&format!("{prefix}shine.toml"))
        .unwrap_or_default();
    let declared = declared_paths(manifest_bytes);
    let mut files = Vec::new();
    let mut diagnostics = BTreeSet::new();
    for (logical, bytes) in scope.snapshot.files() {
        let Some(relative) = logical.strip_prefix(&prefix) else {
            continue;
        };
        if relative == super::PRESET_TEST_FIXTURE_FILE {
            continue;
        }
        if private_material(bytes) {
            diagnostics.insert("private_absolute_path".to_string());
        }
        if plaintext_secret_candidate(relative, bytes) {
            diagnostics.insert("plaintext_secret_candidate".to_string());
        }
        let mode = physical.get(relative).copied().unwrap_or(0o644);
        if relative != "shine.toml"
            && (mode == 0o755 || bytes.starts_with(b"#!"))
            && !declared.contains(relative)
        {
            diagnostics.insert("undeclared_executable_code".to_string());
        }
        files.push((relative.to_string(), bytes.clone(), mode));
    }
    if !diagnostics.is_empty() {
        return PresetPackArtifactV1 {
            report: PresetPackReportV1 {
                schema_version: PRESET_BUNDLE_SCHEMA_VERSION,
                valid: false,
                target: Some(target),
                files: 0,
                archive_bytes: 0,
                bundle_sha256: None,
                diagnostics: diagnostics.into_iter().collect(),
            },
            bytes: Vec::new(),
        };
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let manifest = BundleManifestV1 {
        schema_version: PRESET_BUNDLE_SCHEMA_VERSION,
        target: target.clone(),
        files: files
            .iter()
            .map(|(path, bytes, mode)| BundleFileV1 {
                path: path.clone(),
                sha256: sha256(bytes),
                mode: *mode,
            })
            .collect(),
    };
    let manifest_json = serde_json::to_vec_pretty(&manifest).expect("serializing bundle manifest");
    let bytes = match archive(&target, &manifest_json, &files) {
        Ok(bytes) => bytes,
        Err(_) => return invalid(Some(target), "bundle_encoding_failed"),
    };
    PresetPackArtifactV1 {
        report: PresetPackReportV1 {
            schema_version: PRESET_BUNDLE_SCHEMA_VERSION,
            valid: true,
            target: Some(target),
            files: files.len(),
            archive_bytes: bytes.len(),
            bundle_sha256: Some(sha256(&bytes)),
            diagnostics: Vec::new(),
        },
        bytes,
    }
}

async fn scan_tree(
    host: &impl FileSystemObservationHost,
    root: &Path,
) -> Result<BTreeMap<String, u32>, &'static str> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = BTreeMap::new();
    while let Some(directory) = pending.pop() {
        let entries = host.read_dir(&directory).await.map_err(|_| "read_failed")?;
        for entry in entries {
            let relative = entry.strip_prefix(root).map_err(|_| "path_escape")?;
            if relative
                .components()
                .any(|part| part.as_os_str() == "node_modules")
            {
                return Err("node_modules_forbidden");
            }
            let metadata = host.metadata(&entry).await.map_err(|_| "read_failed")?;
            match metadata.kind {
                FileKind::Directory => pending.push(entry),
                FileKind::Symlink => return Err("symlink_forbidden"),
                FileKind::File => {
                    let mode = if metadata.unix_mode.unwrap_or(0) & 0o111 != 0 {
                        0o755
                    } else {
                        0o644
                    };
                    files.insert(logical_path(relative), mode);
                }
            }
        }
    }
    Ok(files)
}

fn declared_paths(bytes: &[u8]) -> BTreeSet<String> {
    let Ok(value) = toml::from_slice::<toml::Value>(bytes) else {
        return BTreeSet::new();
    };
    let mut values = BTreeSet::new();
    collect_strings(&value, &mut values);
    values
}

fn collect_strings(value: &toml::Value, values: &mut BTreeSet<String>) {
    match value {
        toml::Value::String(value) => {
            values.insert(value.replace('\\', "/"));
        }
        toml::Value::Array(items) => {
            for item in items {
                collect_strings(item, values);
            }
        }
        toml::Value::Table(table) => {
            for value in table.values() {
                collect_strings(value, values);
            }
        }
        _ => {}
    }
}

fn private_material(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let normalized = text.replace('\\', "/");
    normalized.contains("/Users/")
        || normalized.contains("/home/")
        || normalized.to_ascii_lowercase().contains("c:/users/")
}

fn plaintext_secret_candidate(path: &str, bytes: &[u8]) -> bool {
    let name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if name == ".env" || name == "id_rsa" || name == "id_ed25519" || name.ends_with(".key") {
        return true;
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    text.contains("BEGIN PRIVATE KEY")
        || text.contains("BEGIN OPENSSH PRIVATE KEY")
        || text.contains("BEGIN RSA PRIVATE KEY")
}

fn archive(
    target: &str,
    manifest: &[u8],
    files: &[(String, Vec<u8>, u32)],
) -> anyhow::Result<Vec<u8>> {
    let encoder = GzBuilder::new()
        .mtime(0)
        .operating_system(255)
        .write(Vec::new(), Compression::best());
    let mut tar = tar::Builder::new(encoder);
    append(&mut tar, "shine.bundle.json", manifest, 0o644)?;
    for (path, bytes, mode) in files {
        append(&mut tar, &format!("preset/{target}/{path}"), bytes, *mode)?;
    }
    let encoder = tar.into_inner()?;
    Ok(encoder.finish()?)
}

fn append(
    tar: &mut tar::Builder<flate2::write::GzEncoder<Vec<u8>>>,
    path: &str,
    bytes: &[u8],
    mode: u32,
) -> anyhow::Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(mode);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_cksum();
    tar.append_data(&mut header, path, bytes)?;
    Ok(())
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn logical_path(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn invalid(target: Option<String>, code: &str) -> PresetPackArtifactV1 {
    PresetPackArtifactV1 {
        report: PresetPackReportV1 {
            schema_version: PRESET_BUNDLE_SCHEMA_VERSION,
            valid: false,
            target,
            files: 0,
            archive_bytes: 0,
            bundle_sha256: None,
            diagnostics: vec![code.to_string()],
        },
        bytes: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::InMemoryHost;
    use super::*;

    fn source(root: &str) -> InMemoryHost {
        let host = InMemoryHost::new();
        host.put_file(
            format!("{root}/app/demo/shine.toml"),
            b"description = 'Demo'\ndest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\ndescription = 'Config'\n".to_vec(),
        );
        host.put_file(
            format!("{root}/app/demo/config.toml"),
            b"enabled = true\n".to_vec(),
        );
        host.put_file(
            format!("{root}/app/demo/shine.test.toml"),
            b"author-only fixture\n".to_vec(),
        );
        host
    }

    #[tokio::test]
    async fn bundle_is_independent_of_checkout_root() {
        let left =
            pack_preset_path(&source("/one"), Path::new("/one"), Path::new("app/demo")).await;
        let right =
            pack_preset_path(&source("/two"), Path::new("/two"), Path::new("app/demo")).await;
        assert!(left.report.valid);
        assert_eq!(left.bytes, right.bytes);
        assert_eq!(left.report.bundle_sha256, right.report.bundle_sha256);
        let decoder = flate2::read::GzDecoder::new(left.bytes.as_slice());
        let mut archive = tar::Archive::new(decoder);
        let paths = archive
            .entries()
            .unwrap()
            .map(|entry| entry.unwrap().path().unwrap().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert!(!paths.iter().any(|path| path.ends_with("shine.test.toml")));
    }

    #[tokio::test]
    async fn bundle_rejects_ignored_dependency_trees() {
        let host = source("/repo");
        host.put_file(
            "/repo/app/demo/node_modules/pkg/index.js",
            b"code\n".to_vec(),
        );
        let artifact = pack_preset_path(&host, Path::new("/repo"), Path::new("app/demo")).await;
        assert!(!artifact.report.valid);
        assert_eq!(artifact.report.diagnostics, vec!["node_modules_forbidden"]);
    }

    #[tokio::test]
    async fn bundle_rejects_secret_candidates_and_undeclared_executables() {
        let host = source("/repo");
        host.put_file("/repo/app/demo/private.key", b"secret\n".to_vec());
        host.put_file("/repo/app/demo/helper.sh", b"#!/bin/sh\n".to_vec());
        let artifact = pack_preset_path(&host, Path::new("/repo"), Path::new("app/demo")).await;
        assert!(!artifact.report.valid);
        assert_eq!(
            artifact.report.diagnostics,
            vec!["plaintext_secret_candidate", "undeclared_executable_code"]
        );
    }
}
