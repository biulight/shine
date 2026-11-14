use shine_core::runtime::RuntimePlatform;

#[tokio::test]
async fn arch_minimal_is_the_default_and_contains_only_native_packages() {
    let runtime = super::support::runtime(RuntimePlatform::Linux);
    let loaded = runtime.load_sys_preset("arch").await.unwrap();
    assert_eq!(loaded.manifest.default_profile.as_deref(), Some("minimal"));
    assert_eq!(loaded.manifest.profiles.len(), 1);
    assert_eq!(
        loaded.manifest.profiles["minimal"].items,
        ["neovim", "fzf", "bat", "eza", "zoxide"]
    );
    assert_eq!(loaded.manifest.items.len(), 5);
    for item in loaded.manifest.items {
        let shine_core::runtime::SysInstall::Package {
            provider, package, ..
        } = item.install.unwrap()
        else {
            panic!("Arch minimal must use native packages");
        };
        assert_eq!(provider, shine_core::runtime::SysPackageProvider::Pacman);
        assert_eq!(package, item.id);
    }
}

struct NoPrompts;
impl shine_core::runtime::RuntimeInteraction for NoPrompts {
    fn confirm(&mut self, _: &'static str, _: bool) -> anyhow::Result<bool> {
        panic!("explicit approved request must not prompt")
    }
    fn authorize_admin<'a>(
        &'a mut self,
        _: usize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<bool>> + Send + 'a>>
    {
        panic!("root fixture must not invoke sudo")
    }
    fn select_many(
        &mut self,
        _: &'static str,
        _: &[String],
        _: &[String],
    ) -> anyhow::Result<Vec<String>> {
        panic!("non-interactive minimal selection must not prompt")
    }
}

