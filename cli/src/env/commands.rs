//! Handlers for env inspection, directed writes, and secret commands.

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use shine_core::shell_quote::posix_single_quote as posix_shell_quote;
use shine_core::shell_quote::powershell_single_quote as powershell_string_quote;

use super::{EnvConfig, StoredValue, resolve_stored_value, secret_key};
use crate::config::{Config, EnvLayerKind, EnvWriteOptions, EnvWritePlan};
use crate::output::escape_controls;
use crate::secret::{BackendKind, EncryptRecipients};
use crate::{colors, path_display, secret, shells};

pub async fn handle_list(config: &Config, reveal: bool) -> Result<()> {
    let env = EnvConfig::from_config(config);
    let catalog = super::catalog::load(config).await?;
    let terminal_width = usize::from(console::Term::stdout().size().1).max(40);
    let key_width = env
        .iter()
        .map(|(key, _)| console::measure_text_width(&escape_env_cell(key)))
        .max()
        .unwrap_or(0);

    println!("{}", colors::bold("Environment"));
    println!();
    if env.as_map().is_empty() {
        println!("  {}", colors::dim("No variables configured."));
        println!();
    }

    if config.is_project_config() {
        println!(
            "Project root: {}",
            escape_controls(&config.config_path().parent().unwrap().display().to_string())
        );
    } else {
        println!("Scope: global (no active project)");
    }
    for layer in &config.env_layers {
        let keys: Vec<_> = env
            .iter()
            .map(|(k, _)| k)
            .filter(|k| {
                config
                    .env_value_layer(k)
                    .is_some_and(|winner| winner.kind == layer.kind)
            })
            .collect();
        if keys.is_empty() {
            continue;
        }
        println!(
            "{}  {}",
            colors::bold(layer.kind.label()),
            layer
                .path
                .as_ref()
                .map(|p| escape_controls(&path_display::format(p)))
                .unwrap_or_default()
        );
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
            let display_key = escape_env_cell(k);
            let key_padding =
                " ".repeat(key_width.saturating_sub(console::measure_text_width(&display_key)));
            if description.is_empty() {
                println!(
                    "  {}{}  {}",
                    colors::cyan(&display_key),
                    key_padding,
                    display_value
                );
            } else {
                let value_padding = " ".repeat(
                    env_value_width(key_width, terminal_width)
                        .saturating_sub(console::measure_text_width(&display_value)),
                );
                println!(
                    "  {}{}  {}{}  {}",
                    colors::cyan(&display_key),
                    key_padding,
                    display_value,
                    value_padding,
                    colors::dim(&description),
                );
            }
        }
        println!();
    }

    println!(
        "  {}  {}",
        colors::dim("Config"),
        colors::dim(&escape_controls(&path_display::format(
            config.config_path()
        )))
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

// Escape only presentation cells; raw values used by get/export/children stay untouched.
fn escape_env_cell(value: &str) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        if ch.is_control() || ch == '\\' {
            output.extend(ch.escape_debug());
        } else {
            output.push(ch);
        }
    }
    output
}

fn truncate_text(value: &str, max_width: usize) -> String {
    console::truncate_str(&escape_env_cell(value), max_width, "…").into_owned()
}

