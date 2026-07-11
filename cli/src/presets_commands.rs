use anyhow::{Result, bail};
use std::path::PathBuf;

use crate::config::{self, Config};
use crate::{colors, presets};

pub async fn handle_presets_export(
    config: &Config,
    dir: Option<PathBuf>,
    force: bool,
) -> Result<()> {
    use anyhow::Context as _;

    let target = dir.unwrap_or_else(|| config.presets_dir().to_owned());
    tokio::fs::create_dir_all(&target)
        .await
        .with_context(|| format!("creating export directory: {}", target.display()))?;

    println!("Exporting built-in presets to {} ...", target.display());

    let report = presets::extract_all(&target, force).await?;

    let created = report.created.len();
    let overwritten = report.overwritten.len();
    let skipped = report.skipped.len();

    if created > 0 {
        println!("{}", colors::green(&format!("  {created} file(s) created")));
    }
    if overwritten > 0 {
        println!(
            "{}",
            colors::yellow(&format!("  {overwritten} file(s) updated (overwritten)"))
        );
    }
    if skipped > 0 {
        println!("  {skipped} file(s) skipped (already exist; use --force to overwrite)");
    }
    if created == 0 && overwritten == 0 && skipped == 0 {
        println!("  No files exported (empty embedded asset set).");
    }

    if !config.is_external_presets {
        println!();
        println!(
            "Tip: run `shine link {}` to activate this directory.",
            target.display()
        );
    }

    Ok(())
}

pub async fn handle_presets_link(config: &Config, path: PathBuf, create: bool) -> Result<()> {
    use anyhow::Context as _;

    let raw = path.to_string_lossy();
    let expanded = config::full_expand(&raw).with_context(|| format!("expanding path: {raw}"))?;
    let expanded = PathBuf::from(expanded);

    if create {
        tokio::fs::create_dir_all(&expanded)
            .await
            .with_context(|| format!("creating directory: {}", expanded.display()))?;
    }

    let meta = tokio::fs::metadata(&expanded).await.with_context(|| {
        if create {
            format!("accessing directory: {}", expanded.display())
        } else {
            format!(
                "path does not exist: {} (use --create to create it)",
                expanded.display()
            )
        }
    })?;

    if !meta.is_dir() {
        bail!("path is not a directory: {}", expanded.display());
    }

    let absolute = tokio::fs::canonicalize(&expanded).await.unwrap_or(expanded);

    if config
        .presets_dir_override
        .as_deref()
        .is_some_and(|p| p == absolute)
    {
        println!(
            "{}",
            colors::dim(&format!("already linked: {}", absolute.display()))
        );
        return Ok(());
    }

    let updated = config
        .clone()
        .with_presets_dir_override(Some(absolute.clone()));
    updated.save().await?;

    if std::env::var("SHINE_CONFIG_DIR")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
        || std::env::var("SHINE_PRESETS")
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
    {
        println!(
            "{}",
            colors::yellow(
                "Warning: SHINE_CONFIG_DIR or SHINE_PRESETS is set and takes priority over \
                 the active config at runtime. Unset the env var for this setting to take effect."
            )
        );
    }

    println!("{}", colors::external_presets_note(&absolute));
    println!(
        "{}",
        colors::dim("Run `shine export` to populate the directory with built-in presets.")
    );

    Ok(())
}

pub async fn handle_presets_unlink(config: &Config) -> Result<()> {
    if config.presets_dir_override.is_none() {
        println!(
            "{}",
            colors::dim("No external presets directory is configured.")
        );
        return Ok(());
    }

    let updated = config.clone().with_presets_dir_override(None);
    updated.save().await?;

    println!(
        "{}",
        colors::green("External presets directory removed from the active config.")
    );
    println!(
        "{}",
        colors::dim("Built-in embedded presets will be used on the next run.")
    );

    Ok(())
}

