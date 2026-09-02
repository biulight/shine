//! Reviewed CLI adapter for Preset source migration.

use crate::commands::PresetReportFormat;
use crate::config::discover_runtime_paths_read_only;
use crate::{core_runtime, persist};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use shine_core::runtime::{
    InMemoryHost, PresetDiagnosticSeverity, PresetMigrationBaseline, PresetMigrationDiagnosticV1,
    PresetMigrationEdit, PresetMigrationPlan, PresetMigrationSeverityV1, PresetMigrationStatusV1,
    PresetSnapshot, PresetSnapshotRequest, PresetSnapshotSource, RealHost,
    capture_embedded_preset_snapshot, capture_preset_snapshot, plan_preset_migration, sha256,
    validate_preset_path,
};
use std::collections::{BTreeMap, BTreeSet};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

pub async fn handle_migrate(
    path: Option<&Path>,
    dry_run: bool,
    yes: bool,
    format: PresetReportFormat,
) -> Result<bool> {
    if format == PresetReportFormat::Json && !dry_run && !yes {
        bail!("`shine preset migrate --format json` requires --dry-run or --yes");
    }

    let (snapshot, scope, selected, shine_dir, managed_overlay) = migration_inputs(path).await?;
    let current = capture_embedded_preset_snapshot(core_runtime::embedded_preset_files());
    let legacy = legacy_metadata_hashes();
    let mut plan = plan_preset_migration(
        &snapshot,
        scope,
        selected.as_ref(),
        Some(PresetMigrationBaseline {
            current: &current,
            legacy_metadata_sha256: &legacy,
        }),
    );
    validate_candidate(&snapshot, &mut plan).await;
    let display_edits = plan.edits.clone();
    if let Some(root) = managed_overlay.as_deref() {
        mark_managed_overlay_read_only(&mut plan, root);
    }

    if format == PresetReportFormat::Text {
        print_text(&plan);
        print_diffs(&display_edits);
    }
    if dry_run {
        if format == PresetReportFormat::Json {
            println!("{}", serde_json::to_string_pretty(&plan.report)?);
        }
        return Ok(plan.report.summary.blockers == 0);
    }

    if plan.edits.is_empty() {
        if format == PresetReportFormat::Json {
            println!("{}", serde_json::to_string_pretty(&plan.report)?);
        }
        return Ok(plan.report.summary.blockers == 0);
    }
    if !yes {
        if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
            bail!("Preset migration approval requires an interactive terminal or explicit --yes");
        }
        let approved = dialoguer::Confirm::new()
            .with_prompt("Apply this Preset migration?")
            .default(false)
            .interact()?;
        if !approved {
            bail!("Preset migration was not approved; no changes were made");
        }
    }

    let sources = migration_source_observations(&snapshot, &plan.edits);
    let backup = create_backup_set(&shine_dir, &plan.edits, &sources).await?;
    plan.report.backup_set = backup
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| format!("preset-migration-backups/{name}"));
    if let Err(error) = apply_edits(&plan.edits, &sources).await {
        bail!(
            "Preset migration stopped; backup retained at {}: {error:#}",
            backup.display()
        );
    }
    plan.report.status = if plan.report.summary.blockers > 0 {
        PresetMigrationStatusV1::PartiallyApplied
    } else {
        PresetMigrationStatusV1::Applied
    };
    if format == PresetReportFormat::Json {
        println!("{}", serde_json::to_string_pretty(&plan.report)?);
    } else {
        println!();
        println!("Migrated {} Preset metadata file(s).", plan.edits.len());
        println!("Backup: {}", backup.display());
        if plan.report.summary.blockers > 0 {
            println!(
                "Manual migration is still required for {} blocker(s).",
                plan.report.summary.blockers
            );
        }
    }
    Ok(plan.report.summary.blockers == 0)
}

