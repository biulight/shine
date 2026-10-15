//! Session cleanup through the real CLI, without a network connection.
#![cfg(unix)]

use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        // Keep Unix socket paths within macOS's sockaddr_un limit.
        let root = PathBuf::from("/tmp").join(format!("ss-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("bin")).unwrap();
        Self(root)
    }

    fn run(&self, state: &std::path::Path) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_shine"))
            .args(["ssh", "fixture.invalid"])
            .current_dir(&self.0)
            .env("SHINE_CONFIG_DIR", state)
            .env_remove("SHINE_PRESETS")
            .env("PATH", self.0.join("bin"))
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap()
    }

    fn assert_no_sessions(state: &std::path::Path) {
        let sessions = state.join("run/ssh");
        assert!(
            !sessions.exists() || fs::read_dir(&sessions).unwrap().next().is_none(),
            "session resources remain in {}",
            sessions.display()
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn ssh_spawn_failure_removes_context_and_socket() {
    let fixture = Fixture::new();
    let state = fixture.0.join("s");
    let output = fixture.run(&state);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("failed to start ssh"),
        "{output:?}"
    );
    Fixture::assert_no_sessions(&state);
}

#[test]
fn ssh_listener_setup_failure_removes_partial_session_directory() {
    let fixture = Fixture::new();
    let state = fixture.0.join("a".repeat(110));
    let output = fixture.run(&state);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("binding local transfer socket"),
        "{output:?}"
    );
    Fixture::assert_no_sessions(&state);
}

#[test]
fn ssh_nonzero_exit_cleans_session_before_propagating_status() {
    let fixture = Fixture::new();
    let ssh = fixture.0.join("bin/ssh");
    fs::write(&ssh, "#!/bin/sh\nexit 17\n").unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
    let state = fixture.0.join("s");
    let output = fixture.run(&state);
    assert_eq!(output.status.code(), Some(17), "{output:?}");
    Fixture::assert_no_sessions(&state);
}
