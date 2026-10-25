use super::*;
use shine_core::runtime::{FileSystemHost, RealHost};

#[tokio::test]
async fn reviewed_policy_updates_recheck_revocation_and_preserve_unrelated_changes() {
    let dir = crate::test_support::make_temp_dir("shine-broker-contention").await;
    let config = Config::new_for_test(&dir);
    let request = snapshot();
    let initial = plan_remote_enrollment(
        &PolicyStore::default(),
        "dev",
        &request,
        &["API_TOKEN".into()],
        &["bun".into(), "test".into()],
        None,
    )
    .unwrap();
    apply_remote_enrollment(&config, &initial).await.unwrap();
    let mut candidate = initial.candidate.clone();
    candidate.project = "updated".into();
    let plan = RemoteEnrollmentPlan {
        name: initial.name.clone(),
        candidate,
        previous: Some(initial.candidate.clone()),
    };

    let guard = RealHost
        .acquire_operation_lock(&dir.join("ssh-secret-broker.lock"))
        .await
        .unwrap();
    let apply = apply_remote_enrollment(&config, &plan);
    tokio::pin!(apply);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut apply)
            .await
            .is_err()
    );
    // Model a revocation that completed while the reviewed update was waiting.
    save_store(&config, &PolicyStore::default()).await.unwrap();
    drop(guard);
    assert!(
        apply
            .await
            .unwrap_err()
            .to_string()
            .contains("no longer exists")
    );
    assert!(load_store(&config).await.unwrap().policies.is_empty());

    apply_remote_enrollment(&config, &initial).await.unwrap();
    let mut unrelated = initial.candidate.clone();
    unrelated.name = "unrelated".into();
    unrelated.ssh_target = "other".into();
    let guard = RealHost
        .acquire_operation_lock(&dir.join("ssh-secret-broker.lock"))
        .await
        .unwrap();
    let apply = apply_remote_enrollment(&config, &plan);
    tokio::pin!(apply);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut apply)
            .await
            .is_err()
    );
    save_store(
        &config,
        &PolicyStore {
            version: POLICY_VERSION,
            policies: vec![initial.candidate.clone(), unrelated.clone()],
        },
    )
    .await
    .unwrap();
    drop(guard);
    apply.await.unwrap();
    let store = load_store(&config).await.unwrap();
    assert_eq!(store.policies, [plan.candidate.clone(), unrelated.clone()]);

    let guard = RealHost
        .acquire_operation_lock(&dir.join("ssh-secret-broker.lock"))
        .await
        .unwrap();
    let remove = handle_policy_remove(&config, &plan.name);
    tokio::pin!(remove);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut remove)
            .await
            .is_err()
    );
    save_store(
        &config,
        &PolicyStore {
            version: POLICY_VERSION,
            policies: vec![unrelated.clone()],
        },
    )
    .await
    .unwrap();
    drop(guard);
    assert!(remove.await.is_err());
    assert_eq!(load_store(&config).await.unwrap().policies, [unrelated]);
    tokio::fs::remove_dir_all(dir).await.unwrap();
}

fn snapshot() -> WorkspaceSnapshot {
    WorkspaceSnapshot {
            workspace_path: std::env::temp_dir()
                .join("srv/api/shine.workspace.toml")
                .to_string_lossy()
                .into_owned(),
            workspace_contents: "version = 1\n[env]\nfiles = [\"env/development.toml\"]\n".into(),
            mode: "development".into(),
            override_process_env: false,
            sources: vec![SourceSnapshot {
                path: "env/development.toml".into(),
                contents: "version = 1\n[plain]\nPUBLIC = \"x\"\n[secret]\nAPI_TOKEN = true\nNPM_TOKEN = true\n[payload]\ndata = \"ciphertext\"\n".into(),
            }],
        }
}

