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
