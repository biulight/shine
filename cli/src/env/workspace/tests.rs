use super::*;

#[cfg(unix)]
#[test]
fn malformed_decrypted_cache_diagnostics_do_not_expose_plaintext() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use std::os::unix::fs::PermissionsExt;
    let _guard = crate::test_support::env_lock();
    let root =
        std::env::temp_dir().join(format!("shine-cache-diagnostics-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let gpg = root.join("gpg");
    std::fs::write(
        &gpg,
        "#!/bin/sh\nfor arg in \"$@\"; do file=\"$arg\"; done\n/bin/cat \"$file\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&gpg, std::fs::Permissions::from_mode(0o700)).unwrap();
    // SAFETY: the shared guard holds process state until every backend has exited.
    unsafe { std::env::set_var("PATH", &root) };
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    executor.block_on(async {
        let config = Config::new_for_test(&root);
        let path = root.join("cache.toml");
        for plaintext in [
            "version = 1\n[values]\nTOKEN = \"CACHE_PLAINTEXT_SENTINEL\" unexpected\n",
            "version = 1\n[values]\nTOKEN = [\"CACHE_PLAINTEXT_SENTINEL\"]\n",
        ] {
            let cache = CacheFile {
                version: CACHE_FORMAT_VERSION,
                project_root: root.to_string_lossy().into_owned(),
                modes: BTreeMap::from([(
                    "test".into(),
                    CachedMode {
                        input_hash: "matching".into(),
                        keys: vec!["TOKEN".into()],
                        data: STANDARD.encode(plaintext),
                    },
                )]),
            };
            tokio::fs::write(&path, toml::to_string(&cache).unwrap())
                .await
                .unwrap();
            let error = read_valid_cache(&path, "test", "matching", &config)
                .await
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "invalid encrypted environment cache payload"
            );
            let diagnostic = format!("{error:#}\n{error:?}");
            assert!(
                !diagnostic.contains("CACHE_PLAINTEXT_SENTINEL"),
                "{diagnostic}"
            );
        }
    });
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn export_errors_wipe_owned_values_before_returning_without_publication() {
    let root = crate::test_support::make_temp_dir("shine-export-raii").await;
    let workspace = root.join(WORKSPACE_FILE);
    let source = root.join("values.toml");
    let output = root.join("output.env");
    tokio::fs::write(&workspace, "version = 2\n[env]\nfiles = ['values.toml']\n")
        .await
        .unwrap();
    let config = Config::new_for_test(&root);
    for render_error in [true, false] {
        let value = if render_error { "$unsafe" } else { "safe" };
        tokio::fs::write(
            &source,
            format!("[plain]\nRAII_FIRST = 'sensitive'\nRAII_LAST = '{value}'\n"),
        )
        .await
        .unwrap();
        if !render_error {
            tokio::fs::create_dir(&output).await.unwrap();
        }
        values::take_wipe_events();
        let error = handle_export(
            &config,
            EnvWorkspaceExportFormat::Dotenv,
            Some(&workspace),
            "test",
            &output,
            false,
            !render_error,
            false,
        )
        .await
        .unwrap_err();
        assert!(!format!("{error:#}").contains("sensitive"));
        assert!(
            values::take_wipe_events()
                .iter()
                .any(|keys| keys == &["RAII_FIRST", "RAII_LAST"])
        );
        if render_error {
            assert!(!output.exists());
        } else {
            assert!(output.is_dir());
        }
    }
    tokio::fs::remove_dir_all(root).await.unwrap();
}

#[tokio::test]
async fn command_spawn_and_broker_merge_errors_wipe_owned_values() {
    let root = crate::test_support::make_temp_dir("shine-spawn-raii").await;
    let owned = SensitiveValues::from(BTreeMap::from([("RAII_ENV".into(), "secret".into())]));
    let explicit =
        SensitiveValues::from(BTreeMap::from([("RAII_EXPLICIT".into(), "secret".into())]));
    values::take_wipe_events();
    let error = run_command(
        &[root.join("absent-command").into_os_string()],
        owned,
        true,
        explicit,
    )
    .await
    .unwrap_err();
    assert!(!format!("{error:#}").contains("secret"));
    let wiped = values::take_wipe_events();
    assert!(wiped.contains(&vec!["RAII_ENV".into()]));
    assert!(wiped.contains(&vec!["RAII_EXPLICIT".into()]));

    let explicit = SensitiveValues::from(BTreeMap::from([("COLLISION".into(), "secret".into())]));
    let response = BTreeMap::from([
        ("COLLISION".into(), "conflicting-secret".into()),
        ("UNCONSUMED".into(), "remaining-secret".into()),
    ]);
    assert!(merge_explicit(explicit, response).is_err());
    let wiped = values::take_wipe_events();
    assert!(wiped.contains(&vec!["COLLISION".into()]));
    assert!(wiped.contains(&vec!["UNCONSUMED".into()]));
    tokio::fs::remove_dir_all(root).await.unwrap();
}

#[tokio::test]
async fn cancelling_compiled_values_cleans_the_owned_environment() {
    let root = crate::test_support::make_temp_dir("shine-cancel-raii").await;
    let source = root.join("values.toml");
    tokio::fs::write(&source, "[plain]\nRAII_CANCEL = 'sensitive'\n")
        .await
        .unwrap();
    let config = Config::new_for_test(&root);
    let (ready, wait) = tokio::sync::oneshot::channel();
    let mut pending = Box::pin(async {
        let values = compile_export_sources(&[source], &config, false)
            .await
            .unwrap();
        ready.send(()).unwrap();
        std::future::pending::<()>().await;
        drop(values);
    });
    tokio::select! { _ = &mut pending => panic!("operation unexpectedly completed"), _ = wait => {} }
    values::take_wipe_events();
    drop(pending);
    assert_eq!(
        values::take_wipe_events(),
        [vec!["RAII_CANCEL".to_string()]]
    );
    tokio::fs::remove_dir_all(root).await.unwrap();
}

#[tokio::test]
async fn a_shadowed_pending_secret_still_blocks_local_execution_but_not_plain_export() {
    let root = crate::test_support::make_temp_dir("shine-source-order").await;
    let config = Config::new_for_test(&root);
    let captured = vec![
        (
            root.join("base.toml"),
            Some(Zeroizing::new("[secret]\nTOKEN = 'pending'\n".into())),
        ),
        (root.join("local.toml"), None),
        (
            root.join("last.toml"),
            Some(Zeroizing::new("[plain]\nTOKEN = 'public'\n".into())),
        ),
    ];
    let error = compile_captured_sources(&captured, &config)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("is not sealed"));
    let plain = compile_source_layers(&captured, &config, false)
        .await
        .unwrap();
    assert_eq!(plain["TOKEN"], "public");
    let source = parse_source(Path::new("base.toml"), "[secret]\nTOKEN = 'do-not-log-me'").unwrap();
    assert!(!format!("{source:?}").contains("do-not-log-me"));
    tokio::fs::remove_dir_all(root).await.unwrap();
}

#[test]
fn dotenv_import_parses_common_frontend_entries() {
    let values = parse_dotenv(
        Path::new(".env"),
        "# base\nexport VITE_NAME = \"Shine\" # display name\nVITE_OWNER='Shine team' # owner\nVITE_URL=https://example.test # note\nEMPTY=\n",
    )
    .unwrap();

    assert_eq!(values.get("VITE_NAME").map(String::as_str), Some("Shine"));
    assert_eq!(
        values.get("VITE_OWNER").map(String::as_str),
        Some("Shine team")
    );
    assert_eq!(
        values.get("VITE_URL").map(String::as_str),
        Some("https://example.test")
    );
    assert_eq!(values.get("EMPTY").map(String::as_str), Some(""));
}

#[test]
fn dotenv_import_rejects_interpolation() {
    for value in [
        "${BASE_URL}/api",
        "$BASE_URL/api",
        "'$BASE_URL/api'",
        "\"$BASE_URL/api\"",
    ] {
        let error = parse_dotenv(Path::new(".env"), &format!("VITE_URL={value}\n")).unwrap_err();
        assert!(error.to_string().contains("dotenv interpolation"));
    }
}

#[test]
fn dotenv_import_comments_start_at_unquoted_hashes() {
    let values = parse_dotenv(
        Path::new(".env"),
        "UNQUOTED=abc#comment\nQUOTED='abc#value'\n",
    )
    .unwrap();
    assert_eq!(values["UNQUOTED"], "abc");
    assert_eq!(values["QUOTED"], "abc#value");
}

#[test]
fn dotenv_export_is_stable_and_preserves_literal_multiline_values() {
    let rendered = render_dotenv(&BTreeMap::from([
        ("ALPHA".to_owned(), "plain".to_owned()),
        (
            "COMPLEX".to_owned(),
            "quote\" slash\\ first\nsecond".to_owned(),
        ),
    ]))
    .unwrap();

    assert_eq!(
        rendered.as_str(),
        "ALPHA=\"plain\"\nCOMPLEX='quote\" slash\\ first\nsecond'\n"
    );
}

#[test]
fn dotenv_export_rejects_values_without_a_portable_literal_representation() {
    for value in [
        "$BASE/api",
        "cost $5",
        "before\rafter",
        "all '\"` delimiters",
    ] {
        let error = render_dotenv(&BTreeMap::from([("TOKEN".into(), value.into())])).unwrap_err();
        assert!(error.to_string().contains("TOKEN"));
        assert!(
            !error.to_string().contains(value),
            "must not disclose the value"
        );
    }
}

#[test]
fn dotenv_export_rejects_nul_values() {
    let error = render_dotenv(&BTreeMap::from([(
        "BROKEN".to_owned(),
        "before\0after".to_owned(),
    )]))
    .unwrap_err();
    assert!(error.to_string().contains("NUL byte"));
}

#[test]
fn dotenv_mode_discovery_ignores_generated_sources() {
    let directory = std::env::temp_dir().join(format!("shine-dotenv-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join(".env.development"), "VITE_A=1\n").unwrap();
    std::fs::write(directory.join(".env.production.local"), "VITE_A=2\n").unwrap();
    std::fs::write(directory.join(".env.development.shine.toml"), "version=1\n").unwrap();

    assert_eq!(
        dotenv_modes(&directory, &[]).unwrap(),
        vec!["development", "production"]
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn rendered_source_marks_only_requested_keys_secret() {
    let source = render_source(
        Path::new(".env"),
        &BTreeMap::from([
            ("PUBLIC".to_owned(), "yes".to_owned()),
            ("TOKEN".to_owned(), "secret".to_owned()),
        ]),
        &BTreeSet::from(["TOKEN".to_owned()]),
    );
    let parsed: SourceFile = toml::from_str(&source).unwrap();
    assert!(source.contains("Imported from .env"));
    assert_eq!(parsed.plain.get("PUBLIC").map(String::as_str), Some("yes"));
    assert!(
        matches!(parsed.secret.get("TOKEN"), Some(SecretState::Plain(value)) if value == "secret")
    );
}

#[test]
fn rendered_source_includes_an_empty_secret_template() {
    let source = render_source(
        Path::new(".env"),
        &BTreeMap::from([("PUBLIC".to_owned(), "yes".to_owned())]),
        &BTreeSet::new(),
    );
    assert!(source.contains("Optional: move sensitive values"));
    let parsed: SourceFile = toml::from_str(&source).unwrap();
    assert!(parsed.secret.is_empty());
}

#[tokio::test]
async fn dotenv_init_creates_vite_ordered_workspace_without_touching_sources() {
    let directory =
        std::env::temp_dir().join(format!("shine-dotenv-init-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&directory).await.unwrap();
    tokio::fs::write(
        directory.join(".env"),
        "VITE_API=https://api.example.test\nTOKEN=unsealed\n",
    )
    .await
    .unwrap();
    tokio::fs::write(
        directory.join(".env.development"),
        "VITE_API=http://localhost:3000\n",
    )
    .await
    .unwrap();

    init_from_dotenv_at(&directory, &[], &["TOKEN".to_owned()], false, false)
        .await
        .unwrap();

    let workspace = tokio::fs::read_to_string(directory.join(WORKSPACE_FILE))
        .await
        .unwrap();
    assert!(
        workspace.find(".env.local.shine.toml").unwrap()
            < workspace.find(".env.{mode}.shine.toml").unwrap()
    );
    assert!(workspace.contains("Managed by `shine env workspace init --from-dotenv`"));
    assert!(workspace.contains("Add GPG recipients"));
    let base = tokio::fs::read_to_string(directory.join(".env.shine.toml"))
        .await
        .unwrap();
    assert!(base.contains("[secret]"));
    assert!(base.contains("TOKEN = \"unsealed\""));
    assert_eq!(
        tokio::fs::read_to_string(directory.join(".env"))
            .await
            .unwrap(),
        "VITE_API=https://api.example.test\nTOKEN=unsealed\n"
    );
    assert!(
        init_from_dotenv_at(&directory, &[], &[], false, false)
            .await
            .is_err()
    );
    tokio::fs::remove_dir_all(directory).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn dotenv_init_preserves_dangling_destinations_without_force() {
    for name in [".env.shine.toml", WORKSPACE_FILE] {
        let dir = crate::test_support::make_temp_dir("shine-dotenv-linked").await;
        tokio::fs::write(dir.join(".env"), "TOKEN=pending-secret\n")
            .await
            .unwrap();
        let output = dir.join(name);
        std::os::unix::fs::symlink("absent-original", &output).unwrap();
        for dry_run in [false, true] {
            let error = init_from_dotenv_at(&dir, &[], &[], false, dry_run)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("already exists"));
            assert_eq!(
                std::fs::read_link(&output).unwrap(),
                Path::new("absent-original")
            );
            assert!(!dir.join("absent-original").exists());
            if name == WORKSPACE_FILE {
                assert!(!dir.join(".env.shine.toml").exists());
            } else {
                assert!(!dir.join(WORKSPACE_FILE).exists());
            }
        }
        init_from_dotenv_at(&dir, &[], &[], true, false)
            .await
            .unwrap();
        assert!(std::fs::symlink_metadata(&output).unwrap().is_file());
        assert!(!dir.join("absent-original").exists());
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }
}

#[tokio::test]
async fn dotenv_init_concurrent_publication_has_one_winner() {
    let dir = crate::test_support::make_temp_dir("shine-dotenv-concurrent").await;
    tokio::fs::write(dir.join(".env"), "TOKEN=pending-secret\n")
        .await
        .unwrap();
    let (first, second) = tokio::join!(
        init_from_dotenv_at(&dir, &[], &[], false, false),
        init_from_dotenv_at(&dir, &[], &[], false, false),
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    let error = first.err().or_else(|| second.err()).unwrap();
    assert!(error.to_string().contains("already exists"));
    let source = tokio::fs::read_to_string(dir.join(".env.shine.toml"))
        .await
        .unwrap();
    assert!(source.contains("pending-secret"));
    let workspace = tokio::fs::read_to_string(dir.join(WORKSPACE_FILE))
        .await
        .unwrap();
    parse_workspace(&dir.join(WORKSPACE_FILE), &workspace).unwrap();
    for entry in std::fs::read_dir(&dir).unwrap() {
        assert!(
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".shine-write-")
        );
    }
    tokio::fs::remove_dir_all(dir).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn dotenv_import_keeps_pending_plaintext_private_on_create_and_force() {
    use std::os::unix::fs::PermissionsExt;
    let dir = crate::test_support::make_temp_dir("shine-dotenv-private").await;
    let input = dir.join(".env");
    let output = dir.join(".env.shine.toml");
    tokio::fs::write(&input, "TOKEN=pending-secret\n")
        .await
        .unwrap();
    tokio::fs::set_permissions(&input, std::fs::Permissions::from_mode(0o600))
        .await
        .unwrap();
    for force in [false, true] {
        if force {
            tokio::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o644))
                .await
                .unwrap();
        }
        init_from_dotenv_at(&dir, &[], &["TOKEN".into()], force, false)
            .await
            .unwrap();
        assert_eq!(
            tokio::fs::metadata(&output)
                .await
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(
            tokio::fs::read_to_string(&output)
                .await
                .unwrap()
                .contains("pending-secret")
        );
        assert_eq!(
            tokio::fs::read_to_string(&input).await.unwrap(),
            "TOKEN=pending-secret\n"
        );
        assert_eq!(
            tokio::fs::metadata(&input)
                .await
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    tokio::fs::remove_dir_all(dir).await.unwrap();
}

#[test]
fn resolves_vite_style_layers_in_declared_order() {
    let workspace = Path::new("/tmp/project/shine.workspace.toml");
    let files = vec![
        ".env.shine.toml".into(),
        ".env.local.shine.toml".into(),
        ".env.{mode}.shine.toml".into(),
        ".env.{mode}.local.shine.toml".into(),
    ];
    assert_eq!(
        resolve_sources(workspace, &files, "production").unwrap(),
        vec![
            PathBuf::from("/tmp/project/.env.shine.toml"),
            PathBuf::from("/tmp/project/.env.local.shine.toml"),
            PathBuf::from("/tmp/project/.env.production.shine.toml"),
            PathBuf::from("/tmp/project/.env.production.local.shine.toml"),
        ]
    );
}

#[test]
fn source_rejects_duplicate_plain_and_secret_keys() {
    let error = parse_source(
        Path::new(".env.shine.toml"),
        "version = 1\n[plain]\nTOKEN = \"plain\"\n[secret]\nTOKEN = true\n",
    )
    .unwrap_err();
    assert!(error.to_string().contains("both [plain] and [secret]"));
}

#[test]
fn seal_encryption_gpg_recipients_priority_is_cli_workspace_config() {
    let dir = std::env::temp_dir().join(format!("shine-seal-enc-{}", uuid::Uuid::new_v4()));
    let mut config = Config::new_for_test(&dir);
    config.gpg_recipients = vec!["global-one".to_string(), "global-two".to_string()];
    let workspace_encryption = Encryption {
        legacy_recipient: None,
        gpg_recipients: vec!["workspace-one".to_string(), "workspace-two".to_string()],
        backend: None,
        age_recipients: Vec::new(),
    };

    let cli = resolve_seal_encryption(
        None,
        &["cli".to_string()],
        Some(&workspace_encryption),
        &config,
    )
    .unwrap();
    assert!(matches!(cli, Some(EncryptRecipients::Gpg(values)) if values == ["cli"]));

    let workspace =
        resolve_seal_encryption(None, &[], Some(&workspace_encryption), &config).unwrap();
    assert!(
        matches!(workspace, Some(EncryptRecipients::Gpg(values)) if values == ["workspace-one", "workspace-two"])
    );

    let global = resolve_seal_encryption(None, &[], None, &config).unwrap();
    assert!(
        matches!(global, Some(EncryptRecipients::Gpg(values)) if values == ["global-one", "global-two"])
    );
}

#[test]
fn seal_encryption_returns_none_when_nothing_configured() {
    let dir = std::env::temp_dir().join(format!("shine-seal-enc-{}", uuid::Uuid::new_v4()));
    let config = Config::new_for_test(&dir);

    assert!(
        resolve_seal_encryption(None, &[], None, &config)
            .unwrap()
            .is_none()
    );
}

#[test]
fn seal_encryption_age_recipients_prefer_workspace_over_config() {
    let dir = std::env::temp_dir().join(format!("shine-seal-enc-{}", uuid::Uuid::new_v4()));
    let mut config = Config::new_for_test(&dir);
    config.secret_backend = Some("age".to_string());
    config.age_recipients = vec!["age1config".to_string()];
    let workspace_encryption = Encryption {
        legacy_recipient: None,
        gpg_recipients: Vec::new(),
        backend: None,
        age_recipients: vec!["age1workspace".to_string()],
    };

    let resolved =
        resolve_seal_encryption(None, &[], Some(&workspace_encryption), &config).unwrap();
    assert!(
        matches!(resolved, Some(EncryptRecipients::Age(values)) if values == ["age1workspace"])
    );

    let fallback = resolve_seal_encryption(
        None,
        &[],
        Some(&Encryption {
            legacy_recipient: None,
            gpg_recipients: Vec::new(),
            backend: None,
            age_recipients: Vec::new(),
        }),
        &config,
    )
    .unwrap();
    assert!(matches!(fallback, Some(EncryptRecipients::Age(values)) if values == ["age1config"]));
}

#[test]
fn seal_encryption_backend_priority_is_cli_workspace_config() {
    let dir = std::env::temp_dir().join(format!("shine-seal-enc-{}", uuid::Uuid::new_v4()));
    let mut config = Config::new_for_test(&dir);
    config.secret_backend = Some("age".to_string());
    config.gpg_recipients = vec!["global".to_string()];
    let workspace_encryption = Encryption {
        legacy_recipient: None,
        gpg_recipients: vec!["workspace".to_string()],
        backend: Some("gpg".to_string()),
        age_recipients: Vec::new(),
    };

    let resolved =
        resolve_seal_encryption(None, &[], Some(&workspace_encryption), &config).unwrap();
    assert!(matches!(resolved, Some(EncryptRecipients::Gpg(_))));

    let resolved_age = resolve_seal_encryption(None, &[], None, &config).unwrap();
    assert!(
        resolved_age.is_none(),
        "age backend with no age_recipients should be lazily None: {resolved_age:?}"
    );
}

#[tokio::test]
async fn plain_sources_merge_in_declared_order() {
    let directory = std::env::temp_dir().join(format!("shine-workspace-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&directory).await.unwrap();
    let base = directory.join("base.toml");
    let local = directory.join("local.toml");
    tokio::fs::write(&base, "version = 1\n[plain]\nA = \"base\"\nB = \"base\"\n")
        .await
        .unwrap();
    tokio::fs::write(&local, "version = 1\n[plain]\nB = \"local\"\n")
        .await
        .unwrap();

    let config = Config::new_for_test(&directory);
    let values = compile_sources(&[base, local], &config).await.unwrap();
    assert_eq!(values.get("A").map(String::as_str), Some("base"));
    assert_eq!(values.get("B").map(String::as_str), Some("local"));
    tokio::fs::remove_dir_all(directory).await.unwrap();
}

#[cfg(unix)]
#[test]
fn broker_preserves_source_precedence_and_releases_only_winning_secrets() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use std::os::unix::fs::PermissionsExt;
    let _guard = crate::test_support::env_lock();
    struct RestorePath(Option<OsString>);
    impl Drop for RestorePath {
        fn drop(&mut self) {
            // SAFETY: env_lock is retained throughout this test.
            unsafe {
                match &self.0 {
                    Some(value) => std::env::set_var("PATH", value),
                    None => std::env::remove_var("PATH"),
                }
            }
        }
    }
    fn source(path: &str, plain: &[(&str, &str)], secrets: &[(&str, &str)]) -> SourceSnapshot {
        let mut document = DocumentMut::new();
        document["version"] = value(1);
        for (key, text) in plain {
            document["plain"][key] = value(*text);
        }
        for (key, _) in secrets {
            document["secret"][key] = value(true);
        }
        let payload = SecretPayload {
            version: SECRET_PAYLOAD_VERSION,
            values: secrets
                .iter()
                .map(|(key, value)| ((*key).into(), (*value).into()))
                .collect(),
        };
        if !secrets.is_empty() {
            document["payload"]["data"] =
                value(STANDARD.encode(toml::to_string(&payload).unwrap()));
        }
        SourceSnapshot {
            path: path.into(),
            contents: document.to_string(),
        }
    }
    let root =
        std::env::temp_dir().join(format!("shine-broker-precedence-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("gpg"),
        "#!/bin/sh\nfor arg in \"$@\"; do file=\"$arg\"; done\n/bin/cat \"$file\"\n",
    )
    .unwrap();
    std::fs::set_permissions(root.join("gpg"), std::fs::Permissions::from_mode(0o700)).unwrap();
    let _restore = RestorePath(std::env::var_os("PATH"));
    // SAFETY: all environment-mutating tests hold env_lock.
    unsafe { std::env::set_var("PATH", &root) };
    let snapshot = WorkspaceSnapshot {
        workspace_path: root.join(WORKSPACE_FILE).to_string_lossy().into_owned(),
        workspace_contents:
            "version = 2\n[env]\nfiles = ['base.toml', 'local.toml', 'final.toml']\n".into(),
        mode: "development".into(),
        override_process_env: true,
        sources: vec![
            source(
                "base.toml",
                &[("UNRELEASED", "old-plain")],
                &[
                    ("PLAIN", "old-secret"),
                    ("TOKEN", "old-token"),
                    ("AGAIN", "old-secret"),
                    ("UNSELECTED", "private"),
                ],
            ),
            source(
                "local.toml",
                &[("PLAIN", "later-plain"), ("AGAIN", "middle-plain")],
                &[("TOKEN", "new-token"), ("UNRELEASED", "private")],
            ),
            source("final.toml", &[], &[("AGAIN", "final-secret")]),
        ],
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let config = Config::new_for_test(&root);
        let release = vec!["PLAIN".into(), "TOKEN".into(), "AGAIN".into()];
        let secrets = decrypt_broker_snapshot(&config, &snapshot, &release)
            .await
            .unwrap();
        assert_eq!(
            secrets,
            BTreeMap::from([
                ("TOKEN".into(), "new-token".into()),
                ("AGAIN".into(), "final-secret".into())
            ])
        );
        let merged = merge_broker_values(&snapshot, secrets).unwrap();
        assert_eq!(
            merged,
            BTreeMap::from([
                ("PLAIN".into(), "later-plain".into()),
                ("TOKEN".into(), "new-token".into()),
                ("AGAIN".into(), "final-secret".into())
            ])
        );
        // The receiving side also preserves order when an older broker
        // returns a secret that is now shadowed by plain metadata.
        let old_response = BTreeMap::from([("PLAIN".into(), "old-secret".into())]);
        assert_eq!(
            merge_broker_values(&snapshot, old_response).unwrap(),
            BTreeMap::from([("PLAIN".into(), "later-plain".into())])
        );
        assert!(
            decrypt_broker_snapshot(&config, &snapshot, &["UNKNOWN".into()])
                .await
                .is_err()
        );
        let captured = snapshot
            .sources
            .iter()
            .map(|source| {
                (
                    PathBuf::from(&source.path),
                    Some(Zeroizing::new(source.contents.clone())),
                )
            })
            .collect();
        let local = compile_captured_sources(&captured, &config).await.unwrap();
        for (key, value) in &merged {
            assert_eq!(local.get(key), Some(value));
        }
        assert!(!merged.contains_key("UNRELEASED"));
        assert!(!merged.contains_key("UNSELECTED"));
    });
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn workspace_export_plain_is_standalone_and_respects_secret_shadowing() {
    let directory =
        std::env::temp_dir().join(format!("shine-workspace-export-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&directory).await.unwrap();
    let workspace_path = directory.join(WORKSPACE_FILE);
    tokio::fs::write(
        &workspace_path,
        "version = 2\n[env]\nmodes = [\"production\"]\nfiles = [\"base.toml\", \"production.toml\"]\n",
    )
    .await
    .unwrap();
    tokio::fs::write(
        directory.join("base.toml"),
        "version = 1\n[plain]\nPUBLIC = \"base\"\nSHADOWED = \"old\"\n[secret]\nTOKEN = \"pending\"\n",
    )
    .await
    .unwrap();
    tokio::fs::write(
        directory.join("production.toml"),
        "version = 1\n[plain]\nPUBLIC = \"production\"\n[secret]\nSHADOWED = true\n",
    )
    .await
    .unwrap();
    let output = directory.join(".env.production.local");
    let config = Config::new_for_test(&directory);

    handle_export(
        &config,
        EnvWorkspaceExportFormat::Dotenv,
        Some(&workspace_path),
        "production",
        &output,
        false,
        false,
        true,
    )
    .await
    .unwrap();
    assert!(!output.exists());

    handle_export(
        &config,
        EnvWorkspaceExportFormat::Dotenv,
        Some(&workspace_path),
        "production",
        &output,
        false,
        false,
        false,
    )
    .await
    .unwrap();

    assert_eq!(
        tokio::fs::read_to_string(&output).await.unwrap(),
        "PUBLIC=\"production\"\n"
    );
    assert!(
        handle_export(
            &config,
            EnvWorkspaceExportFormat::Dotenv,
            Some(&workspace_path),
            "production",
            &output,
            false,
            false,
            false,
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("--force")
    );
    tokio::fs::remove_dir_all(directory).await.unwrap();
}

#[tokio::test]
async fn workspace_export_requires_explicit_secret_inclusion() {
    let directory =
        std::env::temp_dir().join(format!("shine-workspace-export-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&directory).await.unwrap();
    let source = directory.join("source.toml");
    tokio::fs::write(
        &source,
        "version = 1\n[plain]\nPUBLIC = \"safe\"\n[secret]\nTOKEN = \"pending\"\n",
    )
    .await
    .unwrap();
    let config = Config::new_for_test(&directory);

    let plain = compile_export_sources(std::slice::from_ref(&source), &config, false)
        .await
        .unwrap();
    assert_eq!(plain, BTreeMap::from([("PUBLIC".into(), "safe".into())]));

    let error = compile_export_sources(&[source], &config, true)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("is not sealed"));
    tokio::fs::remove_dir_all(directory).await.unwrap();
}

#[tokio::test]
async fn plain_only_source_can_be_sealed_without_recipient() {
    let directory = std::env::temp_dir().join(format!("shine-seal-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&directory).await.unwrap();
    let path = directory.join("env.toml");
    tokio::fs::write(&path, "version = 1\n[plain]\nNAME = \"shine\"\n")
        .await
        .unwrap();

    let config = Config::new_for_test(&directory);
    seal_file(&path, &config, None).await.unwrap();
    let source = tokio::fs::read_to_string(&path).await.unwrap();
    assert!(source.contains("[payload]"));
    tokio::fs::remove_dir_all(directory).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn run_command_injects_workspace_values() {
    let values = BTreeMap::from([("SHINE_RUN_TEST".to_string(), "injected".to_string())]);
    run_command(
        &[
            OsString::from("sh"),
            OsString::from("-c"),
            OsString::from("test \"$SHINE_RUN_TEST\" = injected"),
        ],
        values.into(),
        true,
        SensitiveValues::default(),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn explicit_values_support_aliases_and_multiple_keys() {
    let directory = std::env::temp_dir().join(format!("shine-with-{}", uuid::Uuid::new_v4()));
    let mut config = Config::new_for_test(&directory);
    config.env.insert("TOKEN_A".into(), "alpha".into());
    config.env.insert("TOKEN_B".into(), "beta".into());

    let values =
        resolve_explicit_values(&config, &["TOKEN_A".into(), "TOKEN_B=OTHER_TOKEN".into()])
            .await
            .unwrap();

    assert_eq!(values.get("TOKEN_A").map(String::as_str), Some("alpha"));
    assert_eq!(values.get("OTHER_TOKEN").map(String::as_str), Some("beta"));
}

#[tokio::test]
async fn explicit_values_reject_duplicate_targets_before_resolution() {
    let directory = std::env::temp_dir().join(format!("shine-with-{}", uuid::Uuid::new_v4()));
    let config = Config::new_for_test(&directory);

    let error = resolve_explicit_values(&config, &["TOKEN_A=TOKEN".into(), "TOKEN_B=TOKEN".into()])
        .await
        .unwrap_err();

    assert!(error.to_string().contains("duplicate target variable"));
}

#[cfg(unix)]
#[tokio::test]
async fn no_workspace_injects_explicit_without_discovery() {
    let directory = std::env::temp_dir().join(format!("shine-nows-{}", uuid::Uuid::new_v4()));
    let mut config = Config::new_for_test(&directory);
    config.env.insert("SHINE_NOWS_TOKEN".into(), "alpha".into());

    // no_workspace = true must skip discovery entirely and inject only --with.
    handle_run(
        &config,
        None,
        None,
        true,
        &["SHINE_NOWS_TOKEN".into()],
        false,
        &[],
        &[
            OsString::from("sh"),
            OsString::from("-c"),
            OsString::from("test \"$SHINE_NOWS_TOKEN\" = alpha"),
        ],
    )
    .await
    .unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn no_workspace_allows_empty_with() {
    let directory = std::env::temp_dir().join(format!("shine-nows-empty-{}", uuid::Uuid::new_v4()));
    let config = Config::new_for_test(&directory);

    handle_run(
        &config,
        None,
        None,
        true,
        &[],
        false,
        &[],
        &[
            OsString::from("sh"),
            OsString::from("-c"),
            OsString::from("true"),
        ],
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn explicit_values_reject_invalid_or_missing_keys() {
    let directory = std::env::temp_dir().join(format!("shine-with-{}", uuid::Uuid::new_v4()));
    let config = Config::new_for_test(&directory);

    let invalid = resolve_explicit_values(&config, &["BAD-KEY".into()])
        .await
        .unwrap_err();
    assert!(
        invalid
            .to_string()
            .contains("invalid environment variable name")
    );

    let missing = resolve_explicit_values(&config, &["MISSING".into()])
        .await
        .unwrap_err();
    assert!(
        missing
            .to_string()
            .contains("MISSING_SECRET or MISSING is not set")
    );
}

#[cfg(unix)]
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn explicit_values_override_workspace_and_process_values() {
    let _guard = crate::test_support::env_lock();
    // SAFETY: the shared test env lock serializes process environment mutation.
    unsafe { std::env::set_var("SHINE_RUN_OVERRIDE_TEST", "process") };
    let workspace = BTreeMap::from([(
        "SHINE_RUN_OVERRIDE_TEST".to_string(),
        "workspace".to_string(),
    )]);
    let explicit = BTreeMap::from([(
        "SHINE_RUN_OVERRIDE_TEST".to_string(),
        "explicit".to_string(),
    )]);

    run_command(
        &[
            OsString::from("sh"),
            OsString::from("-c"),
            OsString::from("test \"$SHINE_RUN_OVERRIDE_TEST\" = explicit"),
        ],
        workspace.into(),
        false,
        explicit.into(),
    )
    .await
    .unwrap();

    assert_eq!(
        std::env::var("SHINE_RUN_OVERRIDE_TEST").as_deref(),
        Ok("process")
    );
    // SAFETY: the shared test env lock serializes process environment mutation.
    unsafe { std::env::remove_var("SHINE_RUN_OVERRIDE_TEST") };
}