pub async fn handle_overlay_link(config: &Config, path: PathBuf, create: bool) -> Result<()> {
    use anyhow::Context as _;

    let raw = path.to_string_lossy();
    let expanded = config::full_expand(&raw).with_context(|| format!("expanding path: {raw}"))?;
    let expanded = PathBuf::from(expanded);

    if create {
        tokio::fs::create_dir_all(&expanded)
            .await
            .with_context(|| format!("creating directory: {}", expanded.display()))?;
    }

    let meta = tokio::fs::metadata(&expanded).await.with_context(|| {
        if create {
            format!("accessing directory: {}", expanded.display())
        } else {
            format!(
                "path does not exist: {} (use --create to create it)",
                expanded.display()
            )
        }
    })?;

    if !meta.is_dir() {
        bail!("path is not a directory: {}", expanded.display());
    }

    let absolute = tokio::fs::canonicalize(&expanded).await.unwrap_or(expanded);

    config::validate_env_override_file(&absolute.join("shine.env.toml")).await?;

    if config
        .presets_overlay_dir_override
        .as_deref()
        .is_some_and(|p| p == absolute)
    {
        println!(
            "{}",
            colors::dim(&format!("overlay already linked: {}", absolute.display()))
        );
        return Ok(());
    }

    let updated = config
        .clone()
        .with_presets_overlay_dir_override(Some(absolute.clone()));
    updated.save().await?;

    println!("{}", colors::presets_overlay_note(&absolute));
    println!(
        "{}",
        colors::dim("Overlay files override the active presets source by matching path.")
    );

    Ok(())
}

pub async fn handle_overlay_unlink(config: &Config) -> Result<()> {
    if config.presets_overlay_dir_override.is_none() {
        println!(
            "{}",
            colors::dim("No presets overlay directory is configured.")
        );
        return Ok(());
    }

    let updated = config.clone().with_presets_overlay_dir_override(None);
    updated.save().await?;

    println!(
        "{}",
        colors::green("Presets overlay directory removed from the active config.")
    );
    println!(
        "{}",
        colors::dim("Built-in embedded presets will be used without overlay on the next run.")
    );

    Ok(())
}

