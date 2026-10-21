use super::*;
#[test]
fn hybrid_policy_validation_and_overrides() {
    let config = Config::new_for_test(Path::new("unused"));
    let mut policy = Encryption {
        backend: Some("hybrid".into()),
        gpg_recipients: vec!["A".repeat(40)],
        age_recipients: vec!["age1test".into()],
        ..Default::default()
    };
    assert!(matches!(
        resolve_seal_encryption(None, &[], Some(&policy), &config).unwrap(),
        Some(EncryptRecipients::Hybrid(_))
    ));
    assert!(resolve_seal_encryption(None, &["".into()], Some(&policy), &config).is_err());
    assert!(matches!(
        resolve_seal_encryption(Some("gpg"), &[], Some(&policy), &config).unwrap(),
        Some(EncryptRecipients::Gpg(_))
    ));
    policy.age_recipients.clear();
    assert!(resolve_seal_encryption(None, &[], Some(&policy), &config).is_err());
    assert!(resolve_seal_encryption(Some("hybrid"), &[], None, &config).is_err());
}
#[tokio::test]
async fn captured_sources_drive_cache_and_compilation() {
    let dir = crate::test_support::make_temp_dir("shine-hybrid-snapshot").await;
    let path = dir.join("source.toml");
    tokio::fs::write(&path, "[plain]\nVALUE = 'captured'\n")
        .await
        .unwrap();
    let captured = capture_sources(std::slice::from_ref(&path)).await.unwrap();
    tokio::fs::write(&path, "[payload]\ndata = 'hybrid:malformed'\n")
        .await
        .unwrap();
    let config = Config::new_for_test(&dir);
    assert!(!involves_hybrid(&Encryption::default(), &captured).unwrap());
    assert_eq!(
        compile_captured_sources(&captured, &config).await.unwrap()["VALUE"],
        "captured"
    );
    let hybrid = capture_sources(std::slice::from_ref(&path)).await.unwrap();
    assert!(involves_hybrid(&Encryption::default(), &hybrid).unwrap());
    assert!(compile_captured_sources(&hybrid, &config).await.is_err());
    let policy = Encryption {
        backend: Some("hybrid".into()),
        ..Default::default()
    };
    assert!(involves_hybrid(&policy, &captured).unwrap());
    tokio::fs::remove_dir_all(dir).await.unwrap();
}
#[tokio::test]
async fn seal_distinguishes_sources_and_workspace_with_matching_stems() {
    let dir = crate::test_support::make_temp_dir("shine-seal-lock-names").await;
    let workspace = dir.join("shine.workspace.toml");
    let names = ["env.dev", "env.prod", "shine.workspace.env"];
    tokio::fs::write(
        &workspace,
        "version = 2\n[env]\nfiles = ['env.dev', 'env.prod', 'shine.workspace.env']\n",
    )
    .await
    .unwrap();
    let original = "[plain]\nVALUE = 'public'\n";
    for name in names {
        tokio::fs::write(dir.join(name), original).await.unwrap();
    }
    let config = Config::new_for_test(&dir);
    let lock = SealLock::acquire(&dir.join("env.prod")).unwrap();
    assert!(
        handle_seal(&config, Some(&workspace), None, None, &[])
            .await
            .is_err()
    );
    for name in names {
        assert_eq!(
            tokio::fs::read_to_string(dir.join(name)).await.unwrap(),
            original
        );
    }
    drop(lock);

    handle_seal(&config, Some(&workspace), None, None, &[])
        .await
        .unwrap();
    for name in names {
        let contents = tokio::fs::read_to_string(dir.join(name)).await.unwrap();
        assert!(contents.contains("[payload]"));
        assert_eq!(
            parse_source(&dir.join(name), &contents).unwrap().plain["VALUE"],
            "public"
        );
    }
    tokio::fs::remove_dir_all(dir).await.unwrap();
}

#[tokio::test]
async fn replacement_preserves_source_and_policy_edits_and_missing_files() {
    let dir = crate::test_support::make_temp_dir("shine-hybrid-race").await;
    let source = dir.join("source.toml");
    let workspace = dir.join("shine.workspace.toml");
    tokio::fs::write(&source, "original").await.unwrap();
    tokio::fs::write(&workspace, "policy").await.unwrap();
    let lock = SealLock::acquire(&workspace).unwrap();
    assert!(SealLock::acquire(&workspace).is_err());
    tokio::fs::write(&workspace, "edited policy").await.unwrap();
    assert!(
        replace_sealed_file(&source, b"sealed", "original", Some((&workspace, "policy")))
            .await
            .is_err()
    );
    assert_eq!(
        tokio::fs::read_to_string(&source).await.unwrap(),
        "original"
    );
    tokio::fs::write(&source, "edited source").await.unwrap();
    assert!(
        replace_sealed_file(&source, b"sealed", "original", None)
            .await
            .is_err()
    );
    assert_eq!(
        tokio::fs::read_to_string(&source).await.unwrap(),
        "edited source"
    );
    tokio::fs::remove_file(&source).await.unwrap();
    assert!(
        replace_sealed_file(&source, b"sealed", "original", None)
            .await
            .is_err()
    );
    assert!(!source.exists());
    drop(lock);
    assert!(SealLock::acquire(&workspace).is_ok());
    tokio::fs::remove_dir_all(dir).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn seal_rejects_linked_sources_before_updating_any_file() {
    let dir = crate::test_support::make_temp_dir("shine-seal-linked-source").await;
    let workspace = dir.join("shine.workspace.toml");
    let original = "version = 1\n[secret]\nTOKEN = 'pending-value'\n";
    let public = "version = 1\n[plain]\nVALUE = 'public'\n";
    tokio::fs::write(dir.join("target.toml"), original)
        .await
        .unwrap();
    tokio::fs::write(dir.join("public.toml"), public)
        .await
        .unwrap();
    tokio::fs::write(
        &workspace,
        "version = 2\n[env]\nfiles = ['public.toml', 'linked.toml']\n",
    )
    .await
    .unwrap();
    let linked = dir.join("linked.toml");
    let config = Config::new_for_test(&dir);
    for hard in [false, true] {
        if hard {
            std::fs::hard_link(dir.join("target.toml"), &linked).unwrap();
        } else {
            std::os::unix::fs::symlink(dir.join("target.toml"), &linked).unwrap();
        }
        let err = handle_seal(&config, Some(&workspace), None, None, &[])
            .await
            .unwrap_err();
        assert!(err.to_string().contains(if hard {
            "one hard link"
        } else {
            "regular environment source"
        }));
        assert_eq!(
            tokio::fs::read_to_string(dir.join("public.toml"))
                .await
                .unwrap(),
            public
        );
        assert_eq!(
            tokio::fs::read_to_string(dir.join("target.toml"))
                .await
                .unwrap(),
            original
        );
        // The replacement boundary must also reject a link introduced after capture.
        assert!(
            replace_sealed_file(&linked, b"sealed", original, None)
                .await
                .is_err()
        );
        assert_eq!(tokio::fs::read_to_string(&linked).await.unwrap(), original);
        tokio::fs::remove_file(&linked).await.unwrap();
    }
    tokio::fs::remove_dir_all(dir).await.unwrap();
}
