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
        "metadata_schema_version = 2\ndest = {dest}\npost_install = {{ command = '/bin/sh', args = ['-c', 'exit 6'] }}\npost_upgrade = {{ command = '/bin/sh', args = ['-c', 'exit 7'] }}\n[permissions]\nschema_version = 1\ncommands = ['/bin/sh']\nfilesystem = [{{ access = ['execute'], base = 'preset', path = 'generate.sh' }}]\nenvironment = [{{ name = 'ENABLE', sensitivity = 'plain' }}]\n[[files]]\nsource = 'config.txt'\nrestart_hint = 'Restart demo'\ngenerator = {{ script = 'generate.sh', env = ['ENABLE'], when_env = 'ENABLE' }}\n"
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
            "metadata_schema_version = 2\ndest = {dest}\n[permissions]\nschema_version = 1\nfilesystem = [{{ access = ['execute'], base = 'preset', path = 'generate.sh' }}]\nenvironment = [{{ name = 'ENABLE', sensitivity = 'plain' }}]\n[[files]]\nsource = 'config.json'\ninstall_mode = 'json-merge'\nmanaged_keys = ['demo']\ngenerator = {{ script = 'generate.sh', env = ['ENABLE'], when_env = 'ENABLE' }}\n{extra}"
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
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd};

    for stderr_terminal in [false, true] {
        for no_color in [false, true] {
            let fixture = Fixture::new();
            let mut master = -1;
            let mut slave = -1;
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
            // SAFETY: each successful openpty descriptor is owned exactly once.
            let mut master = unsafe { fs::File::from_raw_fd(master) };
            let slave = unsafe { fs::File::from_raw_fd(slave) };
            // Keep unread macOS PTY data alive until after the child exits.
            let keep_alive = slave.try_clone().unwrap();
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
            if stderr_terminal {
                command.stderr(Stdio::from(slave));
            } else {
                command.stdout(Stdio::from(slave));
            }
            let output = command.output().unwrap();
            drop(command);
            assert_eq!(output.status.code(), Some(1), "{output:?}");
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
                Err(error) => panic!("reading warning PTY: {error}"),
            }
            drop(keep_alive);
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
        assert_eq!(lines[1], "Command     cargo build");
        assert_eq!(lines.len(), if fixed { 3 } else { 2 });
        if fixed {
            assert!(lines[2].starts_with("Working dir "), "{text}");
        }
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
    let malformed = "invalid = [";
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
fn preset_migration_artifact_hint_preserves_category_argument() {
    for (category, posix, powershell) in [
        ("demo", "demo", "'demo'"),
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
                    "Would export 1 non-secret variable(s) for mode production to {}. Secrets were not decrypted or counted{suffix}",
                    destination.display()
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
