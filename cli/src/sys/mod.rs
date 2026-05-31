use anyhow::{Context, Result, bail};
use console::{Style, style};
use dialoguer::{MultiSelect, theme::ColorfulTheme};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

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
pub(crate) fn detect_os_id() -> Result<String> {
    let os_release = std::fs::read_to_string("/etc/os-release").ok();
    detect_os_id_from(std::env::consts::OS, os_release.as_deref())
}

fn detect_os_id_from(os: &str, os_release: Option<&str>) -> Result<String> {
    match os {
        "macos" => Ok("macos".to_string()),
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
            "Unsupported platform '{}'. Supported targets: ubuntu (Linux), macos",
            other
        ),
    }
}

pub(crate) async fn handle_list(config: &Config) -> Result<()> {
    crate::config::print_presets_note(config);

    let current_os = detect_os_id().ok();

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
    let os_id = detect_os_id()?;
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

    if dry_run {
        print_dry_run(os_id, &loaded, &selection).await?;
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

    println!("Running system init for {}...", colors::bold(os_id));
    print_selection_summary(&selection);
    println!();

    let shell = sys_init_shell(os_id);
    let status = tokio::process::Command::new(shell)
        .arg(&loaded.script_path)
        .args(&selection.item_ids)
        .status()
        .await
        .with_context(|| format!("failed to execute {}", loaded.script_path.display()))?;

    if !status.success() {
        bail!("sys init script exited with {status}");
    }

    println!();
    println!("{}", colors::green("System initialization complete."));
    Ok(())
}

async fn print_dry_run(
    os_id: &str,
    loaded: &LoadedSysPreset,
    selection: &ResolvedSelection,
) -> Result<()> {
    println!("{}", colors::dim("[dry-run] System init preview"));
    println!("  OS: {os_id}");
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
    println!(
        "  Command: {}",
        format_command_preview(
            sys_init_shell(os_id),
            &loaded.script_path,
            &selection.item_ids
        )
    );
    println!();
    let content = tokio::fs::read_to_string(&loaded.script_path)
        .await
        .with_context(|| format!("reading {}", loaded.script_path.display()))?;
    println!("{}", colors::dim("--- script content ---"));
    print!("{content}");
    Ok(())
}

fn sys_init_shell(os_id: &str) -> &'static str {
    if os_id == "macos" { "zsh" } else { "bash" }
}

fn format_command_preview(shell: &str, script_path: &Path, item_ids: &[String]) -> String {
    let mut command = format!("{} {}", shell, script_path.display());
    for item in item_ids {
        command.push(' ');
        command.push_str(item);
    }
    command
}

async fn load_sys_preset(config: &Config, os_id: &str) -> Result<LoadedSysPreset> {
    if os_id.contains('/') || os_id.contains('\\') || os_id.contains("..") {
        bail!("invalid os id: {os_id:?}");
    }
    let prefix = format!("sys/{os_id}");
    if !config.is_external_presets {
        crate::presets::extract_prefix(&prefix, config.presets_dir(), false).await?;
    }

    let root = config.presets_dir().join("sys").join(os_id);
    let script_path = root.join("init.sh");
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

fn parse_and_validate_manifest(content: &str) -> Result<SysManifest> {
    let manifest: SysManifest = toml::from_str(content)?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

fn validate_manifest(manifest: &SysManifest) -> Result<()> {
    let mut ids = BTreeSet::new();
    for item in &manifest.items {
        if item.id.trim().is_empty() {
            bail!("sys init item ids must not be empty");
        }
        if !item
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            bail!(
                "sys init item id `{}` contains invalid characters (allowed: a-z A-Z 0-9 - _)",
                item.id
            );
        }
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

fn resolve_selection(
    manifest: &SysManifest,
    preset: Option<&str>,
    interactive: bool,
) -> Result<ResolvedSelection> {
    if let Some(profile_name) = preset {
        return Ok(ResolvedSelection {
            item_ids: profile_items(manifest, profile_name)?,
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
        item_ids: profile_items(manifest, default_profile)?,
        source: SelectionSource::DefaultProfile(default_profile.to_string()),
    })
}

fn profile_items(manifest: &SysManifest, profile_name: &str) -> Result<Vec<String>> {
    let profile = manifest
        .profiles
        .get(profile_name)
        .with_context(|| format!("unknown sys init profile `{profile_name}`"))?;
    Ok(profile.items.clone())
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
    println!("{}", colors::bold("System Init"));
    if let Some(default_profile) = manifest.default_profile.as_deref() {
        println!(
            "{}",
            colors::dim(&format!("Default profile: {default_profile}"))
        );
    }
    println!("{}", colors::dim("Use Space to toggle, Enter to confirm."));
    println!();
}

fn print_selection_summary(selection: &ResolvedSelection) {
    println!(
        "{}",
        colors::dim(&format!("Selection: {}", selection.source.describe()))
    );
    println!(
        "{}",
        colors::dim(&format!("Items: {}", format_item_ids(&selection.item_ids)))
    );
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
        let err = detect_os_id_from("windows", None).unwrap_err();
        assert!(err.to_string().contains("windows"));
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
    fn sys_init_shell_uses_zsh_for_macos() {
        assert_eq!(sys_init_shell("macos"), "zsh");
    }

    #[test]
    fn sys_init_shell_uses_bash_for_other_systems() {
        assert_eq!(sys_init_shell("ubuntu"), "bash");
        assert_eq!(sys_init_shell("fakeos"), "bash");
    }

    #[test]
    fn format_command_preview_includes_item_ids() {
        let script_path = Path::new("/tmp/init.sh");
        let items = vec!["neovim".to_string(), "atuin".to_string()];
        assert_eq!(
            format_command_preview("bash", script_path, &items),
            "bash /tmp/init.sh neovim atuin"
        );
    }

    #[test]
    fn format_command_preview_uses_selected_shell() {
        let script_path = Path::new("/tmp/init.sh");
        let items = vec!["homebrew".to_string()];
        assert_eq!(
            format_command_preview("zsh", script_path, &items),
            "zsh /tmp/init.sh homebrew"
        );
    }

    // --- list_embedded_sys_entries ---

    #[test]
    fn embedded_entries_include_ubuntu_and_macos() {
        let entries = list_embedded_sys_entries();
        let ids: Vec<&str> = entries.iter().map(|(id, _)| id.as_str()).collect();
        assert!(ids.contains(&"ubuntu"), "ubuntu missing: {ids:?}");
        assert!(ids.contains(&"macos"), "macos missing: {ids:?}");
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
}
