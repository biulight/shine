use super::*;
use crate::lifecycle_plan::tests::bootstrap_display_plan;
use shine_core::plan::{CodeBoundaryV2, EnvironmentSensitivityV1};

#[cfg(unix)]
#[tokio::test]
async fn detected_ubuntu_items_skip_installers_but_keep_profile_and_trust_review() {
    use shine_core::plan::PlanApprovalV1;
    use shine_core::runtime::{
        CoreRuntime, InMemoryHost, PlanningInputVersions, RuntimeContext, RuntimePlatform,
        ShellType, SysBootstrapPlanRequest, capture_embedded_preset_snapshot,
    };
    for docker_present in [false, true] {
        let home = std::env::temp_dir().join("shine-detected-bootstrap");
        let mut context = RuntimeContext::isolated(
            home.clone(),
            home.join(".shine"),
            home.join(".shine/presets"),
            home.join(".shine/bin"),
            RuntimePlatform::Linux,
        );
        context.shell = ShellType::Zsh;
        context.shell_config_paths = vec![home.join(".zshrc")];
        let host = InMemoryHost::new();
        for command in [
            "rustup", "nvim", "atuin", "yazi", "starship", "zoxide", "fzf", "bat", "eza",
        ] {
            host.put_file_with_mode(
                home.join(".local/bin").join(command),
                b"present".to_vec(),
                0o100755,
            );
        }
        for path in [
            ".config/nvim",
            ".local/share/zsh-vi-mode/zsh-vi-mode.plugin.zsh",
        ] {
            host.put_file(home.join(path), b"present".to_vec());
        }
        if docker_present {
            let status = ["docker-ce", "docker-ce-cli", "containerd.io", "docker-buildx-plugin", "docker-compose-plugin"]
                .map(|package| format!("Package: {package}\nStatus: install ok installed\nArchitecture: amd64\nVersion: 1\n\n")).join("");
            host.put_file("/var/lib/dpkg/status", status.into_bytes());
        }
        let runtime = CoreRuntime::new(
            host,
            context,
            capture_embedded_preset_snapshot(crate::core_runtime::embedded_preset_files()),
        );
        let loaded = runtime.load_sys_preset("ubuntu").await.unwrap();
        let items = loaded.manifest.profiles["recommended"].items.clone();
        let plan = runtime
            .plan_sys_bootstrap(SysBootstrapPlanRequest {
                os_id: "ubuntu".into(),
                item_ids: items.clone(),
                sys_shell: "zsh".into(),
                force_profile: false,
                input_versions: PlanningInputVersions::default(),
            })
            .await
            .unwrap();
        assert!(plan.is_ready());
        assert_eq!(
            plan.steps
                .iter()
                .filter(|step| step.resource.as_deref() == Some("bootstrap")
                    && step.action == PlanActionV1::Update)
                .count(),
            if docker_present { 12 } else { 11 }
        );
        let before = serde_json::to_vec(&plan).unwrap();
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        for verbose in [false, true] {
            let output = render_bootstrap_plan_lines(&plan, "config", verbose)
                .unwrap()
                .join("\n");
            assert!(
                !output.contains("Bootstrap code executes during this operation · all 12"),
                "{output}"
            );
            assert!(
                output.contains("installation skipped; trust requirements retained")
                    || output.contains("Installation skipped; trust requirements retained"),
                "{output}"
            );
            assert!(output.contains("~/.bashrc") && output.contains("~/.zshrc"));
            assert!(output.contains("Recovery is unsupported"));
            if !verbose {
                assert!(output.contains("Installation"));
                assert!(!output.contains("(update)"));
                for (index, item) in items.iter().enumerate() {
                    let row = output
                        .lines()
                        .find(|line| line.starts_with(&format!("    {:02}  ", index + 1)))
                        .unwrap();
                    let fields = row.split_whitespace().collect::<Vec<_>>();
                    assert_eq!(fields[1], item);
                    assert_eq!(
                        fields[2],
                        if item == "docker" && !docker_present {
                            "run"
                        } else {
                            "skip"
                        }
                    );
                    assert_eq!(
                        fields[3],
                        [
                            "rustup",
                            "nvim",
                            "—",
                            "atuin",
                            "yazi",
                            "starship",
                            "zoxide",
                            "—",
                            "fzf",
                            "bat",
                            "eza",
                            if docker_present { "—" } else { "bash" }
                        ][index]
                    );
                }
                assert!(output.contains("Detected items skip installation; shell integration and state may still change."));
                assert!(output.contains(
                    "Profile code is delivered for later execution/source · all 12 bootstrap items"
                ));
                if docker_present {
                    assert!(!output.contains("Bootstrap code executes during this operation"));
                } else {
                    assert!(
                        output.contains("Bootstrap code executes during this operation · docker")
                    );
                    assert!(output.contains("Administrator access required: docker"));
                    assert!(output.contains("preset:install/docker.sh"));
                }
            }
            assert_eq!(serde_json::to_vec(&plan).unwrap(), before);
            assert_eq!(PlanApprovalV1::for_reviewed_plan(&plan).unwrap(), approval);
        }
        assert!(
            runtime
                .host()
                .operations()
                .iter()
                .all(|op| matches!(op, shine_core::runtime::HostOperation::Read(_)))
        );
    }
}

