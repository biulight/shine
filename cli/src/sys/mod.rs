use anyhow::{Context, Result, bail};
use console::{Style, style};
use dialoguer::{MultiSelect, theme::ColorfulTheme};
use serde::Deserialize;
use similar::{DiffTag, TextDiff};
use std::collections::{BTreeMap, BTreeSet};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::colors;
use crate::config::Config;
use crate::shells::ShellType;

#[derive(Clone, Debug, Default, Deserialize)]
struct SysManifest {
    #[serde(default)]
    description: String,
    default_profile: Option<String>,
    #[serde(default)]
    items: Vec<SysItem>,
    #[serde(default)]
    profiles: BTreeMap<String, SysProfile>,
}

#[derive(Clone, Debug, Deserialize)]
struct SysItem {
    id: String,
    label: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    default: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct SysProfile {
    #[serde(default)]
    items: Vec<String>,
}

#[derive(Clone, Debug)]
struct LoadedSysPreset {
    manifest: SysManifest,
    script_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SysInitCommand {
    program: &'static str,
    fixed_args: Vec<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SelectionSource {
    Profile(String),
    DefaultProfile(String),
    Interactive,
    NoItems,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedSelection {
    item_ids: Vec<String>,
    source: SelectionSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum SysItemStatus {
    Installed,
    AlreadyInstalled,
    Skipped,
    Updated,
    NeedsAction,
    Completed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SysItemOutcome {
    item_id: String,
    label: String,
    status: SysItemStatus,
    detail: String,
    logs: Vec<String>,
}

const SYS_STATUS_PREFIX: &str = "SHINE_SYS_STATUS\t";
impl SelectionSource {
    fn describe(&self) -> String {
        match self {
            Self::Profile(name) => format!("profile `{name}`"),
            Self::DefaultProfile(name) => format!("default profile `{name}`"),
            Self::Interactive => "interactive selection".to_string(),
            Self::NoItems => "no selectable items".to_string(),
        }
    }
}

fn sys_init_theme() -> ColorfulTheme {
    ColorfulTheme {
        prompt_prefix: style(">".to_string()).for_stderr().cyan().bold(),
        prompt_suffix: style("".to_string()).for_stderr(),
        success_prefix: style("✓".to_string()).for_stderr().green(),
        success_suffix: style("".to_string()).for_stderr(),
        active_item_prefix: style("›".to_string()).for_stderr().cyan().bold(),
        inactive_item_prefix: style(" ".to_string()).for_stderr(),
        checked_item_prefix: style("[x]".to_string()).for_stderr().green(),
        unchecked_item_prefix: style("[ ]".to_string()).for_stderr().black().bright(),
        prompt_style: Style::new().for_stderr().bold(),
        active_item_style: Style::new().for_stderr().cyan(),
        inactive_item_style: Style::new().for_stderr(),
        values_style: Style::new().for_stderr().cyan(),
        hint_style: Style::new().for_stderr().black().bright(),
        ..ColorfulTheme::default()
    }
}

/// Detect the current OS identifier using `std::env::consts::OS` and, on Linux,
/// the `ID=` field from `/etc/os-release`.
pub(crate) async fn detect_os_id() -> Result<String> {
    let os_release = tokio::fs::read_to_string("/etc/os-release").await.ok();
    detect_os_id_from(std::env::consts::OS, os_release.as_deref())
}

fn detect_os_id_from(os: &str, os_release: Option<&str>) -> Result<String> {
    match os {
        "macos" => Ok("macos".to_string()),
        "windows" => Ok("windows".to_string()),
        "linux" => {
            if let Some(content) = os_release {
                for line in content.lines() {
                    if let Some(id) = line.strip_prefix("ID=") {
                        return Ok(id.trim_matches('"').to_lowercase());
                    }
                }
            }
            bail!(
                "Could not detect Linux distribution. \
                 Expected ID= in /etc/os-release. Supported: ubuntu"
            )
        }
        other => bail!(
            "Unsupported platform '{}'. Supported targets: ubuntu (Linux), macos, windows",
            other
        ),
    }
}

pub(crate) async fn handle_list(config: &Config) -> Result<()> {
    crate::config::print_presets_note(config);

    let current_os = detect_os_id().await.ok();

    let entries = if config.is_external_presets {
        list_fs_sys_entries(config.presets_dir()).await
    } else {
        list_embedded_sys_entries()
    };

    if entries.is_empty() {
        println!("{}", colors::dim("No system init presets found."));
        return Ok(());
    }

    println!("{}\n", colors::bold("System Init Presets"));

    for (os_id, description) in &entries {
        let is_current = current_os.as_deref() == Some(os_id.as_str());
        let marker = if is_current { "▶" } else { " " };
        let label = if is_current {
            colors::bold(os_id)
        } else {
            os_id.clone()
        };
        println!("  {marker} {label}");
        if !description.is_empty() {
            println!("      {}", colors::dim(description));
        }
        println!();
    }

    println!(
        "{}",
        colors::dim("Run `shine sys init` to initialize the current system.")
    );
    Ok(())
}

pub(crate) async fn handle_init(
    config: &Config,
    preset: Option<&str>,
    dry_run: bool,
    force_profile: bool,
) -> Result<()> {
    let os_id = detect_os_id().await?;
    handle_init_for_os(config, &os_id, preset, dry_run, force_profile).await
}

async fn handle_init_for_os(
    config: &Config,
    os_id: &str,
    preset: Option<&str>,
    dry_run: bool,
    force_profile: bool,
) -> Result<()> {
    crate::config::print_presets_note(config);

    let loaded = load_sys_preset(config, os_id).await?;
    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let selection = resolve_selection(&loaded.manifest, preset, interactive)?;
    let sys_shell: &'static str = config.shell_type.into();

    if dry_run {
        print_dry_run(os_id, &loaded, &selection, sys_shell).await?;
        return Ok(());
    }

    if selection.item_ids.is_empty() {
        println!(
            "{}",
            colors::dim(&format!(
                "No sys init items selected for {} ({}).",
                os_id,
                selection.source.describe()
            ))
        );
        return Ok(());
    }

    let command = sys_init_command(os_id);
    let script_dir = loaded
        .script_path
        .parent()
        .with_context(|| format!("invalid script path: {}", loaded.script_path.display()))?;

    print_run_header(os_id, sys_shell, &selection);

    let item_labels = manifest_item_labels(&loaded.manifest);
    let label_width = selection
        .item_ids
        .iter()
        .filter_map(|item_id| item_labels.get(item_id.as_str()))
        .map(String::len)
        .chain(std::iter::once("profile".len()))
        .max()
        .unwrap_or(14)
        .max(14);
    let mut outcomes = Vec::new();
    for item_id in &selection.item_ids {
        let label = item_labels
            .get(item_id.as_str())
            .cloned()
            .unwrap_or_else(|| item_id.clone());
        let outcome = run_sys_item(
            &command,
            script_dir,
            &loaded.script_path,
            sys_shell,
            item_id,
            &label,
        )
        .await?;
        print_item_outcome(&outcome, label_width);
        let failed = outcome.status == SysItemStatus::Failed;
        outcomes.push(outcome);
        if failed {
            break;
        }
    }

    if outcomes
        .iter()
        .any(|outcome| outcome.status != SysItemStatus::Failed)
    {
        let profile =
            install_sys_profile_loader(config, os_id, script_dir, sys_shell, force_profile).await?;
        print_item_outcome(&profile, label_width);
        outcomes.push(profile);
    }

    println!();
    print_sys_summary(&outcomes);

    if outcomes
        .iter()
        .any(|outcome| outcome.status == SysItemStatus::Failed)
    {
        bail!("sys init failed");
    }

    Ok(())
}

async fn print_dry_run(
    os_id: &str,
    loaded: &LoadedSysPreset,
    selection: &ResolvedSelection,
    sys_shell: &str,
) -> Result<()> {
    println!("{}", colors::dim("[dry-run] System init preview"));
    println!("  OS: {os_id}");
    println!("  Shell: {sys_shell}");
    println!("  Selection: {}", selection.source.describe());
    println!(
        "  Items: {}",
        if selection.item_ids.is_empty() {
            "(none)".to_string()
        } else {
            selection.item_ids.join(", ")
        }
    );
    println!("  Script: {}", loaded.script_path.display());
    let command = sys_init_command(os_id);
    println!("  Commands:");
    for item_id in &selection.item_ids {
        println!(
            "    {}",
            format_command_preview(&command, &loaded.script_path, std::slice::from_ref(item_id))
        );
    }
    if !selection.item_ids.is_empty() {
        println!("    shine internal sys profile update");
    }
    println!();
    let content = tokio::fs::read_to_string(&loaded.script_path)
        .await
        .with_context(|| format!("reading {}", loaded.script_path.display()))?;
    println!("{}", colors::dim("--- script content ---"));
    print!("{content}");
    Ok(())
}

fn sys_init_command(os_id: &str) -> SysInitCommand {
    match os_id {
        "windows" => SysInitCommand {
            program: "powershell.exe",
            fixed_args: vec!["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"],
        },
        "macos" => SysInitCommand {
            program: "zsh",
            fixed_args: Vec::new(),
        },
        _ => SysInitCommand {
            program: "bash",
            fixed_args: Vec::new(),
        },
    }
}

fn format_command_preview(
    command: &SysInitCommand,
    script_path: &Path,
    item_ids: &[String],
) -> String {
    let script = script_path.display().to_string();
    std::iter::once(command.program)
        .chain(command.fixed_args.iter().copied())
        .chain(std::iter::once(script.as_str()))
        .chain(item_ids.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

fn manifest_item_labels(manifest: &SysManifest) -> BTreeMap<&str, String> {
    manifest
        .items
        .iter()
        .map(|item| (item.id.as_str(), item.label.clone()))
        .collect()
}

fn print_run_header(os_id: &str, sys_shell: &str, selection: &ResolvedSelection) {
    println!("{}", colors::bold("System Init"));
    println!("  OS: {os_id}");
    println!("  Shell: {sys_shell}");
    println!("  Selection: {}", selection.source.describe());
    println!("  Items: {} selected", selection.item_ids.len());
    println!("  {}", colors::dim(&format_item_ids(&selection.item_ids)));
    println!();
}

async fn run_sys_item(
    command: &SysInitCommand,
    script_dir: &Path,
    script_path: &Path,
    sys_shell: &str,
    item_id: &str,
    label: &str,
) -> Result<SysItemOutcome> {
    let output = tokio::process::Command::new(command.program)
        .current_dir(script_dir)
        .env("SHINE_SYS_PRESET_ROOT", script_dir)
        .env("SHINE_SYS_SHELL", sys_shell)
        .args(&command.fixed_args)
        .arg(script_path)
        .arg(item_id)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .with_context(|| format!("failed to execute {}", script_path.display()))?;

    Ok(parse_sys_item_output(
        item_id,
        label,
        output.status.success(),
        &String::from_utf8_lossy(&output.stdout),
        &String::from_utf8_lossy(&output.stderr),
    ))
}

async fn install_sys_profile_loader(
    config: &Config,
    os_id: &str,
    script_dir: &Path,
    sys_shell: &str,
    force_profile: bool,
) -> Result<SysItemOutcome> {
    let update = install_sys_profile_files(config, os_id, script_dir, force_profile).await?;
    let shell_update = update_sys_shell_profiles(config, os_id, sys_shell).await?;
    let status = if update.needs_action {
        SysItemStatus::NeedsAction
    } else if update.updated || shell_update.updated {
        SysItemStatus::Updated
    } else {
        SysItemStatus::Skipped
    };
    let detail = if update.needs_action {
        update.detail
    } else if shell_update.unsupported_shell {
        format!("unsupported shell for sys profile: {sys_shell}")
    } else {
        shell_update.detail
    };

    Ok(SysItemOutcome {
        item_id: "profile".to_string(),
        label: "profile".to_string(),
        status,
        detail,
        logs: Vec::new(),
    })
}

#[derive(Debug)]
struct SysProfileFileUpdate {
    updated: bool,
    needs_action: bool,
    detail: String,
}

#[derive(Debug)]
struct SysShellProfileUpdate {
    updated: bool,
    unsupported_shell: bool,
    detail: String,
}

async fn install_sys_profile_files(
    config: &Config,
    os_id: &str,
    script_dir: &Path,
    force_profile: bool,
) -> Result<SysProfileFileUpdate> {
    let ext = if os_id == "windows" { "ps1" } else { "sh" };
    let stem = format!("{os_id}-sys");
    let template_path = script_dir.join(format!("profile.{ext}"));
    let template = tokio::fs::read(&template_path)
        .await
        .with_context(|| format!("reading {}", template_path.display()))?;

    let profile_dir = config.home_dir.join(".shine/profile");
    let active_path = profile_dir.join(format!("{stem}.{ext}"));
    let base_path = profile_dir.join(format!("{stem}.base.{ext}"));
    let new_path = profile_dir.join(format!("{stem}.new.{ext}"));
    let merge_path = profile_dir.join(format!("{stem}.merge.{ext}"));
    tokio::fs::create_dir_all(&profile_dir)
        .await
        .with_context(|| format!("creating {}", profile_dir.display()))?;

    let mut updated = false;

    if force_profile {
        let backup = if let Ok(active) = tokio::fs::read(&active_path).await
            && active != template
        {
            let backup_path = profile_backup_path(&active_path, ext);
            tokio::fs::copy(&active_path, &backup_path)
                .await
                .with_context(|| {
                    format!(
                        "backing up sys profile {} to {}",
                        active_path.display(),
                        backup_path.display()
                    )
                })?;
            Some(backup_path)
        } else {
            None
        };
        write_if_changed(&active_path, &template).await?;
        write_if_changed(&base_path, &template).await?;
        remove_if_exists(&new_path).await?;
        remove_if_exists(&merge_path).await?;
        let detail = backup
            .map(|path| {
                format!(
                    "{} replaced; previous profile backed up to {}",
                    active_path.display(),
                    path.display()
                )
            })
            .unwrap_or_else(|| format!("{} refreshed", active_path.display()));
        return Ok(SysProfileFileUpdate {
            updated: true,
            needs_action: false,
            detail,
        });
    }

    let active = match tokio::fs::read(&active_path).await {
        Ok(active) => active,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            tokio::fs::write(&active_path, &template)
                .await
                .with_context(|| format!("writing {}", active_path.display()))?;
            tokio::fs::write(&base_path, &template)
                .await
                .with_context(|| format!("writing {}", base_path.display()))?;
            remove_if_exists(&new_path).await?;
            remove_if_exists(&merge_path).await?;
            return Ok(SysProfileFileUpdate {
                updated: true,
                needs_action: false,
                detail: format!("{} created", active_path.display()),
            });
        }
        Err(err) => {
            return Err(err).with_context(|| format!("reading {}", active_path.display()));
        }
    };

    let base = match tokio::fs::read(&base_path).await {
        Ok(base) => base,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            if active == template {
                tokio::fs::write(&base_path, &template)
                    .await
                    .with_context(|| format!("writing {}", base_path.display()))?;
                remove_if_exists(&new_path).await?;
                remove_if_exists(&merge_path).await?;
                return Ok(SysProfileFileUpdate {
                    updated: true,
                    needs_action: false,
                    detail: format!("{} initialized", base_path.display()),
                });
            }
            if is_initial_user_profile_edit(&template, &active) {
                tokio::fs::write(&base_path, &template)
                    .await
                    .with_context(|| format!("writing {}", base_path.display()))?;
                remove_if_exists(&new_path).await?;
                remove_if_exists(&merge_path).await?;
                return Ok(SysProfileFileUpdate {
                    updated: true,
                    needs_action: false,
                    detail: format!("{} initialized with user edits", base_path.display()),
                });
            }

            tokio::fs::write(&new_path, &template)
                .await
                .with_context(|| format!("writing {}", new_path.display()))?;
            remove_if_exists(&merge_path).await?;
            return Ok(SysProfileFileUpdate {
                updated: true,
                needs_action: true,
                detail: format!(
                    "no merge base for {}; review new template at {} or rerun with --force-profile",
                    active_path.display(),
                    new_path.display()
                ),
            });
        }
        Err(err) => return Err(err).with_context(|| format!("reading {}", base_path.display())),
    };

    if active == template {
        updated |= write_if_changed(&base_path, &template).await?;
        remove_if_exists(&new_path).await?;
        remove_if_exists(&merge_path).await?;
        return Ok(SysProfileFileUpdate {
            updated,
            needs_action: false,
            detail: format!("{} already current", active_path.display()),
        });
    }

    if base == template {
        remove_if_exists(&new_path).await?;
        remove_if_exists(&merge_path).await?;
        return Ok(SysProfileFileUpdate {
            updated: false,
            needs_action: false,
            detail: format!("{} already configured", active_path.display()),
        });
    }

    if active == base {
        tokio::fs::write(&active_path, &template)
            .await
            .with_context(|| format!("writing {}", active_path.display()))?;
        tokio::fs::write(&base_path, &template)
            .await
            .with_context(|| format!("writing {}", base_path.display()))?;
        remove_if_exists(&new_path).await?;
        remove_if_exists(&merge_path).await?;
        return Ok(SysProfileFileUpdate {
            updated: true,
            needs_action: false,
            detail: format!("{} updated", active_path.display()),
        });
    }

    match merge_sys_profile(
        &active_path,
        &base_path,
        &template_path,
        &base,
        &active,
        &template,
    )
    .await?
    {
        ProfileMerge::Clean(merged) => {
            tokio::fs::write(&active_path, merged)
                .await
                .with_context(|| format!("writing {}", active_path.display()))?;
            tokio::fs::write(&base_path, &template)
                .await
                .with_context(|| format!("writing {}", base_path.display()))?;
            remove_if_exists(&new_path).await?;
            remove_if_exists(&merge_path).await?;
            updated = true;
        }
        ProfileMerge::Conflict(merged) => {
            tokio::fs::write(&new_path, &template)
                .await
                .with_context(|| format!("writing {}", new_path.display()))?;
            tokio::fs::write(&merge_path, merged)
                .await
                .with_context(|| format!("writing {}", merge_path.display()))?;
            return Ok(SysProfileFileUpdate {
                updated: true,
                needs_action: true,
                detail: format!(
                    "merge conflict for {}; review {} and {}",
                    active_path.display(),
                    new_path.display(),
                    merge_path.display()
                ),
            });
        }
    }

    Ok(SysProfileFileUpdate {
        updated,
        needs_action: false,
        detail: format!("{} merged", active_path.display()),
    })
}

enum ProfileMerge {
    Clean(Vec<u8>),
    Conflict(Vec<u8>),
}

async fn merge_sys_profile(
    active_path: &Path,
    base_path: &Path,
    template_path: &Path,
    base: &[u8],
    active: &[u8],
    template: &[u8],
) -> Result<ProfileMerge> {
    if let Some(result) = try_git_merge_file(active_path, base_path, template_path).await? {
        return Ok(result);
    }

    Ok(match fallback_three_way_merge(base, active, template) {
        Some(merged) => ProfileMerge::Clean(merged),
        None => ProfileMerge::Conflict(conflict_marker_merge(base, active, template)),
    })
}

async fn try_git_merge_file(
    active_path: &Path,
    base_path: &Path,
    template_path: &Path,
) -> Result<Option<ProfileMerge>> {
    let output = match tokio::process::Command::new("git")
        .arg("merge-file")
        .arg("-p")
        .arg(active_path)
        .arg(base_path)
        .arg(template_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
    {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).context("running git merge-file"),
    };

    match output.status.code() {
        Some(0) => Ok(Some(ProfileMerge::Clean(output.stdout))),
        Some(1) => Ok(Some(ProfileMerge::Conflict(output.stdout))),
        _ => Ok(None),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ProfileChange {
    old_start: usize,
    old_end: usize,
    new_lines: Vec<String>,
}

fn fallback_three_way_merge(base: &[u8], active: &[u8], template: &[u8]) -> Option<Vec<u8>> {
    let base = std::str::from_utf8(base).ok()?;
    let active = std::str::from_utf8(active).ok()?;
    let template = std::str::from_utf8(template).ok()?;

    if active == base {
        return Some(template.as_bytes().to_vec());
    }
    if template == base || active == template {
        return Some(active.as_bytes().to_vec());
    }

    let base_lines = split_profile_lines(base);
    let active_lines = split_profile_lines(active);
    let template_lines = split_profile_lines(template);
    let user_changes = profile_changes(&base_lines, &active_lines);
    let template_changes = profile_changes(&base_lines, &template_lines);

    if has_profile_change_conflicts(&user_changes, &template_changes) {
        return None;
    }

    let mut changes = Vec::new();
    for change in user_changes.into_iter().chain(template_changes) {
        if !changes.iter().any(|existing| existing == &change) {
            changes.push(change);
        }
    }
    changes.sort_by_key(|change| (change.old_start, change.old_end));

    let mut merged = Vec::new();
    let mut cursor = 0;
    for change in changes {
        if change.old_start < cursor {
            return None;
        }
        merged.extend_from_slice(&base_lines[cursor..change.old_start]);
        merged.extend(change.new_lines);
        cursor = change.old_end;
    }
    merged.extend_from_slice(&base_lines[cursor..]);
    Some(merged.concat().into_bytes())
}

fn split_profile_lines(content: &str) -> Vec<String> {
    if content.is_empty() {
        Vec::new()
    } else {
        content.split_inclusive('\n').map(str::to_string).collect()
    }
}

fn profile_changes(base_lines: &[String], new_lines: &[String]) -> Vec<ProfileChange> {
    let base_refs = base_lines.iter().map(String::as_str).collect::<Vec<_>>();
    let new_refs = new_lines.iter().map(String::as_str).collect::<Vec<_>>();
    TextDiff::from_slices(&base_refs, &new_refs)
        .ops()
        .iter()
        .filter_map(|op| {
            if matches!(op.tag(), DiffTag::Equal) {
                return None;
            }
            let old_range = op.old_range();
            let new_range = op.new_range();
            Some(ProfileChange {
                old_start: old_range.start,
                old_end: old_range.end,
                new_lines: new_lines[new_range].to_vec(),
            })
        })
        .collect()
}

fn is_initial_user_profile_edit(template: &[u8], active: &[u8]) -> bool {
    let Ok(template) = std::str::from_utf8(template) else {
        return false;
    };
    let Ok(active) = std::str::from_utf8(active) else {
        return false;
    };

    let template_lines = split_profile_lines(template);
    let active_lines = split_profile_lines(active);
    profile_changes(&template_lines, &active_lines)
        .into_iter()
        .all(|change| is_initial_user_profile_change(&template_lines, &change))
}

fn is_initial_user_profile_change(template_lines: &[String], change: &ProfileChange) -> bool {
    if change.old_start == change.old_end {
        return true;
    }

    let old_lines = &template_lines[change.old_start..change.old_end];
    if old_lines.len() != change.new_lines.len() {
        return false;
    }

    old_lines
        .iter()
        .zip(&change.new_lines)
        .all(|(old, new)| uncomment_profile_line(old).as_deref() == Some(new.as_str()))
}

fn uncomment_profile_line(line: &str) -> Option<String> {
    let indent_len = line.len() - line.trim_start_matches([' ', '\t']).len();
    let (indent, rest) = line.split_at(indent_len);
    let uncommented = rest.strip_prefix("# ")?;
    Some(format!("{indent}{uncommented}"))
}

fn has_profile_change_conflicts(left: &[ProfileChange], right: &[ProfileChange]) -> bool {
    left.iter().any(|left| {
        right.iter().any(|right| {
            if left == right {
                return false;
            }
            profile_changes_overlap(left, right)
        })
    })
}

fn profile_changes_overlap(left: &ProfileChange, right: &ProfileChange) -> bool {
    let left_insert = left.old_start == left.old_end;
    let right_insert = right.old_start == right.old_end;

    if left_insert && right_insert {
        return left.old_start == right.old_start;
    }
    if left_insert {
        return left.old_start >= right.old_start && left.old_start <= right.old_end;
    }
    if right_insert {
        return right.old_start >= left.old_start && right.old_start <= left.old_end;
    }

    left.old_start < right.old_end && right.old_start < left.old_end
}

fn conflict_marker_merge(base: &[u8], active: &[u8], template: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"<<<<<<< current\n");
    out.extend_from_slice(active);
    if !active.ends_with(b"\n") {
        out.extend_from_slice(b"\n");
    }
    out.extend_from_slice(b"||||||| base\n");
    out.extend_from_slice(base);
    if !base.ends_with(b"\n") {
        out.extend_from_slice(b"\n");
    }
    out.extend_from_slice(b"=======\n");
    out.extend_from_slice(template);
    if !template.ends_with(b"\n") {
        out.extend_from_slice(b"\n");
    }
    out.extend_from_slice(b">>>>>>> shine\n");
    out
}

async fn write_if_changed(path: &Path, content: &[u8]) -> Result<bool> {
    if tokio::fs::read(path).await.ok().as_deref() == Some(content) {
        return Ok(false);
    }
    tokio::fs::write(path, content)
        .await
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

async fn remove_if_exists(path: &Path) -> Result<bool> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err).with_context(|| format!("removing {}", path.display())),
    }
}

fn profile_backup_path(active_path: &Path, ext: &str) -> PathBuf {
    active_path.with_extension(format!("{ext}.bak.{}", unix_timestamp()))
}

async fn update_sys_shell_profiles(
    config: &Config,
    os_id: &str,
    sys_shell: &str,
) -> Result<SysShellProfileUpdate> {
    match os_id {
        "macos" => {
            let path = config.home_dir.join(".zshrc");
            let block = sys_shell_profile_block(os_id, None);
            let updated = update_shell_profile_block(&path, sys_sentinel(os_id), &block).await?;
            Ok(SysShellProfileUpdate {
                updated,
                unsupported_shell: false,
                detail: format!("~/.zshrc -> {}", sys_loader_display(os_id)),
            })
        }
        "ubuntu" => match config.shell_type {
            ShellType::Bash => {
                let block = sys_shell_profile_block(os_id, Some("bash"));
                let updated = update_shell_profile_block(
                    &config.home_dir.join(".bashrc"),
                    sys_sentinel(os_id),
                    &block,
                )
                .await?;
                remove_shell_profile_block(&config.home_dir.join(".zshrc"), sys_sentinel(os_id))
                    .await?;
                Ok(SysShellProfileUpdate {
                    updated,
                    unsupported_shell: false,
                    detail: format!("~/.bashrc -> {}", sys_loader_display(os_id)),
                })
            }
            ShellType::Zsh => {
                let block = sys_shell_profile_block(os_id, Some("zsh"));
                let updated = update_shell_profile_block(
                    &config.home_dir.join(".zshrc"),
                    sys_sentinel(os_id),
                    &block,
                )
                .await?;
                remove_shell_profile_block(&config.home_dir.join(".bashrc"), sys_sentinel(os_id))
                    .await?;
                Ok(SysShellProfileUpdate {
                    updated,
                    unsupported_shell: false,
                    detail: format!("~/.zshrc -> {}", sys_loader_display(os_id)),
                })
            }
            _ => Ok(SysShellProfileUpdate {
                updated: false,
                unsupported_shell: true,
                detail: format!("unsupported shell for sys profile: {sys_shell}"),
            }),
        },
        "windows" => {
            let block = sys_shell_profile_block(os_id, None);
            let mut updated = false;
            for path in [
                config
                    .home_dir
                    .join("Documents/PowerShell/Microsoft.PowerShell_profile.ps1"),
                config
                    .home_dir
                    .join("Documents/WindowsPowerShell/Microsoft.PowerShell_profile.ps1"),
            ] {
                updated |= update_shell_profile_block(&path, sys_sentinel(os_id), &block).await?;
            }
            Ok(SysShellProfileUpdate {
                updated,
                unsupported_shell: false,
                detail: "PowerShell profiles".to_string(),
            })
        }
        _ => Ok(SysShellProfileUpdate {
            updated: false,
            unsupported_shell: true,
            detail: format!("unsupported OS for sys profile: {os_id}"),
        }),
    }
}

fn sys_sentinel(os_id: &str) -> (&'static str, &'static str) {
    match os_id {
        "macos" => ("# >>> shine macos sys >>>", "# <<< shine macos sys <<<"),
        "windows" => ("# >>> shine windows sys >>>", "# <<< shine windows sys <<<"),
        _ => ("# >>> shine ubuntu sys >>>", "# <<< shine ubuntu sys <<<"),
    }
}

fn sys_shell_profile_block(os_id: &str, shell_name: Option<&str>) -> String {
    let (start, end) = sys_sentinel(os_id);
    match os_id {
        "windows" => format!(
            r#"{start}
$shineWindowsSysProfile = Join-Path $HOME ".shine\profile\windows-sys.ps1"
if (Test-Path -LiteralPath $shineWindowsSysProfile) {{
    . $shineWindowsSysProfile
}}
{end}
"#
        ),
        "macos" => format!(
            r#"{start}
shine_macos_sys_profile="$HOME/.shine/profile/macos-sys.sh"
if [[ -f "$shine_macos_sys_profile" ]]; then
  source "$shine_macos_sys_profile"
fi
{end}
"#
        ),
        _ => {
            let shell_name = shell_name.unwrap_or("bash");
            format!(
                r#"{start}
shine_ubuntu_sys_profile="$HOME/.shine/profile/ubuntu-sys.sh"
if [[ -f "$shine_ubuntu_sys_profile" ]]; then
  SHINE_UBUNTU_SYS_SHELL="{shell_name}"
  source "$shine_ubuntu_sys_profile"
fi
{end}
"#
            )
        }
    }
}

async fn update_shell_profile_block(
    path: &Path,
    sentinel: (&str, &str),
    desired_block: &str,
) -> Result<bool> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let content = tokio::fs::read_to_string(path).await.unwrap_or_default();
    if extract_sentinel_block(&content, sentinel) == Some(desired_block) {
        return Ok(false);
    }

    let mut updated = remove_sentinel_block(&content, sentinel);
    if !updated.ends_with('\n') && !updated.is_empty() {
        updated.push('\n');
    }
    if !updated.is_empty() {
        updated.push('\n');
    }
    updated.push_str(desired_block);

    tokio::fs::write(path, updated)
        .await
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

async fn remove_shell_profile_block(path: &Path, sentinel: (&str, &str)) -> Result<bool> {
    let Ok(content) = tokio::fs::read_to_string(path).await else {
        return Ok(false);
    };
    let updated = remove_sentinel_block(&content, sentinel);
    if updated == content {
        return Ok(false);
    }
    tokio::fs::write(path, updated)
        .await
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

fn extract_sentinel_block<'a>(content: &'a str, sentinel: (&str, &str)) -> Option<&'a str> {
    let start = content.find(sentinel.0)?;
    let after_start = &content[start..];
    let end = after_start.find(sentinel.1)? + sentinel.1.len();
    let end = if after_start[end..].starts_with('\n') {
        end + 1
    } else {
        end
    };
    Some(&after_start[..end])
}

fn remove_sentinel_block(content: &str, sentinel: (&str, &str)) -> String {
    let mut output = Vec::new();
    let mut skip = false;
    for line in content.lines() {
        if line == sentinel.0 {
            skip = true;
            continue;
        }
        if line == sentinel.1 {
            skip = false;
            continue;
        }
        if !skip {
            output.push(line);
        }
    }
    let mut result = output.join("\n");
    if content.ends_with('\n') && !result.is_empty() {
        result.push('\n');
    }
    result
}

fn sys_loader_display(os_id: &str) -> String {
    format!(
        "~/.shine/profile/{os_id}-sys.{}",
        if os_id == "windows" { "ps1" } else { "sh" }
    )
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn parse_sys_item_output(
    item_id: &str,
    label: &str,
    success: bool,
    stdout: &str,
    stderr: &str,
) -> SysItemOutcome {
    let mut status = None;
    let mut logs = Vec::new();

    for line in stdout.lines().chain(stderr.lines()) {
        if let Some(parsed) = parse_status_event(line) {
            status = Some(parsed);
        } else if !line.trim().is_empty() {
            logs.push(line.to_string());
        }
    }

    let (status, detail) = if !success {
        let detail = status
            .map(|(_, detail)| detail)
            .filter(|detail| !detail.is_empty())
            .unwrap_or_else(|| "script exited with a non-zero status".to_string());
        (SysItemStatus::Failed, detail)
    } else if let Some((status, detail)) = status {
        (status, detail)
    } else {
        (SysItemStatus::Completed, String::new())
    };

    SysItemOutcome {
        item_id: item_id.to_string(),
        label: label.to_string(),
        status,
        detail,
        logs,
    }
}

fn parse_status_event(line: &str) -> Option<(SysItemStatus, String)> {
    let rest = line.strip_prefix(SYS_STATUS_PREFIX)?;
    let mut parts = rest.splitn(2, '\t');
    let status = match parts.next()? {
        "installed" => SysItemStatus::Installed,
        "already-installed" => SysItemStatus::AlreadyInstalled,
        "skipped" => SysItemStatus::Skipped,
        "updated" => SysItemStatus::Updated,
        "needs-action" => SysItemStatus::NeedsAction,
        "completed" => SysItemStatus::Completed,
        "failed" => SysItemStatus::Failed,
        _ => return None,
    };
    let detail = normalize_status_detail(parts.next().unwrap_or_default().trim());
    Some((status, detail))
}

fn normalize_status_detail(detail: &str) -> String {
    detail
        .strip_suffix(" ()")
        .unwrap_or(detail)
        .trim()
        .to_string()
}

fn print_item_outcome(outcome: &SysItemOutcome, label_width: usize) {
    let symbol = status_symbol(outcome.status);
    let label = format!("{:<label_width$}", outcome.label);
    let status = format!("{:<17}", status_text(outcome.status));
    let detail = if outcome.detail.is_empty() {
        String::new()
    } else {
        colors::dim(&outcome.detail)
    };

    println!(
        "{} {} {} {}",
        colors::symbol(symbol),
        colors::bold(&label),
        colors::status_label(&status, symbol),
        detail
    );

    for line in &outcome.logs {
        println!("  {}", colors::dim(line));
    }
}

fn status_symbol(status: SysItemStatus) -> &'static str {
    match status {
        SysItemStatus::Skipped | SysItemStatus::NeedsAction => "~",
        SysItemStatus::Failed => "✗",
        _ => "✓",
    }
}

fn status_text(status: SysItemStatus) -> &'static str {
    match status {
        SysItemStatus::Installed => "installed",
        SysItemStatus::AlreadyInstalled => "already installed",
        SysItemStatus::Skipped => "skipped",
        SysItemStatus::Updated => "updated",
        SysItemStatus::NeedsAction => "needs action",
        SysItemStatus::Completed => "completed",
        SysItemStatus::Failed => "failed",
    }
}

fn print_sys_summary(outcomes: &[SysItemOutcome]) {
    let mut counts = BTreeMap::<SysItemStatus, usize>::new();
    for outcome in outcomes {
        *counts.entry(outcome.status).or_default() += 1;
    }

    let parts = [
        SysItemStatus::Installed,
        SysItemStatus::AlreadyInstalled,
        SysItemStatus::Skipped,
        SysItemStatus::Updated,
        SysItemStatus::NeedsAction,
        SysItemStatus::Completed,
        SysItemStatus::Failed,
    ]
    .into_iter()
    .filter_map(|status| {
        counts
            .get(&status)
            .copied()
            .filter(|count| *count > 0)
            .map(|count| format!("{count} {}", status_text(status)))
    })
    .collect::<Vec<_>>();

    println!("Summary: {}", parts.join(", "));
}

async fn load_sys_preset(config: &Config, os_id: &str) -> Result<LoadedSysPreset> {
    if os_id.contains('/') || os_id.contains('\\') || os_id.contains("..") {
        bail!("invalid os id: {os_id:?}");
    }
    let prefix = format!("sys/{os_id}");
    if !config.is_external_presets {
        crate::presets::extract_prefix(&prefix, config.presets_dir(), true).await?;
    }

    let root = config.presets_dir().join("sys").join(os_id);
    let script_path = root.join(sys_init_script_name(os_id));
    if !script_path.exists() {
        bail!(
            "No init script found for '{}'. Expected: {}",
            os_id,
            script_path.display()
        );
    }

    let manifest_path = root.join("shine.toml");
    let content = tokio::fs::read_to_string(&manifest_path)
        .await
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let manifest = parse_and_validate_manifest(&content)
        .with_context(|| format!("parsing {}", manifest_path.display()))?;

    Ok(LoadedSysPreset {
        manifest,
        script_path,
    })
}

fn sys_init_script_name(os_id: &str) -> &'static str {
    if os_id == "windows" {
        "init.ps1"
    } else {
        "init.sh"
    }
}

fn parse_and_validate_manifest(content: &str) -> Result<SysManifest> {
    let manifest: SysManifest = toml::from_str(content)?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

fn validate_manifest(manifest: &SysManifest) -> Result<()> {
    let mut ids = BTreeSet::new();
    for item in &manifest.items {
        validate_item_id(&item.id)?;
        if item.label.trim().is_empty() {
            bail!("sys init item `{}` must have a label", item.id);
        }
        if !ids.insert(item.id.clone()) {
            bail!("duplicate sys init item id `{}`", item.id);
        }
    }

    if let Some(default_profile) = &manifest.default_profile
        && !manifest.profiles.contains_key(default_profile)
    {
        bail!("default profile `{default_profile}` is not defined");
    }

    for (profile_name, profile) in &manifest.profiles {
        for item_id in &profile.items {
            if !ids.contains(item_id) {
                bail!("profile `{profile_name}` references unknown item `{item_id}`");
            }
        }
    }

    Ok(())
}

fn validate_item_id(item_id: &str) -> Result<()> {
    if item_id.trim().is_empty() {
        bail!("sys init item ids must not be empty");
    }
    if !item_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!(
            "sys init item id `{item_id}` contains invalid characters (allowed: a-z A-Z 0-9 - _)"
        );
    }
    Ok(())
}

fn resolve_selection(
    manifest: &SysManifest,
    preset: Option<&str>,
    interactive: bool,
) -> Result<ResolvedSelection> {
    if let Some(profile_name) = preset {
        return Ok(ResolvedSelection {
            item_ids: profile_items(manifest, profile_name)?.to_vec(),
            source: SelectionSource::Profile(profile_name.to_string()),
        });
    }

    if manifest.items.is_empty() {
        return Ok(ResolvedSelection {
            item_ids: Vec::new(),
            source: SelectionSource::NoItems,
        });
    }

    if interactive {
        return select_items_interactively(manifest);
    }

    let Some(default_profile) = manifest.default_profile.as_deref() else {
        bail!("sys init requires `default_profile` for non-interactive runs");
    };

    Ok(ResolvedSelection {
        item_ids: profile_items(manifest, default_profile)?.to_vec(),
        source: SelectionSource::DefaultProfile(default_profile.to_string()),
    })
}

fn profile_items<'a>(manifest: &'a SysManifest, profile_name: &str) -> Result<&'a [String]> {
    let profile = manifest
        .profiles
        .get(profile_name)
        .with_context(|| format!("unknown sys init profile `{profile_name}`"))?;
    Ok(&profile.items)
}

