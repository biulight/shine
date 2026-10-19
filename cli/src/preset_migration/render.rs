//! CLI-only compatibility, remediation and diff rendering.
use super::*;
use crate::preset_report::count_phrase;

pub fn print_compatibility(plan: &PresetMigrationPlan) {
    print!("{}", compatibility_text(plan));
}

pub(super) fn compatibility_text(plan: &PresetMigrationPlan) -> String {
    if plan.edits.is_empty() && plan.report.diagnostics.is_empty() {
        return String::new();
    }
    let mut output = format!("{}\n", crate::colors::bold("Preset compatibility"));
    for file in &plan.report.files {
        let _ = writeln!(output);
        let _ = writeln!(
            output,
            "  {} migrate {} ({})",
            crate::colors::symbol("~"),
            file.target,
            file.source_layer
        );
    }
    for diagnostic in &plan.report.diagnostics {
        let marker = if diagnostic.severity == PresetMigrationSeverityV1::Blocker {
            crate::colors::symbol("✗")
        } else {
            crate::colors::yellow("!")
        };
        let _ = writeln!(output);
        let _ = writeln!(
            output,
            "  {marker} {}{}",
            diagnostic.target,
            diagnostic
                .source_layer
                .as_deref()
                .map(|layer| format!(" ({layer})"))
                .unwrap_or_default()
        );
        crate::preset_report::write_wrapped(&mut output, "    ", &diagnostic.message);
        let _ = writeln!(
            output,
            "    {} {}",
            crate::colors::dim("code:"),
            diagnostic.code
        );
    }
    let _ = writeln!(output);
    output
}

pub fn compatibility_required(plan: &PresetMigrationPlan) -> bool {
    !plan.edits.is_empty() || plan.report.summary.blockers > 0
}

pub fn compatibility_failure_message(plan: &PresetMigrationPlan) -> String {
    let blockers = plan.report.summary.blockers;
    let changes = plan.edits.len();
    let reason = match (blockers, changes) {
        (0, changes) => count_phrase(changes, "automatic change", "automatic changes"),
        (blockers, 0) => count_phrase(blockers, "blocker", "blockers"),
        (blockers, changes) => format!(
            "{} and {}",
            count_phrase(blockers, "blocker", "blockers"),
            count_phrase(changes, "automatic change", "automatic changes")
        ),
    };
    format!(
        "Preset compatibility requires attention ({reason})\n\nNext:\n  shine preset migrate --dry-run"
    )
}

pub(super) fn print_text(
    plan: &PresetMigrationPlan,
    snapshot: &PresetSnapshot,
    managed_overlay: Option<&Path>,
    active_source: bool,
) {
    print!(
        "{}",
        migration_text(plan, snapshot, managed_overlay, active_source)
    );
}

