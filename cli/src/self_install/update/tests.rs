use super::*;
#[test]
fn update_check_failure_warning_is_non_fatal_wording() {
    let err = anyhow::anyhow!(
        "GitHub stable release request failed: HTTP 403 Forbidden: API rate limit exceeded"
    );
    let warning = format_update_check_failure_warning(&err);

    assert!(warning.contains("warning: skipped shine version check"));
    assert!(warning.contains("HTTP 403 Forbidden"));
    assert!(!warning.contains("Update check failed"));
}

#[tokio::test]
async fn targeted_update_uses_one_snapshot_for_migration_and_installed_status() {
    let root = crate::test_support::make_temp_dir("shine-update-compatibility-snapshot").await;
    let mut config = Config::new_for_test(&root);
    config.is_external_presets = true;
    let category = config.presets_dir().join("app/demo");
    std::fs::create_dir_all(&category).unwrap();
    let metadata = category.join("shine.toml");
    let legacy = "dest='~/demo'\n[[files]]\nsource='config.txt'\n";
    let current = format!("metadata_schema_version=2\n{legacy}");
    std::fs::write(&metadata, &current).unwrap();
    std::fs::write(category.join("config.txt"), "installed").unwrap();
    crate::apps::handle_install_approved(&config, Some("demo"), false, false, true)
        .await
        .unwrap();

    for migration_required in [true, false] {
        std::fs::write(
            &metadata,
            if migration_required { legacy } else { &current },
        )
        .unwrap();
        let captured = UpdatePresets::capture(&config).await.unwrap();
        std::fs::write(&metadata, "invalid = [").unwrap();
        let result = handle_captured_update(
            &config,
            captured,
            Some("app/demo"),
            false,
            false,
            false,
            false,
        )
        .await;
        if migration_required {
            let message = result.unwrap_err().to_string();
            assert!(
                message.contains("Preset compatibility requires attention (1 automatic change)"),
                "{message}"
            );
        } else {
            result.unwrap();
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn source_notes_and_app_paths_follow_the_captured_overlay() {
    use std::{fs, path::Path};

    let root = crate::test_support::make_temp_dir("update-overlay-origin").await;
    let mut config = Config::new_for_test(&root)
        .with_presets_overlay_git(Some("https://example.invalid/overlay.git".into()), None);
    config.is_external_presets = true;
    let relative = Path::new("app/demo/nested/config");
    let base = config.presets_dir().join(relative);
    fs::create_dir_all(base.parent().unwrap()).unwrap();
    fs::write(&base, b"base").unwrap();
    fs::write(
        config.presets_dir().join("app/demo/shine.toml"),
        "metadata_schema_version=2\ndest='~/demo'\n[[files]]\nsource='nested/config'\n",
    )
    .unwrap();
    let overlay = root.join("overlay");
    let shadow = overlay.join(relative);
    for (expected_source, bytes, has_overlay) in [
        (&base, b"base".as_slice(), false),
        (&shadow, b"overlay".as_slice(), true),
    ] {
        let capture = UpdatePresets::capture(&config).await.unwrap();
        if has_overlay {
            fs::rename(&overlay, root.join("retired-overlay")).unwrap();
        } else {
            fs::create_dir_all(shadow.parent().unwrap()).unwrap();
            fs::write(&shadow, b"overlay").unwrap();
        }
        let notes = crate::config::presets_source_note_lines(&config, capture.overlay.as_deref());
        assert_eq!(
            notes.contains(&crate::colors::presets_overlay_note(&overlay)),
            has_overlay
        );
        let frontend = capture.into_frontend(&config).await.unwrap();
        let inspection = frontend.inspect_apps(vec!["demo".into()]).await.unwrap();
        assert_eq!(inspection.files.len(), 1);
        assert_eq!(&inspection.files[0].source_path, expected_source);
        assert_eq!(inspection.files[0].desired_content.as_deref(), Some(bytes));
    }
    fs::remove_dir_all(root).unwrap();
}