fn default_flags(manifest: &SysManifest) -> Vec<bool> {
    if let Some(default_profile) = manifest.default_profile.as_deref()
        && let Some(profile) = manifest.profiles.get(default_profile)
    {
        let item_set: BTreeSet<&str> = profile.items.iter().map(String::as_str).collect();
        return manifest
            .items
            .iter()
            .map(|item| item_set.contains(item.id.as_str()))
            .collect();
    }

    manifest.items.iter().map(|item| item.default).collect()
}

fn select_items_interactively(manifest: &SysManifest) -> Result<ResolvedSelection> {
    print_interactive_header(manifest);

    let labels: Vec<String> = manifest.items.iter().map(format_interactive_item).collect();
    let defaults = default_flags(manifest);

    let selection = MultiSelect::with_theme(&sys_init_theme())
        .with_prompt("Select system init items")
        .items(&labels)
        .defaults(&defaults)
        .report(false)
        .interact()?;

    let item_ids = selection
        .into_iter()
        .map(|index| manifest.items[index].id.clone())
        .collect();

    Ok(ResolvedSelection {
        item_ids,
        source: SelectionSource::Interactive,
    })
}

fn format_interactive_item(item: &SysItem) -> String {
    let label = style(item.label.as_str()).for_stderr().bold().to_string();
    if item.description.is_empty() {
        return label;
    }

    let description = style(item.description.as_str())
        .for_stderr()
        .dim()
        .to_string();
    format!("{label}  ·  {description}")
}

