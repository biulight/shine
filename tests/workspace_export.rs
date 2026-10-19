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
fn exported_values_round_trip_through_node_and_bun_dotenv_readers() {
    let fixture = Fixture::new();
    let values = std::collections::BTreeMap::from([
        ("PLAIN", "plain"),
        ("WINDOWS_PATH", r"C:\new\tools\bin"),
        ("DOUBLE_QUOTE", "say \"hello\""),
        ("SINGLE_QUOTE", "it's fine"),
        ("BOTH_QUOTES", "it's \"fine\" with a \\path"),
        ("MULTILINE", "first\nsecond"),
        ("BACKTICK", "a `tick` and a \"quote\""),
        ("EMPTY", ""),
        ("WHITESPACE", "  # keep these spaces  "),
    ]);
    let source = toml::to_string(&serde_json::json!({"version": 1, "plain": values})).unwrap();
    fs::write(fixture.0.join("source.toml"), source).unwrap();
    assert_export_ok(&fixture.command().output().unwrap());
    let keys = serde_json::to_string(&values.keys().collect::<Vec<_>>()).unwrap();
    let scripts = [
        (
            "node",
            vec![
                "-e".to_owned(),
                format!(
                    "const v = require('node:util').parseEnv(require('node:fs').readFileSync('output.env','utf8')); console.log(JSON.stringify(Object.fromEntries({keys}.map(k=>[k,v[k]]))))"
                ),
            ],
        ),
        (
            "bun",
            vec![
                "--env-file=output.env".to_owned(),
                "-e".to_owned(),
                format!(
                    "console.log(JSON.stringify(Object.fromEntries({keys}.map(k=>[k,process.env[k]]))))"
                ),
            ],
        ),
    ];
    for (reader, args) in scripts {
        if Command::new(reader).arg("--version").output().is_err() {
            eprintln!("skipping {reader} dotenv conformance: runtime is unavailable");
            continue;
        }
        let mut command = Command::new(reader);
        command.current_dir(&fixture.0).args(args);
        for key in values.keys() {
            command.env_remove(key);
        }
        let output = command.output().unwrap();
        assert!(output.status.success(), "{reader}: {output:?}");
        let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(parsed, serde_json::to_value(&values).unwrap(), "{reader}");
    }
}

#[test]
fn unrepresentable_export_values_fail_before_creating_or_replacing_output() {
    let fixture = Fixture::new();
    for value in [
        "$BASE/api",
        "cost $5",
        "before\rafter",
        "all '\"` delimiters",
    ] {
        let source =
            toml::to_string(&serde_json::json!({"version": 1, "plain": {"TOKEN": value}})).unwrap();
        fs::write(fixture.0.join("source.toml"), source).unwrap();
        let output = fixture.command().output().unwrap();
        assert!(!output.status.success(), "{output:?}");
        assert!(!fixture.0.join("output.env").exists());
        fs::write(fixture.0.join("output.env"), b"original").unwrap();
        let output = fixture.command().arg("--force").output().unwrap();
        assert!(!output.status.success(), "{output:?}");
        assert!(!String::from_utf8_lossy(&output.stderr).contains(value));
        assert_eq!(fixture.output(), b"original");
        fixture.assert_no_staging();
        fs::remove_file(fixture.0.join("output.env")).unwrap();
    }
}

#[test]
fn dotenv_import_preserves_backtick_quoted_values() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.0.join("shine.workspace.toml")).unwrap();
    let dotenv = "TOKEN=`hello#world`\nQUOTES=`it's \"quoted\"` # comment\nPATH_VALUE=`C:\\new\\tools`\nEMPTY=``\nSPACES=`  keep  `\n";
    fs::write(fixture.0.join(".env"), dotenv).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_shine"))
        .current_dir(&fixture.0)
        .env("SHINE_CONFIG_DIR", fixture.0.join("state"))
        .env_remove("SHINE_PRESETS")
        .args(["env", "workspace", "init", "--from-dotenv", "--mode", "dev"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let source: toml::Value =
        toml::from_str(&fs::read_to_string(fixture.0.join(".env.shine.toml")).unwrap()).unwrap();
    for (key, expected) in [
        ("TOKEN", "hello#world"),
        ("QUOTES", "it's \"quoted\""),
        ("PATH_VALUE", r"C:\new\tools"),
        ("EMPTY", ""),
        ("SPACES", "  keep  "),
    ] {
        assert_eq!(source["plain"][key].as_str(), Some(expected), "{key}");
    }
    assert_eq!(fs::read_to_string(fixture.0.join(".env")).unwrap(), dotenv);
}

#[test]
fn dotenv_import_rejects_invalid_backtick_values_before_publication() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.0.join("shine.workspace.toml")).unwrap();
    for value in [
        "`unterminated",
        "`closed` trailing",
        "`$BASE/api`",
        "`first\nsecond`",
    ] {
        fs::write(
            fixture.0.join(".env"),
            format!("VALID=first\nTOKEN={value}\n"),
        )
        .unwrap();
        for dry_run in [false, true] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_shine"));
            command
                .current_dir(&fixture.0)
                .env("SHINE_CONFIG_DIR", fixture.0.join("state"))
                .env_remove("SHINE_PRESETS")
                .args(["env", "workspace", "init", "--from-dotenv", "--mode", "dev"]);
            if dry_run {
                command.arg("--dry-run");
            }
            let output = command.output().unwrap();
            assert!(!output.status.success(), "{value}: {output:?}");
            assert!(!fixture.0.join("shine.workspace.toml").exists());
            assert!(!fixture.0.join(".env.shine.toml").exists());
        }
    }
}

