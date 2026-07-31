use super::links::{build_link_specs, print_link_conflicts};
use super::profile::{
    append_path_to_shell_config, managed_shell_profile_path, print_source_command_activation_hint,
    shell_source_command,
};
use super::report::{
    ShellUpgradeReport, link_report_summary_parts, preset_extract_summary_parts,
    upgrade_link_report_summary_parts,
};
use super::template::{ScriptTemplate, apply_template_to_scripts};
use super::{PathUpdateStatus, get_shell_config_path, metadata};
use crate::colors;
use crate::config::Config;
use crate::output;
use anyhow::{Context, Result};
use std::path::Path;

const SHELL_TEMPLATE: &str = r#"# Shell preset metadata for shine.
description = "My shell helper commands."

[[files]]
source = "my_tool.sh"
target = "mytool"
needs_source = false
# Optional: limit a file to specific platforms.
# platforms = ["unix"]      # or ["windows"]

# PowerShell scripts are also supported:
# source = "my_tool.ps1"

# Cross-platform Bun helpers (requires `bun` on PATH; shine never installs it):
# [[files]]
# source = "my_tool.ts"     # .ts / .js / .mts / .mjs
# target = "mytool"
# runtime = "bun"
# platforms = ["unix", "windows"]
# description = "What mytool does."  # or a `// ...` header at the top of my_tool.ts
# transforms = ["template"] # opt into @@VAR@@ env substitution (static, needs `shine upgrade`)
# env = ["API_URL", "SERVICE_TOKEN=API_TOKEN"]  # inject shine values at launch; read via Bun.env
"#;

pub async fn handle_init_template(force: bool) -> Result<()> {
    let dir = std::env::current_dir().context("reading current directory")?;
    let (path, overwritten) =
        utils::init_template::write_shine_toml_template(&dir, force, SHELL_TEMPLATE)?;
    if overwritten {
        println!("Updated shell preset template: {}", path.display());
    } else {
        println!("Created shell preset template: {}", path.display());
    }
    Ok(())
}

pub async fn handle_install(config: &Config, category: Option<&str>, force: bool) -> Result<()> {
    crate::config::print_presets_note(config);
    let prefix = match category {
        Some(cat) => format!("shell/{cat}"),
        None => "shell".to_string(),
    };

    // When using the default presets directory, extract the embedded assets first.
    if !config.is_external_presets {
        let report = crate::presets::extract_prefix(&prefix, config.presets_dir(), force).await?;
        output::summary_line("Shell Presets", &preset_extract_summary_parts(&report));
    }

    let categories = metadata::load_installed_categories(config, category).await?;
    // Build (template_source, rendered_dest) pairs for all scripts.
    // apply_template_to_scripts renders source → rendered_dir, never modifies presets_dir.
    let script_pairs = build_script_pairs(config, &categories);

    // Apply env-variable substitution to scripts that opt in via `# shine-template: true`.
    // Output goes to rendered_dir; presets_dir templates are left untouched.
    apply_template_to_scripts(config, &script_pairs).await?;

    // Symlinks point to the rendered file when one was produced, otherwise to the
    // raw source in presets_dir (non-template scripts).
    let link_specs = build_link_specs(config, &categories);
    let link_report =
        crate::bin_links::link_executables_with_names(config.bin_dir(), &link_specs, force).await?;

    output::summary_line("Bin Links", &link_report_summary_parts(&link_report));
    print_link_conflicts(config, &link_report.conflicts, category);

    let source_commands = installed_source_commands(config).await?;
    let installed_commands = installed_source_commands_for_categories(config, &categories).await?;

    let shell_config_path = get_shell_config_path(&config.shell_type, &config.home_dir)?;
    let shell_update = append_path_to_shell_config(config, force, &source_commands).await?;
    let profile_path = managed_shell_profile_path(config);
    if shell_update.profile_updated {
        output::detail_line(
            "Shell Profile",
            &colors::green("updated"),
            Some(profile_path.display().to_string()),
        );
    }
    match shell_update.config_status {
        PathUpdateStatus::AlreadyConfigured => {
            output::detail_line(
                "Shell Config",
                &colors::dim("up to date"),
                Some(shell_config_path.display().to_string()),
            );
        }
        PathUpdateStatus::Updated(path) => {
            output::detail_line(
                "Shell Config",
                &colors::green("updated"),
                Some(path.display().to_string()),
            );
        }
    }
    print_source_command_activation_hint(config, &shell_config_path, &installed_commands);
    Ok(())
}