fn print_interactive_header(manifest: &SysManifest) {
    if let Some(default_profile) = manifest.default_profile.as_deref() {
        println!(
            "{}",
            colors::dim(&format!("Default profile: {default_profile}"))
        );
    }
    println!("{}", colors::dim("Use Space to toggle, Enter to confirm."));
    println!();
}

fn format_item_ids(item_ids: &[String]) -> String {
    if item_ids.is_empty() {
        "(none)".to_string()
    } else {
        item_ids.join(", ")
    }
}

fn list_embedded_sys_entries() -> Vec<(String, String)> {
    let mut os_ids: BTreeSet<String> = BTreeSet::new();

    for path in crate::presets::asset_paths("sys") {
        let without_prefix = match path.strip_prefix("sys/") {
            Some(s) => s,
            None => continue,
        };
        let slash = match without_prefix.find('/') {
            Some(p) => p,
            None => continue,
        };
        os_ids.insert(without_prefix[..slash].to_string());
    }

    os_ids
        .into_iter()
        .map(|os_id| {
            let toml_path = format!("sys/{os_id}/shine.toml");
            let description = crate::presets::read_asset_bytes(&toml_path)
                .and_then(|b| String::from_utf8(b).ok())
                .and_then(|s| toml::from_str::<SysManifest>(&s).ok())
                .map(|m| m.description)
                .unwrap_or_default();
            (os_id, description)
        })
        .collect()
}

