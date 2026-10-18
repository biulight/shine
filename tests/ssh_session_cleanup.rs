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

#[test]
fn ssh_cli_preserves_option_order_and_remote_wrapper_boundaries_in_both_modes() {
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
    fn decoded_command(command: &str) -> String {
        let encoded = command
            .rsplit("-EncodedCommand ")
            .next()
            .unwrap()
            .trim_matches('\'');
        let bytes = BASE64.decode(encoded).unwrap();
        String::from_utf16(
            &bytes
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }
    for mode in ["posix", "windows"] {
        let fixture = Fixture::new();
        let state = fixture.0.join("s");
        fs::create_dir_all(&state).unwrap();
        fs::write(
            state.join("config.toml"),
            "schema_version = 2\n[env]\nVALUE = \"O'Connor $HOME ; & literal\"\n",
        )
        .unwrap();
        let ssh = fixture.0.join("bin/ssh");
        fs::write(
            &ssh,
            "#!/bin/sh\nprintf '%s\\0' \"$@\" > \"$ARGV_CAPTURE\"\nexit 17\n",
        )
        .unwrap();
        fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
        let capture = fixture.0.join("argv");
        let options = [
            "-o",
            "ControlMaster=no",
            "-o",
            "ProxyJump=bastion",
            "-p2222",
        ];
        let output = Command::new(env!("CARGO_BIN_EXE_shine"))
            .args([
                "ssh",
                "--remote-shell",
                mode,
                "--with",
                "VALUE=FORWARDED",
                "--",
            ])
            .args(options)
            .args([
                "fixture.invalid",
                "echo",
                "spaced argument",
                "O'Connor",
                "$literal",
            ])
            .current_dir(&fixture.0)
            .env("SHINE_CONFIG_DIR", &state)
            .env_remove("SHINE_PRESETS")
            .env("PATH", fixture.0.join("bin"))
            .env("ARGV_CAPTURE", &capture)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(17), "{output:?}");
        let bytes = fs::read(capture).unwrap();
        let argv = bytes
            .split(|byte| *byte == 0)
            .filter(|arg| !arg.is_empty())
            .map(|arg| std::str::from_utf8(arg).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(&argv[..options.len()], options);
        assert_eq!(argv[options.len()], "-t");
        assert_eq!(argv[argv.len() - 2], "fixture.invalid");
        let wrapped = argv.last().unwrap();
        if mode == "posix" {
            assert_eq!(argv[options.len() + 1], "-R");
            assert!(wrapped.contains("FORWARDED='O'"));
            assert!(wrapped.contains("spaced argument"));
            assert!(wrapped.contains("$literal"));
            assert!(!argv.iter().any(|arg| arg.starts_with("ControlPath=")));
        } else {
            assert!(!argv.contains(&"-R"));
            assert!(!wrapped.contains("O'Connor"));
            let bootstrap = decoded_command(wrapped);
            let payload_encoded = bootstrap
                .split("-EncodedCommand '")
                .nth(1)
                .unwrap()
                .split('\'')
                .next()
                .unwrap();
            let payload = decoded_command(payload_encoded);
            assert!(payload.contains("$env:FORWARDED = 'O''Connor $HOME ; & literal'"));
            assert!(payload.contains("& 'echo' 'spaced argument' 'O''Connor' '$literal'"));
        }
        Fixture::assert_no_sessions(&state);
    }
}

// Exercise the CLI's output/exit boundary, not just the agent protocol.
async fn local_status_response(mode: &str) -> std::process::Output {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let fixture = Fixture::new();
    let socket = fixture.0.join("status.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let mode = mode.to_string();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let len = stream.read_u32_le().await.unwrap();
        let mut hello = vec![0; len as usize];
        stream.read_exact(&mut hello).await.unwrap();
        let hello: serde_json::Value = serde_json::from_slice(&hello).unwrap();
        assert_eq!(hello["type"], "Hello");
        if mode != "handshake_timeout" {
            let ack = serde_json::to_vec(&serde_json::json!({
                "type": "HelloAck", "protocol_version": hello["protocol_version"]
            }))
            .unwrap();
            stream.write_u32_le(ack.len() as u32).await.unwrap();
            stream.write_all(&ack).await.unwrap();
            let len = stream.read_u32_le().await.unwrap();
            let mut request = vec![0; len as usize];
            stream.read_exact(&mut request).await.unwrap();
            let request: serde_json::Value = serde_json::from_slice(&request).unwrap();
            assert_eq!(request["type"], "Status");
            assert_eq!(request["token"], "fixture-token");
            if mode != "response_timeout" {
                let response = if mode == "rejected" {
                    serde_json::json!({"type": "Error", "message": "invalid session token"})
                } else {
                    serde_json::json!({"type": "StatusResponse", "host": "fixture.invalid", "session_local_dir": "/fixture"})
                };
                let response = serde_json::to_vec(&response).unwrap();
                stream.write_u32_le(response.len() as u32).await.unwrap();
                stream.write_all(&response).await.unwrap();
                return;
            }
        }
        // Keep the connection alive until the client times out; EOF is not a timeout.
        assert_eq!(stream.read(&mut [0]).await.unwrap(), 0);
    });
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_shine"))
            .args(["local", "status"])
            .current_dir(&fixture.0)
            .env("SHINE_CONFIG_DIR", fixture.0.join("state"))
            .env("SHINE_SSH_SESSION", "fixture-session")
            .env("SHINE_SSH_TOKEN", "fixture-token")
            .env("SHINE_SSH_REMOTE_SOCK", &socket)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
            .unwrap();
        server.await.unwrap();
        output
    })
    .await
    .expect("local status must finish without a response")
}

#[tokio::test]
async fn local_status_handshake_timeout_reports_unreachable_with_success() {
    let output = local_status_response("handshake_timeout").await;
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("connection: unreachable"), "{output:?}");
    assert!(
        stdout.contains("connection timed out after 5 seconds"),
        "{output:?}"
    );
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[tokio::test]
async fn local_status_response_timeout_remains_a_command_error() {
    let output = local_status_response("response_timeout").await;
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("response timed out after 5 seconds"),
        "{output:?}"
    );
}

#[tokio::test]
async fn local_status_success_and_token_rejection_preserve_output_streams() {
    let output = local_status_response("connected").await;
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("connection: connected"), "{output:?}");
    assert!(stdout.contains("host:       fixture.invalid"), "{output:?}");
    assert!(stdout.contains("local dir:  /fixture"), "{output:?}");
    let output = local_status_response("rejected").await;
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("invalid session token"),
        "{output:?}"
    );
}
