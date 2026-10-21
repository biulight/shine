use super::*;
use shine_core::runtime::PresetSourceKind;

#[tokio::test]
async fn migration_rechecks_shadowed_sources_after_complete_backup_before_any_edit() {
    let root = crate::test_support::make_temp_dir("shine-migration-apply-boundary").await;
    let base = root.as_path().join("base");
    let overlay = root.as_path().join("overlay");
    let logical = "app/demo/shine.toml";
    tokio::fs::create_dir_all(base.join("app/demo"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(overlay.join("app/demo"))
        .await
        .unwrap();
    tokio::fs::write(base.join(logical), b"shadowed")
        .await
        .unwrap();
    tokio::fs::write(overlay.join(logical), b"reviewed")
        .await
        .unwrap();
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .base_root(&base)
        .file(logical, b"shadowed".to_vec())
        .overlay_root(&overlay)
        .overlay_file(logical, b"reviewed".to_vec())
        .build();
    let edits = vec![PresetMigrationEdit {
        logical_path: logical.into(),
        physical_path: overlay.join(logical),
        source_layer: "overlay".into(),
        operations: Vec::new(),
        original: b"reviewed".to_vec(),
        candidate: Some(b"candidate".to_vec()),
    }];
    let sources = migration_source_observations(&snapshot, &edits);
    let backup = create_backup_set(&root.as_path().join("state"), &edits, &sources)
        .await
        .unwrap();
    assert!(backup.join("manifest.toml").is_file());
    assert_eq!(
        tokio::fs::read(backup.join(logical)).await.unwrap(),
        b"reviewed"
    );
    tokio::fs::write(base.join(logical), b"changed after backup")
        .await
        .unwrap();
    let error = apply_edits(&edits, &sources).await.unwrap_err();
    assert_eq!(error.to_string(), "Preset source changed after review");
    assert_eq!(
        tokio::fs::read(overlay.join(logical)).await.unwrap(),
        b"reviewed"
    );
    assert_eq!(
        tokio::fs::read(base.join(logical)).await.unwrap(),
        b"changed after backup"
    );
    assert_eq!(
        tokio::fs::read(backup.join(logical)).await.unwrap(),
        b"reviewed"
    );
    tokio::fs::remove_dir_all(root).await.unwrap();
}

#[tokio::test]
async fn migration_retains_per_file_recheck_and_stops_before_later_edits() {
    let root = crate::test_support::make_temp_dir("shine-migration-apply-boundary").await;
    let edits = ["first", "second"].map(|name| PresetMigrationEdit {
        logical_path: format!("app/demo/{name}"),
        physical_path: root.as_path().join(name),
        source_layer: "external".into(),
        operations: Vec::new(),
        original: b"reviewed".to_vec(),
        candidate: Some(b"candidate".to_vec()),
    });
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
    assert_eq!(
        tokio::fs::read(&edits[0].physical_path).await.unwrap(),
        b"replacement"
    );
    assert_eq!(
        tokio::fs::read(&edits[1].physical_path).await.unwrap(),
        b"reviewed"
    );
    tokio::fs::remove_dir_all(root).await.unwrap();
}
