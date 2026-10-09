use crate::{
    lifecycle::LifecycleOperation,
    plan::{PlanApprovalError, PlanApprovalV1},
    runtime::{
        CoreRuntime, FileSystemObservationHost, InMemoryHost, NullObserver, PlanningInputVersions,
        PresetSnapshot, PresetSourceKind, PrivilegedFileSystemHost, RuntimeContext,
        RuntimeInteraction, RuntimePlatform, ShellManifest, ShellPlanRequest, ShellType,
        SysManagedPlanRequest, SysRunManifest, command_path_for_name,
    },
};
use anyhow::Result;
use std::{future::Future, pin::Pin, task::Poll};

struct Interaction;

impl RuntimeInteraction for Interaction {
    fn confirm(&mut self, _: &'static str, default: bool) -> Result<bool> {
        Ok(default)
    }

    fn authorize_admin<'a>(
        &'a mut self,
        _: usize,
    ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send + 'a>> {
        Box::pin(async { Ok(true) })
    }

    fn select_many(
        &mut self,
        _: &'static str,
        _: &[String],
        defaults: &[String],
    ) -> Result<Vec<String>> {
        Ok(defaults.to_vec())
    }
}

fn runtime(snapshot: PresetSnapshot) -> CoreRuntime<InMemoryHost> {
    let home = std::env::temp_dir().join("shine-lifecycle-concurrency");
    let shine = home.join("state");
    let mut context = RuntimeContext::isolated(
        home.clone(),
        shine.clone(),
        shine.join("presets"),
        shine.join("bin"),
        RuntimePlatform::Linux,
    );
    context.shell = ShellType::Bash;
    context.shell_config_paths = vec![home.join(".bashrc")];
    CoreRuntime::new(InMemoryHost::new(), context, snapshot)
}

async fn assert_pending(future: Pin<&mut impl Future>) {
    let mut future = future;
    std::future::poll_fn(|cx| {
        assert!(
            future.as_mut().poll(cx).is_pending(),
            "operation did not wait for its lock"
        );
        Poll::Ready(())
    })
    .await;
}

#[tokio::test]
async fn concurrent_managed_sys_installs_preserve_receipts_and_revalidate_approval() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "sys/test/shine.toml",
            br#"version = 2
[[items]]
id = 'a'
label = 'A'
mode = 'managed'
driver = 'managed-file'
[items.config]
source = 'a.txt'
target = '~/.config/a.txt'
[[items]]
id = 'b'
label = 'B'
mode = 'managed'
driver = 'managed-file'
[items.config]
source = 'b.txt'
target = '~/.config/b.txt'
"#
            .to_vec(),
        )
        .file("sys/test/a.txt", b"a".to_vec())
        .file("sys/test/b.txt", b"b".to_vec())
        .build();
    let runtime = runtime(snapshot);
    let request = |target: &str| SysManagedPlanRequest {
        operation: LifecycleOperation::Install,
        os_id: "test".into(),
        target: Some(target.into()),
        input_versions: PlanningInputVersions::default(),
    };
    let a = request("a");
    let b = request("b");
    let approval_a =
        PlanApprovalV1::for_reviewed_plan(&runtime.plan_managed_sys(a.clone()).await.unwrap())
            .unwrap();
    let approval_b =
        PlanApprovalV1::for_reviewed_plan(&runtime.plan_managed_sys(b.clone()).await.unwrap())
            .unwrap();
    let lock = runtime.host().acquire_privileged_operation().await.unwrap();
    let mut interaction_a = Interaction;
    let mut interaction_b = Interaction;
    let mut observer_a = NullObserver;
    let mut observer_b = NullObserver;
    let mut first = Box::pin(runtime.run_managed_sys_approved(
        a,
        &approval_a,
        &mut interaction_a,
        &mut observer_a,
    ));
    let mut second = Box::pin(runtime.run_managed_sys_approved(
        b.clone(),
        &approval_b,
        &mut interaction_b,
        &mut observer_b,
    ));
    assert_pending(first.as_mut()).await;
    assert_pending(second.as_mut()).await;
    drop(lock);
    first.await.unwrap();
    // The complete manifest observation changed while B waited. Waiting does
    // not authorize that new state, even though the targets are independent.
    let error = second.await.unwrap_err();
    assert_eq!(
        error.downcast_ref::<PlanApprovalError>(),
        Some(&PlanApprovalError::PlanChanged)
    );
    let manifest = SysRunManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(manifest.entries.len(), 1);
    assert_eq!(manifest.entries[0].item_id, "a");
    assert!(
        runtime
            .host()
            .metadata(&runtime.context().home_dir.join(".config/b.txt"))
            .await
            .is_err()
    );
    let approval =
        PlanApprovalV1::for_reviewed_plan(&runtime.plan_managed_sys(b.clone()).await.unwrap())
            .unwrap();
    runtime
        .run_managed_sys_approved(b, &approval, &mut Interaction, &mut NullObserver)
        .await
        .unwrap();
    let manifest = SysRunManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(
        manifest
            .entries
            .iter()
            .map(|entry| entry.item_id.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
}

#[tokio::test]
async fn concurrent_shell_install_and_uninstall_preserve_unrelated_receipts() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "shell/a/shine.toml",
            b"[[files]]\nsource = 'a.sh'\ntarget = 'a'\n".to_vec(),
        )
        .file_with_executable("shell/a/a.sh", b"#!/bin/sh\nexit 0\n".to_vec(), true)
        .file(
            "shell/b/shine.toml",
            b"[[files]]\nsource = 'b.sh'\ntarget = 'b'\n".to_vec(),
        )
        .file_with_executable("shell/b/b.sh", b"#!/bin/sh\nexit 0\n".to_vec(), true)
        .build();
    let runtime = runtime(snapshot);
    let request = |target: &str, operation| ShellPlanRequest {
        operation,
        target: Some(target.into()),
        force: false,
        purge: false,
        input_versions: PlanningInputVersions::default(),
    };
    let a = request("a", LifecycleOperation::Install);
    let approval =
        PlanApprovalV1::for_reviewed_plan(&runtime.plan_shells(a.clone()).await.unwrap()).unwrap();
    runtime.install_shells_approved(a, &approval).await.unwrap();
    let b = request("b", LifecycleOperation::Install);
    let remove = request("a", LifecycleOperation::Uninstall);
    let approval_b =
        PlanApprovalV1::for_reviewed_plan(&runtime.plan_shells(b.clone()).await.unwrap()).unwrap();
    let approval_remove =
        PlanApprovalV1::for_reviewed_plan(&runtime.plan_shells(remove.clone()).await.unwrap())
            .unwrap();
    let lock = runtime.host().acquire_privileged_operation().await.unwrap();
    let mut first = Box::pin(runtime.install_shells_approved(b, &approval_b));
    let mut second = Box::pin(runtime.uninstall_shells_approved(remove.clone(), &approval_remove));
    assert_pending(first.as_mut()).await;
    assert_pending(second.as_mut()).await;
    drop(lock);
    first.await.unwrap();
    // Installing an unrelated category does not change A's reviewed removal,
    // but its receipt must survive A's whole-manifest save.
    second.await.unwrap();
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(manifest.entries.len(), 1);
    assert_eq!(manifest.entries[0].category, "b");
    assert!(
        runtime
            .host()
            .metadata(&command_path_for_name(
                &runtime.context().bin_dir,
                "b".as_ref(),
            ))
            .await
            .is_ok()
    );
}
