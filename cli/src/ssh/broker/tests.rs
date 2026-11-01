use super::*;

#[test]
fn confirmation_accepts_plain_and_bracketed_paste_yes() {
    assert!(confirmation_approved("y\n"));
    assert!(confirmation_approved(" YES \r\n"));
    assert!(confirmation_approved("\u{1b}[200~y\u{1b}[201~\n"));
}

#[test]
fn confirmation_rejects_other_or_decorated_input() {
    assert!(!confirmation_approved(""));
    assert!(!confirmation_approved("n\n"));
    assert!(!confirmation_approved("y please\n"));
    assert!(!confirmation_approved("\u{1b}[200~yes\n"));
    assert!(!confirmation_approved("\u{1b}[31my\u{1b}[0m\n"));
}

#[test]
fn enrollment_details_use_bounded_vertical_lists() {
    let snapshot = WorkspaceSnapshot {
        workspace_path: "/srv/api/shine.workspace.toml".into(),
        workspace_contents: "version = 1".into(),
        mode: "production".into(),
        override_process_env: false,
        sources: vec![],
    };
    let allow = broker::BrokerAllow {
        mode: "production".into(),
        argv: vec!["bun".into(), "start".into()],
        release: vec!["FIRST_SECRET".into(), "SECOND_SECRET".into()],
        sources: vec![broker::BrokerSource {
            path: ".env.shine.toml".into(),
            sha256: "a".repeat(64),
            declared_secrets: vec!["FIRST_SECRET".into(), "SECOND_SECRET".into()],
        }],
    };

    let rendered = format_description_details("dev", &snapshot, &allow, "no exact match");
    assert!(rendered.contains("Command argv:\n  [0] bun\n  [1] start"));
    assert!(rendered.contains("Release secrets (2):\n  - FIRST_SECRET\n  - SECOND_SECRET"));
    assert!(rendered.contains("Declared secrets (2):\n      - FIRST_SECRET"));
    assert!(!rendered.contains("FIRST_SECRET, SECOND_SECRET"));
}

use std::{
    future::Future,
    pin::Pin,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

type TestFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;
struct TestInteraction {
    guard_path: PathBuf,
    confirmations: AtomicUsize,
    reject: AtomicBool,
    displays: Mutex<Vec<String>>,
    prompts: Mutex<Vec<String>>,
}
struct GuardFile(PathBuf);
impl Drop for GuardFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
impl BrokerInteraction for TestInteraction {
    fn set_ssh_pid(&self, _: Option<u32>) {}
    fn confirm_prompt(&self, prompt: String) -> TestFuture<'_, LocalConfirmation> {
        Box::pin(async move {
            self.prompts.lock().unwrap().push(prompt);
            self.confirmations.fetch_add(1, Ordering::SeqCst);
            if self.reject.load(Ordering::SeqCst) {
                bail!("test confirmation rejected");
            }
            std::fs::write(&self.guard_path, b"active")?;
            Ok(LocalConfirmation::hold(GuardFile(self.guard_path.clone())))
        })
    }
    fn display_local(&self, contents: String) -> TestFuture<'_, ()> {
        Box::pin(async move {
            self.displays.lock().unwrap().push(contents);
            Ok(())
        })
    }
}
async fn test_session(
    root: &std::path::Path,
    mode: BrokerMode,
    trusted: bool,
) -> (Arc<BrokerSession>, Arc<TestInteraction>) {
    let mut config = Config::new_for_test(root);
    config.env.insert("TOKEN_SECRET".into(), "aGVsbG8=".into());
    let allow = if mode == BrokerMode::Serve {
        vec!["TOKEN".into()]
    } else {
        vec![]
    };
    let mut session = BrokerSession::prepare(
        &config,
        "dev",
        true,
        &[],
        &allow,
        trusted,
        mode == BrokerMode::Inspect,
        mode == BrokerMode::EnrollTrusted,
        mode == BrokerMode::EnrollTrusted,
        None,
    )
    .await
    .unwrap()
    .unwrap();
    let interaction = Arc::new(TestInteraction {
        guard_path: root.join("confirmation-active"),
        confirmations: AtomicUsize::new(0),
        reject: AtomicBool::new(false),
        displays: Mutex::new(Vec::new()),
        prompts: Mutex::new(Vec::new()),
    });
    Arc::get_mut(&mut session).unwrap().interaction = interaction.clone();
    (session, interaction)
}
fn snapshot() -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        workspace_path: "/srv/api/shine.workspace.toml".into(),
        workspace_contents: "version = 1\n[env]\nfiles = ['env/development.toml']\n".into(),
        mode: "development".into(), override_process_env: false,
        sources: vec![broker::SourceSnapshot { path: "env/development.toml".into(), contents: "version = 1\n[secret]\nAPI_TOKEN = true\n[payload]\ndata = 'not-decrypted-by-inspection'\n".into() }],
    }
}