#[tokio::test]
async fn detected_external_script_still_requires_trust_and_explicit_code_consent() {
    use shine_core::frontend::{FrontendService, ReviewRequest};
    use shine_core::runtime::{
        CoreRuntime, InMemoryHost, PlanningInputVersions, PresetSnapshot, PresetSourceKind,
        RuntimeContext, RuntimePlatform, SysBootstrapPlanRequest,
    };
    let home = std::env::temp_dir().join("shine-detected-external");
    let context = RuntimeContext::isolated(
        home.clone(),
        home.join(".shine"),
        home.join(".shine/presets"),
        home.join(".shine/bin"),
        RuntimePlatform::Linux,
    );
    let host = InMemoryHost::new();
    host.put_file(home.join(".present"), b"present".to_vec());
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .file("sys/ubuntu/shine.toml", b"version=2\n[[items]]\nid='tool'\nlabel='Tool'\npermissions={schema_version=1}\ndetect={kind='path',path='$HOME/.present'}\ninstall={kind='script',path='install.sh'}\n".to_vec())
        .file("sys/ubuntu/install.sh", b"exit 99\n".to_vec()).build();
    let mut frontend =
        FrontendService::new(CoreRuntime::new(host.clone(), context, snapshot)).into_trusted();
    let request = ReviewRequest::SysBootstrap(SysBootstrapPlanRequest {
        os_id: "ubuntu".into(),
        item_ids: vec!["tool".into()],
        sys_shell: "bash".into(),
        force_profile: false,
        input_versions: PlanningInputVersions::default(),
    });
    let blocked = frontend.review(request.clone()).await.unwrap();
    assert!(!blocked.report().plan.is_ready());
    let output = render_bootstrap_plan_lines(&blocked.report().plan, "config", false)
        .unwrap()
        .join("\n");
    assert!(output.contains("External preset code is not trusted"));
    assert!(output.contains("installation blocked; trust requirements retained"));
    assert!(blocked.approve_for_automation().is_err());
    let consent = frontend
        .review_with_code_confirmation(request.clone())
        .await
        .unwrap();
    let plan = &consent.report().plan;
    assert!(plan.is_ready());
    assert_eq!(plan.steps[0].action, PlanActionV1::Update);
    let before = serde_json::to_vec(plan).unwrap();
    for verbose in [false, true] {
        let output = render_bootstrap_plan_lines(plan, "config", verbose)
            .unwrap()
            .join("\n");
        assert!(
            output.contains("installation skipped; trust requirements retained"),
            "{output}"
        );
        assert!(output.contains("requires your confirmation for this operation only"));
        assert!(!output.contains("executes during this operation"));
        assert_eq!(serde_json::to_vec(plan).unwrap(), before);
    }
    assert!(consent.approve_for_automation().is_err());
    let human_review = frontend
        .review_with_code_confirmation(request)
        .await
        .unwrap();
    let approved = human_review.approve_after_human_confirmation().unwrap();
    let frontend = frontend.with_approved_code(&approved);
    frontend.validate_approved(&approved).await.unwrap();
    // Removing the detection marker after review must still invalidate the
    // approval, even though the UI had described installation as skipped.
    use shine_core::runtime::FileSystemHost;
    host.remove_file(&home.join(".present")).await.unwrap();
    assert!(frontend.validate_approved(&approved).await.is_err());
    assert!(
        !host
            .operations()
            .iter()
            .any(|op| matches!(op, shine_core::runtime::HostOperation::Run { .. }))
    );
}

