use crate::colors;
use crate::config::{CURRENT_RUNTIME_SCHEMA_VERSION, Config};
use anyhow::{Context, Result, bail};
use std::future::Future;
use std::pin::Pin;

const UPDATE_CACHE_FILE: &str = "update-check.json";

pub async fn handle_clear(config: &Config, dry_run: bool) -> Result<()> {
    let schema_version = config.schema_version;
    if schema_version > CURRENT_RUNTIME_SCHEMA_VERSION {
        bail!(
            "runtime schema {} is newer than this shine supports ({})",
            schema_version,
            CURRENT_RUNTIME_SCHEMA_VERSION
        );
    }

    let steps = pending_steps(schema_version);
    if steps.is_empty() {
        if !dry_run && config.last_cleared_schema_version != Some(CURRENT_RUNTIME_SCHEMA_VERSION) {
            let mut updated = config.clone();
            updated.last_cleared_schema_version = Some(CURRENT_RUNTIME_SCHEMA_VERSION);
            updated.save().await?;
        }
        println!(
            "{}",
            colors::green(&format!(
                "Runtime config schema is already current ({CURRENT_RUNTIME_SCHEMA_VERSION})."
            ))
        );
        return Ok(());
    }

    println!("{}", colors::bold("Clearing old runtime state"));
    crate::config::print_presets_note(config);

    for step in &steps {
        println!("{}", colors::dim(&format!("schema {}", step.to_version)));
        for action in actions_for_step(config, step) {
            if dry_run {
                println!("  [dry-run] {}", action.description);
            } else {
                println!("  {}", action.description);
                (action.apply).await?;
            }
        }
    }

    if dry_run {
        println!();
        println!(
            "{}",
            colors::dim("Dry run only. Run `shine clear` to apply these changes.")
        );
        return Ok(());
    }

    let mut updated = config.clone();
    updated.schema_version = CURRENT_RUNTIME_SCHEMA_VERSION;
    updated.last_cleared_schema_version = Some(CURRENT_RUNTIME_SCHEMA_VERSION);
    updated.save().await?;

    println!();
    println!(
        "{}",
        colors::green(&format!(
            "Runtime config schema is now {CURRENT_RUNTIME_SCHEMA_VERSION}."
        ))
    );
    Ok(())
}

pub fn pending_schema_warning(schema_version: u32) -> Option<String> {
    if schema_version >= CURRENT_RUNTIME_SCHEMA_VERSION {
        return None;
    }

    Some(format!(
        "Runtime config schema is behind: {schema_version} -> {CURRENT_RUNTIME_SCHEMA_VERSION}. Run `shine clear --dry-run` to inspect cleanup, then `shine clear`."
    ))
}

struct CleanupStep {
    to_version: u32,
}

struct CleanupAction<'a> {
    description: String,
    apply: Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>,
}

fn pending_steps(schema_version: u32) -> Vec<CleanupStep> {
    ((schema_version + 1)..=CURRENT_RUNTIME_SCHEMA_VERSION)
        .map(|to_version| CleanupStep { to_version })
        .collect()
}

fn actions_for_step<'a>(config: &'a Config, step: &CleanupStep) -> Vec<CleanupAction<'a>> {
    match step.to_version {
        1 => {
            let path = config.shine_dir().join(UPDATE_CACHE_FILE);
            vec![CleanupAction {
                description: format!("remove stale update cache {}", path.display()),
                apply: Box::pin(async move {
                    match tokio::fs::remove_file(&path).await {
                        Ok(()) => Ok(()),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
                    }
                }),
            }]
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::fs;

    async fn make_temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("shine-clear-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).await.unwrap();
        dir
    }

    #[test]
    fn pending_schema_warning_reports_old_schema() {
        let warning = pending_schema_warning(0).unwrap();
        assert!(warning.contains("0 -> 1"));
        assert!(pending_schema_warning(CURRENT_RUNTIME_SCHEMA_VERSION).is_none());
    }

    #[tokio::test]
    async fn clear_removes_update_cache_and_records_schema() {
        let dir = make_temp_dir().await;
        let mut config = Config::new_for_test(&dir);
        config.schema_version = 0;
        fs::write(dir.join(UPDATE_CACHE_FILE), b"stale")
            .await
            .unwrap();

        handle_clear(&config, false).await.unwrap();

        assert!(!dir.join(UPDATE_CACHE_FILE).exists());
        let content = fs::read_to_string(dir.join("config.toml")).await.unwrap();
        let parsed: toml::Table = toml::from_str(&content).unwrap();
        assert_eq!(
            parsed["schema_version"].as_integer(),
            Some(CURRENT_RUNTIME_SCHEMA_VERSION.into())
        );
        assert_eq!(
            parsed["last_cleared_schema_version"].as_integer(),
            Some(CURRENT_RUNTIME_SCHEMA_VERSION.into())
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn dry_run_does_not_remove_or_save() {
        let dir = make_temp_dir().await;
        let mut config = Config::new_for_test(&dir);
        config.schema_version = 0;
        fs::write(dir.join(UPDATE_CACHE_FILE), b"stale")
            .await
            .unwrap();

        handle_clear(&config, true).await.unwrap();

        assert!(dir.join(UPDATE_CACHE_FILE).exists());
        assert!(!dir.join("config.toml").exists());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn clear_records_last_cleared_when_already_current() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);

        handle_clear(&config, false).await.unwrap();

        let content = fs::read_to_string(dir.join("config.toml")).await.unwrap();
        let parsed: toml::Table = toml::from_str(&content).unwrap();
        assert_eq!(
            parsed["schema_version"].as_integer(),
            Some(CURRENT_RUNTIME_SCHEMA_VERSION.into())
        );
        assert_eq!(
            parsed["last_cleared_schema_version"].as_integer(),
            Some(CURRENT_RUNTIME_SCHEMA_VERSION.into())
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn clear_does_not_remove_other_runtime_files() {
        let dir = make_temp_dir().await;
        let mut config = Config::new_for_test(&dir);
        config.schema_version = 0;
        fs::create_dir_all(dir.join("rendered")).await.unwrap();
        fs::create_dir_all(dir.join("bin")).await.unwrap();
        fs::create_dir_all(dir.join("presets")).await.unwrap();
        fs::write(dir.join("app-manifest.toml"), b"entries = []")
            .await
            .unwrap();

        handle_clear(&config, false).await.unwrap();

        assert!(dir.join("rendered").exists());
        assert!(dir.join("bin").exists());
        assert!(dir.join("presets").exists());
        assert!(dir.join("app-manifest.toml").exists());

        fs::remove_dir_all(&dir).await.unwrap();
    }
}
