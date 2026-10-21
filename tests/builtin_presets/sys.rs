use shine_core::runtime::{PlanningInputVersions, RuntimePlatform, SysBootstrapPlanRequest};

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