#[test]
fn detected_installation_wording_requires_an_unambiguous_bootstrap_step() {
    let mut plan = bootstrap_display_plan();
    plan.steps[0].action = PlanActionV1::Update;
    plan.code_boundaries.push(boundary(
        "sys/rust",
        CodeEntryKindV2::SysBootstrapScript,
        CodeTimingV2::ExecuteNow,
    ));
    for verbose in [false, true] {
        assert!(
            render_code_boundaries(&plan, "  ", verbose)
                .join("\n")
                .contains("installation skipped")
        );
        // Full-review fallback must still distinguish retained trust from execution.
        let mut fallback = plan.clone();
        fallback.permission_scopes.clear();
        assert!(
            render_bootstrap_plan_lines(&fallback, "config", verbose)
                .unwrap()
                .join("\n")
                .contains("installation skipped")
        );
    }
    for exception in [
        "operation",
        "resource",
        "duplicate",
        "kind",
        "timing",
        "role",
    ] {
        let mut changed = plan.clone();
        match exception {
            "operation" => changed.operation = shine_core::plan::PlanOperationV1::Upgrade,
            "resource" => changed.steps[0].resource = Some("other".into()),
            "duplicate" => changed.steps.push(changed.steps[0].clone()),
            "kind" => changed.code_boundaries[0].entry_kind = CodeEntryKindV2::AppHook,
            "timing" => changed.code_boundaries[0].timing = CodeTimingV2::DeliverForLater,
            "role" => {
                changed.code_boundaries[0].target_role = CodeTargetRoleV2::SharedResourceAffected
            }
            _ => unreachable!(),
        }
        assert!(
            !render_code_boundaries(&changed, "  ", false)
                .join("\n")
                .contains("installation skipped"),
            "{exception}"
        );
    }
}

