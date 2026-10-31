use shine_core::runtime::{PlanningInputVersions, RuntimePlatform, SysBootstrapPlanRequest};

#[cfg(unix)]
#[test]
fn atuin_bootstrap_is_non_interactive_and_preserves_failures() {
    let home = std::env::temp_dir().join(format!("shine-atuin-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&home).unwrap();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("presets/sys/ubuntu/install/atuin.sh");
    for (download_exit, installer_exit, expected) in [(0, 0, 0), (22, 0, 22), (0, 17, 17)] {
        // A Bash function intercepts curl in the real preset. The simulated upstream
        // installer refuses the interactive path instead of reading the user's tty.
        let output = std::process::Command::new("/bin/bash")
            .args([
                "--noprofile",
                "--norc",
                "-c",
                r#"curl() {
    if [ "$DOWNLOAD_EXIT" != 0 ]; then return "$DOWNLOAD_EXIT"; fi
    printf '%s\n' '[ "$#" = 1 ] && [ "$1" = --non-interactive ] || exit 90' \
        'printf "non-interactive installer reached\n"' 'exit "$INSTALLER_EXIT"'
}
export -f curl
exec /bin/bash "$1"
"#,
                "atuin-fixture",
            ])
            .arg(&script)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &home)
            .env("SHINE_CONFIG_DIR", home.join("state"))
            .env("DOWNLOAD_EXIT", download_exit.to_string())
            .env("INSTALLER_EXIT", installer_exit.to_string())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(expected), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).contains("non-interactive installer reached"),
            download_exit == 0,
            "{output:?}"
        );
    }
    std::fs::remove_dir_all(home).unwrap();
}

#[tokio::test]
async fn built_in_sys_bootstrap_plan_is_ready_for_each_platform() {
    for (platform, os_id, shell) in [
        (RuntimePlatform::Macos, "macos", "zsh"),
        (RuntimePlatform::Linux, "ubuntu", "zsh"),
        (RuntimePlatform::Windows, "windows", "powershell"),
    ] {
        let runtime = super::support::runtime(platform);
        let plan = runtime
            .plan_sys_bootstrap(SysBootstrapPlanRequest {
                os_id: os_id.to_string(),
                item_ids: vec!["rust".to_string()],
                sys_shell: shell.to_string(),
                force_profile: false,
                input_versions: PlanningInputVersions::default(),
            })
            .await
            .unwrap();
        assert!(
            plan.is_ready(),
            "blocked built-in Plan for {os_id}: {plan:#?}"
        );
    }
}

struct NoBootstrapInteraction;

impl shine_core::runtime::RuntimeInteraction for NoBootstrapInteraction {
    fn confirm(&mut self, _: &'static str, _: bool) -> anyhow::Result<bool> {
        panic!("already-installed Rust must not prompt")
    }

    fn authorize_admin<'a>(
        &'a mut self,
        _: usize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<bool>> + Send + 'a>>
    {
        panic!("already-installed Rust must not request administrator access")
    }

    fn select_many(
        &mut self,
        _: &'static str,
        _: &[String],
        _: &[String],
    ) -> anyhow::Result<Vec<String>> {
        panic!("explicit Rust selection must not prompt")
    }
}