pub async fn active_compatibility_plan(target: Option<&str>) -> Result<PresetMigrationPlan> {
    let (snapshot, scope, mut selected, _, managed_overlay) = migration_inputs(None).await?;
    if let Some(target) = target {
        let canonical = if let Some(item) = target.strip_prefix("sys/") {
            let mut categories = sys_categories_for_item(&snapshot, item);
            if categories.is_empty() {
                let os_id = crate::sys::detect_os_id().await?;
                let active = format!("sys/{os_id}");
                if snapshot.get(&format!("{active}/shine.toml")).is_some() {
                    categories.insert(active);
                }
            }
            categories
        } else if target.starts_with("app/") || target.starts_with("shell/") {
            BTreeSet::from([target.split('/').take(2).collect::<Vec<_>>().join("/")])
        } else {
            let categories = snapshot
                .files()
                .keys()
                .filter_map(|path| {
                    let mut parts = path.split('/');
                    let kind = parts.next()?;
                    let name = parts.next()?;
                    (name == target).then(|| format!("{kind}/{name}"))
                })
                .collect::<BTreeSet<_>>();
            if categories.len() == 1 {
                categories
            } else {
                let sys_categories = sys_categories_for_item(&snapshot, target);
                if sys_categories.is_empty() {
                    BTreeSet::from([target.to_string()])
                } else {
                    sys_categories
                }
            }
        };
        selected = Some(canonical);
    }
    let current = capture_embedded_preset_snapshot(core_runtime::embedded_preset_files());
    let legacy = legacy_metadata_hashes();
    let mut plan = plan_preset_migration(
        &snapshot,
        scope,
        selected.as_ref(),
        Some(PresetMigrationBaseline {
            current: &current,
            legacy_metadata_sha256: &legacy,
        }),
    );
    validate_candidate(&snapshot, &mut plan).await;
    if let Some(root) = managed_overlay.as_deref() {
        mark_managed_overlay_read_only(&mut plan, root);
    }
    Ok(plan)
}

fn sys_categories_for_item(snapshot: &PresetSnapshot, item: &str) -> BTreeSet<String> {
    snapshot
        .files()
        .iter()
        .filter_map(|(logical, bytes)| {
            let category = logical.strip_prefix("sys/")?.strip_suffix("/shine.toml")?;
            if category == item {
                return Some(format!("sys/{category}"));
            }
            let value = toml::from_slice::<toml::Value>(bytes).ok()?;
            value
                .get("items")
                .and_then(toml::Value::as_array)
                .is_some_and(|items| {
                    items
                        .iter()
                        .any(|entry| entry.get("id").and_then(toml::Value::as_str) == Some(item))
                })
                .then(|| format!("sys/{category}"))
        })
        .collect()
}

pub fn print_compatibility(plan: &PresetMigrationPlan) {
    if plan.edits.is_empty() && plan.report.diagnostics.is_empty() {
        return;
    }
    println!("Preset compatibility");
    for file in &plan.report.files {
        println!("  migrate {} ({})", file.target, file.source_layer);
    }
    for diagnostic in &plan.report.diagnostics {
        let marker = if diagnostic.severity == PresetMigrationSeverityV1::Blocker {
            "!"
        } else {
            "i"
        };
        println!(
            "  {marker} {}{}: {} [{}]",
            diagnostic.target,
            diagnostic
                .source_layer
                .as_deref()
                .map(|layer| format!(" ({layer})"))
                .unwrap_or_default(),
            diagnostic.message,
            diagnostic.code
        );
    }
    println!("  Run `shine preset migrate --dry-run` to review the migration.");
}

pub fn compatibility_required(plan: &PresetMigrationPlan) -> bool {
    !plan.edits.is_empty() || plan.report.summary.blockers > 0
}

