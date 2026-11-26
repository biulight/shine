use super::*;

fn request() -> SysInitPlanRequest {
    SysInitPlanRequest {
        os_id: "ubuntu".into(),
        force_profile: false,
    }
}
fn snapshot(kind: PresetSourceKind, pre: Option<&str>, post: Option<&str>) -> PresetSnapshot {
    let mut builder =
        PresetSnapshot::builder(kind).file("sys/ubuntu/shine.toml", b"version = 2\n".to_vec());
    for (phase, content) in [("pre", pre), ("post", post)] {
        if let Some(content) = content {
            builder = builder.file(
                format!("sys/ubuntu/profile/base.{phase}.sh"),
                content.as_bytes().to_vec(),
            );
        }
    }
    builder.build()
}
fn enabled_tool_manifest(os_id: &str, detail: &str) -> Vec<u8> {
    toml::to_string(&SysRunManifest {
        schema_version: crate::runtime::SYS_MANIFEST_SCHEMA_VERSION,
        entries: vec![SysRunEntry {
            os_id: os_id.into(),
            item_id: "tool".into(),
            label: "Tool".into(),
            status: crate::runtime::SysItemStatus::Installed,
            detail: detail.into(),
            updated_at: "1".into(),
            managed: false,
            profile_enabled: true,
            receipt: None,
        }],
    })
    .unwrap()
    .into_bytes()
}

async fn approve_init(runtime: &CoreRuntime<InMemoryHost>) -> PlanApprovalV1 {
    let plan = runtime.plan_sys_init(request()).await.unwrap();
    PlanApprovalV1::for_reviewed_plan(&plan).unwrap()
}

async fn apply(runtime: &CoreRuntime<InMemoryHost>) -> crate::runtime::SysItemOutcome {
    let approval = approve_init(runtime).await;
    runtime
        .init_sys_profile_approved(request(), &approval)
        .await
        .unwrap()
}

