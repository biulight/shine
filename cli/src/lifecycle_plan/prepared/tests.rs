use super::*;
use crate::lifecycle_plan::{planning_input_versions, review_upgrade_plans};
use shine_core::runtime::{AppPlanRequest, NullObserver, ShellPlanRequest, SysManagedPlanRequest};
use std::{future::Future, path::PathBuf, pin::Pin};

struct Interaction;
impl RuntimeInteraction for Interaction {
    fn confirm(&mut self, _: &'static str, default: bool) -> Result<bool> {
        Ok(default)
    }
    fn authorize_admin<'a>(
        &'a mut self,
        _: usize,
    ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send + 'a>> {
        Box::pin(async { panic!("empty upgrade must not request privilege") })
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
fn config() -> (PathBuf, Config) {
    let root =
        std::env::temp_dir().join(format!("shine-prepared-upgrade-{}", uuid::Uuid::new_v4()));
    let mut config = Config::new_for_test(&root);
    config.is_external_presets = true;
    (root, config)
}
fn request(config: &Config, domain: &str) -> LifecyclePlanRequest {
    let operation = LifecycleOperation::Upgrade;
    let input_versions = planning_input_versions(config);
    match domain {
        "shell" => LifecyclePlanRequest::Shell(ShellPlanRequest {
            operation,
            target: None,
            force: false,
            purge: false,
            input_versions,
        }),
        "app" => LifecyclePlanRequest::App(AppPlanRequest {
            operation,
            target: None,
            force: false,
            purge: false,
            prune_stale: false,
            input_versions,
        }),
        "sys" => LifecyclePlanRequest::Sys(SysManagedPlanRequest {
            operation,
            target: None,
            os_id: "macos".into(),
            input_versions,
        }),
        _ => panic!("unknown fixture domain"),
    }
}
async fn reviews(config: &Config, domains: &[&str]) -> Vec<ReviewedLifecyclePlan> {
    review_upgrade_plans(
        config,
        domains.iter().map(|domain| request(config, domain)),
        true,
        false,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn upgrade_batch_rejects_missing_extra_duplicate_and_misordered_domains_before_capture() {
    for domains in [
        vec![],
        vec!["shell", "app"],
        vec!["shell", "app", "sys", "sys"],
        vec!["app", "shell", "sys"],
        vec!["shell", "shell", "sys"],
        vec!["shell", "app", "app"],
    ] {
        let (root, config) = config();
        let reviewed = reviews(&config, &domains).await;
        // A capture would fail on this store. Batch shape errors must win without a capture.
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("trust.toml"), "malformed [").unwrap();
        let error = prepare_upgrade_plans(&config, reviewed)
            .await
            .err()
            .expect("invalid batch");
        assert!(
            error.to_string().starts_with("upgrade requires"),
            "{error:#}"
        );
        assert!(!config.bin_dir().exists());
        assert!(!config.presets_dir().exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[tokio::test]
async fn upgrade_batch_rejects_non_upgrade_before_preparing_any_domain() {
    let (root, config) = config();
    let mut reviewed = reviews(&config, &["shell", "app", "sys"]).await;
    let LifecyclePlanRequest::Sys(request) = &mut reviewed[2].request else {
        unreachable!()
    };
    request.operation = LifecycleOperation::Install;
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("trust.toml"), "malformed [").unwrap();
    let error = prepare_upgrade_plans(&config, reviewed)
        .await
        .err()
        .unwrap();
    assert!(
        error.to_string().contains("managed Sys upgrade Plan third"),
        "{error:#}"
    );
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn later_domain_prevalidation_failure_never_executes_earlier_domains() {
    let (root, config) = config();
    let reviewed = reviews(&config, &["shell", "app", "sys"]).await;
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("sys-manifest.toml"), "schema_version = 999\n").unwrap();
    assert!(prepare_upgrade_plans(&config, reviewed).await.is_err());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    assert!(!config.bin_dir().exists());
    assert!(!config.presets_dir().exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn prepared_domains_return_their_reports_and_still_revalidate_at_apply() {
    let (root, config) = config();
    let reviewed = reviews(&config, &["shell", "app", "sys"]).await;
    let prepared = prepare_upgrade_plans(&config, reviewed).await.unwrap();
    assert!(!root.exists(), "prevalidation must not initialize state");
    let shell = prepared
        .shell
        .execute(
            &config,
            ExecutionOptions::default(),
            &mut NullObserver,
            &mut Interaction,
        )
        .await
        .unwrap();
    assert!(shell.runs.is_empty());
    let app = prepared
        .app
        .execute(
            &config,
            ExecutionOptions::default(),
            &mut NullObserver,
            &mut Interaction,
        )
        .await
        .unwrap();
    assert!(app.files.is_empty());
    let sys = prepared
        .sys
        .into_managed()
        .execute(
            &config,
            ExecutionOptions::default(),
            &mut NullObserver,
            &mut Interaction,
        )
        .await
        .unwrap();
    assert!(sys.items.is_empty());
    std::fs::remove_dir_all(&root).unwrap();

    let reviewed = reviews(&config, &["shell", "app", "sys"]).await;
    let prepared = prepare_upgrade_plans(&config, reviewed).await.unwrap();
    std::fs::create_dir_all(&root).unwrap();
    // Preparation does not replace Core's live approval checks.
    std::fs::write(
        root.join("app-manifest.toml"),
        "schema_version = 1\nentries = []\n",
    )
    .unwrap();
    let error = prepared
        .app
        .execute(
            &config,
            ExecutionOptions::default(),
            &mut NullObserver,
            &mut Interaction,
        )
        .await
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("the Plan changed after approval"),
        "{error:#}"
    );
    assert!(!config.bin_dir().exists());
    assert!(!config.presets_dir().exists());
    assert!(!root.join("app-operation-journal.toml").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn singleton_handoffs_reject_other_domains_before_capture() {
    let (root, config) = config();
    let [shell, app, other_app]: [ReviewedLifecyclePlan; 3] =
        reviews(&config, &["shell", "app", "app"])
            .await
            .try_into()
            .unwrap();
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("trust.toml"), "malformed [").unwrap();
    for error in [
        PreparedAppUpgrade::prepare(&config, shell)
            .await
            .err()
            .unwrap(),
        PreparedShellUpgrade::prepare(&config, app)
            .await
            .err()
            .unwrap(),
        PreparedSysManaged::prepare(&config, other_app)
            .await
            .err()
            .unwrap(),
    ] {
        assert!(
            error.to_string().starts_with("unexpected reviewed request"),
            "{error:#}"
        );
    }
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
    std::fs::remove_dir_all(root).unwrap();
}