pub async fn handle_upgrade_installed(
    config: &Config,
    verbose: bool,
    sep: &mut crate::output::SectionSeparator,
) -> Result<ShellUpgradeReport> {
    let all_categories = if config.is_external_presets {
        metadata::load_installed_categories(config, None).await?
    } else {
        metadata::load_embedded_categories(None)?
    };

    let installed_commands: Vec<(String, String)> = all_categories
        .iter()
        .flat_map(|cat| {
            cat.files.iter().filter_map(|file| {
                let link = crate::bin_links::command_path_for_name(
                    config.bin_dir(),
                    std::ffi::OsStr::new(&file.command_name),
                );
                shell_link_exists(&link).then(|| (cat.name.clone(), file.command_name.clone()))
            })
        })
        .collect();

    if installed_commands.is_empty() {
        if verbose {
            println!("{}", colors::dim("No installed shell presets found."));
        }
        return Ok(ShellUpgradeReport::default());
    }

    let installed_categories: std::collections::BTreeSet<String> = installed_commands
        .iter()
        .map(|(cat_name, _)| cat_name.clone())
        .collect();

    if !config.is_external_presets {
        for category in &installed_categories {
            let prefix = format!("shell/{category}");
            let _ = crate::presets::extract_prefix(&prefix, config.presets_dir(), true).await?;
        }
    }

    let categories = metadata::load_installed_categories(config, None).await?;
    let mut categories: Vec<_> = categories
        .into_iter()
        .filter(|cat| installed_categories.contains(&cat.name))
        .collect();
    for cat in &mut categories {
        cat.files.retain(|file| {
            installed_commands.contains(&(cat.name.clone(), file.command_name.clone()))
        });
    }

    let script_pairs = build_script_pairs(config, &categories);
    let template_report = apply_template_to_scripts(config, &script_pairs).await?;

    let link_specs = build_link_specs(config, &categories);
    let link_report =
        crate::bin_links::link_executables_with_names(config.bin_dir(), &link_specs, true).await?;

    let link_parts = upgrade_link_report_summary_parts(&link_report, verbose);

    let source_commands = installed_source_commands(config).await?;

    let shell_update = append_path_to_shell_config(config, false, &source_commands).await?;
    let updated_shell_config = match shell_update.config_status {
        PathUpdateStatus::AlreadyConfigured => None,
        PathUpdateStatus::Updated(path) => Some(path),
    };

    let has_visible_result = should_print_upgrade_section(
        verbose,
        !template_report.updated.is_empty(),
        !link_parts.is_empty(),
        updated_shell_config.is_some(),
    );
    if has_visible_result {
        sep.begin();
        if verbose {
            output::summary_line(
                "Shell Presets",
                &[colors::dim(&format!(
                    "{} installed categories",
                    installed_categories.len()
                ))],
            );
        } else {
            println!("{}", colors::bold("Shell Presets"));
        }

        for name in &template_report.updated {
            println!("  {} {}", colors::symbol("✓"), name);
        }
        if !link_parts.is_empty() {
            output::summary_line("Bin Links", &link_parts);
        }
        print_link_conflicts(config, &link_report.conflicts, None);
        if let Some(path) = &updated_shell_config {
            output::detail_line(
                "Shell Config",
                &colors::green("updated"),
                Some(path.display().to_string()),
            );
        }
    }

    Ok(ShellUpgradeReport {
        templates_updated: template_report.updated.len(),
        links_created: link_report.created.len(),
        links_updated: link_report.overwritten.len(),
        link_conflicts: link_report.conflicts.len(),
        path_changed: updated_shell_config.is_some(),
    })
}

fn should_print_upgrade_section(
    verbose: bool,
    templates_updated: bool,
    has_link_result: bool,
    path_changed: bool,
) -> bool {
    verbose || templates_updated || has_link_result || path_changed
}

