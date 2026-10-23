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
        self.configure(Command::new(env!("CARGO_BIN_EXE_shine")))
    }
    fn configure(&self, mut command: Command) -> Command {
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

// These review outputs are small enough to drain after the child exits.
#[cfg(unix)]
fn output_with_terminal(
    mut command: Command,
    terminal_stdout: bool,
) -> (std::process::Output, Vec<u8>) {
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd};

    let (mut master, mut slave) = (-1, -1);
    // SAFETY: openpty initializes both descriptors; defaults are sufficient.
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        },
        0
    );
    for fd in [master, slave] {
        assert_eq!(
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) },
            0
        );
    }
    // SAFETY: each descriptor returned by openpty is owned exactly once.
    let mut master = unsafe { fs::File::from_raw_fd(master) };
    let slave = unsafe { fs::File::from_raw_fd(slave) };
    // Keep unread macOS PTY data alive until after the child exits.
    let keep_alive = slave.try_clone().unwrap();
    if terminal_stdout {
        command.stdout(Stdio::from(slave));
    } else {
        command.stderr(Stdio::from(slave));
    }
    let output = command.output().unwrap();
    drop(command);
    assert_eq!(
        unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) },
        0
    );
    let mut terminal = Vec::new();
    match master.read_to_end(&mut terminal) {
        Ok(_) => {}
        Err(error)
            if error.raw_os_error() == Some(libc::EIO)
                || error.kind() == std::io::ErrorKind::WouldBlock => {}
        Err(error) => panic!("reading test PTY: {error}"),
    }
    drop(keep_alive);
    (output, terminal)
}

fn display_path_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    let root = fixture.0.with_extension(if cfg!(unix) {
        "display\nFORGED=path\r\t\u{1b}\\literal中文"
    } else {
        "display中文"
    });
    fs::rename(&fixture.0, &root).unwrap();
    fixture.0 = root;
    fixture
}

#[test]
fn structured_env_migration_is_explicit_backup_first_and_idempotent() {
    let f = Fixture::new();
    let root = f.0.join("presets");
    let app = root.join("app/demo");
    let shell = root.join("shell/demo");
    fs::create_dir_all(&app).unwrap();
    fs::create_dir_all(&shell).unwrap();
    let app_metadata = "metadata_schema_version=2\ndest='~/.demo'\n[artifact]\nscript='build.ts'\nruntime='bun'\nenv=['TOKEN=ALIAS']\n[permissions]\nschema_version=1\nenvironment=[{name='TOKEN',sensitivity='secret'}]\n[[files]]\nsource='config'\n";
    let shell_metadata = "[[files]]\nsource='run.ts'\nruntime='bun'\nenv=['VALUE']\n[files.permissions]\nschema_version=1\nenvironment=[{name='VALUE',sensitivity='plain'}]\n";
    fs::write(app.join("shine.toml"), app_metadata).unwrap();
    fs::write(shell.join("shine.toml"), shell_metadata).unwrap();
    fs::write(app.join("config"), "unchanged").unwrap();
    fs::write(app.join("build.ts"), "console.log(process.env.ALIAS);").unwrap();
    fs::write(shell.join("run.ts"), "console.log(process.env.VALUE);").unwrap();

    let rejected = f
        .command()
        .args(["preset", "validate"])
        .arg(&root)
        .output()
        .unwrap();
    assert!(!rejected.status.success(), "{rejected:?}");
    let report = String::from_utf8(rejected.stdout).unwrap();
    assert!(
        report.contains("shine preset migrate --dry-run"),
        "{report}"
    );
    let preview = f
        .command()
        .args(["preset", "migrate"])
        .arg(&root)
        .args(["--dry-run", "--format", "json"])
        .output()
        .unwrap();
    assert!(preview.status.success(), "{preview:?}");
    assert!(!f.0.join("state").exists());
    assert_eq!(
        fs::read_to_string(app.join("shine.toml")).unwrap(),
        app_metadata
    );

    fs::create_dir_all(f.0.join("state")).unwrap();
    let active = f.0.join("state/presets");
    fs::rename(&root, &active).unwrap();
    let root = active;
    let app = root.join("app/demo");
    fs::write(
        f.0.join("state/config.toml"),
        "schema_version=2\n[env]\nVALUE='available'\n",
    )
    .unwrap();
    let unrelated = f
        .command()
        .env("SHINE_PRESETS", &root)
        .args(["env", "get", "VALUE"])
        .output()
        .unwrap();
    assert!(unrelated.status.success(), "{unrelated:?}");
    assert_eq!(
        fs::read_to_string(app.join("shine.toml")).unwrap(),
        app_metadata
    );

    for args in [["app", "list"], ["shell", "list"]] {
        let output = f
            .command()
            .env("SHINE_PRESETS", &root)
            .args(args)
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!output.status.success(), "{text}");
        assert!(text.contains("shine preset migrate --dry-run"), "{text}");
    }
    let applied = f
        .command()
        .args(["preset", "migrate"])
        .arg(&root)
        .args(["--yes", "--format", "json"])
        .output()
        .unwrap();
    assert!(applied.status.success(), "{applied:?}");
    let result: serde_json::Value = serde_json::from_slice(&applied.stdout).unwrap();
    assert_eq!(result["summary"]["changes"], 2);
    let backup =
        f.0.join("state")
            .join(result["backup_set"].as_str().unwrap());
    assert_eq!(
        fs::read_to_string(backup.join("app/demo/shine.toml")).unwrap(),
        app_metadata
    );
    assert_eq!(
        fs::read_to_string(backup.join("shell/demo/shine.toml")).unwrap(),
        shell_metadata
    );
    let valid = f
        .command()
        .args(["preset", "validate"])
        .arg(&root)
        .output()
        .unwrap();
    assert!(valid.status.success(), "{valid:?}");
    let repeated = f
        .command()
        .args(["preset", "migrate"])
        .arg(&root)
        .args(["--dry-run", "--format", "json"])
        .output()
        .unwrap();
    assert!(repeated.status.success(), "{repeated:?}");
    let result: serde_json::Value = serde_json::from_slice(&repeated.stdout).unwrap();
    assert_eq!(result["summary"]["changes"], 0);
    assert_eq!(fs::read_to_string(app.join("config")).unwrap(), "unchanged");
    assert!(!f.0.join("state/app-manifest.toml").exists());
}

#[cfg(unix)]
#[test]
fn migration_diff_colors_follow_stdout_and_leave_json_and_sources_unchanged() {
    let fixture = Fixture::new();
    let root = fixture.0.join("presets");
    let category = root.join("shell/demo");
    fs::create_dir_all(&category).unwrap();
    let original = "# control\tcomment\n[[files]]\nsource='run.ts'\nruntime='bun'\nenv=['TOKEN']\n[files.permissions]\nschema_version=1\nenvironment=[{name='TOKEN',sensitivity='secret'}]\n";
    fs::write(category.join("shine.toml"), original).unwrap();
    fs::write(category.join("run.ts"), "throw new Error('must not run');").unwrap();

    for (terminal_stdout, no_color, force_color, json) in [
        (true, false, None, false),
        (true, true, None, false),
        (false, false, None, false),
        (true, false, None, true),
        (false, true, Some("FORCE_COLOR"), false),
        (false, true, Some("CLICOLOR_FORCE"), false),
        (false, true, Some("FORCE_COLOR"), true),
    ] {
        let mut command = fixture.command();
        command
            .args(["preset", "migrate"])
            .arg(&root)
            .arg("--dry-run")
            .env("TERM", "xterm-256color")
            .env_remove("NO_COLOR")
            .env_remove("FORCE_COLOR")
            .env_remove("CLICOLOR")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("IGNORE_IS_TERMINAL");
        if no_color {
            command.env("NO_COLOR", "1");
        }
        if let Some(key) = force_color {
            command.env(key, "1");
        }
        if json {
            command.args(["--format", "json"]);
        }
        let (output, terminal) = output_with_terminal(command, terminal_stdout);
        assert!(output.status.success(), "{output:?}");
        let text = String::from_utf8_lossy(if terminal_stdout {
            &terminal
        } else {
            &output.stdout
        });
        if json {
            let report: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(report["summary"]["changes"], 1);
            assert!(!text.contains("\x1b["));
            assert!(!text.contains("--- a/"));
        } else {
            for prefix in [
                "--- a/shell/demo/shine.toml",
                "+++ b/shell/demo/shine.toml",
                "@@ ",
                "+env",
                "-env",
            ] {
                assert!(text.contains(prefix), "{text:?}");
            }
            let colored = force_color.is_some() || (terminal_stdout && !no_color);
            assert_eq!(text.contains("\x1b["), colored, "{text:?}");
            if colored {
                for prefix in [
                    "\x1b[36m--- a/",
                    "\x1b[36m+++ b/",
                    "\x1b[36m@@ ",
                    "\x1b[32m+env",
                    "\x1b[31m-env",
                ] {
                    assert!(text.contains(prefix), "{text:?}");
                }
            }
        }
        assert_eq!(
            fs::read_to_string(category.join("shine.toml")).unwrap(),
            original
        );
        assert!(!fixture.0.join("state").exists());
    }
}

#[test]
fn broker_lookup_errors_encode_names_without_repairing_or_creating_policies() {
    let f = Fixture::new();
    let state = f.0.join("state");
    fs::create_dir_all(&state).unwrap();
    fs::write(state.join("config.toml"), "schema_version=2\n").unwrap();
    let name = "missing\nFORGED\r\t\u{1b}";
    for args in [
        vec!["env", "broker", "policy", "info", name],
        vec!["env", "broker", "policy", "remove", name],
        vec![
            "env",
            "broker",
            "policy",
            "diff",
            name,
            "--workspace",
            "not-read.toml",
            "--mode",
            "dev",
            "--release-all-declared",
            "--",
            "not-executed",
        ],
    ] {
        let out = f
            .command()
            .env("NO_COLOR", "1")
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert!(out.stdout.is_empty());
        let text = String::from_utf8(out.stderr).unwrap();
        assert_eq!(
            text,
            "Error: policy missing\\nFORGED\\r\\t\\u{1b} does not exist\n"
        );
        assert!(!state.join("ssh-secret-broker.toml").exists());
    }
}

