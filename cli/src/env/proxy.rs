//! PATH shims that inject a deliberately small allow-list of shine env values.

use super::{EnvConfig, parse_env_specs, resolve_stored_value};
use crate::config::{Config, EnvProxyRule};
use crate::{persist::atomic_write, secret, shell_quote};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
};
use tokio::process::Command;

const MARKER: &str = "shine-env-proxy";

#[derive(Default, Serialize, Deserialize)]
struct ProxyManifest {
    entries: BTreeMap<String, PathBuf>,
}

pub async fn install(config: &Config, command: &str, with: &[String], project: bool) -> Result<()> {
    validate_command(command)?;
    if command.eq_ignore_ascii_case("shine") || command.eq_ignore_ascii_case("shine.exe") {
        bail!("cannot proxy Shine itself: the launcher would recursively invoke itself");
    }
    parse_env_specs(with)?;
    if project && !config.is_project_config() {
        bail!("--project requires a shine.config.toml in the current directory or an ancestor");
    }
    let target = find_target(command, config.bin_dir())?;
    let path = if project {
        config.config_path()
    } else {
        &config.shine_dir().join("config.toml")
    };
    install_shim(config.bin_dir(), command, &target).await?;
    upsert_rule(
        path,
        EnvProxyRule {
            command: command.into(),
            with: with.to_vec(),
            enabled: true,
        },
    )
    .await?;
    let mut manifest = load_manifest(config.shine_dir()).await?;
    manifest.entries.insert(command.into(), target.clone());
    save_manifest(config.shine_dir(), &manifest).await?;
    println!(
        "installed transparent proxy {command} -> {}",
        target.display()
    );
    Ok(())
}

pub async fn list(config: &Config) -> Result<()> {
    if config.env_proxy.is_empty() {
        println!("No transparent command proxies configured.");
    }
    for rule in &config.env_proxy {
        println!(
            "{}: {} ({})",
            rule.command,
            rule.with.join(", "),
            if rule.enabled { "enabled" } else { "disabled" }
        );
    }
    Ok(())
}

pub async fn set_enabled(
    config: &Config,
    command: &str,
    enabled: bool,
    project: bool,
) -> Result<()> {
    validate_command(command)?;
    if project && !config.is_project_config() {
        bail!("--project requires a shine.config.toml in the current directory or an ancestor");
    }
    let global_path = config.shine_dir().join("config.toml");
    let path = if project {
        config.config_path()
    } else {
        &global_path
    };
    let inherited = config
        .env_proxy
        .iter()
        .find(|rule| rule.command == command)
        .cloned();
    mutate_rules(path, |rules| {
        if let Some(rule) = rules.iter_mut().find(|rule| rule.command == command) {
            rule.enabled = enabled;
        } else if project {
            let mut rule = inherited.with_context(|| {
                format!("{command} is not configured as an env proxy in the active configuration")
            })?;
            rule.enabled = enabled;
            rules.push(rule);
        } else {
            bail!(
                "{command} is not configured as an env proxy in {}",
                path.display()
            );
        }
        Ok(())
    })
    .await?;
    println!(
        "{} transparent proxy {command}",
        if enabled { "enabled" } else { "disabled" }
    );
    Ok(())
}

pub async fn uninstall(config: &Config, command: &str) -> Result<()> {
    validate_command(command)?;
    uninstall_for_platform(config, command, cfg!(windows)).await
}

async fn uninstall_for_platform(config: &Config, command: &str, windows: bool) -> Result<()> {
    let path = config.shine_dir().join("config.toml");
    let mut manifest = load_manifest(config.shine_dir()).await?;
    let mut launchers = vec![config.bin_dir().join(command)];
    if windows {
        for ext in ["cmd", "ps1"] {
            launchers.push(config.bin_dir().join(format!("{command}.{ext}")));
        }
    }
    let mut owned = Vec::new();
    // Inspect every member before changing rules, receipts, or another launcher.
    for launcher in launchers {
        let metadata = match tokio::fs::symlink_metadata(&launcher).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).context("inspecting env proxy destination"),
        };
        if !metadata.is_file()
            || !tokio::fs::read_to_string(&launcher)
                .await
                .with_context(|| format!("reading {}", launcher.display()))?
                .contains(MARKER)
        {
            bail!(
                "refusing to remove {}: it is not a shine env proxy",
                launcher.display()
            );
        }
        owned.push(launcher);
    }
    remove_rule(&path, command).await?;
    for launcher in owned {
        tokio::fs::remove_file(launcher).await?;
    }
    manifest.entries.remove(command);
    save_manifest(config.shine_dir(), &manifest).await?;
    println!("removed transparent proxy {command}");
    Ok(())
}