async fn list_fs_sys_entries(presets_dir: &Path) -> Vec<(String, String)> {
    let sys_root = presets_dir.join("sys");
    if !sys_root.is_dir() {
        return Vec::new();
    }

    let mut entries: BTreeMap<String, String> = BTreeMap::new();

    let Ok(mut dir) = tokio::fs::read_dir(&sys_root).await else {
        return Vec::new();
    };

    while let Ok(Some(entry)) = dir.next_entry().await {
        let Ok(ft) = entry.file_type().await else {
            continue;
        };
        if !ft.is_dir() {
            continue;
        }
        let os_id = entry.file_name().to_string_lossy().to_string();
        let toml_path = sys_root.join(&os_id).join("shine.toml");
        let description = if let Ok(content) = tokio::fs::read_to_string(&toml_path).await {
            toml::from_str::<SysManifest>(&content)
                .map(|m| m.description)
                .unwrap_or_default()
        } else {
            String::new()
        };
        entries.insert(os_id, description);
    }

    entries.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::shells::ShellType;
    use std::path::PathBuf;
    use tokio::fs;

    async fn make_temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shine-sys-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).await.unwrap();
        dir
    }

    fn sample_manifest() -> SysManifest {
        parse_and_validate_manifest(
            r#"
description = "Test distro"
default_profile = "recommended"

[[items]]
id = "neovim"
label = "Neovim"
description = "Install Neovim"

[[items]]
id = "atuin"
label = "Atuin"
description = "Install Atuin"
default = true

[profiles.recommended]
items = ["neovim"]

[profiles.full]
items = ["neovim", "atuin"]
"#,
        )
        .unwrap()
    }

    // --- detect_os_id_from ---

    #[test]
    fn detects_macos() {
        let result = detect_os_id_from("macos", None).unwrap();
        assert_eq!(result, "macos");
    }

    #[test]
    fn detects_windows() {
        let result = detect_os_id_from("windows", None).unwrap();
        assert_eq!(result, "windows");
    }

    #[test]
    fn detects_ubuntu_from_os_release() {
        let os_release = "PRETTY_NAME=\"Ubuntu 22.04\"\nID=ubuntu\nVERSION_ID=\"22.04\"\n";
        let result = detect_os_id_from("linux", Some(os_release)).unwrap();
        assert_eq!(result, "ubuntu");
    }

    #[test]
    fn detects_quoted_id() {
        let os_release = "ID=\"ubuntu\"\n";
        let result = detect_os_id_from("linux", Some(os_release)).unwrap();
        assert_eq!(result, "ubuntu");
    }

    #[test]
    fn lowercases_id() {
        let os_release = "ID=Debian\n";
        let result = detect_os_id_from("linux", Some(os_release)).unwrap();
        assert_eq!(result, "debian");
    }

    #[test]
    fn errors_on_linux_without_os_release() {
        let err = detect_os_id_from("linux", None).unwrap_err();
        assert!(err.to_string().contains("os-release"));
    }

    #[test]
    fn errors_on_unsupported_platform() {
        let err = detect_os_id_from("freebsd", None).unwrap_err();
        assert!(err.to_string().contains("freebsd"));
    }

    // --- manifest validation ---

    #[test]
    fn parses_valid_manifest() {
        let manifest = sample_manifest();
        assert_eq!(manifest.description, "Test distro");
        assert_eq!(manifest.default_profile.as_deref(), Some("recommended"));
        assert_eq!(manifest.items.len(), 2);
    }

    #[test]
    fn rejects_duplicate_item_ids() {
        let err = parse_and_validate_manifest(
            r#"
[[items]]
id = "dup"
label = "One"

[[items]]
id = "dup"
label = "Two"
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("duplicate sys init item id"));
    }

    #[test]
    fn rejects_unknown_profile_items() {
        let err = parse_and_validate_manifest(
            r#"
[[items]]
id = "neovim"
label = "Neovim"

[profiles.recommended]
items = ["atuin"]
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown item `atuin`"));
    }

    #[test]
    fn rejects_missing_default_profile() {
        let err = parse_and_validate_manifest(
            r#"
default_profile = "recommended"

[[items]]
id = "neovim"
label = "Neovim"
"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("default profile `recommended`"));
    }

    // --- selection resolution ---

    #[test]
    fn resolve_selection_uses_explicit_profile() {
        let selection = resolve_selection(&sample_manifest(), Some("full"), false).unwrap();
        assert_eq!(selection.item_ids, vec!["neovim", "atuin"]);
        assert_eq!(
            selection.source,
            SelectionSource::Profile("full".to_string())
        );
    }

    #[test]
    fn resolve_selection_uses_default_profile_when_non_interactive() {
        let selection = resolve_selection(&sample_manifest(), None, false).unwrap();
        assert_eq!(selection.item_ids, vec!["neovim"]);
        assert_eq!(
            selection.source,
            SelectionSource::DefaultProfile("recommended".to_string())
        );
    }

    #[test]
    fn resolve_selection_returns_empty_when_no_items_exist() {
        let manifest = parse_and_validate_manifest(
            r#"
description = "Placeholder"
"#,
        )
        .unwrap();
        let selection = resolve_selection(&manifest, None, false).unwrap();
        assert!(selection.item_ids.is_empty());
        assert_eq!(selection.source, SelectionSource::NoItems);
    }

    #[test]
    fn shell_type_into_static_str() {
        assert_eq!(<&'static str>::from(ShellType::Bash), "bash");
        assert_eq!(<&'static str>::from(ShellType::Zsh), "zsh");
        assert_eq!(<&'static str>::from(ShellType::Fish), "fish");
        assert_eq!(<&'static str>::from(ShellType::PowerShell), "powershell");
        assert_eq!(<&'static str>::from(ShellType::Elvish), "elvish");
    }

    #[test]
    fn format_interactive_item_includes_separator_and_description() {
        let item = SysItem {
            id: "neovim".to_string(),
            label: "Neovim".to_string(),
            description: "Install Neovim".to_string(),
            default: false,
        };
        let rendered = format_interactive_item(&item);
        assert!(rendered.contains("Neovim"));
        assert!(rendered.contains("·"));
        assert!(rendered.contains("Install Neovim"));
    }

    #[test]
    fn format_interactive_item_omits_separator_without_description() {
        let item = SysItem {
            id: "atuin".to_string(),
            label: "Atuin".to_string(),
            description: String::new(),
            default: false,
        };
        let rendered = format_interactive_item(&item);
        assert_eq!(rendered, "Atuin");
    }

    #[test]
    fn format_item_ids_handles_empty_selection() {
        assert_eq!(format_item_ids(&[]), "(none)");
    }

    #[test]
    fn parse_status_event_reads_machine_status() {
        let parsed = parse_status_event("SHINE_SYS_STATUS\talready-installed\tatuin 18.16.0")
            .expect("status event should parse");

        assert_eq!(
            parsed,
            (SysItemStatus::AlreadyInstalled, "atuin 18.16.0".to_string())
        );
    }

    #[test]
    fn parse_status_event_trims_empty_version_suffix() {
        let parsed = parse_status_event("SHINE_SYS_STATUS\talready-installed\tatuin 18.13.6 ()")
            .expect("status event should parse");

        assert_eq!(
            parsed,
            (SysItemStatus::AlreadyInstalled, "atuin 18.13.6".to_string())
        );
    }

    #[test]
    fn parse_status_event_ignores_regular_logs() {
        assert!(parse_status_event("Installing Atuin...").is_none());
    }

    #[test]
    fn parse_sys_item_output_uses_status_event_and_keeps_logs() {
        let outcome = parse_sys_item_output(
            "atuin",
            "Atuin",
            true,
            "Installing Atuin...\nSHINE_SYS_STATUS\tinstalled\tatuin 18.16.0\n",
            "",
        );

        assert_eq!(outcome.status, SysItemStatus::Installed);
        assert_eq!(outcome.detail, "atuin 18.16.0");
        assert_eq!(outcome.logs, vec!["Installing Atuin..."]);
    }

    #[test]
    fn parse_sys_item_output_falls_back_for_legacy_success() {
        let outcome =
            parse_sys_item_output("legacy", "Legacy", true, "legacy script completed\n", "");

        assert_eq!(outcome.status, SysItemStatus::Completed);
        assert_eq!(outcome.logs, vec!["legacy script completed"]);
    }

    #[test]
    fn parse_sys_item_output_marks_failed_exit() {
        let outcome =
            parse_sys_item_output("legacy", "Legacy", false, "", "legacy script failed\n");

        assert_eq!(outcome.status, SysItemStatus::Failed);
        assert_eq!(outcome.detail, "script exited with a non-zero status");
        assert_eq!(outcome.logs, vec!["legacy script failed"]);
    }

    #[test]
    fn sys_init_command_uses_zsh_for_macos() {
        let command = sys_init_command("macos");
        assert_eq!(command.program, "zsh");
        assert!(command.fixed_args.is_empty());
    }

    #[test]
    fn sys_init_command_uses_powershell_for_windows() {
        let command = sys_init_command("windows");
        assert_eq!(command.program, "powershell.exe");
        assert_eq!(
            command.fixed_args,
            vec!["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]
        );
    }

    #[test]
    fn sys_init_command_uses_bash_for_other_systems() {
        let ubuntu = sys_init_command("ubuntu");
        let fakeos = sys_init_command("fakeos");
        assert_eq!(ubuntu.program, "bash");
        assert!(ubuntu.fixed_args.is_empty());
        assert_eq!(fakeos.program, "bash");
        assert!(fakeos.fixed_args.is_empty());
    }

    #[test]
    fn sys_init_script_name_uses_ps1_for_windows() {
        assert_eq!(sys_init_script_name("windows"), "init.ps1");
    }

    #[test]
    fn sys_init_script_name_uses_sh_for_other_systems() {
        assert_eq!(sys_init_script_name("macos"), "init.sh");
        assert_eq!(sys_init_script_name("ubuntu"), "init.sh");
    }

    #[test]
    fn format_command_preview_includes_item_ids() {
        let script_path = Path::new("/tmp/init.sh");
        let items = vec!["neovim".to_string(), "atuin".to_string()];
        assert_eq!(
            format_command_preview(&sys_init_command("ubuntu"), script_path, &items),
            "bash /tmp/init.sh neovim atuin"
        );
    }

    #[test]
    fn format_command_preview_includes_windows_fixed_args() {
        let script_path = Path::new("C:/tmp/init.ps1");
        let items = vec!["rust".to_string(), "yazi".to_string()];
        assert_eq!(
            format_command_preview(&sys_init_command("windows"), script_path, &items),
            "powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:/tmp/init.ps1 rust yazi"
        );
    }

    #[tokio::test]
    async fn install_sys_profile_files_creates_active_profile_and_base() {
        let dir = make_temp_dir().await;
        let script_dir = dir.join("presets/sys/ubuntu");
        fs::create_dir_all(&script_dir).await.unwrap();
        fs::write(script_dir.join("profile.sh"), "echo template\n")
            .await
            .unwrap();
        let config = Config::new_for_test(&dir);

        let update = install_sys_profile_files(&config, "ubuntu", &script_dir, false)
            .await
            .unwrap();

        assert!(update.updated);
        assert!(!update.needs_action);
        let profile_dir = dir.join(".shine/profile");
        assert_eq!(
            fs::read_to_string(profile_dir.join("ubuntu-sys.sh"))
                .await
                .unwrap(),
            "echo template\n"
        );
        assert_eq!(
            fs::read_to_string(profile_dir.join("ubuntu-sys.base.sh"))
                .await
                .unwrap(),
            "echo template\n"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn install_sys_profile_files_without_base_reports_needs_action_for_legacy_edits() {
        let dir = make_temp_dir().await;
        let script_dir = dir.join("presets/sys/ubuntu");
        let profile_dir = dir.join(".shine/profile");
        fs::create_dir_all(&script_dir).await.unwrap();
        fs::create_dir_all(&profile_dir).await.unwrap();
        fs::write(script_dir.join("profile.sh"), "echo new template\n")
            .await
            .unwrap();
        fs::write(profile_dir.join("ubuntu-sys.sh"), "echo user edit\n")
            .await
            .unwrap();
        let config = Config::new_for_test(&dir);

        let update = install_sys_profile_files(&config, "ubuntu", &script_dir, false)
            .await
            .unwrap();

        assert!(update.updated);
        assert!(update.needs_action);
        assert_eq!(
            fs::read_to_string(profile_dir.join("ubuntu-sys.sh"))
                .await
                .unwrap(),
            "echo user edit\n"
        );
        assert!(
            fs::read_to_string(profile_dir.join("ubuntu-sys.new.sh"))
                .await
                .unwrap()
                .contains("echo new template")
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn install_sys_profile_files_without_base_accepts_uncommented_template_lines() {
        let dir = make_temp_dir().await;
        let script_dir = dir.join("presets/sys/macos");
        let profile_dir = dir.join(".shine/profile");
        fs::create_dir_all(&script_dir).await.unwrap();
        fs::create_dir_all(&profile_dir).await.unwrap();
        let template = "# fastfetch\n# if [[ -z \"$ZELLIJ\" ]] && command -v fastfetch >/dev/null 2>&1; then\n#   fastfetch\n# fi\n";
        let active = "# fastfetch\nif [[ -z \"$ZELLIJ\" ]] && command -v fastfetch >/dev/null 2>&1; then\n  fastfetch\nfi\n";
        fs::write(script_dir.join("profile.sh"), template)
            .await
            .unwrap();
        fs::write(profile_dir.join("macos-sys.sh"), active)
            .await
            .unwrap();
        let config = Config::new_for_test(&dir);

        let update = install_sys_profile_files(&config, "macos", &script_dir, false)
            .await
            .unwrap();

        assert!(update.updated);
        assert!(!update.needs_action);
        assert_eq!(
            fs::read_to_string(profile_dir.join("macos-sys.sh"))
                .await
                .unwrap(),
            active
        );
        assert_eq!(
            fs::read_to_string(profile_dir.join("macos-sys.base.sh"))
                .await
                .unwrap(),
            template
        );
        assert!(!profile_dir.join("macos-sys.new.sh").exists());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn install_sys_profile_files_force_profile_backs_up_and_replaces_active() {
        let dir = make_temp_dir().await;
        let script_dir = dir.join("presets/sys/ubuntu");
        let profile_dir = dir.join(".shine/profile");
        fs::create_dir_all(&script_dir).await.unwrap();
        fs::create_dir_all(&profile_dir).await.unwrap();
        fs::write(script_dir.join("profile.sh"), "echo template\n")
            .await
            .unwrap();
        fs::write(profile_dir.join("ubuntu-sys.sh"), "echo user edit\n")
            .await
            .unwrap();
        let config = Config::new_for_test(&dir);

        let update = install_sys_profile_files(&config, "ubuntu", &script_dir, true)
            .await
            .unwrap();

        assert!(update.updated);
        assert!(!update.needs_action);
        assert_eq!(
            fs::read_to_string(profile_dir.join("ubuntu-sys.sh"))
                .await
                .unwrap(),
            "echo template\n"
        );
        assert_eq!(
            fs::read_to_string(profile_dir.join("ubuntu-sys.base.sh"))
                .await
                .unwrap(),
            "echo template\n"
        );
        let mut entries = fs::read_dir(&profile_dir).await.unwrap();
        let mut backup_found = false;
        while let Some(entry) = entries.next_entry().await.unwrap() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("ubuntu-sys.sh.bak.") {
                backup_found = true;
            }
        }
        assert!(backup_found, "legacy profile backup should be created");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn fallback_three_way_merge_preserves_uncommented_line_position() {
        let base = b"before\n# eval \"$(starship init zsh)\"\nafter\n";
        let active = b"before\neval \"$(starship init zsh)\"\nafter\n";
        let template = b"before\n# eval \"$(starship init zsh)\"\nafter\nnew-template-line\n";

        let merged = fallback_three_way_merge(base, active, template).unwrap();

        assert_eq!(
            String::from_utf8(merged).unwrap(),
            "before\neval \"$(starship init zsh)\"\nafter\nnew-template-line\n"
        );
    }

    #[test]
    fn fallback_three_way_merge_reports_conflict_for_same_line_edits() {
        let base = b"before\nvalue=old\nafter\n";
        let active = b"before\nvalue=user\nafter\n";
        let template = b"before\nvalue=shine\nafter\n";

        assert!(fallback_three_way_merge(base, active, template).is_none());
    }

    #[tokio::test]
    async fn update_sys_shell_profiles_writes_active_ubuntu_shell_and_removes_other_shell_block() {
        let dir = make_temp_dir().await;
        let mut config = Config::new_for_test(&dir);
        config.shell_type = ShellType::Bash;
        fs::write(
            dir.join(".zshrc"),
            "# before\n# >>> shine ubuntu sys >>>\nold\n# <<< shine ubuntu sys <<<\n# after\n",
        )
        .await
        .unwrap();

        let update = update_sys_shell_profiles(&config, "ubuntu", "bash")
            .await
            .unwrap();

        assert!(update.updated);
        let bashrc = fs::read_to_string(dir.join(".bashrc")).await.unwrap();
        assert!(bashrc.contains("SHINE_UBUNTU_SYS_SHELL=\"bash\""));
        assert!(bashrc.contains("source \"$shine_ubuntu_sys_profile\""));
        let zshrc = fs::read_to_string(dir.join(".zshrc")).await.unwrap();
        assert!(!zshrc.contains("# >>> shine ubuntu sys >>>"));
        assert!(zshrc.contains("# before"));
        assert!(zshrc.contains("# after"));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    // --- list_embedded_sys_entries ---

    #[test]
    fn embedded_entries_include_supported_systems() {
        let entries = list_embedded_sys_entries();
        let ids: Vec<&str> = entries.iter().map(|(id, _)| id.as_str()).collect();
        assert!(ids.contains(&"ubuntu"), "ubuntu missing: {ids:?}");
        assert!(ids.contains(&"macos"), "macos missing: {ids:?}");
        assert!(ids.contains(&"windows"), "windows missing: {ids:?}");
    }

    #[test]
    fn embedded_entries_have_descriptions() {
        let entries = list_embedded_sys_entries();
        for (id, desc) in &entries {
            assert!(!desc.is_empty(), "description for {id} should not be empty");
        }
    }

    #[test]
    fn embedded_sys_manifests_are_valid() {
        for (id, _) in list_embedded_sys_entries() {
            let toml_path = format!("sys/{id}/shine.toml");
            let content = crate::presets::read_asset_bytes(&toml_path)
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or_else(|| panic!("missing embedded manifest: {toml_path}"));
            parse_and_validate_manifest(&content)
                .unwrap_or_else(|err| panic!("invalid embedded manifest {toml_path}: {err}"));
        }
    }

    #[test]
    fn embedded_ubuntu_profiles_cover_recommended_and_all_items() {
        let content = crate::presets::read_asset_bytes("sys/ubuntu/shine.toml")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .expect("missing embedded Ubuntu manifest");
        let manifest = parse_and_validate_manifest(&content).unwrap();
        let recommended = manifest
            .profiles
            .get("recommended")
            .expect("missing Ubuntu recommended profile");
        let all = manifest
            .profiles
            .get("all")
            .expect("missing Ubuntu all profile");

        assert!(recommended.items.iter().any(|item| item == "starship"));
        assert!(recommended.items.iter().any(|item| item == "zoxide"));
        assert!(recommended.items.iter().any(|item| item == "zsh-vi-mode"));
        assert!(recommended.items.iter().any(|item| item == "fzf"));
        assert!(recommended.items.iter().any(|item| item == "bat"));
        assert!(recommended.items.iter().any(|item| item == "eza"));
        assert!(!recommended.items.iter().any(|item| item == "pnpm"));
        assert!(!recommended.items.iter().any(|item| item == "mise"));
        assert!(!recommended.items.iter().any(|item| item == "homebrew"));

        let item_ids: BTreeSet<&str> = manifest.items.iter().map(|item| item.id.as_str()).collect();
        let all_ids: BTreeSet<&str> = all.items.iter().map(String::as_str).collect();
        assert_eq!(
            all_ids, item_ids,
            "Ubuntu all profile should include every item"
        );
    }

    #[test]
    fn embedded_windows_profiles_cover_required_recommended_and_all_items() {
        let content = crate::presets::read_asset_bytes("sys/windows/shine.toml")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .expect("missing embedded Windows manifest");
        let manifest = parse_and_validate_manifest(&content).unwrap();
        let required = manifest
            .profiles
            .get("required")
            .expect("missing Windows required profile");
        let recommended = manifest
            .profiles
            .get("recommended")
            .expect("missing Windows recommended profile");
        let all = manifest
            .profiles
            .get("all")
            .expect("missing Windows all profile");

        assert_eq!(required.items, vec!["rust", "yazi", "starship"]);
        assert!(recommended.items.iter().any(|item| item == "zoxide"));
        assert!(recommended.items.iter().any(|item| item == "atuin"));
        assert!(recommended.items.iter().any(|item| item == "fzf"));
        assert!(recommended.items.iter().any(|item| item == "bat"));
        assert!(recommended.items.iter().any(|item| item == "eza"));
        assert!(recommended.items.iter().any(|item| item == "zerotier"));
        assert!(!recommended.items.iter().any(|item| item == "bun"));
        assert!(!recommended.items.iter().any(|item| item == "pnpm"));
        assert!(!recommended.items.iter().any(|item| item == "mise"));

        let item_ids: BTreeSet<&str> = manifest.items.iter().map(|item| item.id.as_str()).collect();
        let all_ids: BTreeSet<&str> = all.items.iter().map(String::as_str).collect();
        assert_eq!(
            all_ids, item_ids,
            "Windows all profile should include every item"
        );
    }

    #[test]
    fn embedded_windows_init_uses_current_atuin_winget_id() {
        let content = crate::presets::read_asset_bytes("sys/windows/init.ps1")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .expect("missing embedded Windows init script");

        assert!(content.contains("\"Atuinsh.Atuin\""));
        assert!(!content.contains("\"atuinsh.atuin\""));
    }

    #[test]
    fn embedded_sys_init_scripts_include_yazi_shell_wrapper() {
        for (path, marker) in [
            ("sys/ubuntu/profile.sh", "y() {"),
            ("sys/macos/profile.sh", "y() {"),
            ("sys/windows/profile.ps1", "function y {"),
        ] {
            let content = crate::presets::read_asset_bytes(path)
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or_else(|| panic!("missing embedded sys init script: {path}"));

            assert!(
                content.contains(marker),
                "{path} should define Yazi wrapper"
            );
            assert!(
                content.contains("--cwd-file"),
                "{path} should pass --cwd-file to yazi"
            );
        }
    }

    #[test]
    fn embedded_ubuntu_init_installs_managed_profile_loader() {
        let content = crate::presets::read_asset_bytes("sys/ubuntu/init.sh")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .expect("missing embedded Ubuntu init script");

        assert!(content.contains("SHINE_SYS_STATUS\\t%s\\t%s\\n"));
        assert!(content.contains("status \"already-installed\" \"$(atuin --version)\""));
        assert!(content.contains(
            "curl --proto '=https' --tlsv1.2 -LsSf https://setup.atuin.sh | sh\n    load_atuin_env\n    status \"installed\" \"$(atuin --version)\""
        ));
        assert!(content.contains("load_atuin_env"));
        assert!(content.contains(". \"$HOME/.atuin/bin/env\""));
        assert!(content.contains(
            "__shine_finalize) status \"completed\" \"profile is managed by shine CLI\""
        ));
        assert!(!content.contains("append_shell_block"));
        assert!(!content.contains("cp \"$template_path\" \"$managed_path\""));
    }

    #[test]
    fn embedded_macos_init_installs_managed_profile_loader() {
        let content = crate::presets::read_asset_bytes("sys/macos/init.sh")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .expect("missing embedded macOS init script");

        assert!(content.contains(
            "__shine_finalize) status \"completed\" \"profile is managed by shine CLI\""
        ));
        assert!(!content.contains("append_zshrc_block"));
        assert!(!content.contains("cp \"$template_path\" \"$managed_path\""));
    }

    #[test]
    fn embedded_macos_profile_initializes_homebrew_zsh_completions() {
        let content = crate::presets::read_asset_bytes("sys/macos/profile.sh")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .expect("missing embedded macOS profile script");

        assert!(content.contains("share/zsh/site-functions"));
        assert!(content.contains("ZSH_VERSION"));
        assert!(content.contains("typeset -U fpath"));
    }

    #[test]
    fn embedded_ubuntu_profile_initializes_atuin() {
        let content = crate::presets::read_asset_bytes("sys/ubuntu/profile.sh")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .expect("missing embedded Ubuntu profile script");

        assert!(content.contains("atuin init"));
        assert!(content.contains("shine_ubuntu_sys_shell"));
        assert!(content.contains(". \"$HOME/.atuin/bin/env\""));
    }

    #[test]
    fn embedded_ubuntu_profile_initializes_homebrew_zsh_completions() {
        let content = crate::presets::read_asset_bytes("sys/ubuntu/profile.sh")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .expect("missing embedded Ubuntu profile script");

        assert!(content.contains("share/zsh/site-functions"));
        assert!(content.contains("shine_ubuntu_sys_shell"));
        assert!(content.contains("ZSH_VERSION"));
        assert!(content.contains("typeset -U fpath"));
    }

    #[test]
    fn embedded_windows_init_installs_managed_profile_loader() {
        let content = crate::presets::read_asset_bytes("sys/windows/init.ps1")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .expect("missing embedded Windows init script");

        assert!(content.contains("SHINE_SYS_PRESET_ROOT"));
        assert!(content.contains("SHINE_SYS_STATUS`t$State`t$Detail"));
        assert!(content.contains("\"__shine_finalize\" { Write-Status \"completed\" \"profile is managed by shine CLI\" }"));
        assert!(!content.contains("Update-ManagedProfiles"));
        assert!(!content.contains("Copy-Item -LiteralPath $profileTemplatePath"));
    }

    #[test]
    fn embedded_entries_sorted_alphabetically() {
        let entries = list_embedded_sys_entries();
        let ids: Vec<&str> = entries.iter().map(|(id, _)| id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted, "entries should be alphabetically sorted");
    }

    // --- list_fs_sys_entries ---

    #[tokio::test]
    async fn list_fs_returns_empty_when_sys_dir_missing() {
        let dir = make_temp_dir().await;
        let entries = list_fs_sys_entries(&dir).await;
        assert!(entries.is_empty());
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn list_fs_reads_description_from_shine_toml() {
        let dir = make_temp_dir().await;
        let os_dir = dir.join("sys/testlinux");
        fs::create_dir_all(&os_dir).await.unwrap();
        fs::write(
            os_dir.join("shine.toml"),
            b"description = \"A test distro.\"\n",
        )
        .await
        .unwrap();

        let entries = list_fs_sys_entries(&dir).await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "testlinux");
        assert_eq!(entries[0].1, "A test distro.");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    // --- handle_list ---

    #[tokio::test]
    async fn handle_list_succeeds_with_embedded_presets() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        handle_list(&config).await.unwrap();
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn load_sys_preset_refreshes_stale_embedded_runtime_files() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        let os_dir = config.presets_dir().join("sys/ubuntu");
        fs::create_dir_all(&os_dir).await.unwrap();
        fs::write(
            os_dir.join("shine.toml"),
            r#"
description = "Stale Ubuntu"
default_profile = "recommended"

[[items]]
id = "neovim"
label = "Neovim"

[profiles.recommended]
items = ["neovim"]
"#,
        )
        .await
        .unwrap();
        fs::write(os_dir.join("init.sh"), b"#!/bin/bash\necho stale\n")
            .await
            .unwrap();

        let loaded = load_sys_preset(&config, "ubuntu").await.unwrap();

        assert!(
            loaded
                .manifest
                .items
                .iter()
                .any(|item| item.id == "homebrew"),
            "embedded Ubuntu manifest should refresh stale runtime files"
        );
        assert!(
            loaded
                .manifest
                .profiles
                .get("all")
                .is_some_and(|profile| profile.items.iter().any(|item| item == "homebrew")),
            "refreshed Ubuntu manifest should include all profile"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    // --- handle_init dry_run ---

    #[cfg(unix)]
    #[tokio::test]
    async fn handle_init_dry_run_does_not_execute_script() {
        let dir = make_temp_dir().await;
        let os_dir = dir.join("presets/sys/fakeos");
        fs::create_dir_all(&os_dir).await.unwrap();

        fs::write(
            os_dir.join("shine.toml"),
            r#"
description = "Fake OS"
default_profile = "recommended"

[[items]]
id = "touch-file"
label = "Touch file"

[profiles.recommended]
items = ["touch-file"]
"#,
        )
        .await
        .unwrap();

        let sentinel = dir.join("executed");
        let script = format!("#!/bin/bash\ntouch {}\n", sentinel.display());
        fs::write(os_dir.join("init.sh"), script.as_bytes())
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;

        handle_init_for_os(&config, "fakeos", None, true, false)
            .await
            .unwrap();
        assert!(!sentinel.exists(), "script must not have been executed");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn handle_init_executes_items_then_updates_profile_in_rust() {
        let dir = make_temp_dir().await;
        let os_dir = dir.join("presets/sys/fakeos");
        fs::create_dir_all(&os_dir).await.unwrap();

        fs::write(
            os_dir.join("shine.toml"),
            r#"
description = "Fake OS"
default_profile = "recommended"

[[items]]
id = "first"
label = "First"

[[items]]
id = "second"
label = "Second"

[profiles.recommended]
items = ["first", "second"]
"#,
        )
        .await
        .unwrap();

        let calls = dir.join("calls");
        fs::write(os_dir.join("profile.sh"), "echo fake profile\n")
            .await
            .unwrap();

        let script = format!(
            r#"#!/bin/bash
set -euo pipefail
printf '%s\n' "$1" >> {calls:?}
case "$1" in
  first) printf 'SHINE_SYS_STATUS\tinstalled\tfirst ok\n' ;;
  second) printf 'legacy log\n' ;;
  *) exit 1 ;;
esac
"#
        );
        fs::write(os_dir.join("init.sh"), script.as_bytes())
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;

        handle_init_for_os(&config, "fakeos", None, false, false)
            .await
            .unwrap();

        let calls = fs::read_to_string(&calls).await.unwrap();
        assert_eq!(calls.lines().collect::<Vec<_>>(), ["first", "second"]);
        assert_eq!(
            fs::read_to_string(dir.join(".shine/profile/fakeos-sys.sh"))
                .await
                .unwrap(),
            "echo fake profile\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join(".shine/profile/fakeos-sys.base.sh"))
                .await
                .unwrap(),
            "echo fake profile\n"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn handle_init_stops_items_after_failure_but_updates_profile_for_successes() {
        let dir = make_temp_dir().await;
        let os_dir = dir.join("presets/sys/fakeos");
        fs::create_dir_all(&os_dir).await.unwrap();

        fs::write(
            os_dir.join("shine.toml"),
            r#"
description = "Fake OS"
default_profile = "recommended"

[[items]]
id = "first"
label = "First"

[[items]]
id = "fails"
label = "Fails"

[[items]]
id = "after"
label = "After"

[profiles.recommended]
items = ["first", "fails", "after"]
"#,
        )
        .await
        .unwrap();

        let calls = dir.join("calls");
        fs::write(os_dir.join("profile.sh"), "echo fake profile\n")
            .await
            .unwrap();

        let script = format!(
            r#"#!/bin/bash
set -euo pipefail
printf '%s\n' "$1" >> {calls:?}
case "$1" in
  first) printf 'SHINE_SYS_STATUS\tinstalled\tfirst ok\n' ;;
  fails) printf 'SHINE_SYS_STATUS\tfailed\tbad item\n'; exit 1 ;;
  after) printf 'SHINE_SYS_STATUS\tinstalled\tafter ok\n' ;;
  *) exit 1 ;;
esac
"#
        );
        fs::write(os_dir.join("init.sh"), script.as_bytes())
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;

        let err = handle_init_for_os(&config, "fakeos", None, false, false)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("sys init failed"));
        let calls = fs::read_to_string(&calls).await.unwrap();
        assert_eq!(calls.lines().collect::<Vec<_>>(), ["first", "fails"]);
        assert_eq!(
            fs::read_to_string(dir.join(".shine/profile/fakeos-sys.sh"))
                .await
                .unwrap(),
            "echo fake profile\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join(".shine/profile/fakeos-sys.base.sh"))
                .await
                .unwrap(),
            "echo fake profile\n"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }
}