#[tokio::test]
async fn sys_init_no_items_preserves_code_and_is_idempotent_without_receipts_or_execution() {
    let runtime = runtime(snapshot(
        PresetSourceKind::Embedded,
        Some("export PRE=1\nhello() { echo hi; }\n"),
        Some("set -o vi\n"),
    ));
    let plan = runtime.plan_sys_init(request()).await.unwrap();
    assert_eq!(plan.operation, PlanOperationV1::SysInit);
    assert!(plan.is_ready());
    assert!(
        runtime
            .host()
            .operations()
            .iter()
            .all(|op| matches!(op, HostOperation::Read(_)))
    );
    assert_eq!(
        apply(&runtime).await.status,
        crate::runtime::SysItemStatus::Updated
    );
    let active = runtime
        .context()
        .home_dir
        .join(".shine/profile/ubuntu-sys.pre.sh");
    assert_eq!(
        runtime.host().read(&active).await.unwrap(),
        b"export PRE=1\nhello() { echo hi; }\n"
    );
    let since = runtime.host().operations().len();
    assert_eq!(
        apply(&runtime).await.status,
        crate::runtime::SysItemStatus::Skipped
    );
    assert!(
        !runtime.host().operations()[since..]
            .iter()
            .any(|op| matches!(op, HostOperation::Write(_)))
    );
    assert!(!runtime.host().operations().iter().any(|op| matches!(
        op,
        HostOperation::Run { .. } | HostOperation::AuthorizeAdmin { .. }
    )));
    assert!(
        runtime
            .host()
            .metadata(&runtime.context().shine_dir.join("sys-manifest.toml"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn sys_init_external_base_requires_its_own_scoped_grant_even_without_items() {
    let mut runtime = runtime(snapshot(
        PresetSourceKind::External,
        Some("export PRE=1\n"),
        None,
    ));
    assert!(!runtime.plan_sys_init(request()).await.unwrap().is_ready());
    let requirements = runtime
        .external_code_requirements("sys/profile/ubuntu")
        .await
        .unwrap()
        .requirements;
    assert_eq!(requirements.len(), 1);
    assert_eq!(
        requirements[0].capability,
        TrustCapabilityV1::SysProfileCode
    );
    let mut wrong = crate::trust::TrustGrantV1::for_reviewed_requirement(&requirements[0]);
    wrong.target = "sys/tool".into();
    runtime.context_mut_for_test().trust_grants = vec![wrong];
    assert!(!runtime.plan_sys_init(request()).await.unwrap().is_ready());
    runtime.context_mut_for_test().trust_grants =
        vec![crate::trust::TrustGrantV1::for_reviewed_requirement(
            &requirements[0],
        )];
    assert!(runtime.plan_sys_init(request()).await.unwrap().is_ready());
    apply(&runtime).await;
}

#[tokio::test]
async fn sys_init_missing_base_is_empty_and_needs_no_external_base_grant() {
    let runtime = runtime(snapshot(PresetSourceKind::External, None, None));
    assert!(
        runtime
            .external_code_requirements("sys/profile/ubuntu")
            .await
            .unwrap()
            .requirements
            .is_empty()
    );
    assert!(runtime.plan_sys_init(request()).await.unwrap().is_ready());
    apply(&runtime).await;
    assert_eq!(
        runtime
            .host()
            .read(
                &runtime
                    .context()
                    .home_dir
                    .join(".shine/profile/ubuntu-sys.pre.sh")
            )
            .await
            .unwrap(),
        b""
    );
}

#[tokio::test]
async fn sys_init_rejects_changes_to_every_profile_input_and_manifest_before_writes() {
    for relative in [
        ".bashrc",
        ".zshrc",
        ".shine/profile/ubuntu-sys.pre.sh",
        ".shine/profile/ubuntu-sys.post.base.sh",
        ".shine/profile/ubuntu-sys.pre.new.sh",
        ".shine/profile/ubuntu-sys.post.merge.sh",
        ".shine/sys-manifest.toml",
    ] {
        let runtime = runtime(snapshot(
            PresetSourceKind::Embedded,
            Some("export PRE=1\n"),
            None,
        ));
        let approval = approve_init(&runtime).await;
        runtime.host().put_file(
            runtime.context().home_dir.join(relative),
            b"changed".to_vec(),
        );
        let since = runtime.host().operations().len();
        assert!(
            runtime
                .init_sys_profile_approved(request(), &approval)
                .await
                .is_err(),
            "{relative}"
        );
        assert!(
            !runtime.host().operations()[since..]
                .iter()
                .any(|op| matches!(op, HostOperation::Write(_) | HostOperation::Run { .. })),
            "{relative}"
        );
    }
}

#[tokio::test]
async fn sys_init_rejects_unsupported_shell_and_corrupt_recovery() {
    let mut runtime = runtime(snapshot(PresetSourceKind::Embedded, None, None));
    runtime.context_mut_for_test().shell = crate::runtime::ShellType::Fish;
    assert!(runtime.plan_sys_init(request()).await.is_err());
    runtime.context_mut_for_test().shell = crate::runtime::ShellType::Bash;
    runtime.host().put_file(
        runtime
            .context()
            .shine_dir
            .join(crate::runtime::SYS_OPERATION_JOURNAL_FILE),
        b"pending".to_vec(),
    );
    assert!(runtime.plan_sys_init(request()).await.is_err());
}

#[tokio::test]
async fn sys_init_conflict_preserves_active_and_reports_partial_publication() {
    let runtime = runtime(snapshot(
        PresetSourceKind::Embedded,
        Some("new\n"),
        Some("post\n"),
    ));
    let dir = runtime.context().home_dir.join(".shine/profile");
    runtime
        .host()
        .put_file_with_mode(dir.join("ubuntu-sys.pre.sh"), b"user\n".to_vec(), 0o100600);
    runtime
        .host()
        .put_file(dir.join("ubuntu-sys.pre.base.sh"), b"old\n".to_vec());
    let outcome = apply(&runtime).await;
    assert_eq!(outcome.status, crate::runtime::SysItemStatus::NeedsAction);
    assert_eq!(
        runtime
            .host()
            .read(&dir.join("ubuntu-sys.pre.sh"))
            .await
            .unwrap(),
        b"user\n"
    );
    assert_eq!(
        runtime
            .host()
            .read(&dir.join("ubuntu-sys.post.sh"))
            .await
            .unwrap(),
        b"post\n"
    );
    for suffix in ["new", "merge"] {
        assert_eq!(
            runtime
                .host()
                .metadata(&dir.join(format!("ubuntu-sys.pre.{suffix}.sh")))
                .await
                .unwrap()
                .unix_mode,
            Some(0o100600)
        );
    }
    assert!(
        runtime
            .host()
            .metadata(&dir.join("ubuntu-sys.pre.new.sh"))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn sys_init_preserves_enabled_items_and_requires_each_external_integration() {
    let preset = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "sys/ubuntu/shine.toml",
            br#"version = 2
[[items]]
id = 'tool'
label = 'Tool'
detect = { kind = 'command', command = 'never-detect-me' }
install = { kind = 'package', provider = 'apt', package = 'never-install-me' }
[[items.shell]]
shells = ['bash', 'zsh']
phase = 'post'
fragment = 'profile/tool.sh'
"#
            .to_vec(),
        )
        .file(
            "sys/ubuntu/profile/base.post.sh",
            b"export BASE=1\n".to_vec(),
        )
        .overlay_file("sys/ubuntu/profile/tool.sh", b"export TOOL=1\n".to_vec())
        .build();
    let mut runtime = runtime(preset);
    let manifest = enabled_tool_manifest("ubuntu", "original");
    let manifest_path = runtime.context().shine_dir.join("sys-manifest.toml");
    runtime.host().put_file(&manifest_path, manifest.clone());
    assert!(
        runtime
            .external_code_requirements("sys/profile/ubuntu")
            .await
            .unwrap()
            .requirements
            .is_empty()
    );
    assert!(!runtime.plan_sys_init(request()).await.unwrap().is_ready());
    let requirements = runtime
        .external_code_requirements("sys/tool")
        .await
        .unwrap()
        .requirements;
    runtime.context_mut_for_test().trust_grants = requirements
        .iter()
        .map(crate::trust::TrustGrantV1::for_reviewed_requirement)
        .collect();
    apply(&runtime).await;
    assert_eq!(runtime.host().read(&manifest_path).await.unwrap(), manifest);
    let post = runtime
        .host()
        .read(
            &runtime
                .context()
                .home_dir
                .join(".shine/profile/ubuntu-sys.post.sh"),
        )
        .await
        .unwrap();
    let post = String::from_utf8(post).unwrap();
    assert!(post.find("BASE=1").unwrap() < post.find("TOOL=1").unwrap());
    assert!(
        !runtime
            .host()
            .operations()
            .iter()
            .any(|op| matches!(op, HostOperation::Run { .. }))
    );
}

#[tokio::test]
async fn sys_init_frontend_supports_one_time_base_confirmation_and_stale_source_rejection() {
    use crate::frontend::{ExecutionOptions, FrontendService, ReviewRequest};
    let runtime = runtime(snapshot(
        PresetSourceKind::Overlay,
        Some("export PRE=1\n"),
        None,
    ));
    let host = runtime.host().clone();
    let context = runtime.context().clone();
    let mut trusted = FrontendService::new(runtime).into_trusted();
    let reviewed = trusted
        .review_with_code_confirmation(ReviewRequest::SysInit(request()))
        .await
        .unwrap();
    assert!(reviewed.report().plan.is_ready());
    let approved = reviewed.approve_after_human_confirmation().unwrap();
    trusted
        .apply(
            approved,
            ExecutionOptions::default(),
            &mut crate::runtime::NullObserver,
            &mut Interaction,
            &mut Vec::new(),
        )
        .await
        .unwrap();
    assert!(
        host.metadata(&context.shine_dir.join("trust.toml"))
            .await
            .is_err()
    );

    let before = CoreRuntime::new(
        host.clone(),
        context.clone(),
        snapshot(PresetSourceKind::Embedded, Some("before\n"), None),
    );
    let approval = approve_init(&before).await;
    let after = CoreRuntime::new(
        host.clone(),
        context,
        snapshot(PresetSourceKind::Embedded, Some("after\n"), None),
    );
    let since = host.operations().len();
    assert!(
        after
            .init_sys_profile_approved(request(), &approval)
            .await
            .is_err()
    );
    assert!(
        !host.operations()[since..]
            .iter()
            .any(|op| matches!(op, HostOperation::Write(_)))
    );
}

#[cfg(unix)]
#[tokio::test]
async fn sys_init_native_bash_and_zsh_load_complete_profiles_in_order_every_time() {
    for (shell, executable, rc) in [
        (crate::runtime::ShellType::Bash, "/bin/bash", ".bashrc"),
        (crate::runtime::ShellType::Zsh, "/bin/zsh", ".zshrc"),
    ] {
        if !Path::new(executable).exists() {
            continue;
        }
        let home =
            std::env::temp_dir().join(format!("shine-sys-init-native-{}", uuid::Uuid::new_v4()));
        let mut context = RuntimeContext::isolated(
            home.clone(),
            home.join(".state"),
            home.join("presets"),
            home.join("bin"),
            RuntimePlatform::Linux,
        );
        context.shell = shell;
        let runtime = CoreRuntime::new(
            InMemoryHost::new(),
            context,
            snapshot(
                PresetSourceKind::Embedded,
                Some("TRACE=\"${TRACE}pre>\"\ninit_function() { printf function; }\n"),
                Some("TRACE=\"${TRACE}post>\"\n"),
            ),
        );
        runtime
            .host()
            .put_file(home.join(rc), b"TRACE=\"${TRACE}user>\"\n".to_vec());
        apply(&runtime).await;
        std::fs::create_dir_all(home.join(".shine/profile")).unwrap();
        for relative in [
            rc,
            ".shine/profile/ubuntu-sys.pre.sh",
            ".shine/profile/ubuntu-sys.post.sh",
        ] {
            let bytes = runtime.host().read(&home.join(relative)).await.unwrap();
            std::fs::write(home.join(relative), bytes).unwrap();
        }
        let script =
            format!(". \"$HOME/{rc}\"; . \"$HOME/{rc}\"; printf '%s' \"$TRACE\"; init_function");
        let output = std::process::Command::new(executable)
            .args(["-f", "-c", &script])
            .env_clear()
            .env("HOME", &home)
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap();
        std::fs::remove_dir_all(&home).unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            "pre>user>post>pre>user>post>function"
        );
    }
}

#[tokio::test]
async fn sys_init_supported_platforms_preserve_native_profile_paths() {
    for (platform, os, shell, paths, ext) in [
        (
            RuntimePlatform::Macos,
            "macos",
            crate::runtime::ShellType::Zsh,
            vec![".zshrc"],
            "sh",
        ),
        (
            RuntimePlatform::Linux,
            "arch",
            crate::runtime::ShellType::Zsh,
            vec![".zshrc"],
            "sh",
        ),
        (
            RuntimePlatform::Windows,
            "windows",
            crate::runtime::ShellType::PowerShell,
            vec![
                "Documents/PowerShell/Microsoft.PowerShell_profile.ps1",
                "Documents/WindowsPowerShell/Microsoft.PowerShell_profile.ps1",
            ],
            "ps1",
        ),
    ] {
        let mut runtime = runtime(
            PresetSnapshot::builder(PresetSourceKind::Embedded)
                .file(format!("sys/{os}/shine.toml"), b"version = 2\n".to_vec())
                .file(
                    format!("sys/{os}/profile/base.pre.{ext}"),
                    b"# initialization\n".to_vec(),
                )
                .build(),
        );
        runtime.context_mut_for_test().platform = platform;
        runtime.context_mut_for_test().shell = shell;
        let req = SysInitPlanRequest {
            os_id: os.into(),
            force_profile: false,
        };
        if platform == RuntimePlatform::Windows {
            for path in &paths {
                runtime.host().put_file_with_mode(
                    runtime.context().home_dir.join(path),
                    "\u{feff}# user profile\r\n".as_bytes().to_vec(),
                    0o100600,
                );
            }
        }
        let plan = runtime.plan_sys_init(req.clone()).await.unwrap();
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        runtime
            .init_sys_profile_approved(req, &approval)
            .await
            .unwrap();
        for path in paths {
            if platform == RuntimePlatform::Windows {
                let path = runtime.context().home_dir.join(path);
                let bytes = runtime.host().read(&path).await.unwrap();
                assert!(bytes.starts_with("\u{feff}".as_bytes()));
                assert!(String::from_utf8(bytes).unwrap().contains("# user profile"));
                assert_eq!(
                    runtime.host().metadata(&path).await.unwrap().unix_mode,
                    Some(0o100600)
                );
            }
            assert!(
                runtime
                    .host()
                    .metadata(&runtime.context().home_dir.join(path))
                    .await
                    .is_ok(),
                "{path}"
            );
        }
    }
}

#[tokio::test]
async fn sys_init_pending_journal_review_does_not_request_profile_code_consent() {
    use crate::frontend::{FrontendService, ReviewRequest};
    let (mut runtime, _) = super::sys::interrupted_split_dns_install().await;
    runtime.context_mut_for_test().platform = RuntimePlatform::Macos;
    runtime.context_mut_for_test().shell = crate::runtime::ShellType::Zsh;
    let req = SysInitPlanRequest {
        os_id: "macos".into(),
        force_profile: false,
    };
    let plan = runtime.plan_sys_init(req.clone()).await.unwrap();
    assert!(!plan.is_ready());
    assert!(
        plan.code_boundaries.is_empty(),
        "recovery blockers deliver no code"
    );
    let mut trusted = FrontendService::new(runtime).into_trusted();
    let review = trusted
        .review_with_code_confirmation(ReviewRequest::SysInit(req))
        .await
        .unwrap();
    assert!(review.report().plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .iter()
            .any(|code| code == "sys_recovery_required")
    }));
}

#[tokio::test]
async fn sys_init_arch_confirmation_uses_arch_item_snapshot_not_ubuntu() {
    use crate::frontend::{FrontendService, ReviewRequest};
    let preset = PresetSnapshot::builder(PresetSourceKind::External)
        .file(
            "sys/arch/shine.toml",
            br#"version = 2
[[items]]
id = 'tool'
label = 'Tool'
detect = { kind = 'command', command = 'never-detect' }
install = { kind = 'package', provider = 'pacman', package = 'tool' }
[[items.shell]]
shells = ['bash']
phase = 'post'
fragment = 'profile/tool.sh'
"#
            .to_vec(),
        )
        .file("sys/arch/profile/tool.sh", b"export TOOL=arch\n".to_vec())
        .build();
    let runtime = runtime(preset);
    runtime.host().put_file(
        runtime.context().shine_dir.join("sys-manifest.toml"),
        enabled_tool_manifest("arch", ""),
    );
    // The legacy platform-only query assumes Ubuntu; an explicit operation must not use it.
    assert!(
        runtime
            .external_code_requirements("sys/tool")
            .await
            .is_err()
    );
    let requirements = runtime
        .external_code_requirements_for_os("sys/tool", "arch")
        .await
        .unwrap();
    assert_eq!(requirements.requirements.len(), 1);
    let req = SysInitPlanRequest {
        os_id: "arch".into(),
        force_profile: false,
    };
    assert!(!runtime.plan_sys_init(req.clone()).await.unwrap().is_ready());
    let mut trusted = FrontendService::new(runtime).into_trusted();
    let review = trusted
        .review_with_code_confirmation(ReviewRequest::SysInit(req))
        .await
        .unwrap();
    assert!(review.report().plan.is_ready());
}

#[tokio::test]
async fn sys_init_reports_other_shell_loader_removal_as_an_update() {
    let mut runtime = runtime(snapshot(
        PresetSourceKind::Embedded,
        Some("export BASE=1\n"),
        None,
    ));
    apply(&runtime).await;
    let bash = runtime.context().home_dir.join(".bashrc");
    let old_bash = runtime.host().read(&bash).await.unwrap();
    runtime.context_mut_for_test().shell = crate::runtime::ShellType::Zsh;
    apply(&runtime).await;
    runtime.host().put_file(&bash, old_bash);
    assert_eq!(
        apply(&runtime).await.status,
        crate::runtime::SysItemStatus::Updated
    );
    assert!(
        !String::from_utf8(runtime.host().read(&bash).await.unwrap())
            .unwrap()
            .contains("shine ubuntu sys")
    );
    assert_eq!(
        apply(&runtime).await.status,
        crate::runtime::SysItemStatus::Skipped
    );
}

#[tokio::test]
async fn sys_init_interrupted_publication_retries_without_software_or_recovery_state() {
    for fail_path in [
        ".shine/profile/ubuntu-sys.pre.sh",
        ".shine/profile/ubuntu-sys.pre.base.sh",
        ".shine/profile/ubuntu-sys.post.sh",
        ".shine/profile/ubuntu-sys.post.base.sh",
        ".bashrc",
    ] {
        let runtime = runtime(snapshot(
            PresetSourceKind::Embedded,
            Some("pre\n"),
            Some("post\n"),
        ));
        let approval = approve_init(&runtime).await;
        runtime
            .host()
            .fail_write_after(runtime.context().home_dir.join(fail_path), 0);
        let error = runtime
            .init_sys_profile_approved(request(), &approval)
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("some profile files may already have changed")
        );
        apply(&runtime).await;
        for (phase, expected) in [("pre", b"pre\n".as_slice()), ("post", b"post\n".as_slice())] {
            let path = runtime
                .context()
                .home_dir
                .join(format!(".shine/profile/ubuntu-sys.{phase}.sh"));
            assert_eq!(runtime.host().read(&path).await.unwrap(), expected);
        }
        for file in [
            "sys-manifest.toml",
            crate::runtime::SYS_OPERATION_JOURNAL_FILE,
        ] {
            assert!(
                runtime
                    .host()
                    .metadata(&runtime.context().shine_dir.join(file))
                    .await
                    .is_err()
            );
        }
        assert_eq!(
            apply(&runtime).await.status,
            crate::runtime::SysItemStatus::Skipped
        );
    }
}

#[tokio::test]
async fn sys_init_clean_merge_force_backup_and_crlf_preserve_user_configuration() {
    let runtime = runtime(snapshot(
        PresetSourceKind::Embedded,
        Some("new\nkeep\n"),
        None,
    ));
    let dir = runtime.context().home_dir.join(".shine/profile");
    let active = dir.join("ubuntu-sys.pre.sh");
    runtime
        .host()
        .put_file_with_mode(&active, b"old\r\nkeep\r\nuser\r\n".to_vec(), 0o100600);
    runtime
        .host()
        .put_file(dir.join("ubuntu-sys.pre.base.sh"), b"old\nkeep\n".to_vec());
    apply(&runtime).await;
    assert_eq!(
        runtime.host().read(&active).await.unwrap(),
        b"new\nkeep\nuser\n"
    );
    assert_eq!(
        runtime.host().metadata(&active).await.unwrap().unix_mode,
        Some(0o100600)
    );
    let forced = SysInitPlanRequest {
        force_profile: true,
        ..request()
    };
    let plan = runtime.plan_sys_init(forced.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .init_sys_profile_approved(forced, &approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&active).await.unwrap(), b"new\nkeep\n");
    let backups = runtime
        .host()
        .read_dir(&dir)
        .await
        .unwrap()
        .into_iter()
        .filter(|p| p.to_string_lossy().contains(".bak."))
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    assert_eq!(
        runtime
            .host()
            .metadata(&backups[0])
            .await
            .unwrap()
            .unix_mode,
        Some(0o100600)
    );
    assert_eq!(
        runtime.host().read(&backups[0]).await.unwrap(),
        b"new\nkeep\nuser\n"
    );
    runtime
        .host()
        .put_file_with_mode(&active, b"new\r\nkeep\r\n".to_vec(), 0o100600);
    assert_eq!(
        apply(&runtime).await.status,
        crate::runtime::SysItemStatus::Skipped
    );
    assert_eq!(
        runtime.host().read(&active).await.unwrap(),
        b"new\r\nkeep\r\n"
    );
}

#[tokio::test]
async fn sys_init_rejects_invalid_manifests_and_post_fragment_before_publication() {
    for bytes in [
        None,
        Some(b"invalid".to_vec()),
        Some(b"version = 999\n".to_vec()),
    ] {
        let mut builder = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file("sys/ubuntu/profile/base.pre.sh", b"pre\n".to_vec());
        if let Some(bytes) = bytes {
            builder = builder.file("sys/ubuntu/shine.toml", bytes);
        }
        let runtime = runtime(builder.build());
        assert!(runtime.plan_sys_init(request()).await.is_err());
        assert!(
            runtime
                .host()
                .operations()
                .iter()
                .all(|op| matches!(op, HostOperation::Read(_)))
        );
    }
    let preset = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "sys/ubuntu/shine.toml",
            br#"version = 2
[[items]]
id = 'tool'
label = 'Tool'
detect = { kind = 'path', path = '$HOME/.present' }
install = { kind = 'package', provider = 'apt', package = 'tool' }
[[items.shell]]
shells = ['bash']
phase = 'post'
fragment = 'profile/tool.sh'
"#
            .to_vec(),
        )
        .file("sys/ubuntu/profile/base.pre.sh", b"pre\n".to_vec())
        .file("sys/ubuntu/profile/tool.sh", vec![0xff])
        .build();
    let runtime = runtime(preset);
    runtime.host().put_file(
        runtime.context().shine_dir.join("sys-manifest.toml"),
        enabled_tool_manifest("ubuntu", ""),
    );
    let approval = approve_init(&runtime).await;
    let error = runtime
        .init_sys_profile_approved(request(), &approval)
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("not UTF-8"));
    assert!(
        !runtime
            .host()
            .operations()
            .iter()
            .any(|op| matches!(op, HostOperation::Write(_) | HostOperation::Run { .. }))
    );
}