pub async fn exec(config: &Config, target: &Path, command: &str, args: &[OsString]) -> Result<()> {
    let rule = config.env_proxy.iter().find(|rule| rule.command == command);
    if !target.is_file() {
        bail!(
            "proxy target {} no longer exists; rerun `shine env proxy install {command} --with ...`",
            target.display()
        );
    }
    // Project-local rules share a global launcher. Outside the project there
    // may be no applicable rule; the original command must still work.
    let Some(rule) = rule.filter(|rule| rule.enabled) else {
        return run_target(target, args, BTreeMap::new()).await;
    };
    let env = EnvConfig::load_or_init(config).await?;
    let mut injected = BTreeMap::new();
    for spec in parse_env_specs(&rule.with)? {
        let value = match resolve_stored_value(&env, &spec.source)? {
            super::StoredValue::Secret { key, value } => secret::decrypt_with_config(value, config)
                .await
                .with_context(|| format!("decrypting {key}"))?,
            super::StoredValue::Plaintext(value) => value.to_string(),
        };
        injected.insert(spec.target, value);
    }
    run_target(target, args, injected).await
}

async fn run_target(
    target: &Path,
    args: &[OsString],
    injected: BTreeMap<String, String>,
) -> Result<()> {
    let status = Command::new(target)
        .args(args)
        .envs(injected)
        .status()
        .await
        .with_context(|| format!("running proxy target {}", target.display()))?;
    if status.success() {
        return Ok(());
    }
    std::process::exit(status.code().unwrap_or(1));
}

fn validate_command(command: &str) -> Result<()> {
    if command.is_empty()
        || !command
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        || command == "."
        || command == ".."
    {
        bail!("proxy command must be a bare command name: {command}");
    }
    Ok(())
}

fn find_target(command: &str, shine_bin: &Path) -> Result<PathBuf> {
    let paths = std::env::var_os("PATH").context("PATH is not set")?;
    find_target_in_paths(command, shine_bin, std::env::split_paths(&paths))
}

fn find_target_in_paths(
    command: &str,
    shine_bin: &Path,
    paths: impl IntoIterator<Item = PathBuf>,
) -> Result<PathBuf> {
    let canonical_bin = std::fs::canonicalize(shine_bin).ok();
    for dir in paths {
        if dir == shine_bin
            || canonical_bin
                .as_ref()
                .is_some_and(|bin| std::fs::canonicalize(&dir).is_ok_and(|path| &path == bin))
        {
            continue;
        }
        let candidate = dir.join(command);
        if is_executable_target(&candidate) {
            // Do not canonicalize here. Cargo (and other rustup proxies) are
            // symlinks whose filename is their dispatch identity: resolving
            // `.../cargo` to `.../rustup` makes rustup see `argv[0] == rustup`
            // and reject Cargo's arguments. Keep the executable path exactly
            // as PATH selected it, merely making relative PATH segments absolute.
            return absolute_path(candidate);
        }
        #[cfg(windows)]
        {
            let candidate = dir.join(format!("{command}.exe"));
            if candidate.is_file() {
                return absolute_path(candidate);
            }
        }
    }
    bail!(
        "{command} is not installed on PATH outside {}",
        shine_bin.display()
    )
}

fn is_executable_target(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn absolute_path(path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(std::env::current_dir()
            .context("reading current directory")?
            .join(path))
    }
}

async fn install_shim(bin_dir: &Path, command: &str, target: &Path) -> Result<()> {
    install_shims_for_platform(bin_dir, command, target, cfg!(windows)).await
}

