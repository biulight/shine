use super::*;
use crate::runtime::{
    HostOperation, InMemoryHost, NullObserver, PresetSnapshot, PresetSourceKind, RuntimeContext,
    RuntimePlatform,
};
use std::{future::Future, pin::Pin};

struct Interaction {
    allow: bool,
    admin_calls: usize,
}
impl RuntimeInteraction for Interaction {
    fn confirm(&mut self, _: &'static str, default: bool) -> Result<bool> {
        Ok(default)
    }
    fn authorize_admin<'a>(
        &'a mut self,
        _: usize,
    ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send + 'a>> {
        self.admin_calls += 1;
        Box::pin(async { Ok(self.allow) })
    }
    fn select_many(
        &mut self,
        _: &'static str,
        _: &[String],
        defaults: &[String],
    ) -> Result<Vec<String>> {
        Ok(defaults.to_vec())
    }
}
fn runtime(missing_env: bool) -> CoreRuntime<InMemoryHost> {
    let root = std::env::temp_dir().join("shine-bootstrap-contracts");
    let metadata = format!(
        "version = 2\n[[items]]\nid = 'first'\nlabel = 'First'\ndetect = {{ kind = 'path', path = '~/.first' }}\ninstall = {{ kind = 'package', provider = 'apt', package = 'first' }}\n[[items]]\nid = 'second'\nlabel = 'Second'\nrequired_env = [{}]\ndetect = {{ kind = 'path', path = '~/.second' }}\ninstall = {{ kind = 'package', provider = 'apt', package = 'second' }}\n",
        if missing_env { "'TOKEN'" } else { "" }
    );
    CoreRuntime::new(
        InMemoryHost::new(),
        RuntimeContext::isolated(
            root.join("home"),
            root.join("state"),
            root.join("presets"),
            root.join("bin"),
            RuntimePlatform::Linux,
        ),
        PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file("sys/ubuntu/shine.toml", metadata.into_bytes())
            .build(),
    )
}
fn request() -> SysBootstrapBatchRequest {
    SysBootstrapBatchRequest {
        os_id: "ubuntu".into(),
        requested: vec!["first".into(), "second".into()],
        preset: None,
        interactive: false,
        sys_shell: "bash".into(),
        dry_run: false,
        force_profile: false,
    }
}
#[tokio::test]
async fn batch_preflights_later_items_before_any_probe_install_or_admin_prompt() {
    let runtime = runtime(true);
    let mut interaction = Interaction {
        allow: true,
        admin_calls: 0,
    };
    let error = runtime
        .run_sys_bootstrap_batch(request(), &mut interaction, &mut NullObserver)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("requires environment variable `TOKEN`"),
        "{error:#}"
    );
    assert_eq!(interaction.admin_calls, 0);
    assert!(
        runtime
            .host()
            .operations()
            .iter()
            .all(|op| matches!(op, HostOperation::Read(_)))
    );
}
#[tokio::test]
async fn batch_stops_after_first_timeout_and_denied_authorization_never_runs_installer() {
    for allow in [true, false] {
        let runtime = runtime(false);
        runtime.host().queue_process_output(Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "deadline",
        )
        .into()));
        let mut interaction = Interaction {
            allow,
            admin_calls: 0,
        };
        let report = runtime
            .run_sys_bootstrap_batch(request(), &mut interaction, &mut NullObserver)
            .await
            .unwrap();
        assert_eq!(report.outcomes.len(), 1);
        assert_eq!(report.outcomes[0].item_id, "first");
        assert_eq!(report.outcomes[0].status, SysItemStatus::Failed);
        assert_eq!(interaction.admin_calls, 1);
        assert_eq!(runtime.host().process_requests().len(), usize::from(allow));
        assert!(report.outcomes[0].detail.contains(if allow {
            "timed out after 30 minutes"
        } else {
            "administrator authorization was not granted"
        }));
    }
}
#[tokio::test]
async fn selection_deduplicates_in_request_order_without_process_effects() {
    let runtime = runtime(false);
    let mut interaction = Interaction {
        allow: false,
        admin_calls: 0,
    };
    let selection = runtime
        .resolve_sys_bootstrap_selection(
            "ubuntu",
            &["second".into(), "first".into(), "second".into()],
            None,
            false,
            &mut interaction,
            &mut NullObserver,
        )
        .await
        .unwrap();
    assert_eq!(selection.item_ids, ["second", "first"]);
    assert_eq!(selection.source, SelectionSource::Items);
    assert!(runtime.host().process_requests().is_empty());
}
#[tokio::test]
async fn provider_invocation_keeps_sudo_proxy_order_limits_and_platform_rejection() {
    let mut runtime = runtime(false);
    runtime.context_mut_for_test().proxy_env =
        BTreeMap::from([("HTTPS_PROXY".into(), "http://proxy.test".into())]);
    runtime
        .host()
        .queue_process_output(Ok(crate::runtime::ProcessOutput::default()));
    runtime
        .run_package(
            "ubuntu",
            SysPackageProvider::Apt,
            "pkg",
            true,
            "pkg",
            &mut NullObserver,
        )
        .await
        .unwrap();
    let requests = runtime.host().process_requests();
    let request = &requests[0];
    assert_eq!(request.program, "sudo");
    assert_eq!(
        request.args,
        [
            "-n",
            "env",
            "HTTPS_PROXY=http://proxy.test",
            "apt-get",
            "install",
            "-y",
            "pkg"
        ]
    );
    assert_eq!(request.timeout, Some(BOOTSTRAP_TIMEOUT));
    assert_eq!(request.stdout_limit, Some(MAX_LOG_BYTES));
    assert_eq!(request.stderr_limit, Some(MAX_LOG_BYTES));
    assert!(request.inherit_stdin);
    assert_eq!(request.io, crate::runtime::ProcessIo::CapturedTail);
    assert_eq!(
        package_preview("ubuntu", SysPackageProvider::Apt, "pkg").unwrap(),
        "sudo apt-get install -y pkg"
    );
    assert!(package_preview("windows", SysPackageProvider::Homebrew, "pkg").is_err());
    assert!(
        runtime
            .run_package(
                "macos",
                SysPackageProvider::Apt,
                "pkg",
                false,
                "pkg",
                &mut NullObserver
            )
            .await
            .is_err()
    );
    assert_eq!(runtime.host().process_requests().len(), 1);
    let winget = provider::package_command(
        "windows",
        SysPackageProvider::Winget,
        "Publisher.Tool",
        &BTreeMap::from([("SHINE_SYS_PROXY".into(), "http://proxy.test".into())]),
    )
    .unwrap();
    assert_eq!(winget.program, "winget");
    assert_eq!(
        winget.args,
        [
            "install",
            "--exact",
            "--id",
            "Publisher.Tool",
            "--accept-package-agreements",
            "--accept-source-agreements",
            "--proxy",
            "http://proxy.test"
        ]
    );
    assert!(!winget.requires_admin);
}

