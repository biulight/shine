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
    let path = config.shine_dir().join("config.toml");
    remove_rule(&path, command).await?;
    let mut manifest = load_manifest(config.shine_dir()).await?;
    let shim = config.bin_dir().join(command);
    if shim.is_file() {
        let body = tokio::fs::read_to_string(&shim).await.unwrap_or_default();
        if body.contains(MARKER) {
            tokio::fs::remove_file(&shim).await?;
        } else {
            bail!(
                "refusing to remove {}: it is not a shine env proxy",
                shim.display()
            );
        }
    }
    #[cfg(windows)]
    for ext in ["cmd", "ps1"] {
        let candidate = config.bin_dir().join(format!("{command}.{ext}"));
        if candidate.is_file() {
            let body = tokio::fs::read_to_string(&candidate)
                .await
                .unwrap_or_default();
            if body.contains(MARKER) {
                tokio::fs::remove_file(candidate).await?;
            }
        }
    }
    manifest.entries.remove(command);
    save_manifest(config.shine_dir(), &manifest).await?;
    println!("removed transparent proxy {command}");
    Ok(())
}

pub async fn exec(config: &Config, target: &Path, command: &str, args: &[OsString]) -> Result<()> {
    let rule = config
        .env_proxy
        .iter()
        .find(|rule| rule.command == command)
        .with_context(|| {
            format!("{command} is not configured as a transparent env proxy in the active config")
        })?;
    if !target.is_file() {
        bail!(
            "proxy target {} no longer exists; rerun `shine env proxy install {command} --with ...`",
            target.display()
        );
    }
    if !rule.enabled {
        return run_target(target, args, BTreeMap::new()).await;
    }
    let env = EnvConfig::load_or_init(config).await?;
    let mut injected = BTreeMap::new();
    for spec in parse_env_specs(&rule.with)? {
        let value = match resolve_stored_value(&env, &spec.source)? {
            super::StoredValue::Secret { key, value } => {
                secret::decrypt_secret(value, &config.age_identities())
                    .await
                    .with_context(|| format!("decrypting {key}"))?
            }
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
    for dir in std::env::split_paths(&paths) {
        if dir == shine_bin {
            continue;
        }
        let candidate = dir.join(command);
        if candidate.is_file() {
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
    tokio::fs::create_dir_all(bin_dir).await?;
    let path = bin_dir.join(command);
    if path.exists() {
        let body = tokio::fs::read_to_string(&path).await.unwrap_or_default();
        if !body.contains(MARKER) {
            bail!(
                "{} already exists and is not a shine env proxy",
                path.display()
            );
        }
    }
    let target_string = target.to_string_lossy().into_owned();
    let target = shell_quote::single_quote(&target_string);
    let command_q = shell_quote::single_quote(command);
    atomic_write(&path, format!("#!/bin/sh\n# {MARKER}\nexec shine env proxy exec --target {target} {command_q} \"$@\"\n").as_bytes()).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).await?;
    }
    #[cfg(windows)]
    {
        install_windows_shims(bin_dir, command, &target_string).await?;
    }
    Ok(())
}

#[cfg(windows)]
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
    let text = tokio::fs::read_to_string(path).await.unwrap_or_default();
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
    atomic_write(path, doc.to_string().as_bytes()).await
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

    #[test]
    fn absolute_target_preserves_symlink_dispatch_name() {
        let relative = PathBuf::from("bin/cargo");
        let resolved = absolute_path(relative).unwrap();
        assert!(resolved.ends_with("bin/cargo"));
        assert!(!resolved.ends_with("rustup"));
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
}
