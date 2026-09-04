//! Pure Preset metadata migration assessment.
//!
//! Candidate bytes remain process-local. The serializable report carries only
//! logical identities, hashes, actions, and stable diagnostics.

use super::{PresetSnapshot, PresetSourceKind};
use schemars::JsonSchema;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use toml_edit::{DocumentMut, Item, Table, Value, value};

pub const PRESET_MIGRATION_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PresetMigrationSeverityV1 {
    Advisory,
    Blocker,
}

#[derive(Clone, Copy, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PresetMigrationActionV1 {
    Update,
    RemoveOverride,
}

#[derive(Clone, Copy, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PresetMigrationStatusV1 {
    Current,
    Pending,
    Blocked,
    Applied,
    PartiallyApplied,
}

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct PresetMigrationDiagnosticV1 {
    pub severity: PresetMigrationSeverityV1,
    pub code: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_layer: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct PresetMigrationFileV1 {
    pub target: String,
    pub source_layer: String,
    pub action: PresetMigrationActionV1,
    pub operations: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_schema_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_schema_version: Option<u32>,
    pub original_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidate_sha256: Option<String>,
}

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct PresetMigrationSummaryV1 {
    pub files: usize,
    pub changes: usize,
    pub blockers: usize,
    pub advisories: usize,
}

#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
pub struct PresetMigrationReportV1 {
    pub schema_version: u32,
    pub scope: String,
    pub status: PresetMigrationStatusV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_set: Option<String>,
    pub summary: PresetMigrationSummaryV1,
    pub files: Vec<PresetMigrationFileV1>,
    pub diagnostics: Vec<PresetMigrationDiagnosticV1>,
}

#[derive(Clone, Debug)]
pub struct PresetMigrationEdit {
    pub logical_path: String,
    pub physical_path: PathBuf,
    pub source_layer: String,
    pub operations: Vec<String>,
    pub original: Vec<u8>,
    pub candidate: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct PresetMigrationPlan {
    pub report: PresetMigrationReportV1,
    pub edits: Vec<PresetMigrationEdit>,
}

#[derive(Clone, Debug)]
pub struct PresetMigrationBaseline<'a> {
    pub current: &'a PresetSnapshot,
    pub legacy_metadata_sha256: &'a BTreeMap<String, BTreeSet<String>>,
}

pub fn plan_preset_migration(
    snapshot: &PresetSnapshot,
    scope: impl Into<String>,
    selected_targets: Option<&BTreeSet<String>>,
    baseline: Option<PresetMigrationBaseline<'_>>,
) -> PresetMigrationPlan {
    let mut edits = Vec::new();
    let mut diagnostics = Vec::new();
    let mut categories = BTreeSet::new();

    for logical in snapshot.files().keys() {
        let Some(target) = category_target(logical) else {
            continue;
        };
        if selected_targets.is_some_and(|selected| !selected.contains(&target)) {
            continue;
        }
        categories.insert(target);
    }

    for target in &categories {
        let metadata_path = format!("{target}/shine.toml");
        let Some(original) = snapshot.get(&metadata_path) else {
            if target.starts_with("app/") || target.starts_with("shell/") {
                diagnostics.push(diagnostic(
                    PresetMigrationSeverityV1::Advisory,
                    "legacy_metadata",
                    target,
                    "legacy metadata remains compatible; add shine.toml when adopting the current authoring schema",
                ));
            }
            continue;
        };

        if let Some(candidate) =
            known_builtin_candidate(snapshot, &metadata_path, original, baseline.as_ref())
        {
            let requires_trust_review = baseline
                .as_ref()
                .and_then(|baseline| baseline.current.get(&metadata_path))
                .is_some_and(|current| !executable_paths(&metadata_path, current).is_empty())
                && source_layer(snapshot, &metadata_path) != "embedded"
                && target.starts_with("app/");
            let operations = if candidate.is_some() {
                vec!["rebase_released_builtin_metadata".to_string()]
            } else {
                vec!["remove_legacy_metadata_override".to_string()]
            };
            push_edit(
                snapshot,
                &metadata_path,
                original,
                candidate,
                operations,
                &mut edits,
                &mut diagnostics,
            );
            if requires_trust_review {
                diagnostics.push(diagnostic(
                    PresetMigrationSeverityV1::Advisory,
                    "external_code_trust_review_required",
                    target,
                    "migration never creates trust grants; migrated external executable code still requires a separate trust review",
                ));
            }
            continue;
        }

        match target.split_once('/').map(|(kind, _)| kind) {
            Some("app") => plan_app(
                snapshot,
                target,
                &metadata_path,
                original,
                &mut edits,
                &mut diagnostics,
            ),
            Some("shell") => plan_shell(target, original, &mut diagnostics),
            Some("sys") => plan_sys(target, original, &mut diagnostics),
            _ => {}
        }
    }

    for item in &mut diagnostics {
        item.source_layer = diagnostic_source_layer(snapshot, &item.target);
    }
    diagnostics.sort_by(|left, right| {
        (&left.target, &left.code, &left.message).cmp(&(&right.target, &right.code, &right.message))
    });
    edits.sort_by(|left, right| left.logical_path.cmp(&right.logical_path));
    let blockers = diagnostics
        .iter()
        .filter(|item| item.severity == PresetMigrationSeverityV1::Blocker)
        .count();
    let advisories = diagnostics.len() - blockers;
    let files = edits
        .iter()
        .map(|edit| PresetMigrationFileV1 {
            target: edit.logical_path.clone(),
            source_layer: source_layer(snapshot, &edit.logical_path).to_string(),
            action: if edit.candidate.is_some() {
                PresetMigrationActionV1::Update
            } else {
                PresetMigrationActionV1::RemoveOverride
            },
            operations: edit.operations.clone(),
            original_schema_version: metadata_schema_version(&edit.logical_path, &edit.original),
            candidate_schema_version: edit
                .candidate
                .as_deref()
                .or_else(|| snapshot.base_bytes(&edit.logical_path))
                .and_then(|bytes| metadata_schema_version(&edit.logical_path, bytes)),
            original_sha256: sha256(&edit.original),
            candidate_sha256: edit
                .candidate
                .as_deref()
                .or_else(|| snapshot.base_bytes(&edit.logical_path))
                .map(sha256),
        })
        .collect::<Vec<_>>();
    let status = if blockers > 0 {
        PresetMigrationStatusV1::Blocked
    } else if edits.is_empty() {
        PresetMigrationStatusV1::Current
    } else {
        PresetMigrationStatusV1::Pending
    };
    PresetMigrationPlan {
        report: PresetMigrationReportV1 {
            schema_version: PRESET_MIGRATION_SCHEMA_VERSION,
            scope: scope.into(),
            status,
            backup_set: None,
            summary: PresetMigrationSummaryV1 {
                files: categories.len(),
                changes: edits.len(),
                blockers,
                advisories,
            },
            files,
            diagnostics,
        },
        edits,
    }
}

fn known_builtin_candidate(
    snapshot: &PresetSnapshot,
    logical: &str,
    original: &[u8],
    baseline: Option<&PresetMigrationBaseline<'_>>,
) -> Option<Option<Vec<u8>>> {
    if logical.starts_with("sys/") {
        return None;
    }
    let baseline = baseline?;
    if !baseline
        .legacy_metadata_sha256
        .get(logical)
        .is_some_and(|hashes| hashes.contains(&sha256(original)))
    {
        return None;
    }
    let current = baseline.current.get(logical)?;
    for executable in executable_paths(logical, current) {
        if snapshot.get(&executable) != baseline.current.get(&executable) {
            return None;
        }
    }
    if snapshot.is_overlay(logical) && snapshot.base_bytes(logical) == Some(current) {
        Some(None)
    } else {
        Some(Some(current.to_vec()))
    }
}

fn executable_paths(logical: &str, current: &[u8]) -> BTreeSet<String> {
    let Ok(value) = toml::from_slice::<toml::Value>(current) else {
        return BTreeSet::new();
    };
    let prefix = logical.trim_end_matches("shine.toml");
    let mut paths = BTreeSet::new();
    let kind = logical.split('/').next().unwrap_or_default();
    if kind == "shell" {
        for file in value
            .get("files")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(source) = file.get("source").and_then(toml::Value::as_str) {
                paths.insert(format!("{prefix}{source}"));
            }
        }
    }
    collect_declared_executables(&value, prefix, &mut paths);
    paths
}

