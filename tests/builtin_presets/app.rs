use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;

#[tokio::test]
async fn surge_install_plan_binds_only_enabled_subscription_generators() {
    use shine_core::lifecycle::LifecycleOperation;
    use shine_core::plan::{EnvironmentSensitivityV1, PermissionV1, PlanActionV1, PlanApprovalV1};
    use shine_core::runtime::{
        AppPlanRequest, CoreRuntime, InMemoryHost, OpaqueSecretVersion, PlanningInputVersions,
        RuntimePlatform,
    };

    let main = "SURGE_SUBSCRIPTION_URL";
    let secondary = "W_GET_CLOUD_SURGE_SUBSCRIPTION_URL";
    let embedded = super::support::runtime(RuntimePlatform::Macos);
    let mut context = embedded.context().clone();
    context
        .env
        .insert(main.into(), "https://example.invalid/main".into());
    let host = InMemoryHost::new();
    let runtime = CoreRuntime::new(host.clone(), context.clone(), embedded.presets().clone());
    let mut request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("surge".into()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };

    let missing_identity = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(
        missing_identity
            .permissions
            .uncomputable_codes
            .contains("secret_input_identity_unavailable")
    );
    assert!(PlanApprovalV1::for_reviewed_plan(&missing_identity).is_err());

    request
        .input_versions
        .insert_secret_version(main, OpaqueSecretVersion::new("main-version"));
    let main_only = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&main_only).unwrap_or_else(|err| {
        panic!("main-only Surge install should be approvable: {err}");
    });
    assert!(main_only.steps.iter().any(|step| {
        step.resource.as_deref() == Some("generator:subscription-proxies.conf")
            && step.action == PlanActionV1::Execute
    }));
    assert!(main_only.steps.iter().any(|step| {
        step.resource.as_deref() == Some("sub/wget-cloud-proxies.conf")
            && step.action == PlanActionV1::Create
            && step.diagnostic_codes.is_empty()
    }));
    assert!(
        !main_only.steps.iter().any(|step| {
            step.resource.as_deref() == Some("generator:sub/wget-cloud-proxies.conf")
        })
    );
    assert_eq!(
        main_only
            .steps
            .iter()
            .filter(|step| {
                step.resource.as_deref() == Some("generator-runtime:generate-subscription.ts")
            })
            .count(),
        1
    );
    assert!(
        !main_only
            .permissions
            .required
            .contains(&PermissionV1::Environment {
                name: secondary.into(),
                sensitivity: EnvironmentSensitivityV1::Secret,
            })
    );

    context
        .env
        .insert(secondary.into(), "https://example.invalid/secondary".into());
    let runtime = CoreRuntime::new(host, context, embedded.presets().clone());
    request
        .input_versions
        .insert_secret_version(secondary, OpaqueSecretVersion::new("secondary-version"));
    let both = runtime.plan_apps(request).await.unwrap();
    assert!(both.is_ready());
    assert!(both.steps.iter().any(|step| {
        step.resource.as_deref() == Some("generator:sub/wget-cloud-proxies.conf")
            && step.action == PlanActionV1::Execute
    }));
    assert_ne!(main_only.inputs.state, both.inputs.state);
    assert!(approval.validate(&both).is_err());
}