#[tokio::test]
async fn arch_plan_binds_both_startup_files_and_rejects_stale_approval() {
    use shine_core::plan::{PermissionV1, PlanApprovalV1};
    use shine_core::runtime::{
        CoreRuntime, HostOperation, InMemoryHost, NullObserver, ShellType, SysBootstrapPlanRequest,
    };
    for shell in [ShellType::Bash, ShellType::Zsh] {
        for changed_file in [
            ".bashrc",
            ".zshrc",
            ".shine/profile/arch-sys.pre.sh",
            ".shine/profile/arch-sys.post.sh",
            ".shine/profile/arch-sys.pre.base.sh",
            ".shine/profile/arch-sys.post.base.sh",
            ".shine/profile/arch-sys.pre.new.sh",
            ".shine/profile/arch-sys.post.merge.sh",
        ] {
            let embedded = super::support::runtime(RuntimePlatform::Linux);
            let mut context = embedded.context().clone();
            context.shell = shell;
            context.path_env = Some(String::new());
            context.shine_dir = context.home_dir.join(".shine");
            let runtime =
                CoreRuntime::new(InMemoryHost::new(), context, embedded.presets().clone());
            let request = SysBootstrapPlanRequest {
                os_id: "arch".into(),
                item_ids: vec!["fzf".into()],
                sys_shell: if shell == ShellType::Bash {
                    "bash"
                } else {
                    "zsh"
                }
                .into(),
                force_profile: false,
                input_versions: Default::default(),
            };
            let plan = runtime.plan_sys_bootstrap(request.clone()).await.unwrap();
            assert!(plan.is_ready(), "{plan:#?}");
            assert!(plan.permissions.required.contains(&PermissionV1::Command {
                program: "pacman".into()
            }));
            assert!(
                plan.permissions
                    .required
                    .contains(&PermissionV1::Administrator)
            );
            assert!(
                runtime
                    .host()
                    .operations()
                    .iter()
                    .all(|op| matches!(op, HostOperation::Read(_)))
            );
            let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
            runtime.host().put_file(
                runtime.context().home_dir.join(changed_file),
                b"user edit\n".to_vec(),
            );
            assert!(
                runtime
                    .run_sys_bootstrap_approved(
                        request,
                        &approval,
                        &mut NoPrompts,
                        &mut NullObserver
                    )
                    .await
                    .is_err()
            );
            assert!(runtime.host().operations().iter().all(|op| matches!(
                op,
                HostOperation::Read(_) | HostOperation::AcquireOperationLock(_)
            )));
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn arch_bootstrap_installs_once_and_profiles_switch_disable_and_detach_safely() {
    use shine_core::plan::PlanApprovalV1;
    use shine_core::runtime::{
        CoreRuntime, NullObserver, RealHost, ShellType, SysBootstrapPlanRequest, SysItemStatus,
        SysProfilePlanRequest, SysRunManifest, capture_embedded_preset_snapshot, detach_sys_loader,
    };
    use std::os::unix::fs::PermissionsExt;
    for (shell, kind) in [("bash", ShellType::Bash), ("zsh", ShellType::Zsh)] {
        let embedded = super::support::runtime(RuntimePlatform::Linux);
        let mut context = embedded.context().clone();
        context.shell = kind;
        context.running_as_admin = true;
        context.shine_dir = context.home_dir.join(".shine");
        let home = context.home_dir.clone();
        let bin = home.join("fixture-bin");
        std::fs::create_dir_all(&bin).unwrap();
        let path = format!("{}:/usr/bin:/bin", bin.display());
        context.path_env = Some(path.clone());
        // Explicit child env: no process-global PATH mutation and no real package manager.
        context.proxy_env.insert("PATH".into(), path.clone());
        let mut files = embedded.presets().files().clone();
        let mut manifest = String::from_utf8(files["sys/arch/shine.toml"].clone()).unwrap();
        let suffix = uuid::Uuid::new_v4().to_string();
        for command in ["nvim", "fzf", "bat", "eza", "zoxide"] {
            manifest = manifest.replace(
                &format!("command = \"{command}\", version_args"),
                &format!("command = \"{command}-{suffix}\", version_args"),
            );
        }
        files.insert("sys/arch/shine.toml".into(), manifest.into_bytes());
        let pacman = bin.join("pacman");
        std::fs::write(
            &pacman,
            format!(
                r#"#!/bin/sh
set -eu
[ "$#" = 4 ] && [ "$1 $2 $3" = '-S --needed --noconfirm' ] || exit 91
bin=$(dirname "$0")
printf '%s\n' "$4" >> "$bin/calls"
case "$4" in neovim) command=nvim ;; *) command=$4 ;; esac
cat > "$bin/$command" <<'EOF'
#!/bin/sh
case "$1" in
  --version) echo 'fixture 1.0' ;;
  --bash|--zsh) echo 'export SHINE_FZF_FIXTURE=1' ;;
  init) echo 'export SHINE_ZOXIDE_FIXTURE=1' ;;
  *) exit 92 ;;
esac
EOF
chmod +x "$bin/$command"
cp "$bin/$command" "$bin/$command-{suffix}"
"#
            ),
        )
        .unwrap();
        std::fs::set_permissions(&pacman, std::fs::Permissions::from_mode(0o755)).unwrap();
        for startup in [".bashrc", ".zshrc"] {
            std::fs::write(home.join(startup), "# personal settings\n").unwrap();
        }
        let snapshot = capture_embedded_preset_snapshot(files.into_iter().collect());
        let runtime = CoreRuntime::new(RealHost, context.clone(), snapshot.clone());
        let selection = runtime
            .resolve_sys_bootstrap_selection(
                "arch",
                &[],
                None,
                false,
                &mut NoPrompts,
                &mut NullObserver,
            )
            .await
            .unwrap();
        assert_eq!(
            selection.item_ids,
            ["neovim", "fzf", "bat", "eza", "zoxide"]
        );
        let request = SysBootstrapPlanRequest {
            os_id: "arch".into(),
            item_ids: selection.item_ids,
            sys_shell: shell.into(),
            force_profile: false,
            input_versions: Default::default(),
        };
        for expected in [SysItemStatus::Installed, SysItemStatus::AlreadyInstalled] {
            let plan = runtime.plan_sys_bootstrap(request.clone()).await.unwrap();
            let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
            let report = runtime
                .run_sys_bootstrap_approved(
                    request.clone(),
                    &approval,
                    &mut NoPrompts,
                    &mut NullObserver,
                )
                .await
                .unwrap();
            assert!(
                report
                    .outcomes
                    .iter()
                    .filter(|o| o.item_id != "profile")
                    .all(|o| o.status == expected),
                "{:?}",
                report.outcomes
            );
        }
        assert_eq!(
            std::fs::read_to_string(bin.join("calls")).unwrap(),
            "neovim\nfzf\nbat\neza\nzoxide\n"
        );
        let receipts = SysRunManifest::load(&RealHost, &context.shine_dir)
            .await
            .unwrap();
        assert_eq!(receipts.entries.len(), 5);
        assert!(
            receipts
                .entries
                .iter()
                .all(|e| e.os_id == "arch" && e.profile_enabled == (e.item_id != "neovim"))
        );
        let startup = home.join(if shell == "bash" { ".bashrc" } else { ".zshrc" });
        let content = std::fs::read_to_string(&startup).unwrap();
        assert_eq!(content.matches("# >>> shine arch sys pre >>>").count(), 1);
        assert!(!content.contains("ubuntu"));
        assert!(content.contains("# personal settings"));
        // Execute the generated loader twice in an actual shell with harmless tool fixtures.
        if let Some(executable) =
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .map(|dir| dir.join(shell))
                .find(|p| p.is_file())
        {
            let output = std::process::Command::new(executable).args(["-f", "-c",
                "source \"$STARTUP\"; source \"$STARTUP\"; test \"$SHINE_FZF_FIXTURE:$SHINE_ZOXIDE_FIXTURE\" = 1:1 || exit 93; alias cat; alias ls; printf '%s' \"$PATH\""])
                .env_clear().env("HOME", &home).env("PATH", &path).env("STARTUP", &startup)
                .env("SHINE_SYNC_TERMINAL_THEME", "0").output().unwrap();
            assert!(output.status.success(), "{shell}: {output:?}");
            let stdout = String::from_utf8(output.stdout).unwrap();
            assert!(stdout.contains("bat") && stdout.contains("eza --icons"));
            assert_eq!(
                stdout
                    .matches(&home.join(".local/bin").to_string_lossy().to_string())
                    .count(),
                1
            );
        }
        for enabled in [false, true] {
            let request = SysProfilePlanRequest {
                os_id: "arch".into(),
                item_id: "fzf".into(),
                enabled,
            };
            let plan = runtime.plan_sys_profile(request.clone()).await.unwrap();
            let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
            runtime
                .set_sys_profile_approved(request, &approval)
                .await
                .unwrap();
            let post = std::fs::read_to_string(context.shine_dir.join("profile/arch-sys.post.sh"))
                .unwrap();
            assert_eq!(post.contains("fzf"), enabled);
            assert!(post.contains("zoxide"));
        }
        let detached = detach_sys_loader(&content, "arch", Some(shell))
            .unwrap()
            .unwrap();
        assert_eq!(detached.trim(), "# personal settings");
        assert!(
            detach_sys_loader(
                &content.replace("arch-sys.pre.sh", "custom.sh"),
                "arch",
                Some(shell)
            )
            .is_err()
        );
        // Switching Bash/Zsh removes only Arch's old loader and installs the new one.
        context.shell = if kind == ShellType::Bash {
            ShellType::Zsh
        } else {
            ShellType::Bash
        };
        let switched = CoreRuntime::new(RealHost, context, snapshot);
        let request = SysProfilePlanRequest {
            os_id: "arch".into(),
            item_id: "fzf".into(),
            enabled: true,
        };
        let plan = switched.plan_sys_profile(request.clone()).await.unwrap();
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        switched
            .set_sys_profile_approved(request, &approval)
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(&startup).unwrap().trim(),
            "# personal settings"
        );
        let other = home.join(if shell == "bash" { ".zshrc" } else { ".bashrc" });
        assert!(
            std::fs::read_to_string(other)
                .unwrap()
                .contains("# >>> shine arch sys pre >>>")
        );
        std::fs::remove_dir_all(home.parent().unwrap()).unwrap();
    }
}

