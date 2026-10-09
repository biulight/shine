//! Real CLI regressions with isolated state and local executables only.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("shine-boundaries-{}", uuid::Uuid::new_v4()));
        for dir in [
            "tools",
            "state with 'quotes'",
            "other",
            "project",
            "fake-root-home",
        ] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        let state = root.join("state with 'quotes'");
        fs::write(
            state.join("config.toml"),
            "schema_version = 2\n[env]\nREVIEW_TOKEN = 'installed-value'\n",
        )
        .unwrap();
        fs::write(
            root.join("other/config.toml"),
            "schema_version = 2\n[env]\nREVIEW_TOKEN = 'wrong-value'\n",
        )
        .unwrap();
        fs::write(root.join("project/shine.config.toml"), "").unwrap();
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_shine"), root.join("tools/shine")).unwrap();
        let target = root.join("tools/probe");
        fs::write(
            &target,
            "#!/bin/sh\nprintf '%s' \"${REVIEW_TOKEN:-missing}\"\n",
        )
        .unwrap();
        fs::set_permissions(target, fs::Permissions::from_mode(0o755)).unwrap();
        Self(root)
    }
    fn state(&self) -> PathBuf {
        self.0.join("state with 'quotes'")
    }
    fn command(&self, program: &Path) -> Command {
        let mut command = Command::new(program);
        command
            .env_clear()
            .env(
                "PATH",
                std::env::join_paths([
                    self.0.join("tools"),
                    PathBuf::from("/usr/bin"),
                    PathBuf::from("/bin"),
                ])
                .unwrap(),
            )
            .env("HOME", self.0.join("fake-root-home"))
            .env("SHINE_CONFIG_DIR", self.0.join("other"))
            .current_dir(self.0.join("project"))
            .stdin(Stdio::null());
        command
    }
    fn assert_ok(output: Output) -> Output {
        assert!(output.status.success(), "{:?}", output);
        output
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn env_delete_rejects_inherited_values_and_preserves_both_config_layers() {
    let fixture = Fixture::new();
    // Complete ordinary default initialization before checking deletion's write boundary.
    Fixture::assert_ok(
        fixture
            .command(Path::new(env!("CARGO_BIN_EXE_shine")))
            .arg("--config-dir")
            .arg(fixture.state())
            .args(["env", "get", "REVIEW_TOKEN"])
            .output()
            .unwrap(),
    );
    let global = fixture.state().join("config.toml");
    let project = fixture.0.join("project/shine.config.toml");
    let original_global = fs::read(&global).unwrap();
    let original_project = fs::read(&project).unwrap();
    for force in [false, true] {
        let mut command = fixture.command(Path::new(env!("CARGO_BIN_EXE_shine")));
        command
            .arg("--config-dir")
            .arg(fixture.state())
            .args(["env", "delete", "REVIEW_TOKEN"]);
        if force {
            command.arg("--force");
        }
        let output = command.output().unwrap();
        assert!(!output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("has no local entry"),
            "{output:?}"
        );
        assert_eq!(fs::read(&global).unwrap(), original_global);
        assert_eq!(fs::read(&project).unwrap(), original_project);
    }
    let output = Fixture::assert_ok(
        fixture
            .command(Path::new(env!("CARGO_BIN_EXE_shine")))
            .arg("--config-dir")
            .arg(fixture.state())
            .args(["env", "get", "REVIEW_TOKEN"])
            .output()
            .unwrap(),
    );
    assert_eq!(output.stdout, b"installed-value\n");
}

#[test]
fn env_delete_removes_project_override_and_restores_global_value() {
    let fixture = Fixture::new();
    Fixture::assert_ok(
        fixture
            .command(Path::new(env!("CARGO_BIN_EXE_shine")))
            .arg("--config-dir")
            .arg(fixture.state())
            .args(["env", "get", "REVIEW_TOKEN"])
            .output()
            .unwrap(),
    );
    let global = fixture.state().join("config.toml");
    let project = fixture.0.join("project/shine.config.toml");
    let original_global = fs::read(&global).unwrap();
    fs::write(
        &project,
        "# keep this comment\n[env]\nREVIEW_TOKEN = 'project-value'\nOTHER = 'keep'\n",
    )
    .unwrap();
    Fixture::assert_ok(
        fixture
            .command(Path::new(env!("CARGO_BIN_EXE_shine")))
            .arg("--config-dir")
            .arg(fixture.state())
            .args(["env", "delete", "REVIEW_TOKEN"])
            .output()
            .unwrap(),
    );
    assert_eq!(fs::read(&global).unwrap(), original_global);
    let contents = fs::read_to_string(&project).unwrap();
    assert!(contents.contains("# keep this comment"));
    let table: toml::Table = toml::from_str(&contents).unwrap();
    assert!(
        !table["env"]
            .as_table()
            .unwrap()
            .contains_key("REVIEW_TOKEN")
    );
    assert_eq!(table["env"]["OTHER"].as_str(), Some("keep"));
    let output = Fixture::assert_ok(
        fixture
            .command(Path::new(env!("CARGO_BIN_EXE_shine")))
            .arg("--config-dir")
            .arg(fixture.state())
            .args(["env", "get", "REVIEW_TOKEN"])
            .output()
            .unwrap(),
    );
    assert_eq!(output.stdout, b"installed-value\n");
}

#[test]
fn proxy_uses_install_state_and_still_honors_project_rules() {
    let fixture = Fixture::new();
    Fixture::assert_ok(
        fixture
            .command(Path::new(env!("CARGO_BIN_EXE_shine")))
            .arg("--config-dir")
            .arg(fixture.state())
            .args(["env", "proxy", "install", "probe", "--with", "REVIEW_TOKEN"])
            .output()
            .unwrap(),
    );
    let launcher = fixture.state().join("bin/probe");
    let output = Fixture::assert_ok(fixture.command(&launcher).output().unwrap());
    assert_eq!(output.stdout, b"installed-value");
    fs::write(
        fixture.0.join("project/shine.config.toml"),
        "[[env_proxy]]\ncommand = 'probe'\nwith = ['REVIEW_TOKEN']\nenabled = false\n",
    )
    .unwrap();
    let output = Fixture::assert_ok(fixture.command(&launcher).output().unwrap());
    assert_eq!(output.stdout, b"missing");
}

#[test]
fn sudo_home_uses_system_account_instead_of_root_home() {
    let fixture = Fixture::new();
    let account = Fixture::assert_ok(Command::new("/usr/bin/id").arg("-un").output().unwrap());
    let account = std::str::from_utf8(&account.stdout).unwrap().trim();
    if account == "root" {
        return;
    } // sudo from root intentionally keeps root HOME.
    let expected = directories::UserDirs::new()
        .unwrap()
        .home_dir()
        .canonicalize()
        .unwrap();
    Fixture::assert_ok(
        fixture
            .command(Path::new(env!("CARGO_BIN_EXE_shine")))
            .env("SUDO_USER", account)
            .arg("--config-dir")
            .arg(fixture.state())
            .args([
                "task",
                "save",
                "home-probe",
                "--cwd",
                "~",
                "--",
                "/usr/bin/true",
            ])
            .output()
            .unwrap(),
    );
    let table: toml::Value =
        toml::from_str(&fs::read_to_string(fixture.state().join("tasks.toml")).unwrap()).unwrap();
    assert_eq!(
        Path::new(table["tasks"]["home-probe"]["cwd"].as_str().unwrap()),
        expected
    );
}