#[tokio::test]
async fn installed_rust_bootstrap_delivers_cargo_path_on_each_platform() {
    use shine_core::plan::PlanApprovalV1;
    use shine_core::runtime::{
        CoreRuntime, FileSystemObservationHost, HostOperation, InMemoryHost, NullObserver,
        ProcessOutput, ShellType, SysItemStatus, SysRunEntry, SysRunManifest,
    };

    for (platform, os_id, shell, shell_type) in [
        (RuntimePlatform::Linux, "ubuntu", "bash", ShellType::Bash),
        (RuntimePlatform::Linux, "ubuntu", "zsh", ShellType::Zsh),
        (RuntimePlatform::Macos, "macos", "zsh", ShellType::Zsh),
        (
            RuntimePlatform::Windows,
            "windows",
            "powershell",
            ShellType::PowerShell,
        ),
    ] {
        let embedded = super::support::runtime(platform);
        let mut context = embedded.context().clone();
        context.shell = shell_type;
        context.shell_config_paths =
            vec![cli::shells::get_shell_config_path(&shell_type, &context.home_dir).unwrap()];
        context.path_env = Some(String::new());
        context.shine_dir = context.home_dir.join(".shine");
        let host = InMemoryHost::new();
        let rustup = context
            .home_dir
            .join(if platform == RuntimePlatform::Windows {
                ".cargo/bin/rustup.exe"
            } else {
                ".cargo/bin/rustup"
            });
        host.put_file_with_mode(&rustup, b"existing Rust installation".to_vec(), 0o100755);
        // Older releases recorded Rust without enabling any item-owned profile.
        let mut receipt = SysRunManifest::default();
        receipt.upsert(SysRunEntry {
            os_id: os_id.into(),
            item_id: "rust".into(),
            label: "Rust".into(),
            status: SysItemStatus::AlreadyInstalled,
            detail: String::new(),
            updated_at: "0".into(),
            managed: false,
            profile_enabled: false,
            receipt: None,
        });
        receipt.save(&host, &context.shine_dir).await.unwrap();
        let runtime = CoreRuntime::new(host, context, embedded.presets().clone());
        let request = SysBootstrapPlanRequest {
            os_id: os_id.into(),
            item_ids: vec!["rust".into()],
            sys_shell: shell.into(),
            force_profile: false,
            input_versions: Default::default(),
        };
        let mut previous = None;
        for _ in 0..2 {
            let plan = runtime.plan_sys_bootstrap(request.clone()).await.unwrap();
            let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
            runtime.host().queue_process_output(Ok(ProcessOutput {
                interrupted: false,
                exit_code: Some(0),
                stdout: b"rustup fixture\n".to_vec(),
                stderr: Vec::new(),
            }));
            let report = runtime
                .run_sys_bootstrap_approved(
                    request.clone(),
                    &approval,
                    &mut NoBootstrapInteraction,
                    &mut NullObserver,
                )
                .await
                .unwrap();
            assert_eq!(report.outcomes[0].status, SysItemStatus::AlreadyInstalled);
            assert!(
                report
                    .outcomes
                    .iter()
                    .all(|outcome| outcome.status != SysItemStatus::Failed)
            );
            let ext = if platform == RuntimePlatform::Windows {
                "ps1"
            } else {
                "sh"
            };
            let path = runtime
                .context()
                .home_dir
                .join(format!(".shine/profile/{os_id}-sys.pre.{ext}"));
            let profile = String::from_utf8(runtime.host().read(&path).await.unwrap()).unwrap();
            assert!(
                profile.contains(".cargo/bin") || profile.contains(r".cargo\bin"),
                "{os_id}/{shell}: {profile}"
            );
            if let Some(previous) = &previous {
                assert_eq!(
                    &profile, previous,
                    "rerun must preserve composed profile bytes"
                );
            }
            previous = Some(profile);
        }
        for operation in runtime.host().operations() {
            if let HostOperation::Run { program, args } = operation {
                // Windows accepts both separators; compare path components, not spelling.
                assert_eq!(std::path::Path::new(&program), rustup.as_path());
                assert_eq!(args, ["--version"], "must not reinstall Rust");
            }
        }
        if os_id == "ubuntu" {
            let receipt = SysRunManifest::load(runtime.host(), &runtime.context().shine_dir)
                .await
                .unwrap();
            assert!(
                receipt
                    .entries
                    .iter()
                    .find(|entry| entry.item_id == "rust")
                    .unwrap()
                    .profile_enabled
            );
        }
        #[cfg(unix)]
        if platform != RuntimePlatform::Windows {
            check_rust_profile_commands(shell, &previous.unwrap());
        }
    }
}

