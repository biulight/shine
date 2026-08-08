//! Handlers for `shine env list/set/delete/get/decrypt/export/encrypt`.

use anyhow::{Context, Result, bail};

use super::{EnvConfig, StoredValue, resolve_stored_value, secret_key};
use crate::config::{Config, EnvOverrideKind, EnvOverrideSource};
use crate::secret::{BackendKind, EncryptRecipients};
use crate::{colors, path_display, secret, shells};

/// Which layer supplied a variable's effective value, used to group the
/// `env list` output. `Config` is the `config.toml [env]` table (global or
/// project, deliberately not distinguished); the rest are `shine.env.toml`
/// override files. Ordering matches display order (`config.toml` first, then
/// override layers low-to-high by precedence).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EnvSourceGroup {
    Config,
    Global,
    Overlay { managed: bool },
    Project,
}

impl EnvSourceGroup {
    /// Fixed display order. Lower sorts first.
    fn order(self) -> u8 {
        match self {
            EnvSourceGroup::Config => 0,
            EnvSourceGroup::Global => 1,
            EnvSourceGroup::Overlay { .. } => 2,
            EnvSourceGroup::Project => 3,
        }
    }

    /// The bold header label for this section.
    fn label(self) -> &'static str {
        match self {
            EnvSourceGroup::Config => "config.toml",
            EnvSourceGroup::Global => "global env file",
            EnvSourceGroup::Overlay { managed: false } => "overlay",
            EnvSourceGroup::Overlay { managed: true } => "overlay (managed)",
            EnvSourceGroup::Project => "project env file",
        }
    }
}

/// Classify a key by its override source: no override → `config.toml [env]`,
/// otherwise the override file's layer (folding `is_managed_overlay` into the
/// `Overlay` variant).
fn env_source_group(source: Option<&EnvOverrideSource>) -> EnvSourceGroup {
    match source {
        None => EnvSourceGroup::Config,
        Some(source) => match source.kind {
            EnvOverrideKind::Global => EnvSourceGroup::Global,
            EnvOverrideKind::Overlay => EnvSourceGroup::Overlay {
                managed: source.is_managed_overlay,
            },
            EnvOverrideKind::Project => EnvSourceGroup::Project,
        },
    }
}