#[tokio::test]
async fn sys_init_and_bootstrap_reuse_composition_without_changing_init_receipts() {
    let preset = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "sys/ubuntu/shine.toml",
            br#"version = 2
[[items]]
id = 'tool'
label = 'Tool'
detect = { kind = 'path', path = '$HOME/.present' }
install = { kind = 'package', provider = 'apt', package = 'tool' }
[[items.shell]]
shells = ['bash']
phase = 'post'
path = '$HOME/tool/bin'
"#
            .to_vec(),
        )
        .file(
            "sys/ubuntu/profile/base.pre.sh",
            b"export BASE=1\n".to_vec(),
        )
        .build();
    let runtime = runtime(preset);
    runtime
        .host()
        .put_file(runtime.context().home_dir.join(".present"), Vec::new());
    apply(&runtime).await;
    let req = SysBootstrapPlanRequest {
        os_id: "ubuntu".into(),
        item_ids: vec!["tool".into()],
        sys_shell: "bash".into(),
        force_profile: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_sys_bootstrap(req.clone()).await.unwrap();
    runtime
        .run_sys_bootstrap_approved(
            req,
            &PlanApprovalV1::for_reviewed_plan(&plan).unwrap(),
            &mut Interaction,
            &mut crate::runtime::NullObserver,
        )
        .await
        .unwrap();
    let manifest_path = runtime.context().shine_dir.join("sys-manifest.toml");
    let manifest = runtime.host().read(&manifest_path).await.unwrap();
    assert_eq!(
        apply(&runtime).await.status,
        crate::runtime::SysItemStatus::Skipped
    );
    assert_eq!(runtime.host().read(&manifest_path).await.unwrap(), manifest);
}