#[tokio::test]
async fn pacman_uses_fixed_install_only_argv_with_root_and_sudo_proxy_paths() {
    for admin in [false, true] {
        let mut runtime = runtime(false);
        runtime.context_mut_for_test().running_as_admin = admin;
        runtime.context_mut_for_test().proxy_env =
            BTreeMap::from([("HTTPS_PROXY".into(), "http://proxy.test".into())]);
        runtime
            .host()
            .queue_process_output(Ok(crate::runtime::ProcessOutput {
                exit_code: Some(1),
                stderr: b"database is locked".to_vec(),
                ..Default::default()
            }));
        let result = runtime
            .run_package(
                "arch",
                SysPackageProvider::Pacman,
                "neovim",
                true,
                "neovim",
                &mut NullObserver,
            )
            .await
            .unwrap();
        assert!(!result.success);
        let requests = runtime.host().process_requests();
        assert_eq!(
            requests.len(),
            1,
            "never retry with refresh, upgrade or lock removal"
        );
        let request = &requests[0];
        let mut expected = vec![];
        if !admin {
            expected.extend(["-n", "env", "HTTPS_PROXY=http://proxy.test", "pacman"]);
        }
        expected.extend(["-S", "--needed", "--noconfirm", "neovim"]);
        assert_eq!(request.program, if admin { "pacman" } else { "sudo" });
        assert_eq!(request.args, expected);
        assert_eq!(request.env["HTTPS_PROXY"], "http://proxy.test");
        assert_eq!(request.timeout, Some(BOOTSTRAP_TIMEOUT));
        assert_eq!(request.io, crate::runtime::ProcessIo::CapturedTail);
    }
    assert_eq!(
        package_preview("arch", SysPackageProvider::Pacman, "fzf").unwrap(),
        "sudo pacman -S --needed --noconfirm fzf"
    );
    for os in ["ubuntu", "macos", "windows", "manjaro"] {
        assert!(package_preview(os, SysPackageProvider::Pacman, "fzf").is_err());
    }
}