#[tokio::test]
async fn arch_failed_install_or_missing_binary_stops_before_later_packages() {
    use shine_core::plan::PlanApprovalV1;
    use shine_core::runtime::{
        CoreRuntime, HostOperation, InMemoryHost, NullObserver, ProcessOutput,
        SysBootstrapPlanRequest, SysItemStatus, SysRunManifest,
    };
    for exit_code in [0, 1] {
        let embedded = super::support::runtime(RuntimePlatform::Linux);
        let mut context = embedded.context().clone();
        context.running_as_admin = true;
        context.path_env = Some(String::new());
        let runtime = CoreRuntime::new(InMemoryHost::new(), context, embedded.presets().clone());
        let request = SysBootstrapPlanRequest {
            os_id: "arch".into(),
            item_ids: vec!["neovim".into(), "fzf".into()],
            sys_shell: "bash".into(),
            force_profile: false,
            input_versions: Default::default(),
        };
        let plan = runtime.plan_sys_bootstrap(request.clone()).await.unwrap();
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        runtime.host().queue_process_output(Ok(ProcessOutput {
            exit_code: Some(exit_code),
            stderr: b"error: failed retrieving dependency: HTTP 404".to_vec(),
            ..Default::default()
        }));
        let report = runtime
            .run_sys_bootstrap_approved(request, &approval, &mut NoPrompts, &mut NullObserver)
            .await
            .unwrap();
        assert_eq!(report.outcomes.len(), 1);
        assert_eq!(report.outcomes[0].item_id, "neovim");
        assert_eq!(report.outcomes[0].status, SysItemStatus::Failed);
        assert_eq!(
            report.outcomes[0]
                .detail
                .contains("If downloads return HTTP 404"),
            exit_code != 0
        );
        assert!(
            report.outcomes[0]
                .logs
                .iter()
                .any(|line| line.contains("failed retrieving dependency: HTTP 404"))
        );
        let installs = runtime
            .host()
            .operations()
            .into_iter()
            .filter(|op| matches!(op, HostOperation::Run { .. }))
            .collect::<Vec<_>>();
        assert_eq!(
            installs,
            vec![HostOperation::Run {
                program: "pacman".into(),
                args: vec![
                    "-S".into(),
                    "--needed".into(),
                    "--noconfirm".into(),
                    "neovim".into()
                ]
            }]
        );
        assert!(
            SysRunManifest::load(runtime.host(), &runtime.context().shine_dir)
                .await
                .unwrap()
                .entries
                .is_empty()
        );
    }
}

