//! Real CLI regressions with isolated state and local executables only.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn clear_serve_survives_upgrade_with_invalid_config() {
    let fixture = Fixture::new();
    let config = fixture.state().join("config.toml");
    fs::write(&config, "invalid = [").unwrap();
    let legacy = fixture.state().join("http/app/demo/keep.txt");
    fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    fs::write(&legacy, "user resource").unwrap();
    let (program, relative) = if cfg!(target_os = "macos") {
        (
            "launchctl",
            "Library/LaunchAgents/top.biulight.shine.http.plist",
        )
    } else {
        ("systemctl", ".config/systemd/user/shine-http.service")
    };
    let registration = fixture.0.join("fake-root-home").join(relative);
    fs::create_dir_all(registration.parent().unwrap()).unwrap();
    fs::write(&registration, "legacy registration").unwrap();
    let stub = fixture.0.join("tools").join(program);
    fs::write(
        &stub,
        r#"#!/bin/sh
if [ "${SHINE_SERVICE_FAIL:-}" = 1 ]; then exit 47; fi
if [ "$1" = list ] && [ -f "$SHINE_SERVICE_REGISTRATION" ]; then
    printf '%s\n' '- 0 top.biulight.shine.http'
fi
if [ "$2" = show ]; then
    printf '%s\n' 'LoadState=not-found' 'ActiveState=inactive'
fi
"#,
    )
    .unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
    let run = |fail: bool| {
        fixture
            .shine()
            .env("SHINE_SERVICE_REGISTRATION", &registration)
            .env("SHINE_SERVICE_FAIL", if fail { "1" } else { "0" })
            .args(["clear", "serve"])
            .output()
            .unwrap()
    };
    let failed = run(true);
    assert!(!failed.status.success());
    assert!(
        String::from_utf8_lossy(&failed.stderr).contains("legacy service cleanup"),
        "{failed:?}"
    );
    assert!(registration.exists());
    Fixture::assert_ok(run(false));
    assert!(!registration.exists());
    Fixture::assert_ok(run(false));
    assert_eq!(fs::read_to_string(&config).unwrap(), "invalid = [");
    assert_eq!(fs::read_to_string(&legacy).unwrap(), "user resource");
    assert!(!fixture.state().join("presets").exists());
}

struct Fixture(PathBuf);