// libalpm's download diagnostic includes libcurl's HTTP error text on the same line.
const PACMAN_DOWNLOAD_FAILURE: &str =
    "error: failed to commit transaction (failed to retrieve some files)";
const PACMAN_DOWNLOAD_404: &str = "error: failed retrieving file 'libluv-1.52.1-1-x86_64.pkg.tar.zst' from mirror.example : The requested URL returned error: 404";

#[tokio::test]
async fn pacman_failure_guidance_preserves_status_logs_and_never_retries() {
    for (provider, os) in [
        (SysPackageProvider::Pacman, "arch"),
        (SysPackageProvider::Apt, "ubuntu"),
    ] {
        for exit_code in [0, 1] {
            let stderr = format!("{PACMAN_DOWNLOAD_404}\n{PACMAN_DOWNLOAD_FAILURE}");
            let runtime = runtime(false);
            runtime
                .host()
                .queue_process_output(Ok(crate::runtime::ProcessOutput {
                    exit_code: Some(exit_code),
                    stderr: stderr.as_bytes().to_vec(),
                    ..Default::default()
                }));
            let result = runtime
                .run_package(os, provider, "neovim", false, "neovim", &mut NullObserver)
                .await
                .unwrap();
            assert_eq!(result.success, exit_code == 0);
            assert_eq!(result.logs, [PACMAN_DOWNLOAD_404, PACMAN_DOWNLOAD_FAILURE]);
            if provider == SysPackageProvider::Pacman && exit_code != 0 {
                assert!(
                    result
                        .failure_detail
                        .starts_with("Pacman install failed (exit 1)")
                );
                assert!(
                    result
                        .failure_detail
                        .contains("Package downloads returned HTTP 404.")
                );
                assert!(!result.failure_detail.contains("If downloads"));
                assert!(!result.failure_detail.contains("lock or signature"));
                assert!(
                    result
                        .failure_detail
                        .contains("may be outdated or a mirror out of sync")
                );
                assert!(result.failure_detail.contains("pacman -Syu"));
                assert!(
                    result
                        .failure_detail
                        .contains("do not use pacman -Sy alone")
                );
            } else {
                assert_eq!(
                    result.failure_detail,
                    if exit_code == 0 {
                        ""
                    } else {
                        "Apt install failed (exit 1)"
                    }
                );
            }
            let calls = runtime.host().process_requests();
            assert_eq!(calls.len(), 1);
            if provider == SysPackageProvider::Pacman {
                assert_eq!(calls[0].args, ["-S", "--needed", "--noconfirm", "neovim"]);
            } else {
                assert_eq!(calls[0].program, "apt-get");
                assert_eq!(calls[0].args, ["install", "-y", "neovim"]);
            }
        }
    }
}

#[tokio::test]
async fn pacman_download_hint_requires_an_exact_download_error_not_a_404_substring() {
    for stderr in [
        "error: HTTP 404",
        "错误：无法下载软件包，HTTP 404",
        "error: failed retrieving file 'tool.pkg.tar.zst.sig' from mirror.example : The requested URL returned error: 404",
        "error: failed retrieving file 'tool.pkg.tar.sig' from mirror.example : The requested URL returned error: 404",
        "error: database is locked",
        "错误：签名无效",
        "error: target not found: tool404",
        "error: failed retrieving file 'tool-404.pkg.tar.zst' from mirror404.example : Connection timed out",
        "error: failed retrieving file 'tool-404.pkg.tar.zst' from mirror404.example : The requested URL returned error: 503",
        "error: failed retrieving file 'tool.pkg.tar.zst' from mirror.example : The requested URL returned error: 4040",
        "error: failed retrieving file 'tool.pkg.tar.zst' from mirror.example : The requested URL returned error: 404.html",
        "error: failed retrieving file 'tool.pkg.tar.zst' from mirror.example : The requested URL returned error: 404x",
        "error: failed retrieving file 'tool.pkg.tar.zst' from mirror.example : HTTP 404",
        "error: failed retrieving file 'tool.pkg.tar.zst' from mirror.example : Connection timed out\nThe requested URL returned error: 404",
        "hook: downloading https://example.org/404 : The requested URL returned error: 404",
        "error: failed retrieving file 'core.db' from mirror.example : The requested URL returned error: 404",
        "error: failed retrieving file 'tool.pkg.tar.zst' from mirror.example : The requested URL returned error:",
    ] {
        let stderr = format!("{stderr}\n{PACMAN_DOWNLOAD_FAILURE}");
        let runtime = runtime(false);
        runtime
            .host()
            .queue_process_output(Ok(crate::runtime::ProcessOutput {
                exit_code: Some(1),
                stderr: stderr.as_bytes().to_vec(),
                ..Default::default()
            }));
        let result = runtime
            .run_package(
                "arch",
                SysPackageProvider::Pacman,
                "fzf",
                false,
                "fzf",
                &mut NullObserver,
            )
            .await
            .unwrap();
        assert!(!result.success);
        assert_eq!(
            result.failure_detail, "Pacman install failed (exit 1)",
            "{stderr}"
        );
        assert_eq!(result.logs, stderr.lines().collect::<Vec<_>>());
        assert_eq!(runtime.host().process_requests().len(), 1);
    }
}

