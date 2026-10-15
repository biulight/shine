//! Process-level routing and consent contracts, independent of module layout.
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("shine-dispatch-{}", uuid::Uuid::new_v4()));
        for path in ["home", "home/AppData/Roaming", "home/AppData/Local"] {
            fs::create_dir_all(root.join(path)).unwrap();
        }
        Self(root)
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_shine"));
        command
            .current_dir(&self.0)
            .env("HOME", self.0.join("home"))
            .env("USERPROFILE", self.0.join("home"))
            .env("APPDATA", self.0.join("home/AppData/Roaming"))
            .env("LOCALAPPDATA", self.0.join("home/AppData/Local"))
            .env("SHINE_CONFIG_DIR", self.0.join("state"))
            .env_remove("SHINE_PRESETS")
            .stdin(Stdio::null());
        command
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn schema_and_invalid_validation_do_not_initialize_state() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .args(["preset", "schema", "--format", "json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap();
    assert!(!fixture.0.join("state").exists());
    fs::create_dir_all(fixture.0.join("app/broken")).unwrap();
    fs::write(fixture.0.join("app/broken/shine.toml"), "invalid = [").unwrap();
    let output = fixture
        .command()
        .args(["preset", "validate", "app/broken", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap();
    assert!(!fixture.0.join("state").exists());
}

#[test]
fn app_mutation_requires_consent_and_preserves_modified_files() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let preset = state.join("presets/app/demo");
    let destination = fixture.0.join("destination");
    fs::create_dir_all(&preset).unwrap();
    fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
    let destination_toml =
        toml::Value::String(destination.to_string_lossy().into_owned()).to_string();
    fs::write(preset.join("shine.toml"), format!("metadata_schema_version = 2\ndest = {destination_toml}\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.txt'\n")).unwrap();
    fs::write(preset.join("config.txt"), "managed").unwrap();
    // An explicit external tree makes this fixture independent of embedded presets.
    let run = |args: &[&str]| {
        fixture
            .command()
            .env("SHINE_PRESETS", state.join("presets"))
            .args(args)
            .output()
            .unwrap()
    };
    let rejected = run(&["app", "install", "demo"]);
    assert!(!rejected.status.success(), "{rejected:?}");
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("approval requires"),
        "{rejected:?}"
    );
    assert!(!destination.exists());
    assert!(!state.join("app-manifest.toml").exists());
    let installed = run(&["app", "install", "demo", "--yes"]);
    assert!(installed.status.success(), "{installed:?}");
    assert_eq!(
        fs::read(destination.join("config.txt")).unwrap(),
        b"managed"
    );
    let receipt = fs::read(state.join("app-manifest.toml")).unwrap();
    fs::write(destination.join("config.txt"), "user edit").unwrap();
    let removed = run(&["app", "uninstall", "demo", "--yes"]);
    assert_eq!(removed.status.code(), Some(1), "{removed:?}");
    assert!(String::from_utf8_lossy(&removed.stdout).contains("Uninstall incomplete"));
    assert_eq!(fs::read(state.join("app-manifest.toml")).unwrap(), receipt);
    assert_eq!(
        fs::read(destination.join("config.txt")).unwrap(),
        b"user edit"
    );
}

#[test]
fn top_level_uninstall_uses_receipts_after_app_source_removal() {
    for target in ["app/retired", "retired"] {
        let fixture = Fixture::new();
        let state = fixture.0.join("state");
        let preset = state.join("presets/app/retired");
        let destination = fixture.0.join("destination");
        fs::create_dir_all(&preset).unwrap();
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join("config.txt"), "original").unwrap();
        fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
        let dest = toml::Value::String(destination.to_string_lossy().into_owned()).to_string();
        fs::write(preset.join("shine.toml"), format!("metadata_schema_version = 2\ndest = {dest}\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.txt'\n")).unwrap();
        fs::write(preset.join("config.txt"), "managed").unwrap();
        let run = |args: &[&str]| {
            fixture
                .command()
                .env("SHINE_PRESETS", state.join("presets"))
                .args(args)
                .output()
                .unwrap()
        };
        let installed = run(&["install", "app/retired", "--yes"]);
        assert!(installed.status.success(), "{installed:?}");
        fs::remove_dir_all(&preset).unwrap();
        let receipt = fs::read(state.join("app-manifest.toml")).unwrap();
        let preview = run(&["uninstall", target, "--dry-run"]);
        assert!(preview.status.success(), "{preview:?}");
        assert_eq!(
            fs::read(destination.join("config.txt")).unwrap(),
            b"managed"
        );
        assert_eq!(fs::read(state.join("app-manifest.toml")).unwrap(), receipt);
        let unapproved = run(&["uninstall", target]);
        assert!(!unapproved.status.success(), "{unapproved:?}");
        assert!(String::from_utf8_lossy(&unapproved.stderr).contains("approval requires"));
        fs::write(destination.join("config.txt"), "user edit").unwrap();
        let protected = run(&["uninstall", target, "--yes"]);
        assert!(!protected.status.success(), "{protected:?}");
        assert_eq!(
            fs::read(destination.join("config.txt")).unwrap(),
            b"user edit"
        );
        assert_eq!(fs::read(state.join("app-manifest.toml")).unwrap(), receipt);
        // A same-name Shell source makes the shorthand ambiguous even though
        // the App source is gone. Explicit App selection must still work.
        let shell = state.join("presets/shell/retired");
        fs::create_dir_all(&shell).unwrap();
        let script = if cfg!(windows) { "tool.ps1" } else { "tool.sh" };
        fs::write(shell.join("shine.toml"), format!("[[files]]\nsource = '{script}'\ntarget = 'tool'\n[files.permissions]\nschema_version = 1\n")).unwrap();
        fs::write(shell.join(script), "# fixture; never executed\n").unwrap();
        let ambiguous = run(&["uninstall", "retired", "--yes", "--force"]);
        assert!(!ambiguous.status.success(), "{ambiguous:?}");
        assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("ambiguous"));
        assert_eq!(fs::read(state.join("app-manifest.toml")).unwrap(), receipt);
        if target == "retired" {
            fs::remove_dir_all(&shell).unwrap();
        }
        let removed = run(&["uninstall", target, "--yes", "--force"]);
        assert!(removed.status.success(), "{removed:?}");
        assert_eq!(
            fs::read(destination.join("config.txt")).unwrap(),
            b"original"
        );
        let manifest: shine_core::install::AppManifest =
            toml::from_str(&fs::read_to_string(state.join("app-manifest.toml")).unwrap()).unwrap();
        assert!(manifest.entries.is_empty());
    }
}

#[cfg(windows)]
#[test]
fn proxy_executable_suffixes_reject_before_target_lookup_or_state_writes() {
    let fixture = Fixture::new();
    for name in [
        "missing.exe",
        "missing.EXE",
        "missing.com",
        "missing.bat",
        "missing.cmd",
        "missing.ps1",
    ] {
        let output = fixture
            .command()
            .args(["env", "proxy", "install", name, "--with", "TOKEN"])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("must omit executable extensions"),
            "{output:?}"
        );
        assert!(stderr.contains("use `missing`"), "{output:?}");
        assert!(!fixture.0.join("state").exists());
    }
}
