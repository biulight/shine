use super::*;
use shine_core::runtime::PresetSourceKind;

fn reviewed_edit(logical: &str, physical_path: PathBuf, layer: &str) -> PresetMigrationEdit {
    PresetMigrationEdit {
        logical_path: logical.into(),
        physical_path,
        source_layer: layer.into(),
        operations: Vec::new(),
        original: b"reviewed".to_vec(),
        candidate: Some(b"candidate".to_vec()),
    }
}

async fn assert_file_bytes(path: &Path, expected: &[u8]) {
    let actual = tokio::fs::read(path).await.unwrap();
    assert_eq!(actual, expected, "{}", path.display());
}

#[tokio::test]
async fn migration_rechecks_shadowed_sources_after_complete_backup_before_any_edit() {
    let root = crate::test_support::make_temp_dir("shine-migration-apply-boundary").await;
    let base = root.join("base");
    let overlay = root.join("overlay");
    let logical = "app/demo/shine.toml";
    for (dir, contents) in [(&base, "shadowed"), (&overlay, "reviewed")] {
        tokio::fs::create_dir_all(dir.join("app/demo"))
            .await
            .unwrap();
        tokio::fs::write(dir.join(logical), contents).await.unwrap();
    }
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .base_root(&base)
        .file(logical, b"shadowed".to_vec())
        .overlay_root(&overlay)
        .overlay_file(logical, b"reviewed".to_vec())
        .build();
    let edits = [reviewed_edit(logical, overlay.join(logical), "overlay")];
    let sources = migration_source_observations(&snapshot, &edits);
    let backup = create_backup_set(&root.join("state"), &edits, &sources)
        .await
        .unwrap();
    assert!(backup.join("manifest.toml").is_file());
    assert_file_bytes(&backup.join(logical), b"reviewed").await;
    tokio::fs::write(base.join(logical), b"changed after backup")
        .await
        .unwrap();
    let error = apply_edits(&edits, &sources).await.unwrap_err();
    assert_eq!(error.to_string(), "Preset source changed after review");
    assert_file_bytes(&overlay.join(logical), b"reviewed").await;
    assert_file_bytes(&base.join(logical), b"changed after backup").await;
    assert_file_bytes(&backup.join(logical), b"reviewed").await;
    tokio::fs::remove_dir_all(root).await.unwrap();
}

#[tokio::test]
async fn migration_retains_per_file_recheck_and_stops_before_later_edits() {
    let root = crate::test_support::make_temp_dir("shine-migration-apply-boundary").await;
    let edits = ["first", "second"]
        .map(|name| reviewed_edit(&format!("app/demo/{name}"), root.join(name), "external"));
    for edit in &edits {
        tokio::fs::write(&edit.physical_path, &edit.original)
            .await
            .unwrap();
    }
    // The final per-file check remains a separate guard even after the captured-source check.
    tokio::fs::write(&edits[0].physical_path, b"replacement")
        .await
        .unwrap();
    let error = apply_edits(&edits, &BTreeMap::new()).await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "Preset source changed after review: app/demo/first"
    );
    assert_file_bytes(&edits[0].physical_path, b"replacement").await;
    assert_file_bytes(&edits[1].physical_path, b"reviewed").await;
    tokio::fs::remove_dir_all(root).await.unwrap();
}
