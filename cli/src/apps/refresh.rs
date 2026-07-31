//! Explicit refresh of manifest-owned generated app files.

use anyhow::{Result, bail};
use std::collections::BTreeSet;
use std::path::Path;

use crate::colors;
use crate::config::Config;
use crate::env::EnvConfig;
use crate::install_core::file_ops::InstallOutcome;
use crate::install_core::manifest::{AppEntry, AppManifest};

use super::hooks::{HookPhase, run_app_hooks};
use super::metadata;
use super::report::{print_install_error, print_install_success};
use super::{
    desired_content_hash, install_prepared_content, installed_content_hash,
    materialize_file_content, resolve_install_destination,
};

pub async fn handle_refresh(
    config: &Config,
    category: &str,
    file_selector: Option<&str>,
    force: bool,
) -> Result<()> {
    crate::config::print_presets_note(config);
    let categories = metadata::load_active_categories(config, Some(category)).await?;
    let cat = categories
        .iter()
        .find(|cat| cat.name == category)
        .ok_or_else(|| anyhow::anyhow!("app preset category not found: {category}"))?;
    let env = EnvConfig::load_or_init(config).await?;
    let env_map = env.as_map();
    let mut manifest = AppManifest::load(config.shine_dir()).await?;

    let candidates = if let Some(selector) = file_selector {
        let file = cat
            .files
            .iter()
            .find(|file| file.source_rel == Path::new(selector))
            .ok_or_else(|| anyhow::anyhow!("app '{category}' file not found: {selector}"))?;
        if file.generator.is_none() {
            bail!("app '{category}' file is not generated: {selector}");
        }
        vec![file]
    } else {
        cat.files
            .iter()
            .filter(|file| file.generator.is_some())
            .collect::<Vec<_>>()
    };

    if candidates.is_empty() {
        bail!("app '{category}' has no generated files");
    }

    let mut selected = Vec::new();
    for file in candidates {
        let destination = resolve_install_destination(cat, file, config)?;
        let Some(entry) = manifest.find_by_dest(&destination).cloned() else {
            if file_selector.is_some() {
                bail!(
                    "app '{category}' generated file is not installed: {}",
                    file.source_rel.display()
                );
            }
            continue;
        };
        selected.push((file, destination, entry));
    }
    if selected.is_empty() {
        bail!(
            "app '{category}' has no installed generated files; run `shine app install {category}` first"
        );
    }

    println!(
        "{}",
        colors::bold(&format!("Refreshing app generators: {category}"))
    );
    let mut updated = 0usize;
    let mut unchanged = 0usize;
    let mut failed = 0usize;

    for (file, destination, entry) in selected {
        let label = format!("{category}/{}", file.source_rel.display());
        let generator = file.generator.as_ref().expect("candidate has generator");
        if !env_map.contains_key(&generator.when_env) {
            eprintln!(
                "  {} {label}: generator requires config env '{}'",
                colors::symbol_stderr("✗"),
                generator.when_env
            );
            failed += 1;
            continue;
        }

        let content = match materialize_file_content(config, cat, file, env_map).await {
            Ok(content) => content,
            Err(error) => {
                print_install_error(&label, &error);
                failed += 1;
                continue;
            }
        };
        let desired_hash = match desired_content_hash(file, &content) {
            Ok(hash) => hash,
            Err(error) => {
                print_install_error(&label, &error);
                failed += 1;
                continue;
            }
        };

        let (destination_exists, current_hash) = match tokio::fs::read(&destination).await {
            Ok(bytes) => match installed_content_hash(file, &bytes) {
                Ok(hash) => (true, hash),
                Err(error) => {
                    if !force {
                        print_install_error(&label, &error);
                        failed += 1;
                        continue;
                    }
                    (true, None)
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (false, None),
            Err(error) => {
                print_install_error(&label, &error.into());
                failed += 1;
                continue;
            }
        };

        if current_hash == Some(entry.content_hash) && desired_hash == entry.content_hash {
            println!(
                "  {} {label}  {}",
                colors::dim("-"),
                colors::dim("already up to date")
            );
            unchanged += 1;
            continue;
        }
        if destination_exists && current_hash != Some(entry.content_hash) && !force {
            eprintln!(
                "  {} {label}: user-modified, kept (use --force to overwrite)",
                colors::symbol("!")
            );
            failed += 1;
            continue;
        }

        match install_prepared_content(file, &content, &destination, true, false, true).await {
            Ok(InstallOutcome::Installed { hash })
            | Ok(InstallOutcome::BackedUpAndInstalled { hash, .. }) => {
                print_install_success(&label, "", &destination, config);
                manifest.upsert(AppEntry {
                    source: entry.source,
                    destination,
                    backup: entry.backup,
                    content_hash: hash,
                    install_strategy: file.install_strategy.clone(),
                    uses_env: true,
                    requires_admin: file.requires_admin,
                });
                updated += 1;
            }
            Ok(InstallOutcome::AlreadyManaged) => {
                unchanged += 1;
            }
            Ok(InstallOutcome::DryRun) => unreachable!("refresh is never a dry run"),
            Err(error) => {
                print_install_error(&label, &error);
                failed += 1;
            }
        }
    }

    if updated > 0 {
        manifest.save(config.shine_dir()).await?;
        run_app_hooks(
            config,
            |name| categories.iter().find(|cat| cat.name == name),
            &BTreeSet::from([category.to_string()]),
            HookPhase::PostUpgrade,
        )
        .await;
    }

    println!(
        "{}",
        colors::dim(&format!(
            "Refresh complete: {updated} updated, {unchanged} unchanged, {failed} failed"
        ))
    );
    if failed > 0 {
        bail!("{failed} generated app file(s) failed to refresh");
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::apps::{handle_install, handle_upgrade_installed};
    use crate::status::{FileStatus, app_entry_status};
    use std::os::unix::fs::PermissionsExt;
    use tokio::fs;

    async fn write_fixture(root: &Path, two_files: bool) -> Config {
        let mut config = Config::new_for_test(root);
        config.is_external_presets = true;
        config.allow_app_hooks = true;
        config
            .env
            .insert("SOURCE_URL".to_string(), "https://example.test".to_string());
        let app_dir = config.presets_dir().join("app/sample");
        fs::create_dir_all(&app_dir).await.unwrap();
        let second = if two_files {
            r#"

[[files]]
source = "second.txt"
generator = { script = "second.sh", env = ["SOURCE_URL"], when_env = "SOURCE_URL", auto = false }
"#
        } else {
            ""
        };
        fs::write(
            app_dir.join("shine.toml"),
            format!(
                r#"description = "sample"
dest = "{}"

[[files]]
source = "first.txt"
generator = {{ script = "first.sh", env = ["SOURCE_URL"], when_env = "SOURCE_URL", auto = false }}
{second}"#,
                root.join("dest").display()
            ),
        )
        .await
        .unwrap();
        fs::write(app_dir.join("first.txt"), b"fallback-first\n")
            .await
            .unwrap();
        fs::write(app_dir.join("first.payload"), b"first-v1\n")
            .await
            .unwrap();
        write_generator(&app_dir.join("first.sh"), "first").await;
        if two_files {
            fs::write(app_dir.join("second.txt"), b"fallback-second\n")
                .await
                .unwrap();
            fs::write(app_dir.join("second.payload"), b"second-v1\n")
                .await
                .unwrap();
            write_generator(&app_dir.join("second.sh"), "second").await;
        }
        config
    }

    async fn write_generator(path: &Path, stem: &str) {
        fs::write(
            path,
            format!(
                "#!/bin/sh\nprintf x >> '{counter}'\ncat '{payload}'\n",
                counter = path
                    .parent()
                    .unwrap()
                    .join(format!("{stem}.runs"))
                    .display(),
                payload = path
                    .parent()
                    .unwrap()
                    .join(format!("{stem}.payload"))
                    .display()
            ),
        )
        .await
        .unwrap();
        fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn manual_generator_skips_status_and_upgrade_but_refreshes_explicitly() {
        let root = crate::test_support::make_temp_dir("shine-refresh").await;
        let config = write_fixture(&root, false).await;
        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();

        let app_dir = config.presets_dir().join("app/sample");
        let dest = root.join("dest/first.txt");
        assert_eq!(
            fs::read_to_string(app_dir.join("first.runs"))
                .await
                .unwrap(),
            "x"
        );
        fs::write(app_dir.join("first.payload"), b"first-v2\n")
            .await
            .unwrap();

        let categories = metadata::load_active_categories(&config, Some("sample"))
            .await
            .unwrap();
        let cat = &categories[0];
        let file = &cat.files[0];
        let manifest = AppManifest::load(config.shine_dir()).await.unwrap();
        let entry = manifest.find_by_dest(&dest).unwrap();
        assert_eq!(
            app_entry_status(&config, cat, file, entry, &config.env).await,
            FileStatus::UpToDate
        );
        let mut separator = crate::output::SectionSeparator::new();
        let report = handle_upgrade_installed(&config, false, &mut separator)
            .await
            .unwrap();
        assert_eq!(report.updated, 0);
        assert_eq!(
            fs::read_to_string(app_dir.join("first.runs"))
                .await
                .unwrap(),
            "x"
        );
        assert_eq!(fs::read(&dest).await.unwrap(), b"first-v1\n");

        handle_refresh(&config, "sample", Some("first.txt"), false)
            .await
            .unwrap();
        assert_eq!(fs::read(&dest).await.unwrap(), b"first-v2\n");
        assert_eq!(
            fs::read_to_string(app_dir.join("first.runs"))
                .await
                .unwrap(),
            "xx"
        );
        fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn refresh_selector_and_force_preserve_other_generated_files() {
        let root = crate::test_support::make_temp_dir("shine-refresh").await;
        let config = write_fixture(&root, true).await;
        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        let app_dir = config.presets_dir().join("app/sample");
        let first_dest = root.join("dest/first.txt");
        let second_dest = root.join("dest/second.txt");
        fs::write(app_dir.join("first.payload"), b"first-v2\n")
            .await
            .unwrap();
        fs::write(app_dir.join("second.payload"), b"second-v2\n")
            .await
            .unwrap();
        fs::write(&first_dest, b"user edit\n").await.unwrap();

        assert!(
            handle_refresh(&config, "sample", Some("first.txt"), false)
                .await
                .is_err()
        );
        assert_eq!(fs::read(&first_dest).await.unwrap(), b"user edit\n");
        handle_refresh(&config, "sample", Some("first.txt"), true)
            .await
            .unwrap();
        assert_eq!(fs::read(&first_dest).await.unwrap(), b"first-v2\n");
        assert_eq!(fs::read(&second_dest).await.unwrap(), b"second-v1\n");
        assert_eq!(
            fs::read_to_string(app_dir.join("second.runs"))
                .await
                .unwrap(),
            "x",
            "single-file refresh must not run other generators"
        );
        fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn refresh_keeps_last_good_file_and_continues_after_generator_failure() {
        let root = crate::test_support::make_temp_dir("shine-refresh").await;
        let config = write_fixture(&root, true).await;
        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        let app_dir = config.presets_dir().join("app/sample");
        let first_dest = root.join("dest/first.txt");
        let second_dest = root.join("dest/second.txt");
        fs::write(app_dir.join("first.sh"), b"#!/bin/sh\nexit 1\n")
            .await
            .unwrap();
        fs::write(app_dir.join("second.payload"), b"second-v2\n")
            .await
            .unwrap();

        assert!(
            handle_refresh(&config, "sample", None, false)
                .await
                .is_err()
        );
        assert_eq!(
            fs::read(&first_dest).await.unwrap(),
            b"first-v1\n",
            "failed generator must retain the last-known-good file"
        );
        assert_eq!(
            fs::read(&second_dest).await.unwrap(),
            b"second-v2\n",
            "a failed generator must not prevent later selected files refreshing"
        );
        fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn refresh_requires_the_generator_condition_env() {
        let root = crate::test_support::make_temp_dir("shine-refresh").await;
        let mut config = write_fixture(&root, false).await;
        handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();
        config.env.remove("SOURCE_URL");
        let dest = root.join("dest/first.txt");

        assert!(
            handle_refresh(&config, "sample", Some("first.txt"), false)
                .await
                .is_err()
        );
        assert_eq!(fs::read(&dest).await.unwrap(), b"first-v1\n");
        fs::remove_dir_all(root).await.unwrap();
    }
}
