use crate::colors;
use crate::config::{CURRENT_RUNTIME_SCHEMA_VERSION, Config};
use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

const UPDATE_CACHE_FILE: &str = "update-check.json";

pub async fn handle_migrate(config: &Config, dry_run: bool) -> Result<()> {
    let schema_version = config.schema_version;
    if schema_version > CURRENT_RUNTIME_SCHEMA_VERSION {
        bail!(
            "runtime schema {} is newer than this shine supports ({})",
            schema_version,
            CURRENT_RUNTIME_SCHEMA_VERSION
        );
    }

    // Validate every file in the migration scope before applying any cleanup
    // or recipient rewrite so one malformed legacy recipient cannot leave a
    // partially migrated global/project/workspace set.
    validate_recipient_migration_inputs(config).await?;

    let steps = pending_steps(schema_version);
    if steps.is_empty() {
        let mut migrated_local_state = false;
        for path in config_migration_paths(config) {
            if config_needs_migration(&path).await? {
                let age_count = config_age_migration_count(&path).await?;
                if dry_run {
                    println!(
                        "[dry-run] migrate recipient configuration in {} ({age_count} age1se recipient(s) to age1tag)",
                        path.display(),
                    );
                } else {
                    migrate_config_gpg_recipient(&path).await?;
                    println!(
                        "Migrated recipient configuration in {} ({age_count} age1se recipient(s) to age1tag)",
                        path.display()
                    );
                }
                if age_count > 0 {
                    println!(
                        "  Note: native tagged recipients let someone who knows the recipient test whether a ciphertext targets it."
                    );
                }
                migrated_local_state = true;
            }
        }
        if let Some(path) = find_workspace_from_current_dir()
            && workspace_needs_migration(&path).await?
        {
            let age_count = workspace_age_migration_count(&path).await?;
            if dry_run {
                println!(
                    "[dry-run] migrate workspace format and recipients in {} ({age_count} age1se recipient(s) to age1tag)",
                    path.display()
                );
            } else {
                migrate_workspace_gpg_recipient(&path).await?;
                println!(
                    "Migrated workspace format and recipients in {} ({age_count} age1se recipient(s) to age1tag)",
                    path.display()
                );
            }
            if age_count > 0 {
                println!(
                    "  Note: native tagged recipients let someone who knows the recipient test whether a ciphertext targets it."
                );
            }
            migrated_local_state = true;
        }
        if migrated_local_state {
            return Ok(());
        }
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

    println!("{}", colors::bold("Migrating old runtime state"));
    crate::config::print_presets_note(config);

    for step in &steps {
        println!("{}", colors::dim(&format!("schema {}", step.to_version)));
        for action in actions_for_step(config, step).await? {
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
            colors::dim("Dry run only. Run `shine state migrate` to apply these changes.")
        );
        return Ok(());
    }

    let mut updated = config.clone();
    if let Some(recipients) = gpg_recipients_from_config(config.config_path()).await? {
        updated.gpg_recipients = recipients;
    }
    if let Some(recipients) = age_recipients_from_config(config.config_path()).await? {
        updated.age_recipients = recipients;
    }
    updated.legacy_gpg_key_id = None;
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
        "Runtime config schema is behind: {schema_version} -> {CURRENT_RUNTIME_SCHEMA_VERSION}. Run `shine state migrate --dry-run` to inspect cleanup, then `shine state migrate`."
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

async fn actions_for_step<'a>(
    config: &'a Config,
    step: &CleanupStep,
) -> Result<Vec<CleanupAction<'a>>> {
    Ok(match step.to_version {
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
        2 => migration_actions(config).await?,
        _ => Vec::new(),
    })
}

async fn migration_actions<'a>(config: &'a Config) -> Result<Vec<CleanupAction<'a>>> {
    let mut actions = Vec::new();
    for path in config_migration_paths(config) {
        let age_count = config_age_migration_count(&path).await?;
        let description = migration_description("recipient configuration", &path, age_count);
        actions.push(CleanupAction {
            description,
            apply: Box::pin(async move { migrate_config_gpg_recipient(&path).await }),
        });
    }
    if let Some(path) = find_workspace_from_current_dir() {
        let age_count = workspace_age_migration_count(&path).await?;
        let description =
            migration_description("workspace recipient configuration", &path, age_count);
        actions.push(CleanupAction {
            description,
            apply: Box::pin(
                async move { migrate_workspace_gpg_recipient(&path).await.map(|_| ()) },
            ),
        });
    }
    Ok(actions)
}

