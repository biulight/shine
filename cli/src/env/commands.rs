//! Handlers for `shine env show/set/delete/get/decrypt/export/encrypt`.

use anyhow::{Context, Result, bail};

use super::{EnvConfig, StoredValue, resolve_stored_value, secret_key};
use crate::config::Config;
use crate::secret::{BackendKind, EncryptRecipients};
use crate::{colors, path_display, secret, shells};

pub async fn handle_show(config: &Config, reveal: bool) -> Result<()> {
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
    }
    for (k, v) in env.iter() {
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

pub async fn handle_set(config: &Config, key: &str, value: &str) -> Result<()> {
    let mut env = EnvConfig::load_or_init(config).await?;
    env.set(key, value);
    env.save(config).await?;
    println!("{}", colors::green(&format!("set {key} = \"{value}\"")));
    println!(
        "{}",
        colors::dim("Run `shine upgrade` to apply to already-installed presets.")
    );
    Ok(())
}

pub async fn handle_delete(config: &Config, key: &str) -> Result<()> {
    let mut env = EnvConfig::load_or_init(config).await?;
    if env.remove(key).is_none() {
        bail!("{key} is not set in the active config [env]");
    }
    env.save(config).await?;
    println!("{}", colors::green(&format!("deleted {key}")));
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
        bail!("env export key must not be empty");
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        bail!("env export key must start with a letter or underscore: {key}");
    }
    if !chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric()) {
        bail!("env export key must contain only letters, digits, and underscores: {key}");
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
            let recipient = config
                .gpg_key_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .context(
                    "GPG recipient is required; pass -r/--recipient, set gpg_key_id, or set secret_backend/age_recipients for age",
                )?;
            Ok(EncryptRecipients::Gpg(vec![recipient.to_string()]))
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
        EnvEncryptOutput::Set(key) => {
            let mut env = EnvConfig::load_or_init(config).await?;
            env.set(&key, &encoded);
            env.save(config).await?;
            println!("{}", colors::green(&format!("set {key} = \"{encoded}\"")));
        }
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
        assert!(!is_sensitive_env_key("MONKEY"));
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

        handle_delete(&config, "MY_TOKEN").await.unwrap();

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

        let err = handle_delete(&config, "MY_TOKEN").await.unwrap_err();

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
            err.to_string()
                .contains("env export key must contain only letters, digits, and underscores"),
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
        config.gpg_key_id = Some("config@example.com".to_string());

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
        config.gpg_key_id = Some("config@example.com".to_string());

        let recipients = resolve_encrypt_recipients(BackendKind::Gpg, &[], &config).unwrap();

        match recipients {
            EncryptRecipients::Gpg(values) => assert_eq!(values, vec!["config@example.com"]),
            EncryptRecipients::Age(_) => panic!("expected gpg recipients"),
        }
    }

    #[test]
    fn encrypt_recipients_gpg_treats_empty_config_as_missing() {
        let dir =
            std::env::temp_dir().join(format!("shine-env-recipient-{}", uuid::Uuid::new_v4()));
        let mut config = config_in(&dir);
        config.gpg_key_id = Some("  ".to_string());

        let err = resolve_encrypt_recipients(BackendKind::Gpg, &[], &config).unwrap_err();

        assert!(
            err.to_string()
                .contains("pass -r/--recipient, set gpg_key_id"),
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
                .contains("pass -r/--recipient, set gpg_key_id"),
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
}