pub(super) fn migration_text(
    plan: &PresetMigrationPlan,
    snapshot: &PresetSnapshot,
    managed_overlay: Option<&Path>,
    active_source: bool,
) -> String {
    let mut output = String::new();
    let status = match plan.report.status {
        PresetMigrationStatusV1::Current => "current",
        PresetMigrationStatusV1::Pending => "changes pending",
        PresetMigrationStatusV1::Blocked => "manual action required",
        PresetMigrationStatusV1::Applied => "applied",
        PresetMigrationStatusV1::PartiallyApplied => "partially applied",
    };
    let styled_status = match plan.report.status {
        PresetMigrationStatusV1::Current | PresetMigrationStatusV1::Applied => {
            crate::colors::green(status)
        }
        PresetMigrationStatusV1::Pending | PresetMigrationStatusV1::PartiallyApplied => {
            crate::colors::yellow(status)
        }
        PresetMigrationStatusV1::Blocked => crate::colors::red(status),
    };
    let _ = writeln!(
        output,
        "{} {styled_status}",
        crate::colors::bold("Preset migration:")
    );

    let mut groups = BTreeMap::<String, Vec<&PresetMigrationDiagnosticV1>>::new();
    for diagnostic in &plan.report.diagnostics {
        let key = metadata_logical_path(snapshot, &diagnostic.target)
            .unwrap_or_else(|| diagnostic_category(&diagnostic.target));
        groups.entry(key).or_default().push(diagnostic);
    }
    let mut needs_preset_pull = false;
    for (logical, diagnostics) in groups {
        let layer = diagnostics
            .iter()
            .find_map(|diagnostic| diagnostic.source_layer.as_deref())
            .map(|value| format!(" ({value})"))
            .unwrap_or_default();
        let _ = writeln!(output);
        let _ = writeln!(
            output,
            "  {}{layer}",
            logical.trim_end_matches("/shine.toml")
        );
        for diagnostic in &diagnostics {
            let symbol = if diagnostic.severity == PresetMigrationSeverityV1::Blocker {
                crate::colors::symbol("✗")
            } else {
                crate::colors::symbol("!")
            };
            let _ = writeln!(output, "    {symbol} {}", diagnostic.target);
            crate::preset_report::write_wrapped(&mut output, "      ", &diagnostic.message);
            let _ = writeln!(
                output,
                "      {} {}",
                crate::colors::dim("code:"),
                diagnostic.code
            );
        }
        needs_preset_pull |= render_remediation(
            &mut output,
            snapshot,
            &logical,
            &diagnostics,
            managed_overlay,
            active_source,
        );
    }

    if needs_preset_pull {
        let _ = writeln!(output);
        let _ = writeln!(output, "  {}", crate::colors::cyan("Next:"));
        let _ = writeln!(output, "    Commit the upstream changes, then run:");
        let _ = writeln!(output, "      shine preset pull");
    }

    if !plan.report.diagnostics.is_empty() {
        let _ = writeln!(output);
    }
    let changes = count_phrase(
        plan.report.summary.changes,
        "automatic change",
        "automatic changes",
    );
    let blockers = count_phrase(plan.report.summary.blockers, "blocker", "blockers");
    let advisories = count_phrase(plan.report.summary.advisories, "advisory", "advisories");
    let _ = writeln!(
        output,
        "{} {} · {} · {}",
        crate::colors::bold("Summary:"),
        if plan.report.summary.blockers > 0 {
            crate::colors::red(&blockers)
        } else {
            crate::colors::dim(&blockers)
        },
        if plan.report.summary.changes > 0 {
            crate::colors::green(&changes)
        } else {
            crate::colors::dim(&changes)
        },
        if plan.report.summary.advisories > 0 {
            crate::colors::yellow(&advisories)
        } else {
            crate::colors::dim(&advisories)
        }
    );
    output
}

pub(super) fn render_remediation(
    output: &mut String,
    snapshot: &PresetSnapshot,
    logical: &str,
    diagnostics: &[&PresetMigrationDiagnosticV1],
    managed_overlay: Option<&Path>,
    active_source: bool,
) -> bool {
    let manual_permissions = diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "manual_permission_review_required");
    let managed_read_only = diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "managed_overlay_read_only");
    let manifest = snapshot
        .origin(logical)
        .and_then(|origin| origin.physical_path.as_deref());
    let manifest_is_managed =
        manifest.is_some_and(|path| managed_overlay.is_some_and(|root| path.starts_with(root)));

    if managed_read_only && !manual_permissions {
        let _ = writeln!(output);
        let _ = writeln!(output, "    {}", crate::colors::yellow("Fix:"));
        let _ = writeln!(
            output,
            "      Review and update {logical} in the upstream checkout."
        );
        let _ = writeln!(output, "      The managed overlay mirror is read-only.");
    }

    if manual_permissions {
        if managed_read_only || manifest_is_managed {
            let _ = writeln!(output);
            let _ = writeln!(output, "    {}", crate::colors::yellow("Fix:"));
            let _ = writeln!(
                output,
                "      Review and update {logical} in the upstream checkout."
            );
            let _ = writeln!(output, "      The managed overlay mirror is read-only.");
        } else if let Some(manifest) = manifest {
            let quoted = quote_command_arg(manifest, RuntimePlatform::current());
            let _ = writeln!(output);
            let _ = writeln!(output, "    {}", crate::colors::yellow("Fix:"));
            let _ = writeln!(output, "      Edit: {}", manifest.display());
            let _ = writeln!(output, "      Verify:");
            let _ = writeln!(output, "        shine preset validate {quoted}");
            let _ = writeln!(
                output,
                "        shine preset plan {quoted} --platform {}",
                RuntimePlatform::current().as_str()
            );
        } else {
            let _ = writeln!(output);
            let _ = writeln!(output, "    {}", crate::colors::yellow("Fix:"));
            let _ = writeln!(
                output,
                "      Add the target-local permission declaration in {logical}, then validate and plan that manifest."
            );
        }
    }

    for diagnostic in diagnostics {
        if diagnostic.code == "recursive_artifact_hook_removed"
            && let Some(category) = diagnostic.target.strip_prefix("app/")
        {
            let category = quote_command_arg(Path::new(category), RuntimePlatform::current());
            let _ = writeln!(
                output,
                "    Next: run `shine app artifact apply {category}` after relevant changes."
            );
        }
    }

    for target in trust_review_targets(snapshot, logical, diagnostics) {
        let timing = if active_source {
            "After validation"
        } else {
            "After this source becomes active"
        };
        let _ = writeln!(output, "    {timing}, review the external executable code:");
        let _ = writeln!(output, "      `shine trust inspect {target}`");
        let _ = writeln!(
            output,
            "      If the inspection reports a requirement and you accept its scope, run `shine trust grant {target}`."
        );
    }

    managed_read_only || (manual_permissions && manifest_is_managed)
}

