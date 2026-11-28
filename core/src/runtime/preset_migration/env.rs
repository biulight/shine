//! Legacy input conversion is confined to source migration, never execution.
use crate::permission::PermissionDeclarationV1;
use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, BTreeSet};
use toml_edit::{Array, DocumentMut, InlineTable, Item, TableLike, Value};

type Sensitivities = BTreeMap<String, &'static str>;

fn sensitivities(value: Option<&toml::Value>) -> Result<Sensitivities> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let declaration: PermissionDeclarationV1 =
        value.clone().try_into().context("invalid permissions")?;
    declaration.validate()?;
    Ok(declaration
        .environment
        .into_iter()
        .map(|e| {
            (
                e.name,
                match e.sensitivity {
                    crate::plan::EnvironmentSensitivityV1::Plain => "plain",
                    crate::plan::EnvironmentSensitivityV1::Secret => "secret",
                },
            )
        })
        .collect())
}

fn convert(
    table: &mut dyn TableLike,
    sensitivity: &Sensitivities,
    used: &mut BTreeSet<String>,
) -> Result<bool> {
    let mut changed = false;
    if let Some(item) = table.get_mut("env") {
        let array = item.as_array_mut().context("env must be an array")?;
        if array.iter().any(Value::is_str) {
            if !array.iter().all(Value::is_str) {
                bail!("mixed legacy and structured env requires manual migration")
            }
            let tokens = array
                .iter()
                .map(|v| v.as_str().unwrap().to_owned())
                .collect::<Vec<_>>();
            let specs = crate::env::parse_env_specs(&tokens)?;
            let mut replacement = Array::new();
            for spec in specs {
                let classification = sensitivity
                    .get(&spec.source)
                    .context("env source has no explicit sensitivity; manual migration required")?;
                let mut entry = InlineTable::new();
                entry.insert("source", Value::from(spec.source.clone()));
                if spec.source != spec.target {
                    entry.insert("target", Value::from(spec.target));
                }
                entry.insert("sensitivity", Value::from(*classification));
                replacement.push(Value::InlineTable(entry));
                used.insert(spec.source);
            }
            *array = replacement;
            changed = true;
        }
    }
    Ok(changed)
}

// TOML allows a table entry, an array of tables, or an inline-table array.
// Callers select the actual metadata owner; this never recursively visits keys.
fn visit_tables(
    item: &mut Item,
    mut visit: impl FnMut(&mut dyn TableLike) -> Result<bool>,
) -> Result<bool> {
    if let Some(table) = item.as_table_like_mut() {
        return visit(table);
    }
    let mut changed = false;
    if let Some(tables) = item.as_array_of_tables_mut() {
        for table in tables.iter_mut() {
            changed |= visit(table)?;
        }
    } else if let Some(tables) = item.as_array_mut() {
        for table in tables.iter_mut() {
            changed |= visit(
                table
                    .as_inline_table_mut()
                    .context("expected a metadata table")?,
            )?;
        }
    } else {
        bail!("expected a metadata table or array of tables");
    }
    Ok(changed)
}

fn remove_converted_permissions(table: &mut dyn TableLike, used: &BTreeSet<String>) {
    if used.is_empty() {
        return;
    }
    let Some(permissions) = table
        .get_mut("permissions")
        .and_then(Item::as_table_like_mut)
    else {
        return;
    };
    let Some(array) = permissions
        .get_mut("environment")
        .and_then(Item::as_array_mut)
    else {
        return;
    };
    for i in (0..array.len()).rev() {
        if array
            .get(i)
            .and_then(Value::as_inline_table)
            .and_then(|e| e.get("name"))
            .and_then(Value::as_str)
            .is_some_and(|name| used.contains(name))
        {
            array.remove(i);
        }
    }
    if array.is_empty() {
        permissions.remove("environment");
    }
}

