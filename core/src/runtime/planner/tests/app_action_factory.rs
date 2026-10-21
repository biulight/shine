//! Approved factory regressions spanning resource dispatch, authority and live guards.
use super::*;
use crate::action::ActionKindV1;
use crate::install::{AppInstallStrategy, hash_content};

fn request(operation: LifecycleOperation) -> AppPlanRequest {
    AppPlanRequest {
        operation,
        target: Some("demo".into()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: Default::default(),
    }
}

fn receipt(resource: &str, destination: PathBuf, bytes: &[u8], json: bool) -> AppEntry {
    let strategy = if json {
        AppInstallStrategy::JsonMerge {
            managed_keys: vec!["owned".into()],
        }
    } else {
        AppInstallStrategy::Copy
    };
    let content_hash = if json {
        crate::runtime::managed_json::managed_json_hash(bytes, &["owned".into()]).unwrap()
    } else {
        hash_content(bytes)
    };
    AppEntry {
        source: format!("app/demo/{resource}"),
        destination,
        backup: None,
        content_hash,
        install_strategy: strategy,
        uses_env: false,
        requires_admin: false,
    }
}

async fn factory_actions(
    runtime: &CoreRuntime<ObservationOnlyHost>,
    host: &InMemoryHost,
    request: &AppPlanRequest,
    plan: &PlanV1,
) -> Vec<ActionIrV1> {
    let approval = PlanApprovalV1::for_reviewed_plan(plan).unwrap();
    let before = host.operations().len();
    let actions = runtime
        .approved_app_file_action_irs(request, plan, &approval)
        .await
        .unwrap();
    assert!(
        host.operations()[before..]
            .iter()
            .all(|op| matches!(op, HostOperation::Read(_)))
    );
    for ir in &actions {
        ir.validate().unwrap();
        let permissions = ir.permission_requirements(|path| review_path(runtime.context(), path));
        assert!(permissions.uncomputable_codes.is_empty());
        assert!(
            permissions
                .required
                .iter()
                .all(|permission| approval.approved_permissions.contains(permission))
        );
    }
    actions
}

#[tokio::test]
async fn app_factory_mixed_install_keeps_order_identities_payload_and_preserve_boundaries() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "app/demo/shine.toml",
            br#"
dest = '~/.config/demo'
[permissions]
schema_version = 1
administrator = true
environment = [{ name = 'ENABLE', sensitivity = 'plain' }]
[[files]]
source = 'a-create'
requires_admin = true
[[files]]
source = 'b-backup'
[[files]]
source = 'c-update'
[[files]]
source = 'd-new.json'
install_mode = 'json-merge'
managed_keys = ['owned']
[[files]]
source = 'e-update.json'
install_mode = 'json-merge'
managed_keys = ['owned']
[[files]]
source = 'f-preserve'
[[files]]
source = 'g-generated'
generator = { script = 'gen.ts', runtime = 'bun', env = ['ENABLE'], when_env = 'ENABLE' }
"#
            .to_vec(),
        )
        .file("app/demo/a-create", b"private desired bytes".to_vec())
        .file("app/demo/b-backup", b"private desired bytes".to_vec())
        .file("app/demo/c-update", b"private desired bytes".to_vec())
        .file(
            "app/demo/d-new.json",
            br#"{"owned":"private desired JSON"}"#.to_vec(),
        )
        .file(
            "app/demo/e-update.json",
            br#"{"owned":"private desired JSON"}"#.to_vec(),
        )
        .file("app/demo/f-preserve", b"new managed".to_vec())
        .file("app/demo/g-generated", b"fallback".to_vec())
        .file(
            "app/demo/gen.ts",
            b"throw new Error('must not run');".to_vec(),
        )
        .build();
    let (runtime, host) = observation_runtime(snapshot);
    let root = runtime.context().home_dir.join(".config/demo");
    host.put_file(root.join("b-backup"), b"unowned original".to_vec());
    host.put_file_with_mode(root.join("c-update"), b"old managed".to_vec(), 0o100640);
    host.put_file(
        root.join("e-update.json"),
        br#"{"owned":"old","user":"keep"}"#.to_vec(),
    );
    host.put_file(root.join("f-preserve"), b"user modified".to_vec());
    let mut copy = receipt("c-update", root.join("c-update"), b"old managed", false);
    copy.backup = Some(crate::install::backup_path(&copy.destination));
    host.put_file(
        copy.backup.as_ref().unwrap(),
        b"persistent original".to_vec(),
    );
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![
            copy,
            receipt(
                "e-update.json",
                root.join("e-update.json"),
                br#"{"owned":"old"}"#,
                true,
            ),
            receipt("f-preserve", root.join("f-preserve"), b"old managed", false),
        ],
    }
    .save(&host, &runtime.context().shine_dir)
    .await
    .unwrap();
    let request = request(LifecycleOperation::Install);
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.is_ready(), "{plan:?}");
    let actions = factory_actions(&runtime, &host, &request, &plan).await;
    let resources = [
        "a-create",
        "b-backup",
        "c-update",
        "d-new.json",
        "e-update.json",
    ];
    assert_eq!(
        actions
            .iter()
            .map(|ir| ir.actions[0].resource.as_str())
            .collect::<Vec<_>>(),
        resources
    );
    let fingerprint = plan.fingerprint().unwrap().as_hex();
    for ((ir, resource), prefix) in actions.iter().zip(resources).zip([
        "create",
        "create-with-backup",
        "update",
        "merge-json",
        "merge-json",
    ]) {
        assert_eq!(
            ir.operation_id,
            format!("app-install:{fingerprint}:app/demo/{resource}")
        );
        assert_eq!(ir.actions.len(), 1);
        assert_eq!(
            ir.actions[0].action_id,
            format!("{prefix}:app/demo/{resource}")
        );
        assert_eq!(ir.actions[0].target, "app/demo");
    }
    assert_eq!(
        actions[0].actions[0].kind,
        ActionKindV1::CreateManagedFile {
            destination: root.join("a-create"),
            desired_hash: hash_content(b"private desired bytes"),
            requires_admin: true
        }
    );
    assert_eq!(
        actions[1].actions[0].kind,
        ActionKindV1::CreateManagedFileWithBackup {
            destination: root.join("b-backup"),
            backup: crate::install::backup_path(&root.join("b-backup")),
            original_hash: hash_content(b"unowned original"),
            desired_hash: hash_content(b"private desired bytes"),
            requires_admin: false
        }
    );
    assert_eq!(
        actions[2].actions[0].kind,
        ActionKindV1::UpdateManagedFile {
            destination: root.join("c-update"),
            rollback: managed_file_rollback_path(&root.join("c-update")),
            previous_backup: Some(crate::install::backup_path(&root.join("c-update"))),
            original_mode: Some(0o100640),
            original_hash: hash_content(b"old managed"),
            desired_hash: hash_content(b"private desired bytes"),
            requires_admin: false
        }
    );
    for (index, previous) in [
        (3, None),
        (4, Some(hash_content(br#"{"owned":"old","user":"keep"}"#))),
    ] {
        assert!(
            matches!(&actions[index].actions[0].kind, ActionKindV1::MergeManagedJson { original_hash, previous_receipt_hash, managed_keys, .. }
            if *original_hash == previous && previous_receipt_hash.is_some() == previous.is_some() && managed_keys == &["owned"])
        );
    }
    let encoded = toml::to_string(&actions[0]).unwrap() + &toml::to_string(&actions[4]).unwrap();
    for payload in [
        "private desired bytes",
        "private desired JSON",
        "user modified",
        "persistent original",
    ] {
        assert!(!encoded.contains(payload));
    }
    assert_eq!(
        host.read(&root.join("f-preserve")).await.unwrap(),
        b"user modified"
    );
    assert!(host.metadata(&root.join("g-generated")).await.is_err());
}

#[tokio::test]
async fn app_factory_mixed_upgrade_keeps_relocation_contracts_and_stale_prune_last() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "app/demo/shine.toml",
            br#"
dest = '~/.config/next'
[permissions]
schema_version = 1
administrator = true
environment = [{ name = 'VALUE', sensitivity = 'plain' }]
[[files]]
source = 'copy'
transforms = ['template']
requires_admin = true
[[files]]
source = 'settings.json'
install_mode = 'json-merge'
managed_keys = ['next']
"#
            .to_vec(),
        )
        .file("app/demo/copy", b"@@VALUE@@".to_vec())
        .file("app/demo/settings.json", br#"{"next":"new"}"#.to_vec())
        .build();
    let (mut runtime, host) = observation_runtime(snapshot);
    runtime
        .context_mut_for_test()
        .env
        .insert("VALUE".into(), "rendered private".into());
    let old = runtime.context().home_dir.join(".config/previous");
    let next = runtime.context().home_dir.join(".config/next");
    host.put_file_with_mode(old.join("copy"), b"old copy".to_vec(), 0o100640);
    let mut copy = receipt("copy", old.join("copy"), b"old copy", false);
    copy.backup = Some(crate::install::backup_path(&copy.destination));
    host.put_file_with_mode(
        copy.backup.as_ref().unwrap(),
        b"user backup".to_vec(),
        0o100600,
    );
    let backup = copy.backup.clone().unwrap();
    host.put_file_with_mode(
        old.join("settings.json"),
        br#"{"owned":"old","user":"keep"}"#.to_vec(),
        0o100644,
    );
    host.put_file(old.join("stale"), b"stale managed".to_vec());
    let json = receipt(
        "settings.json",
        old.join("settings.json"),
        br#"{"owned":"old"}"#,
        true,
    );
    let old_json_hash = json.content_hash;
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![
            copy,
            json,
            receipt("stale", old.join("stale"), b"stale managed", false),
        ],
    }
    .save(&host, &runtime.context().shine_dir)
    .await
    .unwrap();
    let mut request = request(LifecycleOperation::Upgrade);
    request.prune_stale = true;
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.is_ready(), "{plan:?}");
    let actions = factory_actions(&runtime, &host, &request, &plan).await;
    assert_eq!(
        actions
            .iter()
            .map(|ir| ir.actions[0].resource.as_str())
            .collect::<Vec<_>>(),
        ["copy", "settings.json", "stale"]
    );
    assert!(
        matches!(&actions[0].actions[0].kind, ActionKindV1::RelocateManagedFile { previous_destination, desired_destination, previous_present, previous_mode, previous_hash, desired_hash, previous_backup: Some(previous_backup), previous_uses_env, desired_uses_env, previous_requires_admin, desired_requires_admin, previous_rollback }
        if previous_destination == &old.join("copy") && desired_destination == &next.join("copy") && *previous_present && *previous_mode == Some(0o100640) && *previous_hash == hash_content(b"old copy") && *desired_hash == hash_content(b"rendered private") && previous_backup.path == backup && previous_backup.mode == Some(0o100600) && previous_backup.hash == hash_content(b"user backup") && !previous_uses_env && *desired_uses_env && !previous_requires_admin && *desired_requires_admin && previous_rollback == &managed_file_rollback_path(&old.join("copy")))
    );
    assert!(
        matches!(&actions[1].actions[0].kind, ActionKindV1::RelocateManagedJson { previous_destination, desired_destination, previous_present, previous_mode, previous_original_hash, previous_receipt_hash, previous_managed_keys, desired_managed_keys, previous_rollback, .. }
        if previous_destination == &old.join("settings.json") && desired_destination == &next.join("settings.json") && *previous_present && *previous_mode == Some(0o100644) && *previous_original_hash == Some(hash_content(br#"{"owned":"old","user":"keep"}"#)) && *previous_receipt_hash == old_json_hash && previous_managed_keys == &["owned"] && desired_managed_keys == &["next"] && previous_rollback == &managed_file_rollback_path(&old.join("settings.json")))
    );
    assert!(
        matches!(&actions[2].actions[0].kind, ActionKindV1::RemoveManagedFile { destination, .. } if destination == &old.join("stale"))
    );
    let fingerprint = plan.fingerprint().unwrap().as_hex();
    assert_eq!(
        actions[0].operation_id,
        format!("app-upgrade:{fingerprint}:app/demo/copy")
    );
    assert_eq!(
        actions[1].actions[0].action_id,
        "relocate-json:app/demo/settings.json"
    );
    assert_eq!(
        actions[2].operation_id,
        format!("app-upgrade-prune:{fingerprint}:app/demo/stale")
    );
    for ir in &actions {
        assert!(!toml::to_string(ir).unwrap().contains("rendered private"));
    }
}

struct UpdateFixture {
    runtime: CoreRuntime<ObservationOnlyHost>,
    host: InMemoryHost,
    request: AppPlanRequest,
    plan: PlanV1,
    previous: PathBuf,
    destination: PathBuf,
}

async fn update_fixture(json: bool, relocate: bool) -> UpdateFixture {
    let strategy = if json {
        "install_mode = 'json-merge'\nmanaged_keys = ['owned']\n"
    } else {
        ""
    };
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file("app/demo/shine.toml", format!("dest = '~/.config/next'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config'\n{strategy}").into_bytes())
        .file("app/demo/config", if json { br#"{"owned":"next"}"#.to_vec() } else { b"next".to_vec() })
        .build();
    let (runtime, host) = observation_runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/next/config");
    let previous = if relocate {
        runtime.context().home_dir.join(".config/previous/config")
    } else {
        destination.clone()
    };
    let original = if json {
        br#"{"owned":"old","user":"keep"}"#.as_slice()
    } else {
        b"old"
    };
    host.put_file_with_mode(&previous, original.to_vec(), 0o100640);
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![receipt("config", previous.clone(), original, json)],
    }
    .save(&host, &runtime.context().shine_dir)
    .await
    .unwrap();
    let request = request(LifecycleOperation::Upgrade);
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.is_ready(), "{plan:?}");
    UpdateFixture {
        runtime,
        host,
        request,
        plan,
        previous,
        destination,
    }
}

#[tokio::test]
async fn app_factory_relocation_requires_typed_intent_and_checks_destination_first() {
    for json in [false, true] {
        let fixture = update_fixture(json, true).await;
        let mut untyped = fixture.plan.clone();
        let step = untyped
            .steps
            .iter_mut()
            .find(|step| {
                step.resource.as_deref() == Some("config") && step.action == PlanActionV1::Update
            })
            .unwrap();
        step.kind = None;
        assert!(
            factory_actions(&fixture.runtime, &fixture.host, &fixture.request, &untyped)
                .await
                .is_empty()
        );

        let approval = PlanApprovalV1::for_reviewed_plan(&fixture.plan).unwrap();
        fixture
            .host
            .put_file(&fixture.destination, b"late target".to_vec());
        fixture
            .host
            .put_file(&fixture.previous, b"late source".to_vec());
        let before = fixture.host.operations().len();
        let error = fixture
            .runtime
            .approved_app_file_action_irs(&fixture.request, &fixture.plan, &approval)
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            if json {
                "managed JSON relocation destination changed after Plan approval"
            } else {
                "App relocation destination changed after Plan approval"
            }
        );
        assert!(
            fixture.host.operations()[before..]
                .iter()
                .all(|op| matches!(op, HostOperation::Read(path) if path != &fixture.previous))
        );
        assert_eq!(
            fixture.host.read(&fixture.destination).await.unwrap(),
            b"late target"
        );
        assert_eq!(
            fixture.host.read(&fixture.previous).await.unwrap(),
            b"late source"
        );
    }
}

#[tokio::test]
async fn app_factory_all_update_handlers_recheck_late_rollback_occupants_without_effects() {
    for (json, relocate, expected) in [
        (
            false,
            false,
            "App update rollback path changed after Plan approval",
        ),
        (
            true,
            false,
            "managed JSON rollback path changed after Plan approval",
        ),
        (
            false,
            true,
            "App relocation rollback path changed after Plan approval",
        ),
        (
            true,
            true,
            "managed JSON relocation rollback path changed after Plan approval",
        ),
    ] {
        let fixture = update_fixture(json, relocate).await;
        let approval = PlanApprovalV1::for_reviewed_plan(&fixture.plan).unwrap();
        let rollback = managed_file_rollback_path(&fixture.previous);
        fixture
            .host
            .put_file(&rollback, b"late rollback occupant".to_vec());
        let before = fixture.host.operations().len();
        let error = fixture
            .runtime
            .approved_app_file_action_irs(&fixture.request, &fixture.plan, &approval)
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), expected);
        assert!(
            fixture.host.operations()[before..]
                .iter()
                .all(|op| matches!(op, HostOperation::Read(_)))
        );
        assert_eq!(
            fixture.host.read(&rollback).await.unwrap(),
            b"late rollback occupant"
        );
    }
}

#[tokio::test]
async fn app_factory_rejects_invalid_approval_before_any_host_observation() {
    let fixture = update_fixture(false, false).await;
    let approval = PlanApprovalV1::for_reviewed_plan(&fixture.plan).unwrap();
    let mut changed = fixture.plan.clone();
    changed.steps[0]
        .diagnostic_codes
        .push("changed-after-review".into());
    let before = fixture.host.operations().len();
    assert!(
        fixture
            .runtime
            .approved_app_file_action_irs(&fixture.request, &changed, &approval)
            .await
            .is_err()
    );
    assert!(fixture.host.operations()[before..].is_empty());
}