#[test]
fn detected_items_retain_every_trust_state_in_compact_verbose_and_fallback_review() {
    use shine_core::plan::PlanApprovalV1;
    for (trust, label) in [
        (CodeTrustStateV2::Distribution, "distribution trust"),
        (CodeTrustStateV2::SnapshotTrusted, "snapshot trusted"),
        (
            CodeTrustStateV2::DevelopmentTrusted,
            "development trusted; source may change",
        ),
        (
            CodeTrustStateV2::OperationConfirmation,
            "requires your confirmation for this operation only",
        ),
        (CodeTrustStateV2::MissingOrStale, "trust missing or stale"),
    ] {
        let mut plan = bootstrap_display_plan();
        plan.steps[0].action = PlanActionV1::Update;
        for target in ["sys/rust", "sys/fzf"] {
            plan.code_boundaries.push(boundary(
                target,
                CodeEntryKindV2::SysBootstrapScript,
                CodeTimingV2::ExecuteNow,
            ));
            plan.code_boundaries.push(boundary(
                target,
                CodeEntryKindV2::SysProfileCode,
                CodeTimingV2::DeliverForLater,
            ));
        }
        let install = &mut plan.code_boundaries[0];
        install.trust = trust;
        if trust != CodeTrustStateV2::Distribution {
            install.source = CodeSourceV2::ExternalOrOverlay;
        }
        if trust == CodeTrustStateV2::MissingOrStale {
            plan.steps[0].action = PlanActionV1::Blocked;
            plan.steps[0]
                .diagnostic_codes
                .push("sys_external_code_not_allowed".into());
        }
        for fallback in [false, true] {
            let mut reviewed = plan.clone();
            if fallback {
                reviewed.permission_scopes.clear();
            }
            let before = serde_json::to_vec(&reviewed).unwrap();
            let approval = PlanApprovalV1::for_reviewed_plan(&reviewed);
            for verbose in [false, true] {
                let output = render_bootstrap_plan_lines(&reviewed, "config", verbose)
                    .unwrap()
                    .join("\n");
                assert!(output.contains(label), "{trust:?}: {output}");
                assert!(
                    output
                        .to_lowercase()
                        .contains("installation skipped; trust requirements retained")
                );
                assert!(output.contains("delivered for later execution/source"));
                assert!(!output.contains("executes during this operation"));
                assert!(!output.contains("(update)"));
                assert!(output.to_lowercase().contains("administrator"));
                assert!(output.to_lowercase().contains("network"));
                assert!(output.contains("~/.bashrc"));
                if trust == CodeTrustStateV2::MissingOrStale {
                    assert!(output.contains("installation blocked; trust requirements retained"));
                    assert!(approval.is_err());
                } else {
                    approval.as_ref().unwrap().validate(&reviewed).unwrap();
                }
                assert_eq!(serde_json::to_vec(&reviewed).unwrap(), before);
                assert_eq!(PlanApprovalV1::for_reviewed_plan(&reviewed), approval);
            }
        }
    }
}

fn add_permission(plan: &mut PlanV1, scope: usize, permission: PermissionV1) {
    plan.permission_scopes[scope]
        .permissions
        .required
        .insert(permission.clone());
    plan.permissions.required.insert(permission);
}

#[test]
fn numbered_bootstrap_review_has_the_complete_compact_layout() {
    let plan = bootstrap_display_plan();
    let before = serde_json::to_value(&plan).unwrap();
    let fingerprint = plan.fingerprint().unwrap();
    assert_eq!(
        render_bootstrap_plan_lines(&plan, "config", false)
            .unwrap()
            .join("\n"),
        r"Security Plan · sys-bootstrap
  2 bootstrap items · 1 profile configuration

  Attention
    ! Administrator access required: fzf
    ! Unrestricted network access required: rust
    ! Profile configuration: Recovery is unsupported for this profile step

  Bootstrap items
    #   Item  Installation  Commands  Execute
    01  rust  run           curl      —
    02  fzf   skip          apt-get   —
    Detected items skip installation; shell integration and state may still change.

  Profile configuration
    ~  bash
      filesystem write: ~/.bashrc

  Shared changes
    filesystem write: Shine state/sys-manifest.toml
  Use --verbose for all paths, identities and diagnostic codes."
    );
    assert_eq!(serde_json::to_value(&plan).unwrap(), before);
    assert_eq!(plan.fingerprint().unwrap(), fingerprint);
}