#[test]
fn allow_separates_declared_keys_from_release_subset() {
    let snapshot = snapshot();
    let allow = allow_from_snapshot(
        &snapshot,
        &["API_TOKEN".into()],
        &["bun".into(), "test".into()],
    )
    .unwrap();

    assert_eq!(allow.release, ["API_TOKEN"]);
    assert_eq!(
        allow.sources[0].declared_secrets,
        ["API_TOKEN", "NPM_TOKEN"]
    );
}

#[test]
fn all_declared_release_expands_to_a_stable_explicit_list() {
    let snapshot = snapshot();
    let release = resolve_release(&snapshot, &[], true).unwrap();
    assert_eq!(release, ["API_TOKEN", "NPM_TOKEN"]);

    assert!(resolve_release(&snapshot, &["API_TOKEN".into()], true).is_err());
    assert!(resolve_release(&snapshot, &[], false).is_err());
}

#[test]
fn policy_diff_retains_serialized_cr_values_without_normalizing_approval_inputs() {
    let initial = plan_remote_enrollment(
        &PolicyStore::default(),
        "dev",
        &snapshot(),
        &["API_TOKEN".into()],
        &["bun".into(), "test".into()],
        None,
    )
    .unwrap();
    let mut previous = initial.candidate;
    previous.allow[0].argv = vec!["old\r\nargument\r".into()];
    let mut candidate = previous.clone();
    candidate.allow[0].argv = vec!["new\r\nargument\r".into()];
    let plan = RemoteEnrollmentPlan {
        name: previous.name.clone(),
        candidate: candidate.clone(),
        previous: Some(previous.clone()),
    };
    let old = toml::to_string_pretty(&previous).unwrap();
    let new = toml::to_string_pretty(&candidate).unwrap();
    let diff = plan.diff().unwrap().unwrap();
    // Policy serialization remains the comparison boundary; it escapes CR as TOML data.
    assert!(!old.contains('\r') && !new.contains('\r'));
    assert_eq!(
        diff,
        similar::TextDiff::from_lines(&old, &new)
            .unified_diff()
            .to_string()
    );
    assert!(
        diff.contains("-old\\r\n+new\\r\n argument\\r\"\"\""),
        "{diff:?}"
    );
    assert!(!diff.contains("Line endings:"));
    assert_eq!(plan.previous.as_ref(), Some(&previous));
    assert_eq!(plan.candidate, candidate);
}

#[test]
fn trusted_remote_update_replaces_one_allow_and_preserves_policy_identity() {
    let original = snapshot();
    let allow = allow_from_snapshot(
        &original,
        &["API_TOKEN".into()],
        &["bun".into(), "test".into()],
    )
    .unwrap();
    let store = PolicyStore {
        version: POLICY_VERSION,
        policies: vec![BrokerPolicy {
            name: "dev-api".into(),
            ssh_target: "dev".into(),
            project: "friendly-api-name".into(),
            workspace_sha256: sha256(original.workspace_contents.as_bytes()),
            remote_workspace: Some(original.workspace_path.clone()),
            allow: vec![allow],
        }],
    };
    let mut changed = original.clone();
    changed
        .workspace_contents
        .push_str("# new workspace revision\n");
    changed.sources[0].contents = changed.sources[0]
        .contents
        .replace("[payload]", "NEW_TOKEN = true\n[payload]");
    let release = resolve_release(&changed, &[], true).unwrap();

    let plan = plan_remote_enrollment(
        &store,
        "dev",
        &changed,
        &release,
        &["bun".into(), "test".into()],
        Some("dev-api"),
    )
    .unwrap();

    assert!(plan.previous.is_some());
    assert_eq!(plan.candidate.project, "friendly-api-name");
    assert_eq!(
        plan.candidate.remote_workspace.as_deref(),
        Some(original.workspace_path.as_str())
    );
    assert_eq!(
        plan.candidate.allow[0].release,
        ["API_TOKEN", "NEW_TOKEN", "NPM_TOKEN"]
    );
    assert!(plan.diff().unwrap().unwrap().contains("NEW_TOKEN"));
    assert!(
        plan_remote_enrollment(
            &store,
            "other-host",
            &changed,
            &release,
            &["bun".into(), "test".into()],
            Some("dev-api"),
        )
        .is_err()
    );
}