pub async fn handle_explain(config: &Config, key: &str, reveal: bool) -> Result<()> {
    println!("Key: {}", escape_controls(key));
    let winner = config.env_value_layer(key).map(|l| l.kind);
    for kind in [
        EnvLayerKind::Builtin,
        EnvLayerKind::Overlay,
        EnvLayerKind::GlobalConfig,
        EnvLayerKind::GlobalEnv,
        EnvLayerKind::ProjectConfig,
        EnvLayerKind::ProjectEnv,
    ] {
        let Some(layer) = config.env_layers.iter().find(|layer| layer.kind == kind) else {
            println!("  {:<18} (inactive)  absent", kind.label());
            continue;
        };
        let status = if !layer.values.contains_key(key) {
            "absent"
        } else if winner == Some(layer.kind) {
            "effective"
        } else {
            "shadowed"
        };
        let path = layer
            .path
            .as_ref()
            .map(|p| escape_controls(&path_display::format(p)))
            .unwrap_or_default();
        println!("  {:<18} {}  {status}", layer.kind.label(), path);
        if reveal && let Some(value) = layer.values.get(key) {
            println!("    value: {value:?}");
        }
    }
    let description_source = config.env_description_sources.get(key).copied();
    if let Some(kind) = description_source {
        let path = config
            .env_layers
            .iter()
            .find(|l| l.kind == kind)
            .and_then(|l| l.path.as_ref());
        println!(
            "Description source: {} {}",
            kind.label(),
            path.map(|p| escape_controls(&path_display::format(p)))
                .unwrap_or_default()
        );
    } else {
        let catalog = super::catalog::load(config).await?;
        println!(
            "Description source: {}",
            if catalog.contains_key(key) {
                "preset catalog"
            } else {
                "none"
            }
        );
    }
    println!(
        "Value source: {}",
        winner.map(EnvLayerKind::label).unwrap_or("none")
    );
    println!(
        "Default write target: {} {}",
        if config.is_project_config() {
            "project config"
        } else {
            "global config"
        },
        escape_controls(&config.config_path().display().to_string())
    );
    if config.is_project_config() {
        println!(
            "Project root: {}",
            escape_controls(&config.config_path().parent().unwrap().display().to_string())
        );
    } else {
        println!("Scope: global (no active project)");
    }
    print_secret_preference(config, key);
    Ok(())
}

fn print_secret_preference(config: &Config, key: &str) {
    let encrypted = secret_key(key);
    if config.env.contains_key(&encrypted) {
        if encrypted.chars().any(char::is_control) {
            println!(
                "Explicit injection prefers {}; inspect it with `shine env explain` using the original key.",
                escape_controls(&encrypted)
            );
            return;
        }
        println!(
            "Explicit injection prefers {encrypted}; run `shine env explain {encrypted}` to inspect its source."
        );
    }
}

async fn report_saved(
    config: &Config,
    plan: &EnvWritePlan,
    key: &str,
    deleted: bool,
) -> Result<()> {
    println!(
        "{} {key} in {} ({})",
        if deleted { "Deleted" } else { "Saved" },
        escape_controls(&plan.target.path.display().to_string()),
        plan.kind.label()
    );
    let current = Config::load_read_only()
        .await
        .context("file was saved, but effective state verification failed")?;
    match current.env_value_layer(key) {
        Some(layer) => println!(
            "Current effective source: {} {}{}",
            layer.kind.label(),
            layer
                .path
                .as_ref()
                .map(|p| escape_controls(&path_display::format(p)))
                .unwrap_or_default(),
            if layer.kind > plan.kind {
                "; selected layer remains shadowed"
            } else if deleted && layer.kind < plan.kind {
                "; restored inheritance"
            } else {
                ""
            }
        ),
        None => println!("No configured value remains."),
    }
    println!(
        "Current value {}.",
        if current.env.get(key) == config.env.get(key) {
            "unchanged"
        } else {
            "changed"
        }
    );
    print_secret_preference(&current, key);
    println!(
        "Run `shine update` to review installed presets and `shine upgrade` to apply reviewed changes."
    );
    Ok(())
}

pub async fn handle_set(
    config: &Config,
    key: &str,
    value: &str,
    target: &EnvWriteOptions,
) -> Result<()> {
    let plan = EnvWritePlan::prepare(config, key, target, false).await?;
    plan.commit(config, key, Some(value)).await?;
    report_saved(config, &plan, key, false).await
}

pub async fn handle_delete(config: &Config, key: &str, target: &EnvWriteOptions) -> Result<()> {
    let plan = EnvWritePlan::prepare(config, key, target, true).await?;
    plan.commit(config, key, None).await?;
    report_saved(config, &plan, key, true).await
}