#[test]
fn compact_bootstrap_retains_extra_permissions_and_attributed_diagnostics() {
    let mut plan = bootstrap_display_plan();
    for permission in [
        PermissionV1::Filesystem {
            access: FilesystemAccessV1::Execute,
            path: "preset:install/first.sh".into(),
        },
        PermissionV1::Filesystem {
            access: FilesystemAccessV1::Execute,
            path: "preset:install/second.sh".into(),
        },
        PermissionV1::Filesystem {
            access: FilesystemAccessV1::Write,
            path: "home:important\nFORGED".into(),
        },
        PermissionV1::Filesystem {
            access: FilesystemAccessV1::Read,
            path: "home:read-target".into(),
        },
        PermissionV1::Filesystem {
            access: FilesystemAccessV1::Remove,
            path: "home:remove-target".into(),
        },
        PermissionV1::Network {
            scope: NetworkScopeV1::Host("example.test".into()),
        },
        PermissionV1::Environment {
            name: "TOKEN".into(),
            sensitivity: EnvironmentSensitivityV1::Secret,
        },
        PermissionV1::Environment {
            name: "TOKEN".into(),
            sensitivity: EnvironmentSensitivityV1::Plain,
        },
        PermissionV1::System {
            capability: "future-capability".into(),
            resource: Some("resource".into()),
        },
    ] {
        add_permission(&mut plan, 0, permission);
    }
    add_permission(&mut plan, 2, PermissionV1::Administrator);
    add_permission(&mut plan, 3, PermissionV1::Administrator);
    add_permission(
        &mut plan,
        3,
        PermissionV1::Command {
            program: "shared-inspect".into(),
        },
    );
    add_permission(
        &mut plan,
        3,
        PermissionV1::Filesystem {
            access: FilesystemAccessV1::Execute,
            path: "preset:shared.sh".into(),
        },
    );
    for step in &mut plan.steps[..2] {
        step.diagnostic_codes
            .push("sys_dpkg_conflicts_may_be_removed".into());
        step.diagnostic_codes.push("future_notice\nFORGED".into());
    }
    let before = serde_json::to_value(&plan).unwrap();
    let rendered = render_bootstrap_plan_lines(&plan, "config", false)
        .unwrap()
        .join("\n");
    for expected in [
        "Administrator access required: fzf, Profile configuration, Shared changes",
        "preset:install/first.sh, preset:install/second.sh",
        "    rust:\n",
        "filesystem write: ~/important\\nFORGED",
        "filesystem read: ~/read-target",
        "filesystem remove: ~/remove-target",
        "network: host example.test",
        "environment secret: TOKEN",
        "environment plain: TOKEN",
        "system future-capability: resource",
        "rust: future_notice\\nFORGED",
        "fzf: future_notice\\nFORGED",
    ] {
        assert!(
            rendered.contains(expected),
            "missing {expected}: {rendered}"
        );
    }
    let table = rendered.find("\n  Bootstrap items").unwrap();
    assert_eq!(
        rendered[..table]
            .matches("package rollback is unsupported")
            .count(),
        2
    );
    assert!(!rendered[table..].contains("package rollback is unsupported"));
    let shared = rendered.find("\n  Shared changes\n").unwrap();
    assert!(rendered[shared..].contains("command: shared-inspect"));
    assert!(rendered[shared..].contains("filesystem execute: preset:shared.sh"));
    assert_eq!(serde_json::to_value(&plan).unwrap(), before);
}

fn boundary(target: &str, kind: CodeEntryKindV2, timing: CodeTimingV2) -> CodeBoundaryV2 {
    CodeBoundaryV2 {
        target: target.into(),
        entry_kind: kind,
        timing,
        source: CodeSourceV2::ShineDistribution,
        trust: CodeTrustStateV2::Distribution,
        unisolated: true,
        target_role: CodeTargetRoleV2::Selected,
        shared_resource: Some("sys/ubuntu/profile".into()),
    }
}