#[test]
fn dotenv_import_rejects_bare_variable_references_without_publishing_files() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.0.join("shine.workspace.toml")).unwrap();
    for value in ["$BASE/api", "\"$BASE/api\"", "'$BASE/api'", "${BASE}/api"] {
        fs::write(
            fixture.0.join(".env"),
            format!("BASE=prefix\nURL={value}\n"),
        )
        .unwrap();
        for dry_run in [false, true] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_shine"));
            command
                .current_dir(&fixture.0)
                .env("SHINE_CONFIG_DIR", fixture.0.join("state"))
                .env_remove("SHINE_PRESETS")
                .args(["env", "workspace", "init", "--from-dotenv", "--mode", "dev"]);
            if dry_run {
                command.arg("--dry-run");
            }
            let output = command.output().unwrap();
            assert!(!output.status.success(), "{output:?}");
            assert!(String::from_utf8_lossy(&output.stderr).contains("dotenv interpolation"));
            assert!(!fixture.0.join("shine.workspace.toml").exists());
            assert!(!fixture.0.join(".env.shine.toml").exists());
        }
    }
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

#[test]
fn dotenv_export_quotes_display_paths_without_changing_publication() {
    let fixture = Fixture::new();
    let home = fixture.0.join("home");
    fs::create_dir_all(home.join("AppData/Roaming")).unwrap();
    fs::create_dir_all(home.join("AppData/Local")).unwrap();
    let mut names = vec!["normal space's.env"];
    if cfg!(unix) {
        names.push("out\nline\ttab\u{1b}escape.env");
    }
    for name in names {
        let destination = fixture.0.join(name);
        let quoted = format!("{:?}", destination.to_string_lossy());
        let command = || {
            let mut command = Command::new(env!("CARGO_BIN_EXE_shine"));
            command
                .current_dir(&fixture.0)
                .env("SHINE_CONFIG_DIR", fixture.0.join("state"))
                .env("SHINE_PRESETS", fixture.0.join("state/presets"))
                .env("HOME", &home)
                .env("USERPROFILE", &home)
                .env("APPDATA", home.join("AppData/Roaming"))
                .env("LOCALAPPDATA", home.join("AppData/Local"))
                .env("NO_COLOR", "1")
                .stdin(Stdio::null())
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
                .arg(&destination);
            command
        };
        let preview = command()
            .args(["--dry-run", "--include-secrets"])
            .output()
            .unwrap();
        assert!(preview.status.success(), "{preview:?}");
        assert!(preview.stderr.is_empty(), "{preview:?}");
        assert_eq!(
            String::from_utf8(preview.stdout).unwrap(),
            format!(
                "Would export 1 non-secret variable(s) for mode dev to {quoted}. Secrets were not decrypted or counted; --include-secrets will be applied during actual export.\n"
            )
        );
        assert!(!destination.exists());
        for force in [false, true] {
            if force {
                fs::write(&destination, b"old\n").unwrap();
            }
            let mut export = command();
            if force {
                export.arg("--force");
            }
            let output = export.output().unwrap();
            assert!(output.status.success(), "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
            assert_eq!(
                String::from_utf8(output.stdout).unwrap(),
                format!("Exported 1 variables for mode dev to {quoted}\n")
            );
            assert_eq!(fs::read(&destination).unwrap(), b"PUBLIC=\"visible\"\n");
        }
        for dry_run in [false, true] {
            let mut blocked = command();
            if dry_run {
                blocked.arg("--dry-run");
            }
            let output = blocked.output().unwrap();
            assert_eq!(output.status.code(), Some(1), "{output:?}");
            assert!(output.stdout.is_empty(), "{output:?}");
            assert!(
                String::from_utf8(output.stderr)
                    .unwrap()
                    .contains(&format!("{quoted} already exists; rerun with --force"))
            );
            assert_eq!(fs::read(&destination).unwrap(), b"PUBLIC=\"visible\"\n");
        }
    }
    fixture.assert_no_staging();
}