async fn migration_inputs(
    path: Option<&Path>,
) -> Result<(
    PresetSnapshot,
    String,
    Option<BTreeSet<String>>,
    PathBuf,
    Option<PathBuf>,
)> {
    let runtime = discover_runtime_paths_read_only().context("resolving active Preset paths")?;
    if let Some(path) = path {
        let canonical = tokio::fs::canonicalize(path)
            .await
            .with_context(|| format!("resolving Preset path {}", path.display()))?;
        let (root, selected) = explicit_scope(&canonical)?;
        let snapshot = capture_preset_snapshot(
            &RealHost,
            PresetSnapshotRequest {
                source: PresetSnapshotSource::External(root.clone()),
                overlay_root: None,
            },
        )
        .await?;
        let managed = runtime
            .managed_overlay
            .then_some(runtime.presets_overlay_dir)
            .flatten()
            .filter(|overlay| canonical.starts_with(overlay));
        let scope = selected
            .as_ref()
            .and_then(|targets| targets.iter().next())
            .cloned()
            .unwrap_or_else(|| "explicit-repository".to_string());
        return Ok((snapshot, scope, selected, runtime.shine_dir, managed));
    }

    let source = if runtime.is_external_presets {
        PresetSnapshotSource::External(runtime.presets_dir.clone())
    } else {
        PresetSnapshotSource::Embedded(core_runtime::embedded_preset_files())
    };
    let snapshot = capture_preset_snapshot(
        &RealHost,
        PresetSnapshotRequest {
            source,
            overlay_root: runtime.presets_overlay_dir.clone(),
        },
    )
    .await?;
    let managed = runtime
        .managed_overlay
        .then_some(runtime.presets_overlay_dir)
        .flatten();
    Ok((
        snapshot,
        "active".to_string(),
        None,
        runtime.shine_dir,
        managed,
    ))
}

fn explicit_scope(path: &Path) -> Result<(PathBuf, Option<BTreeSet<String>>)> {
    let category = if path.is_file() {
        if path.file_name().and_then(|name| name.to_str()) != Some("shine.toml") {
            bail!("Preset migration file input must be shine.toml");
        }
        path.parent()
            .context("shine.toml has no category directory")?
    } else {
        path
    };
    if let (Some(name), Some(kind_dir)) = (
        category.file_name().and_then(|name| name.to_str()),
        category.parent(),
    ) && let Some(kind) = kind_dir.file_name().and_then(|name| name.to_str())
        && matches!(kind, "app" | "shell" | "sys")
    {
        let root = kind_dir
            .parent()
            .context("Preset category has no repository root")?;
        return Ok((
            root.to_path_buf(),
            Some(BTreeSet::from([format!("{kind}/{name}")])),
        ));
    }
    if path.is_file() {
        bail!("shine.toml must be under app/<name>, shell/<name>, or sys/<name>");
    }
    Ok((path.to_path_buf(), None))
}

fn mark_managed_overlay_read_only(plan: &mut PresetMigrationPlan, root: &Path) {
    let blocked = plan
        .edits
        .iter()
        .filter(|edit| edit.physical_path.starts_with(root))
        .map(|edit| edit.logical_path.clone())
        .collect::<BTreeSet<_>>();
    if blocked.is_empty() {
        return;
    }
    plan.edits
        .retain(|edit| !blocked.contains(&edit.logical_path));
    for target in blocked {
        plan.report.diagnostics.push(PresetMigrationDiagnosticV1 {
            severity: PresetMigrationSeverityV1::Blocker,
            code: "managed_overlay_read_only".to_string(),
            target,
            source_layer: Some("overlay".to_string()),
            message: "the active Git-managed overlay is force-mirrored; migrate its upstream checkout instead".to_string(),
        });
    }
    sync_report_with_edits(plan);
}