pub async fn handle_get(config: &Config, key: &str) -> Result<()> {
    let env = EnvConfig::from_config(config);
    match env.get(key) {
        Some(v) => println!("{v}"),
        None => {
            eprintln!(
                "{}",
                colors::yellow_stderr(&format!(
                    "{} is not set in the active config [env]",
                    escape_controls(key)
                ))
            );
            std::process::exit(1);
        }
    }
    Ok(())
}

pub async fn handle_decrypt(config: &Config, key: &str) -> Result<()> {
    let env = EnvConfig::from_config(config);
    let Some(value) = env.get(key) else {
        bail!(
            "{} is not set in the active config [env]",
            escape_controls(key)
        );
    };
    let plaintext = secret::decrypt_with_config(value, config)
        .await
        .with_context(|| format!("decrypting {}", escape_controls(key)))?;
    write_decrypted_plaintext(std::io::stdout().lock(), &plaintext)?;
    Ok(())
}

fn write_decrypted_plaintext(mut output: impl std::io::Write, plaintext: &str) -> Result<()> {
    output
        .write_all(plaintext.as_bytes())
        .context("writing decrypted plaintext")?;
    output.flush().context("flushing decrypted plaintext")
}

pub async fn handle_export(config: &Config, key: &str, alias: Option<&str>) -> Result<()> {
    validate_env_export_key(key)?;
    if let Some(alias) = alias {
        validate_env_export_key(alias)?;
    }
    let env = EnvConfig::from_config(config);
    let value = match resolve_env_export_value(&env, key)? {
        EnvExportValue::Secret {
            key: secret_key,
            value,
        } => secret::decrypt_with_config(value, config)
            .await
            .with_context(|| format!("decrypting {secret_key}"))?,
        EnvExportValue::Plaintext(value) => value.to_string(),
    };
    let export_as = alias.unwrap_or(key);
    if value.contains('\0') {
        bail!("{export_as} contains a NUL byte and cannot be exported as an environment variable");
    }
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
        shells::ShellType::PowerShell if !value.is_ascii() || value.contains(['\r', '\n']) => {
            // Native PowerShell capture splits lines and may decode UTF-8 using a
            // legacy console code page. Keep the expression itself ASCII and one line.
            let encoded = base64::engine::general_purpose::STANDARD.encode(value);
            format!(
                "$env:{key} = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String({}))",
                powershell_string_quote(&encoded)
            )
        }
        shells::ShellType::PowerShell => {
            format!("$env:{key} = {}", powershell_string_quote(value))
        }
        _ => format!("export {key}={}", posix_shell_quote(value)),
    }
}