fn collect_declared_executables(value: &toml::Value, prefix: &str, paths: &mut BTreeSet<String>) {
    match value {
        toml::Value::Table(table) => {
            if let Some(filesystem) = table
                .get("permissions")
                .and_then(|permissions| permissions.get("filesystem"))
                .and_then(toml::Value::as_array)
            {
                for entry in filesystem {
                    let preset = entry.get("base").and_then(toml::Value::as_str) == Some("preset");
                    let execute = entry
                        .get("access")
                        .and_then(toml::Value::as_array)
                        .is_some_and(|items| {
                            items.iter().any(|item| item.as_str() == Some("execute"))
                        });
                    if preset
                        && execute
                        && let Some(path) = entry.get("path").and_then(toml::Value::as_str)
                    {
                        paths.insert(format!("{prefix}{path}"));
                    }
                }
            }
            for child in table.values() {
                collect_declared_executables(child, prefix, paths);
            }
        }
        toml::Value::Array(values) => {
            for child in values {
                collect_declared_executables(child, prefix, paths);
            }
        }
        _ => {}
    }
}

fn plan_app(
    snapshot: &PresetSnapshot,
    target: &str,
    logical: &str,
    original: &[u8],
    edits: &mut Vec<PresetMigrationEdit>,
    diagnostics: &mut Vec<PresetMigrationDiagnosticV1>,
) {
    let Ok(mut document) = std::str::from_utf8(original)
        .ok()
        .and_then(|text| text.parse::<DocumentMut>().ok())
        .ok_or(())
    else {
        diagnostics.push(diagnostic(
            PresetMigrationSeverityV1::Blocker,
            "invalid_app_metadata",
            target,
            "shine.toml is not valid UTF-8 TOML",
        ));
        return;
    };
    let Ok(parsed) = toml::from_slice::<toml::Value>(original) else {
        diagnostics.push(diagnostic(
            PresetMigrationSeverityV1::Blocker,
            "invalid_app_metadata",
            target,
            "shine.toml could not be parsed",
        ));
        return;
    };
    let version = parsed
        .get("metadata_schema_version")
        .and_then(toml::Value::as_integer)
        .unwrap_or(1);
    if version > 2 {
        diagnostics.push(diagnostic(
            PresetMigrationSeverityV1::Blocker,
            "unsupported_app_metadata_schema",
            target,
            "App metadata is newer than this Shine supports",
        ));
        return;
    }
    let category = target.strip_prefix("app/").unwrap_or_default();
    let mut changed = false;
    let mut operations = Vec::new();
    if version < 2 {
        document
            .as_table_mut()
            .insert("metadata_schema_version", value(2));
        changed = true;
        operations.push("set_app_metadata_schema_v2".to_string());
    }
    for key in ["post_install", "post_upgrade"] {
        let hook = parsed.get(key);
        let single_match = hook
            .is_some_and(|value| value.is_table() && is_recursive_artifact_hook(value, category));
        let indices = recursive_hook_indices(hook, category);
        let has_array_matches = !indices.is_empty();
        if single_match {
            document.remove(key);
            changed = true;
        } else if has_array_matches
            && let Some(array) = document
                .get_mut(key)
                .and_then(Item::as_value_mut)
                .and_then(Value::as_array_mut)
        {
            for index in indices.into_iter().rev() {
                array.remove(index);
            }
            if array.is_empty() {
                document.remove(key);
            }
            changed = true;
        }
        if single_match || has_array_matches {
            operations.push("remove_recursive_artifact_hook".to_string());
            diagnostics.push(diagnostic(
                PresetMigrationSeverityV1::Advisory,
                "recursive_artifact_hook_removed",
                target,
                "the recursive artifact hook was removed; artifact application is now an explicit operation",
            ));
        }
    }
    let candidate_text = document.to_string();
    let candidate_value = toml::from_str::<toml::Value>(&candidate_text).unwrap_or(parsed);
    if candidate_value.get("permissions").is_none() {
        if app_has_opaque_code(&candidate_value) {
            diagnostics.push(diagnostic(
                PresetMigrationSeverityV1::Blocker,
                "manual_permission_review_required",
                target,
                "executable App metadata is missing a reviewed `[permissions]` declaration",
            ));
        } else {
            let mut permissions = Table::new();
            permissions.insert("schema_version", value(1));
            document
                .as_table_mut()
                .insert("permissions", Item::Table(permissions));
            changed = true;
            operations.push("add_empty_permission_schema_v1".to_string());
        }
    }
    if changed {
        push_edit(
            snapshot,
            logical,
            original,
            Some(document.to_string().into_bytes()),
            operations,
            edits,
            diagnostics,
        );
    }
}