#[test]
fn bootstrap_code_groups_preserve_timing_membership_and_every_trust_exception() {
    let mut plan = bootstrap_display_plan();
    plan.steps[1].action = PlanActionV1::Execute;
    for target in ["sys/rust", "sys/fzf"] {
        plan.code_boundaries.push(boundary(
            target,
            CodeEntryKindV2::SysBootstrapScript,
            CodeTimingV2::ExecuteNow,
        ));
        plan.code_boundaries.push(boundary(
            target,
            CodeEntryKindV2::SysProfileCode,
            CodeTimingV2::DeliverForLater,
        ));
    }
    let before = serde_json::to_value(&plan).unwrap();
    let grouped = bootstrap_code_boundaries(&plan).join("\n");
    assert_eq!(
        grouped,
        "  Code execution and trust\n    built-in · distribution trust\n      Bootstrap code executes during this operation · all 2 bootstrap items\n      Profile code is delivered for later execution/source · all 2 bootstrap items"
    );
    assert_eq!(serde_json::to_value(&plan).unwrap(), before);

    for trust in [
        CodeTrustStateV2::SnapshotTrusted,
        CodeTrustStateV2::OperationConfirmation,
        CodeTrustStateV2::DevelopmentTrusted,
        CodeTrustStateV2::MissingOrStale,
    ] {
        let mut external = plan.clone();
        external.code_boundaries[0].source = CodeSourceV2::ExternalOrOverlay;
        external.code_boundaries[0].trust = trust;
        let output = bootstrap_code_boundaries(&external).join("\n");
        let individual =
            render_code_boundary_entries(&external, &external.code_boundaries[..1], "  ", false);
        assert!(output.contains(&individual[1]), "{output}");
        assert!(output.contains("Bootstrap code executes during this operation · fzf"));
        assert_eq!(output.matches("all 2 bootstrap items").count(), 1);
    }
    for exception in ["affected", "timing", "kind"] {
        let mut changed = plan.clone();
        let entry = &mut changed.code_boundaries[0];
        match exception {
            "affected" => entry.target_role = CodeTargetRoleV2::SharedResourceAffected,
            "timing" => entry.timing = CodeTimingV2::DeliverForLater,
            "kind" => entry.entry_kind = CodeEntryKindV2::AppHook,
            _ => unreachable!(),
        }
        let individual =
            render_code_boundary_entries(&changed, &changed.code_boundaries[..1], "  ", false);
        assert!(bootstrap_code_boundaries(&changed).contains(&individual[1]));
    }
    plan.code_boundaries.push(boundary(
        "sys/previously-enabled",
        CodeEntryKindV2::SysProfileCode,
        CodeTimingV2::DeliverForLater,
    ));
    let output = bootstrap_code_boundaries(&plan).join("\n");
    assert!(output.contains(
        "Profile code is delivered for later execution/source · fzf, previously-enabled, rust"
    ));
    assert_eq!(output.matches("all 2 bootstrap items").count(), 1);
}

#[test]
fn bootstrap_table_wraps_without_truncating_or_injecting_terminal_controls() {
    let mut plan = bootstrap_display_plan();
    add_permission(
        &mut plan,
        0,
        PermissionV1::Command {
            program: "long-command\nFORGED\u{1b}[2J".into(),
        },
    );
    add_permission(
        &mut plan,
        0,
        PermissionV1::Filesystem {
            access: FilesystemAccessV1::Execute,
            path: "preset:install/very-long-script-name.sh".into(),
        },
    );
    let wide = bootstrap_table(&plan, None).join("\n");
    assert!(wide.contains("long-command\\nFORGED\\u{1b}[2J"));
    assert!(!wide.contains('\u{1b}'));
    let narrow = bootstrap_table(&plan, Some(48));
    assert!(narrow.len() > 3);
    assert!(
        narrow
            .iter()
            .all(|line| console::measure_text_width(line) <= 48)
    );
    assert_eq!(
        narrow
            .iter()
            .filter(|line| line.starts_with("    01  "))
            .count(),
        1
    );
    assert_eq!(
        narrow
            .iter()
            .filter(|line| line.starts_with("    02  "))
            .count(),
        1
    );
    for value in [
        "路径/é/very-long-script.sh",
        "long-command\\nFORGED\\u{1b}[2J",
    ] {
        for width in [2, 7, 20] {
            let wrapped = wrap_cell(value, width);
            assert_eq!(wrapped.concat(), value);
            assert!(
                wrapped
                    .iter()
                    .all(|line| console::measure_text_width(line) <= width)
            );
        }
    }
    plan.steps[0].target = "sys/很长的项目名称-long-name".into();
    plan.permission_scopes[0].target = Some(plan.steps[0].target.clone());
    let wide = bootstrap_table(&plan, None);
    let command_column =
        |row: &str, command: &str| console::measure_text_width(&row[..row.find(command).unwrap()]);
    assert_eq!(
        command_column(&wide[1], "curl"),
        command_column(&wide[2], "apt-get")
    );
    assert!(
        bootstrap_table(&plan, Some(48))
            .iter()
            .all(|line| console::measure_text_width(line) <= 48)
    );
    for (action, label) in [
        (PlanActionV1::None, "unchanged"),
        (PlanActionV1::Create, "create"),
        (PlanActionV1::Update, "skip"),
        (PlanActionV1::Remove, "remove"),
        (PlanActionV1::Preserve, "preserve"),
    ] {
        plan.steps[0].action = action;
        assert!(
            bootstrap_table(&plan, None)[1]
                .split_whitespace()
                .any(|field| field == label)
        );
    }
}