#[test]
fn broker_describe_encodes_source_rows_without_decrypting_or_rewriting_snapshot() {
    let f = Fixture::new();
    let source_name = if cfg!(unix) {
        "source\nFORGED\r\t\u{1b}中文.toml"
    } else {
        "source中文.toml"
    };
    let workspace = f.0.join("shine.workspace.toml");
    let manifest = format!(
        "version=2\n[env]\nfiles=[{}]\n",
        toml::Value::String(source_name.into())
    );
    let source = "version=1\n[secret]\nTOKEN=true\n[payload]\ndata='DESCRIBE_MUST_NOT_DECRYPT'\n";
    fs::write(&workspace, &manifest).unwrap();
    fs::write(f.0.join(source_name), source).unwrap();
    let out = f
        .command()
        .env("NO_COLOR", "1")
        .env_remove("SHINE_SSH_REMOTE_SOCK")
        .env_remove("SHINE_SSH_TOKEN")
        .args(["env", "broker", "describe", "--workspace"])
        .arg(&workspace)
        .args([
            "--mode",
            "dev",
            "--release",
            "TOKEN",
            "--",
            "never-execute-probe",
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(out.stderr.is_empty());
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(text.lines().count(), 5, "{text:?}");
    assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
    assert!(!text.contains("DESCRIBE_MUST_NOT_DECRYPT"));
    assert!(text.contains(&format!(
        "workspace_sha256 = \"{}\"",
        shine_core::digest::sha256_hex(manifest.as_bytes())
    )));
    assert!(text.lines().last().unwrap().ends_with(&format!(
        "{} [\"TOKEN\"]",
        shine_core::digest::sha256_hex(source.as_bytes())
    )));
    assert!(text.contains(r#"argv = ["never-execute-probe"]"#));
    if cfg!(unix) {
        assert!(text.contains(r"source source\nFORGED\r\t\u{1b}中文.toml "));
    }
    assert_eq!(fs::read_to_string(workspace).unwrap(), manifest);
    assert_eq!(fs::read_to_string(f.0.join(source_name)).unwrap(), source);
    assert!(!f.0.join("state/ssh-secret-broker.toml").exists());
}

#[test]
fn task_failure_diagnostics_encode_operands_without_changing_registry_or_exit() {
    let f = Fixture::new();
    let state = f.0.join("state");
    fs::create_dir_all(&state).unwrap();
    fs::write(state.join("config.toml"), "schema_version=2\n").unwrap();
    fs::write(state.join("update-check.json"), serde_json::to_vec(&serde_json::json!({
        "latest_version": env!("CARGO_PKG_VERSION"),
        "checked_at_unix_secs": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
    })).unwrap()).unwrap();
    let bad = f.0.join("missing\nFORGED\r\t\u{1b}");
    for cwd in [true, false] {
        let mut manifest = cli::task::manifest::TaskManifest::default();
        manifest.upsert(
            "demo",
            vec![if cwd {
                "unused-program".into()
            } else {
                bad.to_string_lossy().into_owned()
            }],
            if cwd { Some(bad.clone()) } else { None },
        );
        let original = toml::to_string(&manifest).unwrap();
        fs::write(state.join("tasks.toml"), &original).unwrap();
        let out = f
            .command()
            .env("NO_COLOR", "1")
            .args(["task", "run", "demo"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert!(out.stdout.is_empty());
        let stderr = String::from_utf8(out.stderr).unwrap();
        // The preceding command preview remains literal shell code; inspect only the failure diagnostic.
        let error = stderr.split_once("Error: ").unwrap().1;
        assert!(!error.contains(['\r', '\t', '\u{1b}']), "{stderr:?}");
        assert!(
            !error.lines().any(|line| line.starts_with("FORGED")),
            "{stderr:?}"
        );
        assert!(error.contains(r"missing\nFORGED\r\t\u{1b}"));
        assert_eq!(
            fs::read_to_string(state.join("tasks.toml")).unwrap(),
            original
        );
        let saved = f
            .command()
            .env("NO_COLOR", "1")
            .args(["task", "save", "new", "--cwd"])
            .arg(&bad)
            .args(["--", "unused-program"])
            .output()
            .unwrap();
        assert_eq!(saved.status.code(), Some(1), "{saved:?}");
        let error = String::from_utf8(saved.stderr).unwrap();
        assert!(!error.contains(['\r', '\t', '\u{1b}']), "{error:?}");
        assert!(
            !error.lines().any(|line| line.starts_with("FORGED")),
            "{error:?}"
        );
        assert_eq!(
            fs::read_to_string(state.join("tasks.toml")).unwrap(),
            original
        );
    }
}

#[test]
fn verbose_pull_non_repository_notice_encodes_selected_path() {
    let f = display_path_fixture();
    let state = f.0.join("state");
    fs::create_dir_all(state.join("presets")).unwrap();
    fs::write(state.join("config.toml"), "schema_version=2\n").unwrap();
    fs::write(state.join("update-check.json"), serde_json::to_vec(&serde_json::json!({
        "latest_version": env!("CARGO_PKG_VERSION"),
        "checked_at_unix_secs": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
    })).unwrap()).unwrap();
    let out = f
        .command()
        .env("NO_COLOR", "1")
        .args(["update", "--pull", "--verbose"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("not a Git repository:") && text.contains("Nothing to pull."));
    assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
    assert!(
        !text.lines().any(|line| line.starts_with("FORGED=")),
        "{text:?}"
    );
    assert!(!state.join("presets/.git").exists());
}

#[test]
fn sys_bootstrap_preview_encodes_data_without_executing_or_mutating_inputs() {
    let fixture = display_path_fixture();
    let state = fixture.0.join("state");
    let os = if cfg!(target_os = "linux") {
        fs::read_to_string("/etc/os-release")
            .unwrap()
            .lines()
            .find_map(|line| line.strip_prefix("ID="))
            .unwrap()
            .trim_matches('"')
            .to_lowercase()
    } else {
        std::env::consts::OS.to_string()
    };
    let preset = state.join("presets/sys").join(os);
    fs::create_dir_all(&preset).unwrap();
    let profile = "profile\nFORGED=profile\r\t\u{1b}";
    let metadata = format!(
        "version=2\ndefault_profile={}\n[[items]]\nid='probe'\nlabel='Probe'\npermissions={{schema_version=1}}\ndetect={{kind='path',path='$HOME/.missing-probe'}}\ninstall={{kind='script',path='probe.sh'}}\n[profiles.{}]\nitems=['probe']\n",
        toml::Value::String(profile.into()),
        serde_json::to_string(profile).unwrap()
    );
    fs::write(preset.join("shine.toml"), &metadata).unwrap();
    fs::write(
        preset.join("probe.sh"),
        "# Must never execute in this preview\nexit 99\n",
    )
    .unwrap();
    let config = "schema_version=2\n[env]\nPROXY_HOST=\"host\\nFORGED=proxy\\r\\t\\u001b\"\nPROXY_NO_PROXY=\"local\\nFORGED=no_proxy\\r\\t\\u001b\"\n";
    fs::write(state.join("config.toml"), config).unwrap();
    for explicit in [false, true] {
        let mut command = fixture.command();
        command
            .env("NO_COLOR", "1")
            .args(["sys", "bootstrap", "--dry-run", "--proxy"]);
        if explicit {
            command.args(["--preset", profile]);
        }
        let out = command.output().unwrap();
        assert!(out.status.success(), "{out:?}");
        assert!(out.stderr.is_empty(), "{out:?}");
        let text = String::from_utf8(out.stdout).unwrap();
        assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
        assert!(
            !text.lines().any(|line| line.starts_with("FORGED=")),
            "{text:?}"
        );
        for expected in [
            r"profile\nFORGED=profile\r\t\u{1b}",
            r"host\nFORGED=proxy\r\t\u{1b}",
            r"local\nFORGED=no_proxy\r\t\u{1b}",
            "requires scoped external-code trust",
        ] {
            assert!(text.contains(expected), "missing {expected}: {text:?}");
        }
        assert_eq!(
            fs::read_to_string(state.join("config.toml")).unwrap(),
            config
        );
        assert_eq!(
            fs::read_to_string(preset.join("shine.toml")).unwrap(),
            metadata
        );
        assert!(!state.join("sys-manifest.toml").exists());
        assert!(!state.join("trust.toml").exists());
        assert!(!fixture.0.join("home/.shine").exists());
    }
}

#[test]
fn app_lifecycle_display_preserves_raw_destinations_backups_and_exit_policy() {
    let f = Fixture::new();
    let state = f.0.join("state");
    let preset = state.join("presets/app/demo");
    let destination = f.0.join(if cfg!(unix) {
        "destination\nFORGED=path\r\t\u{1b}\\literal中文"
    } else {
        "destination中文"
    });
    fs::create_dir_all(&preset).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(state.join("config.toml"), "schema_version=2\n").unwrap();
    fs::write(state.join("update-check.json"), serde_json::to_vec(&serde_json::json!({
        "latest_version": env!("CARGO_PKG_VERSION"),
        "checked_at_unix_secs": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
    })).unwrap()).unwrap();
    let metadata = format!(
        "metadata_schema_version=2\ndest={}\n[[files]]\nsource='config.txt'\nrestart_hint=\"Restart\\nFORGED=hint\\r\\t\\u001b\"\n",
        toml::Value::String(destination.to_str().unwrap().into())
    );
    fs::write(preset.join("shine.toml"), &metadata).unwrap();
    fs::write(preset.join("config.txt"), b"one\n").unwrap();
    let target = destination.join("config.txt");
    let backup = destination.join("config.txt.shine.bak");
    let receipt = state.join("app-manifest.toml");
    fs::write(&target, b"original\n").unwrap();
    let run = |args: &[&str], status| {
        let output = f
            .command()
            .env("NO_COLOR", "1")
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(status), "{args:?}: {output:?}");
        for bytes in [&output.stdout, &output.stderr] {
            let text = String::from_utf8_lossy(bytes);
            assert!(!text.contains(['\r', '\t', '\u{1b}']), "{args:?}: {text:?}");
            assert!(
                !text.lines().any(|line| line.starts_with("FORGED=")),
                "{args:?}: {text:?}"
            );
        }
        String::from_utf8(output.stdout).unwrap()
    };
    let preview = run(&["app", "install", "demo", "--dry-run"], 0);
    assert!(preview.contains("1 would install"), "{preview}");
    assert_eq!(fs::read(&target).unwrap(), b"original\n");
    assert!(!receipt.exists() && !backup.exists());
    let installed = run(&["app", "install", "demo", "--yes"], 0);
    assert!(installed.contains(r"Restart\nFORGED=hint\r\t\u{1b}"));
    assert!(installed.contains("backups for") && installed.contains("1 backed up"));
    assert_eq!(fs::read(&target).unwrap(), b"one\n");
    assert_eq!(fs::read(&backup).unwrap(), b"original\n");
    let unchanged = run(&["app", "install", "demo", "--yes"], 0);
    assert!(unchanged.contains("already up to date"));
    for (bytes, args) in [
        (
            b"two\n".as_slice(),
            vec!["upgrade", "app/demo", "--yes", "--verbose"],
        ),
        (b"three\n".as_slice(), vec!["upgrade", "--yes"]),
    ] {
        fs::write(preset.join("config.txt"), bytes).unwrap();
        let updated = run(&args, 0);
        assert!(updated.contains(r"Restart\nFORGED=hint\r\t\u{1b}"));
        assert_eq!(fs::read(&target).unwrap(), bytes);
        assert_eq!(fs::read(&backup).unwrap(), b"original\n");
    }
    fs::write(&target, b"user modification\n").unwrap();
    let receipt_before = fs::read(&receipt).unwrap();
    run(&["app", "uninstall", "demo", "--dry-run"], 0);
    assert_eq!(fs::read(&receipt).unwrap(), receipt_before);
    let kept = run(&["app", "uninstall", "demo", "--yes"], 1);
    assert!(kept.contains("Uninstall incomplete") && kept.contains("protected"));
    assert_eq!(fs::read(&target).unwrap(), b"user modification\n");
    assert_eq!(fs::read(&receipt).unwrap(), receipt_before);
    let removed = run(&["app", "uninstall", "demo", "--force", "--yes"], 0);
    assert!(removed.contains("restored"));
    assert_eq!(fs::read(&target).unwrap(), b"original\n");
    assert!(!backup.exists());
    assert_eq!(
        fs::read_to_string(preset.join("shine.toml")).unwrap(),
        metadata
    );
    assert_eq!(fs::read(preset.join("config.txt")).unwrap(), b"three\n");
    let rejected = run(
        &[
            "upgrade",
            "app/missing\nFORGED=target\r\t\u{1b}",
            "--yes",
            "--verbose",
        ],
        1,
    );
    assert!(rejected.starts_with(r"Upgrading app/missing\nFORGED=target\r\t\u{1b}"));
}

#[test]
fn stored_task_names_encode_controls_align_columns_and_retain_lookup_identity() {
    let f = Fixture::new();
    let state = f.0.join("state");
    fs::create_dir_all(&state).unwrap();
    fs::write(state.join("config.toml"), "schema_version=2\n").unwrap();
    fs::write(state.join("update-check.json"),serde_json::to_vec(&serde_json::json!({
        "latest_version":env!("CARGO_PKG_VERSION"),"checked_at_unix_secs":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
    })).unwrap()).unwrap();
    let names = ["ascii", "任务", "e\u{301}", "old\nFORGED=name\r\t\u{1b}"];
    let mut manifest = cli::task::manifest::TaskManifest::default();
    for name in names {
        manifest.upsert(
            name,
            vec![env!("CARGO_BIN_EXE_shine").into(), "--version".into()],
            None,
        );
    }
    let original = toml::to_string(&manifest).unwrap();
    fs::write(state.join("tasks.toml"), &original).unwrap();
    let list = f
        .command()
        .env("NO_COLOR", "1")
        .args(["task", "list"])
        .output()
        .unwrap();
    assert!(list.status.success(), "{list:?}");
    let text = String::from_utf8(list.stdout).unwrap();
    assert_eq!(text.lines().count(), names.len() + 1, "{text:?}");
    assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
    let mut columns = Vec::new();
    for name in names {
        let info = f
            .command()
            .env("NO_COLOR", "1")
            .args(["task", "info", name])
            .output()
            .unwrap();
        assert!(info.status.success(), "{info:?}");
        let info = String::from_utf8(info.stdout).unwrap();
        assert_eq!(info.lines().count(), 2, "{info:?}");
        let display: String = name
            .chars()
            .flat_map(|ch| {
                if ch.is_control() {
                    ch.escape_debug().collect::<Vec<_>>()
                } else {
                    vec![ch]
                }
            })
            .collect();
        assert!(info.lines().next().unwrap().ends_with(&display), "{info:?}");
        let command = info
            .lines()
            .nth(1)
            .unwrap()
            .strip_prefix("Command")
            .unwrap()
            .trim_start();
        let row = text
            .lines()
            .find(|line| line.starts_with(&display))
            .unwrap();
        columns.push(console::measure_text_width(
            row.strip_suffix(command).unwrap(),
        ));
        let run = f
            .command()
            .env("NO_COLOR", "1")
            .args(["task", "run", name])
            .output()
            .unwrap();
        assert!(run.status.success(), "{run:?}");
        assert!(run.stdout.starts_with(b"shine "), "{run:?}");
        let error = String::from_utf8(run.stderr).unwrap();
        assert_eq!(error.lines().count(), 1, "{error:?}");
        assert!(!error.contains(['\r', '\t', '\u{1b}']), "{error:?}");
        assert_eq!(
            fs::read_to_string(state.join("tasks.toml")).unwrap(),
            original
        );
    }
    assert!(
        columns.iter().all(|column| *column == columns[0]),
        "{columns:?}"
    );
    let deleted = f
        .command()
        .env("NO_COLOR", "1")
        .args(["task", "delete", names[3]])
        .output()
        .unwrap();
    assert!(deleted.status.success(), "{deleted:?}");
    let text = String::from_utf8(deleted.stdout).unwrap();
    assert_eq!(text.lines().count(), 1, "{text:?}");
    assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
    let after: cli::task::manifest::TaskManifest =
        toml::from_str(&fs::read_to_string(state.join("tasks.toml")).unwrap()).unwrap();
    manifest.remove(names[3]);
    assert_eq!(after, manifest);
    for args in [
        vec!["task", "info", names[3]],
        vec!["task", "run", names[3]],
        vec!["task", "delete", names[3]],
        vec!["task", "save", names[3], "--", "never-start"],
    ] {
        let out = f.command().args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(!stderr.contains(['\r', '\t', '\u{1b}']), "{stderr:?}");
        assert!(
            !stderr.lines().any(|line| line.starts_with("FORGED=name")),
            "{stderr:?}"
        );
    }
}

#[cfg(windows)]
#[test]
fn task_command_displays_round_trip_through_native_powershell_without_changing_execution() {
    let f = Fixture::new();
    let state = f.0.join("state");
    fs::create_dir_all(&state).unwrap();
    fs::write(state.join("config.toml"), "schema_version=2\n").unwrap();
    fs::write(state.join("update-check.json"), serde_json::to_vec(&serde_json::json!({
        "latest_version":env!("CARGO_PKG_VERSION"),
        "checked_at_unix_secs":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
    })).unwrap()).unwrap();
    let program = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let saved = f
        .command()
        .args(["task", "save", "literal", "--"])
        .arg(&program)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[Console]::Out.Write('it''s:$literal:`tick'); exit 17",
        ])
        .output()
        .unwrap();
    assert!(saved.status.success(), "{saved:?}");
    let text = String::from_utf8(saved.stdout).unwrap();
    let display = text.lines().nth(1).unwrap();
    let before = fs::read(state.join("tasks.toml")).unwrap();
    for args in [vec!["task", "info", "literal"], vec!["task", "list"]] {
        let out = f.command().args(args).output().unwrap();
        assert!(out.status.success(), "{out:?}");
        assert!(String::from_utf8(out.stdout).unwrap().contains(display));
    }
    let direct = f
        .command()
        .args(["task", "run", "literal"])
        .output()
        .unwrap();
    assert_eq!(direct.status.code(), Some(17), "{direct:?}");
    assert_eq!(direct.stdout, b"it's:$literal:`tick");
    assert!(String::from_utf8(direct.stderr).unwrap().contains(display));
    let copied = Command::new(&program)
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        // PowerShell -Command otherwise maps a native nonzero status to 1.
        .arg(format!("{display}; exit $LASTEXITCODE"))
        .current_dir(&f.0)
        .output()
        .unwrap();
    assert_eq!(copied.status.code(), Some(17), "{display}: {copied:?}");
    assert_eq!(copied.stdout, direct.stdout, "{display}");
    assert!(copied.stderr.is_empty(), "{copied:?}");
    assert_eq!(fs::read(state.join("tasks.toml")).unwrap(), before);
}

#[test]
fn runtime_migration_notices_encode_paths_without_changing_preview_or_apply_scope() {
    for schema in [0, 2] {
        for preview in [true, false] {
            let f = display_path_fixture();
            let state = f.0.join("state");
            fs::create_dir_all(&state).unwrap();
            let config = format!("schema_version={schema}\ngpg_key_id='global-key'\n");
            let project = "gpg_key_id='project-key'\n";
            let workspace =
                "version=1\n[env]\nfiles=[]\n[env.encryption]\nrecipient='workspace-key'\n";
            for (path, contents) in [
                (state.join("config.toml"), config.as_str()),
                (f.0.join("shine.config.toml"), project),
                (f.0.join("shine.workspace.toml"), workspace),
            ] {
                fs::write(path, contents).unwrap();
            }
            fs::write(
                state.join("update-check.json"),
                b"preserved unless schema cleanup applies",
            )
            .unwrap();
            let mut command = f.command();
            command.env("NO_COLOR", "1").args(["state", "migrate"]);
            if preview {
                command.arg("--dry-run");
            }
            let out = command.output().unwrap();
            assert_eq!(out.status.code(), Some(0), "{out:?}");
            assert!(out.stderr.is_empty(), "{out:?}");
            let text = String::from_utf8(out.stdout).unwrap();
            assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
            assert!(
                !text.lines().any(|line| line.starts_with("FORGED=path")),
                "{text:?}"
            );
            assert!(
                text.contains(if cfg!(unix) {
                    "display\\nFORGED=path\\r\\t\\u{1b}\\literal中文"
                } else {
                    "display中文"
                }),
                "{text:?}"
            );
            if preview {
                assert_eq!(
                    fs::read_to_string(state.join("config.toml")).unwrap(),
                    config
                );
                assert_eq!(
                    fs::read_to_string(f.0.join("shine.config.toml")).unwrap(),
                    project
                );
                assert_eq!(
                    fs::read_to_string(f.0.join("shine.workspace.toml")).unwrap(),
                    workspace
                );
            } else {
                for (path, key) in [
                    (state.join("config.toml"), "global-key"),
                    (f.0.join("shine.config.toml"), "project-key"),
                    (f.0.join("shine.workspace.toml"), "workspace-key"),
                ] {
                    let content = fs::read_to_string(path).unwrap();
                    assert!(content.contains("gpg_recipients"), "{content}");
                    assert!(content.contains(key));
                }
            }
            assert_eq!(
                state.join("update-check.json").exists(),
                preview || schema == 2
            );
        }
    }
}

#[test]
fn task_registry_parse_failures_stop_reads_writes_and_execution_without_exposing_contents() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    fs::create_dir_all(&state).unwrap();
    fs::write(state.join("config.toml"), "schema_version=2\n").unwrap();
    fs::write(state.join("update-check.json"), serde_json::to_vec(&serde_json::json!({
        "latest_version": env!("CARGO_PKG_VERSION"),
        "checked_at_unix_secs": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
    })).unwrap()).unwrap();
    for contents in [
        "[tasks.demo]\ncommand='TASK_DOCUMENT_CANARY'\n",
        "[tasks.demo]\ncommand=['TASK_DOCUMENT_CANARY'] invalid\n",
    ] {
        fs::write(state.join("tasks.toml"), contents).unwrap();
        for args in [
            vec!["task", "list"],
            vec!["task", "info", "demo"],
            vec!["task", "run", "demo"],
            vec!["run", "demo"],
            vec!["task", "save", "new", "--", "never-start"],
            vec!["task", "delete", "demo"],
        ] {
            let out = fixture.command().args(&args).output().unwrap();
            assert_eq!(out.status.code(), Some(1), "{args:?}: {out:?}");
            assert!(out.stdout.is_empty(), "{args:?}: {out:?}");
            let error = String::from_utf8(out.stderr).unwrap();
            assert!(error.contains("failed to parse tasks.toml"), "{error}");
            assert!(!error.contains("TASK_DOCUMENT_CANARY"), "{error}");
            assert_eq!(
                fs::read_to_string(state.join("tasks.toml")).unwrap(),
                contents
            );
        }
    }
}

#[test]
fn completion_install_encodes_paths_preserving_profile_identity_and_idempotence() {
    for f in [Fixture::new(), display_path_fixture()] {
        let shell = if cfg!(windows) { "pwsh" } else { "/bin/bash" };
        let mut profile_bytes = None;
        for expected in ["updated", "up to date"] {
            let output = f
                .command()
                .env("SHELL", shell)
                .env("NO_COLOR", "1")
                .args(["completions", "install"])
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
            let text = String::from_utf8(output.stdout).unwrap();
            assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
            assert_eq!(text.lines().count(), 3, "{text:?}");
            if cfg!(unix) && f.0.extension().is_some() {
                assert!(!text.contains("run `"), "{text:?}");
            } else {
                assert!(text.contains("run `"), "{text:?}");
            }
            let rows: Vec<_> = text
                .lines()
                .filter(|line| {
                    line.starts_with("Shell Profile") || line.starts_with("Shell Config")
                })
                .collect();
            assert_eq!(rows.len(), 2, "{text:?}");
            for row in rows {
                assert!(row.contains(expected), "{row:?}");
                assert!(!row.contains(['\r', '\t', '\u{1b}']), "{row:?}");
                if cfg!(unix) && f.0.extension().is_some() {
                    assert!(
                        row.contains("display\\nFORGED=path\\r\\t\\u{1b}\\literal中文"),
                        "{row:?}"
                    );
                }
            }
            let profile = fs::read(f.0.join(if cfg!(windows) {
                "state/shell/profile.ps1"
            } else {
                "state/shell/profile.sh"
            }))
            .unwrap();
            let config_path = f.0.join(if cfg!(windows) {
                "home/Documents/PowerShell/Microsoft.PowerShell_profile.ps1"
            } else {
                "home/.bashrc"
            });
            let shell_config = fs::read_to_string(config_path).unwrap();
            if cfg!(unix) && f.0.extension().is_some() {
                assert!(
                    shell_config.contains("display\nFORGED=path\r\t\u{1b}\\literal中文"),
                    "{shell_config:?}"
                );
                assert!(
                    !shell_config.contains("display\\nFORGED=path"),
                    "{shell_config:?}"
                );
            }
            if let Some(previous) = &profile_bytes {
                assert_eq!(&profile, previous);
            }
            profile_bytes = Some(profile);
        }
    }
}

#[test]
fn managed_overlay_info_encodes_fields_and_reports_directory_presence() {
    let f = display_path_fixture();
    let state = f.0.join("state");
    fs::create_dir(&state).unwrap();
    let url = "https://example.invalid/repo\nFORGED=url\r\t\u{1b}\\literal中文";
    let branch = "branch\nFORGED=branch\u{1b}";
    let config = format!(
        "schema_version=2\npresets_overlay_git={}\npresets_overlay_git_branch={}\n",
        toml::Value::String(url.into()),
        toml::Value::String(branch.into())
    );
    fs::write(state.join("config.toml"), &config).unwrap();
    for present in [false, true] {
        if present {
            fs::create_dir(state.join("overlay")).unwrap();
        }
        let output = f
            .command()
            .env("NO_COLOR", "1")
            .env("PATH", "")
            .args(["preset", "overlay", "info"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(
            text.contains("repo\\nFORGED=url\\r\\t\\u{1b}\\literal中文"),
            "{text:?}"
        );
        assert!(text.contains("branch\\nFORGED=branch\\u{1b}"), "{text:?}");
        assert_eq!(text.lines().count(), 4, "{text:?}");
        assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
        if present {
            assert!(text.contains("Managed directory exists"), "{text:?}");
            assert!(!text.contains("Cloned"), "{text:?}");
        } else {
            assert!(text.contains("Not cloned yet"), "{text:?}");
            assert!(!state.join("overlay").exists());
        }
        assert_eq!(
            fs::read_to_string(state.join("config.toml")).unwrap(),
            config
        );
    }
    // No Git on PATH: saving the source still precedes a failed synchronization.
    let failed = f
        .command()
        .env("NO_COLOR", "1")
        .env("PATH", "")
        .args([
            "preset", "overlay", "link", "--git", url, "--branch", branch,
        ])
        .output()
        .unwrap();
    assert_eq!(failed.status.code(), Some(1), "{failed:?}");
    assert!(!failed.stderr.is_empty(), "{failed:?}");
    let text = String::from_utf8(failed.stdout).unwrap();
    assert_eq!(text.lines().count(), 3, "{text:?}");
    assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
    assert!(
        text.contains("repo\\nFORGED=url\\r\\t\\u{1b}\\literal中文"),
        "{text:?}"
    );
    let saved: toml::Value =
        toml::from_str(&fs::read_to_string(state.join("config.toml")).unwrap()).unwrap();
    assert_eq!(saved["presets_overlay_git"].as_str(), Some(url));
    assert_eq!(saved["presets_overlay_git_branch"].as_str(), Some(branch));
}

#[test]
fn managed_overlay_unlink_encodes_retained_path_without_deleting_checkout() {
    let f = display_path_fixture();
    let state = f.0.join("state");
    fs::create_dir_all(state.join("overlay")).unwrap();
    fs::write(state.join("overlay/marker"), b"unrelated checkout content").unwrap();
    fs::write(state.join("config.toml"), "schema_version=2\npresets_overlay_git='https://example.invalid/repo'\n[env]\n# preserve comment\nKEEP='value'\n").unwrap();
    let output = f
        .command()
        .env("NO_COLOR", "1")
        .env("PATH", "")
        .args(["preset", "overlay", "unlink"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(text.lines().count(), 3, "{text:?}");
    assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
    assert!(text.contains("checkout remains at"), "{text:?}");
    let config = fs::read_to_string(state.join("config.toml")).unwrap();
    assert!(config.contains("# preserve comment"));
    let parsed: toml::Value = toml::from_str(&config).unwrap();
    assert!(
        !parsed
            .as_table()
            .unwrap()
            .contains_key("presets_overlay_git")
    );
    assert_eq!(parsed["env"]["KEEP"].as_str(), Some("value"));
    assert_eq!(
        fs::read(state.join("overlay/marker")).unwrap(),
        b"unrelated checkout content"
    );
}

#[test]
fn self_install_encodes_receipts_and_omits_control_path_commands_preserving_target() {
    let f = display_path_fixture();
    let dir = f.0.join("install-dir");
    let dest = dir.join(if cfg!(windows) { "shine.exe" } else { "shine" });
    for on_path in [false, true] {
        let mut command = f.command();
        command
            .env("NO_COLOR", "1")
            .args(["self", "install", "--dest"])
            .arg(&dest);
        if on_path {
            command.env("PATH", &dir);
        }
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
        assert!(text.starts_with("installed to "), "{text:?}");
        if on_path {
            assert!(text.contains("already on PATH"), "{text:?}");
        } else if cfg!(unix) {
            assert!(text.contains("original directory"), "{text:?}");
            assert!(!text.contains("export PATH="), "{text:?}");
        }
        let config: toml::Value =
            toml::from_str(&fs::read_to_string(f.0.join("state/config.toml")).unwrap()).unwrap();
        assert_eq!(
            PathBuf::from(config["self_install_dest"].as_str().unwrap()),
            dest
        );
        assert!(dest.is_file());
    }
}

#[test]
fn migration_backup_notice_encodes_path_preserving_backup_and_json_report() {
    for json in [false, true] {
        let f = display_path_fixture();
        let state = f.0.join("state");
        let app = f.0.join("source/app/demo");
        fs::create_dir_all(&app).unwrap();
        fs::create_dir(&state).unwrap();
        fs::write(state.join("config.toml"), "schema_version=2\n").unwrap();
        let original = "dest='~/.demo'\n[[files]]\nsource='config.toml'\ntarget='config.toml'\n";
        fs::write(app.join("shine.toml"), original).unwrap();
        fs::write(app.join("config.toml"), b"unrelated payload").unwrap();
        let output = f
            .command()
            .env("NO_COLOR", "1")
            .args(["preset", "migrate"])
            .arg(f.0.join("source"))
            .args(["--yes", "--format", if json { "json" } else { "text" }])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
        let backup = if json {
            let report: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(report["status"], "applied");
            state.join(report["backup_set"].as_str().unwrap())
        } else {
            assert!(
                text.lines().any(|line| line.starts_with("Backup: ")),
                "{text:?}"
            );
            fs::read_dir(state.join("preset-migration-backups"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path()
        };
        assert_eq!(
            fs::read_to_string(backup.join("app/demo/shine.toml")).unwrap(),
            original
        );
        let migrated: toml::Value =
            toml::from_str(&fs::read_to_string(app.join("shine.toml")).unwrap()).unwrap();
        assert_eq!(migrated["metadata_schema_version"].as_integer(), Some(2));
        assert_eq!(
            fs::read(app.join("config.toml")).unwrap(),
            b"unrelated payload"
        );
        assert_eq!(
            fs::read(state.join("config.toml")).unwrap(),
            b"schema_version=2\n"
        );
    }
}

#[test]
fn secretless_seal_encodes_result_path_without_tools_or_changing_plain_values() {
    let f = display_path_fixture();
    let state = f.0.join("state");
    let tools = f.0.join("empty-tools");
    fs::create_dir(&state).unwrap();
    fs::create_dir(&tools).unwrap();
    fs::write(state.join("config.toml"), "schema_version=2\n").unwrap();
    let source = f.0.join("env.public");
    fs::write(&source, "[plain]\nVALUE='PUBLIC_SOURCE_CANARY'\n").unwrap();
    let output = f
        .command()
        .env("NO_COLOR", "1")
        .env("PATH", tools)
        .args(["env", "secret", "seal"])
        .arg(&source)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(text.lines().count(), 1, "{text:?}");
    assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
    assert!(text.starts_with("sealed "), "{text:?}");
    assert!(!text.contains("PUBLIC_SOURCE_CANARY"), "{text:?}");
    let sealed: toml::Value = toml::from_str(&fs::read_to_string(source).unwrap()).unwrap();
    assert_eq!(
        sealed["plain"]["VALUE"].as_str(),
        Some("PUBLIC_SOURCE_CANARY")
    );
    assert_eq!(sealed["payload"]["data"].as_str(), Some(""));
    assert_eq!(
        fs::read(state.join("config.toml")).unwrap(),
        b"schema_version=2\n"
    );
}

#[test]
fn init_encodes_prompt_and_result_preserving_consent_and_project_path() {
    use std::io::Write as _;
    let fixture = Fixture::new();
    let work = fixture.0.join(if cfg!(unix) {
        "work\nFORGED=init\r\t\u{1b}\\literal中文"
    } else {
        "work中文"
    });
    fs::create_dir(&work).unwrap();
    let mut denied = fixture
        .command()
        .current_dir(&work)
        .env("NO_COLOR", "1")
        .arg("init")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    denied.stdin.take().unwrap().write_all(b"n\n").unwrap();
    let denied = denied.wait_with_output().unwrap();
    assert_eq!(denied.status.code(), Some(0), "{denied:?}");
    assert!(denied.stderr.is_empty(), "{denied:?}");
    let prompt = String::from_utf8(denied.stdout).unwrap();
    assert!(prompt.ends_with("[y/N] Init cancelled.\n"), "{prompt:?}");
    assert_eq!(prompt.lines().count(), 1, "{prompt:?}");
    assert!(!work.join("shine.config.toml").exists());
    let yes = fixture
        .command()
        .current_dir(&work)
        .env("NO_COLOR", "1")
        .args(["init", "--yes"])
        .output()
        .unwrap();
    assert_eq!(yes.status.code(), Some(0), "{yes:?}");
    assert!(yes.stderr.is_empty(), "{yes:?}");
    let text = String::from_utf8(yes.stdout).unwrap();
    assert_eq!(text.lines().count(), 2, "{text:?}");
    assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
    if cfg!(unix) {
        assert!(
            prompt.contains("work\\nFORGED=init\\r\\t\\u{1b}\\literal中文"),
            "{prompt:?}"
        );
        assert!(
            text.contains("work\\nFORGED=init\\r\\t\\u{1b}\\literal中文/shine.config.toml"),
            "{text:?}"
        );
    }
    let config: toml::Value =
        toml::from_str(&fs::read_to_string(work.join("shine.config.toml")).unwrap()).unwrap();
    assert_eq!(config["presets_dir"].as_str(), Some("."));
    let original = fs::read(work.join("shine.config.toml")).unwrap();
    let child = work.join("nested");
    fs::create_dir(&child).unwrap();
    for cwd in [&work, &child] {
        let refused = fixture
            .command()
            .current_dir(cwd)
            .env("NO_COLOR", "1")
            .args(["init", "--yes"])
            .output()
            .unwrap();
        assert_eq!(refused.status.code(), Some(1), "{refused:?}");
        assert!(refused.stdout.is_empty());
        let text = String::from_utf8(refused.stderr).unwrap();
        assert!(text.contains("already under a shine project"));
        assert!(
            !text.contains(['\r', '\t', '\u{1b}']) && !text.contains("\nFORGED"),
            "{text:?}"
        );
        assert_eq!(fs::read(work.join("shine.config.toml")).unwrap(), original);
        assert!(!child.join("shine.config.toml").exists());
    }
    assert!(!fixture.0.join("state/config.toml").exists());
}

#[test]
fn preset_new_encodes_paths_preserving_template_and_force_policy() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    fs::create_dir(&state).unwrap();
    fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
    for kind in ["app", "shell", "sys"] {
        let work = fixture.0.join(format!(
            "{kind}{}",
            if cfg!(unix) {
                "\nFORGED=template\r\t\u{1b}\\literal中文"
            } else {
                "中文"
            }
        ));
        fs::create_dir(&work).unwrap();
        let created = fixture
            .command()
            .current_dir(&work)
            .env("NO_COLOR", "1")
            .args(["preset", "new", kind])
            .output()
            .unwrap();
        assert_eq!(created.status.code(), Some(0), "{created:?}");
        assert!(created.stderr.is_empty(), "{created:?}");
        let text = String::from_utf8(created.stdout).unwrap();
        assert!(
            text.starts_with(&format!("Created {kind} preset template: ")),
            "{text:?}"
        );
        assert_eq!(text.lines().count(), 1, "{text:?}");
        assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
        if cfg!(unix) {
            assert!(
                text.contains("\\nFORGED=template\\r\\t\\u{1b}\\literal中文/shine.toml"),
                "{text:?}"
            );
        }
        let path = work.join("shine.toml");
        let template = fs::read(&path).unwrap();
        fs::write(&path, b"unrelated user content\n").unwrap();
        let refused = fixture
            .command()
            .current_dir(&work)
            .args(["preset", "new", kind])
            .output()
            .unwrap();
        assert_eq!(refused.status.code(), Some(1), "{refused:?}");
        assert!(refused.stdout.is_empty(), "{refused:?}");
        assert_eq!(fs::read(&path).unwrap(), b"unrelated user content\n");
        let forced = fixture
            .command()
            .current_dir(&work)
            .env("NO_COLOR", "1")
            .args(["preset", "new", kind, "--force"])
            .output()
            .unwrap();
        assert_eq!(forced.status.code(), Some(0), "{forced:?}");
        assert!(forced.stderr.is_empty(), "{forced:?}");
        let text = String::from_utf8(forced.stdout).unwrap();
        assert!(
            text.starts_with(&format!("Updated {kind} preset template: ")),
            "{text:?}"
        );
        assert_eq!(text.lines().count(), 1, "{text:?}");
        assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
        assert_eq!(fs::read(path).unwrap(), template);
    }
    assert_eq!(
        fs::read(state.join("config.toml")).unwrap(),
        b"schema_version = 2\n"
    );
}

#[test]
fn task_cwd_display_escapes_controls_while_child_output_and_exit_stay_raw() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    fs::create_dir(&state).unwrap();
    fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
    let cwd = fixture.0.join("work\nFORGED=cwd\r\t\u{1b}");
    let mut manifest = cli::task::manifest::TaskManifest::default();
    manifest.upsert("probe", vec!["never-run".into()], Some(cwd.clone()));
    let original = toml::to_string(&manifest).unwrap();
    fs::write(state.join("tasks.toml"), &original).unwrap();
    let encoded = "work\\nFORGED=cwd\\r\\t\\u{1b}";
    let info = fixture
        .command()
        .env("NO_COLOR", "1")
        .args(["task", "info", "probe"])
        .output()
        .unwrap();
    assert_eq!(info.status.code(), Some(0), "{info:?}");
    assert!(info.stderr.is_empty(), "{info:?}");
    let text = String::from_utf8(info.stdout).unwrap();
    assert!(text.contains(encoded), "{text:?}");
    assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
    assert_eq!(
        fs::read_to_string(state.join("tasks.toml")).unwrap(),
        original
    );
    #[cfg(unix)]
    {
        fs::create_dir(&cwd).unwrap();
        let cwd = fs::canonicalize(&cwd).unwrap();
        let saved = fixture
            .command()
            .env("NO_COLOR", "1")
            .args(["task", "save", "probe", "--force", "--cwd"])
            .arg(&cwd)
            .args(["--", "/bin/sh", "-c", "pwd -P; exit 7"])
            .output()
            .unwrap();
        assert_eq!(saved.status.code(), Some(0), "{saved:?}");
        assert!(saved.stderr.is_empty(), "{saved:?}");
        let text = String::from_utf8(saved.stdout).unwrap();
        assert!(text.contains(encoded), "{text:?}");
        assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
        let before = fs::read(state.join("tasks.toml")).unwrap();
        let manifest: cli::task::manifest::TaskManifest =
            toml::from_str(std::str::from_utf8(&before).unwrap()).unwrap();
        assert_eq!(manifest.get("probe").unwrap().cwd.as_ref(), Some(&cwd));
        let run = fixture
            .command()
            .env("NO_COLOR", "1")
            .args(["task", "run", "probe"])
            .output()
            .unwrap();
        assert_eq!(run.status.code(), Some(7), "{run:?}");
        let expected = format!("{}\n", cwd.display()).into_bytes();
        assert_eq!(run.stdout, expected);
        let stderr = String::from_utf8(run.stderr).unwrap();
        assert!(stderr.contains(encoded), "{stderr:?}");
        assert_eq!(stderr.lines().count(), 1, "{stderr:?}");
        assert!(!stderr.contains(['\r', '\t', '\u{1b}']), "{stderr:?}");
        assert_eq!(fs::read(state.join("tasks.toml")).unwrap(), before);
    }
}

#[test]
fn state_migration_rejects_invalid_documents_before_cleanup_or_writes() {
    for schema in [0, 2] {
        for (name, contents, message) in [
            (
                "shine.config.toml",
                "[env]\nTOKEN = \"PRIVATE_MIGRATION_CANARY_UNCLOSED\n",
                "invalid migration TOML syntax",
            ),
            (
                "shine.workspace.toml",
                "[env]\nTOKEN = \"PRIVATE_MIGRATION_CANARY_UNCLOSED\n",
                "invalid migration TOML syntax",
            ),
            (
                "shine.workspace.toml",
                "version=2\nenv='PRIVATE_MIGRATION_CANARY'\n",
                "workspace env must be a table",
            ),
            (
                "shine.workspace.toml",
                "version=2\n[env]\nencryption='PRIVATE_MIGRATION_CANARY'\n",
                "workspace env.encryption must be a table",
            ),
        ] {
            for dry_run in [false, true] {
                let fixture = display_path_fixture();
                let state = fixture.0.join("state");
                fs::create_dir(&state).unwrap();
                let config = state.join("config.toml");
                let original =
                    format!("schema_version = {schema}\ngpg_key_id = 'fixture@example.invalid'\n");
                fs::write(&config, &original).unwrap();
                let cache = state.join("update-check.json");
                fs::write(&cache, b"preserve-cache").unwrap();
                let malformed = fixture.0.join(name);
                fs::write(&malformed, contents).unwrap();
                let mut command = fixture.command();
                command.env_remove("SUDO_USER").args(["state", "migrate"]);
                if dry_run {
                    command.arg("--dry-run");
                }
                let result = command.output().unwrap();
                assert_eq!(result.status.code(), Some(1), "{result:?}");
                assert!(result.stdout.is_empty(), "{result:?}");
                let diagnostic = String::from_utf8(result.stderr).unwrap();
                assert!(diagnostic.contains(name), "{diagnostic}");
                assert!(
                    !diagnostic.contains(['\r', '\t', '\u{1b}'])
                        && !diagnostic.contains("\nFORGED"),
                    "{diagnostic:?}"
                );
                assert!(diagnostic.contains(message), "{diagnostic}");
                assert!(
                    !diagnostic.contains("PRIVATE_MIGRATION_CANARY"),
                    "{diagnostic}"
                );
                assert!(!diagnostic.contains("TOKEN ="), "{diagnostic}");
                assert_eq!(fs::read(&config).unwrap(), original.as_bytes());
                assert_eq!(fs::read(&cache).unwrap(), b"preserve-cache");
                assert_eq!(fs::read(&malformed).unwrap(), contents.as_bytes());
                assert_eq!(fs::read_dir(&state).unwrap().count(), 2);
            }
        }
    }
}

#[test]
fn state_migration_accepts_workspace_without_encryption_settings() {
    for contents in [
        "version=2\n",
        "version=2\n[env]\n",
        "version=2\n[env.encryption]\n",
        "version=2\nenv={ encryption={} }\n",
    ] {
        let fixture = Fixture::new();
        let state = fixture.0.join("state");
        fs::create_dir(&state).unwrap();
        let original = b"schema_version=2\nlast_cleared_schema_version=2\n";
        fs::write(state.join("config.toml"), original).unwrap();
        let workspace = fixture.0.join("shine.workspace.toml");
        fs::write(&workspace, contents).unwrap();
        for dry_run in [true, false] {
            let mut command = fixture.command();
            command.env_remove("SUDO_USER").args(["state", "migrate"]);
            if dry_run {
                command.arg("--dry-run");
            }
            let result = command.output().unwrap();
            assert_eq!(result.status.code(), Some(0), "{result:?}");
            assert!(result.stderr.is_empty(), "{result:?}");
            assert!(String::from_utf8_lossy(&result.stdout).contains("already current"));
            assert_eq!(fs::read(state.join("config.toml")).unwrap(), original);
            assert_eq!(fs::read(&workspace).unwrap(), contents.as_bytes());
            assert_eq!(fs::read_dir(&state).unwrap().count(), 1);
        }
    }
}

#[test]
fn preset_links_reject_unrepresentable_operands_before_alias_selection_or_creation() {
    #[cfg(unix)]
    let component = {
        use std::os::unix::ffi::OsStringExt;
        std::ffi::OsString::from_vec(b"source-\xff".to_vec())
    };
    #[cfg(windows)]
    let component = {
        use std::os::windows::ffi::OsStringExt;
        std::ffi::OsString::from_wide(&[b's' as u16, 0xd800])
    };
    for args in [vec!["preset", "link"], vec!["preset", "overlay", "link"]] {
        for create in [false, true] {
            for alias_exists in [true, false] {
                let fixture = Fixture::new();
                let state = fixture.0.join("state");
                fs::create_dir_all(&state).unwrap();
                let config = state.join("config.toml");
                let before = b"schema_version = 2\n# unrelated setting\n[env]\nKEY = 'unchanged'\n";
                fs::write(&config, before).unwrap();
                let raw = fixture.0.join(&component);
                let alias = fixture.0.join(component.to_string_lossy().as_ref());
                if alias_exists {
                    fs::create_dir_all(&alias).unwrap();
                    fs::write(alias.join("marker"), "sibling").unwrap();
                }
                let mut command = fixture.command();
                command.env("NO_COLOR", "1").args(&args).arg(&raw);
                if create {
                    command.arg("--create");
                }
                let result = command.output().unwrap();
                assert_eq!(
                    result.status.code(),
                    Some(1),
                    "{args:?} create={create} alias={alias_exists}: {result:?}"
                );
                assert!(
                    String::from_utf8_lossy(&result.stderr).contains("not valid UTF-8"),
                    "{result:?}"
                );
                assert!(
                    !String::from_utf8_lossy(&result.stdout).contains("Preset Source"),
                    "{result:?}"
                );
                assert!(
                    !String::from_utf8_lossy(&result.stdout).contains("Presets Overlay"),
                    "{result:?}"
                );
                assert_eq!(fs::read(&config).unwrap(), before);
                assert!(!raw.exists());
                assert_eq!(alias.exists(), alias_exists);
                if alias_exists {
                    assert_eq!(fs::read(alias.join("marker")).unwrap(), b"sibling");
                }
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn preset_links_reject_lossy_home_expansion_but_accept_unrelated_absolute_paths() {
    use std::os::unix::ffi::OsStringExt;
    for args in [vec!["preset", "link"], vec!["preset", "overlay", "link"]] {
        for operand in ["~/source", "$HOME/source", "${HOME}/source"] {
            let fixture = Fixture::new();
            let raw_home = fixture
                .0
                .join(std::ffi::OsString::from_vec(b"home-\xff".to_vec()));
            let alias = fixture.0.join("home-\u{fffd}/source");
            fs::create_dir_all(&alias).unwrap();
            fs::write(alias.join("marker"), "sibling").unwrap();
            let state = fixture.0.join("state");
            fs::create_dir_all(&state).unwrap();
            let config = state.join("config.toml");
            let before = b"schema_version = 2\n# preserve this config\n";
            fs::write(&config, before).unwrap();
            let run = |operand: &std::ffi::OsStr| {
                fixture
                    .command()
                    .env("HOME", &raw_home)
                    .env_remove("SUDO_USER")
                    .args(&args)
                    .arg(operand)
                    .output()
                    .unwrap()
            };
            let result = run(std::ffi::OsStr::new(operand));
            assert_eq!(
                result.status.code(),
                Some(1),
                "{args:?} {operand}: {result:?}"
            );
            assert_eq!(fs::read(&config).unwrap(), before);
            assert_eq!(fs::read(alias.join("marker")).unwrap(), b"sibling");
            let selected = fixture.0.join("absolute-source");
            fs::create_dir_all(&selected).unwrap();
            let result = run(selected.as_os_str());
            assert_eq!(result.status.code(), Some(0), "{result:?}");
            let saved: toml::Value = toml::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
            let key = if args.len() == 2 {
                "presets_dir"
            } else {
                "presets_overlay_dir"
            };
            assert_eq!(
                saved[key].as_str().unwrap(),
                fs::canonicalize(selected).unwrap().to_str().unwrap()
            );
        }
    }
}

#[test]
fn preset_links_keep_utf8_expansion_create_modes_and_noop_persistence() {
    for (args, key, mode) in [
        (vec!["preset", "link"], "presets_dir", "snapshot"),
        (vec!["preset", "link", "--live"], "presets_dir", "live"),
        (
            vec!["preset", "overlay", "link"],
            "presets_overlay_dir",
            "snapshot",
        ),
    ] {
        let fixture = Fixture::new();
        let state = fixture.0.join("state");
        fs::create_dir_all(&state).unwrap();
        let config = state.join("config.toml");
        fs::write(
            &config,
            "schema_version = 2\n# preserve comment\n[env]\nKEY = 'unchanged'\n",
        )
        .unwrap();
        let leaf = if cfg!(unix) {
            "source ' spaces\u{a0}\nFORGED=source\r\t\u{1b}\\literal中文"
        } else {
            "source ' spaces\u{a0}"
        };
        let selected = fixture.0.join("home").join(leaf);
        let run = |operand| {
            fixture
                .command()
                .env_remove("SUDO_USER")
                .env("SHINE_LINK_LEAF", leaf)
                .args(&args)
                .arg(operand)
                .arg("--create")
                .output()
                .unwrap()
        };
        let result = run("~/$SHINE_LINK_LEAF");
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(selected.is_dir());
        let before = fs::read(&config).unwrap();
        let text = std::str::from_utf8(&before).unwrap();
        assert!(text.contains("# preserve comment"));
        let saved: toml::Value = toml::from_str(text).unwrap();
        assert_eq!(
            saved[key].as_str().unwrap(),
            fs::canonicalize(&selected).unwrap().to_str().unwrap()
        );
        assert_eq!(saved["env"]["KEY"].as_str(), Some("unchanged"));
        assert_eq!(
            saved
                .get("external_shell_mode")
                .and_then(toml::Value::as_str)
                .unwrap_or("snapshot"),
            mode
        );
        let result = run("$HOME/$SHINE_LINK_LEAF");
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(
            String::from_utf8_lossy(&result.stdout).contains("already linked"),
            "{result:?}"
        );
        assert!(
            !String::from_utf8_lossy(&result.stdout).contains(['\r', '\t', '\u{1b}']),
            "{result:?}"
        );
        assert_eq!(fs::read(&config).unwrap(), before);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn preset_links_reject_existing_non_utf8_sources_and_canonical_symlink_targets() {
    use std::os::unix::{ffi::OsStringExt, fs::symlink};
    for args in [vec!["preset", "link"], vec!["preset", "overlay", "link"]] {
        let fixture = Fixture::new();
        let raw = fixture
            .0
            .join(std::ffi::OsString::from_vec(b"source-\xff".to_vec()));
        fs::create_dir_all(&raw).unwrap();
        fs::write(raw.join("marker"), "original").unwrap();
        let linked = fixture.0.join("utf8-link");
        symlink(&raw, &linked).unwrap();
        let state = fixture.0.join("state");
        fs::create_dir_all(&state).unwrap();
        let config = state.join("config.toml");
        let before = b"schema_version = 2\n# no changes\n";
        fs::write(&config, before).unwrap();
        for operand in [&raw, &linked] {
            let result = fixture
                .command()
                .args(&args)
                .arg(operand)
                .arg("--create")
                .output()
                .unwrap();
            assert_eq!(result.status.code(), Some(1), "{result:?}");
            assert!(
                String::from_utf8_lossy(&result.stderr).contains("not valid UTF-8"),
                "{result:?}"
            );
            assert_eq!(fs::read(&config).unwrap(), before);
            assert_eq!(fs::read(raw.join("marker")).unwrap(), b"original");
            assert_eq!(fs::read_link(&linked).unwrap(), raw);
        }
    }
}

#[cfg(unix)]
#[test]
fn preset_source_notes_escape_paths_without_changing_effective_layers_or_state() {
    let fixture = Fixture::new();
    let state = fixture.0.join("base\nFORGED=source\u{1b}");
    fs::create_dir(&state).unwrap();
    let base = state.join("presets");
    let overlay = fixture.0.join("overlay\nFORGED=overlay\u{1b}");
    for (root, description) in [(&base, "from-base"), (&overlay, "from-overlay")] {
        let category = root.join("app/demo");
        fs::create_dir_all(&category).unwrap();
        fs::write(category.join("shine.toml"), format!("metadata_schema_version = 2\ndescription = '{description}'\ndest = '~/.config/demo'\n[[files]]\nsource = 'config.txt'\n")).unwrap();
        fs::write(category.join("config.txt"), description).unwrap();
    }
    for use_overlay in [false, true] {
        let config = if use_overlay {
            format!(
                "schema_version = 2\npresets_overlay_dir = {}\n",
                serde_json::to_string(overlay.to_str().unwrap()).unwrap()
            )
        } else {
            "schema_version = 2\n".to_owned()
        };
        fs::write(state.join("config.toml"), &config).unwrap();
        for args in [vec!["app", "list"], vec!["app", "info", "demo"]] {
            let result = fixture
                .command()
                .env_remove("SUDO_USER")
                .env("NO_COLOR", "1")
                .env("SHINE_CONFIG_DIR", &state)
                .args(&args)
                .output()
                .unwrap();
            assert_eq!(result.status.code(), Some(0), "{args:?}: {result:?}");
            assert!(result.stderr.is_empty(), "{result:?}");
            let text = String::from_utf8(result.stdout).unwrap();
            assert!(text.contains("base\\nFORGED=source\\u{1b}"), "{text:?}");
            assert_eq!(
                text.contains("overlay\\nFORGED=overlay\\u{1b}"),
                use_overlay,
                "{text:?}"
            );
            assert_eq!(text.contains("◈ Presets Overlay"), use_overlay, "{text:?}");
            assert!(
                text.contains("snapshot · changes require `shine upgrade`"),
                "{text:?}"
            );
            assert!(
                text.contains(if use_overlay {
                    "from-overlay"
                } else {
                    "from-base"
                }),
                "{text:?}"
            );
            assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
            assert!(
                !text.lines().any(|line| line.starts_with("FORGED=")),
                "{text:?}"
            );
            assert_eq!(
                fs::read_to_string(state.join("config.toml")).unwrap(),
                config
            );
            assert!(!state.join("app-manifest.toml").exists());
            assert!(!fixture.0.join("home/.config").exists());
        }
    }
    for (root, expected) in [
        (&base, b"from-base".as_slice()),
        (&overlay, b"from-overlay".as_slice()),
    ] {
        assert_eq!(
            fs::read(root.join("app/demo/config.txt")).unwrap(),
            expected
        );
    }
}

#[test]
fn app_catalog_queries_escape_metadata_without_changing_source_or_install_state() {
    for (name, shown_name) in [
        ("demo", "demo"),
        #[cfg(unix)]
        ("demo界\nFORGED=name\u{1b}", "demo界\\nFORGED=name\\u{1b}"),
    ] {
        let fixture = Fixture::new();
        let state = fixture.0.join("state");
        let category = state.join("presets/app").join(name);
        fs::create_dir_all(&category).unwrap();
        let source = if name != shown_name {
            "config\nFORGED=file\u{1b}.txt"
        } else {
            "config.txt"
        };
        let dest = if name != shown_name {
            "~/.config/demo\nFORGED=path\u{1b}"
        } else {
            "~/.config/demo"
        };
        let description = "DESC\nFORGED=installed\u{1b}[31m";
        let file_description = "FILE\rFAKE\tVALUE";
        let metadata = format!(
            "metadata_schema_version = 2\ndescription = {}\ndest = {}\nlist_mode = 'files'\n[[files]]\nsource = {}\ndescription = {}\n[[files]]\nsource = 'other.txt'\n",
            serde_json::to_string(description).unwrap(),
            serde_json::to_string(dest).unwrap(),
            serde_json::to_string(source).unwrap(),
            serde_json::to_string(file_description).unwrap(),
        );
        fs::write(category.join("shine.toml"), &metadata).unwrap();
        fs::write(category.join(source), "demo").unwrap();
        fs::write(category.join("other.txt"), "other").unwrap();
        let sibling = state.join("presets/app/short");
        fs::create_dir(&sibling).unwrap();
        fs::write(sibling.join("shine.toml"), "metadata_schema_version = 2\ndescription = 'sibling'\ndest = '~/.config/short'\n[[files]]\nsource = 'other.txt'\n").unwrap();
        fs::write(sibling.join("other.txt"), "sibling").unwrap();
        fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
        let query = |args: &[&str]| {
            let result = fixture
                .command()
                .env_remove("SUDO_USER")
                .env("NO_COLOR", "1")
                .args(args)
                .output()
                .unwrap();
            assert_eq!(result.status.code(), Some(0), "{args:?}: {result:?}");
            assert!(result.stderr.is_empty(), "{result:?}");
            let text = String::from_utf8(result.stdout).unwrap();
            assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
            assert!(
                !text.lines().any(|line| line.starts_with("FORGED=")),
                "{text:?}"
            );
            text
        };
        let canonical = format!("app/{name}");
        for args in [
            vec!["app", "list"],
            vec!["app", "info", name],
            vec!["info", &canonical],
        ] {
            let text = query(&args);
            assert!(
                text.contains("DESC\\nFORGED=installed\\u{1b}[31m"),
                "{text:?}"
            );
            assert!(text.contains("FILE\\rFAKE\\tVALUE"), "{text:?}");
            if name != shown_name {
                assert!(text.contains("config\\nFORGED=file\\u{1b}.txt"), "{text:?}");
                if args[1] != "list" {
                    assert!(
                        text.contains("~/.config/demo\\nFORGED=path\\u{1b}"),
                        "{text:?}"
                    );
                }
            }
            if args[1] == "list" {
                let width = console::measure_text_width(shown_name).max("short".len());
                assert!(
                    text.contains(&format!(
                        "  {shown_name}{}  DESC",
                        " ".repeat(width - console::measure_text_width(shown_name))
                    )),
                    "{text:?}"
                );
                assert!(
                    text.contains(&format!(
                        "  short{}  sibling",
                        " ".repeat(width - "short".len())
                    )),
                    "{text:?}"
                );
            } else {
                assert!(text.contains(&format!("{shown_name}  DESC")), "{text:?}");
                if name != shown_name {
                    assert!(
                        text.contains(
                            "original target (control characters; copyable command omitted)"
                        ),
                        "{text:?}"
                    );
                    assert!(!text.contains("Run `shine install"), "{text:?}");
                } else {
                    assert!(
                        text.contains("Run `shine install app/demo` to install."),
                        "{text:?}"
                    );
                }
            }
            assert_eq!(
                fs::read_to_string(category.join("shine.toml")).unwrap(),
                metadata
            );
            assert_eq!(fs::read(category.join(source)).unwrap(), b"demo");
            assert_eq!(
                fs::read(state.join("config.toml")).unwrap(),
                b"schema_version = 2\n"
            );
            assert!(!state.join("app-manifest.toml").exists());
            assert!(!fixture.0.join("home/.config").exists());
        }
        let installed = fixture
            .command()
            .env_remove("SUDO_USER")
            .args(["app", "install", name, "--yes"])
            .output()
            .unwrap();
        if name != shown_name {
            assert_eq!(installed.status.code(), Some(1), "{installed:?}");
            assert!(
                String::from_utf8_lossy(&installed.stderr)
                    .contains("operation identity must be non-empty and single-line")
            );
            assert!(!state.join("app-manifest.toml").exists());
            assert!(!fixture.0.join("home/.config").exists());
            continue;
        }
        assert!(installed.status.success(), "{installed:?}");
        let receipt = state.join("app-manifest.toml");
        let before = fs::read(&receipt).unwrap();
        let manifest: shine_core::install::AppManifest =
            toml::from_str(std::str::from_utf8(&before).unwrap()).unwrap();
        assert!(
            manifest
                .find_by_source(&format!("app/{name}/{source}"))
                .is_some()
        );
        let text = query(&["app", "info", name]);
        assert!(
            text.contains("DESC\\nFORGED=installed\\u{1b}[31m"),
            "{text:?}"
        );
        assert!(text.contains("FILE\\rFAKE\\tVALUE"), "{text:?}");
        assert!(text.contains("Installed."), "{text:?}");
        assert!(text.contains("--replace-managed"), "{text:?}");
        assert_eq!(
            text.contains("copyable command omitted"),
            name != shown_name
        );
        assert_eq!(fs::read(receipt).unwrap(), before);
        assert_eq!(
            fs::read_to_string(category.join("shine.toml")).unwrap(),
            metadata
        );
    }
}

#[test]
fn shell_catalog_queries_encode_fields_and_keep_comment_paragraphs_and_target_policy() {
    for (name, shown_name) in [
        ("demo", "demo"),
        #[cfg(unix)]
        ("demo\nFORGED=name\u{1b}", "demo\\nFORGED=name\\u{1b}"),
    ] {
        let fixture = Fixture::new();
        let state = fixture.0.join("state");
        let category = state.join("presets/shell").join(name);
        fs::create_dir_all(&category).unwrap();
        let script = if cfg!(windows) { "tool.ps1" } else { "tool.sh" };
        let comment = if cfg!(windows) {
            "comment.ps1"
        } else {
            "comment.sh"
        };
        let metadata = format!(
            "description = \"CATEGORY\\nFORGED=category\\u001b\"\n[[files]]\nsource = '{script}'\ntarget = 'tool'\ndescription = \"DESC\\nFORGED=record\\u001b\"\n[files.permissions]\nschema_version = 1\n[[files]]\nsource = '{comment}'\ntarget = 'comment'\n[files.permissions]\nschema_version = 1\n"
        );
        fs::write(category.join("shine.toml"), &metadata).unwrap();
        fs::write(category.join(script), "# fixture; never executed\n").unwrap();
        let comments = "# COMMENT\u{1b}[31m\n#\n# CONTINUED\tliteral\n";
        fs::write(category.join(comment), comments).unwrap();
        fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
        let run = |args: &[&str], code| {
            let result = fixture
                .command()
                .env_remove("SUDO_USER")
                .env("NO_COLOR", "1")
                .args(args)
                .output()
                .unwrap();
            assert_eq!(result.status.code(), Some(code), "{args:?}: {result:?}");
            if code == 0 {
                assert!(result.stderr.is_empty(), "{result:?}");
            }
            result
        };
        let selected = format!("{name}/tool");
        let canonical = format!("shell/{selected}");
        for args in [
            vec!["shell", "list"],
            vec!["shell", "info", name],
            vec!["shell", "info", &selected],
            vec!["info", &canonical],
        ] {
            let text = String::from_utf8(run(&args, 0).stdout).unwrap();
            assert!(text.contains("DESC\\nFORGED=record\\u{1b}"), "{text:?}");
            assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
            assert!(
                !text.lines().any(|line| line.starts_with("FORGED=")),
                "{text:?}"
            );
            if args[1] == "list" {
                assert!(
                    text.contains(&format!("  {shown_name}  2 scripts")),
                    "{text:?}"
                );
                assert!(
                    text.contains(
                        "comment    COMMENT\\u{1b}[31m\n\n               CONTINUED\\tliteral\n"
                    ),
                    "{text:?}"
                );
            } else {
                assert!(
                    text.contains(&format!(
                        "{shown_name}\n  CATEGORY\\nFORGED=category\\u{{1b}}"
                    )),
                    "{text:?}"
                );
                assert!(
                    text.contains(&format!("shell/{shown_name}/{script}")),
                    "{text:?}"
                );
                if name != shown_name {
                    assert!(
                        text.contains(
                            "original target (control characters; copyable command omitted)"
                        ),
                        "{text:?}"
                    );
                    assert!(!text.contains("Run `shine"), "{text:?}");
                } else {
                    let target = if args.last() == Some(&name) {
                        "demo"
                    } else {
                        "demo/tool"
                    };
                    assert!(
                        text.contains(&format!("Run `shine shell install {target}`")),
                        "{text:?}"
                    );
                }
            }
            assert!(!state.join("shell-manifest.toml").exists());
        }
        let sibling = state.join("presets/shell/other");
        fs::create_dir(&sibling).unwrap();
        fs::write(sibling.join("shine.toml"), format!("[[files]]\nsource = '{script}'\ntarget = 'tool'\n[files.permissions]\nschema_version = 1\n")).unwrap();
        fs::write(sibling.join(script), "# untouched sibling\n").unwrap();
        let ambiguous = run(&["shell", "info", "tool"], 1);
        let error = String::from_utf8(ambiguous.stderr).unwrap();
        assert!(error.contains("ambiguous shell preset target"), "{error:?}");
        assert!(error.contains(&format!("{shown_name}/tool")), "{error:?}");
        assert!(error.contains("other/tool"), "{error:?}");
        assert!(!error.contains('\u{1b}'), "{error:?}");
        assert!(
            !error.lines().any(|line| line.starts_with("FORGED=")),
            "{error:?}"
        );
        for target in [
            "missing\nFORGED=error\u{1b}".to_owned(),
            format!("{name}/missing\nFORGED=error\u{1b}"),
        ] {
            let result = run(&["shell", "info", &target], 1);
            let error = String::from_utf8(result.stderr).unwrap();
            assert!(error.contains("\\nFORGED=error\\u{1b}"), "{error:?}");
            assert!(!error.contains('\u{1b}'), "{error:?}");
            assert!(
                !error.lines().any(|line| line.starts_with("FORGED=")),
                "{error:?}"
            );
        }
        if name == shown_name {
            for command in ["tool", "comment"] {
                run(
                    &["trust", "grant", &format!("shell/demo/{command}"), "--yes"],
                    0,
                );
            }
            run(&["shell", "install", "demo", "--yes"], 0);
            let receipt = state.join("shell-manifest.toml");
            let before = fs::read(&receipt).unwrap();
            let text = String::from_utf8(run(&["shell", "info", "demo/tool"], 0).stdout).unwrap();
            assert!(text.contains("DESC\\nFORGED=record\\u{1b}"), "{text:?}");
            assert!(
                text.contains("Run `shine install shell/demo/tool --replace-managed`"),
                "{text:?}"
            );
            assert_eq!(fs::read(receipt).unwrap(), before);
        }
        assert_eq!(
            fs::read_to_string(category.join("shine.toml")).unwrap(),
            metadata
        );
        assert_eq!(
            fs::read_to_string(category.join(comment)).unwrap(),
            comments
        );
        assert_eq!(
            fs::read(state.join("config.toml")).unwrap(),
            b"schema_version = 2\n"
        );
    }
}

#[test]
fn installed_info_encodes_metadata_but_preserves_explicit_content_and_receipts() {
    for (kind, file_description) in [("app", true), ("app", false), ("shell", true)] {
        let fixture = Fixture::new();
        #[cfg(unix)]
        let fixture = {
            let root = fixture.0.with_file_name(format!(
                "{}-root\nFORGED=path\u{1b}",
                fixture.0.file_name().unwrap().to_str().unwrap()
            ));
            fs::rename(&fixture.0, &root).unwrap();
            Fixture(root)
        };
        let state = fixture.0.join("state");
        let category = state.join("presets").join(kind).join("demo");
        fs::create_dir_all(&category).unwrap();
        let description = "DESC\nFORGED=record\u{1b}[31m";
        let category_description = "CATEGORY\nFORGED=fallback\u{1b}[31m";
        let label = "LABEL\nFORGED=name\u{1b}";
        let script = if cfg!(windows) { "tool.ps1" } else { "tool.sh" };
        let source = if kind == "app" { "config.txt" } else { script };
        let quoted_description = serde_json::to_string(description).unwrap();
        let metadata = if kind == "app" {
            format!(
                "metadata_schema_version = 2\ndest = '~/.config/demo'\ndescription = {}\n[[files]]\nsource = '{source}'\ndisplay_name = {}\n{}",
                serde_json::to_string(category_description).unwrap(),
                serde_json::to_string(label).unwrap(),
                if file_description {
                    format!("description = {quoted_description}\n")
                } else {
                    String::new()
                }
            )
        } else {
            format!(
                "[[files]]\nsource = '{source}'\ntarget = 'tool'\ndescription = {quoted_description}\n[files.permissions]\nschema_version = 1\n"
            )
        };
        let content = b"# fixture; never executed\n# RAW_CONTENT\t\x1b[32m\n";
        fs::write(category.join("shine.toml"), &metadata).unwrap();
        fs::write(category.join(source), content).unwrap();
        fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
        let run = |args: &[&str]| {
            let result = fixture
                .command()
                .env_remove("SUDO_USER")
                .env("NO_COLOR", "1")
                .args(args)
                .output()
                .unwrap();
            assert_eq!(result.status.code(), Some(0), "{args:?}: {result:?}");
            assert!(result.stderr.is_empty(), "{result:?}");
            String::from_utf8(result.stdout).unwrap()
        };
        if kind == "shell" {
            run(&["trust", "grant", "shell/demo/tool", "--yes"]);
        }
        run(&[kind, "install", "demo", "--yes"]);
        let receipt = state.join(format!("{kind}-manifest.toml"));
        let before = fs::read(&receipt).unwrap();
        let target = if kind == "app" {
            "app/demo"
        } else {
            "shell/demo/tool"
        };
        for option in [None, Some("--diff"), Some("--verbose")] {
            let mut args = vec!["info", target];
            args.extend(option);
            let text = run(&args);
            // Source notes have a separate renderer; inspect the installed-info section.
            let heading = if kind == "app" {
                "App Config:"
            } else {
                "Shell Preset:"
            };
            let report = &text[text.find(heading).unwrap()..];
            let (metadata_text, body) = if option == Some("--verbose") {
                report.split_once("--- Content:").unwrap()
            } else {
                (report, "")
            };
            let expected = if kind == "app" && !file_description {
                "Description  CATEGORY\\nFORGED=fallback\\u{1b}[31m"
            } else {
                "Description  DESC\\nFORGED=record\\u{1b}[31m"
            };
            assert!(metadata_text.contains(expected), "{text:?}");
            if kind == "app" {
                assert!(
                    metadata_text.contains("App Config: LABEL\\nFORGED=name\\u{1b}"),
                    "{text:?}"
                );
            } else {
                assert!(
                    metadata_text.contains("Shell Preset: demo/tool"),
                    "{text:?}"
                );
            }
            assert!(!metadata_text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
            assert!(
                !metadata_text
                    .lines()
                    .any(|line| line.starts_with("FORGED=")),
                "{text:?}"
            );
            if cfg!(unix) {
                assert!(
                    metadata_text.contains("-root\\nFORGED=path\\u{1b}"),
                    "{text:?}"
                );
                assert!(!body.contains("-root\nFORGED=path\u{1b}"), "{text:?}");
            }
            if option == Some("--verbose") {
                assert!(
                    body.ends_with(std::str::from_utf8(content).unwrap()),
                    "{text:?}"
                );
            }
            assert_eq!(fs::read(&receipt).unwrap(), before);
            assert_eq!(fs::read(category.join(source)).unwrap(), content);
            assert_eq!(
                fs::read_to_string(category.join("shine.toml")).unwrap(),
                metadata
            );
        }
    }
}

#[test]
fn app_targets_preserve_whitespace_identity_across_lifecycle_and_info() {
    for selected in [
        " demo",
        "demo\u{a0}",
        #[cfg(unix)]
        "demo ",
    ] {
        let fixture = Fixture::new();
        let state = fixture.0.join("state");
        for (category, dest) in [("demo", "base"), (selected, "selected")] {
            let preset = state.join("presets/app").join(category);
            fs::create_dir_all(&preset).unwrap();
            fs::write(preset.join("shine.toml"), format!("metadata_schema_version = 2\ndescription = '{dest} description'\ndest = '~/.config/{dest}'\n[[files]]\nsource = 'config.txt'\n")).unwrap();
            fs::write(preset.join("config.txt"), "v1").unwrap();
        }
        // A real whitespace-only category must not bypass the blank-operand guard
        // through available-info fallback. NBSP is a valid filename on Windows.
        let blank = state.join("presets/app/\u{a0}");
        fs::create_dir_all(&blank).unwrap();
        fs::write(blank.join("shine.toml"), "metadata_schema_version = 2\ndest = '~/.config/blank'\n[[files]]\nsource = 'config.txt'\n").unwrap();
        fs::write(blank.join("config.txt"), "blank").unwrap();
        fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
        let run = |args: &[&str], code| {
            let result = fixture
                .command()
                .env("NO_COLOR", "1")
                .args(args)
                .output()
                .unwrap();
            assert_eq!(result.status.code(), Some(code), "{args:?}: {result:?}");
            result
        };
        let canonical = format!("app/{selected}");
        let check_info = || {
            run(&["info", "\u{a0}"], 1);
            for target in [selected, canonical.as_str()] {
                let result = run(&["info", target], 0);
                let text = String::from_utf8(result.stdout).unwrap();
                assert!(text.contains("selected description"), "{text}");
                assert!(!text.contains("base description"), "{text}");
            }
        };
        check_info(); // Available fallback, before any receipts exist.
        for category in ["demo", selected] {
            run(&["app", "install", category, "--yes"], 0);
        }
        check_info(); // Installed target and alias lookup.
        let base = fixture.0.join("home/.config/base/config.txt");
        let destination = fixture.0.join("home/.config/selected/config.txt");
        let source = state.join("presets/app").join(selected).join("config.txt");
        let receipt = state.join("app-manifest.toml");
        let manifest: shine_core::install::AppManifest =
            toml::from_str(&fs::read_to_string(&receipt).unwrap()).unwrap();
        let sibling = manifest
            .entries
            .iter()
            .find(|entry| entry.destination == base)
            .unwrap()
            .clone();
        fs::write(&source, "v2").unwrap();
        run(&["install", selected, "--yes"], 0);
        assert_eq!(fs::read(&destination).unwrap(), b"v2");
        assert_eq!(fs::read(&base).unwrap(), b"v1");
        fs::write(&source, "v3").unwrap();
        let before = fs::read(&receipt).unwrap();
        let denied = run(&["upgrade", &canonical], 1);
        assert!(String::from_utf8_lossy(&denied.stderr).contains("approval requires"));
        assert_eq!(fs::read(&receipt).unwrap(), before);
        assert_eq!(fs::read(&destination).unwrap(), b"v2");
        run(&["upgrade", &canonical, "--yes"], 0);
        assert_eq!(fs::read(&destination).unwrap(), b"v3");
        fs::write(&source, "v4").unwrap();
        let before = fs::read(&receipt).unwrap();
        for args in [
            vec!["update", canonical.as_str()],
            vec!["uninstall", canonical.as_str(), "--dry-run"],
        ] {
            let result = run(&args, 0);
            let text = String::from_utf8(result.stdout).unwrap().replace('\\', "/");
            if args[0] == "update" {
                assert!(text.contains("update available"), "{text}");
            } else {
                assert!(text.contains("/selected/config.txt"), "{text}");
                assert!(!text.contains("/base/config.txt"), "{text}");
            }
            assert_eq!(fs::read(&receipt).unwrap(), before);
            assert_eq!(fs::read(&destination).unwrap(), b"v3");
            assert_eq!(fs::read(&base).unwrap(), b"v1");
        }
        run(&["uninstall", selected, "--yes"], 0);
        assert!(!destination.exists());
        assert_eq!(fs::read(&base).unwrap(), b"v1");
        let manifest: shine_core::install::AppManifest =
            toml::from_str(&fs::read_to_string(&receipt).unwrap()).unwrap();
        assert_eq!(manifest.entries.len(), 1);
        assert_eq!(manifest.entries[0], sibling);
    }
}

#[test]
fn shell_targets_preserve_whitespace_identity_across_lifecycle_and_info() {
    for selected in [
        " demo",
        "demo\u{a0}",
        #[cfg(unix)]
        "demo ",
    ] {
        let fixture = Fixture::new();
        let state = fixture.0.join("state");
        let script = if cfg!(windows) { "tool.ps1" } else { "tool.sh" };
        for (category, command) in [("demo", "base-tool"), (selected, "selected-tool")] {
            let preset = state.join("presets/shell").join(category);
            fs::create_dir_all(&preset).unwrap();
            fs::write(preset.join("shine.toml"), format!("[[files]]\nsource = '{script}'\ntarget = '{command}'\n[files.permissions]\nschema_version = 1\n")).unwrap();
            fs::write(preset.join(script), "# fixture; never executed\n").unwrap();
        }
        fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
        let run = |args: &[&str], code| {
            let result = fixture
                .command()
                .env("NO_COLOR", "1")
                .args(args)
                .output()
                .unwrap();
            assert_eq!(result.status.code(), Some(code), "{args:?}: {result:?}");
            result
        };
        let canonical = format!("shell/{selected}");
        for args in [
            vec!["shell", "info", selected],
            vec!["info", canonical.as_str()],
        ] {
            let result = run(&args, 0);
            let text = String::from_utf8(result.stdout).unwrap();
            assert!(text.contains("selected-tool"), "{text}");
            assert!(!text.contains("base-tool"), "{text}");
        }
        for (category, command) in [("demo", "base-tool"), (selected, "selected-tool")] {
            run(
                &[
                    "trust",
                    "grant",
                    &format!("shell/{category}/{command}"),
                    "--yes",
                ],
                0,
            );
            run(&["shell", "install", category, "--yes"], 0);
        }
        let receipt = state.join("shell-manifest.toml");
        let manifest: shine_core::runtime::ShellManifest =
            toml::from_str(&fs::read_to_string(&receipt).unwrap()).unwrap();
        let sibling = manifest
            .entries
            .iter()
            .find(|entry| entry.category == "demo")
            .unwrap()
            .clone();
        assert!(
            manifest
                .entries
                .iter()
                .any(|entry| entry.category == selected)
        );
        let launcher = |command| {
            shine_core::runtime::command_path_for_name(
                &state.join("bin"),
                std::ffi::OsStr::new(command),
            )
        };
        let base_launcher = launcher("base-tool");
        let selected_launcher = launcher("selected-tool");
        let base_bytes = fs::read(&base_launcher).unwrap();
        let selected_bytes = fs::read(&selected_launcher).unwrap();
        let before = fs::read(&receipt).unwrap();
        let preview = run(&["shell", "uninstall", selected, "--dry-run"], 0);
        let text = String::from_utf8(preview.stdout).unwrap();
        // Shell preview is an aggregate; target isolation is checked on the
        // launchers and receipts below, rather than assuming per-command rows.
        let removals = if cfg!(windows) { 2 } else { 1 };
        assert!(text.contains(&format!("{removals} would remove")), "{text}");
        let denied = run(&["shell", "uninstall", selected], 1);
        assert!(String::from_utf8_lossy(&denied.stderr).contains("approval requires"));
        assert_eq!(fs::read(&receipt).unwrap(), before);
        assert_eq!(fs::read(&selected_launcher).unwrap(), selected_bytes);
        assert_eq!(fs::read(&base_launcher).unwrap(), base_bytes);
        run(&["shell", "uninstall", selected, "--yes"], 0);
        assert!(!selected_launcher.exists());
        assert_eq!(fs::read(&base_launcher).unwrap(), base_bytes);
        let manifest: shine_core::runtime::ShellManifest =
            toml::from_str(&fs::read_to_string(&receipt).unwrap()).unwrap();
        assert_eq!(manifest.entries, vec![sibling]);
    }
}

#[test]
fn trust_list_and_revoke_escape_display_without_changing_stored_target_identity() {
    use shine_core::plan::{PermissionSetV1, SnapshotDigestV1};
    use shine_core::trust::{
        TrustCapabilityV1, TrustGrantV1, TrustRequirementV1, TrustSourceV1, TrustStoreV1,
    };
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    fs::create_dir_all(state.join("presets")).unwrap();
    fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
    let target = "app/demo\x1b[31m\nFORGED";
    let requirement = TrustRequirementV1 {
        target: target.into(),
        capability: TrustCapabilityV1::AppHook,
        code_digest: SnapshotDigestV1::builder("code").finish(),
        permissions_declared: true,
        permissions: PermissionSetV1::default(),
        development_source: Some(TrustSourceV1 {
            identity: SnapshotDigestV1::builder("source").finish(),
            labels: vec!["external:/source\x1b[0m\nFORGED".into()],
        }),
    };
    let mut sibling = requirement.clone();
    sibling.target = "app/sibling".into();
    let store = TrustStoreV1 {
        grants: vec![
            TrustGrantV1::for_development_requirement(&requirement).unwrap(),
            TrustGrantV1::for_reviewed_requirement(&sibling),
        ],
        ..Default::default()
    };
    let path = state.join("trust.toml");
    fs::write(&path, toml::to_string_pretty(&store).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let before = fs::read(&path).unwrap();
    let run = |args: &[&str], code| {
        let result = fixture
            .command()
            .env("NO_COLOR", "1")
            .args(args)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(code), "{result:?}");
        if code == 0 {
            assert!(result.stderr.is_empty(), "{result:?}");
        } else {
            assert!(
                String::from_utf8_lossy(&result.stderr).contains("trust target must be"),
                "{result:?}"
            );
        }
        let text = String::from_utf8(result.stdout).unwrap();
        assert!(!text.contains('\x1b'), "{text:?}");
        assert!(
            !text.lines().any(|line| line.starts_with("FORGED")),
            "{text:?}"
        );
        text
    };
    for args in [vec!["trust", "list"], vec!["trust", "list", "--verbose"]] {
        let text = run(&args, 0);
        assert!(text.contains("app/demo\\u{1b}[31m\\nFORGED"), "{text:?}");
        if args.len() == 3 {
            assert!(
                text.contains("external:/source\\u{1b}[0m\\nFORGED"),
                "{text:?}"
            );
            assert!(
                text.contains("original target (control characters; copyable command omitted)"),
                "{text:?}"
            );
        } else {
            let target_row = text
                .lines()
                .find(|line| line.starts_with("app/demo\\u"))
                .unwrap();
            let sibling_row = text
                .lines()
                .find(|line| line.starts_with("app/sibling"))
                .unwrap();
            assert_eq!(
                target_row.find("development"),
                sibling_row.find("snapshot"),
                "{text}"
            );
        }
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    run(&["trust", "revoke", "app/demo\\u{1b}[31m\\nFORGED"], 1);
    assert_eq!(fs::read(&path).unwrap(), before);
    let revoked = run(&["trust", "revoke", target], 0);
    assert!(
        revoked.contains("Revoked external-code trust for app/demo\\u{1b}[31m\\nFORGED."),
        "{revoked:?}"
    );
    let remaining: TrustStoreV1 = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(remaining.grants.len(), 1);
    assert_eq!(remaining.grants[0].target, "app/sibling");
    let after = fs::read(&path).unwrap();
    let absent = run(&["trust", "revoke", target], 0);
    assert!(
        absent.contains("No external-code trust grants matched app/demo\\u{1b}[31m\\nFORGED."),
        "{absent:?}"
    );
    assert_eq!(fs::read(&path).unwrap(), after);
}

#[cfg(unix)]
#[test]
fn trust_enrollment_escapes_review_and_preserves_consent_and_modes() {
    use shine_core::trust::{TrustModeV1, TrustStoreV1};
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let target = "app/demo\x1b[31m\nFORGED";
    let category = state.join("presets").join(target);
    fs::create_dir_all(&category).unwrap();
    fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
    let metadata = "metadata_schema_version = 2\ndest = '~/.config/demo'\npost_install = { command = 'never-executed', args = [] }\n[[files]]\nsource = 'config.txt'\n";
    fs::write(category.join("shine.toml"), metadata).unwrap();
    fs::write(category.join("config.txt"), "preserve").unwrap();
    let path = state.join("trust.toml");
    let run = |args: &[&str], code| {
        let result = fixture
            .command()
            .env("NO_COLOR", "1")
            .args(args)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(code), "{result:?}");
        let text = String::from_utf8(result.stdout).unwrap();
        assert!(!text.contains('\x1b'), "{text:?}");
        assert!(
            !text.lines().any(|line| line.starts_with("FORGED")),
            "{text:?}"
        );
        (text, result.stderr)
    };
    assert!(run(&["trust", "inspect", target], 0).1.is_empty());
    assert!(!path.exists());
    let denied = run(&["trust", "grant", target], 1);
    assert!(
        String::from_utf8(denied.1)
            .unwrap()
            .contains("requires an interactive terminal or explicit --yes")
    );
    assert!(!path.exists());
    for development in [false, true] {
        let mut args = vec!["trust", "grant", target, "--yes"];
        if development {
            args.push("--development");
        }
        let (granted, stderr) = run(&args, 0);
        assert!(stderr.is_empty());
        assert!(
            granted.contains("app/demo\\u{1b}[31m\\nFORGED"),
            "{granted:?}"
        );
        let store: TrustStoreV1 = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(store.grants.len(), 1);
        let grant = &store.grants[0];
        assert_eq!(grant.target, target);
        assert_eq!(
            grant.mode,
            if development {
                TrustModeV1::Development
            } else {
                TrustModeV1::Snapshot
            }
        );
        let before = fs::read(&path).unwrap();
        let (inspected, stderr) = run(&["trust", "inspect", target], 0);
        assert!(stderr.is_empty());
        assert!(inspected.contains(&grant.code_digest.as_hex()));
        assert!(
            inspected.contains(if development {
                "development trusted"
            } else {
                "trusted [trusted]"
            }),
            "{inspected:?}"
        );
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    assert_eq!(
        fs::read_to_string(category.join("shine.toml")).unwrap(),
        metadata
    );
    assert_eq!(fs::read(category.join("config.txt")).unwrap(), b"preserve");
    assert!(!fixture.0.join("home/.config/demo/config.txt").exists());
}

#[test]
fn authoring_missing_control_path_preserves_json_and_failure_exits() {
    let fixture = Fixture::new();
    let path = fixture.0.join("missing\x1b[31m\nFORGED");
    for name in ["validate", "plan", "lint"] {
        for format in ["text", "json"] {
            let mut command = fixture.command();
            command
                .env("NO_COLOR", "1")
                .args(["preset", name])
                .arg(&path);
            command.args(["--format", format]);
            if name == "plan" {
                command.args(["--platform", "windows"]);
            }
            let result = command.output().unwrap();
            assert_eq!(result.status.code(), Some(1), "{name}: {result:?}");
            assert!(result.stderr.is_empty(), "{name}: {result:?}");
            let text = String::from_utf8(result.stdout).unwrap();
            if format == "json" {
                let report: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_eq!(report["valid"], false);
                if name == "validate" {
                    assert_eq!(report["path"], path.to_str().unwrap());
                    assert_eq!(report["diagnostics"][0]["code"], "invalid_input");
                }
            } else {
                assert!(!text.contains('\x1b'), "{name}: {text:?}");
                assert!(!text.lines().any(|line| line == "FORGED"), "{text:?}");
                if name == "lint" {
                    assert!(
                        text.contains(
                            "original PATH (control characters; copyable command omitted)"
                        ),
                        "{text:?}"
                    );
                    assert!(!text.contains("`shine preset validate"), "{text:?}");
                }
            }
        }
    }
    assert!(!fixture.0.join("state").exists());
}

#[cfg(unix)]
#[test]
fn authoring_text_escapes_identifiers_without_changing_json_or_warning_policy() {
    let fixture = Fixture::new();
    let name = "demo\x1b[31m\nFORGED";
    let resource = "config\tfile.txt";
    let category = fixture.0.join("app").join(name);
    fs::create_dir_all(&category).unwrap();
    let metadata = "dest = \"~/.config/de\\u001b[31m\\nFORGED\"\n[permissions]\nschema_version = 1\n[[files]]\nsource = \"config\\tfile.txt\"\n";
    fs::write(category.join("shine.toml"), metadata).unwrap();
    fs::write(category.join(resource), "preserve").unwrap();
    for command_name in ["validate", "plan", "lint"] {
        for format in ["text", "json"] {
            let mut command = fixture.command();
            command
                .env("NO_COLOR", "1")
                .args(["preset", command_name])
                .arg(&category);
            command.args(["--format", format]);
            if command_name == "plan" {
                command.args(["--platform", "linux"]);
            }
            let result = command.output().unwrap();
            assert_eq!(result.status.code(), Some(0), "{command_name}: {result:?}");
            assert!(result.stderr.is_empty(), "{result:?}");
            let text = String::from_utf8(result.stdout).unwrap();
            if format == "json" {
                let report: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_eq!(report["valid"], true);
                match command_name {
                    "validate" => assert_eq!(report["categories"][0]["name"], name),
                    "plan" => {
                        assert_eq!(report["target"], format!("app/{name}"));
                        assert_eq!(report["plans"][0]["steps"][0]["resource"], resource);
                    }
                    _ => assert_eq!(report["diagnostics"][0]["target"], format!("app/{name}")),
                }
            } else {
                assert!(!text.contains('\x1b'), "{text:?}");
                assert!(!text.contains('\t'), "{text:?}");
                assert!(
                    !text.lines().any(|line| line.starts_with("FORGED")),
                    "{text:?}"
                );
                assert!(text.contains("demo\\u{1b}[31m\\nFORGED"), "{text:?}");
                if command_name != "validate" {
                    assert!(text.contains("config\\tfile.txt"), "{text:?}");
                }
            }
        }
    }
    let denied = fixture
        .command()
        .args(["preset", "lint"])
        .arg(&category)
        .arg("--deny-warnings")
        .output()
        .unwrap();
    assert_eq!(denied.status.code(), Some(1), "{denied:?}");
    assert_eq!(
        fs::read_to_string(category.join("shine.toml")).unwrap(),
        metadata
    );
    assert_eq!(fs::read(category.join(resource)).unwrap(), b"preserve");
    assert!(!fixture.0.join("state").exists());
}

#[cfg(unix)]
#[test]
fn preset_pack_escapes_text_target_without_changing_bundle_or_force_policy() {
    let fixture = Fixture::new();
    let name = "demo\nFORGED=created\r\t\u{1b}[31m";
    let category = fixture.0.join("app").join(name);
    fs::create_dir_all(&category).unwrap();
    let metadata = "dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.txt'\n";
    fs::write(category.join("shine.toml"), metadata).unwrap();
    fs::write(category.join("config.txt"), "demo").unwrap();
    let mut bundle = None;
    for format in ["text", "json"] {
        let output = fixture.0.join(format!("{format}.tar.gz"));
        for mode in ["create", "blocked", "force"] {
            let mut command = fixture.command();
            command
                .env_remove("SUDO_USER")
                .env("NO_COLOR", "1")
                .args(["preset", "pack"])
                .arg(&category)
                .arg("--output")
                .arg(&output)
                .args(["--format", format]);
            if mode == "blocked" {
                fs::write(&output, "preserve me").unwrap();
            }
            if mode == "force" {
                fs::write(&output, "replace me").unwrap();
                command.arg("--force");
            }
            let result = command.output().unwrap();
            let valid = mode != "blocked";
            assert_eq!(result.status.code(), Some(i32::from(!valid)), "{result:?}");
            assert!(result.stderr.is_empty(), "{result:?}");
            let text = String::from_utf8(result.stdout).unwrap();
            if format == "json" {
                let report: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_eq!(report["target"], format!("app/{name}"));
                assert_eq!(report["valid"], valid);
                if !valid {
                    assert_eq!(report["diagnostics"], serde_json::json!(["output_exists"]));
                }
            } else {
                assert!(
                    text.contains("  Target: app/demo\\nFORGED=created\\r\\t\\u{1b}[31m\n"),
                    "{text:?}"
                );
                assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
                assert!(!text.lines().any(|line| line.starts_with("FORGED=")));
                assert!(text.starts_with(if valid {
                    "Preset pack: created\n"
                } else {
                    "Preset pack: blocked\n"
                }));
                assert_eq!(text.contains("error[output_exists]"), !valid);
                assert_eq!(text.contains("SHA-256:"), valid);
            }
            let bytes = fs::read(&output).unwrap();
            if valid {
                match &bundle {
                    Some(expected) => assert_eq!(&bytes, expected),
                    None => {
                        let mut archive =
                            tar::Archive::new(flate2::read::GzDecoder::new(bytes.as_slice()));
                        let mut entries = archive.entries().unwrap();
                        let manifest = entries.next().unwrap().unwrap();
                        assert_eq!(
                            manifest.path().unwrap(),
                            std::path::Path::new("shine.bundle.json")
                        );
                        let manifest: serde_json::Value =
                            serde_json::from_reader(manifest).unwrap();
                        assert_eq!(manifest["target"], format!("app/{name}"));
                        bundle = Some(bytes);
                    }
                }
            } else {
                assert_eq!(bytes, b"preserve me");
            }
            assert_eq!(
                fs::read_to_string(category.join("shine.toml")).unwrap(),
                metadata
            );
            assert_eq!(fs::read(category.join("config.txt")).unwrap(), b"demo");
            assert!(!fixture.0.join("state").exists());
        }
    }
}

#[test]
fn preset_pack_rejects_file_parent_traversal_in_text_and_json() {
    let fixture = Fixture::new();
    let category = fixture.0.join("app/demo");
    fs::create_dir_all(&category).unwrap();
    let metadata = "dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.txt'\n";
    fs::write(category.join("shine.toml"), metadata).unwrap();
    fs::write(category.join("config.txt"), "demo").unwrap();
    let blocker = fixture.0.join("file");
    fs::write(&blocker, "not a directory").unwrap();
    for format in ["text", "json"] {
        for force in [false, true] {
            let name = format!("{format}-{force}.tar.gz");
            let output = fixture.0.join(&name);
            if force {
                fs::write(&output, "preserve").unwrap();
            }
            let mut command = fixture.command();
            command
                .env("NO_COLOR", "1")
                .args(["preset", "pack"])
                .arg(&category)
                .arg("--output")
                .arg(blocker.join("..").join(&name))
                .args(["--format", format]);
            if force {
                command.arg("--force");
            }

            let refused = command.output().unwrap();

            assert_eq!(refused.status.code(), Some(1), "{refused:?}");
            assert!(refused.stderr.is_empty(), "{refused:?}");
            let text = String::from_utf8(refused.stdout).unwrap();
            if format == "json" {
                let report: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_eq!(report["valid"], false);
                assert_eq!(
                    report["diagnostics"],
                    serde_json::json!(["output_write_failed"])
                );
                assert_eq!(report["files"], 0);
                assert_eq!(report["archive_bytes"], 0);
                assert!(
                    report
                        .get("bundle_sha256")
                        .is_none_or(|value| value.is_null())
                );
            } else {
                assert!(text.contains("Preset pack: blocked"), "{text}");
                assert!(text.contains("error[output_write_failed]"), "{text}");
                assert!(!text.contains("created"), "{text}");
            }
            if force {
                assert_eq!(fs::read(output).unwrap(), b"preserve");
            } else {
                assert!(!output.exists());
            }
            assert_eq!(fs::read(&blocker).unwrap(), b"not a directory");
            assert_eq!(
                fs::read_to_string(category.join("shine.toml")).unwrap(),
                metadata
            );
            assert_eq!(fs::read(category.join("config.txt")).unwrap(), b"demo");
            assert!(!fixture.0.join("state").exists());
        }
    }
}

#[cfg(unix)]
#[test]
fn preset_pull_preserves_selected_path_and_exit_contract() {
    for selected_name in ["selected ", "selected\nFORGED=git\r\t\u{1b} "] {
        let fixture = Fixture::new();
        let source = fixture.0.join("source");
        let selected = fixture.0.join(selected_name);
        let neighbor = fixture.0.join("selected");
        let git = |dir: &std::path::Path, args: &[&str]| {
            let output = fixture
                .configure(Command::new("git"))
                .current_dir(dir)
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success(), "git {args:?}: {output:?}");
        };
        git(&fixture.0, &["init", source.to_str().unwrap()]);
        git(&source, &["config", "user.name", "Shine Tests"]);
        git(&source, &["config", "user.email", "shine@example.invalid"]);
        fs::write(source.join("preset.txt"), "one\n").unwrap();
        git(&source, &["add", "preset.txt"]);
        git(&source, &["commit", "-m", "initial"]);
        for dir in [&selected, &neighbor] {
            git(
                &fixture.0,
                &["clone", source.to_str().unwrap(), dir.to_str().unwrap()],
            );
        }
        let state = fixture.0.join("state");
        fs::create_dir_all(&state).unwrap();
        fs::write(
            state.join("config.toml"),
            format!(
                "schema_version=2\npresets_overlay_dir={}\n",
                toml::Value::String(selected.to_str().unwrap().into())
            ),
        )
        .unwrap();
        // SHINE_CONFIG_DIR selects state/presets before SHINE_PRESETS in global mode.
        std::os::unix::fs::symlink(&selected, state.join("presets")).unwrap();
        fs::write(state.join("update-check.json"), serde_json::to_vec(&serde_json::json!({
        "latest_version": env!("CARGO_PKG_VERSION"),
        "checked_at_unix_secs": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
    })).unwrap()).unwrap();
        let pull = || {
            fixture
                .command()
                .env("NO_COLOR", "1")
                .args(["preset", "pull"])
                .output()
                .unwrap()
        };
        fs::write(selected.join("preset.txt"), "user changes\n").unwrap();

        let refused = pull();

        assert_eq!(refused.status.code(), Some(1), "{refused:?}");
        let stderr = String::from_utf8(refused.stderr).unwrap();
        assert!(stderr.contains("uncommitted changes"), "{stderr}");
        assert!(!selected.join(".git/FETCH_HEAD").exists());
        assert!(!neighbor.join(".git/FETCH_HEAD").exists());
        assert_eq!(
            fs::read(selected.join("preset.txt")).unwrap(),
            b"user changes\n"
        );
        git(&selected, &["checkout", "--", "preset.txt"]);
        fs::write(source.join("preset.txt"), "two\n").unwrap();
        git(&source, &["commit", "-am", "update"]);

        let updated = pull();

        assert_eq!(updated.status.code(), Some(0), "{updated:?}");
        assert!(updated.stderr.is_empty(), "{updated:?}");
        let stdout = String::from_utf8(updated.stdout).unwrap();
        assert!(stdout.contains("preset source updated"), "{stdout}");
        assert_eq!(fs::read(selected.join("preset.txt")).unwrap(), b"two\n");
        assert_eq!(fs::read(neighbor.join("preset.txt")).unwrap(), b"one\n");
        assert!(!neighbor.join(".git/FETCH_HEAD").exists());
        let verbose = fixture
            .command()
            .env("NO_COLOR", "1")
            .args(["update", "--pull", "--verbose"])
            .output()
            .unwrap();
        assert!(verbose.status.success(), "{verbose:?}");
        let text = String::from_utf8(verbose.stdout).unwrap();
        assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
        assert!(
            !text.lines().any(|line| line.starts_with("FORGED=")),
            "{text:?}"
        );
        assert!(
            text.contains("same repository:") && text.contains("Pulling preset source from"),
            "{text}"
        );
        assert_eq!(fs::read(selected.join("preset.txt")).unwrap(), b"two\n");
        assert_eq!(fs::read(neighbor.join("preset.txt")).unwrap(), b"one\n");
    }
}

#[test]
fn external_overlay_completion_matches_catalog_without_loading_trust_or_writing_state() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let overlay = fixture.0.join("overlay");
    let script = if cfg!(windows) { "tool.ps1" } else { "tool.sh" };
    for (root, name) in [(state.join("presets"), "base"), (overlay.clone(), "extra")] {
        let category = root.join("shell").join(name);
        fs::create_dir_all(&category).unwrap();
        fs::write(
            category.join("shine.toml"),
            format!("[[files]]\nsource = '{script}'\ntarget = 'tool'\n"),
        )
        .unwrap();
        fs::write(category.join(script), "# never executed\n").unwrap();
    }
    let config = format!(
        "schema_version = 2\npresets_overlay_dir = {:?}\n",
        overlay.to_str().unwrap()
    );
    fs::write(state.join("config.toml"), &config).unwrap();
    // Keep the catalog comparison offline; completion itself must not recreate this cache.
    let cache = state.join("update-check.json");
    fs::write(&cache, serde_json::to_vec(&serde_json::json!({
        "latest_version": env!("CARGO_PKG_VERSION"),
        "checked_at_unix_secs": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
    })).unwrap()).unwrap();
    let catalog = fixture
        .command()
        .env("NO_COLOR", "1")
        .args(["shell", "list"])
        .output()
        .unwrap();
    assert_eq!(catalog.status.code(), Some(0), "{catalog:?}");
    assert!(catalog.stderr.is_empty(), "{catalog:?}");
    let text = String::from_utf8(catalog.stdout).unwrap();
    assert!(text.contains("base") && text.contains("extra"), "{text}");
    fs::remove_file(&cache).unwrap();

    // Completion runs before runtime construction, including trust-store parsing.
    fs::write(state.join("trust.toml"), "malformed [").unwrap();
    let completed = fixture
        .command()
        .env("COMPLETE", "bash")
        .env("_CLAP_COMPLETE_INDEX", "3")
        .env("_CLAP_IFS", "\n")
        .args(["--", "shine", "shell", "install", ""])
        .output()
        .unwrap();
    assert_eq!(completed.status.code(), Some(0), "{completed:?}");
    assert!(completed.stderr.is_empty(), "{completed:?}");
    let values = String::from_utf8(completed.stdout).unwrap();
    for expected in ["base", "base/tool", "extra", "extra/tool"] {
        assert!(values.lines().any(|line| line == expected), "{values}");
    }
    assert!(!values.contains("proxy"), "{values}");
    assert_eq!(
        fs::read_to_string(state.join("config.toml")).unwrap(),
        config
    );
    assert_eq!(
        fs::read_to_string(state.join("trust.toml")).unwrap(),
        "malformed ["
    );
    for absent in [
        "shell-manifest.toml",
        "app-manifest.toml",
        "installed",
        "update-check.json",
    ] {
        assert!(!state.join(absent).exists(), "unexpected {absent}");
    }
}

#[test]
#[cfg(unix)]
fn app_generator_notes_keep_stderr_and_hook_notes_keep_stdout() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let preset = state.join("presets/app/demo");
    let destination = fixture.0.join("destination");
    fs::create_dir_all(&preset).unwrap();
    let dest = toml::Value::String(destination.to_string_lossy().into_owned());
    fs::write(preset.join("shine.toml"), format!(
        "metadata_schema_version = 2\ndest = {dest}\npost_install = {{ command = '/bin/sh', args = ['-c', 'printf HOOK_STDOUT_NOTE'], show_output = true }}\npost_upgrade = {{ command = '/bin/sh', args = ['-c', 'printf HOOK_STDOUT_NOTE'], show_output = true }}\n[permissions]\nschema_version = 1\ncommands = ['/bin/sh']\nfilesystem = [{{ access = ['execute'], base = 'preset', path = 'generate.sh' }}]\nenvironment = [{{ name = 'ENABLE', sensitivity = 'plain' }}]\n[[files]]\nsource = 'config.txt'\ngenerator = {{ script = 'generate.sh', env = [{{ source = 'ENABLE', sensitivity = 'plain' }}], when_env = 'ENABLE' }}\n"
    )).unwrap();
    fs::write(preset.join("config.txt"), "fallback").unwrap();
    let script = preset.join("generate.sh");
    fs::write(
        &script,
        "#!/bin/sh\nprintf '%s' \"$ENABLE\"\nprintf 'GENERATOR_STDERR_NOTE\\nsecond line\\n' >&2\n",
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let run = |args: &[&str]| {
        fixture
            .command()
            .env("NO_COLOR", "1")
            .args(args)
            .output()
            .unwrap()
    };
    let config = state.join("config.toml");
    fs::write(&config, "schema_version = 2\n[env]\nENABLE = '1'\n").unwrap();
    let trusted = run(&["trust", "grant", "app/demo", "--yes"]);
    assert!(trusted.status.success(), "{trusted:?}");
    let trust = fs::read(state.join("trust.toml")).unwrap();
    for (value, args) in [
        ("1", vec!["app", "install", "demo", "--yes"]),
        ("2", vec!["upgrade", "app/demo", "--yes", "--verbose"]),
        ("3", vec!["app", "refresh", "demo", "config.txt", "--yes"]),
    ] {
        let contents = format!("schema_version = 2\n[env]\nENABLE = '{value}'\n");
        fs::write(&config, &contents).unwrap();
        let output = run(&args);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "     GENERATOR_STDERR_NOTE\n     second line\n"
        );
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(text.contains("HOOK_STDOUT_NOTE"), "{output:?}");
        assert!(!text.contains("GENERATOR_STDERR_NOTE"), "{output:?}");
        assert_eq!(
            fs::read(destination.join("config.txt")).unwrap(),
            value.as_bytes()
        );
        assert_eq!(fs::read_to_string(&config).unwrap(), contents);
        assert_eq!(fs::read(state.join("trust.toml")).unwrap(), trust);
        assert!(!state.join("app-operation-journal.toml").exists());
    }
    // A no-op refresh still renders the note without changing content or receipts.
    let manifest = fs::read(state.join("app-manifest.toml")).unwrap();
    let noop = run(&["app", "refresh", "demo", "--yes"]);
    assert_eq!(noop.status.code(), Some(0), "{noop:?}");
    assert!(String::from_utf8_lossy(&noop.stdout).contains("Already up to date"));
    assert_eq!(
        String::from_utf8_lossy(&noop.stderr),
        "     GENERATOR_STDERR_NOTE\n     second line\n"
    );
    assert_eq!(fs::read(state.join("app-manifest.toml")).unwrap(), manifest);
    assert_eq!(fs::read(destination.join("config.txt")).unwrap(), b"3");
}

#[test]
fn shell_install_preview_labels_candidates_without_changing_conflict_policy() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let preset = state.join("presets/shell/demo");
    fs::create_dir_all(&preset).unwrap();
    let source = if cfg!(windows) { "run.ps1" } else { "run.sh" };
    fs::write(
        preset.join("shine.toml"),
        format!("[[files]]\nsource = '{source}'\ntarget = 'demo-run'\n"),
    )
    .unwrap();
    fs::write(preset.join(source), "# fixture; never executed\n").unwrap();
    let config = "schema_version = 2\n";
    fs::write(state.join("config.toml"), config).unwrap();
    fs::create_dir_all(state.join("bin")).unwrap();
    let launcher = state.join("bin").join(if cfg!(windows) {
        "demo-run.ps1"
    } else {
        "demo-run"
    });
    fs::write(&launcher, "foreign launcher\n").unwrap();
    let run = |args: &[&str]| {
        fixture
            .command()
            .env("NO_COLOR", "1")
            .args(args)
            .output()
            .unwrap()
    };
    let trusted = run(&["trust", "grant", "shell/demo/demo-run", "--yes"]);
    assert_eq!(trusted.status.code(), Some(0), "{trusted:?}");
    let trust = fs::read(state.join("trust.toml")).unwrap();
    for flags in [vec![], vec!["--replace-managed"], vec!["--verbose"]] {
        let mut args = vec!["shell", "install", "demo", "--dry-run"];
        args.extend(flags);
        let output = run(&args);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(
            text.contains("link candidates only (not a security Plan)"),
            "{text}"
        );
        assert!(text.contains("Link candidate demo-run:"), "{text}");
        assert!(text.contains("--replace-managed effects"), "{text}");
        assert!(
            text.contains("--verbose does not expand this preview"),
            "{text}"
        );
        assert!(!text.contains("Would link"), "{text}");
    }
    let blocked = run(&["shell", "install", "demo", "--yes"]);
    assert_eq!(blocked.status.code(), Some(1), "{blocked:?}");
    assert!(
        String::from_utf8_lossy(&blocked.stderr).contains("ownership conflict"),
        "{blocked:?}"
    );
    assert_eq!(fs::read_to_string(&launcher).unwrap(), "foreign launcher\n");
    assert_eq!(
        fs::read_to_string(state.join("config.toml")).unwrap(),
        config
    );
    assert_eq!(fs::read(state.join("trust.toml")).unwrap(), trust);
    for path in [
        "installed/shell",
        "shell",
        "shell-manifest.toml",
        "shell-operation-journal.toml",
    ] {
        assert!(!state.join(path).exists(), "{path}");
    }
    assert!(!fixture.0.join("home/.zshrc").exists());
    assert!(!fixture.0.join("home/Documents/PowerShell").exists());
}

#[test]
fn preset_lint_text_counts_preserve_json_and_warning_exits() {
    let fixture = Fixture::new();
    let category = fixture.0.join("app/demo");
    fs::create_dir_all(&category).unwrap();
    fs::write(category.join("shine.toml"), "metadata_schema_version = 2\ndescription = 'Demo'\ndest = '~/.config/demo'\n[[files]]\nsource = 'config.txt'\n").unwrap();
    fs::write(category.join("config.txt"), "fixture\n").unwrap();
    for (missing, expected, exit) in [
        (false, "Summary: 1 category, 0 errors, 1 warning", 0),
        (true, "Summary: 0 categories, 1 error, 0 warnings", 1),
    ] {
        let path = if missing {
            fixture.0.join("missing")
        } else {
            category.clone()
        };
        for deny in [false, true] {
            let run = |json| {
                let mut command = fixture.command();
                command
                    .env("NO_COLOR", "1")
                    .args(["preset", "lint"])
                    .arg(&path);
                if deny {
                    command.arg("--deny-warnings");
                }
                if json {
                    command.args(["--format", "json"]);
                }
                command.output().unwrap()
            };
            let text = run(false);
            let json = run(true);
            let expected_exit = if deny { 1 } else { exit };
            assert_eq!(text.status.code(), Some(expected_exit), "{text:?}");
            assert_eq!(json.status.code(), text.status.code(), "{json:?}");
            assert!(text.stderr.is_empty() && json.stderr.is_empty());
            assert!(
                String::from_utf8_lossy(&text.stdout).contains(expected),
                "{text:?}"
            );
            let report: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
            assert_eq!(
                report["summary"],
                if missing {
                    serde_json::json!({"categories": 0, "errors": 1, "warnings": 0})
                } else {
                    serde_json::json!({"categories": 1, "errors": 0, "warnings": 1})
                }
            );
        }
    }
    assert!(!fixture.0.join("state").exists());
}

fn sys_output_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.0 = fs::canonicalize(&fixture.0).unwrap();
    let state = fixture.0.join("state");
    for os in ["macos", "ubuntu", "windows"] {
        let preset = state.join("presets/sys").join(os);
        fs::create_dir_all(&preset).unwrap();
        let provider = if os == "windows" {
            "winget"
        } else {
            "homebrew"
        };
        fs::write(preset.join("shine.toml"), format!(r#"version = 2
[[items]]
id = "--help"
label = "Bootstrap"
permissions = {{ schema_version = 1 }}
detect = {{ kind = "command", command = "if", version_args = ["two words", "it's", "$HOME", "a;b", "`literal`", "中文"] }}
install = {{ kind = "package", provider = "{provider}", package = "tool" }}
[[items]]
id = "--version"
label = "Managed\n\r\t\u001b[2J中文é"
mode = "managed"
driver = "managed-file"
permissions = {{ schema_version = 1 }}
[items.config]
source = "managed.txt"
target = "~/.config/managed.txt"
[[items]]
id = "plain"
label = "Plain"
mode = "managed"
driver = "managed-file"
permissions = {{ schema_version = 1 }}
[items.config]
source = "managed.txt"
target = "~/.config/plain.txt"
"#)).unwrap();
        fs::write(preset.join("managed.txt"), "managed fixture\n").unwrap();
    }
    fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
    fs::write(state.join("update-check.json"), serde_json::to_vec(&serde_json::json!({
        "latest_version": env!("CARGO_PKG_VERSION"),
        "checked_at_unix_secs": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
    })).unwrap()).unwrap();
    fixture
}

#[tokio::test]
async fn sys_info_encodes_recorded_detail_preserving_failed_status_and_bytes() {
    let fixture = sys_output_fixture();
    let state = fixture.0.join("state");
    let os = cli::sys::detect_os_id().await.unwrap();
    let mut manifest = shine_core::runtime::SysRunManifest::default();
    manifest.entries.push(shine_core::runtime::SysRunEntry {
        os_id: os,
        item_id: "--help".into(),
        label: "Bootstrap".into(),
        status: shine_core::runtime::SysItemStatus::Failed,
        detail: "detail\nFORGED=status\r\t\u{1b}\\literal中文".into(),
        updated_at: "fixture".into(),
        managed: false,
        profile_enabled: false,
        receipt: None,
    });
    let original = toml::to_string(&manifest).unwrap();
    fs::write(state.join("sys-manifest.toml"), &original).unwrap();
    let output = fixture
        .command()
        .env("NO_COLOR", "1")
        .args(["sys", "info", "--", "--help"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Status         failed"), "{text:?}");
    assert!(
        text.contains("Status detail  detail\\nFORGED=status\\r\\t\\u{1b}\\literal中文"),
        "{text:?}"
    );
    assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
    assert_eq!(
        fs::read_to_string(state.join("sys-manifest.toml")).unwrap(),
        original
    );
}

#[test]
fn sys_catalog_descriptions_encode_controls_without_changing_query_state() {
    let fixture = sys_output_fixture();
    let description = "DETAIL\nFORGED=description\r\t\u{1b}[31m\\literal中文";
    let encoded = "DETAIL\\nFORGED=description\\r\\t\\u{1b}[31m\\literal中文";
    let mut sources = Vec::new();
    for os in ["macos", "ubuntu", "windows"] {
        let path = fixture
            .0
            .join("state/presets/sys")
            .join(os)
            .join("shine.toml");
        let mut manifest: toml::Table =
            toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        manifest.insert(
            "description".into(),
            toml::Value::String(description.into()),
        );
        for item in manifest["items"].as_array_mut().unwrap() {
            item.as_table_mut().unwrap().insert(
                "description".into(),
                toml::Value::String(description.into()),
            );
        }
        let bytes = toml::to_string(&manifest).unwrap().into_bytes();
        fs::write(&path, &bytes).unwrap();
        sources.push((path, bytes));
    }
    for path in ["config.toml", "update-check.json"] {
        let path = fixture.0.join("state").join(path);
        sources.push((path.clone(), fs::read(path).unwrap()));
    }
    for args in [
        vec!["sys", "list"],
        vec!["sys", "list", "--all"],
        vec!["sys", "info", "--", "--help"],
        vec!["sys", "info", "--", "--version"],
    ] {
        let result = fixture
            .command()
            .env_remove("SUDO_USER")
            .env("NO_COLOR", "1")
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(0), "{args:?}: {result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        let text = String::from_utf8(result.stdout).unwrap();
        assert!(text.contains(encoded), "{text:?}");
        assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
        assert!(
            !text.lines().any(|line| line.starts_with("FORGED=")),
            "{text:?}"
        );
        for (path, before) in &sources {
            assert_eq!(&fs::read(path).unwrap(), before, "{path:?}");
        }
        for path in [
            "trust.toml",
            "sys-manifest.toml",
            "app-manifest.toml",
            "shell-manifest.toml",
        ] {
            assert!(!fixture.0.join("state").join(path).exists());
        }
    }
}

#[test]
fn sys_output_hints_preserve_leading_hyphen_ids_through_shell_and_clap() {
    let fixture = sys_output_fixture();
    for (id, action) in [
        ("--help", "bootstrap"),
        ("--version", "apply"),
        ("plain", "apply"),
    ] {
        let info = fixture
            .command()
            .env("NO_COLOR", "1")
            .args(["sys", "info", "--", id])
            .output()
            .unwrap();
        assert_eq!(info.status.code(), Some(0), "{info:?}");
        let text = String::from_utf8(info.stdout).unwrap();
        let hint = text
            .split_once("Next: run `")
            .unwrap()
            .1
            .split('`')
            .next()
            .unwrap();
        let operand = if id.starts_with('-') {
            format!("-- {id}")
        } else {
            id.to_string()
        };
        assert_eq!(hint, format!("shine sys {action} {operand}"));
        let preview = hint.replace(
            &format!("sys {action} "),
            &format!("sys {action} --dry-run "),
        );
        #[cfg(unix)]
        let shell = {
            let mut command = Command::new("/bin/sh");
            command.args([
                "-c",
                &format!(
                    "shine() {{ {} \"$@\"; }}; {preview}",
                    shine_core::shell_quote::posix_single_quote(env!("CARGO_BIN_EXE_shine"))
                ),
            ]);
            command
        };
        #[cfg(windows)]
        let shell = {
            let mut command = Command::new("powershell.exe");
            command.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!(
                    "Set-Alias shine {}; {preview}; exit $LASTEXITCODE",
                    shine_core::shell_quote::powershell_single_quote(env!("CARGO_BIN_EXE_shine"))
                ),
            ]);
            command
        };
        let copied = fixture
            .configure(shell)
            .env("NO_COLOR", "1")
            .output()
            .unwrap();
        let direct = fixture
            .command()
            .env("NO_COLOR", "1")
            .args(["sys", action, "--dry-run", "--", id])
            .output()
            .unwrap();
        assert_eq!(direct.status.code(), Some(0), "{direct:?}");
        assert_eq!(copied.status.code(), direct.status.code(), "{copied:?}");
        assert_eq!(
            String::from_utf8_lossy(&copied.stdout)
                .lines()
                .collect::<Vec<_>>(),
            String::from_utf8_lossy(&direct.stdout)
                .lines()
                .collect::<Vec<_>>()
        );
        assert_eq!(copied.stderr, direct.stderr);
    }
    let listed = fixture.command().args(["sys", "list"]).output().unwrap();
    let text = String::from_utf8(listed.stdout).unwrap();
    assert!(text.contains("Run: shine sys apply -- --version"), "{text}");
    assert!(text.contains("Run: shine sys apply plain"), "{text}");
    assert!(!fixture.0.join("state/sys-manifest.toml").exists());
    assert!(!fixture.0.join("home/.config/managed.txt").exists());
}

#[test]
fn sys_output_labels_escape_controls_without_changing_receipts_or_approval() {
    let fixture = sys_output_fixture();
    let run = |args: &[&str]| {
        fixture
            .command()
            .env("NO_COLOR", "1")
            .args(args)
            .output()
            .unwrap()
    };
    let raw = "Managed\n\r\t\u{1b}[2J中文é";
    let displayed = r"Managed\n\r\t\u{1b}[2J中文é";
    for args in [
        vec!["sys", "list"],
        vec!["sys", "info", "--", "--version"],
        vec!["sys", "apply", "--dry-run", "--", "--version"],
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains(displayed), "{text}");
        assert!(
            !text.contains(raw) && !text.contains(['\r', '\t', '\u{1b}']),
            "{text:?}"
        );
    }
    let denied = run(&["sys", "apply", "--", "--version"]);
    assert_eq!(denied.status.code(), Some(1), "{denied:?}");
    assert!(String::from_utf8_lossy(&denied.stderr).contains("approval requires"));
    let receipt = fixture.0.join("state/sys-manifest.toml");
    assert!(!receipt.exists());
    let destination = fixture.0.join("home/.config/managed.txt");
    assert!(!destination.exists());
    let applied = run(&["sys", "apply", "--yes", "--", "--version"]);
    assert_eq!(applied.status.code(), Some(0), "{applied:?}");
    let text = String::from_utf8(applied.stdout).unwrap();
    assert!(text.contains(displayed) && !text.contains(raw), "{text}");
    assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
    assert_eq!(fs::read(&destination).unwrap(), b"managed fixture\n");
    let before = fs::read(&receipt).unwrap();
    let manifest: shine_core::runtime::SysRunManifest =
        toml::from_str(std::str::from_utf8(&before).unwrap()).unwrap();
    assert_eq!(manifest.entries[0].label, raw);
    for args in [
        vec!["sys", "status"],
        vec!["update", "--verbose"],
        vec!["sys", "info", "--", "--version"],
        vec!["sys", "uninstall", "--dry-run", "--", "--version"],
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(0), "{args:?}: {output:?}");
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains(displayed) && !text.contains(raw), "{text}");
        assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
        if args.contains(&"info") {
            assert!(text.contains("`shine sys apply -- --version`"), "{text}");
            assert!(
                text.contains("`shine sys uninstall -- --version`"),
                "{text}"
            );
        }
        assert_eq!(fs::read(&receipt).unwrap(), before);
        assert_eq!(fs::read(&destination).unwrap(), b"managed fixture\n");
    }
}

#[test]
fn sys_output_detection_preserves_literal_argv_in_the_native_shell() {
    let fixture = sys_output_fixture();
    let output = fixture
        .command()
        .env("NO_COLOR", "1")
        .args(["sys", "info", "--", "--help"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    let detection = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("Detection"))
        .unwrap()
        .trim();
    #[cfg(unix)]
    let shell = {
        use std::os::unix::fs::PermissionsExt;
        let bin = fixture.0.join("bin");
        fs::create_dir(&bin).unwrap();
        let program = bin.join("if");
        fs::write(&program, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        let mut command = Command::new("/bin/sh");
        command.env("PATH", bin).args(["-c", detection]);
        command
    };
    #[cfg(windows)]
    let shell = {
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", &format!("[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); Set-Item -Path Function:if -Value {{ $args | ForEach-Object {{ [Console]::WriteLine($_) }} }}; {detection}")]);
        command
    };
    let copied = fixture.configure(shell).output().unwrap();
    assert_eq!(copied.status.code(), Some(0), "{copied:?}");
    assert!(copied.stderr.is_empty(), "{copied:?}");
    assert_eq!(
        String::from_utf8(copied.stdout)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["two words", "it's", "$HOME", "a;b", "`literal`", "中文"]
    );
    assert!(!fixture.0.join("state/sys-manifest.toml").exists());
}

#[test]
fn lint_validation_hints_recheck_the_original_input_without_changing_json() {
    let fixture = Fixture::new();
    let good = fixture.0.join("app/good");
    fs::create_dir_all(&good).unwrap();
    // PowerShell expands Windows 8.3 cwd aliases; give both launches the same spelling,
    // including when the missing operand cannot itself be canonicalized.
    let good = fs::canonicalize(good).unwrap();
    let metadata =
        "metadata_schema_version = 2\ndest = '~/.config/good'\n[[files]]\nsource = 'config.txt'\n";
    fs::write(good.join("shine.toml"), metadata).unwrap();
    fs::write(good.join("config.txt"), "fixture\n").unwrap();
    for name in ["bad space", "it's"] {
        let bad = fixture.0.join("app").join(name);
        fs::create_dir_all(&bad).unwrap();
        fs::write(bad.join("shine.toml"), "[[files]\n").unwrap();
    }
    let default = fixture
        .command()
        .current_dir(&good)
        .args(["preset", "validate"])
        .output()
        .unwrap();
    assert_eq!(default.status.code(), Some(0), "{default:?}");
    for (path, missing) in [
        ("../bad space", false),
        ("../it's", false),
        ("-missing", true),
        ("../missing space", true),
    ] {
        let run = |format| {
            fixture
                .command()
                .current_dir(&good)
                .args(["preset", "lint", "--format", format, "--", path])
                .output()
                .unwrap()
        };
        let text = run("text");
        let json = run("json");
        for output in [&text, &json] {
            assert_eq!(output.status.code(), Some(1), "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
        }
        let text = String::from_utf8(text.stdout).unwrap();
        let hint = text
            .split_once("run `")
            .unwrap()
            .1
            .split('`')
            .next()
            .unwrap();
        #[cfg(unix)]
        let shell = {
            let mut command = Command::new("/bin/sh");
            command.args([
                "-c",
                &format!(
                    "shine() {{ {} \"$@\"; }}; {hint}",
                    shine_core::shell_quote::posix_single_quote(env!("CARGO_BIN_EXE_shine"))
                ),
            ]);
            command
        };
        #[cfg(windows)]
        let shell = {
            let mut command = Command::new("powershell.exe");
            command.args(["-NoProfile", "-NonInteractive", "-Command"]);
            command.arg(format!(
                "Set-Alias shine {}; {hint}; exit $LASTEXITCODE",
                shine_core::shell_quote::powershell_single_quote(env!("CARGO_BIN_EXE_shine"))
            ));
            command
        };
        let hinted = fixture
            .configure(shell)
            .current_dir(&good)
            .output()
            .unwrap();
        let explicit = fixture
            .command()
            .current_dir(&good)
            .args(["preset", "validate", "--", path])
            .output()
            .unwrap();
        assert_eq!(hinted.status.code(), Some(1), "{hinted:?}");
        // PowerShell may re-emit native stdout with CRLF line endings.
        assert_eq!(
            std::str::from_utf8(&hinted.stdout)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            std::str::from_utf8(&explicit.stdout)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            "{hint}"
        );
        assert_eq!(hinted.stderr, explicit.stderr, "{hint}");
        let report: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
        assert_eq!(report["valid"], false);
        assert_eq!(
            report["diagnostics"][0]["message"],
            if missing {
                "preset input could not be loaded; run `shine preset validate` for path details"
            } else {
                "static validation failed; run `shine preset validate` for details"
            }
        );
    }
    assert_eq!(
        fs::read_to_string(good.join("shine.toml")).unwrap(),
        metadata
    );
    assert!(!fixture.0.join("state").exists());
}

#[cfg(unix)]
#[test]
fn lint_non_utf8_path_does_not_suggest_a_different_valid_input() {
    use std::os::unix::ffi::OsStringExt;
    let fixture = Fixture::new();
    let original = PathBuf::from(std::ffi::OsString::from_vec(
        b"checkout-\xff/app/demo".to_vec(),
    ));
    let replacement = fixture.0.join("checkout-\u{fffd}/app/demo");
    fs::create_dir_all(&replacement).unwrap();
    let metadata =
        "metadata_schema_version = 2\ndest = '~/.config/demo'\n[[files]]\nsource = 'config.txt'\n";
    fs::write(replacement.join("shine.toml"), metadata).unwrap();
    fs::write(replacement.join("config.txt"), "fixture\n").unwrap();
    // The lossy spelling is a distinct, valid input. Never suggest validating it.
    let valid = fixture
        .command()
        .args(["preset", "validate"])
        .arg(&replacement)
        .output()
        .unwrap();
    assert_eq!(valid.status.code(), Some(0), "{valid:?}");
    let run = |format| {
        fixture
            .command()
            .args(["preset", "lint", "--format", format, "--"])
            .arg(&original)
            .output()
            .unwrap()
    };
    let text = run("text");
    let json = run("json");
    for output in [&text, &json] {
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
    }
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(
        text.contains(
            "preset validate with the original PATH (non-UTF-8; copyable command omitted)"
        ),
        "{text}"
    );
    assert!(!text.contains("`shine preset validate"), "{text}");
    assert!(!text.contains('\u{fffd}'), "{text}");
    let report: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(report["valid"], false);
    assert_eq!(report["diagnostics"][0]["code"], "invalid_input");
    assert_eq!(
        report["diagnostics"][0]["message"],
        "preset input could not be loaded; run `shine preset validate` for path details"
    );
    assert_eq!(
        fs::read_to_string(replacement.join("shine.toml")).unwrap(),
        metadata
    );
    assert_eq!(
        fs::read(replacement.join("config.txt")).unwrap(),
        b"fixture\n"
    );
    assert!(!fixture.0.join("state").exists());
}

#[test]
fn broker_policy_list_escapes_controls_without_changing_stored_values() {
    for (project, ssh_target, row) in [
        (
            "API's \"build\" \\ path",
            "example.invalid",
            "review: example.invalid (API's \"build\" \\ path)\n",
        ),
        (
            "api)\nforged: host (injected\r\t\u{1b}[31m",
            "example.invalid\talias",
            r"review: example.invalid\talias (api)\nforged: host (injected\r\t\u{1b}[31m)",
        ),
    ] {
        let fixture = Fixture::new();
        let workspace = fixture.0.join("shine.workspace.toml");
        fs::write(&workspace, "version = 2\n[env]\nfiles = ['env.toml']\n").unwrap();
        let source = "version = 1\n[secret]\nTOKEN = true\n[payload]\ndata = 'not-decrypted-by-policy-commands'\n";
        fs::write(fixture.0.join("env.toml"), source).unwrap();
        let added = fixture
            .command()
            .args([
                "env",
                "broker",
                "policy",
                "add",
                "--name",
                "review",
                "--project",
                project,
                "--ssh-target",
                ssh_target,
                "--workspace",
            ])
            .arg(&workspace)
            .args([
                "--mode",
                "dev",
                "--release",
                "TOKEN",
                "--",
                "echo",
                "never executed",
            ])
            .output()
            .unwrap();
        assert_eq!(added.status.code(), Some(0), "{added:?}");
        assert!(added.stderr.is_empty(), "{added:?}");
        let policy = fixture.0.join("state/ssh-secret-broker.toml");
        let before = fs::read(&policy).unwrap();
        let listed = fixture
            .command()
            .args(["env", "broker", "policy", "list"])
            .output()
            .unwrap();
        assert_eq!(listed.status.code(), Some(0), "{listed:?}");
        assert!(listed.stderr.is_empty(), "{listed:?}");
        let text = String::from_utf8(listed.stdout).unwrap();
        assert_eq!(text.trim_end_matches('\n'), row.trim_end_matches('\n'));
        assert_eq!(text.lines().count(), 1);
        assert!(!text.contains(['\r', '\t', '\u{1b}']));
        let info = fixture
            .command()
            .args(["env", "broker", "policy", "info", "review"])
            .output()
            .unwrap();
        assert_eq!(info.status.code(), Some(0), "{info:?}");
        assert!(info.stderr.is_empty(), "{info:?}");
        let decoded: toml::Value =
            toml::from_str(std::str::from_utf8(&info.stdout).unwrap()).unwrap();
        assert_eq!(decoded["project"].as_str(), Some(project));
        assert_eq!(decoded["ssh_target"].as_str(), Some(ssh_target));
        assert_eq!(fs::read(&policy).unwrap(), before);
        assert_eq!(
            fs::read_to_string(fixture.0.join("env.toml")).unwrap(),
            source
        );
    }
}

#[test]
fn preset_test_text_keeps_control_names_on_one_row_and_preserves_json_and_exit_status() {
    let fixture = Fixture::new();
    let category = fixture.0.join("app/demo");
    fs::create_dir_all(&category).unwrap();
    let metadata =
        "metadata_schema_version = 2\ndest = '~/.config/demo'\n[[files]]\nsource = 'config.txt'\n";
    fs::write(category.join("shine.toml"), metadata).unwrap();
    fs::write(category.join("config.txt"), "fixture\n").unwrap();
    for (name, passed) in [
        ("ordinary name", true),
        ("case\n  PASS forged\r\t\u{1b}[31m", false),
    ] {
        let source = format!(
            "schema_version = 1\n[[cases]]\nname = {}\nplatform = 'linux'\n[cases.expect]\nvalid = {passed}\n",
            toml::Value::String(name.into())
        );
        fs::write(category.join("shine.test.toml"), &source).unwrap();
        let run = |json| {
            let mut command = fixture.command();
            command
                .env("NO_COLOR", "1")
                .args(["preset", "test"])
                .arg(&category);
            if json {
                command.args(["--format", "json"]);
            }
            command.output().unwrap()
        };
        let text = run(false);
        let json = run(true);
        for output in [&text, &json] {
            assert_eq!(output.status.code(), Some(i32::from(!passed)), "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
        }
        let text = String::from_utf8(text.stdout).unwrap();
        assert_eq!(
            text.lines()
                .filter(|line| line.starts_with("  PASS ") || line.starts_with("  FAIL "))
                .count(),
            1,
            "{text}"
        );
        assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
        if passed {
            assert!(text.contains("PASS ordinary name (linux)"), "{text}");
            assert!(text.contains("Summary: 1 case, 1 passed, 0 failed"));
        } else {
            assert!(
                text.contains(r"FAIL case\n  PASS forged\r\t\u{1b}[31m"),
                "{text}"
            );
            assert!(text.contains("Summary: 1 case, 0 passed, 1 failed"));
        }
        let report: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
        assert_eq!(report["cases"].as_array().unwrap().len(), 1);
        assert_eq!(report["cases"][0]["name"], name);
        assert_eq!(report["cases"][0]["passed"], passed);
        assert_eq!(
            fs::read_to_string(category.join("shine.test.toml")).unwrap(),
            source
        );
        assert_eq!(
            fs::read_to_string(category.join("shine.toml")).unwrap(),
            metadata
        );
    }
    assert!(!fixture.0.join("state").exists());
}

#[test]
fn dotenv_import_force_reports_writes_and_preserves_preview_and_conflict_boundaries() {
    let fixture = Fixture::new();
    let input = fixture.0.join(".env");
    fs::write(&input, "PUBLIC=original\n").unwrap();
    let run = |flags: &[&str]| {
        fixture
            .command()
            .args(["env", "workspace", "init", "--from-dotenv"])
            .args(flags)
            .output()
            .unwrap()
    };
    let preview = run(&["--dry-run"]);
    assert!(preview.status.success(), "{preview:?}");
    assert_eq!(
        String::from_utf8(preview.stdout).unwrap(),
        "Would create .env.shine.toml\nWould create shine.workspace.toml\n"
    );
    let source = fixture.0.join(".env.shine.toml");
    let workspace = fixture.0.join("shine.workspace.toml");
    assert!(!source.exists() && !workspace.exists());
    let created = run(&[]);
    assert!(created.status.success(), "{created:?}");
    assert_eq!(
        String::from_utf8(created.stdout).unwrap(),
        "Created .env.shine.toml\nCreated shine.workspace.toml\n"
    );
    let original = [fs::read(&source).unwrap(), fs::read(&workspace).unwrap()];
    fs::write(&input, "PUBLIC=updated\n").unwrap();
    for flags in [&["--force", "--dry-run"][..], &[][..]] {
        let output = run(flags);
        if flags.is_empty() {
            assert_eq!(output.status.code(), Some(1), "{output:?}");
            assert!(String::from_utf8_lossy(&output.stderr).contains("already exists"));
        } else {
            assert!(output.status.success(), "{output:?}");
            assert_eq!(
                String::from_utf8(output.stdout).unwrap(),
                "Would write (replace if present) .env.shine.toml\nWould write (replace if present) shine.workspace.toml\n"
            );
        }
        assert_eq!(
            [fs::read(&source).unwrap(), fs::read(&workspace).unwrap()],
            original
        );
    }
    let written = run(&["--force"]);
    assert!(written.status.success(), "{written:?}");
    assert_eq!(
        String::from_utf8(written.stdout).unwrap(),
        "Wrote (replacing if present) .env.shine.toml\nWrote (replacing if present) shine.workspace.toml\n"
    );
    let imported: toml::Value = toml::from_str(&fs::read_to_string(&source).unwrap()).unwrap();
    assert_eq!(imported["plain"]["PUBLIC"].as_str(), Some("updated"));
    assert_eq!(fs::read(&workspace).unwrap(), original[1]);
    assert_eq!(fs::read_to_string(&input).unwrap(), "PUBLIC=updated\n");
    assert!(!fixture.0.join("state").exists());
}

#[tokio::test]
async fn shell_recovery_reports_rollback_and_committed_cleanup_without_reversing_committed_content()
{
    use shine_core::action::{
        ActionIrV1, DeclarativeActionV1, ShellFileIdentityV1, ShellLauncherReceiptV1,
        ShellReceiptTransitionV1, ShellRenderedFileReplacementSpecV1, managed_file_rollback_path,
    };
    use shine_core::install::hash_content;
    use shine_core::runtime::{FileSystemObservationHost, RealHost};

    for committed in [false, true] {
        let fixture = Fixture::new();
        let state = fixture.0.join("state");
        let presets = state.join("presets");
        let destination = state.join("rendered/demo.ts");
        let rollback = managed_file_rollback_path(&destination);
        fs::create_dir_all(&presets).unwrap();
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
        fs::write(&destination, b"new content\n").unwrap();
        fs::write(&rollback, b"old content\n").unwrap();
        let previous_mode = RealHost.metadata(&rollback).await.unwrap().unix_mode;
        let desired_mode = RealHost.metadata(&destination).await.unwrap().unix_mode;
        let previous = ShellLauncherReceiptV1 {
            launcher_format: None,
            launcher_config_dir: None,
            category: "demo".into(),
            command: "run".into(),
            mode: "snapshot".into(),
            source_path: presets.join("shell/demo/run.ts"),
            rendered_path: destination.clone(),
            runtime: "bun".into(),
            bun_dependencies: None,
            dependency_hash: None,
            transforms: vec!["template".into()],
            env: Vec::new(),
            needs_source: false,
            content_hash: hash_content(b"old source\n"),
        };
        let desired = ShellLauncherReceiptV1 {
            content_hash: hash_content(b"new source\n"),
            ..previous.clone()
        };
        let manifest = toml::to_string(&serde_json::json!({
            "schema_version": 1, "entries": [if committed { &desired } else { &previous }],
        }))
        .unwrap();
        fs::write(state.join("shell-manifest.toml"), &manifest).unwrap();
        let action_ir = ActionIrV1::new(
            "recovery-output-fixture",
            vec![DeclarativeActionV1::replace_shell_rendered_file(
                "rendered",
                "shell/demo/run",
                "rendered",
                ShellRenderedFileReplacementSpecV1 {
                    destination: destination.clone(),
                    previous: Some(ShellFileIdentityV1 {
                        content_hash: hash_content(b"old content\n"),
                        unix_mode: previous_mode,
                    }),
                    desired: ShellFileIdentityV1 {
                        content_hash: hash_content(b"new content\n"),
                        unix_mode: desired_mode,
                    },
                    receipts: vec![ShellReceiptTransitionV1 {
                        target: "shell/demo/run".into(),
                        previous: Some(Box::new(previous)),
                        desired: Box::new(desired),
                    }],
                },
            )],
        );
        action_ir.validate().unwrap();
        // The persisted approval is historical data; the CLI must review a fresh recovery Plan.
        let journal = toml::to_string(&serde_json::json!({
            "schema_version": 1, "action_ir": action_ir, "applied": ["rendered"],
            "receipt_committed": if committed { vec!["rendered"] } else { vec![] },
            "approval": { "schema_version": shine_core::plan::PLAN_APPROVAL_SCHEMA_VERSION,
                "plan_fingerprint": "0".repeat(64), "approved_permissions": [] },
        }))
        .unwrap();
        let journal_path = state.join("shell-operation-journal.toml");
        fs::write(&journal_path, &journal).unwrap();
        let run = |args: &[&str]| {
            fixture
                .command()
                .env("SHINE_PRESETS", &presets)
                .env("NO_COLOR", "1")
                .args(args)
                .output()
                .unwrap()
        };
        let denied = run(&["shell", "recover"]);
        assert!(!denied.status.success(), "{denied:?}");
        assert!(
            String::from_utf8_lossy(&denied.stderr).contains("approval requires"),
            "{denied:?}"
        );
        assert_eq!(fs::read_to_string(&journal_path).unwrap(), journal);
        assert_eq!(fs::read(&destination).unwrap(), b"new content\n");
        assert_eq!(fs::read(&rollback).unwrap(), b"old content\n");
        let output = run(&["shell", "recover", "--yes"]);
        assert!(output.status.success(), "{output:?}");
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(
            text.contains("recovered 1 Shell resource action(s), including rollback or cleanup"),
            "{output:?}"
        );
        assert!(!text.contains("rolled back"), "{output:?}");
        assert_eq!(
            fs::read(&destination).unwrap(),
            if committed {
                b"new content\n"
            } else {
                b"old content\n"
            }
        );
        assert!(!rollback.exists() && !journal_path.exists());
        assert_eq!(
            fs::read_to_string(state.join("shell-manifest.toml")).unwrap(),
            manifest
        );
        if committed {
            fs::write(&journal_path, &journal).unwrap();
            let output = run(&["shell", "recover", "--yes"]);
            assert!(output.status.success(), "{output:?}");
            assert!(
                String::from_utf8_lossy(&output.stdout)
                    .contains("no Shell resource action needed recovery"),
                "{output:?}"
            );
            assert_eq!(fs::read(&destination).unwrap(), b"new content\n");
            assert!(!journal_path.exists());
        }
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
fn schema_output_errors_use_the_normal_failure_exit_without_initializing_state() {
    for format in ["text", "json"] {
        let fixture = Fixture::new();
        let (reader, writer) = std::io::pipe().unwrap();
        drop(reader);
        let output = fixture
            .command()
            .args(["preset", "schema", "--format", format])
            .stdout(writer)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.starts_with("Error:"), "{stderr}");
        assert!(
            !stderr.contains("panicked") && !stderr.contains("RUST_BACKTRACE"),
            "{stderr}"
        );
        assert!(!fixture.0.join("state").exists());
    }
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
fn targeted_upgrade_keeps_category_scope_consent_and_summary_contracts() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let presets = state.join("presets");
    let destination = fixture.0.join("destination");
    fs::create_dir_all(&state).unwrap();
    fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
    for category in ["demo", "other"] {
        let preset = presets.join("app").join(category);
        fs::create_dir_all(&preset).unwrap();
        let dest = toml::Value::String(destination.join(category).to_string_lossy().into_owned())
            .to_string();
        fs::write(preset.join("shine.toml"), format!("metadata_schema_version = 2\ndest = {dest}\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'one.txt'\n[[files]]\nsource = 'two.txt'\n")).unwrap();
        for file in ["one.txt", "two.txt"] {
            fs::write(preset.join(file), "v1").unwrap();
        }
    }
    let run = |args: &[&str]| {
        fixture
            .command()
            .env("SHINE_PRESETS", &presets)
            .args(args)
            .output()
            .unwrap()
    };
    for category in ["demo", "other"] {
        let installed = run(&["app", "install", category, "--yes"]);
        assert!(installed.status.success(), "{installed:?}");
        for file in ["one.txt", "two.txt"] {
            fs::write(presets.join("app").join(category).join(file), "v2").unwrap();
        }
    }
    let receipt = fs::read(state.join("app-manifest.toml")).unwrap();
    let rejected = run(&["upgrade", "app/demo/one.txt"]);
    assert!(!rejected.status.success(), "{rejected:?}");
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("approval requires"),
        "{rejected:?}"
    );
    assert_eq!(fs::read(state.join("app-manifest.toml")).unwrap(), receipt);
    assert_eq!(fs::read(destination.join("demo/one.txt")).unwrap(), b"v1");

    // A file suffix intentionally still selects the entire category, not one file.
    let upgraded = run(&["upgrade", "app/demo/one.txt", "--yes"]);
    assert!(upgraded.status.success(), "{upgraded:?}");
    assert!(
        String::from_utf8_lossy(&upgraded.stdout).contains("1 updated"),
        "{upgraded:?}"
    );
    for file in ["one.txt", "two.txt"] {
        assert_eq!(
            fs::read(destination.join("demo").join(file)).unwrap(),
            b"v2"
        );
        assert_eq!(
            fs::read(destination.join("other").join(file)).unwrap(),
            b"v1"
        );
    }
    let receipt = fs::read(state.join("app-manifest.toml")).unwrap();
    let noop = run(&["upgrade", "app/demo", "--yes"]);
    assert!(noop.status.success(), "{noop:?}");
    assert!(
        String::from_utf8_lossy(&noop.stdout).contains("Nothing to upgrade."),
        "{noop:?}"
    );
    assert_eq!(fs::read(state.join("app-manifest.toml")).unwrap(), receipt);

    fs::write(destination.join("demo/one.txt"), "user edit").unwrap();
    fs::write(presets.join("app/demo/one.txt"), "v3").unwrap();
    let preserved = run(&["upgrade", "app/demo", "--yes"]);
    assert!(preserved.status.success(), "{preserved:?}");
    assert!(
        String::from_utf8_lossy(&preserved.stdout).contains("1 user-modified (kept)"),
        "{preserved:?}"
    );
    assert_eq!(
        fs::read(destination.join("demo/one.txt")).unwrap(),
        b"user edit"
    );
    assert_eq!(fs::read(state.join("app-manifest.toml")).unwrap(), receipt);

    for (args, error) in [
        (vec!["upgrade", "sys/", "--yes"], "invalid system target"),
        (
            vec!["upgrade", "sys/demo/extra", "--yes"],
            "invalid system target",
        ),
        (
            vec!["upgrade", "sys/demo", "--prune-stale", "--yes"],
            "applies only to app targets",
        ),
    ] {
        let invalid = run(&args);
        assert!(!invalid.status.success(), "{invalid:?}");
        assert!(
            String::from_utf8_lossy(&invalid.stderr).contains(error),
            "{invalid:?}"
        );
        assert_eq!(fs::read(state.join("app-manifest.toml")).unwrap(), receipt);
    }
}

#[test]
#[cfg(unix)]
fn targeted_upgrade_distinguishes_nonfatal_hook_and_generator_failures() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let presets = state.join("presets");
    let preset = presets.join("app/demo");
    let destination = fixture.0.join("destination");
    fs::create_dir_all(&preset).unwrap();
    fs::write(
        state.join("config.toml"),
        "schema_version = 2\n[env]\nENABLE = '1'\n",
    )
    .unwrap();
    let dest = toml::Value::String(destination.to_string_lossy().into_owned()).to_string();
    let metadata = format!(
        "metadata_schema_version = 2\ndest = {dest}\npost_install = {{ command = '/bin/sh', args = ['-c', 'exit 6'] }}\npost_upgrade = {{ command = '/bin/sh', args = ['-c', 'exit 7'] }}\n[permissions]\nschema_version = 1\ncommands = ['/bin/sh']\nfilesystem = [{{ access = ['execute'], base = 'preset', path = 'generate.sh' }}]\nenvironment = [{{ name = 'ENABLE', sensitivity = 'plain' }}]\n[[files]]\nsource = 'config.txt'\nrestart_hint = 'Restart demo'\ngenerator = {{ script = 'generate.sh', env = [{{ source = 'ENABLE', sensitivity = 'plain' }}], when_env = 'ENABLE' }}\n"
    );
    fs::write(preset.join("shine.toml"), metadata).unwrap();
    fs::write(preset.join("config.txt"), "fallback").unwrap();
    let script = preset.join("generate.sh");
    let write_script = |body: &str| {
        use std::os::unix::fs::PermissionsExt;
        fs::write(&script, body).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    };
    let run = |args: &[&str]| {
        fixture
            .command()
            .env("SHINE_PRESETS", &presets)
            .args(args)
            .output()
            .unwrap()
    };
    write_script("#!/bin/sh\nprintf v1\n");
    let trusted = run(&["trust", "grant", "app/demo", "--yes"]);
    assert!(trusted.status.success(), "{trusted:?}");
    let installed = run(&["install", "app/demo", "--yes"]);
    assert!(installed.status.success(), "{installed:?}");
    assert!(
        String::from_utf8_lossy(&installed.stderr).contains("/bin/sh exited with 6"),
        "{installed:?}"
    );
    assert!(
        String::from_utf8_lossy(&installed.stdout).contains("1 installed"),
        "{installed:?}"
    );
    assert_eq!(fs::read(destination.join("config.txt")).unwrap(), b"v1");
    let initial_receipt = fs::read(state.join("app-manifest.toml")).unwrap();

    write_script("#!/bin/sh\nprintf v2\n");
    // Snapshot trust must be renewed for changed executable input.
    let untrusted = run(&["upgrade", "app/demo", "--yes"]);
    assert!(!untrusted.status.success(), "{untrusted:?}");
    assert!(
        String::from_utf8_lossy(&untrusted.stderr).contains("Plan is blocked"),
        "{untrusted:?}"
    );
    assert_eq!(fs::read(destination.join("config.txt")).unwrap(), b"v1");
    assert_eq!(
        fs::read(state.join("app-manifest.toml")).unwrap(),
        initial_receipt
    );
    let trusted = run(&["trust", "grant", "app/demo", "--yes"]);
    assert!(trusted.status.success(), "{trusted:?}");
    let upgraded = run(&["upgrade", "app/demo", "--yes"]);
    assert!(upgraded.status.success(), "{upgraded:?}");
    let stdout = String::from_utf8_lossy(&upgraded.stdout);
    assert!(stdout.contains("1 updated"), "{upgraded:?}");
    assert!(stdout.contains("Restart demo"), "{upgraded:?}");
    assert!(
        String::from_utf8_lossy(&upgraded.stderr).contains("/bin/sh exited with 7"),
        "{upgraded:?}"
    );
    assert_eq!(fs::read(destination.join("config.txt")).unwrap(), b"v2");
    let receipt = fs::read(state.join("app-manifest.toml")).unwrap();

    write_script("#!/bin/sh\nexit 9\n");
    let trusted = run(&["trust", "grant", "app/demo", "--yes"]);
    assert!(trusted.status.success(), "{trusted:?}");
    let failed = run(&["upgrade", "app/demo", "--yes"]);
    assert_eq!(failed.status.code(), Some(1), "{failed:?}");
    let text = String::from_utf8_lossy(&failed.stdout);
    assert!(
        text.contains("Upgrade incomplete") && text.contains("1 failed"),
        "{text}"
    );
    assert!(!text.contains("nothing changed"), "{text}");
    assert!(
        String::from_utf8_lossy(&failed.stderr).contains("1 managed configuration item(s) failed"),
        "{failed:?}"
    );
    assert_eq!(fs::read(destination.join("config.txt")).unwrap(), b"v2");
    assert_eq!(fs::read(state.join("app-manifest.toml")).unwrap(), receipt);
    // Install preserves the last good file on generator failure; unlike upgrade,
    // that existing policy is a warning, not a failed file result.
    let preserved = run(&["app", "install", "demo", "--yes"]);
    assert!(preserved.status.success(), "{preserved:?}");
    let stdout = String::from_utf8_lossy(&preserved.stdout);
    assert!(
        stdout.contains("1 skipped") && !stdout.contains("Install incomplete"),
        "{stdout}"
    );
    assert!(
        String::from_utf8_lossy(&preserved.stderr)
            .contains("generator unavailable; installed copy kept"),
        "{preserved:?}"
    );
    assert_eq!(fs::read(destination.join("config.txt")).unwrap(), b"v2");
    assert_eq!(fs::read(state.join("app-manifest.toml")).unwrap(), receipt);
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

#[cfg(unix)]
#[test]
fn app_install_reports_file_failures_without_hiding_partial_success() {
    use std::os::unix::fs::PermissionsExt;

    for partial in [false, true] {
        let fixture = Fixture::new();
        let state = fixture.0.join("state");
        let preset = state.join("presets/app/demo");
        let destination = fixture.0.join("destination");
        fs::create_dir_all(&preset).unwrap();
        fs::write(
            state.join("config.toml"),
            "schema_version = 2\n[env]\nENABLE = '1'\n",
        )
        .unwrap();
        let dest = toml::Value::String(destination.to_string_lossy().into_owned()).to_string();
        let extra = if partial {
            "[[files]]\nsource = 'good.txt'\n"
        } else {
            ""
        };
        fs::write(preset.join("shine.toml"), format!(
            "metadata_schema_version = 2\ndest = {dest}\n[permissions]\nschema_version = 1\nfilesystem = [{{ access = ['execute'], base = 'preset', path = 'generate.sh' }}]\nenvironment = [{{ name = 'ENABLE', sensitivity = 'plain' }}]\n[[files]]\nsource = 'config.json'\ninstall_mode = 'json-merge'\nmanaged_keys = ['demo']\ngenerator = {{ script = 'generate.sh', env = [{{ source = 'ENABLE', sensitivity = 'plain' }}], when_env = 'ENABLE' }}\n{extra}"
        )).unwrap();
        fs::write(preset.join("config.json"), r#"{"demo":1}"#).unwrap();
        fs::write(preset.join("good.txt"), "good").unwrap();
        let script = preset.join("generate.sh");
        fs::write(&script, "#!/bin/sh\nprintf invalid-json\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let trusted = fixture
            .command()
            .args(["trust", "grant", "app/demo", "--yes"])
            .output()
            .unwrap();
        assert!(trusted.status.success(), "{trusted:?}");
        // Exercise both public spellings through normal approval and execution.
        let args = if partial {
            vec!["install", "app/demo", "--yes"]
        } else {
            vec!["app", "install", "demo", "--yes"]
        };
        let output = fixture
            .command()
            .env("NO_COLOR", "1")
            .args(args)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(
            stdout.contains("Install incomplete") && stdout.contains("1 failed"),
            "{stdout}"
        );
        assert!(
            !stdout.contains("Done") && !stdout.contains("nothing changed"),
            "{stdout}"
        );
        assert!(stderr.contains("demo/config.json: json-merge"), "{stderr}");
        assert!(
            !stderr.contains("Error:") && !stderr.contains("command incomplete;"),
            "{stderr}"
        );
        assert!(!destination.join("config.json").exists());
        let receipt = fs::read_to_string(state.join("app-manifest.toml")).unwrap();
        assert!(!receipt.contains("config.json"), "{receipt}");
        if partial {
            assert!(stdout.contains("1 installed"), "{stdout}");
            assert_eq!(fs::read(destination.join("good.txt")).unwrap(), b"good");
            assert!(receipt.contains("good.txt"), "{receipt}");
        }
    }
}

#[test]
fn app_install_preview_and_unchanged_install_keep_distinct_counts() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let preset = state.join("presets/app/demo");
    let destination = fixture.0.join("destination");
    fs::create_dir_all(&preset).unwrap();
    fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
    let dest = toml::Value::String(destination.to_string_lossy().into_owned()).to_string();
    let metadata = format!(
        "metadata_schema_version = 2\ndest = {dest}\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'one.txt'\n"
    );
    fs::write(preset.join("shine.toml"), &metadata).unwrap();
    fs::write(preset.join("one.txt"), "one").unwrap();
    let preview = fixture
        .command()
        .args(["app", "install", "demo", "--dry-run"])
        .output()
        .unwrap();
    assert!(preview.status.success(), "{preview:?}");
    let stdout = String::from_utf8_lossy(&preview.stdout);
    assert!(
        stdout.contains("1 file available") && stdout.contains("1 would install"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("skipped") && !stdout.contains("Done"),
        "{stdout}"
    );
    assert!(!destination.exists());
    assert!(!state.join("app-manifest.toml").exists());
    let installed = fixture
        .command()
        .args(["app", "install", "demo", "--yes"])
        .output()
        .unwrap();
    assert!(installed.status.success(), "{installed:?}");
    let receipt = fs::read(state.join("app-manifest.toml")).unwrap();
    let unchanged = fixture
        .command()
        .args(["app", "install", "demo", "--yes"])
        .output()
        .unwrap();
    assert!(unchanged.status.success(), "{unchanged:?}");
    let unchanged_stdout = String::from_utf8_lossy(&unchanged.stdout);
    assert!(
        unchanged_stdout.contains("1 skipped") && !unchanged_stdout.contains("would install"),
        "{unchanged_stdout}"
    );
    fs::write(
        preset.join("shine.toml"),
        format!("{metadata}[[files]]\nsource = 'two.txt'\n"),
    )
    .unwrap();
    fs::write(preset.join("two.txt"), "two").unwrap();
    let preview = fixture
        .command()
        .args(["app", "install", "demo", "--dry-run"])
        .output()
        .unwrap();
    assert!(preview.status.success(), "{preview:?}");
    let stdout = String::from_utf8_lossy(&preview.stdout);
    assert!(
        stdout.contains("2 files available")
            && !stdout.contains("skipped")
            && stdout.contains("2 would install"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Preview") && !stdout.contains("Done"),
        "{stdout}"
    );
    assert_eq!(fs::read(state.join("app-manifest.toml")).unwrap(), receipt);
    assert_eq!(fs::read(destination.join("one.txt")).unwrap(), b"one");
    assert!(!destination.join("two.txt").exists());
}

#[cfg(unix)]
#[test]
fn stderr_warning_color_follows_its_own_terminal_and_no_color() {
    for stderr_terminal in [false, true] {
        for no_color in [false, true] {
            let fixture = Fixture::new();
            let mut command = fixture.command();
            command
                .args(["env", "get", "SHINE_OUTPUT_TEST_MISSING"])
                .env("TERM", "xterm-256color")
                .env_remove("NO_COLOR")
                .env_remove("FORCE_COLOR")
                .env_remove("CLICOLOR_FORCE");
            if no_color {
                command.env("NO_COLOR", "1");
            }
            let (output, terminal) = output_with_terminal(command, !stderr_terminal);
            assert_eq!(output.status.code(), Some(1), "{output:?}");
            let warning = if stderr_terminal {
                &terminal
            } else {
                &output.stderr
            };
            let warning = String::from_utf8_lossy(warning);
            assert!(
                warning.contains("SHINE_OUTPUT_TEST_MISSING is not set"),
                "{warning}"
            );
            assert_eq!(
                warning.contains("\x1b["),
                stderr_terminal && !no_color,
                "{warning:?}"
            );
        }
    }
}

#[test]
fn app_install_early_error_is_not_suppressed_as_already_reported() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let preset = state.join("presets/app/demo");
    fs::create_dir_all(&preset).unwrap();
    fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
    fs::write(preset.join("shine.toml"), "metadata_schema_version = 2\ndest = '~'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.txt'\n").unwrap();
    fs::write(preset.join("config.txt"), "managed").unwrap();
    let future_receipt = b"schema_version = 999\n";
    fs::write(state.join("app-manifest.toml"), future_receipt).unwrap();
    let output = fixture
        .command()
        .args(["app", "install", "demo", "--yes"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Error:") && stderr.contains("newer than this Shine supports"),
        "{stderr}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("Done") && !stdout.contains("Install incomplete"),
        "{stdout}"
    );
    assert_eq!(
        fs::read(state.join("app-manifest.toml")).unwrap(),
        future_receipt
    );
    assert!(!fixture.0.join("home/config.txt").exists());
}

#[test]
fn task_info_aligns_values_with_and_without_working_directory() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    fs::create_dir_all(&state).unwrap();
    fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
    let cwd = toml::Value::String(fixture.0.to_string_lossy().into_owned()).to_string();
    for fixed in [false, true] {
        let cwd_line = if fixed {
            format!("cwd = {cwd}\n")
        } else {
            String::new()
        };
        fs::write(
            state.join("tasks.toml"),
            format!("[tasks.build]\ncommand = ['cargo', 'build']\n{cwd_line}"),
        )
        .unwrap();
        let output = fixture
            .command()
            .env("NO_COLOR", "1")
            .args(["task", "info", "build"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let text = String::from_utf8(output.stdout).unwrap();
        let lines = text.lines().collect::<Vec<_>>();
        assert_eq!(lines[0], "Task        build");
        assert_eq!(
            lines[1],
            if cfg!(windows) {
                "Command     & 'cargo' build"
            } else {
                "Command     cargo build"
            }
        );
        assert_eq!(lines.len(), if fixed { 3 } else { 2 });
        if fixed {
            assert!(lines[2].starts_with("Working dir "), "{text}");
        }
    }
}

#[cfg(unix)]
#[test]
fn task_displays_execute_reserved_program_names_with_original_argv_and_exit_status() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let bin = fixture.0.join("bin");
    fs::create_dir_all(&state).unwrap();
    fs::create_dir_all(&bin).unwrap();
    fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
    // Save/list/info run version checks; keep this regression independent of GitHub releases.
    let update_cache = state.join("update-check.json");
    let cached_version = serde_json::to_vec(&serde_json::json!({
        "latest_version": env!("CARGO_PKG_VERSION"),
        "checked_at_unix_secs": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
    }))
    .unwrap();
    fs::write(&update_cache, &cached_version).unwrap();
    let mut paths = vec![bin.clone()];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let path = std::env::join_paths(paths).unwrap();
    let args = [
        "",
        "two words",
        "it's",
        "$HOME",
        "`literal`",
        "MODE=test",
        "if",
    ];
    let expected = format!("{}\0", args.join("\0")).into_bytes();
    for program in ["if", "then", "time", "MODE=test"] {
        let executable = bin.join(program);
        fs::write(&executable, "#!/bin/sh\nprintf '%s\\0' \"$@\"\nexit 7\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let saved = fixture
            .command()
            .env("NO_COLOR", "1")
            .args(["task", "save", "literal", "--force", "--", program])
            .args(args)
            .output()
            .unwrap();
        assert!(saved.status.success(), "{saved:?}");
        let saved_text = String::from_utf8(saved.stdout).unwrap();
        let display = saved_text.lines().nth(1).unwrap();
        let manifest = fs::read(state.join("tasks.toml")).unwrap();
        for (command, prefix) in [("info", "Command"), ("list", "literal")] {
            let mut query = fixture.command();
            query.env("NO_COLOR", "1").args(["task", command]);
            if command == "info" {
                query.arg("literal");
            }
            let output = query.output().unwrap();
            assert!(output.status.success(), "{output:?}");
            let text = String::from_utf8(output.stdout).unwrap();
            assert_eq!(
                text.lines()
                    .find_map(|line| line.strip_prefix(prefix))
                    .unwrap()
                    .trim(),
                display
            );
        }
        let direct = fixture
            .command()
            .env("NO_COLOR", "1")
            .env("PATH", &path)
            .args(["task", "run", "literal"])
            .output()
            .unwrap();
        assert_eq!(direct.status.code(), Some(7), "{direct:?}");
        assert_eq!(direct.stdout, expected);
        assert_eq!(
            String::from_utf8(direct.stderr).unwrap(),
            format!("Running literal: {display}\n")
        );
        let copied = Command::new("/bin/sh")
            .env("PATH", &path)
            .args(["-c", display])
            .output()
            .unwrap();
        assert_eq!(copied.status.code(), Some(7), "{copied:?}");
        assert_eq!(copied.stdout, expected);
        assert!(copied.stderr.is_empty(), "{copied:?}");
        assert_eq!(fs::read(state.join("tasks.toml")).unwrap(), manifest);
        assert_eq!(fs::read(&update_cache).unwrap(), cached_version);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn sys_profile_reports_attention_and_noop_without_changing_exit_policy() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let preset = state.join("presets/sys/macos");
    fs::create_dir_all(&preset).unwrap();
    fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
    fs::write(fixture.0.join("home/.tool-present"), "present").unwrap();
    fs::write(preset.join("shine.toml"), "version = 2\n[[items]]\nid = 'tool'\nlabel = 'Tool'\npermissions = { schema_version = 1 }\ndetect = { kind = 'path', path = '$HOME/.tool-present' }\ninstall = { kind = 'package', provider = 'homebrew', package = 'tool' }\n[[items.shell]]\nshells = ['zsh']\nphase = 'post'\npath = '$HOME/.tool/bin'\n").unwrap();
    let run = |args: &[&str]| {
        fixture
            .command()
            .env("SHELL", "/bin/zsh")
            .env("NO_COLOR", "1")
            .args(args)
            .output()
            .unwrap()
    };
    let denied = run(&["sys", "profile", "enable", "tool"]);
    assert!(!denied.status.success(), "{denied:?}");
    assert!(
        String::from_utf8_lossy(&denied.stderr).contains("approval requires"),
        "{denied:?}"
    );
    let profile = fixture.0.join("home/.shine/profile");
    assert!(!profile.exists());
    assert!(!state.join("sys-manifest.toml").exists());
    assert!(!fixture.0.join("home/.zshrc").exists());
    let preview = run(&["sys", "profile", "enable", "tool", "--dry-run"]);
    assert!(preview.status.success(), "{preview:?}");
    assert!(
        String::from_utf8_lossy(&preview.stdout).contains("[dry-run] sys/tool: would enable"),
        "{preview:?}"
    );
    assert!(!profile.exists());
    assert!(!state.join("sys-manifest.toml").exists());
    assert!(!fixture.0.join("home/.zshrc").exists());
    let enabled = run(&["sys", "profile", "enable", "tool", "--yes"]);
    assert!(enabled.status.success(), "{enabled:?}");
    assert!(
        String::from_utf8_lossy(&enabled.stdout).contains("shell integration enabled"),
        "{enabled:?}"
    );
    let again = run(&["sys", "profile", "enable", "tool", "--yes"]);
    assert!(again.status.success(), "{again:?}");
    assert!(
        String::from_utf8_lossy(&again.stdout).contains("shell integration already enabled"),
        "{again:?}"
    );
    // Make the active profile diverge from its merge base at the integration itself.
    let active = profile.join("macos-sys.post.sh");
    let original = fs::read_to_string(&active).unwrap();
    assert!(original.contains(".tool/bin"), "{original}");
    fs::write(&active, original.replace(".tool/bin", ".custom/bin")).unwrap();
    let user_edit = fs::read(&active).unwrap();
    let conflict = run(&["sys", "profile", "disable", "tool", "--yes"]);
    assert!(conflict.status.success(), "{conflict:?}");
    let text = String::from_utf8_lossy(&conflict.stdout);
    assert!(
        text.lines()
            .last()
            .unwrap()
            .starts_with("~ sys/tool: shell integration needs action")
            && !text.contains("shell integration disabled"),
        "{text}"
    );
    assert_eq!(fs::read(&active).unwrap(), user_edit);
    assert!(profile.join("macos-sys.post.new.sh").exists());
    // Missing merge ancestry also requires attention and must not replace user content.
    fs::remove_file(profile.join("macos-sys.post.base.sh")).unwrap();
    let missing_base = run(&["sys", "profile", "enable", "tool", "--yes"]);
    assert!(missing_base.status.success(), "{missing_base:?}");
    let text = String::from_utf8_lossy(&missing_base.stdout);
    assert!(
        text.lines()
            .last()
            .unwrap()
            .starts_with("~ sys/tool: shell integration needs action")
            && text.contains("no merge base"),
        "{text}"
    );
    assert_eq!(fs::read(&active).unwrap(), user_edit);
}

#[test]
fn theme_quiet_preserves_config_errors_and_keeps_stdout_clean() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    fs::create_dir_all(&state).unwrap();
    let malformed = "TOKEN = \"PRIVATE_THEME_CONFIG_CANARY_UNCLOSED\n";
    fs::write(state.join("config.toml"), malformed).unwrap();
    let output = fixture
        .command()
        .args(["theme", "sync", "--quiet"])
        .env("SHINE_TERMINAL_THEME", "dark")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("TOML parse error"),
        "{output:?}"
    );
    assert!(!String::from_utf8_lossy(&output.stderr).contains("PRIVATE_THEME_CONFIG_CANARY"));
    assert_eq!(
        fs::read_to_string(state.join("config.toml")).unwrap(),
        malformed
    );
    assert_eq!(fs::read_dir(&state).unwrap().count(), 1);
}

#[test]
fn theme_quiet_exports_to_stdout_without_initializing_state() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .args(["theme", "sync", "--quiet"])
        .env("SHINE_TERMINAL_THEME", "dark")
        .env("BAT_THEME", "user-choice")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("SHINE_TERMINAL_THEME"), "{output:?}");
    assert!(stdout.contains("dark"), "{output:?}");
    assert!(!stdout.contains("BAT_THEME"), "{output:?}");
    assert!(!fixture.0.join("state").exists());
}

#[test]
fn self_upgrade_preserves_error_chain_without_printing_credentials() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.0.join("state")).unwrap();
    fs::write(fixture.0.join("state/config.toml"), "schema_version = 2\n").unwrap();
    // Invalid header bytes fail before any HTTP request or binary installation.
    let output = fixture
        .command()
        .args(["self", "upgrade"])
        .env("GITHUB_TOKEN", "fixture-secret\ninvalid")
        .env_remove("GH_TOKEN")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    for message in [
        "Upgrade failed",
        "invalid GitHub token header",
        "failed to parse header value",
    ] {
        assert!(stderr.contains(message), "{output:?}");
    }
    assert!(!stderr.contains("fixture-secret"), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Checking for stable upgrades"),
        "{output:?}"
    );
    assert!(!stdout.contains("Upgraded shine"), "{output:?}");
    assert!(!stdout.contains("fixture-secret"), "{output:?}");
}

#[test]
fn preset_link_warning_matches_project_and_environment_source_selection() {
    for (project, env_presets, warns, selected) in [
        (false, None, true, "state-source"),
        (true, None, false, "linked-source"),
        (true, Some(""), false, "linked-source"),
        (true, Some("env"), true, "env-source"),
    ] {
        let fixture = Fixture::new();
        let state = fixture.0.join("state");
        let project_dir = fixture.0.join("project");
        let linked = fixture.0.join("linked");
        let env_source = fixture.0.join("env-source");
        fs::create_dir_all(&project_dir).unwrap();
        let script = if cfg!(windows) { "run.ps1" } else { "run.sh" };
        for (source, category) in [
            (state.join("presets"), "state-source"),
            (linked.clone(), "linked-source"),
            (env_source.clone(), "env-source"),
        ] {
            let category = source.join("shell").join(category);
            fs::create_dir_all(&category).unwrap();
            fs::write(
                category.join("shine.toml"),
                format!("[[files]]\nsource = '{script}'\n"),
            )
            .unwrap();
            fs::write(category.join(script), "# inspection only\n").unwrap();
        }
        let global_config = "schema_version = 2\n# retain global settings\ncustom = 'keep'\n";
        fs::write(state.join("config.toml"), global_config).unwrap();
        if project {
            fs::write(
                project_dir.join("shine.config.toml"),
                "# retain project settings\ncustom = 'keep'\npresets_dir = 'old-source'\n",
            )
            .unwrap();
        }
        let command = || {
            let mut command = fixture.command();
            if project {
                command.current_dir(&project_dir);
            }
            if let Some(value) = env_presets {
                command.env(
                    "SHINE_PRESETS",
                    if value.is_empty() {
                        PathBuf::new()
                    } else {
                        env_source.clone()
                    },
                );
            }
            command
        };
        let output = command()
            .args(["preset", "link", "--live"])
            .arg(&linked)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(stdout.contains("Warning:"), warns, "{output:?}");
        assert!(stdout.contains("Shell mode: live"), "{output:?}");
        let config_path = if project {
            project_dir.join("shine.config.toml")
        } else {
            state.join("config.toml")
        };
        let saved = fs::read(&config_path).unwrap();
        let config: toml::Table = toml::from_str(std::str::from_utf8(&saved).unwrap()).unwrap();
        assert_eq!(
            PathBuf::from(config["presets_dir"].as_str().unwrap()),
            fs::canonicalize(&linked).unwrap()
        );
        assert_eq!(config["external_shell_mode"].as_str(), Some("live"));
        assert_eq!(config["custom"].as_str(), Some("keep"));
        let list = command().args(["shell", "list"]).output().unwrap();
        assert_eq!(list.status.code(), Some(0), "{list:?}");
        assert!(list.stderr.is_empty(), "{list:?}");
        assert!(
            String::from_utf8_lossy(&list.stdout).contains(selected),
            "{list:?}"
        );
        assert_eq!(fs::read(config_path).unwrap(), saved);
        if project {
            assert_eq!(
                fs::read(state.join("config.toml")).unwrap(),
                global_config.as_bytes()
            );
        }
        for (source, category) in [
            (state.join("presets"), "state-source"),
            (linked, "linked-source"),
            (env_source, "env-source"),
        ] {
            assert_eq!(
                fs::read(source.join("shell").join(category).join(script)).unwrap(),
                b"# inspection only\n"
            );
        }
        assert!(!state.join("shell-manifest.toml").exists());
        assert!(!state.join("installed").exists());
        assert!(!state.join("trust.toml").exists());
    }
}

#[test]
fn preset_unlink_reports_saved_scope_without_promising_the_next_source() {
    for (project, config_env, presets_env, selected) in [
        (false, true, false, Some("state-source")),
        (true, true, true, Some("env-source")),
        (true, false, false, Some("global-source")),
        (false, false, false, None),
    ] {
        let fixture = Fixture::new();
        let state = fixture
            .0
            .join(if config_env { "state" } else { "home/.shine" });
        let project_dir = fixture.0.join("project");
        let global_source = fixture.0.join("global-source");
        let env_source = fixture.0.join("env-source");
        fs::create_dir_all(&project_dir).unwrap();
        let script = if cfg!(windows) { "run.ps1" } else { "run.sh" };
        for (source, category) in [
            (state.join("presets"), "state-source"),
            (global_source.clone(), "global-source"),
            (env_source.clone(), "env-source"),
        ] {
            let category = source.join("shell").join(category);
            fs::create_dir_all(&category).unwrap();
            fs::write(
                category.join("shine.toml"),
                format!("[[files]]\nsource = '{script}'\n"),
            )
            .unwrap();
            fs::write(category.join(script), "# inspection only\n").unwrap();
        }
        let global_config = format!(
            "schema_version = 2\n# retained\ncustom = 'keep'\nexternal_shell_mode = 'live'\npresets_dir = {}\n",
            toml::Value::String(global_source.to_string_lossy().into_owned())
        );
        fs::write(state.join("config.toml"), &global_config).unwrap();
        if project {
            fs::write(project_dir.join("shine.config.toml"), "# retained\ncustom = 'keep'\npresets_dir = 'local-source'\nexternal_shell_mode = 'live'\n").unwrap();
        }
        let command = || {
            let mut command = fixture.command();
            if project {
                command.current_dir(&project_dir);
            }
            if !config_env {
                command.env_remove("SHINE_CONFIG_DIR");
            }
            if presets_env {
                command.env("SHINE_PRESETS", &env_source);
            }
            command
        };
        let output = command().args(["preset", "unlink"]).output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("removed from the active config"),
            "{output:?}"
        );
        assert!(
            stdout.contains(
                "Environment overrides and inherited settings still apply on the next run."
            ),
            "{output:?}"
        );
        assert!(
            !stdout.contains("Built-in embedded presets will be used"),
            "{output:?}"
        );
        let config_path = if project {
            project_dir.join("shine.config.toml")
        } else {
            state.join("config.toml")
        };
        let saved = fs::read_to_string(&config_path).unwrap();
        let config: toml::Table = toml::from_str(&saved).unwrap();
        assert!(!config.contains_key("presets_dir"));
        assert_eq!(config["custom"].as_str(), Some("keep"));
        assert!(saved.contains("# retained"));
        assert_ne!(
            config
                .get("external_shell_mode")
                .and_then(toml::Value::as_str),
            Some("live")
        );
        let list = command().args(["shell", "list"]).output().unwrap();
        assert_eq!(list.status.code(), Some(0), "{list:?}");
        assert!(list.stderr.is_empty(), "{list:?}");
        let stdout = String::from_utf8_lossy(&list.stdout);
        if let Some(category) = selected {
            assert!(stdout.contains(category), "{list:?}");
        } else {
            assert!(stdout.contains("built-in"), "{list:?}");
            assert!(!stdout.contains("global-source"), "{list:?}");
        }
        assert_eq!(fs::read_to_string(config_path).unwrap(), saved);
        if project {
            assert_eq!(
                fs::read_to_string(state.join("config.toml")).unwrap(),
                global_config
            );
        }
        for (source, category) in [
            (state.join("presets"), "state-source"),
            (global_source, "global-source"),
            (env_source, "env-source"),
        ] {
            assert_eq!(
                fs::read(source.join("shell").join(category).join(script)).unwrap(),
                b"# inspection only\n"
            );
        }
        assert!(!state.join("shell-manifest.toml").exists());
        assert!(!state.join("installed").exists());
        assert!(!state.join("trust.toml").exists());
    }
}

#[test]
fn overlay_unlink_reports_inheritance_without_removing_global_or_source_files() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let global = fixture.0.join("global-overlay");
    let project = fixture.0.join("project");
    let local = project.join("local-overlay");
    for dir in [&state, &global, &local] {
        fs::create_dir_all(dir).unwrap();
    }
    fs::write(global.join("marker"), "global").unwrap();
    fs::write(local.join("marker"), "project").unwrap();
    let global_config = format!(
        "schema_version = 2\npresets_overlay_dir = {}\n",
        toml::Value::String(global.to_string_lossy().into_owned())
    );
    fs::write(state.join("config.toml"), &global_config).unwrap();
    fs::write(
        project.join("shine.config.toml"),
        "# retained\ncustom = 'keep'\npresets_overlay_dir = 'local-overlay'\n",
    )
    .unwrap();
    let output = fixture
        .command()
        .current_dir(&project)
        .args(["preset", "overlay", "unlink"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("removed from the active config"),
        "{output:?}"
    );
    assert!(
        stdout.contains("Global overlay settings may still be inherited"),
        "{output:?}"
    );
    assert!(
        !stdout.contains("without overlay on the next run"),
        "{output:?}"
    );
    assert_eq!(
        fs::read_to_string(state.join("config.toml")).unwrap(),
        global_config
    );
    let project_config = fs::read_to_string(project.join("shine.config.toml")).unwrap();
    assert!(project_config.contains("# retained"));
    assert!(project_config.contains("custom = 'keep'"));
    assert!(!project_config.contains("presets_overlay_dir"));
    let info = fixture
        .command()
        .current_dir(&project)
        .args(["preset", "overlay", "info"])
        .output()
        .unwrap();
    assert!(info.status.success(), "{info:?}");
    assert!(
        String::from_utf8_lossy(&info.stdout).contains("global-overlay"),
        "{info:?}"
    );
    assert_eq!(fs::read(global.join("marker")).unwrap(), b"global");
    assert_eq!(fs::read(local.join("marker")).unwrap(), b"project");
    let output = fixture
        .command()
        .args(["preset", "overlay", "unlink"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("base presets source"),
        "{output:?}"
    );
    assert_eq!(fs::read(global.join("marker")).unwrap(), b"global");
}

#[test]
fn preset_export_reports_overlay_sources_and_preserves_export_semantics() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let overlay = fixture.0.join("overlay");
    fs::create_dir_all(state.join("presets")).unwrap();
    fs::create_dir_all(&overlay).unwrap();
    fs::write(overlay.join("overlay-only.txt"), "overlay").unwrap();
    fs::write(state.join("presets/external-only.txt"), "external").unwrap();
    for with_overlay in [false, true] {
        let mut config = "schema_version = 2\n".to_string();
        if with_overlay {
            config.push_str(&format!(
                "presets_overlay_dir = {}\n",
                toml::Value::String(overlay.to_string_lossy().into_owned())
            ));
        }
        fs::write(state.join("config.toml"), config).unwrap();
        let out = fixture
            .0
            .join(if cfg!(unix) {
                "export\nFORGED=export\r\t\u{1b}"
            } else {
                "export"
            })
            .join(if with_overlay { "merged" } else { "builtin" });
        let output = fixture
            .command()
            .args(["preset", "export"])
            .arg(&out)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("Exporting built-in presets"), "{output:?}");
        assert!(!stdout.contains(['\r', '\t', '\u{1b}']), "{output:?}");
        if cfg!(unix) {
            assert!(
                stdout
                    .lines()
                    .next()
                    .unwrap()
                    .contains("export\\nFORGED=export\\r\\t\\u{1b}"),
                "{output:?}"
            );
        }
        assert_eq!(
            stdout.contains("with overlay overrides and additions"),
            with_overlay,
            "{output:?}"
        );
        assert_eq!(out.join("overlay-only.txt").exists(), with_overlay);
        assert!(out.join("shell/proxy/shine.toml").is_file());
        assert!(!out.join("external-only.txt").exists());
    }
}

#[test]
fn age_preflight_diagnostics_encode_fields_without_running_tools_or_exposing_plaintext() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    fs::create_dir(&state).unwrap();
    let missing = fixture.0.join("missing\nFORGED\r\t\u{1b}");
    let config = format!(
        "schema_version=2\nage_identity={}\n[env]\nINPUT='PRIVATE_INPUT'\nCIPHER='age:Zm9v'\n",
        toml::Value::String(missing.to_str().unwrap().into())
    );
    fs::write(state.join("config.toml"), &config).unwrap();
    for (args, reason) in [
        (
            vec![
                "env",
                "secret",
                "encrypt",
                "--backend",
                "age",
                "--recipient",
                "age1bad\nFORGED\r\t\u{1b}X",
                "--from",
                "INPUT",
            ],
            "recipient must not contain whitespace",
        ),
        (
            vec!["env", "secret", "decrypt", "CIPHER"],
            "age identity file not found",
        ),
    ] {
        let output = fixture
            .command()
            .env("PATH", fixture.0.join("no-external-tools"))
            .env("NO_COLOR", "1")
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains(reason), "{stderr}");
        assert!(!stderr.contains(['\r', '\t', '\u{1b}']), "{stderr:?}");
        assert!(stderr.contains(r"\nFORGED\r\t\u{1b}"), "{stderr}");
        assert!(!stderr.contains("PRIVATE_INPUT"));
        assert_eq!(
            fs::read_to_string(state.join("config.toml")).unwrap(),
            config
        );
        assert_eq!(fs::read_dir(&state).unwrap().count(), 1);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn phone_setup_notices_encode_paths_and_keep_manual_toml_and_pairing_state() {
    use std::os::unix::fs::PermissionsExt;
    let recipient = "age1phone1qypkk9737tsjcsj8lz7wdetr53q0yacr0kqjm6en5r62zw29mzvv99sa27n9c";
    for save_fails in [false, true] {
        let fixture = display_path_fixture();
        let state = fixture.0.join("state");
        fs::create_dir(&state).unwrap();
        let config =
            "schema_version=2\nsecret_backend='gpg'\nage_recipients=['recovery-placeholder']\n";
        fs::write(state.join("config.toml"), config).unwrap();
        let stub = fixture.0.join("phone\nSTUB\r\t\u{1b}\\literal.txt");
        let contents = format!("# recipient: {recipient}\nAGE-PLUGIN-PHONE-1EXAMPLE\n");
        fs::write(&stub, &contents).unwrap();
        let bin = fixture.0.join("test-bin");
        fs::create_dir(&bin).unwrap();
        let payload = serde_json::json!({
            "schema_version": 1,
            "identity_path": stub,
            "recipient": recipient,
        });
        let program = bin.join("age-plugin-phone");
        fs::write(
            &program,
            format!(
                "#!/bin/sh\nprintf 'called\\n' >> \"$SHINE_TEST_CALLS\"\nprintf '%s\\n' {}\n",
                shine_core::shell_quote::posix_single_quote(&payload.to_string())
            ),
        )
        .unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        if save_fails {
            // Fail the config lock without touching plugin-owned state or depending on UID.
            fs::create_dir(state.join(".config.toml.shine.lock")).unwrap();
        }
        let calls = fixture.0.join("calls");
        let output = fixture
            .command()
            .env("PATH", &bin)
            .env("NO_COLOR", "1")
            .env("SHINE_TEST_CALLS", &calls)
            .args([
                "env",
                "secret",
                "identity",
                "init",
                "--phone",
                "--recipient-type",
                "phone",
                "--label",
                "Review fixture",
            ])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(if save_fails { 1 } else { 0 }),
            "{output:?}"
        );
        assert_eq!(fs::read_to_string(&calls).unwrap(), "called\n");
        assert_eq!(fs::read_to_string(&stub).unwrap(), contents);
        let saved = fs::read_to_string(state.join("config.toml")).unwrap();
        let identities = if save_fails {
            assert!(output.stdout.is_empty());
            assert_eq!(saved, config);
            let stderr = String::from_utf8(output.stderr).unwrap();
            let (notice, remaining) = stderr.split_once("\n\n").unwrap();
            assert!(notice.contains("pairing remains active; do not start another setup"));
            assert!(
                !notice.contains(['\r', '\t', '\u{1b}']) && !notice.contains("\nFORGED"),
                "{notice:?}"
            );
            let (manual, error) = remaining.split_once("\nError:").unwrap();
            assert!(error.contains("saving the phone identity in global Shine config"));
            manual.parse::<toml::Table>().unwrap()
        } else {
            assert!(output.stderr.is_empty());
            let stdout = String::from_utf8(output.stdout).unwrap();
            assert!(!stdout.contains(['\r', '\t', '\u{1b}']), "{stdout:?}");
            assert!(!stdout.contains("\nFORGED") && !stdout.contains("\nSTUB"));
            assert!(
                stdout.contains(r"phone\nSTUB\r\t\u{1b}\literal.txt"),
                "{stdout}"
            );
            assert!(stdout.contains(&format!("  recipient: {recipient}\n")));
            let saved = saved.parse::<toml::Table>().unwrap();
            assert_eq!(saved["secret_backend"].as_str(), Some("gpg"));
            assert_eq!(
                saved["age_recipients"][0].as_str(),
                Some("recovery-placeholder")
            );
            saved
        };
        assert_eq!(identities["age_identities"][0].as_str(), stub.to_str());
    }
}

#[test]
fn creation_preflight_errors_encode_paths_without_replacing_files_or_configuration() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    fs::create_dir(&state).unwrap();
    let config = "schema_version=2\n";
    fs::write(state.join("config.toml"), config).unwrap();
    let blocked = fixture.0.join(if cfg!(unix) {
        "blocked\nFORGED\r\t\u{1b}"
    } else {
        "blocked中文"
    });
    fs::write(&blocked, "PRIVATE_CONTENT_MUST_NOT_BE_PRINTED").unwrap();
    for (args, path, reason) in [
        (
            vec!["env", "secret", "identity", "init", "--output"],
            blocked.clone(),
            "already exists; pass --force",
        ),
        (
            vec!["env", "secret", "identity", "init", "--output"],
            blocked.join("identity.txt"),
            "creating",
        ),
        (
            vec!["preset", "export"],
            blocked.clone(),
            "creating export directory",
        ),
    ] {
        let output = fixture
            .command()
            .env("NO_COLOR", "1")
            .args(&args)
            .arg(&path)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        let text = String::from_utf8(output.stderr).unwrap();
        assert!(text.contains(reason), "{text}");
        assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
        assert!(
            !text.contains("\nFORGED") && !text.contains("PRIVATE_CONTENT_MUST_NOT_BE_PRINTED")
        );
        assert_eq!(
            fs::read_to_string(&blocked).unwrap(),
            "PRIVATE_CONTENT_MUST_NOT_BE_PRINTED"
        );
        assert_eq!(
            fs::read_to_string(state.join("config.toml")).unwrap(),
            config
        );
        let initializes_state = args[0] == "preset";
        assert_eq!(
            fs::read_dir(&state).unwrap().count(),
            if initializes_state { 3 } else { 1 }
        );
        if initializes_state {
            for initialized in ["bin", "presets"] {
                assert_eq!(fs::read_dir(state.join(initialized)).unwrap().count(), 0);
            }
        }
    }
    let output = fixture
        .command()
        .env("NO_COLOR", "1")
        .args([
            "env",
            "secret",
            "identity",
            "init",
            "--touch-id",
            "--access-control",
            "bad\nFORGED\r\t\u{1b}",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty());
    let text = String::from_utf8(output.stderr).unwrap();
    assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
    if cfg!(target_os = "macos") {
        assert!(
            text.contains(r#"unknown --access-control "bad\nFORGED\r\t\u{1b}""#),
            "{text}"
        );
    } else {
        assert!(
            text.contains("Secure Enclave identities require macOS"),
            "{text}"
        );
    }
    assert_eq!(
        fs::read_to_string(state.join("config.toml")).unwrap(),
        config
    );
    assert!(!state.join("age").exists());
}

#[test]
fn self_install_failure_notices_encode_paths_and_preserve_refusal_and_cleanup() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    fs::create_dir(&state).unwrap();
    let config = "schema_version=2\n";
    fs::write(state.join("config.toml"), config).unwrap();
    let root = fixture.0.join(if cfg!(unix) {
        "install\nFORGED\r\t\u{1b}"
    } else {
        "install中文"
    });
    fs::create_dir(&root).unwrap();
    let blocked = root.join("parent-file");
    fs::write(&blocked, "retain").unwrap();
    let directory = root.join("occupied-directory");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("keep"), "retain").unwrap();
    for dest in [blocked.join("shine"), directory.clone()] {
        let output = fixture
            .command()
            .env("NO_COLOR", "1")
            .args(["self", "install", "--dest"])
            .arg(&dest)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        let text = String::from_utf8(output.stderr).unwrap();
        assert!(
            text.contains("failed to copy to") && text.contains("Caused by:"),
            "{text}"
        );
        assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
        assert!(!text.contains("\nFORGED"));
        assert_eq!(fs::read_to_string(&blocked).unwrap(), "retain");
        assert_eq!(
            fs::read_to_string(directory.join("keep")).unwrap(),
            "retain"
        );
        assert_eq!(
            fs::read_dir(&root).unwrap().count(),
            2,
            "staged binary leaked"
        );
        assert_eq!(
            fs::read_to_string(state.join("config.toml")).unwrap(),
            config
        );
    }
    let binary = root.join(if cfg!(windows) { "shine.exe" } else { "shine" });
    fs::copy(env!("CARGO_BIN_EXE_shine"), &binary).unwrap();
    let before = shine_core::digest::sha256_hex(&fs::read(&binary).unwrap());
    let output = fixture
        .configure(Command::new(&binary))
        .env("NO_COLOR", "1")
        .args(["self", "install", "--dest"])
        .arg(&binary)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty());
    let text = String::from_utf8(output.stderr).unwrap();
    assert!(text.contains("source and destination are the same binary"));
    assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
    assert!(!text.contains("\nFORGED"));
    assert_eq!(
        shine_core::digest::sha256_hex(&fs::read(&binary).unwrap()),
        before
    );
    assert_eq!(
        fs::read_to_string(state.join("config.toml")).unwrap(),
        config
    );
}

#[test]
fn preset_failures_escape_operands_and_preserve_source_configuration() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    fs::create_dir(&state).unwrap();
    let config = "schema_version=2\n";
    fs::write(state.join("config.toml"), config).unwrap();
    let missing = fixture.0.join(if cfg!(unix) {
        "missing\nFORGED\r\t\u{1b}"
    } else {
        "missing中文"
    });
    let file = fixture.0.join(if cfg!(unix) {
        "file\nFORGED\r\t\u{1b}"
    } else {
        "file中文"
    });
    fs::write(&file, "retain").unwrap();
    for prefix in [vec!["preset", "link"], vec!["preset", "overlay", "link"]] {
        for (path, create) in [(&missing, false), (&file, false), (&file, true)] {
            let mut command = fixture.command();
            command.env("NO_COLOR", "1").args(&prefix).arg(path);
            if create {
                command.arg("--create");
            }
            let output = command.output().unwrap();
            assert_eq!(output.status.code(), Some(1), "{output:?}");
            assert!(output.stdout.is_empty(), "{output:?}");
            let text = String::from_utf8(output.stderr).unwrap();
            assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
            assert!(!text.contains("\nFORGED"), "{text:?}");
            assert_eq!(
                fs::read_to_string(state.join("config.toml")).unwrap(),
                config
            );
            assert_eq!(fs::read_to_string(&file).unwrap(), "retain");
            assert!(!missing.exists());
        }
    }
    let output = fixture
        .command()
        .env("NO_COLOR", "1")
        .args(["preset", "copy", "app/missing\nFORGED\r\t\u{1b}"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "Error: built-in preset not found: app/missing\\nFORGED\\r\\t\\u{1b}\n"
    );
    assert!(!fixture.0.join("app").exists());
    assert_eq!(
        fs::read_to_string(state.join("config.toml")).unwrap(),
        config
    );
}

#[test]
fn preset_copy_encodes_destination_preserving_existing_files_and_config() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    fs::create_dir(&state).unwrap();
    fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
    let work = fixture.0.join(if cfg!(unix) {
        "copy\nFORGED=copy\r\t\u{1b}\\literal中文"
    } else {
        "copy中文"
    });
    fs::create_dir(&work).unwrap();
    for existing in [false, true] {
        let output = fixture
            .command()
            .current_dir(&work)
            .env("NO_COLOR", "1")
            .args(["preset", "copy", "shell/proxy"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(!text.contains("file(s)"), "{text}");
        assert!(
            text.contains(if existing {
                "files skipped"
            } else {
                "files created"
            }),
            "{text}"
        );
        assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
        let header = text.lines().next().unwrap();
        assert!(
            header.starts_with("Copying built-in preset shell/proxy to "),
            "{text:?}"
        );
        assert!(header.ends_with(" ..."), "{text:?}");
        if cfg!(unix) {
            assert!(
                header.contains("copy\\nFORGED=copy\\r\\t\\u{1b}\\literal中文"),
                "{text:?}"
            );
        }
        let path = work.join("shell/proxy/shine.toml");
        if existing {
            assert_eq!(fs::read(path).unwrap(), b"unrelated user content\n");
            assert!(
                text.contains("skipped (already exist; use --force to overwrite)"),
                "{text:?}"
            );
        } else {
            assert!(path.is_file());
            fs::write(path, b"unrelated user content\n").unwrap();
        }
    }
    assert_eq!(
        fs::read(state.join("config.toml")).unwrap(),
        b"schema_version = 2\n"
    );
}

#[test]
fn preset_activation_hints_use_templates_and_separate_escaped_paths() {
    let fixture = Fixture::new();
    // Apostrophes and shell metacharacters remain legal on Windows too.
    let destination = fixture.0.join("custom presets 'quoted' $NAME");
    fs::create_dir_all(&destination).unwrap();
    let output = fixture
        .command()
        .current_dir(&destination)
        .args(["preset", "copy", "shell/proxy"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("`shine preset overlay link <PATH>`"),
        "{output:?}"
    );
    let shown = stdout
        .lines()
        .find_map(|line| line.strip_prefix("  PATH (display-escaped): "))
        .unwrap();
    let shown: String = serde_json::from_str(shown).unwrap();
    assert_eq!(
        fs::canonicalize(shown).unwrap(),
        fs::canonicalize(&destination).unwrap()
    );
    assert!(!stdout.contains("Tip: run `"), "{output:?}");
    // Without the explicit config-dir override, export emits the activation hint.
    let output = fixture
        .command()
        .env_remove("SHINE_CONFIG_DIR")
        .args(["preset", "export"])
        .arg(&destination)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("`shine preset link <PATH>`"), "{output:?}");
    assert!(
        stdout.contains(&format!(
            "PATH (display-escaped): {:?}",
            destination.to_string_lossy()
        )),
        "{output:?}"
    );
    assert!(!stdout.contains("Tip: run `"), "{output:?}");
}

#[test]
fn task_run_help_describes_saved_and_caller_working_directories() {
    let fixture = Fixture::new();
    for args in [vec!["task", "--help"], vec!["task", "run", "--help"]] {
        let output = fixture.command().args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{args:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{args:?}: {output:?}");
        let help = String::from_utf8(output.stdout).unwrap();
        let help = help.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(help.contains("Run a saved task in its saved working directory, or the caller's directory if none was saved"), "{args:?}: {help}");
        assert!(
            !help.contains("Run a saved task in the current directory"),
            "{args:?}: {help}"
        );
    }
    assert!(!fixture.0.join("state").exists());
}

#[tokio::test]
async fn sys_status_reports_per_item_records_without_mutating_them() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    fs::create_dir_all(&state).unwrap();
    let os = cli::sys::detect_os_id().await.unwrap();
    let mut manifest = shine_core::runtime::SysRunManifest::default();
    for (index, (label, managed, status)) in [
        (
            "ASCII",
            false,
            shine_core::runtime::SysItemStatus::Installed,
        ),
        (
            "中文标签中文标签",
            true,
            shine_core::runtime::SysItemStatus::Updated,
        ),
        (
            "e\u{301}",
            false,
            shine_core::runtime::SysItemStatus::Failed,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        manifest.entries.push(shine_core::runtime::SysRunEntry {
            os_id: os.clone(),
            item_id: format!("item-{index}"),
            label: label.into(),
            status,
            detail: "recorded detail".into(),
            updated_at: index.to_string(),
            managed,
            profile_enabled: false,
            receipt: None,
        });
    }
    let mut foreign = manifest.entries[0].clone();
    foreign.os_id = "foreign-os".into();
    foreign.label = "Foreign item".into();
    manifest.entries.push(foreign.clone());
    // Foreign-only and genuinely empty records share the current-OS empty state.
    for entries in [manifest.entries.clone(), vec![foreign], vec![]] {
        manifest.entries = entries;
        let bytes = toml::to_string(&manifest).unwrap();
        let path = state.join("sys-manifest.toml");
        fs::write(&path, &bytes).unwrap();
        let output = fixture
            .command()
            .env("NO_COLOR", "1")
            .args(["sys", "status"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(!stdout.contains("Foreign item"), "{stdout}");
        assert!(!stdout.contains("bootstrap"), "{stdout}");
        if manifest.entries.len() > 1 {
            assert!(
                stdout.contains("Recorded System Operation Results"),
                "{stdout}"
            );
            assert!(
                stdout.contains(
                    "latest recorded results per item across system operations, not live checks"
                ),
                "{stdout}"
            );
            let rows: Vec<_> = stdout
                .lines()
                .filter(|line| line.contains("recorded detail"))
                .collect();
            assert_eq!(rows.len(), 3, "{stdout}");
            for (row, (label, status)) in rows.iter().zip([
                ("ASCII", "installed"),
                ("中文标签中文标签", "updated"),
                ("e\u{301}", "failed"),
            ]) {
                assert!(row.contains(label), "{row}");
                let (prefix, _) = row.split_once(status).unwrap();
                assert_eq!(console::measure_text_width(prefix), 21, "{row}");
            }
        } else {
            assert_eq!(
                stdout.trim(),
                format!(
                    "No system operation results recorded for {os}. Use `shine sys list` to view available items."
                )
            );
        }
        assert_eq!(fs::read_to_string(path).unwrap(), bytes);
    }
}

#[cfg(unix)]
#[test]
fn self_install_path_hint_preserves_literal_directory_and_existing_path() {
    let fixture = Fixture::new();
    let dir = fixture
        .0
        .join("space ' \" $HOME $(touch sentinel) `touch sentinel` \\ 中文");
    let dest = dir.join("shine");
    let output = fixture
        .command()
        .env("NO_COLOR", "1")
        .args(["self", "install", "--dest"])
        .arg(&dest)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert!(dest.is_file());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let command = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Add it to PATH, for example: "))
        .unwrap();
    let output = Command::new("/bin/sh")
        .current_dir(&fixture.0)
        .env("PATH", "/usr/bin:/bin")
        .arg("-c")
        .arg(format!("{command}; printf '%s' \"$PATH\""))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("{}:/usr/bin:/bin", dir.display())
    );
    assert!(!fixture.0.join("sentinel").exists());
    let config: toml::Value =
        toml::from_str(&fs::read_to_string(fixture.0.join("state/config.toml")).unwrap()).unwrap();
    assert_eq!(
        config["self_install_dest"].as_str(),
        Some(dest.to_str().unwrap())
    );
}

#[test]
fn migration_preview_encodes_diff_without_changing_applied_metadata() {
    let fixture = Fixture::new();
    let category = if cfg!(unix) {
        "demo\nFORGED\r\t\u{1b}"
    } else {
        "demo中文"
    };
    let root = fixture.0.join("presets");
    let app = root.join("app").join(category);
    fs::create_dir_all(&app).unwrap();
    let original = format!(
        "# preserved\tcomment\r\ndest = '~/.demo'\r\npost_install = {{ command = 'shine', args = ['app', 'artifact', 'apply', {}] }}\r\n[artifact]\r\nscript = 'build.ts'\r\n",
        toml::Value::String(category.into())
    );
    fs::write(app.join("shine.toml"), &original).unwrap();
    fs::write(
        app.join("build.ts"),
        "throw new Error('must not execute');\n",
    )
    .unwrap();
    let run = |args: &[&str]| {
        fixture
            .command()
            .env("NO_COLOR", "1")
            .args(["preset", "migrate"])
            .arg(&root)
            .args(args)
            .output()
            .unwrap()
    };
    let json = run(&["--dry-run", "--format", "json"]);
    assert!(json.status.success(), "{json:?}");
    assert!(json.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(
        report["files"][0]["target"],
        format!("app/{category}/shine.toml")
    );
    let preview = run(&["--dry-run"]);
    assert!(preview.status.success(), "{preview:?}");
    assert!(preview.stderr.is_empty());
    assert_eq!(
        fs::read(app.join("shine.toml")).unwrap(),
        original.as_bytes()
    );
    assert!(!fixture.0.join("state").exists());
    let text = String::from_utf8(preview.stdout).unwrap();
    assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
    assert!(!text.contains("\nFORGED"), "{text:?}");
    assert!(text.contains(r"# preserved\tcomment\r"), "{text:?}");
    assert_eq!(
        text.lines()
            .filter(|line| line.starts_with("--- a/"))
            .count(),
        1
    );
    assert_eq!(
        text.lines()
            .filter(|line| line.starts_with("+++ b/"))
            .count(),
        1
    );
    let applied = run(&["--yes", "--format", "json"]);
    assert!(applied.status.success(), "{applied:?}");
    let applied_report: serde_json::Value = serde_json::from_slice(&applied.stdout).unwrap();
    assert_eq!(applied_report["status"], "applied");
    let bytes = fs::read(app.join("shine.toml")).unwrap();
    assert_eq!(
        shine_core::digest::sha256_hex(&bytes),
        report["files"][0]["candidate_sha256"]
    );
    assert!(
        bytes
            .windows(b"# preserved\tcomment\n".len())
            .any(|w| w == b"# preserved\tcomment\n")
    );
    assert_eq!(
        fs::read_to_string(app.join("build.ts")).unwrap(),
        "throw new Error('must not execute');\n"
    );
}

#[cfg(unix)]
#[test]
fn blocked_plan_and_development_notice_encode_targets_without_executing() {
    let fixture = Fixture::new();
    let category = "demo\nFORGED\r\t\u{1b}";
    let target = format!("app/{category}");
    let app = fixture.0.join("state/presets/app").join(category);
    fs::create_dir_all(&app).unwrap();
    fs::write(
        fixture.0.join("state/config.toml"),
        "schema_version=2\n[env]\nENABLE='1'\n",
    )
    .unwrap();
    fs::write(app.join("shine.toml"), "metadata_schema_version=2\ndest='~/.demo'\n[permissions]\nschema_version=2\nopaque_code='unrestricted'\n[[files]]\nsource='config.txt'\ngenerator={script='generate.ts', runtime='bun', env=[{ source='ENABLE', sensitivity='plain' }], when_env='ENABLE'}\n").unwrap();
    fs::write(app.join("config.txt"), "original\n").unwrap();
    let sentinel = fixture.0.join("must-not-run");
    fs::write(
        app.join("generate.ts"),
        format!(
            "await Bun.write({:?}, 'ran');\n",
            sentinel.to_str().unwrap()
        ),
    )
    .unwrap();
    for trusted in [false, true] {
        if trusted {
            let grant = fixture
                .command()
                .args(["trust", "grant", &target, "--development", "--yes"])
                .output()
                .unwrap();
            assert!(grant.status.success(), "{grant:?}");
        }
        let output = fixture
            .command()
            .env("NO_COLOR", "1")
            .args(["app", "install", category, "--yes"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("security Plan is blocked; no changes were made"));
        assert!(
            stderr.contains("code analysis blocked this operation"),
            "{stderr}"
        );
        assert!(!sentinel.exists());
        assert!(!fixture.0.join("state/app-manifest.toml").exists());
        assert!(!fixture.0.join("home/.demo/config.txt").exists());
        for text in [&stdout, &stderr] {
            assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
            assert!(!text.contains("\nFORGED"), "{text:?}");
        }
        if trusted {
            assert!(stdout.contains(r"app/demo\nFORGED\r\t\u{1b} · code changes allowed"));
        } else {
            assert!(
                stderr.contains("original target (control characters; copyable command omitted)")
            );
            assert!(!stderr.contains("`shine trust inspect"));
        }
    }
}

#[test]
fn preset_migration_artifact_hint_preserves_category_argument() {
    for (category, posix, powershell) in [
        ("demo", "demo", "demo"),
        ("demo space", "'demo space'", "'demo space'"),
        ("it's", "'it'\\''s'", "'it''s'"),
        ("literal$HOME", "'literal$HOME'", "'literal$HOME'"),
        #[cfg(unix)]
        ("double\"quote", "'double\"quote'", ""),
    ] {
        let fixture = Fixture::new();
        let root = fixture.0.join("presets");
        let app = root.join("app").join(category);
        fs::create_dir_all(&app).unwrap();
        let quoted = toml::Value::String(category.into()).to_string();
        let original = format!(
            "dest = '~/.demo'\npost_install = {{ command = 'shine', args = ['app', 'artifact', 'apply', {quoted}] }}\n[artifact]\nscript = 'build.ts'\n"
        );
        fs::write(app.join("shine.toml"), &original).unwrap();
        let script = "throw new Error('artifact must not run during migration');\n";
        fs::write(app.join("build.ts"), script).unwrap();
        let output = fixture
            .command()
            .env("NO_COLOR", "1")
            .args(["preset", "migrate"])
            .arg(&root)
            .arg("--dry-run")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        let command = stdout
            .lines()
            .find_map(|line| line.strip_prefix("    Next: run `"))
            .and_then(|line| line.strip_suffix("` after relevant changes."))
            .unwrap();
        let expected = if cfg!(windows) { powershell } else { posix };
        assert_eq!(command, format!("shine app artifact apply {expected}"));
        #[cfg(unix)]
        {
            // Parse only: set positional arguments instead of executing the suggested operation.
            let parsed = Command::new("/bin/sh")
                .arg("-c")
                .arg(format!("set -- {command}; printf '%s\\0' \"$@\""))
                .env("HOME", fixture.0.join("home"))
                .output()
                .unwrap();
            assert!(parsed.status.success(), "{parsed:?}");
            assert!(parsed.stderr.is_empty(), "{parsed:?}");
            assert_eq!(
                parsed.stdout,
                format!("shine\0app\0artifact\0apply\0{category}\0").as_bytes()
            );
        }
        #[cfg(windows)]
        {
            // A local function captures arguments; no artifact command is invoked.
            let parsed = Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command"])
                .arg(format!(
                    "function shine {{ ConvertTo-Json -Compress -InputObject @($args) }}; {command}"
                ))
                .output()
                .unwrap();
            assert!(parsed.status.success(), "{parsed:?}");
            assert!(parsed.stderr.is_empty(), "{parsed:?}");
            let args: Vec<String> = serde_json::from_slice(&parsed.stdout).unwrap();
            assert_eq!(args, ["app", "artifact", "apply", category]);
        }
        assert_eq!(
            fs::read_to_string(app.join("shine.toml")).unwrap(),
            original
        );
        assert_eq!(fs::read_to_string(app.join("build.ts")).unwrap(), script);
        assert!(!fixture.0.join("state").exists());
    }
}

#[test]
fn dotenv_export_preview_counts_only_effective_non_secret_values() {
    let fixture = Fixture::new();
    let workspace = fixture.0.join("shine.workspace.toml");
    let base = fixture.0.join("base.toml");
    let overlay = fixture.0.join("production.toml");
    let workspace_text = "version = 2\n[env]\nfiles = ['base.toml', 'production.toml']\n";
    let base_text = "version = 1\n[plain]\nPUBLIC = 'public-value-sentinel'\nSHADOWED = 'shadowed-value-sentinel'\n[secret]\nTOKEN = 'secret-value-sentinel'\n";
    let overlay_text = "version = 1\n[secret]\nSHADOWED = true\n";
    fs::write(&workspace, workspace_text).unwrap();
    fs::write(&base, base_text).unwrap();
    fs::write(&overlay, overlay_text).unwrap();
    // Pending/unsealed secrets would fail if preview tried the secret export path.
    for include_secrets in [false, true] {
        for existing in [false, true] {
            let destination = fixture.0.join("export.env");
            if existing {
                fs::write(&destination, "existing-output-sentinel").unwrap();
            }
            let mut command = fixture.command();
            command
                .env("NO_COLOR", "1")
                .args([
                    "env",
                    "workspace",
                    "export",
                    "--format",
                    "dotenv",
                    "--mode",
                    "production",
                    "--dry-run",
                    "--output",
                ])
                .arg(&destination);
            if include_secrets {
                command.arg("--include-secrets");
            }
            if existing {
                command.arg("--force");
            }
            let output = command.output().unwrap();
            assert_eq!(output.status.code(), Some(0), "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
            let stdout = String::from_utf8(output.stdout).unwrap();
            let suffix = if include_secrets {
                "; --include-secrets will be applied during actual export."
            } else {
                "."
            };
            assert_eq!(
                stdout.trim(),
                format!(
                    "Would export 1 non-secret variable(s) for mode production to {:?}. Secrets were not decrypted or counted{suffix}",
                    destination.to_string_lossy()
                )
            );
            for sentinel in [
                "public-value-sentinel",
                "shadowed-value-sentinel",
                "secret-value-sentinel",
                "existing-output-sentinel",
            ] {
                assert!(!stdout.contains(sentinel), "{stdout}");
            }
            if existing {
                assert_eq!(
                    fs::read_to_string(&destination).unwrap(),
                    "existing-output-sentinel"
                );
                fs::remove_file(&destination).unwrap();
            } else {
                assert!(!destination.exists());
            }
            for (path, original) in [
                (&workspace, workspace_text),
                (&base, base_text),
                (&overlay, overlay_text),
            ] {
                assert_eq!(fs::read_to_string(path).unwrap(), original);
            }
        }
    }
}

#[test]
fn broker_policy_diffs_encode_controls_and_preserve_update_approval() {
    let fixture = Fixture::new();
    let workspace = fixture.0.join("shine.workspace.toml");
    fs::write(&workspace, "version = 2\n[env]\nfiles = ['env.toml']\n").unwrap();
    let source = fixture.0.join("env.toml");
    let source_bytes = "version = 1\n[secret]\nTOKEN = true\n[payload]\ndata = 'not-decrypted-by-policy-commands'\n";
    fs::write(&source, source_bytes).unwrap();
    let input = |operation: &str, argument: &str| {
        let mut command = fixture.command();
        command.args(["env", "broker", "policy", operation]);
        if operation == "diff" {
            command.arg("review");
        } else {
            command.args([
                "--name",
                "review",
                "--ssh-target",
                "example.invalid",
                "--project",
                "project\u{85}\u{9b}31m",
            ]);
        }
        command.arg("--workspace").arg(&workspace).args([
            "--mode",
            "dev",
            "--release",
            "TOKEN",
            "--",
            "echo",
            argument,
        ]);
        command
    };
    let added = input("add", "original").output().unwrap();
    assert!(added.status.success(), "{added:?}");
    let policy = fixture.0.join("state/ssh-secret-broker.toml");
    let original = fs::read(&policy).unwrap();
    // A workspace hash change puts the control-bearing project in both diffs.
    let workspace_bytes = "# changed\nversion = 2\n[env]\nfiles = ['env.toml']\n";
    fs::write(&workspace, workspace_bytes).unwrap();
    let mut modes = vec![
        (false, false, None),
        (false, true, None),
        (false, true, Some("FORCE_COLOR")),
        (false, true, Some("CLICOLOR_FORCE")),
    ];
    if cfg!(unix) {
        modes.extend([(true, false, None), (true, true, None)]);
    }
    for operation in ["diff", "update"] {
        for &(terminal_stdout, no_color, force_color) in &modes {
            let mut command = input(operation, "changed\targument");
            command
                .env("TERM", "xterm-256color")
                .env_remove("NO_COLOR")
                .env_remove("FORCE_COLOR")
                .env_remove("CLICOLOR_FORCE")
                .env_remove("CLICOLOR")
                .env_remove("IGNORE_IS_TERMINAL");
            if no_color {
                command.env("NO_COLOR", "1");
            }
            if let Some(key) = force_color {
                command.env(key, "1");
            }
            #[cfg(unix)]
            let (output, terminal) = if terminal_stdout {
                output_with_terminal(command, true)
            } else {
                (command.output().unwrap(), Vec::new())
            };
            #[cfg(not(unix))]
            let (output, terminal) = (command.output().unwrap(), Vec::new());
            let text = String::from_utf8_lossy(if terminal_stdout {
                &terminal
            } else {
                &output.stdout
            });
            assert_eq!(
                output.status.code(),
                Some(if operation == "diff" { 0 } else { 1 }),
                "{output:?}"
            );
            if operation == "update" {
                assert!(
                    String::from_utf8_lossy(&output.stderr)
                        .contains("reading policy update confirmation"),
                    "{output:?}"
                );
            }
            assert!(text.contains("project\\u{85}\\u{9b}31m"), "{text:?}");
            assert!(!text.contains(['\u{85}', '\u{9b}', '\t']), "{text:?}");
            let colored = force_color.is_some() || (terminal_stdout && !no_color);
            assert_eq!(text.contains("\x1b["), colored, "{text:?}");
            if colored {
                for prefix in ["\x1b[36m@@ ", "\x1b[32m+", "\x1b[31m-"] {
                    assert!(text.contains(prefix), "{text:?}");
                }
            }
            assert_eq!(fs::read(&policy).unwrap(), original);
        }
    }
    assert_eq!(fs::read_to_string(&workspace).unwrap(), workspace_bytes);
    assert_eq!(fs::read_to_string(source).unwrap(), source_bytes);
    assert!(!fixture.0.join("state/trust.toml").exists());
}

#[test]
fn broker_duplicate_policy_hint_preserves_policy_and_update_approval() {
    let fixture = Fixture::new();
    let workspace = fixture.0.join("shine.workspace.toml");
    fs::write(&workspace, "version = 2\n[env]\nfiles = ['env.toml']\n").unwrap();
    fs::write(fixture.0.join("env.toml"), "version = 1\n[secret]\nTOKEN = true\n[payload]\ndata = 'not-decrypted-by-policy-commands'\n").unwrap();
    let run = |operation: &str, argument: &str| {
        fixture
            .command()
            .env("NO_COLOR", "1")
            .args([
                "env",
                "broker",
                "policy",
                operation,
                "--name",
                "review",
                "--ssh-target",
                "example.invalid",
                "--workspace",
            ])
            .arg(&workspace)
            .args([
                "--mode",
                "dev",
                "--release",
                "TOKEN",
                "--",
                "echo",
                argument,
            ])
            .output()
            .unwrap()
    };
    let added = run("add", "original");
    assert_eq!(added.status.code(), Some(0), "{added:?}");
    assert!(added.stderr.is_empty(), "{added:?}");
    assert!(
        String::from_utf8_lossy(&added.stdout).contains("added SSH secret broker policy review")
    );
    let policy = fixture.0.join("state/ssh-secret-broker.toml");
    let original = fs::read(&policy).unwrap();
    let duplicate = run("add", "changed");
    assert_eq!(duplicate.status.code(), Some(1), "{duplicate:?}");
    assert!(duplicate.stdout.is_empty(), "{duplicate:?}");
    let error = String::from_utf8_lossy(&duplicate.stderr);
    for fragment in [
        "policy review already exists",
        "shine env broker policy update --name review",
        "policy parameters supplied again",
        "--ssh-target",
        "--workspace",
        "--mode",
        "--release or --release-all-declared",
        "command after `--`",
        "--project",
        "--remote-workspace",
        "Review and confirmation are still required for changes",
        "shine env broker policy update --help",
    ] {
        assert!(error.contains(fragment), "{error}");
    }
    assert!(!error.contains("policy update review"), "{error}");
    assert_eq!(fs::read(&policy).unwrap(), original);
    let help = fixture
        .command()
        .args(["env", "broker", "policy", "update", "--help"])
        .output()
        .unwrap();
    assert_eq!(help.status.code(), Some(0), "{help:?}");
    assert!(help.stderr.is_empty(), "{help:?}");
    let help = String::from_utf8_lossy(&help.stdout);
    for option in [
        "--name",
        "--ssh-target",
        "--workspace",
        "--mode",
        "--release",
        "--release-all-declared",
        "--project",
        "--remote-workspace",
    ] {
        assert!(help.contains(option), "{help}");
    }
    // Supplying the corrected arguments does not approve a changed policy.
    let update = run("update", "changed");
    assert_eq!(update.status.code(), Some(1), "{update:?}");
    assert!(
        String::from_utf8_lossy(&update.stdout).contains("changed"),
        "{update:?}"
    );
    assert!(
        String::from_utf8_lossy(&update.stderr).contains("reading policy update confirmation"),
        "{update:?}"
    );
    assert_eq!(fs::read(&policy).unwrap(), original);
}

#[cfg(target_os = "linux")]
#[test]
fn identity_generation_preserves_non_utf8_paths_and_replacement_boundaries() {
    use std::os::unix::{ffi::OsStringExt, fs::PermissionsExt};
    for in_parent in [false, true] {
        for mode in ["create", "conflict", "force", "failed", "invalid"] {
            for alias_exists in [false, true] {
                let fixture = Fixture::new();
                let bin = fixture.0.join("test-bin");
                fs::create_dir(&bin).unwrap();
                let program = bin.join("age-keygen");
                // Only public placeholders; never invoke a real backend or hardware.
                fs::write(
                    &program,
                    r#"#!/bin/sh
[ "$1" = -o ] && [ "$#" = 2 ] || exit 92
printf '%s' "$2" > "$SHINE_TEST_ARGV"
[ "$SHINE_TEST_MODE" != failed ] || exit 7
set -C
if [ "$SHINE_TEST_MODE" = invalid ]; then
    printf 'no recipient\n' > "$2"
else
    printf '# recipient: age1reviewplaceholder\n' > "$2"
fi
"#,
                )
                .unwrap();
                fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
                let component = std::ffi::OsString::from_vec(b"identity-\xff".to_vec());
                let raw = if in_parent {
                    fixture.0.join(&component).join("identity.txt")
                } else {
                    fixture.0.join(&component)
                };
                let alias = PathBuf::from(raw.to_string_lossy().into_owned());
                fs::create_dir_all(raw.parent().unwrap()).unwrap();
                fs::create_dir_all(alias.parent().unwrap()).unwrap();
                if alias_exists {
                    fs::write(&alias, b"unrelated sibling").unwrap();
                }
                if mode != "create" {
                    fs::write(&raw, b"old identity").unwrap();
                }
                let log = fixture.0.join("argv");
                let mut command = fixture.command();
                command
                    .env("PATH", &bin)
                    .env("NO_COLOR", "1")
                    .env("SHINE_TEST_ARGV", &log)
                    .env("SHINE_TEST_MODE", mode)
                    .args(["env", "secret", "identity", "init", "--output"])
                    .arg(&raw);
                if matches!(mode, "force" | "failed" | "invalid") {
                    command.arg("--force");
                }
                let result = command.output().unwrap();
                let success = matches!(mode, "create" | "force");
                assert_eq!(
                    result.status.code(),
                    Some(if success { 0 } else { 1 }),
                    "parent={in_parent} mode={mode} alias={alias_exists}: {result:?}"
                );
                if success {
                    assert!(result.stderr.is_empty(), "{result:?}");
                    assert_eq!(
                        fs::read(&raw).unwrap(),
                        b"# recipient: age1reviewplaceholder\n"
                    );
                    assert_eq!(
                        fs::metadata(&raw).unwrap().permissions().mode() & 0o777,
                        0o600
                    );
                    let stdout = String::from_utf8(result.stdout).unwrap();
                    assert!(
                        stdout.contains("non-UTF-8; copyable age_identity assignment omitted"),
                        "{stdout}"
                    );
                    assert!(!stdout.contains("Set age_identity = "), "{stdout}");
                } else {
                    assert_eq!(fs::read(&raw).unwrap(), b"old identity");
                    let error = String::from_utf8_lossy(&result.stderr);
                    let expected = match mode {
                        "conflict" => "already exists; pass --force",
                        "failed" => "failed",
                        _ => "no recipient found",
                    };
                    assert!(error.contains(expected), "{error}");
                }
                if mode == "conflict" {
                    assert!(!log.exists(), "conflict must not invoke keygen");
                } else {
                    let generated =
                        PathBuf::from(std::ffi::OsString::from_vec(fs::read(log).unwrap()));
                    if mode == "create" {
                        assert_eq!(generated, raw);
                    } else {
                        assert_eq!(
                            generated.parent().unwrap().parent().unwrap(),
                            raw.parent().unwrap()
                        );
                    }
                }
                if alias_exists {
                    assert_eq!(fs::read(&alias).unwrap(), b"unrelated sibling");
                } else {
                    assert!(!alias.exists());
                }
                for parent in [raw.parent().unwrap(), alias.parent().unwrap()] {
                    assert!(!fs::read_dir(parent).unwrap().any(|entry| {
                        entry
                            .unwrap()
                            .file_name()
                            .to_string_lossy()
                            .starts_with(".shine-identity-")
                    }));
                }
                assert!(!fixture.0.join("state/config.toml").exists());
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn identity_generation_passes_native_path_to_failing_backend() {
    use std::os::unix::{ffi::OsStringExt, fs::PermissionsExt};
    for touch_id in [false, true] {
        if touch_id && !cfg!(target_os = "macos") {
            continue;
        }
        let fixture = Fixture::new();
        let bin = fixture.0.join("test-bin");
        fs::create_dir(&bin).unwrap();
        let program = bin.join(if touch_id {
            "age-plugin-se"
        } else {
            "age-keygen"
        });
        // A failing substitute records argv without requiring the filesystem to
        // support raw filename bytes (the local macOS filesystem rejects them).
        fs::write(&program, "#!/bin/sh\nfor arg do output=$arg; done\nprintf '%s' \"$output\" > \"$SHINE_TEST_ARGV\"\nexit 7\n").unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        let raw = fixture
            .0
            .join(std::ffi::OsString::from_vec(b"identity-\xff".to_vec()));
        let alias = PathBuf::from(raw.to_string_lossy().into_owned());
        fs::write(&alias, b"unrelated sibling").unwrap();
        let log = fixture.0.join("argv");
        let mut command = fixture.command();
        command
            .env("PATH", &bin)
            .env("SHINE_TEST_ARGV", &log)
            .args(["env", "secret", "identity", "init", "--output"])
            .arg(&raw);
        if touch_id {
            command.arg("--touch-id");
        }
        let result = command.output().unwrap();
        assert_eq!(result.status.code(), Some(1), "{result:?}");
        assert!(result.stdout.is_empty(), "{result:?}");
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("failed"),
            "{result:?}"
        );
        assert_eq!(
            std::ffi::OsString::from_vec(fs::read(log).unwrap()),
            raw.as_os_str()
        );
        assert!(!raw.exists());
        assert_eq!(fs::read(alias).unwrap(), b"unrelated sibling");
        assert!(!fixture.0.join("state/config.toml").exists());
    }
}

#[test]
fn identity_list_encodes_display_fields_preserving_source_and_config() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    fs::create_dir(&state).unwrap();
    let name = if cfg!(unix) {
        "identity\nFORGED=path\r\t\u{1b}\\literal中文.txt"
    } else {
        "identity中文.txt"
    };
    let identity = fixture.0.join(name);
    let original = "# recipient: age1review\u{1b}\\literal中文\n";
    fs::write(&identity, original).unwrap();
    let config = format!(
        "schema_version = 2\nage_identity = {}\n",
        toml::Value::String(identity.to_str().unwrap().into())
    );
    fs::write(state.join("config.toml"), &config).unwrap();
    let output = fixture
        .command()
        .env("NO_COLOR", "1")
        .args(["env", "secret", "identity", "list"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.ends_with("  age1review\\u{1b}\\literal中文\n"),
        "{text:?}"
    );
    if cfg!(unix) {
        assert!(
            text.contains("identity\\nFORGED=path\\r\\t\\u{1b}\\literal中文.txt"),
            "{text:?}"
        );
    }
    assert_eq!(text.lines().count(), 1, "{text:?}");
    assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
    assert_eq!(fs::read_to_string(identity).unwrap(), original);
    assert_eq!(
        fs::read_to_string(state.join("config.toml")).unwrap(),
        config
    );
}

#[test]
fn proxy_list_encodes_stored_fields_without_revalidating_or_mutating_rules() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    fs::create_dir(&state).unwrap();
    let original = "schema_version = 2\n[[env_proxy]]\ncommand = \"probe\\nFORGED=command\\r\\t\\u001b\\\\literal中文\"\nwith = [\"TOKEN\\nFORGED=with\\u001b\", \"ALIAS=OTHER\"]\nenabled = false\n[[env_proxy]]\ncommand = \"normal\"\nwith = [\"TOKEN\"]\nenabled = true\n";
    fs::write(state.join("config.toml"), original).unwrap();
    let output = fixture
        .command()
        .env("NO_COLOR", "1")
        .args(["env", "proxy", "list"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "probe\\nFORGED=command\\r\\t\\u{1b}\\literal中文: TOKEN\\nFORGED=with\\u{1b}, ALIAS=OTHER (disabled)\nnormal: TOKEN (enabled)\n"
    );
    assert_eq!(
        fs::read_to_string(state.join("config.toml")).unwrap(),
        original
    );
    assert!(!state.join("proxy-manifest.toml").exists());
    assert!(!state.join("bin").exists());
}

#[cfg(unix)]
#[test]
fn identity_hint_round_trips_toml_and_survives_changed_directory() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let bin = fixture.0.join("test-bin");
    fs::create_dir_all(&bin).unwrap();
    let keygen = bin.join("age-keygen");
    // Public placeholder only: no real keys, device interaction or external keygen.
    fs::write(&keygen, "#!/bin/sh\n[ \"$1\" = '-o' ] && [ \"$#\" = 2 ] || exit 91\nprintf '# recipient: age1reviewplaceholder\\n' > \"$2\"\nprintf 'called\\n' >> \"$SHINE_TEST_KEYGEN_LOG\"\n").unwrap();
    fs::set_permissions(&keygen, fs::Permissions::from_mode(0o755)).unwrap();
    let log = fixture.0.join("keygen.log");
    let relative = PathBuf::from("keys/space ' quote\" back\\slash $HOME/identity.txt");
    let expected = fs::canonicalize(&fixture.0).unwrap().join(&relative);
    let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    )))
    .unwrap();
    let output = fixture
        .command()
        .env("PATH", path)
        .env("SHINE_TEST_KEYGEN_LOG", &log)
        .env("NO_COLOR", "1")
        .args(["env", "secret", "identity", "init", "--output"])
        .arg(&relative)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let original = fs::read(&expected).unwrap();
    assert_eq!(original, b"# recipient: age1reviewplaceholder\n");
    assert_eq!(fs::read_to_string(&log).unwrap(), "called\n");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let assignment = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Set age_identity = "))
        .and_then(|line| line.strip_suffix(" in config.toml so shine can find this identity."))
        .unwrap();
    let snippet = format!("age_identity = {assignment}\n");
    let parsed: toml::Value = toml::from_str(&snippet).unwrap();
    let value = parsed["age_identity"].as_str().unwrap();
    assert!(std::path::Path::new(value).is_absolute());
    assert_eq!(std::path::Path::new(value), expected);
    let state = fixture.0.join("state");
    assert!(
        !state.join("config.toml").exists(),
        "init must not apply its config hint"
    );
    fs::create_dir_all(&state).unwrap();
    fs::write(
        state.join("config.toml"),
        format!("schema_version = 2\n{snippet}"),
    )
    .unwrap();
    let elsewhere = fixture.0.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    for cwd in [&fixture.0, &elsewhere] {
        let listed = fixture
            .command()
            .current_dir(cwd)
            .env("NO_COLOR", "1")
            .args(["env", "secret", "identity", "list"])
            .output()
            .unwrap();
        assert_eq!(listed.status.code(), Some(0), "{listed:?}");
        assert!(listed.stderr.is_empty(), "{listed:?}");
        assert_eq!(
            String::from_utf8(listed.stdout).unwrap(),
            format!("{}  age1reviewplaceholder\n", expected.to_string_lossy())
        );
    }
    assert_eq!(fs::read(expected).unwrap(), original);
    assert_eq!(fs::read_to_string(log).unwrap(), "called\n");
}

#[cfg(unix)]
#[test]
fn identity_init_encodes_result_rows_preserving_argv_and_multiline_toml_hint() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let bin = fixture.0.join("bin");
    fs::create_dir(&bin).unwrap();
    let keygen = bin.join("age-keygen");
    // Public comment only; this substitute never creates or accesses a real key.
    fs::write(&keygen, "#!/bin/sh\n[ \"$1\" = '-o' ] && [ \"$#\" = 2 ] || exit 91\nprintf '%s' \"$2\" > \"$SHINE_TEST_ARGV\"\nprintf '# recipient: age1review\\033\\\\literal中文\\n' > \"$2\"\n").unwrap();
    fs::set_permissions(&keygen, fs::Permissions::from_mode(0o700)).unwrap();
    let identity = fixture
        .0
        .join("identity\nFORGED=init\r\t\u{1b}\\literal中文.txt");
    let argv = fixture.0.join("argv");
    let output = fixture
        .command()
        .env("PATH", &bin)
        .env("SHINE_TEST_ARGV", &argv)
        .env("NO_COLOR", "1")
        .args(["env", "secret", "identity", "init", "--output"])
        .arg(&identity)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        fs::read_to_string(argv).unwrap(),
        identity.to_str().unwrap()
    );
    assert_eq!(
        fs::read_to_string(&identity).unwrap(),
        "# recipient: age1review\u{1b}\\literal中文\n"
    );
    let text = String::from_utf8(output.stdout).unwrap();
    let mut rows = text.lines();
    assert!(
        rows.next()
            .unwrap()
            .ends_with("identity\\nFORGED=init\\r\\t\\u{1b}\\literal中文.txt"),
        "{text:?}"
    );
    assert_eq!(
        rows.next().unwrap(),
        "  recipient: age1review\\u{1b}\\literal中文"
    );
    // TOML may deliberately span lines; it must still refer to the original file.
    let assignment = text
        .split_once("Set age_identity = ")
        .unwrap()
        .1
        .strip_suffix(" in config.toml so shine can find this identity.\n")
        .unwrap();
    let config: toml::Value = toml::from_str(&format!("age_identity = {assignment}\n")).unwrap();
    assert_eq!(
        PathBuf::from(config["age_identity"].as_str().unwrap()),
        fs::canonicalize(identity).unwrap()
    );
    assert!(!fixture.0.join("state/config.toml").exists());
}

#[cfg(unix)]
#[test]
fn identity_hint_survives_working_directory_removed_by_keygen() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let work = fixture.0.join("work");
    let bin = fixture.0.join("bin");
    fs::create_dir_all(&work).unwrap();
    fs::create_dir_all(&bin).unwrap();
    let keygen = bin.join("age-keygen");
    // Keep the output outside the directory removed by this isolated substitute.
    fs::write(&keygen, "#!/bin/sh\n[ \"$1\" = '-o' ] && [ \"$2\" = '../identity.txt' ] && [ \"$#\" = 2 ] || exit 91\nprintf '# recipient: age1reviewplaceholder\\n' > \"$2\"\nprintf 'called\\n' >> \"$SHINE_TEST_KEYGEN_LOG\"\n/bin/rmdir \"$SHINE_TEST_CWD\"\n").unwrap();
    fs::set_permissions(&keygen, fs::Permissions::from_mode(0o755)).unwrap();
    let log = fixture.0.join("keygen.log");
    let output = fixture
        .command()
        .current_dir(&work)
        .env("PATH", &bin)
        .env("SHINE_TEST_CWD", &work)
        .env("SHINE_TEST_KEYGEN_LOG", &log)
        .env("NO_COLOR", "1")
        .args([
            "env",
            "secret",
            "identity",
            "init",
            "--output",
            "../identity.txt",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert!(!work.exists());
    assert_eq!(fs::read_to_string(&log).unwrap(), "called\n");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("generated age identity at ../identity.txt"),
        "{stdout}"
    );
    let value = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Set age_identity = "))
        .and_then(|line| line.strip_suffix(" in config.toml so shine can find this identity."))
        .unwrap();
    let snippet = format!("age_identity = {value}\n");
    let parsed: toml::Value = toml::from_str(&snippet).unwrap();
    let hinted = std::path::Path::new(parsed["age_identity"].as_str().unwrap());
    assert!(hinted.is_absolute());
    assert_eq!(
        hinted,
        fs::canonicalize(fixture.0.join("identity.txt")).unwrap()
    );
    assert_eq!(
        fs::read(hinted).unwrap(),
        b"# recipient: age1reviewplaceholder\n"
    );
    let state = fixture.0.join("state");
    assert!(!state.join("config.toml").exists());
    fs::create_dir_all(&state).unwrap();
    fs::write(
        state.join("config.toml"),
        format!("schema_version = 2\n{snippet}"),
    )
    .unwrap();
    let listed = fixture
        .command()
        .args(["env", "secret", "identity", "list"])
        .output()
        .unwrap();
    assert_eq!(listed.status.code(), Some(0), "{listed:?}");
    assert!(listed.stderr.is_empty(), "{listed:?}");
    assert!(String::from_utf8_lossy(&listed.stdout).contains("age1reviewplaceholder"));
    assert_eq!(fs::read_to_string(log).unwrap(), "called\n");
}

#[test]
fn sys_status_help_describes_recorded_current_os_results() {
    let fixture = Fixture::new();
    for args in [vec!["sys", "--help"], vec!["sys", "status", "--help"]] {
        let output = fixture.command().args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{args:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{args:?}: {output:?}");
        let help = String::from_utf8(output.stdout).unwrap();
        let help = help.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            help.contains("Show recorded results per item for the current OS, not live checks"),
            "{args:?}: {help}"
        );
        assert!(
            !help.contains("Show system bootstrap items previously initialized by shine"),
            "{args:?}: {help}"
        );
        assert!(
            !fixture.0.join("state").exists(),
            "{args:?} initialized state"
        );
    }
}

#[test]
fn uninstall_help_describes_default_cleanup_and_rejects_removed_purge() {
    let fixture = Fixture::new();
    for (args, expected) in [
        (
            vec!["uninstall"],
            "App targets remove the entire selected category cache, including extra files",
        ),
        (
            vec!["app", "uninstall"],
            "Without CATEGORY, remove the entire internal App cache tree; remove the App manifest only when no installation records remain",
        ),
        (
            vec!["shell", "uninstall"],
            "With TARGET, retain extra cache files and shared files still needed by installed sibling commands",
        ),
    ] {
        for flag in ["--help", "-h"] {
            let output = fixture.command().args(&args).arg(flag).output().unwrap();
            assert_eq!(output.status.code(), Some(0), "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
            let help = String::from_utf8(output.stdout).unwrap();
            let help = help.split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(help.contains("Internal cleanup is included"), "{help}");
            assert!(help.contains(expected), "{help}");
            assert!(help.contains("External preset sources"), "{help}");
            assert!(!help.contains("--purge"), "{help}");
        }
        let output = fixture
            .command()
            .args(&args)
            .args(["demo", "--purge"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected argument '--purge'"));
        assert!(output.stdout.is_empty());
        assert!(!fixture.0.join("state").exists());
    }
}

#[test]
fn app_uninstall_defaults_to_reviewed_internal_cleanup() {
    use shine_core::install::{AppEntry, AppInstallStrategy, AppManifest};
    for args in [
        vec!["uninstall", "app/git"],
        vec!["app", "uninstall", "git"],
        vec!["app", "uninstall"],
    ] {
        for external in [false, true] {
            let fixture = Fixture::new();
            // A project config without presets_dir selects embedded presets while keeping
            // SHINE_CONFIG_DIR isolated; SHINE_PRESETS below selects the external variant.
            fs::write(fixture.0.join("shine.config.toml"), "schema_version = 2\n").unwrap();
            let state = fixture.0.join("state");
            let cache = state.join("presets/app/git");
            let sibling = state.join("presets/app/sibling");
            fs::create_dir_all(&cache).unwrap();
            fs::create_dir_all(&sibling).unwrap();
            fs::write(cache.join("extra.txt"), "extra cache").unwrap();
            fs::write(sibling.join("extra.txt"), "sibling cache").unwrap();
            fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
            let manifest = AppManifest {
                entries: vec![AppEntry {
                    source: "app/git/gitconfig".to_string(),
                    destination: fixture.0.join("missing-config"),
                    backup: None,
                    content_hash: 1,
                    install_strategy: AppInstallStrategy::Copy,
                    uses_env: false,
                    requires_admin: false,
                }],
                ..AppManifest::default()
            };
            let receipt = toml::to_string(&manifest).unwrap();
            fs::write(state.join("app-manifest.toml"), &receipt).unwrap();
            let run = |flags: &[&str]| {
                let mut command = fixture.command();
                if external {
                    command.env("SHINE_PRESETS", state.join("presets"));
                }
                command.args(&args).args(flags).output().unwrap()
            };
            let preview = run(&["--dry-run"]);
            assert_eq!(
                preview.status.code(),
                Some(0),
                "{args:?} external={external}: {preview:?}"
            );
            let preview_text = String::from_utf8_lossy(&preview.stdout);
            assert!(!preview_text.contains("purged"), "{preview_text}");
            assert!(
                !preview_text.contains("preset cache removed"),
                "{preview_text}"
            );
            assert_eq!(
                preview_text.contains("would remove app"),
                !external,
                "{preview_text}"
            );
            assert_eq!(
                fs::read_to_string(state.join("app-manifest.toml")).unwrap(),
                receipt
            );
            assert_eq!(fs::read(cache.join("extra.txt")).unwrap(), b"extra cache");
            let denied = run(&[]);
            assert_eq!(denied.status.code(), Some(1), "{denied:?}");
            assert!(String::from_utf8_lossy(&denied.stderr).contains("approval requires"));
            assert_eq!(fs::read(cache.join("extra.txt")).unwrap(), b"extra cache");
            assert_eq!(
                fs::read_to_string(state.join("app-manifest.toml")).unwrap(),
                receipt
            );
            let removed = run(&["--yes"]);
            assert_eq!(
                removed.status.code(),
                Some(0),
                "{args:?} external={external}: {removed:?}"
            );
            let whole = args.len() == 2 && args[0] == "app";
            let removed_text = String::from_utf8_lossy(&removed.stdout);
            assert!(!removed_text.contains("would remove"), "{removed_text}");
            assert_eq!(
                removed_text.contains("preset cache removed"),
                !external,
                "{removed_text}"
            );
            assert_eq!(
                removed_text.contains("empty App manifest removed"),
                !external && whole,
                "{removed_text}"
            );
            if external {
                assert_eq!(fs::read(cache.join("extra.txt")).unwrap(), b"extra cache");
            } else {
                assert!(!cache.exists());
            }
            if external || !whole {
                assert_eq!(
                    fs::read(sibling.join("extra.txt")).unwrap(),
                    b"sibling cache"
                );
                let remaining: AppManifest =
                    toml::from_str(&fs::read_to_string(state.join("app-manifest.toml")).unwrap())
                        .unwrap();
                assert!(remaining.entries.is_empty());
            } else {
                assert!(!state.join("presets/app").exists());
                assert!(!state.join("app-manifest.toml").exists());
            }
            assert_eq!(
                fs::read(state.join("config.toml")).unwrap(),
                b"schema_version = 2\n"
            );
        }
    }
}

#[test]
fn whole_app_cleanup_keeps_protected_receipts_for_forced_retry() {
    use shine_core::install::{AppEntry, AppInstallStrategy, AppManifest, hash_content};
    for mixed in [false, true] {
        let fixture = Fixture::new();
        let state = fixture.0.join("state");
        let cache = state.join("presets/app/git");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("extra.txt"), "extra cache").unwrap();
        fs::write(fixture.0.join("shine.config.toml"), "schema_version = 2\n").unwrap();
        fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
        let protected = fixture.0.join("protected");
        let backup = fixture.0.join("protected.shine.bak");
        let normal = fixture.0.join("normal");
        fs::write(&protected, "user edit").unwrap();
        fs::write(&backup, "original").unwrap();
        let entry = AppEntry {
            source: "app/git/gitconfig".into(),
            destination: protected.clone(),
            backup: Some(backup.clone()),
            content_hash: hash_content(b"installed"),
            install_strategy: AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        };
        let mut manifest = AppManifest {
            entries: vec![entry.clone()],
            ..AppManifest::default()
        };
        if mixed {
            fs::write(&normal, "installed").unwrap();
            manifest.entries.push(AppEntry {
                source: "app/git/normal".into(),
                destination: normal.clone(),
                backup: None,
                ..entry.clone()
            });
        }
        let manifest_path = state.join("app-manifest.toml");
        let original_manifest = toml::to_string(&manifest).unwrap();
        fs::write(&manifest_path, &original_manifest).unwrap();
        let run = |flags: &[&str]| {
            fixture
                .command()
                .args(["app", "uninstall"])
                .args(flags)
                .output()
                .unwrap()
        };
        let preview = run(&["--dry-run"]);
        assert_eq!(preview.status.code(), Some(0), "{preview:?}");
        let text = String::from_utf8_lossy(&preview.stdout);
        assert!(text.contains("would remove app preset cache"), "{text}");
        assert!(!text.contains("empty App manifest"), "{text}");
        assert!(!text.contains("purged"), "{text}");
        assert_eq!(
            fs::read_to_string(&manifest_path).unwrap(),
            original_manifest
        );
        let removed = run(&["--yes"]);
        assert_eq!(removed.status.code(), Some(1), "{removed:?}");
        assert!(removed.stderr.is_empty(), "{removed:?}");
        let text = String::from_utf8_lossy(&removed.stdout);
        assert!(text.contains("Uninstall incomplete"), "{text}");
        assert!(text.contains("installation receipts retained"), "{text}");
        assert!(text.contains("app preset cache removed"), "{text}");
        assert!(!text.contains("empty App manifest removed"), "{text}");
        let remaining: AppManifest =
            toml::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
        assert_eq!(remaining.entries, vec![entry]);
        assert_eq!(fs::read(&protected).unwrap(), b"user edit");
        assert_eq!(fs::read(&backup).unwrap(), b"original");
        assert!(!normal.exists());
        assert!(!state.join("presets/app").exists());
        let repeated = run(&["--yes"]);
        assert_eq!(repeated.status.code(), Some(1), "{repeated:?}");
        assert!(!String::from_utf8_lossy(&repeated.stdout).contains("preset cache removed"));
        let forced_preview = run(&["--force", "--dry-run"]);
        assert_eq!(forced_preview.status.code(), Some(0), "{forced_preview:?}");
        assert!(
            String::from_utf8_lossy(&forced_preview.stdout)
                .contains("would remove empty App manifest")
        );
        assert!(manifest_path.exists());
        assert_eq!(fs::read(&protected).unwrap(), b"user edit");
        let forced = run(&["--force", "--yes"]);
        assert_eq!(forced.status.code(), Some(0), "{forced:?}");
        assert!(String::from_utf8_lossy(&forced.stdout).contains("empty App manifest removed"));
        assert_eq!(fs::read(&protected).unwrap(), b"original");
        assert!(!backup.exists());
        assert!(!manifest_path.exists());
    }
}

#[test]
fn shell_category_then_whole_uninstall_cleans_remaining_profile_with_approval() {
    fn config_files(dir: &std::path::Path, files: &mut Vec<PathBuf>) {
        for item in fs::read_dir(dir).unwrap() {
            let item = item.unwrap();
            if item.file_type().unwrap().is_dir() {
                config_files(&item.path(), files);
            } else if fs::read_to_string(item.path())
                .is_ok_and(|text| text.contains("# >>> shine >>>"))
            {
                files.push(item.path());
            }
        }
    }
    for category_args in [
        vec!["shell", "uninstall", "proxy"],
        vec!["uninstall", "shell/proxy"],
    ] {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("shine.config.toml"), "schema_version = 2\n").unwrap();
        let installed = fixture
            .command()
            .args(["shell", "install", "proxy", "--yes"])
            .output()
            .unwrap();
        assert_eq!(installed.status.code(), Some(0), "{installed:?}");
        let removed = fixture
            .command()
            .args(&category_args)
            .arg("--yes")
            .output()
            .unwrap();
        assert_eq!(removed.status.code(), Some(0), "{removed:?}");
        let state = fixture.0.join("state");
        let manifest = state.join("shell-manifest.toml");
        let manifest_before = fs::read(&manifest).unwrap();
        let profile = fs::read_dir(state.join("shell"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("profile.")
            })
            .unwrap();
        let profile_before = fs::read(&profile).unwrap();
        let mut configs = Vec::new();
        config_files(&fixture.0.join("home"), &mut configs);
        assert!(!configs.is_empty());
        let before = configs
            .iter()
            .map(|path| {
                let mut content = fs::read_to_string(path).unwrap();
                content.push_str("\n# user setting survives\n");
                fs::write(path, &content).unwrap();
                content.into_bytes()
            })
            .collect::<Vec<_>>();
        for flags in [vec!["--dry-run"], vec![]] {
            let output = fixture
                .command()
                .args(["shell", "uninstall"])
                .args(&flags)
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(if flags.is_empty() { 1 } else { 0 }),
                "{output:?}"
            );
            if flags.is_empty() {
                assert!(String::from_utf8_lossy(&output.stderr).contains("approval requires"));
            }
            assert_eq!(fs::read(&profile).unwrap(), profile_before);
            assert_eq!(fs::read(&manifest).unwrap(), manifest_before);
            for (path, content) in configs.iter().zip(&before) {
                assert_eq!(fs::read(path).unwrap(), *content);
            }
        }
        let removed = fixture
            .command()
            .args(["shell", "uninstall", "--yes"])
            .output()
            .unwrap();
        assert_eq!(removed.status.code(), Some(0), "{removed:?}");
        assert!(removed.stderr.is_empty(), "{removed:?}");
        assert!(!profile.exists());
        assert!(String::from_utf8_lossy(&removed.stdout).contains("Uninstall complete"));
        for path in &configs {
            let content = fs::read_to_string(path).unwrap();
            assert!(content.contains("# user setting survives"));
            assert!(!content.contains("# >>> shine >>>"));
        }
        let repeated = fixture
            .command()
            .args(["shell", "uninstall", "--yes"])
            .output()
            .unwrap();
        assert_eq!(repeated.status.code(), Some(0), "{repeated:?}");
    }
}

#[test]
fn shell_uninstall_defaults_to_cleanup_without_widening_target_scope() {
    for args in [
        vec!["uninstall", "shell/utils/shine-env-export"],
        vec!["shell", "uninstall", "utils/shine-env-export"],
        vec!["shell", "uninstall"],
    ] {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("shine.config.toml"), "schema_version = 2\n").unwrap();
        for target in ["utils/shine-env-export", "utils/shine-theme-sync"] {
            let installed = fixture
                .command()
                .args(["shell", "install", target, "--yes"])
                .output()
                .unwrap();
            assert_eq!(installed.status.code(), Some(0), "{installed:?}");
        }
        let state = fixture.0.join("state");
        let extra = state.join("presets/shell/utils/extra.txt");
        fs::write(&extra, "extra cache").unwrap();
        let selected = cli::bin_links::command_path_for_name(
            &state.join("bin"),
            std::ffi::OsStr::new("shine-env-export"),
        );
        let sibling = cli::bin_links::command_path_for_name(
            &state.join("bin"),
            std::ffi::OsStr::new("shine-theme-sync"),
        );
        let preview = fixture
            .command()
            .args(&args)
            .arg("--dry-run")
            .output()
            .unwrap();
        assert_eq!(preview.status.code(), Some(0), "{preview:?}");
        assert!(selected.exists());
        assert!(sibling.exists());
        assert_eq!(fs::read(&extra).unwrap(), b"extra cache");
        let denied = fixture.command().args(&args).output().unwrap();
        assert_eq!(denied.status.code(), Some(1), "{denied:?}");
        assert!(String::from_utf8_lossy(&denied.stderr).contains("approval requires"));
        assert!(selected.exists());
        assert_eq!(fs::read(&extra).unwrap(), b"extra cache");
        let removed = fixture.command().args(&args).arg("--yes").output().unwrap();
        assert_eq!(removed.status.code(), Some(0), "{removed:?}");
        assert!(
            fs::symlink_metadata(&selected)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        );
        if args.len() == 2 && args[0] == "shell" {
            assert!(!state.join("presets/shell").exists());
            assert!(!state.join("bin").exists());
        } else {
            assert!(sibling.exists());
            assert_eq!(fs::read(&extra).unwrap(), b"extra cache");
        }
        assert!(state.join("config.toml").exists());
    }
}

#[test]
fn shell_uninstall_help_describes_inventory_and_cleanup_scope() {
    let fixture = Fixture::new();
    for args in [
        vec!["shell", "--help"],
        vec!["shell", "uninstall", "--help"],
    ] {
        let output = fixture.command().args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{args:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{args:?}: {output:?}");
        let help = String::from_utf8(output.stdout).unwrap();
        let help = help.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            help.contains("Run 'shine list' to see installed resources"),
            "{args:?}: {help}"
        );
        assert!(
            !help.contains("Run 'shine shell list' to see installed categories"),
            "{args:?}: {help}"
        );
        if args.contains(&"uninstall") {
            assert_eq!(
                help.matches("Run 'shine list' to see installed resources")
                    .count(),
                2,
                "{help}"
            );
            for text in [
                "without TARGET, remove the entire internal Shell preset cache tree, including extra files",
                "With TARGET, retain extra cache files and shared files still needed by installed sibling commands",
                "External preset sources and config.toml are preserved",
            ] {
                assert!(help.contains(text), "{help}");
            }
            assert!(
                !help.contains("Also remove empty managed directories after uninstall"),
                "{help}"
            );
        }
        assert!(
            !fixture.0.join("state").exists(),
            "{args:?} initialized state"
        );
    }
}

#[test]
fn invalid_trust_store_notices_encode_paths_without_repairing_state() {
    for directory in [false, true] {
        let f = display_path_fixture();
        let state = f.0.join("state");
        fs::create_dir(&state).unwrap();
        fs::write(state.join("config.toml"), "schema_version=2\n").unwrap();
        let store = state.join("trust.toml");
        let invalid = "SECRET='PRIVATE_TRUST_CANARY' invalid\n";
        if directory {
            fs::create_dir(&store).unwrap();
        } else {
            fs::write(&store, invalid).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&store, fs::Permissions::from_mode(0o600)).unwrap();
            }
        }
        let out = f.command().args(["trust", "list"]).output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert!(out.stdout.is_empty());
        let text = String::from_utf8(out.stderr).unwrap();
        assert!(
            !text.contains(['\r', '\t', '\u{1b}']) && !text.contains("\nFORGED"),
            "{text:?}"
        );
        assert!(text.contains("trust.toml") && !text.contains("PRIVATE_TRUST_CANARY"));
        if directory {
            assert!(store.is_dir());
        } else {
            assert_eq!(fs::read_to_string(&store).unwrap(), invalid);
        }
        assert_eq!(
            fs::read_to_string(state.join("config.toml")).unwrap(),
            "schema_version=2\n"
        );
    }
}
