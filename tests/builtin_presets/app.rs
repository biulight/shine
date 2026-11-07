use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;

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
