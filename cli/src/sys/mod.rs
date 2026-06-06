use anyhow::{Context, Result, bail};
use console::{Style, style};
use dialoguer::{MultiSelect, theme::ColorfulTheme};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::colors;
use crate::config::Config;

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
const SYS_FINALIZE_ITEM: &str = "__shine_finalize";

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
) -> Result<()> {
    let os_id = detect_os_id().await?;
    handle_init_for_os(config, &os_id, preset, dry_run).await
}

async fn handle_init_for_os(
    config: &Config,
    os_id: &str,
    preset: Option<&str>,
    dry_run: bool,
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
        let finalize = run_sys_item(
            &command,
            script_dir,
            &loaded.script_path,
            sys_shell,
            SYS_FINALIZE_ITEM,
            "profile",
        )
        .await?;
        if finalize.status != SysItemStatus::Completed || !finalize.logs.is_empty() {
            print_item_outcome(&finalize, label_width);
        }
        outcomes.push(finalize);
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
        println!(
            "    {}",
            format_command_preview(
                &command,
                &loaded.script_path,
                &[SYS_FINALIZE_ITEM.to_string()]
            )
        );
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

        assert!(content.contains("profile.sh"));
        assert!(content.contains(".shine/profile/ubuntu-sys.sh"));
        assert!(content.contains("cp \"$template_path\" \"$managed_path\""));
        assert!(content.contains("SHINE_UBUNTU_SYS_SHELL"));
        assert!(content.contains("SHINE_SYS_SHELL"));
        assert!(content.contains("[[ -f \"$file\" ]] || return 0"));
        assert!(content.contains("append_shell_block \"$HOME/.bashrc\" bash"));
        assert!(content.contains("remove_shell_block \"$HOME/.zshrc\""));
        assert!(content.contains("append_shell_block \"$HOME/.zshrc\" zsh"));
        assert!(content.contains("remove_shell_block \"$HOME/.bashrc\""));
        assert!(content.contains("SHINE_SYS_STATUS\\t%s\\t%s\\n"));
        assert!(content.contains("status \"already-installed\" \"$(atuin --version)\""));
        assert!(content.contains(
            "curl --proto '=https' --tlsv1.2 -LsSf https://setup.atuin.sh | sh\n    load_atuin_env\n    status \"installed\" \"$(atuin --version)\""
        ));
        assert!(content.contains("load_atuin_env"));
        assert!(content.contains(". \"$HOME/.atuin/bin/env\""));
        assert!(content.contains("__shine_finalize) append_shell_init_blocks"));
    }

    #[test]
    fn embedded_macos_init_installs_managed_profile_loader() {
        let content = crate::presets::read_asset_bytes("sys/macos/init.sh")
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .expect("missing embedded macOS init script");

        assert!(content.contains("profile.sh"));
        assert!(content.contains(".shine/profile/macos-sys.sh"));
        assert!(content.contains("cp \"$template_path\" \"$managed_path\""));
        assert!(content.contains("shine_macos_sys_profile"));
        assert!(content.contains("[[ -f \"$file\" ]] || return 0"));
        assert!(content.contains("__shine_finalize) append_zshrc_init_block"));
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

        assert!(content.contains("profile.ps1"));
        assert!(content.contains("SHINE_SYS_PRESET_ROOT"));
        assert!(content.contains(".shine\\profile\\windows-sys.ps1"));
        assert!(content.contains("Copy-Item -LiteralPath $profileTemplatePath"));
        assert!(content.contains("$shineWindowsSysProfile"));
        assert!(content.contains("SHINE_SYS_STATUS`t$State`t$Detail"));
        assert!(content.contains("\"__shine_finalize\" { Update-ManagedProfiles }"));
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

        handle_init_for_os(&config, "fakeos", None, true)
            .await
            .unwrap();
        assert!(!sentinel.exists(), "script must not have been executed");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn handle_init_executes_items_then_finalize() {
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
        let script = format!(
            r#"#!/bin/bash
set -euo pipefail
printf '%s\n' "$1" >> {calls:?}
case "$1" in
  first) printf 'SHINE_SYS_STATUS\tinstalled\tfirst ok\n' ;;
  second) printf 'legacy log\n' ;;
  __shine_finalize) printf 'SHINE_SYS_STATUS\tupdated\tprofile ok\n' ;;
  *) exit 1 ;;
esac
"#
        );
        fs::write(os_dir.join("init.sh"), script.as_bytes())
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;

        handle_init_for_os(&config, "fakeos", None, false)
            .await
            .unwrap();

        let calls = fs::read_to_string(&calls).await.unwrap();
        assert_eq!(
            calls.lines().collect::<Vec<_>>(),
            ["first", "second", SYS_FINALIZE_ITEM]
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn handle_init_stops_items_after_failure_but_finalizes_successes() {
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
        let script = format!(
            r#"#!/bin/bash
set -euo pipefail
printf '%s\n' "$1" >> {calls:?}
case "$1" in
  first) printf 'SHINE_SYS_STATUS\tinstalled\tfirst ok\n' ;;
  fails) printf 'SHINE_SYS_STATUS\tfailed\tbad item\n'; exit 1 ;;
  after) printf 'SHINE_SYS_STATUS\tinstalled\tafter ok\n' ;;
  __shine_finalize) printf 'SHINE_SYS_STATUS\tupdated\tprofile ok\n' ;;
  *) exit 1 ;;
esac
"#
        );
        fs::write(os_dir.join("init.sh"), script.as_bytes())
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;

        let err = handle_init_for_os(&config, "fakeos", None, false)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("sys init failed"));
        let calls = fs::read_to_string(&calls).await.unwrap();
        assert_eq!(
            calls.lines().collect::<Vec<_>>(),
            ["first", "fails", SYS_FINALIZE_ITEM]
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }
}