#[tokio::test]
async fn trusted_remote_update_applies_once_and_rejects_a_stale_preview() {
    let dir = crate::test_support::make_temp_dir("shine-broker-remote-update").await;
    let config = Config::new_for_test(&dir);
    let original = snapshot();
    let allow = allow_from_snapshot(
        &original,
        &["API_TOKEN".into()],
        &["bun".into(), "test".into()],
    )
    .unwrap();
    let store = PolicyStore {
        version: POLICY_VERSION,
        policies: vec![BrokerPolicy {
            name: "dev-api".into(),
            ssh_target: "dev".into(),
            project: "api".into(),
            workspace_sha256: sha256(original.workspace_contents.as_bytes()),
            remote_workspace: None,
            allow: vec![allow],
        }],
    };
    save_store(&config, &store).await.unwrap();

    let mut changed = original;
    changed.sources[0].contents = changed.sources[0]
        .contents
        .replace("[payload]", "NEW_TOKEN = true\n[payload]");
    let release = resolve_release(&changed, &[], true).unwrap();
    let plan = plan_remote_enrollment(
        &store,
        "dev",
        &changed,
        &release,
        &["bun".into(), "test".into()],
        Some("dev-api"),
    )
    .unwrap();

    apply_remote_enrollment(&config, &plan).await.unwrap();
    let loaded = load_store(&config).await.unwrap();
    assert_eq!(loaded.policies[0], plan.candidate);
    assert!(apply_remote_enrollment(&config, &plan).await.is_err());
    tokio::fs::remove_dir_all(dir).await.unwrap();
}

#[test]
fn exact_policy_match_rejects_changed_argv_or_source() {
    let snapshot = snapshot();
    let allow = allow_from_snapshot(
        &snapshot,
        &["API_TOKEN".into()],
        &["bun".into(), "test".into()],
    )
    .unwrap();
    let store = PolicyStore {
        version: POLICY_VERSION,
        policies: vec![BrokerPolicy {
            name: "dev-api".into(),
            ssh_target: "dev".into(),
            project: "api".into(),
            workspace_sha256: sha256(snapshot.workspace_contents.as_bytes()),
            remote_workspace: None,
            allow: vec![allow],
        }],
    };

    let matched =
        match_workspace_request(&store, "dev", &snapshot, &["bun".into(), "test".into()]).unwrap();
    assert_eq!(matched.release, ["API_TOKEN"]);

    assert!(
        match_workspace_request(
            &store,
            "dev",
            &snapshot,
            &["bun".into(), "run".into(), "test".into()],
        )
        .is_err()
    );
    let mut changed = snapshot.clone();
    changed.sources[0].contents.push_str("# changed\n");
    assert!(
        match_workspace_request(&store, "dev", &changed, &["bun".into(), "test".into()],).is_err()
    );
}

#[test]
fn wire_display_fields_reject_control_characters_and_limits() {
    assert!(validate_wire_string("safe", "field").is_ok());
    assert!(validate_wire_string("evil\u{1b}[2J", "field").is_err());
    assert!(validate_wire_string(&"x".repeat(4097), "field").is_err());
    assert!(validate_wire_strings(&vec!["x".into(); 129], "argv").is_err());
}