async fn install_shims_for_platform(
    bin_dir: &Path,
    command: &str,
    target: &Path,
    windows: bool,
) -> Result<()> {
    let path = bin_dir.join(command);
    let mut paths = vec![path.clone()];
    if windows {
        paths.push(bin_dir.join(format!("{command}.cmd")));
        paths.push(bin_dir.join(format!("{command}.ps1")));
    }
    // Check the entire launcher set before writing any member. Inspect links
    // themselves, including dangling links, rather than following their target.
    for candidate in &paths {
        let metadata = match tokio::fs::symlink_metadata(candidate).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).context("inspecting env proxy destination"),
        };
        if !metadata.is_file()
            || !tokio::fs::read_to_string(candidate)
                .await
                .with_context(|| format!("reading {}", candidate.display()))?
                .contains(MARKER)
        {
            bail!(
                "{} already exists and is not a shine env proxy",
                candidate.display()
            );
        }
    }
    tokio::fs::create_dir_all(bin_dir).await?;
    let target_string = target.to_string_lossy().into_owned();
    let target = shell_quote::single_quote(&target_string);
    let command_q = shell_quote::single_quote(command);
    atomic_write(&path, format!("#!/bin/sh\n# {MARKER}\nexec shine env proxy exec --target {target} {command_q} \"$@\"\n").as_bytes()).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).await?;
    }
    if windows {
        install_windows_shims(bin_dir, command, &target_string).await?;
    }
    Ok(())
}

async fn install_windows_shims(bin_dir: &Path, command: &str, target: &str) -> Result<()> {
    atomic_write(&bin_dir.join(format!("{command}.cmd")), format!("@echo off\r\nREM {MARKER}\r\nshine env proxy exec --target \"{target}\" {command} %*\r\n").as_bytes()).await?;
    let target_ps = target.replace('\'', "''");
    atomic_write(&bin_dir.join(format!("{command}.ps1")), format!("# {MARKER}\n& shine env proxy exec --target '{target_ps}' {command} @args\nexit $LASTEXITCODE\n").as_bytes()).await
}

