//! Export publication races through the CLI. The backend only returns fixture plaintext.
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command, Output, Stdio},
    sync::LazyLock,
    time::{Duration, Instant},
};

const BACKEND: &str = r#"
use std::{env, fs, time::{Duration, Instant}};
fn main() {
    fs::write(env::var_os("EXPORT_READY").unwrap(), b"ready").unwrap();
    let release = env::var_os("EXPORT_RELEASE").unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !std::path::Path::new(&release).exists() {
        assert!(Instant::now() < deadline, "export fixture was not released");
        std::thread::sleep(Duration::from_millis(10));
    }
    println!("version = 1\n[values]\nTOKEN = {:?}", env::var("EXPORT_VALUE").unwrap());
}
"#;

struct Backend(PathBuf);
impl Drop for Backend {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
static BACKEND_DIR: LazyLock<Backend> = LazyLock::new(|| {
    let root = std::env::temp_dir().join(format!("shine-export-backend-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("backend.rs"), BACKEND).unwrap();
    let output = Command::new("rustc")
        .args(["--edition=2024"])
        .arg(root.join("backend.rs"))
        .arg("-o")
        .arg(root.join(format!("gpg{}", std::env::consts::EXE_SUFFIX)))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    Backend(root)
});

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("shine-export-publication-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("state")).unwrap();
        fs::write(root.join("state/config.toml"), "schema_version = 2\n").unwrap();
        fs::write(
            root.join("shine.workspace.toml"),
            "version = 2\n[env]\nfiles = ['source.toml']\n",
        )
        .unwrap();
        fs::write(root.join("source.toml"), "version = 1\n[plain]\nPUBLIC = 'visible'\n[secret]\nTOKEN = true\n[payload]\ndata = 'eA=='\n").unwrap();
        Self(root)
    }

    fn command(&self) -> Command {
        let mut paths = vec![BACKEND_DIR.0.clone()];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        let mut command = Command::new(env!("CARGO_BIN_EXE_shine"));
        command
            .current_dir(&self.0)
            .env("SHINE_CONFIG_DIR", self.0.join("state"))
            .env_remove("SHINE_PRESETS")
            .env("PATH", std::env::join_paths(paths).unwrap())
            .args([
                "env",
                "workspace",
                "export",
                "--format",
                "dotenv",
                "--mode",
                "dev",
                "--output",
            ])
            .arg(self.0.join("output.env"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    fn start(&self, name: &str, force: bool) -> Child {
        let mut command = self.command();
        command
            .arg("--include-secrets")
            .env("EXPORT_VALUE", name)
            .env("EXPORT_READY", self.0.join(format!("ready-{name}")))
            .env("EXPORT_RELEASE", self.0.join("release"));
        if force {
            command.arg("--force");
        }
        command.spawn().unwrap()
    }

    fn ready(&self, name: &str, child: &mut Child) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !self.0.join(format!("ready-{name}")).exists() {
            assert!(
                child.try_wait().unwrap().is_none(),
                "export exited before decryption"
            );
            assert!(Instant::now() < deadline, "export did not start decryption");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn release(&self) {
        fs::write(self.0.join("release"), b"go").unwrap();
    }

    fn output(&self) -> Vec<u8> {
        fs::read(self.0.join("output.env")).unwrap()
    }

    fn assert_no_staging(&self) {
        assert!(fs::read_dir(&self.0).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".shine-write-")
        }));
    }

    #[cfg(unix)]
    fn assert_private(&self) {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(self.0.join("output.env"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn assert_export_ok(output: &Output) {
    assert!(output.status.success(), "{output:?}");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("TOKEN="));
}

#[test]
fn export_preserves_a_destination_created_while_decrypting() {
    let fixture = Fixture::new();
    let mut child = fixture.start("late", false);
    fixture.ready("late", &mut child);
    fs::write(fixture.0.join("output.env"), b"user-created\n").unwrap();
    fixture.release();
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("--force"),
        "{output:?}"
    );
    assert_eq!(fixture.output(), b"user-created\n");
    fixture.assert_no_staging();
}

#[test]
fn concurrent_secret_exports_have_one_winner_with_private_complete_contents() {
    let fixture = Fixture::new();
    let mut first = fixture.start("first", false);
    let mut second = fixture.start("second", false);
    fixture.ready("first", &mut first);
    fixture.ready("second", &mut second);
    fixture.release();
    let first = first.wait_with_output().unwrap();
    let second = second.wait_with_output().unwrap();
    let (winner, loser, value) = match (first.status.success(), second.status.success()) {
        (true, false) => (&first, &second, "first"),
        (false, true) => (&second, &first, "second"),
        _ => panic!("exactly one export must succeed: {first:?} {second:?}"),
    };
    assert_export_ok(winner);
    assert!(String::from_utf8_lossy(&loser.stderr).contains("--force"));
    assert_eq!(
        fixture.output(),
        format!("PUBLIC=\"visible\"\nTOKEN=\"{value}\"\n").as_bytes()
    );
    #[cfg(unix)]
    fixture.assert_private();
    fixture.assert_no_staging();
}

#[test]
fn force_export_replaces_plain_and_secret_outputs_and_keeps_secrets_private() {
    let fixture = Fixture::new();
    fs::write(fixture.0.join("output.env"), b"old\n").unwrap();
    let output = fixture.command().arg("--force").output().unwrap();
    assert_export_ok(&output);
    assert_eq!(fixture.output(), b"PUBLIC=\"visible\"\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            fixture.0.join("output.env"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
    }
    let mut child = fixture.start("forced", true);
    fixture.ready("forced", &mut child);
    fixture.release();
    assert_export_ok(&child.wait_with_output().unwrap());
    assert_eq!(fixture.output(), b"PUBLIC=\"visible\"\nTOKEN=\"forced\"\n");
    #[cfg(unix)]
    fixture.assert_private();
    fixture.assert_no_staging();
}

#[cfg(unix)]
#[test]
fn export_without_force_preserves_a_dangling_destination_link() {
    let fixture = Fixture::new();
    let target = fixture.0.join("missing.env");
    std::os::unix::fs::symlink(&target, fixture.0.join("output.env")).unwrap();
    let output = fixture.command().output().unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("--force"));
    assert_eq!(fs::read_link(fixture.0.join("output.env")).unwrap(), target);
    assert!(!target.exists());
    fixture.assert_no_staging();
}