fn migration_description(kind: &str, path: &Path, age_count: usize) -> String {
    let mut description = format!(
        "migrate {kind} in {} ({age_count} age1se recipient(s) to age1tag)",
        path.display()
    );
    if age_count > 0 {
        description.push_str(
            "; tagged recipients let someone who knows the recipient test whether a ciphertext targets it",
        );
    }
    description
}

fn config_migration_paths(config: &Config) -> Vec<PathBuf> {
    let mut paths = BTreeSet::from([config.config_path().to_path_buf()]);
    if let Ok(current_dir) = std::env::current_dir()
        && let Some(project) = crate::config::find_project_config(&current_dir)
    {
        paths.insert(project.path);
    }
    paths.into_iter().filter(|path| path.is_file()).collect()
}

fn find_workspace_from_current_dir() -> Option<PathBuf> {
    let current_dir = std::env::current_dir().ok()?;
    current_dir
        .ancestors()
        .map(|dir| dir.join("shine.workspace.toml"))
        .find(|path| path.is_file())
}

async fn validate_recipient_migration_inputs(config: &Config) -> Result<()> {
    for path in config_migration_paths(config) {
        config_age_migration_count(&path)
            .await
            .with_context(|| format!("validating recipient migration in {}", path.display()))?;
    }
    if let Some(path) = find_workspace_from_current_dir() {
        workspace_age_migration_count(&path)
            .await
            .with_context(|| format!("validating recipient migration in {}", path.display()))?;
    }
    Ok(())
}

async fn migrate_config_gpg_recipient(path: &Path) -> Result<()> {
    migrate_recipient_key(path, |document| {
        let mut changed = migrate_key(document, "gpg_key_id", "gpg_recipients")?;
        changed |= migrate_age_recipients(document.as_table_mut())? > 0;
        Ok(changed)
    })
    .await
    .map(|_| ())
}

async fn migrate_workspace_gpg_recipient(path: &Path) -> Result<bool> {
    migrate_recipient_key(path, |document| {
        let version = document
            .as_table()
            .get("version")
            .and_then(toml_edit::Item::as_integer)
            .unwrap_or(1);
        if version > 2 {
            bail!("workspace version {version} is newer than this shine supports (2)");
        }
        let mut changed = false;
        let encryption = document["env"]["encryption"].as_table_mut();
        if let Some(table) = encryption {
            changed |= migrate_table_key(table, "recipient", "gpg_recipients")?;
            changed |= migrate_age_recipients(table)? > 0;
        }
        if version < 2 {
            document["version"] = toml_edit::value(2);
            changed = true;
        }
        Ok(changed)
    })
    .await
}

async fn migrate_recipient_key(
    path: &Path,
    mutate: impl FnOnce(&mut toml_edit::DocumentMut) -> Result<bool>,
) -> Result<bool> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    let mut document = contents
        .parse::<toml_edit::DocumentMut>()
        .with_context(|| format!("parsing {}", path.display()))?;
    if mutate(&mut document)? {
        crate::persist::atomic_write(path, document.to_string().as_bytes())
            .await
            .with_context(|| format!("writing {}", path.display()))?;
        return Ok(true);
    }
    Ok(false)
}

async fn workspace_needs_migration(path: &Path) -> Result<bool> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    let document = contents
        .parse::<toml_edit::DocumentMut>()
        .with_context(|| format!("parsing {}", path.display()))?;
    Ok(document
        .as_table()
        .get("version")
        .and_then(toml_edit::Item::as_integer)
        .unwrap_or(1)
        < 2
        || document["env"]["encryption"]["recipient"].is_value()
        || age_migration_count(
            document["env"]["encryption"]
                .as_table()
                .and_then(|table| table.get("age_recipients")),
        )? > 0)
}

async fn config_needs_migration(path: &Path) -> Result<bool> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    let document = contents
        .parse::<toml_edit::DocumentMut>()
        .with_context(|| format!("parsing {}", path.display()))?;
    Ok(document.as_table().contains_key("gpg_key_id")
        || age_migration_count(document.as_table().get("age_recipients"))? > 0)
}