async fn upsert_rule(path: &Path, rule: EnvProxyRule) -> Result<()> {
    mutate_rules(path, |rules| {
        rules.retain(|r| r.command != rule.command);
        rules.push(rule);
        Ok(())
    })
    .await
}
async fn remove_rule(path: &Path, command: &str) -> Result<()> {
    mutate_rules(path, |rules| {
        rules.retain(|r| r.command != command);
        Ok(())
    })
    .await
}
async fn mutate_rules(
    path: &Path,
    change: impl FnOnce(&mut Vec<EnvProxyRule>) -> Result<()>,
) -> Result<()> {
    let text = match tokio::fs::read_to_string(path).await {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let mut table: toml::Table =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let mut rules: Vec<EnvProxyRule> = table
        .get("env_proxy")
        .map(|v| v.clone().try_into())
        .transpose()?
        .unwrap_or_default();
    change(&mut rules)?;
    if rules.is_empty() {
        table.remove("env_proxy");
    } else {
        table.insert("env_proxy".into(), toml::Value::try_from(rules)?);
    }
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .with_context(|| format!("parsing {}", path.display()))?;
    shine_core::migration::sync_table(doc.as_table_mut(), &table);
    crate::persist::atomic_write_private(path, doc.to_string().as_bytes()).await
}

fn manifest_path(shine_dir: &Path) -> PathBuf {
    shine_dir.join("proxy-manifest.toml")
}

async fn load_manifest(shine_dir: &Path) -> Result<ProxyManifest> {
    let path = manifest_path(shine_dir);
    match tokio::fs::read_to_string(&path).await {
        Ok(contents) => {
            toml::from_str(&contents).with_context(|| format!("parsing {}", path.display()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ProxyManifest::default()),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

async fn save_manifest(shine_dir: &Path, manifest: &ProxyManifest) -> Result<()> {
    let path = manifest_path(shine_dir);
    atomic_write(&path, toml::to_string_pretty(manifest)?.as_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_command_must_be_a_bare_name() {
        assert!(validate_command("gh").is_ok());
        assert!(validate_command("tool-name").is_ok());
        assert!(validate_command("../gh").is_err());
        assert!(validate_command("a/b").is_err());
    }

    #[tokio::test]
    async fn proxy_install_rejects_shine_before_writing_state() {
        let dir = crate::test_support::make_temp_dir("shine-proxy-self").await;
        let config = Config::new_for_test(&dir);
        for name in ["shine", "ShInE", "shine.exe", "SHINE.EXE"] {
            let error = install(&config, name, &["TOKEN".into()], false)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("cannot proxy Shine itself"));
        }
        assert!(
            tokio::fs::read_dir(&dir)
                .await
                .unwrap()
                .next_entry()
                .await
                .unwrap()
                .is_none()
        );
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    async fn uninstall_fixture(dir: &Path) -> Config {
        let config = Config::new_for_test(dir);
        upsert_rule(
            config.config_path(),
            EnvProxyRule {
                command: "demo".into(),
                with: vec!["TOKEN".into()],
                enabled: true,
            },
        )
        .await
        .unwrap();
        let mut manifest = ProxyManifest::default();
        manifest.entries.insert("demo".into(), dir.join("real"));
        save_manifest(config.shine_dir(), &manifest).await.unwrap();
        install_shims_for_platform(config.bin_dir(), "demo", &dir.join("real"), true)
            .await
            .unwrap();
        config
    }

    #[tokio::test]
    async fn proxy_uninstall_preflights_every_launcher_and_preserves_state_on_conflict() {
        let dir = crate::test_support::make_temp_dir("shine-proxy-uninstall-conflict").await;
        for name in ["demo", "demo.cmd", "demo.ps1"] {
            let config = uninstall_fixture(&dir.join(name)).await;
            let foreign = config.bin_dir().join(name);
            tokio::fs::write(&foreign, b"user replacement")
                .await
                .unwrap();
            let config_before = tokio::fs::read(config.config_path()).await.unwrap();
            let manifest_before = tokio::fs::read(manifest_path(config.shine_dir()))
                .await
                .unwrap();
            let mut launchers = Vec::new();
            for candidate in ["demo", "demo.cmd", "demo.ps1"] {
                let path = config.bin_dir().join(candidate);
                launchers.push((path.clone(), tokio::fs::read(path).await.unwrap()));
            }
            assert!(uninstall_for_platform(&config, "demo", true).await.is_err());
            assert_eq!(
                tokio::fs::read(config.config_path()).await.unwrap(),
                config_before
            );
            assert_eq!(
                tokio::fs::read(manifest_path(config.shine_dir()))
                    .await
                    .unwrap(),
                manifest_before
            );
            for (path, bytes) in launchers {
                assert_eq!(tokio::fs::read(path).await.unwrap(), bytes);
            }
        }
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn proxy_uninstall_rejects_links_even_when_the_target_has_a_proxy_marker() {
        let dir = crate::test_support::make_temp_dir("shine-proxy-uninstall-link").await;
        for dangling in [false, true] {
            let config = uninstall_fixture(&dir.join(dangling.to_string())).await;
            let launcher = config.bin_dir().join("demo.ps1");
            tokio::fs::remove_file(&launcher).await.unwrap();
            let target = config.shine_dir().join("target");
            if !dangling {
                tokio::fs::write(&target, MARKER).await.unwrap();
            }
            tokio::fs::symlink(&target, &launcher).await.unwrap();
            let before = tokio::fs::read(config.config_path()).await.unwrap();
            assert!(uninstall_for_platform(&config, "demo", true).await.is_err());
            assert_eq!(tokio::fs::read(config.config_path()).await.unwrap(), before);
            assert_eq!(tokio::fs::read_link(launcher).await.unwrap(), target);
            assert!(config.bin_dir().join("demo").is_file());
            assert!(
                load_manifest(config.shine_dir())
                    .await
                    .unwrap()
                    .entries
                    .contains_key("demo")
            );
        }
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn proxy_uninstall_removes_owned_launchers_rule_and_receipt() {
        let dir = crate::test_support::make_temp_dir("shine-proxy-uninstall-owned").await;
        let config = uninstall_fixture(&dir).await;
        uninstall_for_platform(&config, "demo", true).await.unwrap();
        for name in ["demo", "demo.cmd", "demo.ps1"] {
            assert!(!config.bin_dir().join(name).exists());
        }
        let table: toml::Table = toml::from_str(
            &tokio::fs::read_to_string(config.config_path())
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(!table.contains_key("env_proxy"));
        assert!(
            load_manifest(config.shine_dir())
                .await
                .unwrap()
                .entries
                .is_empty()
        );
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[test]
    fn absolute_target_preserves_symlink_dispatch_name() {
        let relative = PathBuf::from("bin/cargo");
        let resolved = absolute_path(relative).unwrap();
        assert!(resolved.ends_with("bin/cargo"));
        assert!(!resolved.ends_with("rustup"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn target_discovery_skips_bin_alias_without_resolving_executable_name() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::test_support::make_temp_dir("shine-proxy-paths").await;
        let bin = dir.join("bin");
        let real = dir.join("real");
        tokio::fs::create_dir_all(&bin).await.unwrap();
        tokio::fs::create_dir_all(&real).await.unwrap();
        tokio::fs::write(bin.join("cargo"), MARKER).await.unwrap();
        tokio::fs::write(real.join("rustup"), b"real executable")
            .await
            .unwrap();
        tokio::fs::set_permissions(real.join("rustup"), std::fs::Permissions::from_mode(0o755))
            .await
            .unwrap();
        tokio::fs::symlink(real.join("rustup"), real.join("cargo"))
            .await
            .unwrap();
        let alias = dir.join("bin-alias");
        tokio::fs::symlink(&bin, &alias).await.unwrap();
        assert!(find_target_in_paths("cargo", &bin, [alias.clone()]).is_err());
        assert_eq!(
            find_target_in_paths("cargo", &bin, [alias, real.clone()]).unwrap(),
            real.join("cargo")
        );
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn target_discovery_skips_non_executable_files_and_links() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::test_support::make_temp_dir("shine-proxy-executable").await;
        let shadow = dir.join("shadow");
        let real = dir.join("real");
        tokio::fs::create_dir_all(&shadow).await.unwrap();
        tokio::fs::create_dir_all(&real).await.unwrap();
        tokio::fs::write(shadow.join("tool"), b"not executable")
            .await
            .unwrap();
        tokio::fs::set_permissions(shadow.join("tool"), std::fs::Permissions::from_mode(0o644))
            .await
            .unwrap();
        tokio::fs::write(real.join("tool"), b"executable")
            .await
            .unwrap();
        tokio::fs::set_permissions(real.join("tool"), std::fs::Permissions::from_mode(0o755))
            .await
            .unwrap();
        let bin = dir.join("bin");
        assert!(find_target_in_paths("tool", &bin, [shadow.clone()]).is_err());
        assert_eq!(
            find_target_in_paths("tool", &bin, [shadow.clone(), real.clone()]).unwrap(),
            real.join("tool")
        );
        tokio::fs::symlink(shadow.join("tool"), shadow.join("linked"))
            .await
            .unwrap();
        assert!(find_target_in_paths("linked", &bin, [shadow]).is_err());
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn proxy_without_active_rule_runs_original_command() {
        let dir = crate::test_support::make_temp_dir("shine-proxy-no-rule").await;
        let config = Config::new_for_test(&dir);
        let output = dir.join("executed");
        exec(
            &config,
            Path::new("/bin/sh"),
            "project-command",
            &[
                "-c".into(),
                "printf reached > \"$1\"".into(),
                "sh".into(),
                output.clone().into_os_string(),
            ],
        )
        .await
        .unwrap();
        assert_eq!(tokio::fs::read(&output).await.unwrap(), b"reached");
        assert!(
            exec(&config, &dir.join("missing"), "project-command", &[])
                .await
                .is_err()
        );
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn rule_mutation_replaces_only_matching_command() {
        let dir = crate::test_support::make_temp_dir("shine-env-proxy").await;
        let path = dir.join("config.toml");
        tokio::fs::write(
            &path,
            "[[env_proxy]]\ncommand = \"gh\"\nwith = [\"OLD\"]\n\n[[env_proxy]]\ncommand = \"docker\"\nwith = [\"DOCKER_TOKEN\"]\n",
        )
        .await
        .unwrap();
        upsert_rule(
            &path,
            EnvProxyRule {
                command: "gh".into(),
                with: vec!["GH_TOKEN".into()],
                enabled: true,
            },
        )
        .await
        .unwrap();
        let parsed: toml::Table =
            toml::from_str(&tokio::fs::read_to_string(&path).await.unwrap()).unwrap();
        let rules: Vec<EnvProxyRule> = parsed["env_proxy"].clone().try_into().unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(
            rules.iter().find(|rule| rule.command == "gh").unwrap().with,
            ["GH_TOKEN"]
        );
        assert!(rules.iter().any(|rule| rule.command == "docker"));
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[test]
    fn legacy_rule_defaults_to_enabled() {
        let rule: EnvProxyRule =
            toml::from_str("command = \"gh\"\nwith = [\"GH_TOKEN\"]\n").unwrap();
        assert!(rule.enabled);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn proxy_rule_mutations_keep_config_private_and_preserve_env() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::test_support::make_temp_dir("shine-proxy-private").await;
        for name in ["config.toml", "shine.config.toml"] {
            let path = dir.join(name);
            crate::persist::atomic_write_private(
                &path,
                b"# retained\n[env]\nTOKEN = 'synthetic-value'\n",
            )
            .await
            .unwrap();
            for action in 0..3 {
                match action {
                    0 => upsert_rule(
                        &path,
                        EnvProxyRule {
                            command: "demo".into(),
                            with: vec!["TOKEN".into()],
                            enabled: true,
                        },
                    )
                    .await
                    .unwrap(),
                    1 => mutate_rules(&path, |rules| {
                        rules[0].enabled = false;
                        Ok(())
                    })
                    .await
                    .unwrap(),
                    _ => remove_rule(&path, "demo").await.unwrap(),
                }
                assert_eq!(
                    tokio::fs::metadata(&path)
                        .await
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600,
                );
                let contents = tokio::fs::read_to_string(&path).await.unwrap();
                assert!(contents.contains("# retained"));
                let table: toml::Table = toml::from_str(&contents).unwrap();
                assert_eq!(table["env"]["TOKEN"].as_str(), Some("synthetic-value"));
            }
        }
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn windows_proxy_preflights_all_launchers_before_writing() {
        let dir = crate::test_support::make_temp_dir("shine-proxy-conflict").await;
        for occupied in ["demo", "demo.cmd", "demo.ps1"] {
            let bin = dir.join(occupied);
            tokio::fs::create_dir_all(&bin).await.unwrap();
            let foreign = bin.join(occupied);
            tokio::fs::write(&foreign, b"user launcher").await.unwrap();
            assert!(
                install_shims_for_platform(&bin, "demo", &dir.join("real-demo"), true)
                    .await
                    .is_err()
            );
            assert_eq!(tokio::fs::read(&foreign).await.unwrap(), b"user launcher");
            for candidate in ["demo", "demo.cmd", "demo.ps1"] {
                assert_eq!(bin.join(candidate).exists(), candidate == occupied);
            }
        }
        let bin = dir.join("owned");
        install_shims_for_platform(&bin, "demo", &dir.join("first"), true)
            .await
            .unwrap();
        let before = tokio::fs::read(bin.join("demo")).await.unwrap();
        tokio::fs::write(bin.join("demo.ps1"), b"user replacement")
            .await
            .unwrap();
        assert!(
            install_shims_for_platform(&bin, "demo", &dir.join("second"), true)
                .await
                .is_err()
        );
        assert_eq!(tokio::fs::read(bin.join("demo")).await.unwrap(), before);
        tokio::fs::remove_file(bin.join("demo.ps1")).await.unwrap();
        install_shims_for_platform(&bin, "demo", &dir.join("second"), true)
            .await
            .unwrap();
        for name in ["demo", "demo.cmd", "demo.ps1"] {
            assert!(
                tokio::fs::read_to_string(bin.join(name))
                    .await
                    .unwrap()
                    .contains("second")
            );
        }
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn proxy_refuses_dangling_launcher_links() {
        let dir = crate::test_support::make_temp_dir("shine-proxy-link").await;
        let missing = dir.join("missing");
        tokio::fs::symlink(&missing, dir.join("demo.cmd"))
            .await
            .unwrap();
        assert!(
            install_shims_for_platform(&dir, "demo", &dir.join("real"), true)
                .await
                .is_err()
        );
        assert_eq!(
            tokio::fs::read_link(dir.join("demo.cmd")).await.unwrap(),
            missing
        );
        assert!(!dir.join("demo").exists());
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }
}