#[tokio::test]
async fn direct_requests_confirm_even_with_session_trust_and_rejections_consume_nonces() {
    let root = crate::test_support::make_temp_dir("shine-broker-interaction").await;
    let (session, interaction) = test_session(&root, BrokerMode::Serve, true).await;
    interaction.reject.store(true, Ordering::SeqCst);
    let nonce = "direct-rejected-nonce-0001";
    let error = session
        .handle_direct(&["TOKEN".into()], &[], nonce)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "test confirmation rejected");
    assert_eq!(interaction.confirmations.load(Ordering::SeqCst), 1);
    assert!(!interaction.guard_path.exists());
    let replay = session
        .handle_direct(&["TOKEN".into()], &[], nonce)
        .await
        .unwrap_err();
    assert!(replay.to_string().contains("already been used"));
    assert_eq!(interaction.confirmations.load(Ordering::SeqCst), 1);
    assert!(
        session
            .handle_direct(&["TOKEN=OTHER".into()], &[], "unauthorized-nonce-0001")
            .await
            .is_err()
    );
    assert_eq!(interaction.confirmations.load(Ordering::SeqCst), 1);
    tokio::fs::remove_dir_all(root).await.unwrap();
}

#[tokio::test]
async fn workspace_policy_match_precedes_confirmation_and_trusted_session_only_skips_that_prompt() {
    let root = crate::test_support::make_temp_dir("shine-broker-policy").await;
    let (mut session, interaction) = test_session(&root, BrokerMode::Serve, true).await;
    let mut snapshot = snapshot();
    snapshot.workspace_contents = "version = 1\n[env]\nfiles = []\n".into();
    snapshot.sources.clear();
    let argv = vec!["bun".into(), "start".into()];
    Arc::get_mut(&mut session)
        .unwrap()
        .policies
        .policies
        .push(broker::BrokerPolicy {
            name: "exact".into(),
            ssh_target: "dev".into(),
            project: "demo".into(),
            workspace_sha256: broker::sha256(snapshot.workspace_contents.as_bytes()),
            remote_workspace: Some(snapshot.workspace_path.clone()),
            allow: vec![broker::allow_from_snapshot(&snapshot, &[], &argv).unwrap()],
        });
    assert!(
        session
            .handle_workspace(
                &snapshot,
                &["bun".into(), "other".into()],
                "workspace-mismatch-0001"
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("no SSH secret broker policy")
    );
    assert_eq!(interaction.confirmations.load(Ordering::SeqCst), 0);
    assert!(
        session
            .handle_workspace(&snapshot, &argv, "workspace-trusted-0001")
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(interaction.confirmations.load(Ordering::SeqCst), 0);
    Arc::get_mut(&mut session).unwrap().trust_remote_session = false;
    interaction.reject.store(true, Ordering::SeqCst);
    assert_eq!(
        session
            .handle_workspace(&snapshot, &argv, "workspace-confirm-0001")
            .await
            .unwrap_err()
            .to_string(),
        "test confirmation rejected"
    );
    assert_eq!(interaction.confirmations.load(Ordering::SeqCst), 1);
    tokio::fs::remove_dir_all(root).await.unwrap();
}

#[tokio::test]
async fn inspect_and_enroll_remain_metadata_only_and_enrollment_requires_confirmation() {
    for mode in [BrokerMode::Inspect, BrokerMode::EnrollTrusted] {
        let root = crate::test_support::make_temp_dir("shine-broker-metadata").await;
        let (session, interaction) = test_session(&root, mode, false).await;
        let snapshot = snapshot();
        let release = vec!["API_TOKEN".into()];
        let argv = vec!["bun".into(), "start".into()];
        assert!(
            session
                .handle_direct(&["TOKEN".into()], &argv, "forbidden-direct-0001")
                .await
                .is_err()
        );
        assert!(
            session
                .handle_workspace(&snapshot, &argv, "forbidden-workspace-0001")
                .await
                .is_err()
        );
        if mode == BrokerMode::EnrollTrusted {
            interaction.reject.store(true, Ordering::SeqCst);
            assert!(
                session
                    .handle_description(&snapshot, &release, &argv, "enroll-rejected-0001")
                    .await
                    .is_err()
            );
            assert!(!broker::policy_path(&session.config).exists());
            interaction.reject.store(false, Ordering::SeqCst);
        }
        let summary = session
            .handle_description(&snapshot, &release, &argv, "metadata-accepted-0001")
            .await
            .unwrap();
        assert!(!summary.contains("not-decrypted-by-inspection"));
        assert!(!interaction.guard_path.exists());
        if mode == BrokerMode::Inspect {
            assert_eq!(interaction.displays.lock().unwrap().len(), 1);
            assert_eq!(interaction.confirmations.load(Ordering::SeqCst), 0);
        } else {
            assert_eq!(interaction.confirmations.load(Ordering::SeqCst), 2);
            assert_eq!(
                broker::load_store(&session.config)
                    .await
                    .unwrap()
                    .policies
                    .len(),
                1
            );
        }
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}

#[test]
fn enrollment_diff_preserves_line_boundaries_and_escapes_controls() {
    assert_eq!(
        escape_multiline("-old\\\t\r\n+new\u{85}\u{1b}[2J\n"),
        "-old\\\\\\t\\r\n+new\\u{85}\\u{1b}[2J\n"
    );
}

#[tokio::test]
async fn enrollment_review_keeps_long_diff_tails_and_requires_fresh_confirmation() {
    let root = crate::test_support::make_temp_dir("shine-broker-long-review").await;
    let config = Config::new_for_test(&root);
    let mut original = snapshot();
    let prefix = format!("/{}", format!("{}/", "a\t".repeat(80)).repeat(20));
    original.sources[0].path = format!("{prefix}OLD.toml");
    let release = vec!["API_TOKEN".into()];
    let argv = vec!["bun".into(), "start".into()];
    let initial = broker::plan_remote_enrollment(
        &PolicyStore::default(),
        "dev",
        &original,
        &release,
        &argv,
        None,
    )
    .unwrap();
    broker::apply_remote_enrollment(&config, &initial)
        .await
        .unwrap();
    let path = broker::policy_path(&config);
    let before = tokio::fs::read(&path).await.unwrap();
    let (mut session, interaction) = test_session(&root, BrokerMode::EnrollTrusted, false).await;
    Arc::get_mut(&mut session).unwrap().enroll_update_policy = Some(initial.name);
    let mut changed = original.clone();
    changed.sources[0].path = format!("{prefix}NEW.toml");
    interaction.reject.store(true, Ordering::SeqCst);
    assert!(
        session
            .handle_description(&changed, &release, &argv, "long-review-rejected-0001")
            .await
            .is_err()
    );
    assert_eq!(tokio::fs::read(&path).await.unwrap(), before);
    {
        let prompts = interaction.prompts.lock().unwrap();
        let diff = prompts[0].split_once("Policy diff:\n").unwrap().1;
        for (sign, tail) in [('-', "OLD.toml\""), ('+', "NEW.toml\"")] {
            let line = diff
                .lines()
                .find(|line| line.starts_with(&format!("{sign}path = ")))
                .unwrap();
            assert!(
                line.len() > 4096,
                "the fixture must exercise the old truncation limit"
            );
            assert_eq!(
                line,
                format!("{sign}path = \"{}{tail}", prefix.replace('\t', "\\\\t"))
            );
        }
        assert!(diff.chars().all(|ch| !ch.is_control() || ch == '\n'));
        assert!(!diff.contains("not-decrypted-by-inspection"));
    }
    interaction.reject.store(false, Ordering::SeqCst);
    // A rejected request's nonce cannot be reused to reach approval or mutation.
    assert!(
        session
            .handle_description(&changed, &release, &argv, "long-review-rejected-0001")
            .await
            .is_err()
    );
    assert_eq!(interaction.confirmations.load(Ordering::SeqCst), 1);
    assert_eq!(tokio::fs::read(&path).await.unwrap(), before);
    session
        .handle_description(&changed, &release, &argv, "long-review-accepted-0001")
        .await
        .unwrap();
    assert_eq!(interaction.confirmations.load(Ordering::SeqCst), 2);
    let stored = broker::load_store(&config).await.unwrap();
    assert_eq!(
        stored.policies[0].allow[0].sources[0].path,
        changed.sources[0].path
    );
    assert_eq!(stored.policies[0].allow[0].argv, argv);
    assert!(!interaction.guard_path.exists());
    assert!(!config.shine_dir().join("trust.toml").exists());
    tokio::fs::remove_dir_all(root).await.unwrap();
}

#[cfg(unix)]
#[allow(clippy::await_holding_lock)]
#[tokio::test(flavor = "current_thread")]
async fn confirmation_guard_covers_real_decryption_success_failure_and_cancellation() {
    use std::os::unix::fs::PermissionsExt;
    let _environment = crate::test_support::env_lock();
    let root = crate::test_support::make_temp_dir("shine-broker-guard").await;
    let (session, interaction) = test_session(&root, BrokerMode::Serve, true).await;
    let tool_dir = root.join("tools");
    tokio::fs::create_dir_all(&tool_dir).await.unwrap();
    let script = tool_dir.join("gpg");
    let seen = root.join("decrypt-saw-confirmation");
    // This controlled local backend checks the guard while the production secret path is active.
    let write_backend = |body: &str| {
        let quote = |path: &std::path::Path| {
            format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
        };
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\n[ -f {} ] || exit 23\n: > {}\n{body}\n",
                quote(&interaction.guard_path),
                quote(&seen)
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    let path = std::env::join_paths(std::iter::once(tool_dir.clone()).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))
    .unwrap();
    // SAFETY: the shared environment guard restores PATH and serializes participating tests.
    unsafe {
        std::env::set_var("PATH", path);
    }
    for (index, success) in [true, false].into_iter().enumerate() {
        write_backend(if success {
            "printf decrypted"
        } else {
            "exit 17"
        });
        let result = session
            .handle_direct(
                &["TOKEN".into()],
                &[],
                &format!("decrypt-result-nonce-{index:04}"),
            )
            .await;
        if success {
            assert_eq!(result.unwrap()["TOKEN"], "decrypted");
        } else {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("decrypting TOKEN_SECRET")
            );
        }
        assert!(seen.exists());
        assert!(!interaction.guard_path.exists());
        std::fs::remove_file(&seen).unwrap();
    }
    write_backend("exec /bin/sleep 60");
    let task_session = session.clone();
    let task = tokio::spawn(async move {
        task_session
            .handle_direct(&["TOKEN".into()], &[], "decrypt-cancel-nonce-0001")
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !seen.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(interaction.guard_path.exists());
    let waiting_session = session.clone();
    let mut waiting = tokio::spawn(async move {
        waiting_session
            .handle_direct(&["TOKEN".into()], &[], "serialized-waiting-nonce-0001")
            .await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), &mut waiting)
            .await
            .is_err()
    );
    assert_eq!(interaction.confirmations.load(Ordering::SeqCst), 3);
    assert!(
        session
            .handle_direct(&["TOKEN".into()], &[], "decrypt-cancel-nonce-0001")
            .await
            .unwrap_err()
            .to_string()
            .contains("already been used")
    );
    // The waiter must not prompt while the first request is decrypting.
    interaction.reject.store(true, Ordering::SeqCst);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .to_string(),
        "test confirmation rejected"
    );
    assert!(!interaction.guard_path.exists());
    assert_eq!(interaction.confirmations.load(Ordering::SeqCst), 4);
    tokio::fs::remove_dir_all(root).await.unwrap();
}

fn file_request() -> super::super::file_broker::FileReleaseRequest {
    super::super::file_broker::FileReleaseRequest {
        operation: "export".into(),
        workspace_path: "/srv/app/shine.workspace.toml".into(),
        workspace_contents:
            "version=3\n[env]\ndefault_mode='production'\n[[file_groups.keys.files]]\nsource='key.enc'\ntarget='private/key'\n"
                .into(),
        mode: "production".into(),
        groups: vec!["keys".into()],
        release: vec!["key.enc".into()],
        ciphertexts: std::collections::BTreeMap::from([("key.enc".into(), "opaque".into())]),
        permissions: Default::default(),
        app: None,
    }
}

#[tokio::test]
async fn file_release_requires_separate_policy_and_rejection_consumes_nonce() {
    let root = crate::test_support::make_temp_dir("file-broker-authorization").await;
    let (mut session, interaction) = test_session(&root, BrokerMode::Serve, false).await;
    let request = file_request();
    let error = session
        .handle_files(&request, false, "no-file-policy-0001")
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("matching local file policy"));
    assert_eq!(interaction.confirmations.load(Ordering::SeqCst), 0);
    Arc::get_mut(&mut session)
        .unwrap()
        .policies
        .file_policies
        .push(super::super::file_broker::FilePolicy {
            name: "keys".into(),
            ssh_target: "dev".into(),
            descriptor: super::super::file_broker::descriptor(&request).unwrap(),
        });
    interaction.reject.store(true, Ordering::SeqCst);
    let error = session
        .handle_files(&request, false, "file-rejected-0001")
        .await
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "test confirmation rejected");
    assert_eq!(interaction.confirmations.load(Ordering::SeqCst), 1);
    let error = session
        .handle_files(&request, false, "file-rejected-0001")
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("already been used"));
    Arc::get_mut(&mut session).unwrap().trust_remote_session = true;
    let mut changed = request;
    changed.workspace_path = "/other/shine.workspace.toml".into();
    assert!(
        session
            .handle_files(&changed, false, "file-tampered-0001")
            .await
            .is_err()
    );
    assert_eq!(interaction.confirmations.load(Ordering::SeqCst), 1);
    tokio::fs::remove_dir_all(root).await.unwrap();
}

#[tokio::test]
async fn file_inspection_never_releases_plaintext() {
    let root = crate::test_support::make_temp_dir("file-broker-inspect").await;
    let (session, interaction) = test_session(&root, BrokerMode::Inspect, false).await;
    let request = file_request();
    assert!(matches!(
        session
            .handle_files(&request, true, "file-inspect-0001")
            .await
            .unwrap(),
        FileResult::Description(_)
    ));
    assert_eq!(interaction.confirmations.load(Ordering::SeqCst), 0);
    assert!(
        session
            .handle_files(&request, false, "file-inspect-0002")
            .await
            .is_err()
    );
    assert!(!interaction.displays.lock().unwrap()[0].contains("opaque"));
    tokio::fs::remove_dir_all(root).await.unwrap();
}