#[cfg(unix)]
fn check_rust_profile_commands(shell: &str, profile: &str) {
    use std::os::unix::fs::PermissionsExt;
    let Some(executable) = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join(shell))
        .find(|path| path.is_file())
    else {
        eprintln!("{shell} unavailable; skipping native profile execution only");
        return;
    };
    let home = std::env::temp_dir().join(format!("shine Rust home [1]-{}", uuid::Uuid::new_v4()));
    let bin = home.join(".cargo/bin");
    std::fs::create_dir_all(&bin).unwrap();
    for command in ["rustup", "cargo", "rustc"] {
        let path = bin.join(command);
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let output = std::process::Command::new(executable)
        .args([
            "-f",
            "-c",
            &format!("{profile}\n{profile}\ncommand -v rustup cargo rustc\nprintf '%s' \"$PATH\""),
        ])
        .env_clear()
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .env("SHINE_SYNC_TERMINAL_THEME", "0")
        .output()
        .unwrap();
    assert!(output.status.success(), "{shell}: {output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let mut lines = stdout.lines();
    for command in ["rustup", "cargo", "rustc"] {
        assert_eq!(lines.next().unwrap(), bin.join(command).to_string_lossy());
    }
    assert_eq!(
        lines
            .next()
            .unwrap()
            .split(':')
            .filter(|path| *path == bin.to_string_lossy())
            .count(),
        1
    );
    std::fs::remove_dir_all(home).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn atuin_first_install_is_detected_and_enables_shell_integration() {
    use shine_core::plan::PlanApprovalV1;
    use shine_core::runtime::{
        CoreRuntime, RealHost, ShellType, SysItemStatus, SysRunManifest,
        capture_embedded_preset_snapshot,
    };
    for (shell, kind) in [("bash", ShellType::Bash), ("zsh", ShellType::Zsh)] {
        let embedded = super::support::runtime(RuntimePlatform::Linux);
        let mut context = embedded.context().clone();
        context.shell = kind;
        context.shine_dir = context.home_dir.join(".shine");
        context.shell_config_paths =
            vec![cli::shells::get_shell_config_path(&kind, &context.home_dir).unwrap()];
        context.path_env = Some("/usr/bin:/bin".into());
        let home = context.home_dir.clone();
        // Replace the network installer. Detection, approval, profile composition,
        // receipts and the embedded Atuin integrations all use production code.
        let mut files = embedded.presets().files().clone();
        // A unique detection alias prevents a developer's globally installed Atuin
        // from satisfying this fresh-install fixture through provider fallback roots.
        let alias = format!("atuin-fixture-{}", uuid::Uuid::new_v4());
        let manifest = String::from_utf8(files["sys/ubuntu/shine.toml"].clone()).unwrap();
        let detection =
            r#"detect = { kind = "command", command = "atuin", version_args = ["--version"] }"#;
        assert_eq!(manifest.matches(detection).count(), 1);
        files.insert(
            "sys/ubuntu/shine.toml".into(),
            manifest
                .replace(
                    detection,
                    &detection.replace("\"atuin\"", &format!("\"{alias}\"")),
                )
                .into_bytes(),
        );
        files.insert(
            "sys/ubuntu/install/atuin.sh".into(),
            br#"set -eu
mkdir -p "$SHINE_TARGET_HOME/.atuin/bin"
printf 'installed\n' >> "$SHINE_TARGET_HOME/install-count"
cat > "$SHINE_TARGET_HOME/.atuin/bin/atuin" <<'EOF'
#!/bin/sh
case "$1" in
  --version) echo 'atuin fixture' ;;
  init) printf 'export SHINE_ATUIN_INITIALIZED=%s\n' "$2" ;;
  *) exit 1 ;;
esac
EOF
chmod +x "$SHINE_TARGET_HOME/.atuin/bin/atuin"
cat > "$SHINE_TARGET_HOME/.atuin/bin/env" <<'EOF'
case ":$PATH:" in
  *":$HOME/.atuin/bin:"*) ;;
  *) export PATH="$HOME/.atuin/bin:$PATH" ;;
