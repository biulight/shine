use super::*;
use base64::Engine as _;
use shine_core::lifecycle::LifecycleOperation;
use shine_core::plan::PlanApprovalV1;
use shine_core::runtime::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Fake {
    io: Provider,
    calls: AtomicUsize,
    fail_decrypt: AtomicBool,
    fail_receipt: AtomicBool,
    fail_create: AtomicBool,
    fail_applied_journal: AtomicBool,
}
impl SecretFileIo for Fake {
    fn resolve_permissions(&self, p: &SecretFilePermissions) -> SecretFilePermissions {
        self.io.resolve_permissions(p)
    }
    fn observe(&self, p: &Path) -> Result<Option<SecretObservation>> {
        self.io.observe(p)
    }
    fn fingerprint(&self, b: &[u8]) -> Result<String> {
        self.io.fingerprint(b)
    }
    fn state(&self, n: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        self.io.state(n)
    }
    fn save_state(&self, n: &str, b: Option<&[u8]>) -> Result<()> {
        if n == "app-secret-operation.json"
            && b.is_some_and(|bytes| {
                serde_json::from_slice::<serde_json::Value>(bytes).unwrap()["applied"].is_object()
            })
            && self.fail_applied_journal.swap(false, Ordering::SeqCst)
        {
            anyhow::bail!("injected applied journal failure");
        }
        if n == "app-secret-manifest.json" && self.fail_receipt.swap(false, Ordering::SeqCst) {
            anyhow::bail!("injected receipt failure");
        }
        self.io.save_state(n, b)
    }
    fn prepare_key(&self) -> Result<()> {
        self.io.prepare_key()
    }
    fn lock_paths(&self, p: &[PathBuf]) -> Result<Box<dyn Send>> {
        self.io.lock_paths(p)
    }
    fn create(&self, p: &Path, b: &[u8], m: &SecretFilePermissions) -> Result<()> {
        ensure!(
            !self.fail_create.swap(false, Ordering::SeqCst),
            "injected create failure"
        );
        self.io.create(p, b, m)
    }
    fn set_permissions(&self, p: &Path, m: &SecretFilePermissions) -> Result<()> {
        self.io.set_permissions(p, m)
    }
    fn rename(&self, a: &Path, b: &Path) -> Result<()> {
        self.io.rename(a, b)
    }
    fn remove(&self, p: &Path) -> Result<()> {
        self.io.remove(p)
    }
    fn check_target(&self, p: &Path, permissions: &SecretFilePermissions) -> Result<()> {
        self.io.check_target(p, permissions)
    }
}
impl SecretFileProvider for Fake {
    fn decrypt<'a>(&'a self, r: &'a FileReleaseRequest) -> SecretFuture<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            ensure!(
                !self.fail_decrypt.load(Ordering::SeqCst),
                "injected decryption failure"
            );
            Ok(r.released_mappings()?
                .iter()
                .map(|m| {
                    Zeroizing::new(
                        base64::engine::general_purpose::STANDARD
                            .decode(&r.ciphertexts[&m.source])
                            .unwrap(),
                    )
                })
                .collect())
        })
    }
}
struct Interaction;
impl RuntimeInteraction for Interaction {
    fn confirm(&mut self, _: &'static str, _: bool) -> Result<bool> {
        Ok(true)
    }
    fn select_many(
        &mut self,
        _: &'static str,
        _: &[String],
        defaults: &[String],
    ) -> Result<Vec<String>> {
        Ok(defaults.to_vec())
    }
    fn authorize_admin<'a>(
        &'a mut self,
        _: usize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool>> + Send + 'a>> {
        Box::pin(async { Ok(false) })
    }
}