async fn validate_candidate(snapshot: &PresetSnapshot, plan: &mut PresetMigrationPlan) {
    // `validate_preset_path` derives a synthetic home beneath this root. Keep the
    // root absolute according to the native path semantics so its all-platform
    // metadata validation can run on Windows too.
    #[cfg(windows)]
    let root = Path::new(r"C:\shine-preset-migration");
    #[cfg(not(windows))]
    let root = Path::new("/shine-preset-migration");
    let host = InMemoryHost::new();
    let candidate_targets = plan
        .edits
        .iter()
        .map(|edit| {
            edit.logical_path
                .split('/')
                .take(2)
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect::<BTreeSet<_>>();
    if candidate_targets.is_empty() {
        return;
    }
    let mut files = snapshot.files().clone();
    for edit in &plan.edits {
        match &edit.candidate {
            Some(candidate) => {
                files.insert(edit.logical_path.clone(), candidate.clone());
            }
            None => {
                if let Some(base) = snapshot.base_bytes(&edit.logical_path) {
                    files.insert(edit.logical_path.clone(), base.to_vec());
                } else {
                    files.remove(&edit.logical_path);
                }
            }
        }
    }
    for (logical, bytes) in files {
        let target = logical.split('/').take(2).collect::<Vec<_>>().join("/");
        if !candidate_targets.contains(&target) {
            continue;
        }
        host.put_file(root.join(logical), bytes);
    }
    let validation = validate_preset_path(&host, root, root).await;
    let invalid_targets = validation
        .categories
        .iter()
        .filter(|category| !category.valid)
        .map(|category| format!("{}/{}", category.kind, category.name))
        .collect::<BTreeSet<_>>();
    for category in validation
        .categories
        .iter()
        .filter(|category| !category.valid)
    {
        let target = format!("{}/{}", category.kind, category.name);
        for item in category
            .diagnostics
            .iter()
            .filter(|item| item.severity == PresetDiagnosticSeverity::Error)
        {
            plan.report.diagnostics.push(PresetMigrationDiagnosticV1 {
                severity: PresetMigrationSeverityV1::Blocker,
                code: format!("candidate_{}", item.code),
                target: target.clone(),
                source_layer: report_source_layer(plan, &target),
                message: item.message.clone(),
            });
        }
    }
    for item in validation
        .diagnostics
        .iter()
        .filter(|item| item.severity == PresetDiagnosticSeverity::Error)
    {
        plan.report.diagnostics.push(PresetMigrationDiagnosticV1 {
            severity: PresetMigrationSeverityV1::Blocker,
            code: format!("candidate_{}", item.code),
            target: plan.report.scope.clone(),
            source_layer: None,
            message: item.message.clone(),
        });
    }
    if !invalid_targets.is_empty() {
        plan.edits.retain(|edit| {
            let target = edit
                .logical_path
                .split('/')
                .take(2)
                .collect::<Vec<_>>()
                .join("/");
            !invalid_targets.contains(&target)
        });
    }
    sync_report_with_edits(plan);
}

fn sync_report_with_edits(plan: &mut PresetMigrationPlan) {
    let edited = plan
        .edits
        .iter()
        .map(|edit| edit.logical_path.as_str())
        .collect::<BTreeSet<_>>();
    plan.report
        .files
        .retain(|file| edited.contains(file.target.as_str()));
    plan.report.summary.changes = plan.edits.len();
    plan.report.summary.blockers = plan
        .report
        .diagnostics
        .iter()
        .filter(|item| item.severity == PresetMigrationSeverityV1::Blocker)
        .count();
    plan.report.status = if plan.report.summary.blockers > 0 {
        PresetMigrationStatusV1::Blocked
    } else if plan.edits.is_empty() {
        PresetMigrationStatusV1::Current
    } else {
        PresetMigrationStatusV1::Pending
    };
}

fn report_source_layer(plan: &PresetMigrationPlan, target: &str) -> Option<String> {
    plan.report
        .files
        .iter()
        .find(|file| file.target.starts_with(target))
        .map(|file| file.source_layer.clone())
}

fn print_text(plan: &PresetMigrationPlan) {
    println!(
        "Preset migration: {}",
        match plan.report.status {
            PresetMigrationStatusV1::Current => "current",
            PresetMigrationStatusV1::Pending => "changes pending",
            PresetMigrationStatusV1::Blocked => "manual review required",
            PresetMigrationStatusV1::Applied => "applied",
            PresetMigrationStatusV1::PartiallyApplied => "partially applied",
        }
    );
    for diagnostic in &plan.report.diagnostics {
        let severity = if diagnostic.severity == PresetMigrationSeverityV1::Blocker {
            "error"
        } else {
            "note"
        };
        println!(
            "  {severity}[{}]: {}{}: {}",
            diagnostic.code,
            diagnostic.target,
            diagnostic
                .source_layer
                .as_deref()
                .map(|layer| format!(" ({layer})"))
                .unwrap_or_default(),
            diagnostic.message
        );
    }
    println!(
        "Summary: {} changes, {} blockers, {} advisories",
        plan.report.summary.changes, plan.report.summary.blockers, plan.report.summary.advisories
    );
}

fn print_diffs(edits: &[PresetMigrationEdit]) {
    for edit in edits {
        let old = String::from_utf8_lossy(&edit.original);
        let new = edit
            .candidate
            .as_deref()
            .map(String::from_utf8_lossy)
            .unwrap_or_default();
        println!();
        println!(
            "{}",
            similar::TextDiff::from_lines(&old, &new)
                .unified_diff()
                .header(
                    &format!("a/{}", edit.logical_path),
                    &format!("b/{}", edit.logical_path)
                )
        );
    }
}

#[derive(Serialize)]
struct BackupManifest<'a> {
    schema_version: u32,
    files: Vec<BackupEntry<'a>>,
}

#[derive(Serialize)]
struct BackupEntry<'a> {
    logical_path: &'a str,
    source_layer: &'a str,
    original_sha256: String,
    mode: Option<u32>,
}