pub async fn handle_completion_install(config: &Config) -> Result<()> {
    let source_commands = installed_source_commands(config).await?;
    let shell_config_path = get_shell_config_path(&config.shell_type, &config.home_dir)?;
    let shell_update = append_path_to_shell_config(config, false, &source_commands).await?;
    let profile_path = managed_shell_profile_path(config);

    if shell_update.profile_updated {
        output::detail_line(
            "Shell Profile",
            &colors::green("updated"),
            Some(profile_path.display().to_string()),
        );
    } else {
        output::detail_line(
            "Shell Profile",
            &colors::dim("up to date"),
            Some(profile_path.display().to_string()),
        );
    }

    match shell_update.config_status {
        PathUpdateStatus::AlreadyConfigured => {
            output::detail_line(
                "Shell Config",
                &colors::dim("up to date"),
                Some(shell_config_path.display().to_string()),
            );
        }
        PathUpdateStatus::Updated(path) => {
            output::detail_line(
                "Shell Config",
                &colors::green("updated"),
                Some(path.display().to_string()),
            );
        }
    }

    if !super::profile::supports_completion_registration(&config.shell_type) {
        let shell: &'static str = config.shell_type.into();
        output::detail_line(
            "Completion",
            &colors::yellow("unsupported"),
            Some(format!("{shell}; PATH setup was installed")),
        );
    }

    output::hint_line(
        "Next Step",
        &format!(
            "run `{}` once, or open a new shell",
            shell_source_command(&config.shell_type, &shell_config_path)
        ),
    );
    Ok(())
}

fn shell_link_exists(link: &Path) -> bool {
    link.exists()
        || std::fs::symlink_metadata(link)
            .map(|meta| meta.file_type().is_symlink())
            .unwrap_or(false)
}

/// For each script that declares `# shine-template: true`, read the template from
/// `source_path` (presets_dir — never modified), substitute env variables from
fn build_script_pairs(
    config: &Config,
    categories: &[metadata::ShellCategory],
) -> Vec<ScriptTemplate> {
    categories
        .iter()
        .flat_map(|cat| {
            cat.files.iter().map(|file| {
                let source =
                    config.preset_path(Path::new("shell").join(&cat.name).join(&file.source_rel));
                let rendered = config
                    .rendered_dir()
                    .join("shell")
                    .join(&cat.name)
                    .join(&file.source_rel);
                ScriptTemplate {
                    source_path: source,
                    rendered_path: rendered,
                    display_name: format!("{}/{}", cat.name, file.command_name),
                    transforms: file.transforms.clone(),
                }
            })
        })
        .collect()
}

pub(super) async fn installed_source_commands(config: &Config) -> Result<Vec<String>> {
    let categories = metadata::load_installed_categories(config, None).await?;
    installed_source_commands_for_categories(config, &categories).await
}

async fn installed_source_commands_for_categories(
    config: &Config,
    categories: &[metadata::ShellCategory],
) -> Result<Vec<String>> {
    let mut commands = categories
        .iter()
        .flat_map(|cat| cat.files.iter())
        .filter(|file| file.needs_source)
        .filter(|file| {
            let link = crate::bin_links::command_path_for_name(
                config.bin_dir(),
                std::ffi::OsStr::new(&file.command_name),
            );
            shell_link_exists(&link)
        })
        .map(|file| file.command_name.clone())
        .collect::<Vec<_>>();
    commands.sort();
    commands.dedup();
    Ok(commands)
}

#[cfg(test)]
mod tests {
    use super::super::ShellType;
    use super::super::uninstall::handle_uninstall;
    use super::*;
    use crate::config::Config;
    use std::path::PathBuf;
    use tokio::fs;

    #[test]
    fn upgrade_section_hides_no_op_by_default_and_shows_verbose_or_changes() {
        assert!(!should_print_upgrade_section(false, false, false, false));
        assert!(should_print_upgrade_section(true, false, false, false));
        assert!(should_print_upgrade_section(false, true, false, false));
        assert!(should_print_upgrade_section(false, false, true, false));
        assert!(should_print_upgrade_section(false, false, false, true));
    }

    async fn make_temp_dir() -> PathBuf {
        crate::test_support::make_temp_dir("shine-shell").await
    }