#[test]
fn exceptional_bootstrap_layouts_keep_expanded_review() {
    for exception in [
        "blocked",
        "missing-declaration",
        "duplicate-step",
        "unexpected-step",
        "profile-first",
    ] {
        let mut plan = bootstrap_display_plan();
        match exception {
            "blocked" => plan.steps[0].action = PlanActionV1::Blocked,
            "missing-declaration" => {
                plan.permission_scopes[0]
                    .permissions
                    .missing_declarations
                    .insert(PermissionV1::Administrator);
            }
            "duplicate-step" => plan.steps.insert(0, plan.steps[0].clone()),
            "unexpected-step" => plan.steps[0].resource = Some("future-resource".into()),
            "profile-first" => plan.steps.swap(0, 2),
            _ => unreachable!(),
        }
        let output = render_bootstrap_plan_lines(&plan, "config", false)
            .unwrap()
            .join("\n");
        assert!(!output.contains("#   Item"), "{exception}: {output}");
        assert!(output.contains("curl"));
        assert!(output.contains("apt-get"));
        assert!(output.contains("~/.bashrc"));
    }
}

#[test]
fn bootstrap_inline_paths_distinguish_one_comma_bearing_path_from_two_paths() {
    for (value, expected) in [
        ("simple.sh", "simple.sh"),
        (" a.sh ", "\" a.sh \""),
        ("—", "\"—\""),
        ("\"a, b\"", "\"\\\"a, b\\\"\""),
        ("a,\n\u{1b}b", "\"a,\\n\\u{1b}b\""),
    ] {
        assert_eq!(inline_permission_value(value), expected);
    }
    let mut one = bootstrap_display_plan();
    one.permission_scopes[2].permissions.required = PermissionSetV1::default();
    one.permissions.required = PermissionSetV1::new(
        one.permission_scopes
            .iter()
            .flat_map(|scope| scope.permissions.required.iter().cloned()),
    );
    let mut two = one.clone();
    add_permission(
        &mut one,
        2,
        PermissionV1::Filesystem {
            access: FilesystemAccessV1::Write,
            path: "home:.bashrc, ~/.zshrc".into(),
        },
    );
    for path in ["home:.bashrc", "home:.zshrc"] {
        add_permission(
            &mut two,
            2,
            PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: path.into(),
            },
        );
    }
    let one_rendered = render_bootstrap_plan_lines(&one, "config", false).unwrap();
    let two_rendered = render_bootstrap_plan_lines(&two, "config", false).unwrap();
    assert_ne!(
        one_rendered, two_rendered,
        "one path must not look like two permissions"
    );
    assert!(
        one_rendered
            .join("\n")
            .contains("filesystem write: \"~/.bashrc, ~/.zshrc\"")
    );

    add_permission(
        &mut one,
        0,
        PermissionV1::Filesystem {
            access: FilesystemAccessV1::Execute,
            path: "preset:install/a.sh, preset:install/b.sh".into(),
        },
    );
    for path in ["preset:install/a.sh", "preset:install/b.sh"] {
        add_permission(
            &mut two,
            0,
            PermissionV1::Filesystem {
                access: FilesystemAccessV1::Execute,
                path: path.into(),
            },
        );
    }
    assert!(
        bootstrap_table(&one, None)
            .join("\n")
            .contains("\"preset:install/a.sh, preset:install/b.sh\"")
    );
    assert!(
        bootstrap_table(&two, None)
            .join("\n")
            .contains("preset:install/a.sh, preset:install/b.sh")
    );
}