/// Partition `keys` into ordered, non-empty sections by source group,
/// preserving each key's relative order within its group. Pure over a
/// `source_of` lookup so it can be unit-tested without terminal/config I/O.
fn group_env_keys<'a>(
    keys: impl Iterator<Item = &'a str>,
    source_of: impl Fn(&str) -> Option<&'a EnvOverrideSource>,
) -> Vec<(EnvSourceGroup, Vec<&'a str>)> {
    let mut groups: Vec<(EnvSourceGroup, Vec<&'a str>)> = Vec::new();
    for key in keys {
        let group = env_source_group(source_of(key));
        match groups.iter_mut().find(|(g, _)| *g == group) {
            Some((_, members)) => members.push(key),
            None => groups.push((group, vec![key])),
        }
    }
    groups.sort_by_key(|(group, _)| group.order());
    groups
}

pub async fn handle_list(config: &Config, reveal: bool) -> Result<()> {
    let env = EnvConfig::load_or_init(config).await?;
    let catalog = super::catalog::load(config).await?;
    let terminal_width = usize::from(console::Term::stdout().size().1).max(40);
    let key_width = env
        .iter()
        .map(|(key, _)| key.chars().count())
        .max()
        .unwrap_or(0);

    println!("{}", colors::bold("Environment"));
    println!();
    if env.as_map().is_empty() {
        println!("  {}", colors::dim("No variables configured."));
        println!();
    }

    let groups = group_env_keys(env.iter().map(|(k, _)| k), |key| {
        config.env_override_source(key)
    });
    for (group, keys) in groups {
        // Header: bold label, plus the override file's path for non-config groups.
        match keys.first().and_then(|key| config.env_override_source(key)) {
            Some(source) => println!(
                "{}  {}",
                colors::bold(group.label()),
                colors::dim(&path_display::format(&source.path))
            ),
            None => println!("{}", colors::bold(group.label())),
        }
        for k in keys {
            let v = env.get(k).unwrap_or_default();
            let metadata = catalog.get(k);
            let description = env
                .description(k)
                .or_else(|| metadata.map(|item| item.description.as_str()))
                .unwrap_or_default();
            let sensitive = metadata.is_some_and(|item| item.sensitive) || is_sensitive_env_key(k);
            let display_value = display_env_value(v, sensitive, reveal);
            let (display_value, description) =
                fit_env_row(&display_value, description, key_width, terminal_width);
            let key_padding = " ".repeat(key_width.saturating_sub(k.chars().count()));
            if description.is_empty() {
                println!("  {}{}  {}", colors::cyan(k), key_padding, display_value);
            } else {
                println!(
                    "  {}{}  {:<value_width$}  {}",
                    colors::cyan(k),
                    key_padding,
                    display_value,
                    colors::dim(&description),
                    value_width = env_value_width(key_width, terminal_width),
                );
            }
        }
        println!();
    }

    println!(
        "  {}  {}",
        colors::dim("Config"),
        colors::dim(&path_display::format(config.config_path()))
    );
    println!(
        "  {}",
        colors::dim(&format!("{} variables", env.as_map().len()))
    );
    Ok(())
}

fn is_sensitive_env_key(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    [
        "SECRET",
        "TOKEN",
        "PASSWORD",
        "PASSPHRASE",
        "API_KEY",
        "PRIVATE_KEY",
        "ACCESS_KEY",
        "SUBSCRIPTION_URL",
    ]
    .iter()
    .any(|suffix| key == *suffix || key.ends_with(&format!("_{suffix}")))
}

fn display_env_value(value: &str, sensitive: bool, reveal: bool) -> String {
    if value.is_empty() {
        "<empty>".to_string()
    } else if sensitive && !reveal {
        "<redacted>".to_string()
    } else {
        value.to_string()
    }
}

fn env_value_width(key_width: usize, terminal_width: usize) -> usize {
    terminal_width.saturating_sub(key_width + 28).clamp(12, 36)
}

fn fit_env_row(
    value: &str,
    description: &str,
    key_width: usize,
    terminal_width: usize,
) -> (String, String) {
    let value_width = env_value_width(key_width, terminal_width);
    let value = truncate_text(value, value_width);
    let description_width = terminal_width.saturating_sub(2 + key_width + 2 + value_width + 2);
    let description = if description_width < 8 {
        String::new()
    } else {
        truncate_text(description, description_width)
    };
    (value, description)
}

fn truncate_text(value: &str, max_width: usize) -> String {
    if value.chars().count() <= max_width {
        return value.to_string();
    }
    if max_width <= 1 {
        return "…".to_string();
    }
    let mut result = value.chars().take(max_width - 1).collect::<String>();
    result.push('…');
    result
}

/// Where an `env set`/`encrypt`/`delete` write should land: `config.toml [env]`
/// (the default, unshadowed case), or a specific override file that already
/// supplies the key's effective value.
#[derive(Debug)]
enum EnvWriteTarget<'a> {
    ConfigToml,
    OverrideFile(&'a crate::config::EnvOverrideSource),
}

/// Decide where a write to `key` should go. Refuses (unless `force`) when an
/// override file already shadows `config.toml [env]` for this key, since a
/// plain write there would silently have no effect on the resolved value. With
/// `force`, warns loudly when the winning file is the shine-managed overlay
/// mirror, since that write will be discarded on the next `shine preset pull`.
fn resolve_env_write_target<'a>(
    config: &'a Config,
    key: &str,
    force: bool,
) -> Result<EnvWriteTarget<'a>> {
    let Some(source) = config.env_override_source(key) else {
        return Ok(EnvWriteTarget::ConfigToml);
    };
    if !force {
        bail!(
            "{key} currently resolves from {} (an env override file), which takes precedence over {}; this write would have no effect.\nRe-run with --force to write directly into that file instead.",
            path_display::format(&source.path),
            path_display::format(config.config_path()),
        );
    }
    if source.is_managed_overlay {
        eprintln!(
            "{}",
            colors::yellow(&format!(
                "Warning: {} is the shine-managed overlay mirror; this change will be discarded on the next `shine preset pull`/`shine update`. Edit it upstream on the maintaining device instead.",
                path_display::format(&source.path)
            ))
        );
    }
    Ok(EnvWriteTarget::OverrideFile(source))
}

pub async fn handle_set(config: &Config, key: &str, value: &str, force: bool) -> Result<()> {
    let catalog = super::catalog::load(config).await?;
    let sensitive =
        catalog.get(key).is_some_and(|item| item.sensitive) || is_sensitive_env_key(key);
    let display_value = display_env_value(value, sensitive, false);
    match resolve_env_write_target(config, key, force)? {
        EnvWriteTarget::ConfigToml => {
            let mut env = EnvConfig::load_or_init(config).await?;
            env.set(key, value);
            env.save(config).await?;
            println!(
                "{}",
                colors::green(&format!(
                    "set {key} = \"{display_value}\" in {}",
                    path_display::format(config.config_path())
                ))
            );
        }
        EnvWriteTarget::OverrideFile(source) => {
            crate::config::write_env_override_entry(&source.path, key, Some(value)).await?;
            println!(
                "{}",
                colors::green(&format!(
                    "set {key} = \"{display_value}\" in {}",
                    path_display::format(&source.path)
                ))
            );
        }
    }
    println!(
        "{}",
        colors::dim("Run `shine upgrade` to apply to already-installed presets.")
    );
    Ok(())
}