#[tokio::test]
async fn bootstrap_approval_binds_unselected_enabled_profile_items() {
    use shine_core::plan::PlanApprovalV1;
    use shine_core::runtime::{
        CoreRuntime, InMemoryHost, SysBootstrapPlanRequest, SysItemStatus, SysRunEntry,
        SysRunManifest,
    };
    for os in ["arch", "ubuntu"] {
        let embedded = super::support::runtime(RuntimePlatform::Linux);
        let mut context = embedded.context().clone();
        context.path_env = Some(String::new());
        let runtime = CoreRuntime::new(InMemoryHost::new(), context, embedded.presets().clone());
        let mut receipt = SysRunManifest::default();
        receipt.upsert(SysRunEntry {
            os_id: os.into(),
            item_id: "bat".into(),
            label: "bat".into(),
            status: SysItemStatus::AlreadyInstalled,
            detail: String::new(),
            updated_at: "0".into(),
            managed: false,
            profile_enabled: false,
            receipt: None,
        });
        receipt
            .save(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap();
        let request = SysBootstrapPlanRequest {
            os_id: os.into(),
            item_ids: vec!["fzf".into()],
            sys_shell: "bash".into(),
            force_profile: false,
            input_versions: Default::default(),
        };
        let plan = runtime.plan_sys_bootstrap(request.clone()).await.unwrap();
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        receipt.entries[0].profile_enabled = true;
        receipt
            .save(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap();
        let changed = runtime.plan_sys_bootstrap(request).await.unwrap();
        assert!(
            approval.validate(&changed).is_err(),
            "{os}: unreviewed bat alias activation reused approval"
        );
    }
}

#[tokio::test]
async fn sys_profile_plan_covers_pending_artifact_removal() {
    use shine_core::plan::{FilesystemAccessV1, PermissionV1};
    use shine_core::runtime::{CoreRuntime, InMemoryHost, SysBootstrapPlanRequest};
    for os in ["arch", "ubuntu"] {
        let embedded = super::support::runtime(RuntimePlatform::Linux);
        let mut context = embedded.context().clone();
        context.shine_dir = context.home_dir.join(".shine");
        let runtime = CoreRuntime::new(InMemoryHost::new(), context, embedded.presets().clone());
        let plan = runtime
            .plan_sys_bootstrap(SysBootstrapPlanRequest {
                os_id: os.into(),
                item_ids: vec!["fzf".into()],
                sys_shell: "bash".into(),
                force_profile: false,
                input_versions: Default::default(),
            })
            .await
            .unwrap();
        for phase in ["pre", "post"] {
            for suffix in ["new", "merge"] {
                assert!(
                    plan.permissions
                        .required
                        .contains(&PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Remove,
                            path: format!("shine:profile/{os}-sys.{phase}.{suffix}.sh")
                        }),
                    "{os}: cleanup needs removal permission"
                );
            }
        }
    }
}

#[tokio::test]
async fn force_profile_plan_covers_backup_directory_only_when_requested() {
    use shine_core::plan::{FilesystemAccessV1, PermissionV1};
    use shine_core::runtime::{CoreRuntime, InMemoryHost, SysBootstrapPlanRequest};
    for os in ["arch", "ubuntu"] {
        let embedded = super::support::runtime(RuntimePlatform::Linux);
        let mut context = embedded.context().clone();
        context.shine_dir = context.home_dir.join(".shine");
        let runtime = CoreRuntime::new(InMemoryHost::new(), context, embedded.presets().clone());
        for force_profile in [false, true] {
            let plan = runtime
                .plan_sys_bootstrap(SysBootstrapPlanRequest {
                    os_id: os.into(),
                    item_ids: vec!["fzf".into()],
                    sys_shell: "bash".into(),
                    force_profile,
                    input_versions: Default::default(),
                })
                .await
                .unwrap();
            assert_eq!(
                plan.permissions
                    .required
                    .contains(&PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Write,
                        path: "shine:profile".into()
                    }),
                force_profile
            );
        }
    }
}