fn runtime(root: &Path, provider: Arc<Fake>, value: &str, auto: bool) -> CoreRuntime<RealHost> {
    let metadata = format!(
        r#"metadata_schema_version=3
dest='~/service'
[[files]]
source='compose.yml'
[[secret_files]]
workspace='shine.workspace.toml'
file_group='credentials'
mode='production'
auto_decrypt={auto}
unix_mode='0400'
"#
    );
    let workspace = r#"version=3
[env]
default_mode='production'
[[file_groups.credentials.files]]
source='encrypted/password.enc'
target='secrets/password'
"#;
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file("app/demo/shine.toml", metadata.into_bytes())
        .file("app/demo/compose.yml", b"services: {}".to_vec())
        .file(
            "app/demo/shine.workspace.toml",
            workspace.as_bytes().to_vec(),
        )
        .file(
            "app/demo/encrypted/password.enc",
            base64::engine::general_purpose::STANDARD
                .encode(value)
                .into_bytes(),
        )
        .build();
    let context = RuntimeContext::isolated(
        root.into(),
        root.join(".shine"),
        root.join("presets"),
        root.join("bin"),
        RuntimePlatform::current(),
    );
    CoreRuntime::new(RealHost, context, snapshot).with_secret_file_provider(provider)
}
fn request(operation: LifecycleOperation) -> AppPlanRequest {
    AppPlanRequest {
        operation,
        target: Some("demo".into()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    }
}
async fn install(runtime: &CoreRuntime<RealHost>, force: bool) -> Result<AppLifecycleReport> {
    let mut request = request(LifecycleOperation::Install);
    request.force = force;
    let plan = runtime.plan_apps(request.clone()).await?;
    let approval = PlanApprovalV1::for_reviewed_plan(&plan)?;
    runtime
        .install_apps_approved(request, &approval, &mut NullObserver, &mut Interaction)
        .await
}
async fn setup() -> (PathBuf, Arc<Fake>) {
    let root = crate::test_support::make_temp_dir("app-secret-lifecycle").await;
    let root = fs::canonicalize(root).unwrap();
    let config = crate::config::Config::new_for_test(&root.join(".shine"));
    (
        root,
        Arc::new(Fake {
            io: Provider::new(config),
            calls: AtomicUsize::new(0),
            fail_decrypt: AtomicBool::new(false),
            fail_receipt: AtomicBool::new(false),
            fail_create: AtomicBool::new(false),
            fail_applied_journal: AtomicBool::new(false),
        }),
    )
}

#[tokio::test]
async fn missing_applied_identity_blocks_creation_and_update_recovery() {
    use std::os::unix::fs::PermissionsExt;
    for update in [false, true] {
        for replaced in [false, true] {
            let (root, provider) = setup().await;
            if update {
                install(&runtime(&root, provider.clone(), "old", true), false)
                    .await
                    .unwrap();
            }
            let runtime = runtime(&root, provider.clone(), "new", true);
            provider.fail_applied_journal.store(true, Ordering::SeqCst);
            assert!(
                install(&runtime, false)
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("injected applied journal failure")
            );
            let target = root.join("service/secrets/password");
            if replaced {
                let replacement = root.join("service/secrets/replacement");
                fs::write(&replacement, "new").unwrap();
                fs::set_permissions(&replacement, fs::Permissions::from_mode(0o400)).unwrap();
                fs::rename(replacement, &target).unwrap();
            }
            let before = provider.observe(&target).unwrap();
            let journal = provider.state("app-secret-operation.json").unwrap();
            let receipt = provider.state("app-secret-manifest.json").unwrap();
            let plan = runtime.plan_app_operation_recovery().await.unwrap();
            assert!(PlanApprovalV1::for_reviewed_plan(&plan).is_err());
            assert_eq!(provider.observe(&target).unwrap(), before);
            assert_eq!(
                provider.state("app-secret-operation.json").unwrap(),
                journal
            );
            assert_eq!(provider.state("app-secret-manifest.json").unwrap(), receipt);
            let rollback = root.join("service/secrets/password.shine.secret.rollback");
            if update {
                assert_eq!(fs::read(rollback).unwrap(), b"old");
            } else {
                assert!(!rollback.exists());
            }
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[tokio::test]
async fn service_owner_missing_parents_fail_before_decryption_or_writes() {
    let (root, provider) = setup().await;
    let original = runtime(&root, provider.clone(), "password", true);
    let uid = unsafe { libc::geteuid() }.saturating_add(1);
    let mut builder = PresetSnapshot::builder(PresetSourceKind::Embedded);
    for (path, bytes) in original.presets().files() {
        let bytes = if path == "app/demo/shine.toml" {
            format!("{}\nunix_uid={uid}\n", String::from_utf8_lossy(bytes)).into_bytes()
        } else {
            bytes.to_vec()
        };
        builder = builder.file(path, bytes);
    }
    let mut context = original.context().clone();
    // The filesystem check must still run when administrator authorization succeeds.
    context.running_as_admin = true;
    let runtime = CoreRuntime::new(RealHost, context, builder.build())
        .with_secret_file_provider(provider.clone());
    assert!(
        install(&runtime, false)
            .await
            .unwrap_err()
            .to_string()
            .contains("pre-create secret parent directories")
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    assert!(!root.join("service").exists());
    assert!(
        provider
            .state("app-secret-operation.json")
            .unwrap()
            .is_none()
    );
    assert!(
        provider
            .state("app-secret-manifest.json")
            .unwrap()
            .is_none()
    );
    assert!(!root.join(".shine/app-secret-fingerprint.key").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn service_owner_requires_traversable_existing_parents() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    // Use the shared temporary root, rather than a per-user 0700 temp directory.
    let root = fs::canonicalize("/tmp")
        .unwrap()
        .join(format!("shine-service-parent-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
    let parent = root.join("secrets");
    fs::create_dir(&parent).unwrap();
    let target = parent.join("password");
    let permissions = SecretFilePermissions {
        uid: Some(unsafe { libc::geteuid() }.saturating_add(1)),
        gid: Some(fs::metadata(&parent).unwrap().gid()),
        mode: 0o400,
    };
    // Group search suffices, but other search must not override a denying group class.
    for (mode, allowed) in [(0o710, true), (0o701, false), (0o700, false)] {
        fs::set_permissions(&parent, fs::Permissions::from_mode(mode)).unwrap();
        assert_eq!(check_parent_access(&target, &permissions).is_ok(), allowed);
        assert_eq!(
            fs::metadata(&parent).unwrap().permissions().mode() & 0o777,
            mode
        );
    }
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o711)).unwrap();
    let other_group = SecretFilePermissions {
        gid: Some(permissions.gid.unwrap().saturating_add(1)),
        ..permissions.clone()
    };
    assert!(check_parent_access(&target, &other_group).is_ok());
    // A searchable immediate parent does not bypass an inaccessible ancestor.
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(check_parent_access(&target, &permissions).is_err());
    assert!(!target.exists());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn secret_install_update_preserve_uninstall_and_redaction() {
    let (root, provider) = setup().await;
    let first = runtime(&root, provider.clone(), "initial-password", true);
    let plan = first
        .plan_apps(request(LifecycleOperation::Install))
        .await
        .unwrap();
    assert!(
        !serde_json::to_string(&plan)
            .unwrap()
            .contains("initial-password")
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    let report = install(&first, false).await.unwrap();
    assert!(
        report
            .files
            .iter()
            .any(|f| f.transforms.iter().any(|t| t == "secret-file"))
    );
    let target = root.join("service/secrets/password");
    assert_eq!(fs::read(&target).unwrap(), b"initial-password");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o400
    );
    install(&first, false).await.unwrap();
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    let second = runtime(&root, provider.clone(), "rotated-password", true);
    let inspection = second
        .inspect_apps_with_options(
            AppInspectionOptions {
                categories: vec!["demo".into()],
                run_generators: false,
            },
            &mut NullObserver,
        )
        .await
        .unwrap();
    let secret = inspection
        .iter()
        .find(|f| f.assessment_diagnostic == Some("app_secret_content_redacted"))
        .unwrap();
    assert_eq!(secret.status, InspectionFileStatus::UpdateAvail);
    assert!(secret.current_content.is_none() && secret.desired_content.is_none());
    let upgrade = request(LifecycleOperation::Upgrade);
    let plan = second.plan_apps(upgrade.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    second
        .upgrade_apps_approved(
            upgrade,
            &approval,
            AppApprovedUpgradeOptions::default(),
            &mut NullObserver,
            &mut Interaction,
        )
        .await
        .unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"rotated-password");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&target, b"local-change").unwrap();
    install(&second, false).await.unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"local-change");
    install(&second, true).await.unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"rotated-password");
    let request = request(LifecycleOperation::Uninstall);
    let plan = second.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    second
        .uninstall_apps_approved(request, &approval, &mut NullObserver, &mut Interaction)
        .await
        .unwrap();
    assert!(!target.exists());
    for path in [
        root.join(".shine/app-secret-manifest.json"),
        root.join(".shine/app-manifest.toml"),
    ] {
        let bytes = fs::read(path).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("rotated-password"));
    }
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn secret_adoption_conflict_and_decrypt_failure_do_not_write_configs() {
    let (root, provider) = setup().await;
    let runtime = runtime(&root, provider.clone(), "password", true);
    provider.fail_decrypt.store(true, Ordering::SeqCst);
    assert!(install(&runtime, false).await.is_err());
    assert!(!root.join("service/compose.yml").exists());
    provider.fail_decrypt.store(false, Ordering::SeqCst);
    fs::create_dir_all(root.join("service/secrets")).unwrap();
    let target = root.join("service/secrets/password");
    fs::write(&target, b"different").unwrap();
    assert!(install(&runtime, true).await.is_err());
    assert!(!root.join("service/compose.yml").exists());
    fs::write(&target, b"password").unwrap();
    install(&runtime, false).await.unwrap();
    assert!(!target.with_extension("shine.bak").exists());
    assert_eq!(runtime.secret_store().unwrap().entries.len(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn secret_receipt_failure_recovers_the_previous_file() {
    let (root, provider) = setup().await;
    install(&runtime(&root, provider.clone(), "old", true), false)
        .await
        .unwrap();
    let new = runtime(&root, provider.clone(), "new", true);
    provider.fail_receipt.store(true, Ordering::SeqCst);
    assert!(install(&new, false).await.is_err());
    let plan = new.plan_app_operation_recovery().await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    new.recover_app_operation_approved(&approval).await.unwrap();
    assert_eq!(
        fs::read(root.join("service/secrets/password")).unwrap(),
        b"old"
    );
    assert!(!root.join(".shine/app-secret-operation.json").exists());
    install(&new, false).await.unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn manual_declaration_installs_only_ciphertext_and_workspace() {
    let (root, provider) = setup().await;
    install(&runtime(&root, provider.clone(), "cipher", false), false)
        .await
        .unwrap();
    assert!(root.join("service/encrypted/password.enc").exists());
    assert!(!root.join("service/secrets/password").exists());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn preview_and_stale_approval_never_decrypt_or_overwrite() {
    let (root, provider) = setup().await;
    let runtime = runtime(&root, provider.clone(), "approved", true);
    let preview = runtime
        .preview_install_apps(
            AppLifecycleRequest {
                target: Some("demo".into()),
                dry_run: true,
                force: false,
            },
            &mut NullObserver,
            &mut Interaction,
        )
        .await
        .unwrap();
    assert!(
        preview
            .files
            .iter()
            .any(|f| f.transforms.iter().any(|t| t == "secret-file"))
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    let request = request(LifecycleOperation::Install);
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    fs::create_dir_all(root.join("service/secrets")).unwrap();
    fs::write(root.join("service/secrets/password"), "concurrent").unwrap();
    assert!(
        runtime
            .install_apps_approved(request, &approval, &mut NullObserver, &mut Interaction)
            .await
            .is_err()
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    assert!(!root.join("service/compose.yml").exists());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn disabling_auto_keeps_ownership_and_missing_key_blocks_mutation() {
    let (root, provider) = setup().await;
    install(&runtime(&root, provider.clone(), "old", true), false)
        .await
        .unwrap();
    let manual = runtime(&root, provider.clone(), "new", false);
    install(&manual, false).await.unwrap();
    assert_eq!(
        fs::read(root.join("service/secrets/password")).unwrap(),
        b"old"
    );
    assert_eq!(manual.secret_store().unwrap().entries.len(), 1);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    fs::remove_file(root.join(".shine/app-secret-fingerprint.key")).unwrap();
    assert!(install(&manual, false).await.is_err());
    assert_eq!(
        fs::read(root.join("service/secrets/password")).unwrap(),
        b"old"
    );
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn recovery_preserves_replaced_destination() {
    let (root, provider) = setup().await;
    install(&runtime(&root, provider.clone(), "old", true), false)
        .await
        .unwrap();
    let updated = runtime(&root, provider.clone(), "new", true);
    provider.fail_receipt.store(true, Ordering::SeqCst);
    assert!(install(&updated, false).await.is_err());
    let target = root.join("service/secrets/password");
    let replacement = root.join("service/secrets/replacement");
    fs::write(&replacement, "new").unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&replacement, fs::Permissions::from_mode(0o400)).unwrap();
    fs::rename(replacement, &target).unwrap();
    let plan = updated.plan_app_operation_recovery().await.unwrap();
    assert!(PlanApprovalV1::for_reviewed_plan(&plan).is_err());
    assert!(root.join(".shine/app-secret-operation.json").exists());
    assert_eq!(fs::read(&target).unwrap(), b"new");
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn removed_secret_declaration_requires_explicit_prune() {
    let (root, provider) = setup().await;
    let original = runtime(&root, provider.clone(), "password", true);
    install(&original, false).await.unwrap();
    let mut builder = PresetSnapshot::builder(PresetSourceKind::Embedded);
    for (path, bytes) in original.presets().files() {
        let bytes = if path == "app/demo/shine.toml" {
            String::from_utf8(bytes.to_vec())
                .unwrap()
                .split("[[secret_files]]")
                .next()
                .unwrap()
                .as_bytes()
                .to_vec()
        } else {
            bytes.to_vec()
        };
        builder = builder.file(path, bytes);
    }
    let changed = CoreRuntime::new(RealHost, original.context().clone(), builder.build())
        .with_secret_file_provider(provider.clone());
    for prune in [false, true] {
        let mut request = request(LifecycleOperation::Upgrade);
        request.prune_stale = prune;
        let plan = changed.plan_apps(request.clone()).await.unwrap();
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        changed
            .upgrade_apps_approved(
                request,
                &approval,
                AppApprovedUpgradeOptions::default(),
                &mut NullObserver,
                &mut Interaction,
            )
            .await
            .unwrap();
        assert_eq!(root.join("service/secrets/password").exists(), !prune);
        assert_eq!(
            changed.secret_store().unwrap().entries.len(),
            usize::from(!prune)
        );
    }
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn interrupted_permission_repair_protects_rollback_and_restores_original() {
    use std::os::unix::fs::PermissionsExt;
    let (root, provider) = setup().await;
    let runtime = runtime(&root, provider.clone(), "password", true);
    install(&runtime, false).await.unwrap();
    let target = root.join("service/secrets/password");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
    fs::write(&target, "local-change").unwrap();
    provider.fail_create.store(true, Ordering::SeqCst);
    assert!(install(&runtime, true).await.is_err());
    let rollback = root.join("service/secrets/password.shine.secret.rollback");
    assert_eq!(
        fs::metadata(&rollback).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(fs::read(&rollback).unwrap(), b"local-change");
    let plan = runtime.plan_app_operation_recovery().await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .recover_app_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"local-change");
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o644
    );
    assert!(!rollback.exists());
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn initial_secret_receipt_failure_recovers_only_the_created_file() {
    use std::os::unix::fs::PermissionsExt;
    for replaced in [false, true] {
        let (root, provider) = setup().await;
        let runtime = runtime(&root, provider.clone(), "new", true);
        provider.fail_receipt.store(true, Ordering::SeqCst);
        assert!(install(&runtime, false).await.is_err());
        let target = root.join("service/secrets/password");
        if replaced {
            let replacement = root.join("service/secrets/replacement");
            fs::write(&replacement, "new").unwrap();
            fs::set_permissions(&replacement, fs::Permissions::from_mode(0o400)).unwrap();
            fs::rename(replacement, &target).unwrap();
        }
        let plan = runtime.plan_app_operation_recovery().await.unwrap();
        let approval = PlanApprovalV1::for_reviewed_plan(&plan);
        if replaced {
            assert!(approval.is_err());
            assert_eq!(fs::read(&target).unwrap(), b"new");
            assert!(root.join(".shine/app-secret-operation.json").exists());
        } else {
            runtime
                .recover_app_operation_approved(&approval.unwrap())
                .await
                .unwrap();
            assert!(!target.exists());
            assert!(!root.join(".shine/app-secret-operation.json").exists());
        }
        fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn secret_and_ordinary_ownership_reject_case_alias_takeover() {
    for secret_first in [false, true] {
        let (root, provider) = setup().await;
        let secret = runtime(&root, provider.clone(), "password", true);
        let mut builder = PresetSnapshot::builder(PresetSourceKind::Embedded);
        for (path, bytes) in secret.presets().files() {
            let bytes = if path == "app/demo/shine.toml" {
                b"metadata_schema_version=2\ndest='~/service'\n[[files]]\nsource='compose.yml'\ntarget='SECRETS/password'\n".to_vec()
            } else {
                bytes.to_vec()
            };
            builder = builder.file(path, bytes);
        }
        let ordinary = CoreRuntime::new(RealHost, secret.context().clone(), builder.build())
            .with_secret_file_provider(provider.clone());
        let (first, next) = if secret_first {
            (&secret, &ordinary)
        } else {
            (&ordinary, &secret)
        };
        install(first, false).await.unwrap();
        let plan = next
            .plan_apps(request(LifecycleOperation::Install))
            .await
            .unwrap();
        assert!(PlanApprovalV1::for_reviewed_plan(&plan).is_err());
        assert!(install(next, true).await.is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