#[tokio::test]
async fn policy_store_is_written_with_private_permissions() {
    let dir = crate::test_support::make_temp_dir("shine-broker-policy").await;
    let config = Config::new_for_test(&dir);
    let snapshot = snapshot();
    let allow = allow_from_snapshot(
        &snapshot,
        &["API_TOKEN".into()],
        &["bun".into(), "test".into()],
    )
    .unwrap();
    let store = PolicyStore {
        version: POLICY_VERSION,
        policies: vec![BrokerPolicy {
            name: "dev-api".into(),
            ssh_target: "dev".into(),
            project: "api".into(),
            workspace_sha256: sha256(snapshot.workspace_contents.as_bytes()),
            remote_workspace: None,
            allow: vec![allow],
        }],
    };
    save_store(&config, &store).await.unwrap();
    let loaded = load_store(&config).await.unwrap();
    assert_eq!(loaded, store);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = tokio::fs::metadata(policy_path(&config))
            .await
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    tokio::fs::remove_dir_all(&dir).await.unwrap();
}

#[tokio::test]
async fn invalid_policy_document_diagnostics_omit_contents_and_preserve_storage() {
    let dir = crate::test_support::make_temp_dir("shine-broker-document").await;
    let config = Config::new_for_test(&dir);
    let path = policy_path(&config);
    for contents in [
        "version='BROKER_DOCUMENT_CANARY'\npolicies=[]\n",
        "version=1\npolicies=[]\nunknown='BROKER_DOCUMENT_CANARY' invalid\n",
    ] {
        crate::persist::atomic_write_private(&path, contents.as_bytes())
            .await
            .unwrap();
        let error = load_store(&config).await.unwrap_err();
        let text = format!("{error:#}\n{error:?}");
        assert!(!text.contains("BROKER_DOCUMENT_CANARY"), "{text}");
        assert!(
            text.contains("parsing ") && text.contains("ssh-secret-broker.toml"),
            "{text}"
        );
        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), contents);
    }
    crate::persist::atomic_write_private(&path, b"version=999\npolicies=[]\n")
        .await
        .unwrap();
    assert!(
        load_store(&config)
            .await
            .unwrap_err()
            .to_string()
            .contains("unsupported SSH secret broker policy version 999")
    );
    tokio::fs::remove_dir_all(dir).await.unwrap();
}

#[tokio::test]
async fn additional_local_policy_files_are_merged_and_validated() {
    let dir = crate::test_support::make_temp_dir("shine-broker-policy-merge").await;
    let config = Config::new_for_test(&dir);
    let snapshot = snapshot();
    let allow = allow_from_snapshot(
        &snapshot,
        &["API_TOKEN".into()],
        &["bun".into(), "test".into()],
    )
    .unwrap();
    let extra = PolicyStore {
        version: POLICY_VERSION,
        policies: vec![BrokerPolicy {
            name: "extra-api".into(),
            ssh_target: "dev".into(),
            project: "api".into(),
            workspace_sha256: sha256(snapshot.workspace_contents.as_bytes()),
            remote_workspace: None,
            allow: vec![allow],
        }],
    };
    let path = dir.join("extra.toml");
    tokio::fs::write(&path, toml::to_string(&extra).unwrap())
        .await
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .await
            .unwrap();
    }

    let merged = load_stores(&config, std::slice::from_ref(&path))
        .await
        .unwrap();
    assert_eq!(merged.policies.len(), 1);
    assert_eq!(merged.policies[0].name, "extra-api");
    tokio::fs::remove_dir_all(&dir).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn policy_store_rejects_symlink_and_broad_permissions() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = crate::test_support::make_temp_dir("shine-broker-policy-safety").await;
    let config = Config::new_for_test(&dir);
    let path = policy_path(&config);
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .unwrap();
    let target = dir.join("real-policy.toml");
    tokio::fs::write(&target, "version = 1\n").await.unwrap();
    symlink(&target, &path).unwrap();
    assert!(
        load_store(&config)
            .await
            .unwrap_err()
            .to_string()
            .contains("symlink")
    );
    tokio::fs::remove_file(&path).await.unwrap();
    tokio::fs::write(&path, "version = 1\n").await.unwrap();
    tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
        .await
        .unwrap();
    assert!(
        load_store(&config)
            .await
            .unwrap_err()
            .to_string()
            .contains("too broad")
    );
    tokio::fs::remove_dir_all(&dir).await.unwrap();
}