#[cfg(any(target_os = "macos", target_os = "linux"))]
struct MaintenanceFixture(Fixture);

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl MaintenanceFixture {
    fn new() -> Self {
        let fixture = Fixture::new();
        fixture.initialize();
        let program = if cfg!(target_os = "macos") {
            "launchctl"
        } else {
            "systemctl"
        };
        let stub = fixture.0.join("tools").join(program);
        fs::write(
            &stub,
            r#"#!/bin/sh
query=0
if [ "$1" = list ] || [ "$2" = show ]; then query=1; fi
if [ "$query" = 1 ]; then
    if [ "${SHINE_TEST_FAIL:-0}" = 1 ]; then exit 47; fi
    present=1
    if [ -f "$SHINE_TEST_MAINTENANCE/cleared" ]; then present=0; fi
    printf '%s\n' probe >> "$SHINE_TEST_MAINTENANCE/calls"
    : > "$SHINE_TEST_MAINTENANCE/started"
    if [ "${SHINE_TEST_BLOCK:-0}" = 1 ]; then
        while [ ! -f "$SHINE_TEST_MAINTENANCE/release" ]; do sleep 0.02; done
    fi
    if [ "$1" = list ]; then
        if [ "$present" = 1 ]; then printf '%s\n' '- 0 top.biulight.shine.http'; fi
    elif [ "$present" = 1 ]; then
        printf '%s\n' 'LoadState=loaded' 'ActiveState=active' 'UnitFileState=disabled'
    else
        printf '%s\n' 'LoadState=not-found' 'ActiveState=inactive'
    fi
else
    : > "$SHINE_TEST_MAINTENANCE/cleared"
fi
"#,
        )
        .unwrap();
        fs::set_permissions(stub, fs::Permissions::from_mode(0o755)).unwrap();
        Self(fixture)
    }
    fn command(&self, args: &[&str], block: bool) -> Command {
        let mut command = self.0.shine();
        command
            .args(args)
            .env("SHINE_TEST_MAINTENANCE", &self.0.0)
            .env("SHINE_TEST_BLOCK", if block { "1" } else { "0" });
        command
    }
    fn interactive(&self, block: bool) -> (Output, String) {
        self.interactive_command(self.command(&["task", "list"], block))
    }
    fn interactive_command(&self, mut command: Command) -> (Output, String) {
        use std::io::Read;
        use std::os::fd::FromRawFd;
        let mut master = -1;
        let mut slave = -1;
        // SAFETY: openpty initializes the two descriptors; no termios or winsize override.
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
        // Stdio duplicates the slave to fd 2. Keep the original PTY descriptors out of
        // the worker: otherwise reading the master waits for an unrelated inherited fd.
        for fd in [master, slave] {
            assert_eq!(
                unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) },
                0
            );
        }
        let mut master = unsafe { fs::File::from_raw_fd(master) };
        let slave = unsafe { fs::File::from_raw_fd(slave) };
        // macOS may discard unread PTY data at the last slave close. Keep one local
        // descriptor alive until the child's completed output has been drained.
        let keep_alive = slave.try_clone().unwrap();
        command
            .env("TERM", "xterm-256color")
            .stderr(Stdio::from(slave));
        let output = command.output().unwrap();
        drop(command);
        use std::os::fd::AsRawFd;
        assert_eq!(
            unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) },
            0
        );
        let mut text = Vec::new();
        match master.read_to_end(&mut text) {
            Ok(_) => {}
            Err(error)
                if error.raw_os_error() == Some(libc::EIO)
                    || error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => panic!("reading maintenance PTY: {error}"),
        }
        drop(keep_alive);
        (Fixture::assert_ok(output), String::from_utf8(text).unwrap())
    }
    fn wait(&self, mut ready: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        while !ready() {
            assert!(
                std::time::Instant::now() < deadline,
                "maintenance worker did not finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    fn cache_root(&self) -> PathBuf {
        let mut scopes = fs::read_dir(self.0.state().join("maintenance")).unwrap();
        scopes.next().unwrap().unwrap().path()
    }
    fn cache(&self) -> serde_json::Value {
        serde_json::from_slice(&fs::read(self.cache_root().join("checks.json")).unwrap()).unwrap()
    }
    fn worker_finished(&self) -> bool {
        use fs2::FileExt;
        let path = self.cache_root().join("worker.lock");
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .ok()
            .is_some_and(|file| file.try_lock_exclusive().is_ok())
    }
    fn release(&self) {
        fs::write(self.0.0.join("release"), "").unwrap();
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn maintenance_background_survives_parent_exit_and_notifies_on_next_call() {
    let f = MaintenanceFixture::new();
    let start = std::time::Instant::now();
    let (_, first) = f.interactive(true);
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    assert!(first.is_empty(), "{first:?}");
    f.wait(|| f.0.0.join("started").exists());
    let (_, second) = f.interactive(true);
    assert!(second.is_empty(), "{second:?}");
    assert_eq!(
        fs::read_to_string(f.0.0.join("calls"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    f.release();
    f.wait(|| f.cache()["entries"]["serve-native"]["required"] == true);
    f.wait(|| f.worker_finished());
    let before = f.cache();
    let (output, third) = f.interactive(false);
    assert!(
        third.contains("shine clear serve"),
        "stderr={third:?}\nbefore={before}\nafter={}",
        f.cache()
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("shine clear serve"));
    assert_highlighted_advisories(&third, 1);
    let fourth = f.interactive(false).1;
    assert!(fourth.is_empty(), "{fourth:?}");
    f.wait(|| f.worker_finished());
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn assert_highlighted_advisories(text: &str, count: usize) {
    use owo_colors::{OwoColorize, Style};
    let normalized = text.replace("\r\n", "\n");
    let lines: Vec<_> = normalized.lines().collect();
    assert_eq!(lines.len(), count + 1, "{text:?}");
    assert_eq!(lines[count], "", "{text:?}");
    for line in &lines[..count] {
        let (_, styled_message) = line.split_once('m').expect("ANSI style prefix");
        let plain = styled_message.strip_suffix("\x1b[0m").expect("ANSI reset");
        // Compare the whole styled line, including the reset before the separator.
        assert_eq!(
            *line,
            plain.style(Style::new().bold().yellow()).to_string(),
            "{text:?}"
        );
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn maintenance_advisory_block_styles_findings_and_failures_and_respects_no_color() {
    let f = MaintenanceFixture::new();
    let relative = if cfg!(target_os = "macos") {
        "Library/LaunchAgents/top.biulight.shine.http.plist"
    } else {
        ".config/systemd/user/shine-http.service"
    };
    let registration = f.0.0.join("fake-root-home").join(relative);
    fs::create_dir_all(registration.parent().unwrap()).unwrap();
    fs::write(&registration, "legacy registration").unwrap();
    let mut command = f.command(&["state", "check"], false);
    command.env("SHINE_TEST_FAIL", "1");
    let (output, terminal) = f.interactive_command(command);
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 check(s) incomplete"));
    assert!(terminal.contains("shine clear serve"));
    assert!(terminal.contains("Warning:"));
    assert_highlighted_advisories(&terminal, 2);

    let mut command = f.command(&["state", "check"], false);
    command.env("SHINE_TEST_FAIL", "1").env("NO_COLOR", "1");
    let (_, plain) = f.interactive_command(command);
    assert!(!plain.contains('\x1b'), "{plain:?}");
    assert_eq!(plain.replace("\r\n", "\n").lines().count(), 3);
    assert!(plain.ends_with("\r\n\r\n"), "{plain:?}");

    fs::remove_file(registration).unwrap();
    fs::write(f.0.0.join("cleared"), "").unwrap();
    let (output, empty) = f.interactive_command(f.command(&["state", "check"], false));
    assert!(empty.is_empty(), "{empty:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("0 action(s) required"));
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn maintenance_clear_discards_a_result_from_an_already_running_worker() {
    let f = MaintenanceFixture::new();
    f.interactive(true);
    f.wait(|| f.0.0.join("started").exists());
    assert!(!f.worker_finished());
    Fixture::assert_ok(f.command(&["clear", "serve"], false).output().unwrap());
    let generation = f.cache()["generation"].clone();
    f.release();
    f.wait(|| f.worker_finished());
    let cache = f.cache();
    assert_eq!(cache["generation"], generation);
    assert!(cache["entries"]["serve-native"].is_null());
    Fixture::assert_ok(f.command(&["state", "check"], false).output().unwrap());
    assert!(!f.interactive(false).1.contains("shine clear serve"));
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn assert_superseded_maintenance_check(replacement: &[&str]) {
    let f = MaintenanceFixture::new();
    let first = f
        .command(&["state", "check"], true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    f.wait(|| f.0.0.join("started").exists());
    let replacement = f.command(replacement, false).output().unwrap();
    let cache = f.cache();
    f.release();
    let first = first.wait_with_output().unwrap();
    Fixture::assert_ok(replacement);
    assert!(!first.status.success(), "{first:?}");
    assert!(first.stdout.is_empty(), "{first:?}");
    let stderr = String::from_utf8(first.stderr).unwrap();
    assert!(stderr.contains("superseded"), "{stderr:?}");
    assert!(stderr.contains("retry `shine state check`"), "{stderr:?}");
    assert_eq!(stderr.lines().count(), 2, "{stderr:?}");
    assert!(stderr.ends_with("\n\n"), "{stderr:?}");
    assert!(!stderr.contains('\x1b'), "{stderr:?}");
    // The obsolete finding must not overwrite either a newer inspection or cleanup.
    assert_eq!(f.cache(), cache);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn maintenance_explicit_check_superseded_by_another_check_reports_retry() {
    assert_superseded_maintenance_check(&["state", "check"]);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn maintenance_explicit_check_superseded_by_cleanup_reports_retry() {
    assert_superseded_maintenance_check(&["clear", "serve"]);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn maintenance_noninteractive_is_quiet_and_explicit_check_bypasses_invalid_config_and_patch_gate() {
    let f = MaintenanceFixture::new();
    Fixture::assert_ok(f.command(&["task", "list"], false).output().unwrap());
    assert!(!f.0.state().join("maintenance").exists());
    assert!(!f.0.0.join("calls").exists());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut version = semver::Version::parse(env!("CARGO_PKG_VERSION")).unwrap();
    version.patch += 1;
    fs::write(
        f.0.state().join("update-check.json"),
        serde_json::json!({ "latest_version": version.to_string(), "checked_at_unix_secs": now })
            .to_string(),
    )
    .unwrap();
    let gated = f.command(&["task", "list"], false).output().unwrap();
    assert!(!gated.status.success());
    assert!(String::from_utf8_lossy(&gated.stderr).contains("newer patch release"));
    fs::write(f.0.state().join("config.toml"), "invalid = [").unwrap();
    let checked = Fixture::assert_ok(f.command(&["state", "check"], false).output().unwrap());
    assert!(String::from_utf8_lossy(&checked.stderr).contains("shine clear serve"));
    assert!(!checked.stderr.contains(&0x1b));
    assert!(checked.stderr.ends_with(b"\n\n"));
    assert!(String::from_utf8_lossy(&checked.stdout).contains("1 action(s) required"));
    assert_eq!(
        fs::read_to_string(f.0.state().join("config.toml")).unwrap(),
        "invalid = ["
    );
}

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
    fn shine(&self) -> Command {
        let mut command = self.command(Path::new(env!("CARGO_BIN_EXE_shine")));
        command.arg("--config-dir").arg(self.state());
        command
    }

    fn initialize(&self) {
        Self::assert_ok(
            self.shine()
                .args(["env", "get", "REVIEW_TOKEN"])
                .output()
                .unwrap(),
        );
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        fs::write(
            self.state().join("update-check.json"),
            format!(
                r#"{{"latest_version":"{}","checked_at_unix_secs":{now}}}"#,
                env!("CARGO_PKG_VERSION")
            ),
        )
        .unwrap();
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
            String::from_utf8_lossy(&output.stderr).contains(if force {
                "--force is retired"
            } else {
                "has no local entry"
            }),
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
fn task_save_rejects_lossy_cwd_without_replacing_existing_tasks() {
    use std::os::unix::ffi::OsStrExt;
    let fixture = Fixture::new();
    fixture.initialize();
    let raw = fixture.0.join(std::ffi::OsStr::from_bytes(b"project-\xff"));
    let replacement = fixture.0.join("project-\u{fffd}");
    fs::create_dir(&replacement).unwrap();
    let manifest = fixture.state().join("tasks.toml");
    let original = b"[tasks.keep]\ncommand = ['/bin/echo', 'original']\n";
    fs::write(&manifest, original).unwrap();
    let config = fs::read(fixture.state().join("config.toml")).unwrap();

    for force in [false, true] {
        let mut command = fixture.shine();
        command.args(["task", "save", if force { "keep" } else { "new" }]);
        if force {
            command.arg("--force");
        }
        let refused = command
            .arg("--cwd")
            .arg(&raw)
            .args(["--", "/bin/echo", "changed"])
            .output()
            .unwrap();
        assert_eq!(refused.status.code(), Some(1), "{refused:?}");
        assert!(
            String::from_utf8_lossy(&refused.stderr).contains("does not exist"),
            "{refused:?}"
        );
        assert!(!String::from_utf8_lossy(&refused.stdout).contains("Saved task"));
        assert_eq!(fs::read(&manifest).unwrap(), original);
        assert_eq!(
            fs::read(fixture.state().join("config.toml")).unwrap(),
            config
        );
        assert!(fs::read_dir(&replacement).unwrap().next().is_none());
    }
    let kept = Fixture::assert_ok(
        fixture
            .shine()
            .args(["task", "run", "keep"])
            .output()
            .unwrap(),
    );
    assert_eq!(kept.stdout, b"original\n");
    assert!(String::from_utf8_lossy(&kept.stderr).starts_with("Running keep:"));
    assert_eq!(fs::read(&manifest).unwrap(), original);
    let saved = Fixture::assert_ok(
        fixture
            .shine()
            .args(["task", "save", "valid", "--cwd"])
            .arg(&replacement)
            .args(["--", "/bin/pwd"])
            .output()
            .unwrap(),
    );
    assert!(String::from_utf8_lossy(&saved.stdout).contains("Saved task valid"));
    let valid = Fixture::assert_ok(
        fixture
            .shine()
            .args(["task", "run", "valid"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        valid.stdout,
        format!("{}\n", replacement.canonicalize().unwrap().display()).as_bytes()
    );
    assert!(String::from_utf8_lossy(&valid.stderr).starts_with("Running valid (cwd:"));
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

#[test]
fn concurrent_trust_revocations_do_not_restore_revoked_grants() {
    let fixture = Fixture::new();
    fixture.initialize();
    let path = fixture.state().join("trust.toml");
    let mut encoded = "schema_version = 2\n".to_owned();
    for index in 0..32 {
        encoded.push_str(&format!(
            "[[grants]]\nschema_version = 2\ntarget = 'app/demo-{index}'\ncapability = 'app-hook'\ncode_digest = '{}'\nmode = 'snapshot'\n", "0".repeat(64)));
    }
    fs::write(&path, encoded).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let mut children = Vec::new();
    for index in 0..32 {
        children.push(
            fixture
                .shine()
                .args(["trust", "revoke", &format!("app/demo-{index}")])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap(),
        );
    }
    for child in children {
        Fixture::assert_ok(child.wait_with_output().unwrap());
    }
    let store: shine_core::trust::TrustStoreV1 =
        toml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert!(
        store.grants.is_empty(),
        "successful revocations must remain revoked"
    );
}

#[test]
fn concurrent_broker_policy_adds_and_removes_preserve_successful_changes() {
    let fixture = Fixture::new();
    fixture.initialize();
    let workspace = fixture.0.join("project/shine.workspace.toml");
    fs::write(&workspace, "version = 2\n[env]\nfiles = ['source.toml']\n").unwrap();
    fs::write(
        fixture.0.join("project/source.toml"),
        "version = 1\n[secret]\nTOKEN = true\n[payload]\ndata = 'synthetic-ciphertext'\n",
    )
    .unwrap();
    let mut children = Vec::new();
    for index in 0..32 {
        children.push(
            fixture
                .shine()
                .args([
                    "env",
                    "broker",
                    "policy",
                    "add",
                    "--name",
                    &format!("policy-{index}"),
                    "--ssh-target",
                    &format!("host-{index}"),
                    "--workspace",
                ])
                .arg(&workspace)
                .args([
                    "--mode",
                    "development",
                    "--release",
                    "TOKEN",
                    "--",
                    "echo",
                    "test",
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
    let path = fixture.state().join("ssh-secret-broker.toml");
    let store: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(store["policy"].as_array().unwrap().len(), 32);
    let mut children = Vec::new();
    for index in 0..32 {
        children.push(
            fixture
                .shine()
                .args([
                    "env",
                    "broker",
                    "policy",
                    "remove",
                    &format!("policy-{index}"),
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
    let store: toml::Value = toml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert!(store["policy"].as_array().unwrap().is_empty());
}

#[test]
fn identity_force_stages_replacement_and_preserves_old_key_on_failure() {
    let fixture = Fixture::new();
    let keygen = fixture.0.join("tools/age-keygen");
    fs::write(
        &keygen,
        r#"#!/bin/sh
[ "$1" = '-o' ] && [ "$#" = 2 ] || exit 11
[ ! -e "$2" ] || exit 12
case "${SHINE_TEST_KEYGEN_MODE:-success}" in
fail) printf 'partial' > "$2"; exit 13 ;;
invalid) printf 'no recipient' > "$2" ;;
*) printf '# public key: age1review\nAGE-SECRET-KEY-REVIEW\n' > "$2" ;;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(keygen, fs::Permissions::from_mode(0o755)).unwrap();
    let output = fixture.0.join("project/identity.txt");
    let original = b"original private identity\n";
    fs::write(&output, original).unwrap();
    fs::set_permissions(&output, fs::Permissions::from_mode(0o600)).unwrap();
    let run = |force: bool, mode: &str| {
        let mut command = fixture.shine();
        command
            .env("SHINE_TEST_KEYGEN_MODE", mode)
            .args(["env", "secret", "identity", "init", "--output"])
            .arg(&output);
        if force {
            command.arg("--force");
        }
        command.output().unwrap()
    };
    assert!(!run(false, "success").status.success());
    assert_eq!(fs::read(&output).unwrap(), original);
    for mode in ["fail", "invalid"] {
        let failed = run(true, mode);
        assert!(!failed.status.success(), "{failed:?}");
        assert_eq!(fs::read(&output).unwrap(), original);
        assert!(
            !fs::read_dir(output.parent().unwrap())
                .unwrap()
                .any(|entry| entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".shine-identity-"))
        );
    }
    Fixture::assert_ok(run(true, "success"));
    assert_eq!(
        fs::read_to_string(&output).unwrap(),
        "# public key: age1review\nAGE-SECRET-KEY-REVIEW\n"
    );
    assert_eq!(
        fs::metadata(&output).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(
        !fs::read_dir(output.parent().unwrap())
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".shine-identity-"))
    );
}