pub(super) fn migrate(kind: &str, document: &mut DocumentMut) -> Result<bool> {
    let value: toml::Value = toml::from_str(&document.to_string())
        .map_err(|_| anyhow::anyhow!("invalid Preset metadata"))?;
    let mut changed = false;
    if kind == "app" {
        let sensitivity = sensitivities(value.get("permissions"))?;
        let mut used = BTreeSet::new();
        for key in ["artifact", "post_install", "post_upgrade"] {
            if let Some(entry) = document.get_mut(key) {
                changed |= visit_tables(entry, |table| convert(table, &sensitivity, &mut used))?;
            }
        }
        if let Some(files) = document.get_mut("files") {
            changed |= visit_tables(files, |file| {
                let Some(generator) = file.get_mut("generator") else {
                    return Ok(false);
                };
                let generator = generator
                    .as_table_like_mut()
                    .context("expected a generator table")?;
                let mut changed = convert(generator, &sensitivity, &mut used)?;
                if let Some(fallback) = generator.get_mut("fallback") {
                    let fallback = fallback
                        .as_table_like_mut()
                        .context("expected a generator fallback table")?;
                    changed |= convert(fallback, &sensitivity, &mut used)?;
                }
                Ok(changed)
            })?;
        }
        // A filtered generator may disappear on another platform, whereas its
        // original category declaration also covers static/template files.
        // Retain shared statements rather than reduce that platform's contract.
        let conditional_generator = value
            .get("files")
            .and_then(toml::Value::as_array)
            .is_some_and(|files| {
                files.iter().any(|file| {
                    file.get("generator").is_some()
                        && (file.get("platforms").is_some() || file.get("dest").is_some())
                })
            });
        if !conditional_generator {
            remove_converted_permissions(document.as_table_mut(), &used);
        }
    } else if kind == "shell" {
        let entries = value.get("files").and_then(toml::Value::as_array);
        let mut index = 0;
        if let Some(files) = document.get_mut("files") {
            changed |= visit_tables(files, |file| {
                let entry = entries
                    .and_then(|entries| entries.get(index))
                    .context("invalid Shell files")?;
                index += 1;
                let sensitivity = sensitivities(
                    entry
                        .get("permissions")
                        .or_else(|| value.get("permission_defaults")),
                )?;
                let mut used = BTreeSet::new();
                let changed = convert(file, &sensitivity, &mut used)?;
                remove_converted_permissions(file, &used);
                Ok(changed)
            })?;
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn migrate(kind: &str, original: &str) -> Result<Option<String>> {
        let mut document = original.parse::<DocumentMut>()?;
        Ok(super::migrate(kind, &mut document)?.then(|| document.to_string()))
    }

    fn migrate_once(kind: &str, original: &str) -> String {
        let migrated = migrate(kind, original).unwrap().unwrap();
        assert!(
            migrate(kind, &migrated).unwrap().is_none(),
            "second migration changed {kind} metadata"
        );
        migrated
    }

    #[test]
    fn app_migration_only_converts_env_owned_by_executable_entries() {
        for (root_extra, file_extra) in [("env=['TOKEN']\n", ""), ("", "env=['TOKEN']\n")] {
            let original = format!(
                "metadata_schema_version=2\ndest='~/.demo'\n{root_extra}[permissions]\nschema_version=1\nenvironment=[{{name='TOKEN',sensitivity='secret'}}]\n[[files]]\nsource='config'\n{file_extra}"
            );
            assert!(
                migrate("app", &original).unwrap().is_none(),
                "ignored metadata was rewritten: {original}"
            );
        }
        let original = r#"dest='~/.demo'
env=['TOKEN']
[artifact]
script='build.ts'
env=['RUN']
[permissions]
schema_version=1
environment=[{name='TOKEN',sensitivity='secret'},{name='RUN',sensitivity='plain'}]
[[files]]
source='config'
env=['TOKEN']
"#;
        let value: toml::Value = toml::from_str(&migrate_once("app", original)).unwrap();
        assert_eq!(value["env"][0].as_str(), Some("TOKEN"));
        assert_eq!(value["files"][0]["env"][0].as_str(), Some("TOKEN"));
        assert_eq!(
            value["permissions"]["environment"][0]["name"].as_str(),
            Some("TOKEN")
        );
        assert_eq!(value["artifact"]["env"][0]["source"].as_str(), Some("RUN"));
    }

    #[test]
    fn app_migration_preserves_comments_conditions_aliases_and_remaining_permissions() {
        let original = r#"# Keep this comment.
dest = '~/.config/demo'
post_install = [{ script = 'hook.ts', runtime = 'bun', env = ['TOKEN'] }]
[artifact]
script = 'build.ts'
runtime = 'bun'
env = ['PROFILE']
[permissions]
schema_version = 1
environment = [{ name = 'PROFILE', sensitivity = 'plain' }, { name = 'TOKEN', sensitivity = 'secret' }, { name = 'TEMPLATE', sensitivity = 'plain' }]
[[files]]
source = 'config'
generator = { script = 'gen.ts', runtime = 'bun', env = ['TOKEN=ALIAS'], when_env = 'TOKEN', auto = false }
"#;
        let text = migrate_once("app", original);
        assert!(text.starts_with("# Keep this comment."));
        let value: toml::Value = toml::from_str(&text).unwrap();
        let generator = &value["files"][0]["generator"];
        assert_eq!(generator["env"][0]["source"].as_str(), Some("TOKEN"));
        assert_eq!(generator["env"][0]["target"].as_str(), Some("ALIAS"));
        assert_eq!(generator["env"][0]["sensitivity"].as_str(), Some("secret"));
        assert_eq!(generator["when_env"].as_str(), Some("TOKEN"));
        assert_eq!(generator["auto"].as_bool(), Some(false));
        assert_eq!(
            value["artifact"]["env"][0]["source"].as_str(),
            Some("PROFILE")
        );
        let remaining = value["permissions"]["environment"].as_array().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0]["name"].as_str(), Some("TEMPLATE"));
    }

    #[test]
    fn shell_migration_respects_whole_entry_override_and_never_guesses_sensitivity() {
        let original = r#"[permission_defaults]
schema_version = 1
environment = [{ name = 'TOKEN', sensitivity = 'secret' }]
[[files]]
source = 'one.ts'
runtime = 'bun'
env = ['TOKEN=ALIAS']
[[files]]
source = 'two.ts'
runtime = 'bun'
env = ['TOKEN']
[files.permissions]
schema_version = 1
environment = [{ name = 'TOKEN', sensitivity = 'plain' }]
"#;
        let value: toml::Value = toml::from_str(&migrate_once("shell", original)).unwrap();
        assert_eq!(
            value["files"][0]["env"][0]["sensitivity"].as_str(),
            Some("secret")
        );
        assert_eq!(
            value["files"][1]["env"][0]["sensitivity"].as_str(),
            Some("plain")
        );
        let text = original.replace(
            "environment = [{ name = 'TOKEN', sensitivity = 'plain' }]",
            "",
        );
        assert!(migrate("shell", &text).is_err());
        assert!(migrate("app", "[artifact]\nscript = 'build.ts'\nenv = ['TOKEN']").is_err());
    }

    #[test]
    fn app_migration_keeps_shared_contract_when_generator_can_be_filtered() {
        let original = r#"dest='~/.demo'
[permissions]
schema_version=1
environment=[{name='TOKEN',sensitivity='secret'}]
[[files]]
source='common'
transforms=['template']
[[files]]
source='generated'
platforms=['windows']
generator={script='gen.ts',runtime='bun',env=['TOKEN'],when_env='TOKEN'}
"#;
        let value: toml::Value = toml::from_str(&migrate_once("app", original)).unwrap();
        assert_eq!(
            value["permissions"]["environment"][0]["name"].as_str(),
            Some("TOKEN")
        );
        assert_eq!(
            value["files"][1]["generator"]["env"][0]["source"].as_str(),
            Some("TOKEN")
        );
    }
}