#[tokio::test]
async fn sys_init_conflict_artifacts_do_not_follow_links_or_change_target_permissions() {
    let runtime = runtime(snapshot(PresetSourceKind::Embedded, Some("new\n"), None));
    let dir = runtime.context().home_dir.join(".shine/profile");
    runtime.host().put_file_with_mode(
        dir.join("ubuntu-sys.pre.sh"),
        b"private user edit\n".to_vec(),
        0o100600,
    );
    runtime
        .host()
        .put_file(dir.join("ubuntu-sys.pre.base.sh"), b"old\n".to_vec());
    let victim = runtime.context().home_dir.join("unrelated");
    runtime
        .host()
        .put_file_with_mode(&victim, b"unrelated\n".to_vec(), 0o100640);
    runtime
        .host()
        .symlink(&victim, &dir.join("ubuntu-sys.pre.new.sh"))
        .await
        .unwrap();
    let approval = approve_init(&runtime).await;
    let error = runtime
        .init_sys_profile_approved(request(), &approval)
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("not a regular file"));
    assert_eq!(runtime.host().read(&victim).await.unwrap(), b"unrelated\n");
    assert_eq!(
        runtime.host().metadata(&victim).await.unwrap().unix_mode,
        Some(0o100640)
    );
}

#[tokio::test]
async fn sys_init_force_rejects_symlink_even_when_content_is_eol_equivalent() {
    for link_name in ["ubuntu-sys.pre.sh", "ubuntu-sys.pre.base.sh"] {
        let runtime = runtime(snapshot(PresetSourceKind::Embedded, Some("same\n"), None));
        let dir = runtime.context().home_dir.join(".shine/profile");
        let original = runtime.context().home_dir.join("private-original");
        runtime
            .host()
            .put_file_with_mode(&original, b"same\r\n".to_vec(), 0o100600);
        runtime.host().put_file(
            dir.join("ubuntu-sys.pre.sh"),
            b"different user content\n".to_vec(),
        );
        if link_name.ends_with("pre.sh") {
            runtime
                .host()
                .remove_file(&dir.join(link_name))
                .await
                .unwrap();
        }
        runtime
            .host()
            .symlink(&original, &dir.join(link_name))
            .await
            .unwrap();
        let req = SysInitPlanRequest {
            force_profile: true,
            ..request()
        };
        let plan = runtime.plan_sys_init(req.clone()).await.unwrap();
        let since = runtime.host().operations().len();
        let error = runtime
            .init_sys_profile_approved(req, &PlanApprovalV1::for_reviewed_plan(&plan).unwrap())
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("not a regular file"));
        assert!(
            !runtime.host().operations()[since..]
                .iter()
                .any(|op| matches!(op, HostOperation::Write(_)))
        );
        assert_eq!(runtime.host().read(&original).await.unwrap(), b"same\r\n");
        assert_eq!(
            runtime
                .host()
                .metadata(&dir.join(link_name))
                .await
                .unwrap()
                .kind,
            FileKind::Symlink
        );
    }
}