pub async fn handle_delete(config: &Config, key: &str, force: bool) -> Result<()> {
    if !config.env.contains_key(key) && config.env_override_source(key).is_none() {
        bail!("{key} is not set in the active config [env]");
    }
    match resolve_env_write_target(config, key, force)? {
        EnvWriteTarget::ConfigToml => {
            let mut env = EnvConfig::load_or_init(config).await?;
            env.remove(key);
            env.save(config).await?;
            println!(
                "{}",
                colors::green(&format!(
                    "deleted {key} from {}",
                    path_display::format(config.config_path())
                ))
            );
        }
        EnvWriteTarget::OverrideFile(source) => {
            crate::config::write_env_override_entry(&source.path, key, None).await?;
            println!(
                "{}",
                colors::green(&format!(
                    "deleted {key} from {}",
                    path_display::format(&source.path)
                ))
            );
        }
    }
    println!(
        "{}",
        colors::dim("Run `shine upgrade` to apply to already-installed presets.")
    );
    Ok(())
}

pub async fn handle_get(config: &Config, key: &str) -> Result<()> {
    let env = EnvConfig::load_or_init(config).await?;
    match env.get(key) {
        Some(v) => println!("{v}"),
        None => {
            eprintln!(
                "{}",
                colors::yellow(&format!("{key} is not set in the active config [env]"))
            );
            std::process::exit(1);
        }
    }
    Ok(())
}

pub async fn handle_decrypt(config: &Config, key: &str) -> Result<()> {
    let env = EnvConfig::load_or_init(config).await?;
    let Some(value) = env.get(key) else {
        bail!("{key} is not set in the active config [env]");
    };
    let plaintext = secret::decrypt_secret(value, &config.age_identities())
        .await
        .with_context(|| format!("decrypting {key}"))?;
    print!("{plaintext}");
    Ok(())
}

pub async fn handle_export(config: &Config, key: &str, alias: Option<&str>) -> Result<()> {
    validate_env_export_key(key)?;
    if let Some(alias) = alias {
        validate_env_export_key(alias)?;
    }
    let env = EnvConfig::load_or_init(config).await?;
    let value = match resolve_env_export_value(&env, key)? {
        EnvExportValue::Secret {
            key: secret_key,
            value,
        } => secret::decrypt_secret(value, &config.age_identities())
            .await
            .with_context(|| format!("decrypting {secret_key}"))?,
        EnvExportValue::Plaintext(value) => value.to_string(),
    };
    let export_as = alias.unwrap_or(key);
    println!(
        "{}",
        format_env_export(&config.shell_type, export_as, &value)
    );
    Ok(())
}

type EnvExportValue<'a> = StoredValue<'a>;

fn resolve_env_export_value<'a>(env: &'a EnvConfig, key: &str) -> Result<EnvExportValue<'a>> {
    resolve_stored_value(env, key)
}

fn env_export_secret_key(key: &str) -> String {
    secret_key(key)
}

fn validate_env_export_key(key: &str) -> Result<()> {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        bail!("env secret export key must not be empty");
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        bail!("env secret export key must start with a letter or underscore: {key}");
    }
    if !chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric()) {
        bail!("env secret export key must contain only letters, digits, and underscores: {key}");
    }
    Ok(())
}

/// `pub(crate)` so `theme::handle_sync` can reuse the same per-shell quoting
/// instead of adding a fourth `single_quote` implementation to the codebase
/// (see docs/terminal-theme-sync-prd.md §7/§10).
pub(crate) fn format_env_export(shell: &shells::ShellType, key: &str, value: &str) -> String {
    match shell {
        shells::ShellType::Fish => format!("set -gx {key} {}", fish_quote(value)),
        shells::ShellType::PowerShell => {
            format!("$env:{key} = {}", powershell_string_quote(value))
        }
        _ => format!("export {key}={}", posix_shell_quote(value)),
    }
}

fn posix_shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn fish_quote(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn powershell_string_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[derive(Debug, PartialEq, Eq)]
enum EnvEncryptOutput {
    Print,
    Set(String),
}

fn resolve_env_encrypt_output(
    set_key: Option<&str>,
    from_key: Option<&str>,
) -> Result<EnvEncryptOutput> {
    if let Some(key) = set_key {
        return Ok(EnvEncryptOutput::Set(key.to_string()));
    }
    if let Some(key) = from_key {
        validate_env_export_key(key)?;
        return Ok(EnvEncryptOutput::Set(env_export_secret_key(key)));
    }
    Ok(EnvEncryptOutput::Print)
}

fn resolve_encrypt_backend(config: &Config, backend: Option<&str>) -> Result<BackendKind> {
    if let Some(backend) = backend.map(str::trim).filter(|value| !value.is_empty()) {
        return backend.parse();
    }
    if let Some(backend) = config
        .secret_backend
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return backend.parse();
    }
    Ok(BackendKind::default())
}

