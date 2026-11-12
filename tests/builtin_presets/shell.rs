#[cfg(unix)]
use std::path::PathBuf;

#[test]
fn embedded_shell_metadata_returns_proxy_and_utils() {
    let cats = super::support::runtime(shine_core::runtime::RuntimePlatform::Linux)
        .shell_categories(None)
        .unwrap();
    let names: Vec<&str> = cats.iter().map(|c| c.name.as_str()).collect();
    assert!(
        names.contains(&"proxy"),
        "proxy category missing: {names:?}"
    );
    assert!(
        names.contains(&"utils"),
        "utils category missing: {names:?}"
    );
}

#[test]
fn agent_provider_config_is_embedded_auxiliary_data_on_every_platform() {
    use shine_core::runtime::RuntimePlatform;
    for platform in [
        RuntimePlatform::Macos,
        RuntimePlatform::Linux,
        RuntimePlatform::Windows,
    ] {
        let runtime = super::support::runtime(platform);
        let categories = runtime.shell_categories(Some("agent")).unwrap();
        let agent = categories.first().unwrap();
        assert_eq!(agent.files.len(), 1);
        assert_eq!(agent.files[0].command_name, "ccenv");
        assert_eq!(agent.files[0].source_rel, std::path::Path::new("cc.ts"));
        assert_eq!(
            runtime
                .presets()
                .get("shell/agent/cc.providers.toml")
                .unwrap(),
            include_bytes!("../../presets/shell/agent/cc.providers.toml")
        );
    }
}

#[test]
fn embedded_shell_metadata_proxy_scripts_have_descriptions() {
    let cats = super::support::runtime(shine_core::runtime::RuntimePlatform::Linux)
        .shell_categories(None)
        .unwrap();
    let proxy = cats.iter().find(|c| c.name == "proxy").unwrap();
    assert!(!proxy.files.is_empty());
    for script in &proxy.files {
        assert!(
            !script.description.is_empty(),
            "{} should have a description",
            script.source_rel.display()
        );
    }
}

#[cfg(unix)]
#[test]
fn proxy_scripts_fail_fast_when_not_sourced() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let preset_dir = manifest_dir.join("presets/shell/proxy");

    for script in ["set_proxy.sh", "uset_proxy.sh"] {
        let output = std::process::Command::new("bash")
            .arg(preset_dir.join(script))
            .output()
            .expect("proxy script should run under bash");

        assert!(
            !output.status.success(),
            "{script} should fail when executed directly"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("must be sourced"),
            "{script} should explain source requirement: {stderr}"
        );
    }
}