esac
EOF
"#
            .to_vec(),
        );
        files
            .get_mut("sys/ubuntu/install/atuin.sh")
            .unwrap()
            .extend_from_slice(
                format!("ln -s atuin \"$SHINE_TARGET_HOME/.atuin/bin/{alias}\"\n").as_bytes(),
            );
        let runtime = CoreRuntime::new(
            RealHost,
            context,
            capture_embedded_preset_snapshot(files.into_iter().collect()),
        );
        let request = SysBootstrapPlanRequest {
            os_id: "ubuntu".into(),
            item_ids: vec!["atuin".into()],
            sys_shell: shell.into(),
            force_profile: false,
            input_versions: Default::default(),
        };
        assert!(!home.join(".atuin/bin/atuin").exists());
        for expected in [SysItemStatus::Installed, SysItemStatus::AlreadyInstalled] {
            let plan = runtime.plan_sys_bootstrap(request.clone()).await.unwrap();
            let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
            let report = runtime
                .run_sys_bootstrap_approved(
                    request.clone(),
                    &approval,
                    &mut NoBootstrapInteraction,
                    &mut shine_core::runtime::NullObserver,
                )
                .await
                .unwrap();
            assert_eq!(report.outcomes[0].status, expected, "{:?}", report.outcomes);
            assert!(
                report
                    .outcomes
                    .iter()
                    .all(|outcome| outcome.status != SysItemStatus::Failed)
            );
        }
        assert_eq!(
            std::fs::read_to_string(home.join("install-count")).unwrap(),
            "installed\n"
        );
        let receipt = SysRunManifest::load(&RealHost, &runtime.context().shine_dir)
            .await
            .unwrap();
        assert_eq!(receipt.entries.len(), 1);
        assert_eq!(receipt.entries[0].item_id, "atuin");
        assert!(receipt.entries[0].profile_enabled);
        let pre = std::fs::read_to_string(home.join(".shine/profile/ubuntu-sys.pre.sh")).unwrap();
        let post = std::fs::read_to_string(home.join(".shine/profile/ubuntu-sys.post.sh")).unwrap();
        assert!(pre.contains(".atuin/bin/env"));
        assert!(post.contains("atuin"));
        let startup = std::fs::read_to_string(&runtime.context().shell_config_paths[0]).unwrap();
        assert!(startup.contains("ubuntu-sys.pre.sh"));
        assert!(startup.contains("ubuntu-sys.post.sh"));
        // Shell execution is additional native evidence; composition above runs even
        // on Unix CI hosts without Zsh installed.
        if let Some(executable) =
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .map(|directory| directory.join(shell))
                .find(|path| path.is_file())
        {
            let output = std::process::Command::new(executable)
                .args(["-f", "-c", &format!("{pre}\n{post}\n{pre}\n{post}\ncommand -v atuin\nprintf '%s\\n%s' \"$SHINE_ATUIN_INITIALIZED\" \"$PATH\"")])
                .env_clear().env("HOME", &home).env("PATH", "/usr/bin:/bin")
                .env("SHINE_SYNC_TERMINAL_THEME", "0").output().unwrap();
            assert!(output.status.success(), "{output:?}");
            let stdout = String::from_utf8(output.stdout).unwrap();
            let mut lines = stdout.lines();
            assert_eq!(
                std::path::Path::new(lines.next().unwrap()),
                home.join(".atuin/bin/atuin")
            );
            assert_eq!(lines.next(), Some(shell));
            assert_eq!(
                lines
                    .next()
                    .unwrap()
                    .split(':')
                    .filter(|path| std::path::Path::new(path) == home.join(".atuin/bin"))
                    .count(),
                1
            );
        }
        std::fs::remove_dir_all(home.parent().unwrap()).unwrap();
    }
}