fn clean_recipients(recipients: &[String]) -> Vec<String> {
    recipients
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn resolve_encrypt_recipients(
    backend: BackendKind,
    cli_recipients: &[String],
    config: &Config,
) -> Result<EncryptRecipients> {
    let cli_recipients = clean_recipients(cli_recipients);
    if !cli_recipients.is_empty() {
        if backend == BackendKind::Gpg
            && let Some(hint) = cli_recipients
                .iter()
                .find(|value| value.starts_with("age1"))
        {
            bail!("recipient \"{hint}\" looks like an age recipient; did you mean --backend age?");
        }
        return Ok(match backend {
            BackendKind::Gpg => EncryptRecipients::Gpg(cli_recipients),
            BackendKind::Age => EncryptRecipients::Age(cli_recipients),
        });
    }

    match backend {
        BackendKind::Gpg => {
            let recipients = clean_recipients(&config.gpg_recipients);
            if recipients.is_empty() {
                bail!(
                    "GPG recipients are required; pass -r/--recipient, set gpg_recipients, or set secret_backend/age_recipients for age"
                );
            }
            Ok(EncryptRecipients::Gpg(recipients))
        }
        BackendKind::Age => {
            let recipients = clean_recipients(&config.age_recipients);
            if recipients.is_empty() {
                bail!(
                    "age recipients are required; pass -r/--recipient or set age_recipients in config.toml"
                );
            }
            Ok(EncryptRecipients::Age(recipients))
        }
    }
}

pub async fn handle_encrypt(
    config: &Config,
    backend: Option<&str>,
    recipients: &[String],
    set_key: Option<&str>,
    from_key: Option<&str>,
    force: bool,
) -> Result<()> {
    use std::io::Read as _;

    let backend = resolve_encrypt_backend(config, backend)?;
    let recipients = resolve_encrypt_recipients(backend, recipients, config)?;
    let plaintext = if let Some(key) = from_key {
        let env = EnvConfig::load_or_init(config).await?;
        let Some(value) = env.get(key) else {
            bail!("{key} is not set in the active config [env]");
        };
        value.as_bytes().to_vec()
    } else {
        let mut input = Vec::new();
        std::io::stdin()
            .read_to_end(&mut input)
            .context("reading secret from stdin")?;
        input
    };
    let encoded = secret::encrypt_secret(&plaintext, &recipients)
        .await
        .context("encrypting secret")?;
    match resolve_env_encrypt_output(set_key, from_key)? {
        EnvEncryptOutput::Set(key) => match resolve_env_write_target(config, &key, force)? {
            EnvWriteTarget::ConfigToml => {
                let mut env = EnvConfig::load_or_init(config).await?;
                env.set(&key, &encoded);
                env.save(config).await?;
                println!(
                    "{}",
                    colors::green(&format!(
                        "set {key} = \"{encoded}\" in {}",
                        path_display::format(config.config_path())
                    ))
                );
            }
            EnvWriteTarget::OverrideFile(source) => {
                crate::config::write_env_override_entry(&source.path, &key, Some(&encoded)).await?;
                println!(
                    "{}",
                    colors::green(&format!(
                        "set {key} = \"{encoded}\" in {}",
                        path_display::format(&source.path)
                    ))
                );
            }
        },
        EnvEncryptOutput::Print => println!("{encoded}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use tokio::fs;

    async fn make_temp_dir() -> std::path::PathBuf {
        crate::test_support::make_temp_dir("shine-env-cmd-test").await
    }

    fn config_in(dir: &std::path::Path) -> Config {
        crate::test_support::test_config(dir)
    }

    #[test]
    fn env_show_redacts_sensitive_values() {
        assert_eq!(display_env_value("secret", true, false), "<redacted>");
        assert_eq!(display_env_value("secret", true, true), "secret");
        assert_eq!(display_env_value("", true, false), "<empty>");
        assert!(is_sensitive_env_key("MY_API_KEY"));
        assert!(is_sensitive_env_key("token"));
        assert!(is_sensitive_env_key("SURGE_SUBSCRIPTION_URL"));
        assert!(!is_sensitive_env_key("MONKEY"));
    }

    fn source(kind: EnvOverrideKind, managed: bool) -> EnvOverrideSource {
        EnvOverrideSource {
            path: std::path::PathBuf::from("/tmp/shine.env.toml"),
            kind,
            is_managed_overlay: managed,
        }
    }

    #[test]
    fn env_source_group_maps_each_layer() {
        assert_eq!(env_source_group(None), EnvSourceGroup::Config);
        assert_eq!(
            env_source_group(Some(&source(EnvOverrideKind::Global, false))),
            EnvSourceGroup::Global
        );
        assert_eq!(
            env_source_group(Some(&source(EnvOverrideKind::Overlay, false))),
            EnvSourceGroup::Overlay { managed: false }
        );
        assert_eq!(
            env_source_group(Some(&source(EnvOverrideKind::Overlay, true))),
            EnvSourceGroup::Overlay { managed: true }
        );
        assert_eq!(
            env_source_group(Some(&source(EnvOverrideKind::Project, false))),
            EnvSourceGroup::Project
        );
    }

    #[test]
    fn group_env_keys_orders_sections_and_skips_empty() {
        let global = source(EnvOverrideKind::Global, false);
        let overlay = source(EnvOverrideKind::Overlay, true);
        // Keys deliberately out of source order; config keys have no override.
        let keys = ["PROJECT_LESS", "FROM_OVERLAY", "FROM_CONFIG", "FROM_GLOBAL"];
        let groups = group_env_keys(keys.iter().copied(), |key| match key {
            "FROM_GLOBAL" => Some(&global),
            "FROM_OVERLAY" => Some(&overlay),
            _ => None,
        });

        // Only Config, Global, Overlay are present (Project skipped), in order.
        assert_eq!(
            groups.iter().map(|(g, _)| *g).collect::<Vec<_>>(),
            vec![
                EnvSourceGroup::Config,
                EnvSourceGroup::Global,
                EnvSourceGroup::Overlay { managed: true },
            ]
        );
        assert_eq!(groups[0].1, vec!["PROJECT_LESS", "FROM_CONFIG"]);
        assert_eq!(groups[1].1, vec!["FROM_GLOBAL"]);
        assert_eq!(groups[2].1, vec!["FROM_OVERLAY"]);
    }

    #[test]
    fn group_env_keys_all_config_yields_single_group() {
        let keys = ["A", "B", "C"];
        let groups = group_env_keys(keys.iter().copied(), |_| None);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, EnvSourceGroup::Config);
        assert_eq!(groups[0].1, vec!["A", "B", "C"]);
    }

    #[test]
    fn env_source_group_labels_are_stable() {
        assert_eq!(EnvSourceGroup::Config.label(), "config.toml");
        assert_eq!(EnvSourceGroup::Global.label(), "global env file");
        assert_eq!(
            EnvSourceGroup::Overlay { managed: false }.label(),
            "overlay"
        );
        assert_eq!(
            EnvSourceGroup::Overlay { managed: true }.label(),
            "overlay (managed)"
        );
        assert_eq!(EnvSourceGroup::Project.label(), "project env file");
    }

    #[test]
    fn env_show_truncates_long_values_to_requested_width() {
        assert_eq!(truncate_text("abcdefgh", 5), "abcd…");
        let (value, description) = fit_env_row(
            "abcdefghijklmnopqrstuvwxyz",
            "A description that is also fairly long",
            8,
            48,
        );
        assert!(value.chars().count() <= env_value_width(8, 48));
        assert!(description.chars().count() <= 48);
    }

    #[test]
    fn env_export_uses_alias_as_variable_name() {
        let value = "secret123";
        assert_eq!(
            format_env_export(&shells::ShellType::Zsh, "MY_ALIAS", value),
            "export MY_ALIAS='secret123'"
        );
    }

    #[test]
    fn env_export_alias_formats_powershell_correctly() {
        let value = "secret123";
        assert_eq!(
            format_env_export(&shells::ShellType::PowerShell, "MY_ALIAS", value),
            "$env:MY_ALIAS = 'secret123'"
        );
    }

    #[tokio::test]
    async fn env_delete_removes_key_from_saved_config() {
        let dir = make_temp_dir().await;
        let mut config = config_in(&dir);
        config.env.insert("MY_TOKEN".into(), "secret".into());
        config.save().await.unwrap();

        handle_delete(&config, "MY_TOKEN", false).await.unwrap();

        let contents = fs::read_to_string(config.config_path()).await.unwrap();
        let parsed: toml::Table = toml::from_str(&contents).unwrap();
        let env = parsed
            .get("env")
            .and_then(|value| value.as_table())
            .unwrap();
        assert!(
            !env.contains_key("MY_TOKEN"),
            "deleted key should not remain in saved config: {contents}"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn env_delete_fails_when_key_is_missing() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);

        let err = handle_delete(&config, "MY_TOKEN", false).await.unwrap_err();

        assert!(
            err.to_string()
                .contains("MY_TOKEN is not set in the active config [env]"),
            "error should explain missing key: {err:#}"
        );
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn env_export_secret_key_appends_secret_suffix() {
        assert_eq!(
            env_export_secret_key("DEEPSEEK_API_KEY"),
            "DEEPSEEK_API_KEY_SECRET"
        );
        assert_eq!(env_export_secret_key("xxx"), "xxx_SECRET");
    }

    #[test]
    fn env_export_resolves_secret_when_present() {
        let mut env = EnvConfig::default();
        env.set("MY_TOKEN_SECRET", "encrypted");

        assert_eq!(
            resolve_env_export_value(&env, "MY_TOKEN").unwrap(),
            EnvExportValue::Secret {
                key: "MY_TOKEN_SECRET".to_string(),
                value: "encrypted"
            }
        );
    }

    #[test]
    fn env_export_falls_back_to_plaintext_value() {
        let mut env = EnvConfig::default();
        env.set("MY_TOKEN", "plain");

        assert_eq!(
            resolve_env_export_value(&env, "MY_TOKEN").unwrap(),
            EnvExportValue::Plaintext("plain")
        );
    }

    #[test]
    fn env_export_secret_wins_over_plaintext_value() {
        let mut env = EnvConfig::default();
        env.set("MY_TOKEN", "plain");
        env.set("MY_TOKEN_SECRET", "encrypted");

        assert_eq!(
            resolve_env_export_value(&env, "MY_TOKEN").unwrap(),
            EnvExportValue::Secret {
                key: "MY_TOKEN_SECRET".to_string(),
                value: "encrypted"
            }
        );
    }

    #[test]
    fn env_export_reports_both_missing_keys() {
        let env = EnvConfig::default();

        let err = resolve_env_export_value(&env, "MY_TOKEN").unwrap_err();

        assert!(
            err.to_string()
                .contains("MY_TOKEN_SECRET or MY_TOKEN is not set in the active config [env]"),
            "error should explain both checked keys: {err:#}"
        );
    }

    #[test]
    fn env_export_key_validation_accepts_shell_variable_names() {
        for key in ["FOO", "_FOO", "foo_123", "A1"] {
            validate_env_export_key(key).unwrap();
        }
    }

    #[test]
    fn env_export_key_validation_rejects_unsafe_names() {
        for key in ["", "1FOO", "FOO-BAR", "FOO;BAR", "FOO BAR", "FOO.SECRET"] {
            assert!(
                validate_env_export_key(key).is_err(),
                "key should be rejected: {key}"
            );
        }
    }

    #[test]
    fn env_export_formats_posix_shell_code_safely() {
        let value = "abc def'ghi$HOME\nnext; rm -rf /";
        assert_eq!(
            format_env_export(&shells::ShellType::Zsh, "TOKEN", value),
            "export TOKEN='abc def'\\''ghi$HOME\nnext; rm -rf /'"
        );
    }

    #[test]
    fn env_export_formats_fish_shell_code_safely() {
        let value = "abc def'ghi\\path\nnext; rm -rf /";
        assert_eq!(
            format_env_export(&shells::ShellType::Fish, "TOKEN", value),
            "set -gx TOKEN 'abc def\\'ghi\\\\path\nnext; rm -rf /'"
        );
    }

    #[test]
    fn env_export_formats_powershell_code_safely() {
        let value = "abc def'ghi$HOME\nnext; Remove-Item /";
        assert_eq!(
            format_env_export(&shells::ShellType::PowerShell, "TOKEN", value),
            "$env:TOKEN = 'abc def''ghi$HOME\nnext; Remove-Item /'"
        );
    }

    #[test]
    fn env_encrypt_output_defaults_from_key_to_secret_key() {
        assert_eq!(
            resolve_env_encrypt_output(None, Some("GH_TOKEN")).unwrap(),
            EnvEncryptOutput::Set("GH_TOKEN_SECRET".to_string())
        );
    }

    #[test]
    fn env_encrypt_output_explicit_set_wins_over_default() {
        assert_eq!(
            resolve_env_encrypt_output(Some("CUSTOM_SECRET"), Some("GH_TOKEN")).unwrap(),
            EnvEncryptOutput::Set("CUSTOM_SECRET".to_string())
        );
    }

    #[test]
    fn env_encrypt_output_prints_stdin_without_set() {
        assert_eq!(
            resolve_env_encrypt_output(None, None).unwrap(),
            EnvEncryptOutput::Print
        );
    }

    #[test]
    fn env_encrypt_output_rejects_invalid_inferred_from_key() {
        let err = resolve_env_encrypt_output(None, Some("GH-TOKEN")).unwrap_err();

        assert!(
            err.to_string().contains(
                "env secret export key must contain only letters, digits, and underscores"
            ),
            "error should explain invalid inferred key: {err:#}"
        );
    }

    #[test]
    fn encrypt_backend_cli_wins_over_config() {
        let dir = std::env::temp_dir().join(format!("shine-env-backend-{}", uuid::Uuid::new_v4()));
        let mut config = config_in(&dir);
        config.secret_backend = Some("age".to_string());

        assert_eq!(
            resolve_encrypt_backend(&config, Some("gpg")).unwrap(),
            BackendKind::Gpg
        );
    }

    #[test]
    fn encrypt_backend_falls_back_to_config() {
        let dir = std::env::temp_dir().join(format!("shine-env-backend-{}", uuid::Uuid::new_v4()));
        let mut config = config_in(&dir);
        config.secret_backend = Some("age".to_string());

        assert_eq!(
            resolve_encrypt_backend(&config, None).unwrap(),
            BackendKind::Age
        );
    }

    #[test]
    fn encrypt_backend_defaults_to_gpg() {
        let dir = std::env::temp_dir().join(format!("shine-env-backend-{}", uuid::Uuid::new_v4()));
        let config = config_in(&dir);

        assert_eq!(
            resolve_encrypt_backend(&config, None).unwrap(),
            BackendKind::Gpg
        );
    }

    #[test]
    fn encrypt_recipients_cli_wins_over_config_for_gpg() {
        let dir =
            std::env::temp_dir().join(format!("shine-env-recipient-{}", uuid::Uuid::new_v4()));
        let mut config = config_in(&dir);
        config.gpg_recipients = vec!["config@example.com".to_string()];

        let recipients =
            resolve_encrypt_recipients(BackendKind::Gpg, &["cli@example.com".to_string()], &config)
                .unwrap();

        match recipients {
            EncryptRecipients::Gpg(values) => assert_eq!(values, vec!["cli@example.com"]),
            EncryptRecipients::Age(_) => panic!("expected gpg recipients"),
        }
    }

    #[test]
    fn encrypt_recipients_gpg_falls_back_to_config() {
        let dir =
            std::env::temp_dir().join(format!("shine-env-recipient-{}", uuid::Uuid::new_v4()));
        let mut config = config_in(&dir);
        config.gpg_recipients = vec![
            "config@example.com".to_string(),
            "team@example.com".to_string(),
        ];

        let recipients = resolve_encrypt_recipients(BackendKind::Gpg, &[], &config).unwrap();

        match recipients {
            EncryptRecipients::Gpg(values) => {
                assert_eq!(values, vec!["config@example.com", "team@example.com"])
            }
            EncryptRecipients::Age(_) => panic!("expected gpg recipients"),
        }
    }

    #[test]
    fn encrypt_recipients_gpg_treats_empty_config_as_missing() {
        let dir =
            std::env::temp_dir().join(format!("shine-env-recipient-{}", uuid::Uuid::new_v4()));
        let mut config = config_in(&dir);
        config.gpg_recipients = vec!["  ".to_string()];

        let err = resolve_encrypt_recipients(BackendKind::Gpg, &[], &config).unwrap_err();

        assert!(
            err.to_string()
                .contains("pass -r/--recipient, set gpg_recipients"),
            "error should explain how to set recipient: {err:#}"
        );
    }

    #[test]
    fn encrypt_recipients_gpg_errors_when_missing() {
        let dir =
            std::env::temp_dir().join(format!("shine-env-recipient-{}", uuid::Uuid::new_v4()));
        let config = config_in(&dir);

        let err = resolve_encrypt_recipients(BackendKind::Gpg, &[], &config).unwrap_err();

        assert!(
            err.to_string()
                .contains("pass -r/--recipient, set gpg_recipients"),
            "error should explain how to set recipient: {err:#}"
        );
    }

    #[test]
    fn encrypt_recipients_age_falls_back_to_config() {
        let dir =
            std::env::temp_dir().join(format!("shine-env-recipient-{}", uuid::Uuid::new_v4()));
        let mut config = config_in(&dir);
        config.age_recipients = vec!["age1qexample".to_string()];

        let recipients = resolve_encrypt_recipients(BackendKind::Age, &[], &config).unwrap();

        match recipients {
            EncryptRecipients::Age(values) => assert_eq!(values, vec!["age1qexample"]),
            EncryptRecipients::Gpg(_) => panic!("expected age recipients"),
        }
    }

    #[test]
    fn encrypt_recipients_age_errors_when_missing() {
        let dir =
            std::env::temp_dir().join(format!("shine-env-recipient-{}", uuid::Uuid::new_v4()));
        let config = config_in(&dir);

        let err = resolve_encrypt_recipients(BackendKind::Age, &[], &config).unwrap_err();

        assert!(
            err.to_string().contains("age recipients are required"),
            "error should explain how to set age recipients: {err:#}"
        );
    }

    #[test]
    fn encrypt_recipients_hints_when_age_recipient_used_with_gpg_backend() {
        let dir =
            std::env::temp_dir().join(format!("shine-env-recipient-{}", uuid::Uuid::new_v4()));
        let config = config_in(&dir);

        let err =
            resolve_encrypt_recipients(BackendKind::Gpg, &["age1qexample".to_string()], &config)
                .unwrap_err();

        assert!(
            err.to_string().contains("did you mean --backend age"),
            "error should hint at the age backend: {err:#}"
        );
    }

    fn shadow_key(
        config: &mut Config,
        key: &str,
        path: std::path::PathBuf,
        is_managed_overlay: bool,
    ) {
        let kind = if is_managed_overlay {
            crate::config::EnvOverrideKind::Overlay
        } else {
            crate::config::EnvOverrideKind::Global
        };
        config.env_override_sources.insert(
            key.to_string(),
            crate::config::EnvOverrideSource {
                path,
                kind,
                is_managed_overlay,
            },
        );
    }

    #[test]
    fn resolve_env_write_target_returns_config_toml_when_unshadowed() {
        let dir = std::env::temp_dir().join(format!("shine-env-write-{}", uuid::Uuid::new_v4()));
        let config = config_in(&dir);

        let target = resolve_env_write_target(&config, "MY_TOKEN", false).unwrap();

        assert!(matches!(target, EnvWriteTarget::ConfigToml));
    }

    #[test]
    fn resolve_env_write_target_refuses_without_force_when_shadowed() {
        let dir = std::env::temp_dir().join(format!("shine-env-write-{}", uuid::Uuid::new_v4()));
        let mut config = config_in(&dir);
        let override_path = dir.join("shine.env.toml");
        shadow_key(&mut config, "MY_TOKEN", override_path.clone(), false);

        let err = resolve_env_write_target(&config, "MY_TOKEN", false).unwrap_err();

        assert!(
            err.to_string().contains(override_path.to_str().unwrap()),
            "error should name the winning override file: {err:#}"
        );
        assert!(
            err.to_string().contains("--force"),
            "error should hint at --force: {err:#}"
        );
    }

    #[test]
    fn resolve_env_write_target_returns_override_file_with_force() {
        let dir = std::env::temp_dir().join(format!("shine-env-write-{}", uuid::Uuid::new_v4()));
        let mut config = config_in(&dir);
        let override_path = dir.join("shine.env.toml");
        shadow_key(&mut config, "MY_TOKEN", override_path.clone(), false);

        let target = resolve_env_write_target(&config, "MY_TOKEN", true).unwrap();

        match target {
            EnvWriteTarget::OverrideFile(source) => assert_eq!(source.path, override_path),
            EnvWriteTarget::ConfigToml => panic!("expected the shadowing override file"),
        }
    }

    #[test]
    fn resolve_env_write_target_allows_managed_overlay_with_force() {
        let dir = std::env::temp_dir().join(format!("shine-env-write-{}", uuid::Uuid::new_v4()));
        let mut config = config_in(&dir);
        let overlay_path = dir.join("overlay").join("shine.env.toml");
        shadow_key(&mut config, "MY_TOKEN", overlay_path.clone(), true);

        let target = resolve_env_write_target(&config, "MY_TOKEN", true).unwrap();

        match target {
            EnvWriteTarget::OverrideFile(source) => {
                assert_eq!(source.path, overlay_path);
                assert!(source.is_managed_overlay);
            }
            EnvWriteTarget::ConfigToml => panic!("expected the managed overlay override file"),
        }
    }

    #[tokio::test]
    async fn env_set_refuses_when_shadowed_without_force() {
        let dir = make_temp_dir().await;
        let mut config = config_in(&dir);
        let override_path = dir.join("shine.env.toml");
        shadow_key(&mut config, "MY_TOKEN", override_path.clone(), false);

        let err = handle_set(&config, "MY_TOKEN", "newval", false)
            .await
            .unwrap_err();

        assert!(err.to_string().contains(override_path.to_str().unwrap()));
        assert!(
            !fs::try_exists(&override_path).await.unwrap(),
            "refused write must not touch the override file"
        );
        assert!(
            !fs::read_to_string(config.config_path())
                .await
                .unwrap_or_default()
                .contains("MY_TOKEN"),
            "refused write must not touch config.toml either"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn env_set_writes_into_override_file_when_forced() {
        let dir = make_temp_dir().await;
        let mut config = config_in(&dir);
        let override_path = dir.join("shine.env.toml");
        fs::write(&override_path, "MY_TOKEN = \"old\"\n")
            .await
            .unwrap();
        shadow_key(&mut config, "MY_TOKEN", override_path.clone(), false);

        handle_set(&config, "MY_TOKEN", "newval", true)
            .await
            .unwrap();

        let content = fs::read_to_string(&override_path).await.unwrap();
        assert!(content.contains("MY_TOKEN = \"newval\""));
        assert!(
            !fs::read_to_string(config.config_path())
                .await
                .unwrap_or_default()
                .contains("MY_TOKEN"),
            "forced write must go into the override file, not config.toml"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn env_delete_refuses_when_shadowed_without_force() {
        let dir = make_temp_dir().await;
        let mut config = config_in(&dir);
        let override_path = dir.join("shine.env.toml");
        fs::write(&override_path, "MY_TOKEN = \"secret\"\n")
            .await
            .unwrap();
        shadow_key(&mut config, "MY_TOKEN", override_path.clone(), false);

        let err = handle_delete(&config, "MY_TOKEN", false).await.unwrap_err();

        assert!(err.to_string().contains(override_path.to_str().unwrap()));
        let content = fs::read_to_string(&override_path).await.unwrap();
        assert!(
            content.contains("MY_TOKEN"),
            "refused delete must leave the override file untouched"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn env_delete_removes_from_override_file_when_forced() {
        let dir = make_temp_dir().await;
        let mut config = config_in(&dir);
        let override_path = dir.join("shine.env.toml");
        fs::write(&override_path, "MY_TOKEN = \"secret\"\nOTHER = \"kept\"\n")
            .await
            .unwrap();
        shadow_key(&mut config, "MY_TOKEN", override_path.clone(), false);

        handle_delete(&config, "MY_TOKEN", true).await.unwrap();

        let content = fs::read_to_string(&override_path).await.unwrap();
        let table: toml::Table = toml::from_str(&content).unwrap();
        assert!(!table.contains_key("MY_TOKEN"));
        assert!(table.contains_key("OTHER"));

        fs::remove_dir_all(&dir).await.unwrap();
    }
}