#[tokio::test]
async fn pacman_preparation_is_planned_only_for_missing_native_packages() {
    use shine_core::runtime::{CoreRuntime, HostOperation, InMemoryHost, SysBootstrapPlanRequest};
    for (os, present) in [("arch", false), ("arch", true), ("ubuntu", false)] {
        let embedded = super::support::runtime(RuntimePlatform::Linux);
        let mut context = embedded.context().clone();
        context.path_env = Some(String::new());
        let runtime = CoreRuntime::new(InMemoryHost::new(), context, embedded.presets().clone());
        if present {
            runtime.host().put_file_with_mode(
                runtime.context().home_dir.join(".local/bin/fzf"),
                b"fixture".to_vec(),
                0o100755,
            );
        }
        let plan = runtime
            .plan_sys_bootstrap(SysBootstrapPlanRequest {
                os_id: os.into(),
                item_ids: vec!["fzf".into()],
                sys_shell: "bash".into(),
                force_profile: false,
                input_versions: Default::default(),
            })
            .await
            .unwrap();
        assert!(plan.is_ready());
        assert_eq!(
            plan.steps.iter().any(|step| step
                .diagnostic_codes
                .iter()
                .any(|code| code == "sys_pacman_existing_database")),
            os == "arch" && !present
        );
        assert!(
            runtime
                .host()
                .operations()
                .iter()
                .all(|op| matches!(op, HostOperation::Read(_)))
        );
    }
}
