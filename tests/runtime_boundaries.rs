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

#[test]
fn gpg_uses_stdout_for_sealing_and_decryption_despite_configured_output() {
    let fixture = Fixture::new();
    let gpg = fixture.0.join("tools/gpg");
    // Emulate GPG's output option: a local option file redirects output unless
    // the invocation explicitly overrides it. No actual cryptography is needed here.
    fs::write(
        &gpg,
        r#"#!/bin/sh
mode=$1
output=$REVIEW_GPG_OUTPUT
while [ "$#" -gt 0 ]; do
    case "$1" in
        --output) shift; output=$1 ;;
    esac
    last=$1
    shift
done
if [ "$output" != - ]; then exec > "$output"; fi
if [ "$mode" = --decrypt ]; then cat "$last"; else cat; fi
"#,
    )
    .unwrap();
    fs::set_permissions(gpg, fs::Permissions::from_mode(0o755)).unwrap();
    let source = fixture.0.join("project/source.toml");
    fs::write(&source, "version = 1\n[secret]\nTOKEN = 'pending-value'\n").unwrap();
    let redirected = fixture.0.join("redirected-output");
    Fixture::assert_ok(
        fixture
            .command(Path::new(env!("CARGO_BIN_EXE_shine")))
            .env("REVIEW_GPG_OUTPUT", &redirected)
            .arg("--config-dir")
            .arg(fixture.state())
            .args(["env", "secret", "seal"])
            .arg(&source)
            .args(["--backend", "gpg", "-r", "review"])
            .output()
            .unwrap(),
    );
    let table: toml::Value = toml::from_str(&fs::read_to_string(source).unwrap()).unwrap();
    assert_eq!(table["secret"]["TOKEN"].as_bool(), Some(true));
    let ciphertext = table["payload"]["data"].as_str().unwrap();
    assert!(!ciphertext.is_empty());
    assert!(!redirected.exists());

    fs::write(
        fixture.state().join("config.toml"),
        format!("schema_version = 2\n[env]\nCIPHER = '{ciphertext}'\n"),
    )
    .unwrap();
    let output = Fixture::assert_ok(
        fixture
            .command(Path::new(env!("CARGO_BIN_EXE_shine")))
            .env("REVIEW_GPG_OUTPUT", &redirected)
            .arg("--config-dir")
            .arg(fixture.state())
            .args(["env", "secret", "decrypt", "CIPHER"])
            .output()
            .unwrap(),
    );
    let plaintext: toml::Value =
        toml::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(plaintext["values"]["TOKEN"].as_str(), Some("pending-value"));
    assert!(!redirected.exists());
}

#[test]
fn empty_gpg_ciphertext_preserves_pending_source() {
    let fixture = Fixture::new();
    let gpg = fixture.0.join("tools/gpg");
    fs::write(&gpg, "#!/bin/sh\ncat >/dev/null\nexit 0\n").unwrap();
    fs::set_permissions(gpg, fs::Permissions::from_mode(0o755)).unwrap();
    let source = fixture.0.join("project/source.toml");
    let original = b"version = 1\n[secret]\nTOKEN = 'pending-value'\n";
    fs::write(&source, original).unwrap();
    let output = fixture
        .command(Path::new(env!("CARGO_BIN_EXE_shine")))
        .arg("--config-dir")
        .arg(fixture.state())
        .args(["env", "secret", "seal"])
        .arg(&source)
        .args(["--backend", "gpg", "-r", "review"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("no ciphertext"),
        "{output:?}"
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("pending-value"));
    assert_eq!(fs::read(source).unwrap(), original);
}

#[test]
fn concurrent_task_saves_preserve_every_successful_update() {
    let fixture = Fixture::new();
    // Initialize once so this test isolates task persistence from config bootstrapping.
    Fixture::assert_ok(
        fixture
            .command(Path::new(env!("CARGO_BIN_EXE_shine")))
            .arg("--config-dir")
            .arg(fixture.state())
            .args(["env", "get", "REVIEW_TOKEN"])
            .output()
            .unwrap(),
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    fs::write(
        fixture.state().join("update-check.json"),
        format!(
            r#"{{"latest_version":"{}","checked_at_unix_secs":{now}}}"#,
            env!("CARGO_PKG_VERSION")
        ),
    )
    .unwrap();
    let mut children = Vec::new();
    for index in 0..32 {
        children.push(
            fixture
                .command(Path::new(env!("CARGO_BIN_EXE_shine")))
                .arg("--config-dir")
                .arg(fixture.state())
                .args([
                    "task",
                    "save",
                    &format!("task-{index}"),
                    "--",
                    "echo",
                    &index.to_string(),
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    for child in children {
        Fixture::assert_ok(child.wait_with_output().unwrap());
    }
    let table: toml::Value =
        toml::from_str(&fs::read_to_string(fixture.state().join("tasks.toml")).unwrap()).unwrap();
    assert_eq!(table["tasks"].as_table().unwrap().len(), 32);
    for index in 0..32 {
        assert_eq!(
            table["tasks"][format!("task-{index}")]["command"][1].as_str(),
            Some(index.to_string().as_str())
        );
    }
}