#[tokio::test]
async fn sys_init_repeated_conflicts_replace_artifacts_and_keep_active_private() {
    let runtime = runtime(snapshot(PresetSourceKind::Embedded, Some("new\n"), None));
    let dir = runtime.context().home_dir.join(".shine/profile");
    let active = dir.join("ubuntu-sys.pre.sh");
    runtime
        .host()
        .put_file_with_mode(&active, b"user\n".to_vec(), 0o100600);
    runtime
        .host()
        .put_file(dir.join("ubuntu-sys.pre.base.sh"), b"old\n".to_vec());
    assert_eq!(
        apply(&runtime).await.status,
        crate::runtime::SysItemStatus::NeedsAction
    );
    runtime
        .host()
        .put_file_with_mode(&active, b"second user edit\n".to_vec(), 0o100600);
    assert_eq!(
        apply(&runtime).await.status,
        crate::runtime::SysItemStatus::NeedsAction
    );
    let merge_path = dir.join("ubuntu-sys.pre.merge.sh");
    assert!(
        String::from_utf8(runtime.host().read(&merge_path).await.unwrap())
            .unwrap()
            .contains("second user edit")
    );
    assert_eq!(
        runtime
            .host()
            .metadata(&merge_path)
            .await
            .unwrap()
            .unix_mode,
        Some(0o100600)
    );
    assert_eq!(
        runtime.host().read(&active).await.unwrap(),
        b"second user edit\n"
    );
}
