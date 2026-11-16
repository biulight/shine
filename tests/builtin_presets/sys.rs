use shine_core::runtime::{PlanningInputVersions, RuntimePlatform, SysBootstrapPlanRequest};

#[tokio::test]
async fn builtin_bootstrap_shell_selection_preserves_platform_profiles() {
    use shine_core::runtime::{NullObserver, SelectionSource, SysBootstrapBatchRequest};
    for (os, platform, shell, excluded) in [
        (
            "ubuntu",
            RuntimePlatform::Linux,
            "bash",
            vec!["zsh-vi-mode"],
        ),
        ("ubuntu", RuntimePlatform::Linux, "zsh", vec![]),
        ("arch", RuntimePlatform::Linux, "bash", vec![]),
        ("arch", RuntimePlatform::Linux, "zsh", vec![]),
        ("macos", RuntimePlatform::Macos, "zsh", vec![]),
        (
            "macos",
            RuntimePlatform::Macos,
            "bash",
            vec![
                "homebrew",
                "yazi",
                "zsh-autosuggestions",
                "zsh-syntax-highlighting",
                "zsh-vi-mode",
                "nvm",
                "bun",
                "pnpm",
            ],
        ),
        ("windows", RuntimePlatform::Windows, "powershell", vec![]),
    ] {
        let runtime = super::support::runtime(platform);
        let loaded = runtime.load_sys_preset(os).await.unwrap();
        for (profile, declared) in &loaded.manifest.profiles {
            let expected = declared
                .items
                .iter()
                .filter(|id| !excluded.contains(&id.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            let report = runtime
                .preview_sys_bootstrap(
                    SysBootstrapBatchRequest {
                        os_id: os.into(),
                        requested: vec![],
                        preset: Some(profile.clone()),
                        interactive: false,
                        sys_shell: shell.into(),
                        dry_run: true,
                        force_profile: false,
                    },
                    &mut NoBootstrapInteraction,
                    &mut NullObserver,
                )
                .await
                .unwrap();
            assert_eq!(
                report.selection.item_ids, expected,
                "{os}/{shell}/{profile}"
            );
            assert_eq!(
                report.selection.source,
                SelectionSource::Profile(profile.clone())
            );
            assert_eq!(
                report.loaded.manifest.items.len(),
                loaded.manifest.items.len()
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn docker_apt_installer_removes_conflicts_and_stops_on_failure() {
    let root = std::env::temp_dir().join(format!("shine docker-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let script = root.join("docker.sh");
    std::fs::write(
        &script,
        cli::presets::read_embedded_asset_bytes("sys/ubuntu/install/docker.sh").unwrap(),
    )
    .unwrap();
    for conflicts in [false, true] {
        let mut steps = vec!["query"];
        if conflicts {
            steps.push("remove");
        }
        steps.extend([
            "update",
            "dependencies",
            "keydir",
            "download",
            "key",
            "source",
            "source-mode",
            "refresh",
            "packages",
            "start",
            "hello",
        ]);
        for failure in std::iter::once("").chain(steps.iter().copied()) {
            let home = root.join(format!("{conflicts}-{failure}"));
            std::fs::create_dir_all(home.join("tmp")).unwrap();
            let statuses = if conflicts {
                "docker.io\tinstalled\ncontainerd\tinstalled\nrunc\tunpacked\npodman-docker\tinstalled\ndocker-compose\tinstalled\ndocker-compose-v2\tinstalled\ndocker-doc\tinstalled\ndocker-buildx\tinstalled\n"
            } else {
                "docker.io\tconfig-files\nrunc\tnot-installed\nunrelated\tinstalled\n"
            };
            std::fs::write(home.join("status"), statuses).unwrap();
            let output = std::process::Command::new("/bin/bash")
                .args(["--noprofile", "--norc", "-c", r#"
step() { printf '%s\n' "$1" >> "$HOME/calls"; [ "$FAILURE" != "$1" ] || return 42; }
source() { [ "$*" = /etc/os-release ] || return 90; ID=ubuntu; VERSION_CODENAME=noble; UBUNTU_CODENAME=resolute; }
dpkg() { [ "$*" = --print-architecture ] || return 91; echo amd64; }
dpkg-query() {
    [ "$#" = 2 ] && [ "$1" = -W ] && [ "$2" = '-f=${Package}\t${db:Status-Status}\n' ] || return 92
    step query || return $?
    command cat "$HOME/status"
}
apt-get() {
    case "$*" in
        'remove -y docker.io containerd runc podman-docker docker-compose docker-compose-v2 docker-doc docker-buildx') step remove ;;
        'update --error-on=any')
            if [ -f "$HOME/source" ]; then step refresh; else step update; fi ;;
        'install -y ca-certificates curl') step dependencies ;;
        'install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin') step packages ;;
        *) return 93 ;;
    esac
}
install() {
    if [ "$*" = '-m 0755 -d /etc/apt/keyrings' ]; then step keydir
    else
        [ "$#" = 4 ] && [ "$1 $2" = '-m 0644' ] && [ "$4" = /etc/apt/keyrings/docker.asc ] || return 94
        step key || return $?
        command install -m 0644 "$3" "$HOME/key"
    fi
}
curl() {
    [ "$#" = 4 ] && [ "$1 $2 $3" = '-fsSL https://download.docker.com/linux/ubuntu/gpg -o' ] || return 95
    printf partial > "$4"
    step download || return $?
    printf complete-key > "$4"
}
tee() {
    [ "$*" = /etc/apt/sources.list.d/docker.sources ] || return 96
    step source || return $?
    command tee "$HOME/source"
}
chmod() {
    [ "$*" = '0644 /etc/apt/sources.list.d/docker.sources' ] || return 99
    step source-mode || return $?
    command chmod 0644 "$HOME/source"
}
systemctl() {
    case "$*" in
        'is-active --quiet docker') [ -f "$HOME/started" ] ;;
        'start docker') step start || return $?; touch "$HOME/started" ;;
        *) return 97 ;;
    esac
}
docker() { [ "$*" = 'run --rm hello-world' ] || return 98; step hello; }
export -f step source dpkg dpkg-query apt-get install curl tee chmod systemctl docker
umask 077
exec /bin/bash "$1"
"#, "docker-fixture"])
                .arg(&script).env_clear().env("PATH", "/usr/bin:/bin")
                .env("HOME", &home).env("TMPDIR", home.join("tmp"))
                .env("FAILURE", failure).output().unwrap();
            assert_eq!(
                output.status.code(),
                Some(if failure.is_empty() { 0 } else { 42 }),
                "{conflicts}/{failure}: {output:?}"
            );
            let expected = if failure.is_empty() {
                &steps[..]
            } else {
                &steps[..=steps.iter().position(|s| *s == failure).unwrap()]
            };
            assert_eq!(
                std::fs::read_to_string(home.join("calls"))
                    .unwrap()
                    .lines()
                    .collect::<Vec<_>>(),
                expected,
                "{conflicts}/{failure}"
            );
            assert_eq!(std::fs::read_dir(home.join("tmp")).unwrap().count(), 0);
            if failure.is_empty() {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    std::fs::metadata(home.join("key"))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o644
                );
                assert_eq!(
                    std::fs::metadata(home.join("source"))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o644
                );
                assert_eq!(
                    std::fs::read_to_string(home.join("source")).unwrap(),
                    "Types: deb\nURIs: https://download.docker.com/linux/ubuntu\nSuites: resolute\nComponents: stable\nArchitectures: amd64\nSigned-By: /etc/apt/keyrings/docker.asc\n"
                );
            }
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn docker_is_in_ubuntu_recommended_and_all_but_not_minimal() {
    let runtime = super::support::runtime(RuntimePlatform::Linux);
    let loaded = runtime.load_sys_preset("ubuntu").await.unwrap();
    assert_eq!(
        loaded.manifest.default_profile.as_deref(),
        Some("recommended")
    );
    for profile in ["recommended", "all", "minimal"] {
        assert_eq!(
            loaded.manifest.profiles[profile]
                .items
                .iter()
                .filter(|id| *id == "docker")
                .count(),
            usize::from(profile != "minimal")
        );
    }
}

#[test]
fn dpkg_detection_rejects_invalid_or_overlapping_package_names() {
    for detection in [
        "packages = []",
        "packages = ['--option']",
        "packages = ['docker*']",
        "packages = ['docker-ce'], conflicts = ['docker-ce']",
    ] {
        let manifest = format!(
            "version = 2\n[[items]]\nid = 'test'\nlabel = 'Test'\ndetect = {{ kind = 'dpkg', {detection} }}\ninstall = {{ kind = 'script', path = 'install.sh' }}\n"
        );
        assert!(
            shine_core::runtime::parse_sys_manifest(&manifest).is_err(),
            "{detection}"
        );
    }
}

#[tokio::test]
async fn docker_bootstrap_skips_complete_packages_but_not_conflicts_or_partial_installs() {
    use shine_core::plan::{PermissionV1, PlanApprovalV1};
    use shine_core::runtime::{
        CoreRuntime, FileSystemObservationHost, HostOperation, InMemoryHost, NullObserver,
        ProcessOutput, SysItemStatus, SysRunManifest,
    };
    for (scenario, installer_exit) in [
        ("complete", 0),
        ("cli", 0),
        ("docker.io", 42),
        ("docker-compose", 42),
        ("docker-compose-v2", 42),
        ("docker-doc", 42),
        ("docker-buildx", 42),
        ("podman-docker", 42),
        ("containerd", 42),
        ("runc", 42),
        ("partial", 0),
        ("empty", 42),
    ] {
        let engine_present = scenario == "complete";
        let embedded = super::support::runtime(RuntimePlatform::Linux);
        let mut context = embedded.context().clone();
        context.running_as_admin = !engine_present;
        context.path_env = Some(String::new());
        let runtime = CoreRuntime::new(InMemoryHost::new(), context, embedded.presets().clone());
        // A dockerd binary alone must never suppress conflict removal or plugin installation.
        runtime.host().put_file_with_mode(
            runtime.context().home_dir.join(".local/bin/dockerd"),
            b"fixture".to_vec(),
            0o100755,
        );
        let packages = [
            "docker-ce",
            "docker-ce-cli",
            "containerd.io",
            "docker-buildx-plugin",
            "docker-compose-plugin",
        ];
        let mut status = String::new();
        for (index, package) in packages.iter().enumerate() {
            if scenario == "empty"
                || (scenario == "cli" && *package != "docker-ce-cli")
                || (scenario == "partial" && index == 4)
            {
                continue;
            }
            status.push_str(&format!(
                "Package: {package}\nStatus: install ok installed\n\n"
            ));
        }
        if !matches!(scenario, "complete" | "cli" | "partial" | "empty") {
            status.push_str(&format!(
                "Package: {scenario}\nStatus: install ok installed\n\n"
            ));
        }
        runtime
            .host()
            .put_file("/var/lib/dpkg/status", status.into_bytes());
        let request = SysBootstrapPlanRequest {
            os_id: "ubuntu".into(),
            item_ids: vec!["docker".into()],
            sys_shell: "zsh".into(),
            force_profile: false,
            input_versions: Default::default(),
        };
        let plan = runtime.plan_sys_bootstrap(request.clone()).await.unwrap();
        assert!(plan.is_ready(), "{plan:#?}");
        if !engine_present {
            assert!(
                plan.permissions
                    .required
                    .contains(&PermissionV1::Administrator)
            );
        }
        assert!(
            runtime
                .host()
                .operations()
                .iter()
                .all(|op| matches!(op, HostOperation::Read(_)))
        );
        assert_eq!(
            plan.steps.iter().any(|step| step
                .diagnostic_codes
                .iter()
                .any(|code| code == "sys_dpkg_conflicts_may_be_removed")),
            !engine_present
        );
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        // Even a package change that keeps the same install/skip decision invalidates approval.
        let original_status = runtime
            .host()
            .read(std::path::Path::new("/var/lib/dpkg/status"))
            .await
            .unwrap();
        let mut changed_status = original_status.clone();
        changed_status.extend_from_slice(b"\nPackage: unrelated\nStatus: install ok installed\n");
        runtime
            .host()
            .put_file("/var/lib/dpkg/status", changed_status);
        assert!(
            runtime
                .run_sys_bootstrap_approved(
                    request.clone(),
                    &approval,
                    &mut NoBootstrapInteraction,
                    &mut NullObserver
                )
                .await
                .is_err()
        );
        assert!(
            !runtime
                .host()
                .operations()
                .iter()
                .any(|op| matches!(op, HostOperation::Run { .. }))
        );
        runtime
            .host()
            .put_file("/var/lib/dpkg/status", original_status);

        runtime.host().queue_process_output(Ok(ProcessOutput {
            exit_code: Some(installer_exit),
            ..Default::default()
        }));
        let report = runtime
            .run_sys_bootstrap_approved(
                request,
                &approval,
                &mut NoBootstrapInteraction,
                &mut NullObserver,
            )
            .await
            .unwrap();
        assert_eq!(
            report.outcomes[0].status,
            if engine_present {
                SysItemStatus::AlreadyInstalled
            } else {
                // A zero exit without a newly detectable Engine is not success either.
                SysItemStatus::Failed
            }
        );
        let runs = runtime
            .host()
            .operations()
            .into_iter()
            .filter_map(|op| match op {
                HostOperation::Run { program, args } => Some((program, args)),
                _ => None,
            })
            .collect::<Vec<_>>();
        if engine_present {
            assert!(
                runs.is_empty(),
                "existing Engine must not execute any process: {runs:?}"
            );
        } else {
            assert_eq!(runs.len(), 1, "{runs:?}");
            assert!(
                runs[0]
                    .1
                    .iter()
                    .any(|arg| std::path::Path::new(arg).ends_with("install/docker.sh")),
                "{runs:?}"
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
}

#[cfg(unix)]
#[test]
fn yazi_apt_installer_uses_signed_stable_packages_and_stops_on_failure() {
    let root = std::env::temp_dir().join(format!("shine-yazi-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let script = root.join("yazi.sh");
    std::fs::write(
        &script,
        cli::presets::read_embedded_asset_bytes("sys/ubuntu/install/yazi.sh").unwrap(),
    )
    .unwrap();
    for failure in ["", "download", "keyring", "source", "update", "install"] {
        let home = root.join(if failure.is_empty() {
            "success"
        } else {
            failure
        });
        std::fs::create_dir(&home).unwrap();
        std::fs::create_dir(home.join("tmp")).unwrap();
        std::fs::write(home.join("keyring"), "previous key").unwrap();
        std::fs::write(home.join("source"), "previous source").unwrap();
        let output = std::process::Command::new("/bin/bash")
            .args(["--noprofile", "--norc", "-c", r#"
step() {
    printf '%s\n' "$1" >> "$HOME/calls"
    [ "$FAILURE" != "$1" ] || return 42
}
curl() {
    [ "$#" = 4 ] && [ "$1" = -fsSL ] &&
        [ "$2" = https://yazi-rs.github.io/builds/yazi-keyring.gpg ] &&
        [ "$3" = -o ] || return 91
    printf 'partial key' > "$4"
    step download || return $?
    printf 'fixture key' > "$4"
}
sudo() {
    case "$1" in
        install)
            [ "$#" = 5 ] && [ "$2 $3" = '-m 0644' ] &&
                [ "$5" = /usr/share/keyrings/yazi-keyring.gpg ] || return 92
            step keyring || return $?
            command install -m 0644 "$4" "$HOME/keyring" ;;
        tee)
            [ "$#" = 2 ] && [ "$2" = /etc/apt/sources.list.d/yazi.list ] || return 93
            step source || return $?
            command tee "$HOME/source" ;;
        apt-get)
            if [ "$2" = update ]; then
                [ "$*" = 'apt-get update --error-on=any' ] || return 94
                step update
            else
                [ "$*" = 'apt-get install -y yazi file ffmpeg 7zip jq poppler-utils fd-find ripgrep fzf imagemagick xclip' ] || return 95
                step install
            fi ;;
        *) return 96 ;;
    esac
}
export -f step curl sudo
umask 077
/bin/bash "$1" || exit $?
/bin/bash "$1"
"#, "yazi-fixture"])
            .arg(&script)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &home)
            .env("TMPDIR", home.join("tmp"))
            .env("FAILURE", failure)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(if failure.is_empty() { 0 } else { 42 }),
            "{failure}: {output:?}"
        );
        let steps = ["download", "keyring", "source", "update", "install"];
        let expected = if failure.is_empty() {
            steps.repeat(2)
        } else {
            steps[..=steps.iter().position(|step| *step == failure).unwrap()].to_vec()
        };
        assert_eq!(
            std::fs::read_to_string(home.join("calls"))
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(std::fs::read_dir(home.join("tmp")).unwrap().count(), 0);
        if failure == "download" {
            assert_eq!(
                std::fs::read_to_string(home.join("keyring")).unwrap(),
                "previous key"
            );
            assert_eq!(
                std::fs::read_to_string(home.join("source")).unwrap(),
                "previous source"
            );
        }
        if failure.is_empty() {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(home.join("keyring"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o644
            );
            assert_eq!(
                std::fs::read_to_string(home.join("source")).unwrap(),
                "deb [signed-by=/usr/share/keyrings/yazi-keyring.gpg] https://yazi-rs.github.io/builds/ stable main\n"
            );
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

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
                item_ids: vec!["rust".to_string(), "yazi".to_string()],
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
                output_truncated: false,
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

#[cfg(unix)]
#[test]
fn pnpm_standalone_installer_preserves_failures_and_shell_configuration() {
    let home = std::env::temp_dir().join(format!("shine-pnpm-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join(".bashrc"), "# existing startup\n").unwrap();
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("presets/sys/ubuntu/install/pnpm.sh");
    for (download_exit, installer_exit, expected) in [(0, 0, 0), (22, 0, 22), (0, 17, 17)] {
        let output = std::process::Command::new("/bin/bash")
            .args([
                "--noprofile",
                "--norc",
                "-c",
                r#"
curl() {
    [ "$*" = '-fsSL https://get.pnpm.io/install.sh' ] || return 91
    [ "$DOWNLOAD_EXIT" = 0 ] || return "$DOWNLOAD_EXIT"
    printf '%s\n' '[ "$PNPM_NO_MODIFY_PATH" = 1 ] || exit 92' \
        '[ "$PNPM_HOME" = "$SHINE_TARGET_HOME/.local/share/pnpm" ] || exit 93' \
        'echo standalone-installer-reached' 'exit "$INSTALLER_EXIT"'
}
export -f curl
exec /bin/bash "$1"
"#,
                "pnpm-fixture",
            ])
            .arg(&script)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", home.join("other-home"))
            .env("SHINE_TARGET_HOME", &home)
            .env("DOWNLOAD_EXIT", download_exit.to_string())
            .env("INSTALLER_EXIT", installer_exit.to_string())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(expected), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).contains("standalone-installer-reached"),
            download_exit == 0
        );
        assert_eq!(
            std::fs::read_to_string(home.join(".bashrc")).unwrap(),
            "# existing startup\n"
        );
    }
    std::fs::remove_dir_all(home).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn pnpm_first_install_is_detected_before_shell_activation() {
    use shine_core::plan::PlanApprovalV1;
    use shine_core::runtime::{
        CoreRuntime, RealHost, ShellType, SysItemStatus, SysRunManifest,
        capture_embedded_preset_snapshot,
    };
    for (shell, kind, creates_binary) in [
        ("bash", ShellType::Bash, true),
        ("zsh", ShellType::Zsh, true),
        ("bash", ShellType::Bash, false),
    ] {
        let embedded = super::support::runtime(RuntimePlatform::Linux);
        let mut context = embedded.context().clone();
        context.shell = kind;
        context.shine_dir = context.home_dir.join(".shine");
        context.shell_config_paths =
            vec![cli::shells::get_shell_config_path(&kind, &context.home_dir).unwrap()];
        context.path_env = Some("/usr/bin:/bin".into());
        let home = context.home_dir.clone();
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(&context.shell_config_paths[0], "# existing startup\n").unwrap();
        let mut files = embedded.presets().files().clone();
        // Prevent an ambient pnpm in a provider root from hiding first-install failures.
        let alias = format!("pnpm-fixture-{}", uuid::Uuid::new_v4());
        let manifest = String::from_utf8(files["sys/ubuntu/shine.toml"].clone()).unwrap();
        let detection =
            r#"detect = { kind = "command", command = "pnpm", version_args = ["--version"] }"#;
        assert_eq!(manifest.matches(detection).count(), 1);
        files.insert(
            "sys/ubuntu/shine.toml".into(),
            manifest
                .replace(
                    detection,
                    &detection.replace("\"pnpm\"", &format!("\"{alias}\"")),
                )
                .into_bytes(),
        );
        let script = String::from_utf8(files["sys/ubuntu/install/pnpm.sh"].clone()).unwrap();
        files.insert(
            "sys/ubuntu/install/pnpm.sh".into(),
            format!(
                "curl() {{ cat \"$SHINE_SYS_PRESET_ROOT/install/pnpm-fixture.sh\"; }}\n{script}"
            )
            .into_bytes(),
        );
        let installer = if creates_binary {
            format!(
                r#"set -eu
[ "$PNPM_NO_MODIFY_PATH" = 1 ]
[ "$PNPM_HOME" = "$SHINE_TARGET_HOME/.local/share/pnpm" ]
printf 'installed\n' >> "$SHINE_TARGET_HOME/install-count"
mkdir -p "$PNPM_HOME/bin"
printf '#!/bin/sh\necho pnpm-fixture\n' > "$PNPM_HOME/bin/pnpm"
chmod +x "$PNPM_HOME/bin/pnpm"
ln -s pnpm "$PNPM_HOME/bin/{alias}"
"#
            )
        } else {
            "exit 0\n".into()
        };
        files.insert(
            "sys/ubuntu/install/pnpm-fixture.sh".into(),
            installer.into_bytes(),
        );
        let runtime = CoreRuntime::new(
            RealHost,
            context,
            capture_embedded_preset_snapshot(files.into_iter().collect()),
        );
        let request = SysBootstrapPlanRequest {
            os_id: "ubuntu".into(),
            item_ids: vec!["pnpm".into()],
            sys_shell: shell.into(),
            force_profile: false,
            input_versions: Default::default(),
        };
        let expected = if creates_binary {
            vec![SysItemStatus::Installed, SysItemStatus::AlreadyInstalled]
        } else {
            vec![SysItemStatus::Failed]
        };
        for status in expected {
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
            assert_eq!(report.outcomes[0].status, status, "{:?}", report.outcomes);
        }
        let receipt = SysRunManifest::load(&RealHost, &runtime.context().shine_dir)
            .await
            .unwrap();
        if creates_binary {
            assert_eq!(
                std::fs::read_to_string(home.join("install-count")).unwrap(),
                "installed\n"
            );
            assert_eq!(receipt.entries.len(), 1);
            assert_eq!(receipt.entries[0].item_id, "pnpm");
            assert!(receipt.entries[0].profile_enabled);
            let startup =
                std::fs::read_to_string(&runtime.context().shell_config_paths[0]).unwrap();
            assert_eq!(startup.matches("# existing startup\n").count(), 1);
            assert!(!startup.contains("PNPM_HOME"));
            let pre =
                std::fs::read_to_string(home.join(".shine/profile/ubuntu-sys.pre.sh")).unwrap();
            if let Some(executable) =
                std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                    .map(|dir| dir.join(shell))
                    .find(|path| path.is_file())
            {
                let output = std::process::Command::new(executable)
                    .args([
                        "-f",
                        "-c",
                        &format!("{pre}\n{pre}\ncommand -v pnpm\nprintf '%s' \"$PATH\""),
                    ])
                    .env_clear()
                    .env("HOME", &home)
                    .env("PATH", "/usr/bin:/bin")
                    .output()
                    .unwrap();
                assert!(output.status.success(), "{output:?}");
                let stdout = String::from_utf8(output.stdout).unwrap();
                let mut lines = stdout.lines();
                let bin = home.join(".local/share/pnpm/bin");
                assert_eq!(
                    std::path::Path::new(lines.next().unwrap()),
                    bin.join("pnpm")
                );
                assert_eq!(
                    lines
                        .next()
                        .unwrap()
                        .split(':')
                        .filter(|path| std::path::Path::new(path) == bin)
                        .count(),
                    1
                );
            }
        } else {
            assert!(receipt.entries.iter().all(|entry| entry.item_id != "pnpm"));
        }
        std::fs::remove_dir_all(home.parent().unwrap()).unwrap();
    }
}
