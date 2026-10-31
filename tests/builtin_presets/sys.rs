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