fn recursive_hook_indices(value: Option<&toml::Value>, category: &str) -> Vec<usize> {
    value
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, hook)| is_recursive_artifact_hook(hook, category).then_some(index))
        .collect()
}

fn is_recursive_artifact_hook(hook: &toml::Value, category: &str) -> bool {
    let command = hook.get("command").and_then(toml::Value::as_str);
    let args = hook
        .get("args")
        .and_then(toml::Value::as_array)
        .map(|args| {
            args.iter()
                .filter_map(toml::Value::as_str)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    command == Some("shine") && args == ["app", "artifact", "apply", category]
}

fn app_has_opaque_code(value: &toml::Value) -> bool {
    ["post_install", "post_upgrade", "artifact"]
        .iter()
        .any(|key| value.get(*key).is_some())
        || value
            .get("files")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
            .any(|file| file.get("generator").is_some())
}

fn plan_shell(target: &str, original: &[u8], diagnostics: &mut Vec<PresetMigrationDiagnosticV1>) {
    let Ok(value) = toml::from_slice::<toml::Value>(original) else {
        diagnostics.push(diagnostic(
            PresetMigrationSeverityV1::Blocker,
            "invalid_shell_metadata",
            target,
            "shine.toml could not be parsed",
        ));
        return;
    };
    for file in value
        .get("files")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
    {
        if file.get("permissions").is_none() {
            let name = file
                .get("target")
                .or_else(|| file.get("source"))
                .and_then(toml::Value::as_str)
                .unwrap_or("unknown");
            diagnostics.push(diagnostic(
                PresetMigrationSeverityV1::Blocker,
                "manual_permission_review_required",
                &format!("{target}/{name}"),
                "Shell command is missing `[files.permissions]`; permissions cannot be inferred safely from its source",
            ));
        }
    }
}

fn plan_sys(target: &str, original: &[u8], diagnostics: &mut Vec<PresetMigrationDiagnosticV1>) {
    let Ok(value) = toml::from_slice::<toml::Value>(original) else {
        diagnostics.push(diagnostic(
            PresetMigrationSeverityV1::Blocker,
            "invalid_sys_metadata",
            target,
            "shine.toml could not be parsed",
        ));
        return;
    };
    if value.get("version").and_then(toml::Value::as_integer) != Some(2) {
        diagnostics.push(diagnostic(
            PresetMigrationSeverityV1::Blocker,
            "sys_v1_manual_migration_required",
            target,
            "Sys v1 dispatchers must be split into v2 detect/install items; see the Sys Preset v2 migration guide",
        ));
        return;
    }
    for item in value
        .get("items")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
    {
        if item.get("permissions").is_none() {
            let name = item
                .get("id")
                .and_then(toml::Value::as_str)
                .unwrap_or("unknown");
            diagnostics.push(diagnostic(
                PresetMigrationSeverityV1::Blocker,
                "manual_permission_review_required",
                &format!("sys/{name}"),
                "Sys item is missing `[items.permissions]`; permissions cannot be inferred safely from scripts or profile code",
            ));
        }
    }
}

fn push_edit(
    snapshot: &PresetSnapshot,
    logical: &str,
    original: &[u8],
    candidate: Option<Vec<u8>>,
    operations: Vec<String>,
    edits: &mut Vec<PresetMigrationEdit>,
    diagnostics: &mut Vec<PresetMigrationDiagnosticV1>,
) {
    let Some(path) = snapshot
        .origin(logical)
        .and_then(|origin| origin.physical_path.clone())
    else {
        diagnostics.push(diagnostic(
            PresetMigrationSeverityV1::Blocker,
            "embedded_metadata_read_only",
            logical.trim_end_matches("/shine.toml"),
            "embedded Preset metadata is read-only",
        ));
        return;
    };
    edits.push(PresetMigrationEdit {
        logical_path: logical.to_string(),
        physical_path: path,
        source_layer: source_layer(snapshot, logical).to_string(),
        operations,
        original: original.to_vec(),
        candidate,
    });
}

fn metadata_schema_version(logical: &str, bytes: &[u8]) -> Option<u32> {
    let value = toml::from_slice::<toml::Value>(bytes).ok()?;
    if logical.starts_with("app/") {
        return value
            .get("metadata_schema_version")
            .and_then(toml::Value::as_integer)
            .unwrap_or(1)
            .try_into()
            .ok();
    }
    if logical.starts_with("sys/") {
        return value
            .get("version")
            .and_then(toml::Value::as_integer)
            .and_then(|version| u32::try_from(version).ok());
    }
    None
}

fn source_layer(snapshot: &PresetSnapshot, logical: &str) -> &'static str {
    match snapshot.origin(logical).map(|origin| origin.source_kind) {
        Some(PresetSourceKind::Embedded) => "embedded",
        Some(PresetSourceKind::External) => "external",
        Some(PresetSourceKind::Overlay) => "overlay",
        None => "unknown",
    }
}

fn category_target(logical: &str) -> Option<String> {
    let mut parts = logical.split('/');
    let kind = parts.next()?;
    let name = parts.next()?;
    let _file = parts.next()?;
    (matches!(kind, "app" | "shell" | "sys") && !name.starts_with('.'))
        .then(|| format!("{kind}/{name}"))
}

fn diagnostic(
    severity: PresetMigrationSeverityV1,
    code: &str,
    target: &str,
    message: &str,
) -> PresetMigrationDiagnosticV1 {
    PresetMigrationDiagnosticV1 {
        severity,
        code: code.to_string(),
        target: target.to_string(),
        source_layer: None,
        message: message.to_string(),
    }
}

fn diagnostic_source_layer(snapshot: &PresetSnapshot, target: &str) -> Option<String> {
    let mut parts = target.split('/');
    let kind = parts.next()?;
    let name = parts.next()?;
    let category = format!("{kind}/{name}");
    let direct = format!("{category}/shine.toml");
    if snapshot.get(&direct).is_some() {
        return Some(source_layer(snapshot, &direct).to_string());
    }
    if kind == "sys" {
        for (logical, bytes) in snapshot.files() {
            if !logical.starts_with("sys/") || !logical.ends_with("/shine.toml") {
                continue;
            }
            let Ok(value) = toml::from_slice::<toml::Value>(bytes) else {
                continue;
            };
            let contains = value
                .get("items")
                .and_then(toml::Value::as_array)
                .is_some_and(|items| {
                    items
                        .iter()
                        .any(|item| item.get("id").and_then(toml::Value::as_str) == Some(name))
                });
            if contains {
                return Some(source_layer(snapshot, logical).to_string());
            }
        }
    }
    None
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_v1_static_metadata_is_migrated_without_touching_payload() {
        let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .base_root("/presets")
            .file(
                "app/demo/shine.toml",
                b"# keep\ndest = '~/.demo'\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"secret payload\n".to_vec())
            .build();
        let plan = plan_preset_migration(&snapshot, "test", None, None);
        assert_eq!(plan.edits.len(), 1);
        let candidate = String::from_utf8(plan.edits[0].candidate.clone().unwrap()).unwrap();
        assert!(candidate.contains("# keep"));
        assert!(candidate.contains("metadata_schema_version = 2"));
        assert!(candidate.contains("[permissions]"));
        assert_eq!(
            snapshot.get("app/demo/config.toml"),
            Some(b"secret payload\n".as_slice())
        );
    }

    #[test]
    fn safe_app_edit_is_retained_when_an_independent_sys_blocker_exists() {
        let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .base_root("/presets")
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.demo'\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", Vec::new())
            .file("sys/macos/shine.toml", b"version = 1\n".to_vec())
            .file("sys/macos/init.sh", b"#!/bin/sh\n".to_vec())
            .build();

        let plan = plan_preset_migration(&snapshot, "test", None, None);

        assert_eq!(plan.edits.len(), 1);
        assert_eq!(plan.report.summary.blockers, 1);
        assert_eq!(plan.report.status, PresetMigrationStatusV1::Blocked);
    }

    #[test]
    fn reusable_diagnostics_are_factual_and_do_not_embed_cli_commands() {
        let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .base_root("/presets")
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.demo'\n[artifact]\nscript = 'build.ts'\n".to_vec(),
            )
            .file("app/demo/build.ts", Vec::new())
            .file(
                "shell/demo/shine.toml",
                b"[[files]]\nsource = 'run.sh'\ntarget = 'run'\n".to_vec(),
            )
            .file("shell/demo/run.sh", Vec::new())
            .file(
                "sys/linux/shine.toml",
                b"version = 2\n[[items]]\nid = 'tool'\ninstall = { kind = 'script', path = 'install.sh' }\n".to_vec(),
            )
            .file("sys/linux/install.sh", Vec::new())
            .build();

        let plan = plan_preset_migration(&snapshot, "test", None, None);

        assert!(
            plan.report
                .diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.message.contains("shine "))
        );
        assert!(plan.report.diagnostics.iter().any(|diagnostic| {
            diagnostic.target == "shell/demo/run"
                && diagnostic.message.contains("[files.permissions]")
        }));
    }

    #[test]
    fn recursive_artifact_hook_is_removed_but_other_hooks_remain() {
        let metadata = br#"dest = '~/.demo'
post_install = [{ command = 'shine', args = ['app', 'artifact', 'apply', 'demo'] }]
post_upgrade = [{ command = 'shine', args = ['app', 'artifact', 'apply', 'other'] }]
[artifact]
script = 'build.ts'
"#;
        let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .base_root("/presets")
            .file("app/demo/shine.toml", metadata.to_vec())
            .file("app/demo/build.ts", Vec::new())
            .build();
        let plan = plan_preset_migration(&snapshot, "test", None, None);
        let candidate = String::from_utf8(plan.edits[0].candidate.clone().unwrap()).unwrap();
        assert!(!candidate.contains("'demo'"));
        assert!(candidate.contains("'other'"));
        assert_eq!(plan.report.summary.blockers, 1);
    }

    #[test]
    fn single_recursive_artifact_hook_is_removed_exactly() {
        let metadata = br#"dest = '~/.demo'
post_install = { command = 'shine', args = ['app', 'artifact', 'apply', 'demo'] }
[[files]]
source = 'config.toml'
"#;
        let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .base_root("/presets")
            .file("app/demo/shine.toml", metadata.to_vec())
            .file("app/demo/config.toml", Vec::new())
            .build();

        let plan = plan_preset_migration(&snapshot, "test", None, None);
        let candidate = String::from_utf8(plan.edits[0].candidate.clone().unwrap()).unwrap();
        assert!(!candidate.contains("post_install"));
        assert_eq!(plan.report.summary.blockers, 0);
        assert_eq!(
            plan.report.files[0].operations,
            [
                "set_app_metadata_schema_v2",
                "remove_recursive_artifact_hook",
                "add_empty_permission_schema_v1"
            ]
        );
    }

    #[test]
    fn exact_legacy_builtin_metadata_rebases_and_overlay_can_be_removed() {
        let logical = "app/demo/shine.toml";
        let old = b"dest = '~/.demo'\n".to_vec();
        let current =
            b"metadata_schema_version = 2\ndest = '~/.demo'\n[permissions]\nschema_version = 1\n"
                .to_vec();
        let baseline = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(logical, current.clone())
            .build();
        let hashes = BTreeMap::from([(logical.to_string(), BTreeSet::from([sha256(&old)]))]);

        let external = PresetSnapshot::builder(PresetSourceKind::External)
            .base_root("/external")
            .file(logical, old.clone())
            .build();
        let update = plan_preset_migration(
            &external,
            "test",
            None,
            Some(PresetMigrationBaseline {
                current: &baseline,
                legacy_metadata_sha256: &hashes,
            }),
        );
        assert_eq!(
            update.edits[0].candidate.as_deref(),
            Some(current.as_slice())
        );
        assert_eq!(update.report.files[0].original_schema_version, Some(1));
        assert_eq!(update.report.files[0].candidate_schema_version, Some(2));
        let json = serde_json::to_string(&update.report).unwrap();
        assert!(!json.contains("/external"));
        assert!(!json.contains("dest ="));

        let overlay = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(logical, current)
            .overlay_root("/overlay")
            .overlay_file(logical, old)
            .build();
        let remove = plan_preset_migration(
            &overlay,
            "test",
            None,
            Some(PresetMigrationBaseline {
                current: &baseline,
                legacy_metadata_sha256: &hashes,
            }),
        );
        assert!(remove.edits[0].candidate.is_none());
        assert_eq!(
            remove.report.files[0].action,
            PresetMigrationActionV1::RemoveOverride
        );
    }

    #[test]
    fn exact_legacy_sys_metadata_still_requires_manual_dispatcher_migration() {
        let logical = "sys/macos/shine.toml";
        let old = b"version = 1\n".to_vec();
        let current = b"version = 2\n".to_vec();
        let baseline = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(logical, current)
            .build();
        let hashes = BTreeMap::from([(logical.to_string(), BTreeSet::from([sha256(&old)]))]);
        let external = PresetSnapshot::builder(PresetSourceKind::External)
            .base_root("/external")
            .file(logical, old)
            .build();

        let plan = plan_preset_migration(
            &external,
            "test",
            None,
            Some(PresetMigrationBaseline {
                current: &baseline,
                legacy_metadata_sha256: &hashes,
            }),
        );

        assert!(plan.edits.is_empty());
        assert!(
            plan.report
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.code == "sys_v1_manual_migration_required" })
        );
    }
}