async fn config_age_migration_count(path: &Path) -> Result<usize> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    let document = contents
        .parse::<toml_edit::DocumentMut>()
        .with_context(|| format!("parsing {}", path.display()))?;
    age_migration_count(document.as_table().get("age_recipients"))
}

async fn workspace_age_migration_count(path: &Path) -> Result<usize> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    let document = contents
        .parse::<toml_edit::DocumentMut>()
        .with_context(|| format!("parsing {}", path.display()))?;
    age_migration_count(
        document["env"]["encryption"]
            .as_table()
            .and_then(|table| table.get("age_recipients")),
    )
}

fn age_migration_count(item: Option<&toml_edit::Item>) -> Result<usize> {
    let Some(item) = item else {
        return Ok(0);
    };
    let recipients = item.as_array().context("age_recipients must be an array")?;
    let mut count = 0;
    for value in recipients {
        let recipient = value
            .as_str()
            .context("age_recipients entries must be strings")?;
        if recipient.starts_with("age1se1") {
            crate::secret::secure_enclave_recipient_to_tag(recipient)?;
            count += 1;
        }
    }
    Ok(count)
}

fn migrate_age_recipients(table: &mut toml_edit::Table) -> Result<usize> {
    let Some(item) = table.get_mut("age_recipients") else {
        return Ok(0);
    };
    let recipients = item
        .as_array_mut()
        .context("age_recipients must be an array")?;
    let mut count = 0;
    for value in recipients.iter_mut() {
        let recipient = value
            .as_str()
            .context("age_recipients entries must be strings")?;
        if recipient.starts_with("age1se1") {
            let tagged = crate::secret::secure_enclave_recipient_to_tag(recipient)?;
            let decor = value.decor().clone();
            *value = toml_edit::Value::from(tagged);
            *value.decor_mut() = decor;
            count += 1;
        }
    }
    Ok(count)
}

fn migrate_key(document: &mut toml_edit::DocumentMut, old: &str, new: &str) -> Result<bool> {
    migrate_table_key(document.as_table_mut(), old, new)
}

fn migrate_table_key(table: &mut toml_edit::Table, old: &str, new: &str) -> Result<bool> {
    let Some(old_item) = table.get(old) else {
        return Ok(false);
    };
    if table.contains_key(new) {
        bail!("configuration contains both {old} and {new}; resolve the conflict before migrating");
    }
    let recipient = old_item
        .as_str()
        .context("legacy GPG recipient must be a string")?
        .to_owned();
    let key_decor = table.key(old).map(|key| key.leaf_decor().clone());
    let decor = old_item.as_value().map(|value| value.decor().clone());
    table.remove(old);
    let mut recipients = toml_edit::Array::new();
    recipients.push(recipient);
    table.insert(
        new,
        toml_edit::Item::Value(toml_edit::Value::Array(recipients)),
    );
    if let (Some(decor), Some(value)) = (
        decor,
        table.get_mut(new).and_then(toml_edit::Item::as_value_mut),
    ) {
        *value.decor_mut() = decor;
    }
    if let (Some(decor), Some(mut key)) = (key_decor, table.key_mut(new)) {
        *key.leaf_decor_mut() = decor;
    }
    Ok(true)
}

async fn gpg_recipients_from_config(path: &Path) -> Result<Option<Vec<String>>> {
    recipients_from_config(path, "gpg_recipients").await
}

async fn age_recipients_from_config(path: &Path) -> Result<Option<Vec<String>>> {
    recipients_from_config(path, "age_recipients").await
}