    #[cfg(unix)]
    async fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path).await.unwrap().permissions();
        perms.set_mode(perms.mode() | 0o111);
        fs::set_permissions(path, perms).await.unwrap();
    }

    fn wrapper_marker(command: &str, shell: &ShellType) -> String {
        match shell {
            ShellType::PowerShell => format!("\nfunction {command} {{ . (Join-Path $shineBin"),
            ShellType::Fish => format!("\nfunction {command}"),
            _ => format!("\n{command}() {{ source"),
        }
    }

    fn managed_profile_source_marker(shell: &ShellType) -> &'static str {
        match shell {
            ShellType::PowerShell => ". (Join-Path $HOME 'shell/profile.ps1')",
            ShellType::Fish => "source \"$HOME/shell/config.fish\"",
            ShellType::Bash | ShellType::Zsh | ShellType::Elvish => {
                "source \"$HOME/shell/profile.sh\""
            }
        }
    }

    fn managed_profile_path_marker(shell: &ShellType) -> &'static str {
        match shell {
            ShellType::PowerShell => "$shinePathEntries",
            ShellType::Fish => "fish_add_path",
            ShellType::Bash | ShellType::Zsh | ShellType::Elvish => "export PATH",
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn install_then_uninstall_roundtrip() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, None, false).await.unwrap();
        assert!(
            config
                .presets_dir()
                .join("shell/proxy/set_proxy.sh")
                .exists(),
            "preset should exist after install"
        );
        let first_bin_entry = fs::read_dir(config.bin_dir())
            .await
            .unwrap()
            .next_entry()
            .await
            .unwrap();
        assert!(
            first_bin_entry.is_some(),
            "bin dir should have symlinks after install"
        );
        // symlinks use stem names (no .sh suffix)
        assert!(
            config.bin_dir().join("setproxy").exists(),
            "bin link should use configured rename"
        );
        assert!(!config.bin_dir().join("set_proxy").exists());
        assert!(
            managed_shell_profile_path(&config).exists(),
            "managed shell profile should exist after install"
        );

        handle_uninstall(&config, None, false, false).await.unwrap();
        assert!(
            !config
                .presets_dir()
                .join("shell/proxy/set_proxy.sh")
                .exists(),
            "preset should be gone after uninstall"
        );
        let mut rd = fs::read_dir(config.bin_dir()).await.unwrap();
        assert!(
            rd.next_entry().await.unwrap().is_none(),
            "bin dir should be empty after uninstall"
        );
        assert!(
            !managed_shell_profile_path(&config).exists(),
            "managed shell profile should be removed after full uninstall"
        );

        // Idempotency: second uninstall must not error
        handle_uninstall(&config, None, false, false).await.unwrap();

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn append_writes_snippet_to_shell_config() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);

        append_path_to_shell_config(&config, false, &[])
            .await
            .unwrap();

        let config_path = get_shell_config_path(&config.shell_type, &config.home_dir).unwrap();
        let content = fs::read_to_string(&config_path).await.unwrap();
        assert!(
            content.contains(super::super::SENTINEL_START),
            "sentinel should be present"
        );
    }

    #[tokio::test]
    async fn completion_install_updates_profile_without_installing_presets() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);

        handle_completion_install(&config).await.unwrap();

        let profile = fs::read_to_string(managed_shell_profile_path(&config))
            .await
            .unwrap();
        let shell_name: &'static str = config.shell_type.into();
        assert!(
            profile.contains(&format!("COMPLETE={shell_name} shine")),
            "profile should register shine completion: {profile}"
        );
        assert!(
            !config.presets_dir().join("shell/proxy").exists(),
            "completion install must not extract or install shell presets"
        );
    }

    #[tokio::test]
    async fn append_is_idempotent() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);

        append_path_to_shell_config(&config, false, &[])
            .await
            .unwrap();
        append_path_to_shell_config(&config, false, &[])
            .await
            .unwrap();

        let config_path = get_shell_config_path(&config.shell_type, &config.home_dir).unwrap();
        let content = fs::read_to_string(&config_path).await.unwrap();
        let count = content.matches(super::super::SENTINEL_START).count();
        assert_eq!(count, 1, "sentinel should appear exactly once");
    }

    #[tokio::test]
    async fn append_is_idempotent_with_source_wrappers() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        let source_commands = vec!["setproxy".to_string(), "usetproxy".to_string()];

        append_path_to_shell_config(&config, false, &source_commands)
            .await
            .unwrap();
        append_path_to_shell_config(&config, false, &source_commands)
            .await
            .unwrap();

        let config_path = get_shell_config_path(&config.shell_type, &config.home_dir).unwrap();
        let content = fs::read_to_string(&config_path).await.unwrap();
        assert_eq!(
            content.matches(super::super::SENTINEL_START).count(),
            1,
            "sentinel should appear exactly once"
        );
        assert!(
            !content.contains("setproxy()"),
            "source wrappers should live in the managed profile: {content}"
        );

        let profile_path = managed_shell_profile_path(&config);
        let profile = fs::read_to_string(&profile_path).await.unwrap();
        let setproxy_marker = wrapper_marker("setproxy", &config.shell_type);
        let usetproxy_marker = wrapper_marker("usetproxy", &config.shell_type);
        assert_eq!(
            profile.matches(&setproxy_marker).count(),
            1,
            "setproxy wrapper should not be duplicated: {content}"
        );
        assert_eq!(
            profile.matches(&usetproxy_marker).count(),
            1,
            "usetproxy wrapper should not be duplicated: {content}"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn append_writes_source_entry_and_managed_profile() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        let source_commands = vec!["setproxy".to_string()];

        append_path_to_shell_config(&config, false, &source_commands)
            .await
            .unwrap();

        let config_path = get_shell_config_path(&config.shell_type, &config.home_dir).unwrap();
        let content = fs::read_to_string(&config_path).await.unwrap();
        assert!(
            content.contains(managed_profile_source_marker(&config.shell_type)),
            "shell config should only source managed profile: {content}"
        );
        assert!(
            !content.contains("export PATH"),
            "shell config should not contain direct PATH setup: {content}"
        );
        assert!(
            !content.contains("setproxy()"),
            "shell config should not contain direct wrapper functions: {content}"
        );

        let profile = fs::read_to_string(managed_shell_profile_path(&config))
            .await
            .unwrap();
        assert!(
            profile.contains(managed_profile_path_marker(&config.shell_type)),
            "managed profile should contain PATH setup: {profile}"
        );
        assert!(
            profile.contains(&wrapper_marker("setproxy", &config.shell_type)),
            "managed profile should contain source wrapper: {profile}"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn append_writes_both_windows_powershell_profiles() {
        let dir = make_temp_dir().await;
        let mut config = Config::new_for_test(&dir);
        config.shell_type = ShellType::PowerShell;
        let source_commands = vec!["setproxy".to_string(), "usetproxy".to_string()];

        append_path_to_shell_config(&config, false, &source_commands)
            .await
            .unwrap();

        let profile = fs::read_to_string(managed_shell_profile_path(&config))
            .await
            .unwrap();
        for config_path in
            super::super::get_shell_config_paths(&config.shell_type, &config.home_dir).unwrap()
        {
            let content = fs::read_to_string(&config_path).await.unwrap();
            assert!(
                content.contains(". (Join-Path $HOME 'shell/profile.ps1')"),
                "PowerShell profile should source managed shine profile from {}: {content}",
                config_path.display()
            );
        }
        assert!(
            profile.contains("function setproxy"),
            "managed PowerShell profile should contain setproxy wrapper: {profile}"
        );
        assert!(
            profile.contains("function usetproxy"),
            "managed PowerShell profile should contain usetproxy wrapper: {profile}"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn append_refreshes_stale_sentinel_with_managed_profile_source() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        let config_path = get_shell_config_path(&config.shell_type, &config.home_dir).unwrap();
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).await.unwrap();
        }
        let sentinel_start = super::super::SENTINEL_START;
        let sentinel_end = "# <<< shine <<<";
        fs::write(
            &config_path,
            format!(
                "before\n\n{sentinel_start}\nif [[ \":$PATH:\" != *\":$HOME/.shine/bin:\"* ]]; then\n  export PATH=\"$HOME/.shine/bin:$PATH\"\nfi\n{sentinel_end}\nafter\n"
            ),
        )
        .await
        .unwrap();

        let source_commands = vec!["setproxy".to_string(), "usetproxy".to_string()];
        let update = append_path_to_shell_config(&config, false, &source_commands)
            .await
            .unwrap();

        assert!(
            matches!(update.config_status, PathUpdateStatus::Updated(_)),
            "stale sentinel should be refreshed"
        );
        let content = fs::read_to_string(&config_path).await.unwrap();
        assert!(
            content.contains(managed_profile_source_marker(&config.shell_type)),
            "shell config should source managed profile: {content}"
        );
        assert!(
            !content.contains("export PATH"),
            "stale PATH setup should be removed from shell config: {content}"
        );
        assert!(
            !content.contains("setproxy()"),
            "source wrappers should not be added directly to shell config: {content}"
        );
        assert!(
            content.contains("before"),
            "non-managed content should be preserved"
        );
        assert!(
            content.contains("after"),
            "non-managed content should be preserved"
        );
        let profile = fs::read_to_string(managed_shell_profile_path(&config))
            .await
            .unwrap();
        let setproxy_marker = wrapper_marker("setproxy", &config.shell_type);
        let usetproxy_marker = wrapper_marker("usetproxy", &config.shell_type);
        assert!(
            profile.contains(&setproxy_marker),
            "setproxy wrapper should be added to managed profile: {profile}"
        );
        assert!(
            profile.contains(&usetproxy_marker),
            "usetproxy wrapper should be added to managed profile: {profile}"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn installed_source_commands_for_categories_are_scoped() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("agent"), false).await.unwrap();
        handle_install(&config, Some("proxy"), false).await.unwrap();

        let proxy_only = metadata::load_installed_categories(&config, Some("proxy"))
            .await
            .unwrap();
        let commands = installed_source_commands_for_categories(&config, &proxy_only)
            .await
            .unwrap();

        assert_eq!(
            commands,
            vec!["setproxy".to_string(), "usetproxy".to_string()]
        );
        assert!(!commands.contains(&"ccenv".to_string()));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn external_presets_install_links_disk_scripts_without_extraction() {
        let dir = make_temp_dir().await;
        // new_for_test sets presets_dir = dir/presets, bin_dir = dir/bin
        // Create a script in presets_dir/shell/custom/ to simulate user-managed presets.
        let cat_dir = dir.join("presets/shell/custom");
        fs::create_dir_all(&cat_dir).await.unwrap();
        let script = cat_dir.join("my_tool.sh");
        fs::write(&script, b"#!/bin/bash\n# My tool.\necho hi\n")
            .await
            .unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script).await.unwrap().permissions();
        perms.set_mode(perms.mode() | 0o111);
        fs::set_permissions(&script, perms).await.unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("custom"), false)
            .await
            .unwrap();

        // The script must NOT have been extracted from embedded assets into
        // presets_dir — the only file there is the one we created above.
        let count = {
            let mut rd = fs::read_dir(&cat_dir).await.unwrap();
            let mut n = 0u32;
            while rd.next_entry().await.unwrap().is_some() {
                n += 1;
            }
            n
        };
        assert_eq!(count, 1, "no embedded assets should have been extracted");

        // A bin symlink for the script should have been created.
        let link = config.bin_dir().join("my_tool");
        assert!(link.exists(), "bin symlink should point at disk script");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn external_presets_install_applies_metadata_rename() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/custom");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"set_proxy.sh\"\ntarget = \"setproxy\"\n",
        )
        .await
        .unwrap();
        let script = cat_dir.join("set_proxy.sh");
        fs::write(&script, b"#!/bin/bash\n# Set proxy.\necho hi\n")
            .await
            .unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script).await.unwrap().permissions();
        perms.set_mode(perms.mode() | 0o111);
        fs::set_permissions(&script, perms).await.unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("custom"), false)
            .await
            .unwrap();

        assert!(config.bin_dir().join("setproxy").exists());
        assert!(!config.bin_dir().join("set_proxy").exists());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn external_presets_install_links_non_executable_source_scripts() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/proxy");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"set_proxy.sh\"\ntarget = \"setproxy\"\nneeds_source = true\n[[files]]\nsource = \"uset_proxy.sh\"\ntarget = \"usetproxy\"\nneeds_source = true\n",
        )
        .await
        .unwrap();
        fs::write(
            &cat_dir.join("set_proxy.sh"),
            b"#!/bin/bash\n# Set proxy.\n",
        )
        .await
        .unwrap();
        fs::write(
            &cat_dir.join("uset_proxy.sh"),
            b"#!/bin/bash\n# Unset proxy.\n",
        )
        .await
        .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("proxy"), false).await.unwrap();

        assert!(config.bin_dir().join("setproxy").exists());
        assert!(config.bin_dir().join("usetproxy").exists());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn init_template_creates_parseable_shell_metadata() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/custom");
        fs::create_dir_all(&cat_dir).await.unwrap();

        let (path, overwritten) =
            utils::init_template::write_shine_toml_template(&cat_dir, false, SHELL_TEMPLATE)
                .unwrap();
        fs::write(
            cat_dir.join("my_tool.sh"),
            b"#!/bin/bash\n# My tool.\necho hi\n",
        )
        .await
        .unwrap();

        let config = Config::new_for_test(&dir);
        let categories = metadata::load_installed_categories(&config, Some("custom"))
            .await
            .unwrap();

        assert_eq!(path, cat_dir.join("shine.toml"));
        assert!(!overwritten);
        assert_eq!(categories.len(), 1);
        assert_eq!(
            categories[0].description.as_deref(),
            Some("My shell helper commands.")
        );
        assert_eq!(
            categories[0].files[0].source_rel,
            PathBuf::from("my_tool.sh")
        );
        assert_eq!(categories[0].files[0].command_name, "mytool");
        assert!(!categories[0].files[0].needs_source);

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn init_template_refuses_existing_file_unless_forced() {
        let dir = make_temp_dir().await;
        fs::write(dir.join("shine.toml"), b"old").await.unwrap();

        let err = utils::init_template::write_shine_toml_template(&dir, false, SHELL_TEMPLATE)
            .unwrap_err();
        assert!(
            err.to_string().contains("use --force to overwrite"),
            "unexpected error: {err:#}"
        );
        assert_eq!(fs::read(dir.join("shine.toml")).await.unwrap(), b"old");

        let (_path, overwritten) =
            utils::init_template::write_shine_toml_template(&dir, true, SHELL_TEMPLATE).unwrap();
        assert!(overwritten);
        let content = fs::read_to_string(dir.join("shine.toml")).await.unwrap();
        assert!(content.contains("target = \"mytool\""));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn template_render_error_does_not_link_raw_script() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/proxy");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"set_proxy.sh\"\ntarget = \"setproxy\"\nneeds_source = true\n",
        )
        .await
        .unwrap();
        let script = cat_dir.join("set_proxy.sh");
        fs::write(
            &script,
            b"#!/bin/bash\n# shine-template: true\necho @@PROXY_HOST@@\n",
        )
        .await
        .unwrap();
        make_executable(&script).await;

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();
        fs::write(config.rendered_dir(), b"not a directory")
            .await
            .unwrap();

        let err = handle_install(&config, Some("proxy"), false)
            .await
            .expect_err("install should fail when rendered_dir cannot be created");

        assert!(
            err.to_string()
                .contains("creating rendered script directory"),
            "unexpected error: {err:#}"
        );
        assert!(
            !config.bin_dir().join("setproxy").exists(),
            "failed render must not link the raw template script"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn embedded_agent_installs_bun_launcher_without_rendering_credentials() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("agent"), false).await.unwrap();

        let source = config.presets_dir().join("shell/agent/cc.ts");
        assert!(source.exists());
        assert!(!config.rendered_dir().join("shell/agent/cc.ts").exists());
        let launcher = crate::bin_links::command_path_for_name(
            config.bin_dir(),
            std::ffi::OsStr::new("ccenv"),
        );
        let launcher_content = fs::read_to_string(&launcher).await.unwrap();
        assert!(launcher_content.contains("shine-managed"));
        assert!(launcher_content.contains(&source.display().to_string()));
        assert!(launcher_content.contains("bun"));

        let source_commands = installed_source_commands(&config).await.unwrap();
        assert!(!source_commands.contains(&"ccenv".to_string()));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn external_presets_upgrade_does_not_install_preset_only_scripts() {
        let dir = make_temp_dir().await;
        let proxy_dir = dir.join("presets/shell/proxy");
        let extra_dir = dir.join("presets/shell/extra");
        fs::create_dir_all(&proxy_dir).await.unwrap();
        fs::create_dir_all(&extra_dir).await.unwrap();

        fs::write(
            proxy_dir.join("shine.toml"),
            b"[[files]]\nsource = \"set_proxy.sh\"\ntarget = \"setproxy\"\nneeds_source = true\n",
        )
        .await
        .unwrap();
        let setproxy = proxy_dir.join("set_proxy.sh");
        fs::write(
            &setproxy,
            b"#!/bin/bash\n# shine-template: true\necho @@PROXY_HOST@@\n",
        )
        .await
        .unwrap();
        make_executable(&setproxy).await;

        let extra_tool = extra_dir.join("extra_tool.sh");
        fs::write(&extra_tool, b"#!/bin/bash\n# Extra tool.\necho extra\n")
            .await
            .unwrap();
        make_executable(&extra_tool).await;

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("proxy"), false).await.unwrap();
        assert!(config.bin_dir().join("setproxy").exists());
        assert!(
            !config.bin_dir().join("extra_tool").exists(),
            "extra preset should start as present but not installed"
        );

        fs::write(
            &setproxy,
            b"#!/bin/bash\n# shine-template: true\necho changed @@PROXY_HOST@@\n",
        )
        .await
        .unwrap();
        make_executable(&setproxy).await;

        let mut sep = crate::output::SectionSeparator::new();
        let report = handle_upgrade_installed(&config, false, &mut sep)
            .await
            .unwrap();

        assert_eq!(
            report.templates_updated, 1,
            "changed shell template should be reported under shell presets"
        );
        assert!(config.bin_dir().join("setproxy").exists());
        assert!(
            !config.bin_dir().join("extra_tool").exists(),
            "upgrade must not install preset-only scripts"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn external_bun_preset_installs_launcher_and_uninstall_removes_it() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/custom");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"tool.ts\"\ntarget = \"mytool\"\nruntime = \"bun\"\n",
        )
        .await
        .unwrap();
        // A non-executable .ts source: bun launchers do not require the exec bit.
        fs::write(cat_dir.join("tool.ts"), b"console.log('hi')\n")
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("custom"), false)
            .await
            .unwrap();

        let launcher = config.bin_dir().join("mytool");
        assert!(launcher.exists(), "bun launcher should be installed");
        assert!(!launcher.is_symlink(), "bun launcher is a regular file");
        let content = fs::read_to_string(&launcher).await.unwrap();
        assert!(content.contains("exec bun"));
        assert!(content.contains(&cat_dir.join("tool.ts").display().to_string()));
        assert!(
            !config.bin_dir().join("tool").exists(),
            "command should use the target rename, not the .ts stem"
        );

        handle_uninstall(&config, Some("custom"), false, false)
            .await
            .unwrap();
        assert!(
            !launcher.exists(),
            "managed bun launcher must be removed on uninstall"
        );
        assert!(
            cat_dir.join("tool.ts").exists(),
            "external source must be preserved"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn external_bun_preset_with_env_wraps_launcher_in_shine_env_run() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/custom");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"tool.ts\"\ntarget = \"mytool\"\nruntime = \"bun\"\nenv = [\"API_URL\", \"SERVICE_TOKEN=API_TOKEN\"]\n",
        )
        .await
        .unwrap();
        fs::write(cat_dir.join("tool.ts"), b"console.log(Bun.env.API_URL)\n")
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("custom"), false)
            .await
            .unwrap();

        let launcher = fs::read_to_string(config.bin_dir().join("mytool"))
            .await
            .unwrap();
        assert!(launcher.contains("command -v shine"));
        assert!(launcher.contains(
            "exec shine env run --no-workspace --with 'API_URL' --with 'SERVICE_TOKEN=API_TOKEN' -- bun "
        ));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn external_bun_preset_with_template_transform_targets_rendered_copy() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/custom");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"tool.ts\"\ntarget = \"mytool\"\nruntime = \"bun\"\ntransforms = [\"template\"]\n",
        )
        .await
        .unwrap();
        fs::write(cat_dir.join("tool.ts"), b"const host = '@@PROXY_HOST@@'\n")
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        config
            .env
            .insert("PROXY_HOST".into(), "proxy.example".into());
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("custom"), false)
            .await
            .unwrap();

        let rendered = config.rendered_dir().join("shell/custom/tool.ts");
        assert!(
            rendered.exists(),
            "template transform should render the .ts"
        );
        assert!(
            fs::read_to_string(&rendered)
                .await
                .unwrap()
                .contains("proxy.example"),
            "rendered bun script should have @@PROXY_HOST@@ substituted"
        );
        let launcher = fs::read_to_string(config.bin_dir().join("mytool"))
            .await
            .unwrap();
        assert!(
            launcher.contains(&rendered.display().to_string()),
            "launcher must target the rendered copy: {launcher}"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }
}