pub fn handle_overlay_show(config: &Config) -> Result<()> {
    if let Some(dir) = &config.presets_overlay_dir_override {
        println!("{}", colors::presets_overlay_note(dir));
        println!("{}", colors::green("Active"));
    } else {
        println!(
            "{}",
            colors::dim("No presets overlay directory is configured.")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::env_lock;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tokio::fs;

    async fn make_temp_dir() -> PathBuf {
        crate::test_support::make_temp_dir("shine-presets-commands-test").await
    }

    fn config_in(dir: &std::path::Path) -> Config {
        crate::test_support::test_config(dir)
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn overlay_link_rejects_invalid_env_without_saving_link() {
        let _guard = env_lock();
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("shine-overlay-link-{suffix}"));
        let state_dir = root.join("state");
        let overlay_dir = root.join("overlay");
        tokio::fs::create_dir_all(&overlay_dir).await.unwrap();
        tokio::fs::write(
            overlay_dir.join("shine.env.toml"),
            "INVALID = \"unterminated\n",
        )
        .await
        .unwrap();

        // SAFETY: env_lock serializes process-global environment changes in tests.
        unsafe {
            std::env::set_var("SHINE_CONFIG_DIR", &state_dir);
            std::env::remove_var("SHINE_PRESETS");
        }
        let config = Config::load_or_init().await.unwrap();
        let error = handle_overlay_link(&config, overlay_dir.clone(), false)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("shine.env.toml"));

        let saved = tokio::fs::read_to_string(state_dir.join("config.toml"))
            .await
            .unwrap();
        assert!(!saved.contains("presets_overlay_dir"));

        // SAFETY: env_lock serializes process-global environment changes in tests.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn link_writes_presets_dir_to_config() {
        let dir = make_temp_dir().await;
        let presets = make_temp_dir().await;
        let config = config_in(&dir);

        handle_presets_link(&config, presets.clone(), false)
            .await
            .unwrap();

        let content = fs::read_to_string(dir.join("config.toml")).await.unwrap();
        assert!(
            content.contains(presets.to_str().unwrap()),
            "config.toml should contain the linked path"
        );

        fs::remove_dir_all(&dir).await.unwrap();
        fs::remove_dir_all(&presets).await.unwrap();
    }

    #[tokio::test]
    async fn link_creates_dir_when_create_flag_set() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);
        let new_dir = dir.join("new-presets");

        handle_presets_link(&config, new_dir.clone(), true)
            .await
            .unwrap();

        assert!(new_dir.exists(), "directory should have been created");
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn link_fails_when_path_missing_and_no_create() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);
        let missing = dir.join("does-not-exist");

        let err = handle_presets_link(&config, missing, false).await;
        assert!(err.is_err());
        let msg = err.unwrap_err().to_string();
        assert!(
            msg.contains("--create") || msg.contains("does not exist"),
            "error should mention --create: {msg}"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn link_fails_when_path_is_a_file() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);
        let file = dir.join("not-a-dir.txt");
        fs::write(&file, b"hello").await.unwrap();

        let err = handle_presets_link(&config, file, false).await;
        assert!(err.is_err());
        assert!(
            err.unwrap_err().to_string().contains("not a directory"),
            "error should mention 'not a directory'"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn link_is_noop_when_already_linked_to_same_path() {
        let dir = make_temp_dir().await;
        let presets = make_temp_dir().await;
        let abs = tokio::fs::canonicalize(&presets)
            .await
            .unwrap_or(presets.clone());
        let config = config_in(&dir).with_presets_dir_override(Some(abs.clone()));

        // Should return Ok without error
        handle_presets_link(&config, presets.clone(), false)
            .await
            .unwrap();

        // Config file should not be written (config_in has no pre-existing file)
        assert!(!dir.join("config.toml").exists());

        fs::remove_dir_all(&dir).await.unwrap();
        fs::remove_dir_all(&presets).await.unwrap();
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn link_warns_when_env_var_overrides() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        let presets = make_temp_dir().await;
        let config = config_in(&dir);

        // SAFETY: `_guard` holds `env_lock()`, serialising SHINE_PRESETS mutations across test threads.
        unsafe { std::env::set_var("SHINE_PRESETS", "/some/override") };
        // Should succeed even with env var set
        handle_presets_link(&config, presets.clone(), false)
            .await
            .unwrap();
        // SAFETY: `_guard` holds `env_lock()`, serialising SHINE_PRESETS mutations across test threads.
        unsafe { std::env::remove_var("SHINE_PRESETS") };

        fs::remove_dir_all(&dir).await.unwrap();
        fs::remove_dir_all(&presets).await.unwrap();
    }

    #[tokio::test]
    async fn unlink_removes_presets_dir_key() {
        let dir = make_temp_dir().await;
        let presets = make_temp_dir().await;
        let config = config_in(&dir).with_presets_dir_override(Some(presets.clone()));
        // Write initial config with presets_dir set
        config.save().await.unwrap();

        handle_presets_unlink(&config).await.unwrap();

        let content = fs::read_to_string(dir.join("config.toml")).await.unwrap();
        let parsed: toml::Table = toml::from_str(&content).unwrap();
        assert!(
            !parsed.contains_key("presets_dir"),
            "presets_dir key must be absent after unlink"
        );

        fs::remove_dir_all(&dir).await.unwrap();
        fs::remove_dir_all(&presets).await.unwrap();
    }

    #[tokio::test]
    async fn unlink_is_noop_when_no_override_set() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);

        // Should return Ok, no file written
        handle_presets_unlink(&config).await.unwrap();
        assert!(!dir.join("config.toml").exists());

        fs::remove_dir_all(&dir).await.unwrap();
    }
}