fn fish_quote(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
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
            BackendKind::Hybrid => {
                bail!("hybrid requires workspace access lists; use env secret seal")
            }
        });
    }

    match backend {
        BackendKind::Hybrid => bail!("hybrid requires workspace access lists; use env secret seal"),
        BackendKind::Gpg => {
            if config.legacy_gpg_key_id.is_some() {
                bail!(
                    "gpg_key_id is retired; run `shine state migrate` to convert it to gpg_recipients"
                );
            }
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
    target: &EnvWriteOptions,
) -> Result<()> {
    use std::io::Read as _;

    let output = resolve_env_encrypt_output(set_key, from_key)?;
    let plan = match &output {
        EnvEncryptOutput::Set(key) => {
            Some(EnvWritePlan::prepare(config, key, target, false).await?)
        }
        EnvEncryptOutput::Print => {
            if target.force || target.global || target.project || target.overlay || target.env_file
            {
                bail!("write target options require --set or --from; nothing was written");
            }
            None
        }
    };

    let backend = resolve_encrypt_backend(config, backend)?;
    let recipients = resolve_encrypt_recipients(backend, recipients, config)?;
    let plaintext = if let Some(key) = from_key {
        let env = EnvConfig::from_config(config);
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
    match output {
        EnvEncryptOutput::Set(key) => {
            let plan = plan.as_ref().unwrap();
            plan.commit(config, &key, Some(&encoded)).await?;
            report_saved(config, plan, &key, false).await?;
        }
        EnvEncryptOutput::Print => println!("{encoded}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn config_in(dir: &std::path::Path) -> Config {
        crate::test_support::test_config(dir)
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn failed_post_save_reload_reports_that_the_file_was_saved() {
        let _guard = crate::test_support::env_lock();
        let dir = crate::test_support::make_temp_dir("env-post-save").await;
        let cwd = std::env::current_dir().unwrap();
        let previous = std::env::var_os("SHINE_CONFIG_DIR");
        // SAFETY: shared env_lock is held until process context is restored.
        unsafe {
            std::env::set_var("SHINE_CONFIG_DIR", &dir);
        }
        std::env::set_current_dir(&dir).unwrap();
        let config = Config::load_read_only().await.unwrap();
        let plan = EnvWritePlan::prepare(&config, "KEY", &EnvWriteOptions::default(), false)
            .await
            .unwrap();
        plan.commit(&config, "KEY", Some("saved")).await.unwrap();
        tokio::fs::write(dir.join("shine.env.toml"), "invalid = [")
            .await
            .unwrap();
        let error = report_saved(&config, &plan, "KEY", false)
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("file was saved, but effective state verification failed")
        );
        assert!(
            tokio::fs::read_to_string(dir.join("config.toml"))
                .await
                .unwrap()
                .contains("saved")
        );
        std::env::set_current_dir(cwd).unwrap();
        // SAFETY: shared env_lock is still held.
        unsafe {
            match previous {
                Some(value) => std::env::set_var("SHINE_CONFIG_DIR", value),
                None => std::env::remove_var("SHINE_CONFIG_DIR"),
            }
        }
        tokio::fs::remove_dir_all(dir).await.unwrap();
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

    #[test]
    fn decrypted_plaintext_is_written_without_an_appended_line_ending() {
        let mut output = Vec::new();
        write_decrypted_plaintext(&mut output, "abc").unwrap();
        assert_eq!(output, b"abc");

        output.clear();
        write_decrypted_plaintext(&mut output, "abc\n").unwrap();
        assert_eq!(output, b"abc\n");
    }

    #[test]
    fn env_show_truncates_by_display_width_and_escapes_controls() {
        assert_eq!(truncate_text("中文中文", 5), "中文…");
        assert_eq!(truncate_text("e\u{301}e\u{301}", 2), "e\u{301}e\u{301}");
        assert_eq!(truncate_text("a\nb\tc\r\u{1b}", 40), r"a\nb\tc\r\u{1b}");
        assert_eq!(truncate_text(r"a\nb", 40), r"a\\nb");
        for width in 8..48 {
            let (value, description) =
                fit_env_row(&"中文".repeat(30), &"说明".repeat(30), 8, width);
            assert!(console::measure_text_width(&value) <= env_value_width(8, width));
            assert!(console::measure_text_width(&description) <= width);
        }
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
        let value = "abc def'ghi$HOME; Remove-Item /";
        assert_eq!(
            format_env_export(&shells::ShellType::PowerShell, "TOKEN", value),
            "$env:TOKEN = 'abc def''ghi$HOME; Remove-Item /'"
        );
    }

    #[test]
    fn env_export_powershell_transport_is_ascii_and_single_line() {
        for (value, encoded) in [
            ("line\r\nnext", "bGluZQ0KbmV4dA=="),
            ("line\rnext", "bGluZQ1uZXh0"),
            ("line\nnext", "bGluZQpuZXh0"),
            ("中文", "5Lit5paH"),
        ] {
            let code = format_env_export(&shells::ShellType::PowerShell, "TOKEN", value);
            assert_eq!(
                code,
                format!(
                    "$env:TOKEN = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{encoded}'))"
                )
            );
            assert!(code.is_ascii());
            assert_eq!(code.lines().count(), 1);
            assert!(!code.contains('\r'));
        }
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
            _ => panic!("expected gpg recipients"),
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
            _ => panic!("expected gpg recipients"),
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
            _ => panic!("expected age recipients"),
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
