use std::collections::BTreeMap;
#[cfg(unix)]
use std::time::Duration;

use super::*;

#[cfg(unix)]
#[tokio::test]
async fn transfer_disconnect_and_cancellation_stop_silent_child_and_descendants() {
    use std::os::unix::fs::PermissionsExt;
    for cancel in [false, true] {
        let dir = crate::test_support::make_temp_dir("shine-transfer-stop").await;
        let pid_file = dir.join("pid");
        let ready = dir.join("ready");
        let marker = dir.join("continued");
        let script = dir.join("transfer.sh");
        std::fs::write(&script, format!(
                "#!/bin/sh\nprintf '%s' $$ > '{}'\n(sleep 1; printf continued > '{}') &\nprintf ready > '{}'\nwait\n",
                pid_file.display(), marker.display(), ready.display()
            )).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (mut server, client) = tokio::io::duplex(1024);
        let task = tokio::spawn(async move {
            spawn_and_relay(
                &mut server,
                TransferCommand {
                    program: script.display().to_string(),
                    args: vec![],
                    tool: Tool::Scp,
                },
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            while !ready.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let pid: i32 = std::fs::read_to_string(&pid_file).unwrap().parse().unwrap();
        if cancel {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            drop(client);
        } else {
            drop(client);
            assert!(
                tokio::time::timeout(Duration::from_secs(5), task)
                    .await
                    .unwrap()
                    .unwrap()
                    .is_err()
            );
        }
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(
            !marker.exists(),
            "descendant kept writing after transfer ended"
        );
        // Signal zero probes existence. The parent must also have been reaped.
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }
}

fn ctx_with(host: &str, ssh_options: Vec<&str>, control_path: Option<&str>) -> SessionContext {
    SessionContext {
        host: host.to_string(),
        ssh_options: ssh_options.into_iter().map(String::from).collect(),
        local_dir: PathBuf::from("/home/u/proj"),
        control_path: control_path.map(PathBuf::from),
    }
}

fn plan(direction: Direction, remote: &str, locals: &[&str], force: bool) -> TransferPlan {
    TransferPlan {
        direction,
        remote_spec: remote.to_string(),
        local_paths: locals.iter().map(PathBuf::from).collect(),
        force,
        dry_run: false,
    }
}

#[cfg(unix)]
#[tokio::test]
#[allow(clippy::await_holding_lock)] // Shared process environment, per test_support.
async fn transfer_tilde_hints_do_not_resolve_a_lossy_home_alias() {
    use std::os::unix::ffi::OsStringExt;
    let _guard = crate::test_support::env_lock();
    let root = std::env::temp_dir().join(format!("shine-transfer-home-{}", uuid::Uuid::new_v4()));
    let native = root.join(std::ffi::OsString::from_vec(b"home-\xff".to_vec()));
    let alias = root.join("home-\u{fffd}");
    std::fs::create_dir_all(&alias).unwrap();
    std::fs::write(alias.join("private.txt"), b"different directory").unwrap();
    // SAFETY: the shared environment guard restores HOME and sudo context after this test.
    unsafe {
        std::env::set_var("HOME", &native);
        std::env::remove_var("SUDO_USER");
    }
    let ctx = ctx_with("dev", vec![], None);
    assert_eq!(home::effective_home_dir(), native);
    let result = resolve_local_sources(&ctx, Some("~/private.txt"));
    assert!(
        result.is_err(),
        "resolved a different directory: {result:?}"
    );
    let result = resolve_local_sources(&ctx, Some("~/*.txt"));
    assert!(result.is_err(), "globbed a different directory: {result:?}");
    assert_eq!(
        resolve_local_sources(&ctx, Some("literal-$HOME-${TOKEN}.txt")).unwrap(),
        vec![ctx.local_dir.join("literal-$HOME-${TOKEN}.txt")]
    );
    assert_eq!(
        expand_hint(&ctx.local_dir, "/absolute").unwrap(),
        Path::new("/absolute")
    );
    assert_eq!(
        expand_hint(&ctx.local_dir, "~other/file").unwrap(),
        ctx.local_dir.join("~other/file")
    );
    assert!(resolve_local_dest(&ctx, Some("~/private.txt")).is_err());
    for direction in [Direction::Upload, Direction::Download] {
        for use_scp in [false, true] {
            for dry_run in [false, true] {
                for force in [false, true] {
                    let (mut server, mut client) = tokio::io::duplex(1024);
                    handle_transfer(
                        &mut server,
                        &ctx,
                        TransferRequest {
                            direction,
                            remote_spec: "/remote/file".into(),
                            local_spec: Some("~/private.txt".into()),
                            force,
                            dry_run,
                            use_scp,
                        },
                    )
                    .await
                    .unwrap();
                    assert!(matches!(
                        protocol::read_message::<_, ServerMessage>(&mut client).await.unwrap(),
                        ServerMessage::Error { message } if message.contains("not valid UTF-8")
                    ));
                }
            }
        }
    }
    assert_eq!(
        std::fs::read(alias.join("private.txt")).unwrap(),
        b"different directory"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(any(unix, windows))]
#[test]
fn transfer_argv_rejects_unrepresentable_local_paths() {
    #[cfg(unix)]
    let native = {
        use std::os::unix::ffi::OsStringExt;
        PathBuf::from(std::ffi::OsString::from_vec(b"local-\xff".to_vec()))
    };
    #[cfg(windows)]
    let native = {
        use std::os::windows::ffi::OsStringExt;
        PathBuf::from(std::ffi::OsString::from_wide(&[0xd800]))
    };
    let ctx = ctx_with("dev", vec![], None);
    for tool in [Tool::Rsync, Tool::Scp] {
        for direction in [Direction::Upload, Direction::Download] {
            let mut request = plan(direction, "/remote/file", &["normal.txt"], false);
            request.local_paths.push(native.clone());
            assert!(build_transfer_argv(tool, &ctx, &request).is_err());
        }
    }
    let mut ctx = ctx;
    ctx.local_dir = native;
    assert!(resolve_local_dest(&ctx, None).is_err());
    assert!(resolve_local_sources(&ctx, Some("relative.txt")).is_err());
    assert!(resolve_local_sources(&ctx, Some("*.txt")).is_err());
}

#[test]
fn scp_ipv6_operands_keep_the_complete_address_and_remote_path() {
    for (host, expected) in [
        ("2001:db8::1", "[2001:db8::1]:/remote/file"),
        ("alice@2001:db8::1", "alice@[2001:db8::1]:/remote/file"),
        ("alice@[2001:db8::1]", "alice@[2001:db8::1]:/remote/file"),
        ("alice@fe80::1%en0", "alice@[fe80::1%en0]:/remote/file"),
    ] {
        let ctx = ctx_with(host, vec![], None);
        for direction in [Direction::Download, Direction::Upload] {
            let request = plan(direction, "/remote/file", &["/local/file"], true);
            let command = build_transfer_argv(Tool::Scp, &ctx, &request).unwrap();
            let separator = command.args.iter().position(|arg| arg == "--").unwrap();
            let remote = if direction == Direction::Download {
                separator + 1
            } else {
                separator + 2
            };
            assert_eq!(command.args[remote], expected);
        }
    }
}

#[cfg(unix)]
#[tokio::test]
#[allow(clippy::await_holding_lock)] // Shared process environment, per test_support.
async fn ipv6_transfer_falls_back_to_scp_and_dry_run_never_spawns_a_copy() {
    use std::os::unix::fs::PermissionsExt;
    let _env = crate::test_support::env_lock();
    let root = crate::test_support::make_temp_dir("shine-ipv6-tool").await;
    let stub = root.join("rsync");
    let copied = root.join("copied");
    std::fs::write(
        &stub,
        format!(
            "#!/bin/sh\nif [ \"$1\" = --version ]; then exit 0; fi\nprintf copied > '{}'\n",
            copied.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    unsafe { std::env::set_var("SHINE_TEST_RSYNC", &stub) };
    let (mut server, mut client) = tokio::io::duplex(4096);
    let context = ctx_with("alice@2001:db8::1", vec![], None);
    handle_transfer(
        &mut server,
        &context,
        TransferRequest {
            direction: Direction::Download,
            remote_spec: "/remote/file".to_string(),
            local_spec: None,
            force: false,
            dry_run: true,
            use_scp: false,
        },
    )
    .await
    .unwrap();
    unsafe { std::env::remove_var("SHINE_TEST_RSYNC") };
    let starting: ServerMessage = protocol::read_message(&mut client).await.unwrap();
    assert!(matches!(
        starting,
        ServerMessage::Starting {
            tool: Tool::Scp,
            fell_back: true,
            note: Some(_)
        }
    ));
    let preview: ServerMessage = protocol::read_message(&mut client).await.unwrap();
    assert!(
        matches!(preview, ServerMessage::Log { chunk, .. } if chunk.contains("alice@[2001:db8::1]:/remote/file"))
    );
    assert!(matches!(
        protocol::read_message::<_, ServerMessage>(&mut client)
            .await
            .unwrap(),
        ServerMessage::Done { code: 0 }
    ));
    assert!(!copied.exists());
    tokio::fs::remove_dir_all(root).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn system_scp_passes_ipv6_address_intact_to_ssh() {
    use std::os::unix::fs::PermissionsExt;
    let scp = Path::new("/usr/bin/scp");
    if !scp.is_file() {
        eprintln!("system scp unavailable; skipping native parser check");
        return;
    }
    let root = crate::test_support::make_temp_dir("shine-scp-ipv6").await;
    let stub = root.join("ssh");
    let captured = root.join("argv");
    std::fs::write(
        &stub,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$SHINE_TEST_CAPTURE_ARGV\"\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    for host in ["2001:db8::1", "alice@2001:db8::1", "alice@fe80::1%en0"] {
        let ctx = ctx_with(host, vec![], None);
        let request = plan(
            Direction::Download,
            "/remote/file",
            &[root.to_str().unwrap()],
            true,
        );
        let mut command = build_transfer_argv(Tool::Scp, &ctx, &request).unwrap();
        command.program = scp.display().to_string();
        let separator = command.args.iter().position(|arg| arg == "--").unwrap();
        command.args.splice(
            separator..separator,
            ["-S".to_string(), stub.display().to_string()],
        );
        let output = command
            .process()
            .env("SHINE_TEST_CAPTURE_ARGV", &captured)
            .stdin(Stdio::null())
            .output()
            .await
            .unwrap();
        assert!(
            !output.status.success(),
            "the local SSH substitute must fail"
        );
        let argv = std::fs::read_to_string(&captured).unwrap();
        let address = host.rsplit_once('@').map_or(host, |(_, address)| address);
        assert!(argv.lines().any(|arg| arg == address), "{host}: {argv}");
        if host.starts_with("alice@") {
            assert!(argv.lines().any(|arg| arg == "alice"), "{argv}");
        }
    }
    tokio::fs::remove_dir_all(root).await.unwrap();
}

#[test]
fn rsync_remote_paths_preserve_spaces_and_globs_without_shell_evaluation() {
    let ctx = ctx_with("dev", vec![], None);
    let request = plan(
        Direction::Download,
        "/logs with space/a'\";$HOME/$(id)/*.log",
        &["/local"],
        false,
    );
    let command = build_transfer_argv(Tool::Rsync, &ctx, &request).unwrap();
    let separator = command.args.iter().position(|arg| arg == "--").unwrap();
    assert_eq!(
        command.args[separator + 1],
        "dev:/logs\\ with\\ space/a\\'\\\"\\;\\$HOME/\\$\\(id\\)/*.log"
    );
    let process = command.process();
    let env: BTreeMap<_, _> = process.as_std().get_envs().collect();
    assert_eq!(
        env[std::ffi::OsStr::new("RSYNC_OLD_ARGS")],
        Some(std::ffi::OsStr::new("1"))
    );
    assert_eq!(
        env[std::ffi::OsStr::new("RSYNC_PROTECT_ARGS")],
        Some(std::ffi::OsStr::new("0"))
    );
}

#[cfg(unix)]
#[tokio::test]
async fn system_rsync_transfers_remote_spaces_metacharacters_and_globs() {
    use std::os::unix::fs::PermissionsExt;
    let rsync = Path::new("/usr/bin/rsync");
    if !rsync.is_file() {
        eprintln!("system rsync unavailable; skipping native transfer check");
        return;
    }
    let dir = crate::test_support::make_temp_dir("shine-rsync-remote-paths").await;
    let remote = dir.join("remote files");
    let local = dir.join("local files");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::create_dir_all(&local).unwrap();
    let literal = "literal $HOME;$(printf injected)'\".txt";
    std::fs::write(remote.join(literal), b"literal contents").unwrap();
    for name in ["report a1.log", "report b2.log", "report c3.log"] {
        std::fs::write(remote.join(name), name).unwrap();
    }
    // Model SSH's remote-shell command boundary, but execute the peer rsync
    // locally. No SSH server, credentials, or external host is involved.
    let ssh = dir.join("ssh");
    std::fs::write(&ssh, "#!/bin/sh\nshift\nexec /bin/sh -c \"$*\"\n").unwrap();
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let ctx = ctx_with("fixture", vec![], None);
    for pattern in [literal, "report [!c]?.log"] {
        let request = plan(
            Direction::Download,
            remote.join(pattern).to_str().unwrap(),
            &[local.to_str().unwrap()],
            false,
        );
        let mut command = build_transfer_argv(Tool::Rsync, &ctx, &request).unwrap();
        command.program = rsync.display().to_string();
        let output = command
            .process()
            .env("RSYNC_RSH", &ssh)
            .output()
            .await
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    assert_eq!(
        std::fs::read(local.join(literal)).unwrap(),
        b"literal contents"
    );
    assert!(local.join("report a1.log").is_file());
    assert!(local.join("report b2.log").is_file());
    assert!(!local.join("report c3.log").exists());

    let destination = dir.join("upload destination 'quoted';$HOME");
    std::fs::create_dir(&destination).unwrap();
    let request = plan(
        Direction::Upload,
        destination.to_str().unwrap(),
        &[local.join(literal).to_str().unwrap()],
        true,
    );
    let mut command = build_transfer_argv(Tool::Rsync, &ctx, &request).unwrap();
    command.program = rsync.display().to_string();
    let output = command
        .process()
        .env("RSYNC_RSH", &ssh)
        .output()
        .await
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        std::fs::read(destination.join(literal)).unwrap(),
        b"literal contents"
    );
    tokio::fs::remove_dir_all(dir).await.unwrap();
}

#[test]
fn rsync_download_argv_uses_dashdash_and_control_master() {
    let ctx = ctx_with("dev", vec![], Some("/run/ctl.sock"));
    let plan = plan(
        Direction::Download,
        "/abs/logs/*.log",
        &["/home/u/proj"],
        false,
    );
    let cmd = build_transfer_argv(Tool::Rsync, &ctx, &plan).unwrap();
    assert_eq!(cmd.program, "rsync");
    assert!(cmd.args.contains(&"--".to_string()));
    assert!(cmd.args.contains(&"-a".to_string()));
    assert!(cmd.args.contains(&"--progress".to_string()));
    assert!(cmd.args.contains(&"--ignore-existing".to_string()));
    // -e string is built only from ctx (control master), never the wire.
    let e_idx = cmd.args.iter().position(|a| a == "-e").unwrap();
    assert_eq!(
        cmd.args[e_idx + 1],
        "ssh -o ControlPath=/run/ctl.sock -o ControlMaster=no"
    );
    // remote operand is host:spec, after the -- separator.
    let sep = cmd.args.iter().position(|a| a == "--").unwrap();
    assert_eq!(cmd.args[sep + 1], "dev:/abs/logs/*.log");
    assert_eq!(cmd.args[sep + 2], "/home/u/proj");
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn system_rsync_accepts_transfer_flags_and_preserves_existing_files() {
    let dir = crate::test_support::make_temp_dir("shine-system-rsync").await;
    let source = dir.join("source");
    let destination = dir.join("destination");
    tokio::fs::write(&source, b"new").await.unwrap();
    tokio::fs::write(&destination, b"existing").await.unwrap();
    let ctx = ctx_with("unused", vec![], None);
    for (dry_run, force, expected) in [
        (true, true, b"existing".as_slice()),
        (false, false, b"existing"),
        (false, true, b"new"),
    ] {
        let mut request = plan(Direction::Download, "/unused", &["/unused"], force);
        request.dry_run = dry_run;
        let command = build_transfer_argv(Tool::Rsync, &ctx, &request).unwrap();
        let separator = command.args.iter().position(|arg| arg == "--").unwrap();
        // Exercise the actual local rsync parser and copy without opening SSH.
        let output = std::process::Command::new("/usr/bin/rsync")
            .args(&command.args[..=separator])
            .arg(&source)
            .arg(&destination)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(tokio::fs::read(&destination).await.unwrap(), expected);
    }
    tokio::fs::remove_dir_all(dir).await.unwrap();
}

#[test]
fn rsync_upload_puts_local_sources_before_remote_dest() {
    let ctx = ctx_with("dev", vec![], Some("/run/ctl.sock"));
    let plan = plan(
        Direction::Upload,
        "/remote/dir",
        &["/home/u/proj/a.txt", "/home/u/proj/b.txt"],
        true,
    );
    let cmd = build_transfer_argv(Tool::Rsync, &ctx, &plan).unwrap();
    assert!(!cmd.args.contains(&"--ignore-existing".to_string())); // force set
    let sep = cmd.args.iter().position(|a| a == "--").unwrap();
    assert_eq!(cmd.args[sep + 1], "/home/u/proj/a.txt");
    assert_eq!(cmd.args[sep + 2], "/home/u/proj/b.txt");
    assert_eq!(cmd.args[sep + 3], "dev:/remote/dir");
}

#[test]
fn scp_download_argv_shape() {
    let ctx = ctx_with("dev", vec![], Some("/run/ctl.sock"));
    let plan = plan(Direction::Download, "/abs/file", &["/home/u/proj"], false);
    let cmd = build_transfer_argv(Tool::Scp, &ctx, &plan).unwrap();
    assert_eq!(cmd.program, "scp");
    assert!(cmd.args.contains(&"-r".to_string()));
    let sep = cmd.args.iter().position(|a| a == "--").unwrap();
    assert_eq!(cmd.args[sep + 1], "dev:/abs/file");
    assert_eq!(cmd.args[sep + 2], "/home/u/proj");
}

#[test]
fn scp_reconnect_translates_split_glued_and_clustered_ssh_options() {
    for options in [
        vec![
            "-v",
            "-p",
            "2222",
            "-l",
            "alice",
            "-S",
            "/tmp/control socket",
        ],
        vec!["-vp2222", "-lalice", "-S/tmp/control socket"],
    ] {
        let ctx = ctx_with("dev", options, None);
        assert_eq!(
            scp_connection_options(&ctx).unwrap(),
            [
                "-v",
                "-o",
                "Port=2222",
                "-o",
                "User=alice",
                "-o",
                "ControlPath=/tmp/control socket"
            ]
        );
        // The same conversion is used for explicit SCP and rsync fallback.
        let plan = plan(Direction::Download, "/file", &["/local"], false);
        let command = build_transfer_argv(Tool::Scp, &ctx, &plan).unwrap();
        assert!(
            !command
                .args
                .iter()
                .any(|arg| arg == "-p" || arg == "-l" || arg == "-S")
        );
        assert_eq!(
            &command.args[command.args.len() - 3..],
            ["--", "dev:/file", "/local"]
        );
    }
}

#[test]
fn scp_reconnect_preserves_connection_values_without_session_flags() {
    let ctx = ctx_with(
        "dev",
        vec![
            "-tt",
            "-R",
            "9000:localhost:9000",
            "-L8080:localhost:80",
            "-e",
            "none",
            "-Nn",
            "-F",
            "/tmp/ssh config",
            "-i/tmp/key",
            "-J",
            "jump",
            "-o",
            "ControlMaster=no",
            "-oProxyCommand=ssh jump nc %h %p",
            "-b127.0.0.1",
            "-B",
            "en0",
            "-m",
            "hmac-sha2-256",
            "-I",
            "/tmp/provider",
        ],
        None,
    );
    assert_eq!(
        scp_connection_options(&ctx).unwrap(),
        [
            "-F",
            "/tmp/ssh config",
            "-i",
            "/tmp/key",
            "-J",
            "jump",
            "-o",
            "ControlMaster=no",
            "-o",
            "ProxyCommand=ssh jump nc %h %p",
            "-o",
            "BindAddress=127.0.0.1",
            "-o",
            "BindInterface=en0",
            "-o",
            "MACs=hmac-sha2-256",
            "-o",
            "PKCS11Provider=/tmp/provider",
        ]
    );
    assert!(rsync_rsh(&ctx).is_err());
    for invalid in [vec!["-p"], vec!["-Z"], vec!["not-an-option"]] {
        assert!(scp_connection_options(&ctx_with("dev", invalid, None)).is_err());
    }
    let master = ctx_with("dev", vec!["-p2222", "-tt"], Some("/tmp/master"));
    assert_eq!(
        scp_connection_options(&master).unwrap(),
        ["-o", "ControlPath=/tmp/master", "-o", "ControlMaster=no"]
    );
}

#[test]
fn hostile_remote_spec_never_becomes_an_option() {
    let ctx = ctx_with("dev", vec![], Some("/run/ctl.sock"));
    let plan = plan(
        Direction::Download,
        "-oProxyCommand=evil",
        &["/home/u/proj"],
        false,
    );
    let cmd = build_transfer_argv(Tool::Rsync, &ctx, &plan).unwrap();
    // It only ever appears glued behind the host and after --.
    assert!(cmd.args.contains(&"dev:-oProxyCommand=evil".to_string()));
    assert!(!cmd.args.iter().any(|a| a == "-oProxyCommand=evil"));
}

#[test]
fn remote_spec_with_newline_is_rejected() {
    let ctx = ctx_with("dev", vec![], Some("/run/ctl.sock"));
    let plan = plan(Direction::Download, "a\nb", &["/home/u/proj"], false);
    assert!(build_transfer_argv(Tool::Rsync, &ctx, &plan).is_err());
}

#[test]
fn local_operand_leading_dash_is_dot_slashed() {
    let ctx = ctx_with("dev", vec![], Some("/run/ctl.sock"));
    let plan = plan(Direction::Upload, "/remote", &["-rf"], true);
    let cmd = build_transfer_argv(Tool::Rsync, &ctx, &plan).unwrap();
    assert!(cmd.args.contains(&"./-rf".to_string()));
    assert!(!cmd.args.iter().any(|a| a == "-rf"));
}

#[test]
fn rsync_rsh_rejects_whitespace_options() {
    let ctx = ctx_with("dev", vec!["-oProxyCommand=ssh jump nc %h %p"], None);
    assert!(rsync_rsh(&ctx).is_err());
}

#[test]
fn rsync_rsh_none_for_bare_alias() {
    let ctx = ctx_with("dev", vec![], None);
    assert_eq!(rsync_rsh(&ctx).unwrap(), None);
}

#[test]
fn choose_tool_prefers_rsync_but_falls_back() {
    assert_eq!(choose_tool(false, true).0, Tool::Rsync);
    assert_eq!(choose_tool(true, true).0, Tool::Scp);
    let (tool, fell_back, note) = choose_tool(false, false);
    assert_eq!(tool, Tool::Scp);
    assert!(fell_back);
    assert!(note.is_some());
}

#[test]
fn expand_hint_is_tilde_only_and_anchors_relative() {
    let _guard = crate::test_support::env_lock();
    let home = std::env::current_dir().unwrap().join("transfer-home");
    // SAFETY: the shared guard restores process state after the expansion assertions.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::remove_var("SUDO_USER");
    }
    let base = Path::new("/home/u/proj");
    assert_eq!(
        expand_hint(base, "sub/file").unwrap(),
        base.join("sub/file")
    );
    assert_eq!(expand_hint(base, "/abs").unwrap(), PathBuf::from("/abs"));
    assert_eq!(
        expand_hint(base, "$HOME/${TOKEN}").unwrap(),
        base.join("$HOME/${TOKEN}")
    );
    assert_eq!(expand_hint(base, "~").unwrap(), home);
    assert_eq!(
        expand_hint(base, "~/sub/file").unwrap(),
        home.join("sub/file")
    );
}

#[test]
fn detects_glob_metacharacters() {
    assert!(has_glob_metacharacters("*.log"));
    assert!(has_glob_metacharacters("a[0-9]"));
    assert!(!has_glob_metacharacters("plain.txt"));
}

#[test]
fn resolve_local_sources_expands_glob_and_errors_on_no_match() {
    let dir = std::env::temp_dir().join(format!("shine-glob-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), b"a").unwrap();
    std::fs::write(dir.join("b.txt"), b"b").unwrap();
    std::fs::write(dir.join("c.log"), b"c").unwrap();
    let ctx = SessionContext {
        host: "dev".to_string(),
        ssh_options: vec![],
        local_dir: dir.clone(),
        control_path: None,
    };

    let mut matched = resolve_local_sources(&ctx, Some("*.txt")).unwrap();
    matched.sort();
    assert_eq!(matched, vec![dir.join("a.txt"), dir.join("b.txt")]);

    // A non-glob source is returned verbatim (resolved), even if missing —
    // rsync/scp reports a missing single source itself.
    assert_eq!(
        resolve_local_sources(&ctx, Some("c.log")).unwrap(),
        vec![dir.join("c.log")]
    );

    assert!(resolve_local_sources(&ctx, Some("*.missing")).is_err());

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn scp_preview_keeps_operand_boundaries_and_control_characters_visible() {
    let ctx = ctx_with("dev", vec![], None);
    let locals = vec![
        PathBuf::from("/local/a b"),
        PathBuf::from("/local/\"q\"\nnext"),
    ];
    let expected_local = r#""/local/a b" "/local/\"q\"\nnext""#;
    let expected_remote = r#""dev:/remote/a 'b'""#;
    for direction in [Direction::Download, Direction::Upload] {
        let actual = describe_transfer(&ctx, direction, "/remote/a 'b'", &locals);
        let expected = match direction {
            Direction::Download => format!("{expected_remote} -> {expected_local}"),
            Direction::Upload => format!("{expected_local} -> {expected_remote}"),
        };
        assert_eq!(actual, expected);
        assert_eq!(actual.lines().count(), 1);
    }
}
