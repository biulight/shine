use crate::colors;
use crate::config::{CURRENT_RUNTIME_SCHEMA_VERSION, Config};
use crate::output::escape_controls;
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
    let targets = inspect_recipient_targets(config).await?;

    let steps = pending_steps(schema_version);
    if steps.is_empty() {
        let mut migrated_local_state = false;
        for target in targets.iter().filter(|target| target.changes.changed) {
            let kind = match target.kind {
                RecipientFile::Config => "recipient configuration",
                RecipientFile::Workspace => "workspace format and recipients",
            };
            let (verb, age_count) = if dry_run {
                ("[dry-run] migrate", target.changes.age_count)
            } else {
                let applied = target.kind.apply(&target.path).await?;
                if !applied.changed {
                    continue;
                }
                ("Migrated", applied.age_count)
            };
            println!(
                "{verb} {kind} in {} ({age_count} age1se recipient(s) to age1tag)",
                escape_controls(&target.path.display().to_string())
            );
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
            save_runtime_schema_state(config).await?;
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
        for action in actions_for_step(config, step, &targets) {
            if dry_run {
                println!("  [dry-run] {}", escape_controls(&action.description));
            } else {
                println!("  {}", escape_controls(&action.description));
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

    save_runtime_schema_state(config).await?;

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

fn actions_for_step<'a>(
    config: &'a Config,
    step: &CleanupStep,
    targets: &'a [RecipientTarget],
) -> Vec<CleanupAction<'a>> {
    match step.to_version {
        1 => {
            let path = config.shine_dir().join(UPDATE_CACHE_FILE);
            vec![CleanupAction {
                description: format!("remove stale update cache {}", path.display()),
                apply: Box::pin(async move {
                    match tokio::fs::remove_file(&path).await {
                        Ok(()) => Ok(()),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                        Err(e) => Err(e).with_context(|| {
                            format!("removing {}", escape_controls(&path.display().to_string()))
                        }),
                    }
                }),
            }]
        }
        2 => targets
            .iter()
            .map(|target| {
                let kind = match target.kind {
                    RecipientFile::Config => "recipient configuration",
                    RecipientFile::Workspace => "workspace recipient configuration",
                };
                CleanupAction {
                    description: migration_description(
                        kind,
                        &target.path,
                        target.changes.age_count,
                    ),
                    apply: Box::pin(
                        async move { target.kind.apply(&target.path).await.map(|_| ()) },
                    ),
                }
            })
            .collect(),
        _ => Vec::new(),
    }
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

struct RecipientTarget {
    kind: RecipientFile,
    path: PathBuf,
    changes: RecipientChanges,
}

async fn inspect_recipient_targets(config: &Config) -> Result<Vec<RecipientTarget>> {
    let configs = config_migration_paths(config)
        .into_iter()
        .map(|path| (RecipientFile::Config, path));
    let workspace = find_workspace_from_current_dir().map(|path| (RecipientFile::Workspace, path));
    let mut targets = Vec::new();
    for (kind, path) in configs.chain(workspace) {
        let changes = kind.inspect(&path).await.with_context(|| {
            format!(
                "validating recipient migration in {}",
                escape_controls(&path.display().to_string())
            )
        })?;
        targets.push(RecipientTarget {
            kind,
            path,
            changes,
        });
    }
    Ok(targets)
}

#[derive(Clone, Copy)]
enum RecipientFile {
    Config,
    Workspace,
}

#[derive(Debug, Default)]
struct RecipientChanges {
    changed: bool,
    age_count: usize,
}

impl RecipientFile {
    async fn prepare(self, path: &Path) -> Result<(toml_edit::DocumentMut, RecipientChanges)> {
        let contents = crate::config::ConfigInput::capture(path)
            .await?
            .contents
            .context("recipient migration target disappeared; retry")?;
        let mut document = parse_migration_document(&contents, path)?;
        let changes = match self {
            Self::Config => migrate_recipients(document.as_table_mut(), "gpg_key_id")?,
            Self::Workspace => migrate_workspace_document(&mut document)?,
        };
        Ok((document, changes))
    }

    async fn inspect(self, path: &Path) -> Result<RecipientChanges> {
        self.prepare(path).await.map(|(_, changes)| changes)
    }

    async fn apply(self, path: &Path) -> Result<RecipientChanges> {
        let _guard = match self {
            Self::Config => Some(crate::config::env_write::lock_target(path).await?),
            Self::Workspace => None,
        };
        let (document, changes) = self.prepare(path).await?;
        if changes.changed {
            crate::persist::atomic_write_private(path, document.to_string().as_bytes())
                .await
                .with_context(|| {
                    format!("writing {}", escape_controls(&path.display().to_string()))
                })?;
        }
        Ok(changes)
    }
}

fn migrate_workspace_document(document: &mut toml_edit::DocumentMut) -> Result<RecipientChanges> {
    let version = document
        .get("version")
        .map(|value| {
            value
                .as_integer()
                .filter(|version| *version >= 0)
                .context("workspace version must be a nonnegative integer")
        })
        .transpose()?
        .unwrap_or(1);
    if version > 3 {
        bail!("workspace version {version} is newer than this shine supports (3)");
    }
    let mut changes = RecipientChanges::default();
    if let Some(env) = document.get_mut("env") {
        let env = env
            .as_table_like_mut()
            .context("workspace env must be a table")?;
        if let Some(encryption) = env.get_mut("encryption") {
            let table = encryption
                .as_table_like_mut()
                .context("workspace env.encryption must be a table")?;
            changes = migrate_recipients(table, "recipient")?;
        }
    }
    if version < 2 {
        document["version"] = toml_edit::value(2);
        changes.changed = true;
    }
    Ok(changes)
}

fn migrate_recipients(
    table: &mut dyn toml_edit::TableLike,
    legacy_key: &str,
) -> Result<RecipientChanges> {
    let gpg_changed = migrate_table_key(table, legacy_key, "gpg_recipients")?;
    let age_count = migrate_age_recipients(table)?;
    Ok(RecipientChanges {
        changed: gpg_changed || age_count > 0,
        age_count,
    })
}

fn parse_migration_document(contents: &str, path: &Path) -> Result<toml_edit::DocumentMut> {
    // Parser errors retain source snippets and may echo values or duplicate keys.
    // Drop that chain, as environment-source parsing does, and keep only the file.
    contents.parse().map_err(|_| {
        anyhow::anyhow!(
            "invalid migration TOML syntax in {}",
            escape_controls(&path.display().to_string())
        )
    })
}

fn migrate_age_recipients(table: &mut dyn toml_edit::TableLike) -> Result<usize> {
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

fn migrate_table_key(table: &mut dyn toml_edit::TableLike, old: &str, new: &str) -> Result<bool> {
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

/// Migration already updated recipient fields under the target lock. Patch only
/// runtime schema fields from a fresh raw file, never re-save a stale Config map.
async fn save_runtime_schema_state(config: &Config) -> Result<()> {
    let path = config.global_config_path();
    let _guard = crate::config::env_write::lock_target(&path).await?;
    let input = crate::config::ConfigInput::capture(&path).await?;
    let mut document =
        parse_migration_document(input.contents.as_deref().unwrap_or_default(), &path)?;
    if document
        .get("schema_version")
        .and_then(toml_edit::Item::as_integer)
        .is_some_and(|v| v > i64::from(CURRENT_RUNTIME_SCHEMA_VERSION))
    {
        bail!(
            "runtime configuration changed to a newer schema; retry with a compatible Shine version"
        );
    }
    document["schema_version"] = toml_edit::value(i64::from(CURRENT_RUNTIME_SCHEMA_VERSION));
    document["last_cleared_schema_version"] =
        toml_edit::value(i64::from(CURRENT_RUNTIME_SCHEMA_VERSION));
    crate::persist::atomic_write_private(&path, document.to_string().as_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::fs;

    const LEGACY_RECIPIENT: &str =
        "age1se1qgg72x2qfk9wg3wh0qg9u0v7l5dkq4jx69fv80p6wdus3ftg6flwg5dz2dp";
    const TAGGED_RECIPIENT: &str =
        "age1tag1qgg72x2qfk9wg3wh0qg9u0v7l5dkq4jx69fv80p6wdus3ftg6flwgc25f05";

    async fn make_temp_dir() -> std::path::PathBuf {
        crate::test_support::make_temp_dir("shine-state-migrate").await
    }

    async fn assert_schema_recorded(path: &Path) {
        let content = fs::read_to_string(path).await.unwrap();
        let parsed: toml::Table = toml::from_str(&content).unwrap();
        for key in ["schema_version", "last_cleared_schema_version"] {
            assert_eq!(
                parsed[key].as_integer(),
                Some(CURRENT_RUNTIME_SCHEMA_VERSION.into()),
                "{key}"
            );
        }
    }

    async fn assert_workspace_current(path: &Path, expected: &str) {
        let changes = RecipientFile::Workspace.inspect(path).await.unwrap();
        assert_eq!(changes.age_count, 0);
        assert!(!changes.changed);
        assert!(!RecipientFile::Workspace.apply(path).await.unwrap().changed);
        assert_eq!(fs::read_to_string(path).await.unwrap(), expected);
    }

    #[tokio::test]
    async fn migration_document_readers_do_not_retain_source_in_errors() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        let path = config.config_path();
        for contents in [
            "[env]\nTOKEN = \"PRIVATE_MIGRATION_CANARY_UNCLOSED\n",
            "PRIVATE_MIGRATION_CANARY_KEY = 1\nPRIVATE_MIGRATION_CANARY_KEY = 2\n",
        ] {
            fs::write(path, contents).await.unwrap();
            let errors = [
                RecipientFile::Config.inspect(path).await.unwrap_err(),
                RecipientFile::Workspace.inspect(path).await.unwrap_err(),
                RecipientFile::Config.apply(path).await.unwrap_err(),
                RecipientFile::Workspace.apply(path).await.unwrap_err(),
                save_runtime_schema_state(&config).await.unwrap_err(),
            ];
            for error in errors {
                // Check the complete retained chain and Debug, not just the outer
                // context: anyhow prints the parser's source snippet on CLI failure.
                for diagnostic in [format!("{error:#}"), format!("{error:?}")] {
                    assert!(diagnostic.contains("invalid migration TOML syntax"));
                    assert!(diagnostic.contains("config.toml"));
                    assert!(
                        !diagnostic.contains("PRIVATE_MIGRATION_CANARY"),
                        "{diagnostic}"
                    );
                    assert!(!diagnostic.contains("TOKEN ="), "{diagnostic}");
                }
            }
            assert_eq!(fs::read_to_string(path).await.unwrap(), contents);
        }
        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn workspace_migration_readers_validate_table_shapes_without_changing_valid_inputs() {
        let dir = make_temp_dir().await;
        let path = dir.join("shine.workspace.toml");
        for (contents, field) in [
            ("version=2\nenv=1\n", "env"),
            ("version=2\nenv=[]\n", "env"),
            ("version=2\n[[env]]\n", "env"),
            ("version=2\n[env]\nencryption=1\n", "env.encryption"),
            ("version=2\n[env]\nencryption=[]\n", "env.encryption"),
            ("version=2\n[[env.encryption]]\n", "env.encryption"),
        ] {
            fs::write(&path, contents).await.unwrap();
            for error in [
                RecipientFile::Workspace.inspect(&path).await.unwrap_err(),
                RecipientFile::Workspace.apply(&path).await.unwrap_err(),
            ] {
                assert!(
                    error
                        .to_string()
                        .contains(&format!("workspace {field} must be a table"))
                );
            }
            assert_eq!(fs::read_to_string(&path).await.unwrap(), contents);
        }
        for contents in ["version=-1\n", "version='current'\n", "version=4\n"] {
            fs::write(&path, contents).await.unwrap();
            assert!(RecipientFile::Workspace.inspect(&path).await.is_err());
            assert!(RecipientFile::Workspace.apply(&path).await.is_err());
            assert_eq!(fs::read_to_string(&path).await.unwrap(), contents);
        }
        for contents in [
            "version=2\n",
            "version=2\n[env.encryption]\n",
            "version=2\nenv={ encryption={} }\n",
        ] {
            fs::write(&path, contents).await.unwrap();
            assert_workspace_current(&path, contents).await;
        }
        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn runtime_schema_save_preserves_current_raw_env_without_expanding_defaults() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::write(config.config_path(), "# latest user edit\ncustom_field = 'keep'\n[env]\nTOKEN = { value = 'latest', description = 'keep' }\n").await.unwrap();
        save_runtime_schema_state(&config).await.unwrap();
        let text = fs::read_to_string(config.config_path()).await.unwrap();
        assert!(text.contains("# latest user edit"));
        let table: toml::Table = toml::from_str(&text).unwrap();
        assert_eq!(table["env"].as_table().unwrap().len(), 1);
        assert_eq!(table["env"]["TOKEN"]["value"].as_str(), Some("latest"));
        assert_eq!(table["env"]["TOKEN"]["description"].as_str(), Some("keep"));
        assert_eq!(table["custom_field"].as_str(), Some("keep"));
        fs::remove_dir_all(dir).await.unwrap();
    }

    #[test]
    fn pending_schema_warning_reports_old_schema() {
        let warning = pending_schema_warning(0).unwrap();
        assert!(warning.contains("0 -> 2"));
        assert!(pending_schema_warning(CURRENT_RUNTIME_SCHEMA_VERSION).is_none());
    }

    #[tokio::test]
    async fn migrate_removes_only_update_cache_and_records_schema() {
        let dir = make_temp_dir().await;
        let mut config = Config::new_for_test(&dir);
        config.schema_version = 0;
        fs::write(dir.join(UPDATE_CACHE_FILE), b"stale")
            .await
            .unwrap();

        let retained_dirs = ["rendered", "bin", "presets"];
        for name in retained_dirs {
            fs::create_dir_all(dir.join(name)).await.unwrap();
        }
        let manifest = dir.join("app-manifest.toml");
        fs::write(&manifest, b"entries = []").await.unwrap();

        handle_migrate(&config, false).await.unwrap();

        assert!(!dir.join(UPDATE_CACHE_FILE).exists());
        for name in retained_dirs {
            assert!(dir.join(name).is_dir(), "{name}");
        }
        assert_eq!(fs::read(manifest).await.unwrap(), b"entries = []");
        assert_schema_recorded(config.config_path()).await;

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
        fs::write(
            &workspace,
            format!(
                "version = 2\n[env.encryption]\n# deployment key\nrecipient = \"alice@example.com\"\nage_recipients = [\"{LEGACY_RECIPIENT}\"]\n"
            ),
        )
        .await
        .unwrap();

        RecipientFile::Workspace.apply(&workspace).await.unwrap();

        let content = fs::read_to_string(&workspace).await.unwrap();
        assert!(!content.contains("recipient ="));
        assert!(content.contains("# deployment key"));
        assert!(content.contains("gpg_recipients = [\"alice@example.com\"]"));
        assert!(content.contains(TAGGED_RECIPIENT));
        assert!(content.contains("version = 2"));
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn workspace_migration_handles_inline_recipient_tables() {
        let dir = make_temp_dir().await;
        let workspace = dir.join("shine.workspace.toml");
        for (env, has_gpg) in [
            (
                format!(
                    "[env]\nfiles=['env.toml']\nencryption={{ recipient='alice', age_recipients=['{LEGACY_RECIPIENT}'] }}"
                ),
                true,
            ),
            (
                format!(
                    "env={{ files=['env.toml'], encryption={{ age_recipients=['{LEGACY_RECIPIENT}'] }} }}"
                ),
                false,
            ),
        ] {
            fs::write(&workspace, format!("version=2\n{env} # keep\n"))
                .await
                .unwrap();
            let changes = RecipientFile::Workspace.inspect(&workspace).await.unwrap();
            assert_eq!(changes.age_count, 1);
            assert!(changes.changed);
            assert!(
                RecipientFile::Workspace
                    .apply(&workspace)
                    .await
                    .unwrap()
                    .changed
            );
            let content = fs::read_to_string(&workspace).await.unwrap();
            let parsed: toml::Table = toml::from_str(&content).unwrap();
            let encryption = parsed["env"]["encryption"].as_table().unwrap();
            assert!(!encryption.contains_key("recipient"));
            if has_gpg {
                assert_eq!(encryption["gpg_recipients"][0].as_str(), Some("alice"));
            }
            assert_eq!(
                encryption["age_recipients"][0].as_str(),
                Some(TAGGED_RECIPIENT)
            );
            assert_eq!(parsed["env"]["files"][0].as_str(), Some("env.toml"));
            assert!(content.contains("# keep"));
            assert_workspace_current(&workspace, &content).await;
        }
        let conflict = "version=2\n[env]\nencryption={recipient='old',gpg_recipients=['new']}\n";
        fs::write(&workspace, conflict).await.unwrap();
        let error = RecipientFile::Workspace
            .apply(&workspace)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("resolve the conflict"));
        assert_eq!(fs::read_to_string(&workspace).await.unwrap(), conflict);
        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn migration_converts_secure_enclave_recipients_in_place() {
        let dir = make_temp_dir().await;
        let config_path = dir.join("config.toml");
        fs::write(
            &config_path,
            format!("# team recipients\nage_recipients = [\n  \"{LEGACY_RECIPIENT}\", # Touch ID\n  \"age1phone1example\",\n]\n"),
        )
        .await
        .unwrap();

        RecipientFile::Config.apply(&config_path).await.unwrap();

        let content = fs::read_to_string(&config_path).await.unwrap();
        assert!(content.contains(TAGGED_RECIPIENT));
        assert!(content.contains("# Touch ID"));
        assert!(content.contains("age1phone1example"));
        assert!(!content.contains(LEGACY_RECIPIENT));
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn invalid_secure_enclave_recipient_leaves_file_unchanged() {
        let dir = make_temp_dir().await;
        let config_path = dir.join("config.toml");
        let original = "age_recipients = [\"age1se1qgg72x2qfk9wg3wh0qg9u0v7l5dkq4jx69fv80p6wdus3ftg6flwg5dz2dp\", \"age1se1invalid\"]\n";
        fs::write(&config_path, original).await.unwrap();

        assert!(RecipientFile::Config.apply(&config_path).await.is_err());
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
        let global_original =
            format!("schema_version = 2\nage_recipients = [\"{LEGACY_RECIPIENT}\"]\n");
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
        let original = format!("schema_version = 2\nage_recipients = [\"{LEGACY_RECIPIENT}\"]\n");
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

        assert_schema_recorded(config.config_path()).await;

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
        fs::write(
            &project_config,
            format!(
                "# project key\ngpg_key_id = \"project@example.com\"\nage_recipients = [\"{LEGACY_RECIPIENT}\"]\n"
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
        assert!(content.contains(TAGGED_RECIPIENT));

        fs::remove_dir_all(&dir).await.unwrap();
    }
}