#[test]
fn bootstrap_rendering_preserves_full_approval_and_external_consent() {
    use crate::lifecycle_plan::{LifecyclePlanRequest, confirmation_prompt};
    use shine_core::plan::{PlanApprovalError, PlanApprovalV1};
    use shine_core::runtime::{PlanningInputVersions, SysBootstrapPlanRequest};

    let mut plan = bootstrap_display_plan();
    let mut entry = boundary(
        "sys/rust",
        CodeEntryKindV2::SysBootstrapScript,
        CodeTimingV2::ExecuteNow,
    );
    entry.source = CodeSourceV2::ExternalOrOverlay;
    entry.trust = CodeTrustStateV2::OperationConfirmation;
    plan.code_boundaries.push(entry);
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let before = serde_json::to_vec(&plan).unwrap();
    let request = LifecyclePlanRequest::SysBootstrap {
        request: SysBootstrapPlanRequest {
            os_id: "ubuntu".into(),
            item_ids: vec!["rust".into(), "fzf".into()],
            sys_shell: "bash".into(),
            force_profile: false,
            input_versions: PlanningInputVersions::default(),
        },
        proxy_env: Default::default(),
    };
    for verbose in [false, true] {
        let output = render_bootstrap_plan_lines(&plan, "config", verbose)
            .unwrap()
            .join("\n");
        assert!(output.contains("requires your confirmation for this operation only"));
        assert_eq!(
            confirmation_prompt(&[(request.clone(), plan.clone())]),
            "Apply this Plan and authorize its listed external code for this operation only?"
        );
        assert_eq!(serde_json::to_vec(&plan).unwrap(), before);
        assert_eq!(PlanApprovalV1::for_reviewed_plan(&plan).unwrap(), approval);
        approval.validate(&plan).unwrap();
    }
    for change in [
        "permissions",
        "scope",
        "trust",
        "timing",
        "state",
        "blocked",
    ] {
        let mut changed = plan.clone();
        let expected = match change {
            "permissions" => {
                add_permission(
                    &mut changed,
                    0,
                    PermissionV1::Command {
                        program: "new-program".into(),
                    },
                );
                PlanApprovalError::PermissionSetChanged
            }
            "blocked" => {
                changed.steps[0].action = PlanActionV1::Blocked;
                PlanApprovalError::PlanNotReady
            }
            _ => {
                match change {
                    "scope" => changed.permission_scopes.swap(0, 1),
                    "trust" => changed.code_boundaries[0].trust = CodeTrustStateV2::SnapshotTrusted,
                    "timing" => changed.code_boundaries[0].timing = CodeTimingV2::DeliverForLater,
                    "state" => {
                        changed.inputs.state =
                            shine_core::plan::SnapshotDigestV1::builder("changed-state").finish()
                    }
                    _ => unreachable!(),
                }
                PlanApprovalError::PlanChanged
            }
        };
        for verbose in [false, true] {
            render_bootstrap_plan_lines(&changed, "config", verbose).unwrap();
            assert_eq!(approval.validate(&changed), Err(expected), "{change}");
        }
    }
}