pub(super) fn metadata_logical_path(snapshot: &PresetSnapshot, target: &str) -> Option<String> {
    let mut parts = target.split('/');
    let kind = parts.next()?;
    let name = parts.next()?;
    let direct = format!("{kind}/{name}/shine.toml");
    if snapshot.get(&direct).is_some() {
        return Some(direct);
    }
    if kind != "sys" {
        return None;
    }
    snapshot.files().iter().find_map(|(logical, bytes)| {
        if !logical.starts_with("sys/") || !logical.ends_with("/shine.toml") {
            return None;
        }
        let value = toml::from_slice::<toml::Value>(bytes).ok()?;
        value
            .get("items")
            .and_then(toml::Value::as_array)
            .is_some_and(|items| {
                items
                    .iter()
                    .any(|item| item.get("id").and_then(toml::Value::as_str) == Some(name))
            })
            .then(|| logical.clone())
    })
}

pub(super) fn diagnostic_category(target: &str) -> String {
    target.split('/').take(2).collect::<Vec<_>>().join("/")
}

pub(super) fn trust_review_targets(
    snapshot: &PresetSnapshot,
    logical: &str,
    diagnostics: &[&PresetMigrationDiagnosticV1],
) -> BTreeSet<String> {
    diagnostics
        .iter()
        .filter_map(|diagnostic| {
            if diagnostic.source_layer.as_deref() == Some("embedded") {
                return None;
            }
            match diagnostic.target.split_once('/') {
                Some(("app", _))
                    if matches!(
                        diagnostic.code.as_str(),
                        "manual_permission_review_required" | "external_code_trust_review_required"
                    ) =>
                {
                    Some(diagnostic.target.clone())
                }
                Some(("sys", item))
                    if diagnostic.code == "manual_permission_review_required"
                        && sys_item_has_executable_code(snapshot, logical, item) =>
                {
                    Some(diagnostic.target.clone())
                }
                _ => None,
            }
        })
        .collect()
}

pub(super) fn sys_item_has_executable_code(
    snapshot: &PresetSnapshot,
    logical: &str,
    item_id: &str,
) -> bool {
    let Some(value) = snapshot
        .get(logical)
        .and_then(|bytes| toml::from_slice::<toml::Value>(bytes).ok())
    else {
        return false;
    };
    let item_code = value
        .get("items")
        .and_then(toml::Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|item| item.get("id").and_then(toml::Value::as_str) == Some(item_id))
        })
        .is_some_and(|item| {
            let script = item
                .get("install")
                .and_then(|install| install.get("kind"))
                .and_then(toml::Value::as_str)
                == Some("script");
            let shell_code = item
                .get("shell")
                .and_then(toml::Value::as_array)
                .into_iter()
                .flatten()
                .any(|integration| {
                    ["eval", "source", "fragment"]
                        .iter()
                        .any(|key| integration.get(*key).is_some())
                });
            script || shell_code
        });
    if item_code {
        return true;
    }
    let category_prefix = logical.trim_end_matches("shine.toml");
    snapshot.files().keys().any(|path| {
        path.starts_with(category_prefix)
            && path.contains("/profile/base.")
            && snapshot
                .origin(path)
                .is_some_and(|origin| origin.source_kind != PresetSourceKind::Embedded)
    })
}

pub(super) fn quote_command_arg(path: &Path, platform: RuntimePlatform) -> String {
    let value = path.display().to_string();
    if platform == RuntimePlatform::Windows {
        format!("'{}'", value.replace('\'', "''"))
    } else {
        crate::shell_quote::quote_if_needed(&value)
    }
}

pub(super) fn print_diffs(edits: &[PresetMigrationEdit]) {
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