fn migration_source_observations(
    snapshot: &PresetSnapshot,
    edits: &[PresetMigrationEdit],
) -> BTreeMap<PathBuf, Vec<u8>> {
    let targets = edits
        .iter()
        .map(|edit| {
            edit.logical_path
                .split('/')
                .take(2)
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect::<BTreeSet<_>>();
    snapshot
        .source_files()
        .filter(|(logical, _)| {
            let target = logical.split('/').take(2).collect::<Vec<_>>().join("/");
            targets.contains(&target)
        })
        .filter_map(|(_, file)| {
            file.origin
                .physical_path
                .as_ref()
                .map(|path| (path.clone(), file.bytes.clone()))
        })
        .collect()
}

async fn ensure_sources_unchanged(sources: &BTreeMap<PathBuf, Vec<u8>>) -> Result<()> {
    for (path, original) in sources {
        let current = tokio::fs::read(path)
            .await
            .with_context(|| format!("reading {} after review", path.display()))?;
        if current != *original {
            bail!("Preset source changed after review");
        }
    }
    Ok(())
}

async fn create_backup_set(
    shine_dir: &Path,
    edits: &[PresetMigrationEdit],
    sources: &BTreeMap<PathBuf, Vec<u8>>,
) -> Result<PathBuf> {
    ensure_sources_unchanged(sources).await?;
    let root = shine_dir
        .join("preset-migration-backups")
        .join(uuid::Uuid::new_v4().to_string());
    tokio::fs::create_dir_all(&root).await?;
    #[cfg(unix)]
    tokio::fs::set_permissions(&root, std::os::unix::fs::PermissionsExt::from_mode(0o700)).await?;
    let mut entries = Vec::new();
    for edit in edits {
        let backup = root.join(&edit.logical_path);
        persist::atomic_write_private(&backup, &edit.original).await?;
        entries.push(BackupEntry {
            logical_path: &edit.logical_path,
            source_layer: &edit.source_layer,
            original_sha256: sha256(&edit.original),
            mode: file_mode(&edit.physical_path).await?,
        });
    }
    let manifest = toml::to_string_pretty(&BackupManifest {
        schema_version: 1,
        files: entries,
    })?;
    persist::atomic_write_private(&root.join("manifest.toml"), manifest.as_bytes()).await?;
    Ok(root)
}

async fn apply_edits(
    edits: &[PresetMigrationEdit],
    sources: &BTreeMap<PathBuf, Vec<u8>>,
) -> Result<()> {
    ensure_sources_unchanged(sources).await?;
    for edit in edits {
        let current = tokio::fs::read(&edit.physical_path)
            .await
            .with_context(|| {
                format!("reading {} before migration", edit.physical_path.display())
            })?;
        if current != edit.original {
            bail!("Preset source changed after review: {}", edit.logical_path);
        }
        let permissions = tokio::fs::metadata(&edit.physical_path)
            .await?
            .permissions();
        match &edit.candidate {
            Some(candidate) => {
                persist::atomic_write(&edit.physical_path, candidate).await?;
                tokio::fs::set_permissions(&edit.physical_path, permissions).await?;
            }
            None => tokio::fs::remove_file(&edit.physical_path).await?,
        }
    }
    Ok(())
}

#[cfg(unix)]
async fn file_mode(path: &Path) -> Result<Option<u32>> {
    use std::os::unix::fs::PermissionsExt;
    Ok(Some(tokio::fs::metadata(path).await?.permissions().mode()))
}

#[cfg(not(unix))]
async fn file_mode(_path: &Path) -> Result<Option<u32>> {
    Ok(None)
}

fn legacy_metadata_hashes() -> BTreeMap<String, BTreeSet<String>> {
    // Unique shine.toml SHA-256 values shipped by every v1.0.0 through v1.8.0
    // release tag (including patch releases). Sys fingerprints remain evidence
    // only: Core deliberately refuses to turn a v1 dispatcher into v2 items.
    const ENTRIES: &[(&str, &str)] = &[
        (
            "app/JetBrains/shine.toml",
            "0d6545cdfa392b6d4742bcddc288b4faf7382df3a23395a69300508e7c09e8f1",
        ),
        (
            "app/archey4/shine.toml",
            "97355899e41859f3c63323c5f4419a8e2c5f6723673765b7bf7935d3f8e2b6c2",
        ),
        (
            "app/clash-verge/shine.toml",
            "1a90f41ca438622b212de8fd5732a89c302f19216b53ac9c633bf8bef43ce97f",
        ),
        (
            "app/clash-verge/shine.toml",
            "842c227f8e53e3ddd6113402b53f293b495d4833d2530fa79ef0f8337a04f5bf",
        ),
        (
            "app/docker-desktop/shine.toml",
            "9ed5a6b310a152639bbacc3cc157d2e408fe2c4187d9d8f92b346941f6b6314b",
        ),
        (
            "app/docker-engine/shine.toml",
            "658acb03b9dc488f214daa0b2b13dd2516028047de8952e30557de26cbbc63a0",
        ),
        (
            "app/fastfetch/shine.toml",
            "2dd7d716ddaaf13f07649a24f3ce580d23275649a8744d7359f66edf3ada3731",
        ),
        (
            "app/ghostty/shine.toml",
            "7c8201f5059a7bb3e81382cf5436a1da14906656a62aaa243cdc970b149f9ef4",
        ),
        (
            "app/surge/shine.toml",
            "5df30183647d35bb9359c9a09ad7efe94fe8b5c212b0d486ded7c6288b349034",
        ),
        (
            "app/surge/shine.toml",
            "ac5db93291294515aba2c8f457af1790d1520732e5a7c337fcbbd71a60144a23",
        ),
        (
            "app/vim/shine.toml",
            "75d27a891409dc484bb833ca8b1c192461ee993ff7ec49ea71e0d5c05f5735a1",
        ),
        (
            "shell/agent/shine.toml",
            "e8eb84b91e3dfd958a81cc36d2425a0847257f9e940932643edfff5e80d53fd9",
        ),
        (
            "shell/image-tools/shine.toml",
            "5d18cec5a585d58f897ad8aa74073c805f95c2af84db714ca09c54d02ce92a28",
        ),
        (
            "shell/proxy/shine.toml",
            "9e1dbfca07fab117c067ea8a8244cc7a8d64684e204fb5ecf8690f17fe743d6e",
        ),
        (
            "shell/utils/shine.toml",
            "b670ab168bc4eaf4cc75b20b7637a6c109bd0ea6db309464d6b0cde81497997d",
        ),
        (
            "sys/macos/shine.toml",
            "18cf178c1f3e8b6d456731356c62c6db3af005a388b647f10e38513c5d25d49d",
        ),
        (
            "sys/macos/shine.toml",
            "31d2809917dfb40d6c5642acbca4d4fb8a5d0514eb3b4208bc840becdedf735e",
        ),
        (
            "sys/ubuntu/shine.toml",
            "72788f06f29e7e554be80f25bfac1239b8759f89e504285f3045db12b6cc9a96",
        ),
        (
            "sys/ubuntu/shine.toml",
            "957133cacf805a4041e91bf08f86ff727415fd5165ca52a34c4da8c2044e9d57",
        ),
        (
            "sys/ubuntu/shine.toml",
            "f52d5ae3be3506269b1e8b0d34c5daab0fd5ea3dc87fa22b0500c05bbc4fd4b5",
        ),
        (
            "sys/ubuntu/shine.toml",
            "fb0ec4efa47a16618d63eab975066eb19a0157a682399505d0bae9f67e559769",
        ),
        (
            "sys/windows/shine.toml",
            "707ad4c963722a983c853705d1bd7ee4d0c72f00c3e8cb5f3bf0490e7d346708",
        ),
        (
            "sys/windows/shine.toml",
            "916f8f87ac7f36c37d2970417176e3a49ac07b4fa8dee40a3ed409e94feefce2",
        ),
        (
            "sys/windows/shine.toml",
            "edb1ec46dd84cb5ca4c799164b6f4e21591a1e988876bf1d3fdbc8fc20870ad8",
        ),
    ];
    let mut map = BTreeMap::<String, BTreeSet<String>>::new();
    for (path, hash) in ENTRIES {
        map.entry((*path).to_string())
            .or_default()
            .insert((*hash).to_string());
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{Cli, Commands, PresetCommands};
    use crate::test_support::{env_lock, make_temp_dir};
    use clap::Parser;

    #[test]
    fn migrate_cli_parses_review_flags_and_rejects_yes_with_dry_run() {
        let cli = Cli::try_parse_from([
            "shine",
            "preset",
            "migrate",
            "presets/app/demo",
            "--dry-run",
            "--format",
            "json",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Preset {
                command: PresetCommands::Migrate {
                    path: Some(_),
                    dry_run: true,
                    yes: false,
                    format: PresetReportFormat::Json,
                }
            }
        ));
        assert!(Cli::try_parse_from(["shine", "preset", "migrate", "--dry-run", "--yes"]).is_err());
    }

    #[test]
    fn managed_overlay_candidates_are_diagnostic_only() {
        let root = Path::new("/managed-overlay");
        let snapshot = PresetSnapshot::builder(shine_core::runtime::PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"metadata_schema_version = 2\ndest = '~/.demo'\n[permissions]\nschema_version = 1\n"
                    .to_vec(),
            )
            .overlay_root(root)
            .overlay_file(
                "app/demo/shine.toml",
                b"dest = '~/.demo'\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .overlay_file("app/demo/config.toml", Vec::new())
            .build();
        let mut plan = plan_preset_migration(&snapshot, "active", None, None);
        assert_eq!(plan.edits.len(), 1);

        mark_managed_overlay_read_only(&mut plan, root);

        assert!(plan.edits.is_empty());
        assert!(plan.report.diagnostics.iter().any(|item| {
            item.code == "managed_overlay_read_only"
                && item.source_layer.as_deref() == Some("overlay")
        }));
    }

    #[test]
    fn targeted_sys_compatibility_selects_only_the_item_category() {
        let snapshot = PresetSnapshot::builder(shine_core::runtime::PresetSourceKind::External)
            .file(
                "sys/macos/shine.toml",
                b"version = 2\n[[items]]\nid = 'one'\n".to_vec(),
            )
            .file(
                "sys/ubuntu/shine.toml",
                b"version = 2\n[[items]]\nid = 'two'\n".to_vec(),
            )
            .build();

        assert_eq!(
            sys_categories_for_item(&snapshot, "two"),
            BTreeSet::from(["sys/ubuntu".to_string()])
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // env_lock serializes process-global discovery state.
    async fn dry_run_is_read_only_and_apply_creates_private_backup_state() {
        let _guard = env_lock();
        let root = make_temp_dir("shine-preset-migrate").await;
        let state = root.join("state");
        let presets = root.join("source");
        let category = presets.join("app/demo");
        tokio::fs::create_dir_all(&category).await.unwrap();
        let metadata = category.join("shine.toml");
        let original = b"dest = '~/.demo'\n[[files]]\nsource = 'config.toml'\n";
        tokio::fs::write(&metadata, original).await.unwrap();
        tokio::fs::write(category.join("config.toml"), b"payload")
            .await
            .unwrap();

        let previous_config = std::env::var_os("SHINE_CONFIG_DIR");
        let previous_presets = std::env::var_os("SHINE_PRESETS");
        // SAFETY: env_lock serializes process-global environment mutation.
        unsafe {
            std::env::set_var("SHINE_CONFIG_DIR", &state);
            std::env::remove_var("SHINE_PRESETS");
        }

        let dry = handle_migrate(Some(&presets), true, false, PresetReportFormat::Text)
            .await
            .unwrap();
        assert!(dry);
        assert_eq!(tokio::fs::read(&metadata).await.unwrap(), original);
        assert!(!state.exists());

        let json_without_yes =
            handle_migrate(Some(&presets), false, false, PresetReportFormat::Json)
                .await
                .unwrap_err();
        assert!(json_without_yes.to_string().contains("--dry-run or --yes"));
        assert!(!state.exists());

        let non_interactive =
            handle_migrate(Some(&presets), false, false, PresetReportFormat::Text)
                .await
                .unwrap_err();
        assert!(non_interactive.to_string().contains("explicit --yes"));
        assert_eq!(tokio::fs::read(&metadata).await.unwrap(), original);
        assert!(!state.exists());

        let applied = handle_migrate(Some(&presets), false, true, PresetReportFormat::Text)
            .await
            .unwrap();
        assert!(applied);
        let migrated = tokio::fs::read_to_string(&metadata).await.unwrap();
        assert!(migrated.contains("metadata_schema_version = 2"));
        assert!(migrated.contains("[permissions]"));
        let backups = state.join("preset-migration-backups");
        assert!(backups.is_dir());
        let mut sets = tokio::fs::read_dir(&backups).await.unwrap();
        let backup = sets.next_entry().await.unwrap().unwrap().path();
        let manifest = tokio::fs::read_to_string(backup.join("manifest.toml"))
            .await
            .unwrap();
        assert!(manifest.contains("source_layer = \"external\""));
        assert!(!manifest.contains("source_path"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                tokio::fs::metadata(&backup)
                    .await
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }

        // SAFETY: env_lock remains held while the previous environment is restored.
        unsafe {
            match previous_config {
                Some(value) => std::env::set_var("SHINE_CONFIG_DIR", value),
                None => std::env::remove_var("SHINE_CONFIG_DIR"),
            }
            match previous_presets {
                Some(value) => std::env::set_var("SHINE_PRESETS", value),
                None => std::env::remove_var("SHINE_PRESETS"),
            }
        }
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn backup_refuses_a_source_changed_after_review() {
        let root = make_temp_dir("shine-preset-source-change").await;
        let source = root.join("shine.toml");
        tokio::fs::write(&source, b"original").await.unwrap();
        let edit = PresetMigrationEdit {
            logical_path: "app/demo/shine.toml".to_string(),
            physical_path: source.clone(),
            source_layer: "external".to_string(),
            operations: vec!["test".to_string()],
            original: b"original".to_vec(),
            candidate: Some(b"candidate".to_vec()),
        };
        tokio::fs::write(&source, b"changed").await.unwrap();

        let sources = BTreeMap::from([(source.clone(), b"original".to_vec())]);
        let error = create_backup_set(&root.join("state"), &[edit], &sources)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("changed after review"));
        assert!(!root.join("state").exists());

        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[test]
    fn migration_observations_include_shadowed_base_files() {
        let snapshot = PresetSnapshot::builder(shine_core::runtime::PresetSourceKind::External)
            .base_root("/base")
            .file("app/demo/shine.toml", b"base metadata".to_vec())
            .file("app/demo/config.toml", b"base payload".to_vec())
            .overlay_root("/overlay")
            .overlay_file("app/demo/shine.toml", b"overlay metadata".to_vec())
            .build();
        let edits = vec![PresetMigrationEdit {
            logical_path: "app/demo/shine.toml".to_string(),
            physical_path: PathBuf::from("/overlay/app/demo/shine.toml"),
            source_layer: "overlay".to_string(),
            operations: Vec::new(),
            original: b"overlay metadata".to_vec(),
            candidate: None,
        }];

        let observations = migration_source_observations(&snapshot, &edits);

        assert_eq!(observations.len(), 3);
        assert_eq!(
            observations.get(&PathBuf::from("/base/app/demo/shine.toml")),
            Some(&b"base metadata".to_vec())
        );
        assert_eq!(
            observations.get(&PathBuf::from("/overlay/app/demo/shine.toml")),
            Some(&b"overlay metadata".to_vec())
        );
    }

    #[test]
    fn report_drops_rejected_edits() {
        let snapshot = PresetSnapshot::builder(shine_core::runtime::PresetSourceKind::External)
            .base_root("/presets")
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.demo'\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", Vec::new())
            .build();
        let mut plan = plan_preset_migration(&snapshot, "test", None, None);

        plan.edits.clear();
        sync_report_with_edits(&mut plan);

        assert!(plan.report.files.is_empty());
        assert_eq!(plan.report.summary.changes, 0);
        assert_eq!(plan.report.status, PresetMigrationStatusV1::Current);
    }
}