#[tokio::test]
async fn pacman_download_hint_handles_curl_wording_and_preserves_truncated_logs() {
    for stderr in [
        PACMAN_DOWNLOAD_404.to_string(),
        PACMAN_DOWNLOAD_404.replace("returned error:", "returned an error:"),
        format!("{PACMAN_DOWNLOAD_404}\r\n{PACMAN_DOWNLOAD_404}\n"),
    ] {
        let stderr = format!("{stderr}\n{PACMAN_DOWNLOAD_FAILURE}");
        let runtime = runtime(false);
        runtime
            .host()
            .queue_process_output(Ok(crate::runtime::ProcessOutput {
                exit_code: Some(1),
                stderr: stderr.as_bytes().to_vec(),
                output_truncated: true,
                ..Default::default()
            }));
        let result = runtime
            .run_package(
                "arch",
                SysPackageProvider::Pacman,
                "fzf",
                false,
                "fzf",
                &mut NullObserver,
            )
            .await
            .unwrap();
        assert!(!result.success);
        assert_eq!(
            result
                .failure_detail
                .matches("Package downloads returned HTTP 404.")
                .count(),
            1
        );
        assert_eq!(
            result.logs[0],
            "installer output truncated at 64 KiB; showing final output"
        );
        assert_eq!(
            result.logs[1..],
            stderr
                .lines()
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
        );
    }
}

#[tokio::test]
async fn pacman_download_hint_does_not_join_streams_or_trust_stdout() {
    for (stdout, stderr) in [
        (PACMAN_DOWNLOAD_404, "error: invalid signature"),
        (
            "error: failed retrieving file 'tool.pkg.tar.zst' from mirror.example : ",
            "The requested URL returned error: 404",
        ),
    ] {
        let stderr = format!("{stderr}\n{PACMAN_DOWNLOAD_FAILURE}");
        let runtime = runtime(false);
        runtime
            .host()
            .queue_process_output(Ok(crate::runtime::ProcessOutput {
                exit_code: Some(1),
                stdout: stdout.as_bytes().to_vec(),
                stderr: stderr.as_bytes().to_vec(),
                ..Default::default()
            }));
        let result = runtime
            .run_package(
                "arch",
                SysPackageProvider::Pacman,
                "fzf",
                false,
                "fzf",
                &mut NullObserver,
            )
            .await
            .unwrap();
        assert!(!result.success);
        assert_eq!(result.failure_detail, "Pacman install failed (exit 1)");
        assert_eq!(
            result.logs,
            std::iter::once(stdout)
                .chain(stderr.lines())
                .collect::<Vec<_>>()
        );
        assert_eq!(runtime.host().process_requests().len(), 1);
    }
}

#[tokio::test]
async fn pacman_hint_accepts_native_color_prefix_without_changing_raw_logs() {
    let stderr = format!("{PACMAN_DOWNLOAD_404}\n{PACMAN_DOWNLOAD_FAILURE}")
        .lines()
        .map(|line| line.replacen("error: ", "\x1b[1;31merror: \x1b[0m", 1))
        .collect::<Vec<_>>()
        .join("\n");
    for exit_code in [0, 1] {
        let runtime = runtime(false);
        runtime
            .host()
            .queue_process_output(Ok(crate::runtime::ProcessOutput {
                exit_code: Some(exit_code),
                stderr: stderr.as_bytes().to_vec(),
                ..Default::default()
            }));
        let result = runtime
            .run_package(
                "arch",
                SysPackageProvider::Pacman,
                "fzf",
                false,
                "fzf",
                &mut NullObserver,
            )
            .await
            .unwrap();
        assert_eq!(result.success, exit_code == 0);
        assert_eq!(
            result
                .failure_detail
                .contains("Package downloads returned HTTP 404."),
            exit_code != 0
        );
        assert_eq!(result.logs, stderr.lines().collect::<Vec<_>>());
        assert_eq!(runtime.host().process_requests().len(), 1);
    }
}