async fn recipients_from_config(path: &Path, key: &str) -> Result<Option<Vec<String>>> {
    let contents = match tokio::fs::read_to_string(path).await {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let table: toml::Table =
        toml::from_str(&contents).with_context(|| format!("parsing {}", path.display()))?;
    let Some(value) = table.get(key) else {
        return Ok(None);
    };
    let recipients = value
        .as_array()
        .with_context(|| format!("{key} must be an array"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .with_context(|| format!("{key} entries must be strings"))
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .map(str::to_owned)
        .collect();
    Ok(Some(recipients))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::fs;

    async fn make_temp_dir() -> std::path::PathBuf {
        crate::test_support::make_temp_dir("shine-state-migrate").await
    }

    #[test]
    fn pending_schema_warning_reports_old_schema() {
        let warning = pending_schema_warning(0).unwrap();
        assert!(warning.contains("0 -> 2"));
        assert!(pending_schema_warning(CURRENT_RUNTIME_SCHEMA_VERSION).is_none());
    }

    #[tokio::test]
    async fn migrate_removes_update_cache_and_records_schema() {
        let dir = make_temp_dir().await;
        let mut config = Config::new_for_test(&dir);
        config.schema_version = 0;
        fs::write(dir.join(UPDATE_CACHE_FILE), b"stale")
            .await
            .unwrap();

        handle_migrate(&config, false).await.unwrap();

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
    async fn migrate_converts_legacy_gpg_recipient_to_a_list() {
        let dir = make_temp_dir().await;
        let mut config = Config::new_for_test(&dir);
        config.schema_version = 1;
        fs::write(
            config.config_path(),
            "# team encryption key\ngpg_key_id = \"alice@example.com\"\n",
        )
        .await
        .unwrap();

        handle_migrate(&config, false).await.unwrap();

        let content = fs::read_to_string(config.config_path()).await.unwrap();
        assert!(!content.contains("gpg_key_id"));
        assert!(content.contains("# team encryption key"));
        let parsed: toml::Table = toml::from_str(&content).unwrap();
        assert_eq!(
            parsed["gpg_recipients"].as_array().unwrap(),
            &[toml::Value::String("alice@example.com".to_string())]
        );
        assert_eq!(
            parsed["schema_version"].as_integer(),
            Some(CURRENT_RUNTIME_SCHEMA_VERSION.into())
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn workspace_migration_converts_legacy_recipient() {
        let dir = make_temp_dir().await;
        let workspace = dir.join("shine.workspace.toml");
        let legacy = "age1se1qgg72x2qfk9wg3wh0qg9u0v7l5dkq4jx69fv80p6wdus3ftg6flwg5dz2dp";
        let tagged = "age1tag1qgg72x2qfk9wg3wh0qg9u0v7l5dkq4jx69fv80p6wdus3ftg6flwgc25f05";
        fs::write(
            &workspace,
            format!(
                "version = 2\n[env.encryption]\n# deployment key\nrecipient = \"alice@example.com\"\nage_recipients = [\"{legacy}\"]\n"
            ),
        )
        .await
        .unwrap();

        migrate_workspace_gpg_recipient(&workspace).await.unwrap();

        let content = fs::read_to_string(&workspace).await.unwrap();
        assert!(!content.contains("recipient ="));
        assert!(content.contains("# deployment key"));
        assert!(content.contains("gpg_recipients = [\"alice@example.com\"]"));
        assert!(content.contains(tagged));
        assert!(content.contains("version = 2"));
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn migration_converts_secure_enclave_recipients_in_place() {
        let dir = make_temp_dir().await;
        let config_path = dir.join("config.toml");
        let legacy = "age1se1qgg72x2qfk9wg3wh0qg9u0v7l5dkq4jx69fv80p6wdus3ftg6flwg5dz2dp";
        let tagged = "age1tag1qgg72x2qfk9wg3wh0qg9u0v7l5dkq4jx69fv80p6wdus3ftg6flwgc25f05";
        fs::write(
            &config_path,
            format!("# team recipients\nage_recipients = [\n  \"{legacy}\", # Touch ID\n  \"age1phone1example\",\n]\n"),
        )
        .await
        .unwrap();

        migrate_config_gpg_recipient(&config_path).await.unwrap();

        let content = fs::read_to_string(&config_path).await.unwrap();
        assert!(content.contains(tagged));
        assert!(content.contains("# Touch ID"));
        assert!(content.contains("age1phone1example"));
        assert!(!content.contains(legacy));
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn invalid_secure_enclave_recipient_leaves_file_unchanged() {
        let dir = make_temp_dir().await;
        let config_path = dir.join("config.toml");
        let original = "age_recipients = [\"age1se1qgg72x2qfk9wg3wh0qg9u0v7l5dkq4jx69fv80p6wdus3ftg6flwg5dz2dp\", \"age1se1invalid\"]\n";
        fs::write(&config_path, original).await.unwrap();

        assert!(migrate_config_gpg_recipient(&config_path).await.is_err());
        assert_eq!(fs::read_to_string(&config_path).await.unwrap(), original);
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn invalid_project_recipient_prevents_any_config_write() {
        let _lock = crate::test_support::env_lock();
        let original_dir = std::env::current_dir().unwrap();
        let dir = make_temp_dir().await;
        let project_dir = dir.join("project");
        fs::create_dir_all(&project_dir).await.unwrap();
        let config = Config::new_for_test(&dir);
        let legacy = "age1se1qgg72x2qfk9wg3wh0qg9u0v7l5dkq4jx69fv80p6wdus3ftg6flwg5dz2dp";
        let global_original = format!("schema_version = 2\nage_recipients = [\"{legacy}\"]\n");
        fs::write(config.config_path(), &global_original)
            .await
            .unwrap();
        fs::write(
            project_dir.join("shine.config.toml"),
            "age_recipients = [\"age1se1invalid\"]\n",
        )
        .await
        .unwrap();
        std::env::set_current_dir(&project_dir).unwrap();

        let result = handle_migrate(&config, false).await;
        crate::test_support::restore_current_dir(&original_dir);

        assert!(result.is_err());
        assert_eq!(
            fs::read_to_string(config.config_path()).await.unwrap(),
            global_original
        );
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn dry_run_does_not_convert_secure_enclave_recipients() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        let legacy = "age1se1qgg72x2qfk9wg3wh0qg9u0v7l5dkq4jx69fv80p6wdus3ftg6flwg5dz2dp";
        let original = format!("schema_version = 2\nage_recipients = [\"{legacy}\"]\n");
        fs::write(config.config_path(), &original).await.unwrap();

        handle_migrate(&config, true).await.unwrap();

        assert_eq!(
            fs::read_to_string(config.config_path()).await.unwrap(),
            original
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

        handle_migrate(&config, true).await.unwrap();

        assert!(dir.join(UPDATE_CACHE_FILE).exists());
        assert!(!dir.join("config.toml").exists());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn migrate_records_last_cleared_when_already_current() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);

        handle_migrate(&config, false).await.unwrap();

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
    #[allow(clippy::await_holding_lock)]
    async fn migrate_current_schema_converts_legacy_project_config() {
        let _lock = crate::test_support::env_lock();
        let original_dir = std::env::current_dir().unwrap();
        let dir = make_temp_dir().await;
        let project_dir = dir.join("project");
        fs::create_dir_all(&project_dir).await.unwrap();
        let project_config = project_dir.join("shine.config.toml");
        let legacy = "age1se1qgg72x2qfk9wg3wh0qg9u0v7l5dkq4jx69fv80p6wdus3ftg6flwg5dz2dp";
        let tagged = "age1tag1qgg72x2qfk9wg3wh0qg9u0v7l5dkq4jx69fv80p6wdus3ftg6flwgc25f05";
        fs::write(
            &project_config,
            format!(
                "# project key\ngpg_key_id = \"project@example.com\"\nage_recipients = [\"{legacy}\"]\n"
            ),
        )
        .await
        .unwrap();
        std::env::set_current_dir(&project_dir).unwrap();
        let config = Config::new_for_test(&dir);

        let result = handle_migrate(&config, false).await;
        crate::test_support::restore_current_dir(&original_dir);
        result.unwrap();

        let content = fs::read_to_string(&project_config).await.unwrap();
        assert!(!content.contains("gpg_key_id"));
        assert!(content.contains("# project key"));
        assert!(content.contains("gpg_recipients = [\"project@example.com\"]"));
        assert!(content.contains(tagged));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn migrate_does_not_remove_other_runtime_files() {
        let dir = make_temp_dir().await;
        let mut config = Config::new_for_test(&dir);
        config.schema_version = 0;
        fs::create_dir_all(dir.join("rendered")).await.unwrap();
        fs::create_dir_all(dir.join("bin")).await.unwrap();
        fs::create_dir_all(dir.join("presets")).await.unwrap();
        fs::write(dir.join("app-manifest.toml"), b"entries = []")
            .await
            .unwrap();

        handle_migrate(&config, false).await.unwrap();

        assert!(dir.join("rendered").exists());
        assert!(dir.join("bin").exists());
        assert!(dir.join("presets").exists());
        assert!(dir.join("app-manifest.toml").exists());

        fs::remove_dir_all(&dir).await.unwrap();
    }
}