#[test]
fn surge_deploys_an_inert_general_module_to_its_native_profiles_directory() {
    let runtime = super::support::runtime(shine_core::runtime::RuntimePlatform::Macos);
    let categories = runtime.app_categories(Some("surge")).unwrap();
    let surge = categories.first().unwrap();
    let module = surge
        .files
        .iter()
        .find(|file| file.source_rel == Path::new("shine-general.sgmodule"))
        .expect("the General module is an ordinary managed App file");
    assert_eq!(
        runtime.app_destination(surge, module).unwrap(),
        runtime
            .context()
            .home_dir
            .join("Library/Application Support/Surge/Profiles/shine-general.sgmodule")
    );
    assert!(module.generator.is_none());
    let mut relocated = surge.clone();
    relocated.destination_root = Some(
        runtime
            .context()
            .home_dir
            .join("other-profiles")
            .display()
            .to_string(),
    );
    assert_eq!(
        runtime.app_destination(&relocated, module).unwrap(),
        runtime.app_destination(surge, module).unwrap()
    );
    let content =
        String::from_utf8_lossy(&runtime.presets().files()["app/surge/shine-general.sgmodule"]);
    assert!(content.contains("#!name=shine-general"));
    assert!(
        content
            .lines()
            .all(|line| { line.trim().is_empty() || line.starts_with('#') || line == "[General]" })
    );
    let metadata = String::from_utf8_lossy(&runtime.presets().files()["app/surge/shine.toml"]);
    assert!(!metadata.contains("\"module\", \"enable\""));
    assert_eq!(
        surge.artifact.as_ref().unwrap().teardown.as_deref(),
        Some("unbuild.ts")
    );
}

fn embedded_destination_runtime(
    home: &Path,
) -> shine_core::runtime::CoreRuntime<shine_core::runtime::InMemoryHost> {
    use shine_core::runtime::{CoreRuntime, InMemoryHost, RuntimeContext, RuntimePlatform};
    let mut context = RuntimeContext::isolated(
        home.to_path_buf(),
        home.join(".shine"),
        home.join("presets"),
        home.join("bin"),
        RuntimePlatform::current(),
    );
    // The captured data directory is independent of HOME and ambient BaseDirs.
    context.data_dir = std::env::temp_dir().join("shine-captured-app-data");
    CoreRuntime::new(
        InMemoryHost::new(),
        context,
        super::support::runtime(RuntimePlatform::current())
            .presets()
            .clone(),
    )
}

#[cfg(windows)]
#[test]
fn install_resolves_windows_docker_engine_destination_on_windows() {
    let dir = std::env::temp_dir().join("shine-apps-win-dest");
    let runtime = embedded_destination_runtime(&dir);
    let categories = runtime.app_categories(Some("docker-engine")).unwrap();
    let docker = categories
        .iter()
        .find(|c| c.name == "docker-engine")
        .unwrap();
    let file = docker.files.first().unwrap();

    let destination = runtime.app_destination(docker, file).unwrap();

    assert_eq!(destination, dir.join(".docker").join("daemon.json"));
}

#[cfg(unix)]
#[test]
fn install_accepts_unix_metadata_destination_on_unix() {
    let dir = std::env::temp_dir().join("shine-apps-unix-dest");
    let runtime = embedded_destination_runtime(&dir);
    let categories = runtime.app_categories(Some("docker-engine")).unwrap();
    let docker = categories
        .iter()
        .find(|c| c.name == "docker-engine")
        .unwrap();
    let file = docker.files.first().unwrap();

    let destination = runtime.app_destination(docker, file).unwrap();

    assert_eq!(
        destination,
        PathBuf::from("/etc/docker").join("daemon.json")
    );
}

#[test]
fn per_file_destination_overrides_category_root() {
    let dir = std::env::temp_dir().join("shine-apps-file-dest");
    let runtime = embedded_destination_runtime(&dir);
    let categories = runtime.app_categories(Some("clash-verge")).unwrap();
    let clash = categories.first().unwrap();
    let merge = clash
        .files
        .iter()
        .find(|file| file.source_rel == Path::new("merge.yaml"))
        .unwrap();
    let local_rule = clash
        .files
        .iter()
        .find(|file| file.source_rel == Path::new("rules/lan.list"))
        .unwrap();

    assert_eq!(
        runtime.app_destination(clash, merge).unwrap(),
        dir.join(".shine/clash-verge/merge.yaml")
    );
    assert_eq!(
        runtime.app_destination(clash, local_rule).unwrap(),
        runtime
            .context()
            .data_dir
            .join("io.github.clash-verge-rev.clash-verge-rev")
            .join("ruleset/shine-source/lan.list")
    );
}