#[tokio::test]
async fn pacman_hint_requires_download_transaction_failure_after_404() {
    for stderr in [
        PACMAN_DOWNLOAD_404.to_string(),
        format!(
            "{PACMAN_DOWNLOAD_404}\nerror: failed to commit transaction (invalid or corrupted package (PGP signature))"
        ),
        format!("{PACMAN_DOWNLOAD_404}\nerror: failed to commit transaction (conflicting files)"),
        format!(
            "{PACMAN_DOWNLOAD_404}\nerror: failed to commit transaction (not enough free disk space)"
        ),
        format!("{PACMAN_DOWNLOAD_404}\nerror: command failed to execute correctly"),
        format!("{PACMAN_DOWNLOAD_FAILURE}\n{PACMAN_DOWNLOAD_404}"),
        format!(
            "{PACMAN_DOWNLOAD_404}\n{PACMAN_DOWNLOAD_FAILURE}\nerror: failed to commit transaction (conflicting files)"
        ),
        format!(
            "{PACMAN_DOWNLOAD_404}\nwarning: failed to commit transaction (failed to retrieve some files)"
        ),
        // Never strip arbitrary terminal controls to manufacture a matching diagnostic.
        format!("\x1b[2K{PACMAN_DOWNLOAD_404}\n{PACMAN_DOWNLOAD_FAILURE}"),
        format!(
            "{}\n{PACMAN_DOWNLOAD_FAILURE}",
            PACMAN_DOWNLOAD_404.replace("404", "404\x1b[0m0")
        ),
    ] {
        let runtime = runtime(false);
        runtime
            .host()
            .queue_process_output(Ok(crate::runtime::ProcessOutput {
                exit_code: Some(1),
                stderr: stderr.as_bytes().to_vec(),
                ..Default::default()
            }));
        let result = runtime
            .run_package(
                "arch",
                SysPackageProvider::Pacman,
                "fzf",
                false,
                "fzf",
                &mut NullObserver,
            )
            .await
            .unwrap();
        assert_eq!(
            result.failure_detail, "Pacman install failed (exit 1)",
            "{stderr}"
        );
        assert!(!result.success);
        assert_eq!(result.logs, stderr.lines().collect::<Vec<_>>());
        assert_eq!(runtime.host().process_requests().len(), 1);
    }
}

#[tokio::test]
async fn pacman_signal_termination_keeps_original_failure_without_download_advice() {
    // SIGINT/SIGTERM set interrupted; other signals also have no exit code.
    for interrupted in [false, true] {
        let runtime = runtime(false);
        let stderr = format!("{PACMAN_DOWNLOAD_404}\n{PACMAN_DOWNLOAD_FAILURE}");
        runtime
            .host()
            .queue_process_output(Ok(crate::runtime::ProcessOutput {
                exit_code: None,
                interrupted,
                stderr: stderr.as_bytes().to_vec(),
                ..Default::default()
            }));
        let result = runtime
            .run_package(
                "arch",
                SysPackageProvider::Pacman,
                "fzf",
                false,
                "fzf",
                &mut NullObserver,
            )
            .await
            .unwrap();
        assert!(!result.success);
        assert_eq!(result.failure_detail, "Pacman install failed (exit signal)");
        assert_eq!(result.logs, stderr.lines().collect::<Vec<_>>());
        assert_eq!(runtime.host().process_requests().len(), 1);
    }
}

#[tokio::test]
async fn pacman_diagnostics_do_not_reclassify_timeouts_or_spawn_errors() {
    for kind in [
        std::io::ErrorKind::TimedOut,
        std::io::ErrorKind::PermissionDenied,
    ] {
        let runtime = runtime(false);
        runtime
            .host()
            .queue_process_output(Err(std::io::Error::new(kind, "HTTP 404").into()));
        let result = runtime
            .run_package(
                "arch",
                SysPackageProvider::Pacman,
                "fzf",
                false,
                "fzf",
                &mut NullObserver,
            )
            .await;
        if kind == std::io::ErrorKind::TimedOut {
            let result = result.unwrap();
            assert!(!result.success);
            assert_eq!(
                result.failure_detail,
                "Pacman install failed: timed out after 30 minutes"
            );
        } else {
            let error = result.err().expect("spawn error must propagate");
            assert!(format!("{error:#}").contains("HTTP 404"));
            assert!(!format!("{error:#}").contains("pacman -Syu"));
        }
        assert_eq!(runtime.host().process_requests().len(), 1);
    }
}
