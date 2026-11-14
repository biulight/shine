//! Bounded installer execution and typed failure classification.
use super::provider::{ProviderInvocation, package_command};
use super::*;

pub(super) struct ExecutionResult {
    pub(super) success: bool,
    pub(super) failure_detail: String,
    pub(super) logs: Vec<String>,
}

impl<H: FileSystemHost + ProcessHost> CoreRuntime<H> {
    pub(super) async fn run_package(
        &self,
        os_id: &str,
        provider: SysPackageProvider,
        package: &str,
        admin: bool,
        item_id: &str,
        observer: &mut impl RuntimeObserver,
    ) -> Result<ExecutionResult> {
        let ProviderInvocation {
            program, mut args, ..
        } = package_command(os_id, provider, package, &self.context().proxy_env)?;
        let mut env = self.context().proxy_env.clone();
        self.add_installer_display_env(&mut env);
        let program = if admin && !self.context().running_as_admin {
            let mut sudo_args = vec!["-n".to_string()];
            if !env.is_empty() {
                sudo_args.push("env".to_string());
                sudo_args.extend(env.iter().map(|(key, value)| format!("{key}={value}")));
            }
            sudo_args.push(program);
            sudo_args.append(&mut args);
            args = sudo_args;
            "sudo".to_string()
        } else {
            program
        };
        let output = self
            .run_installer(
                ProcessRequest {
                    program,
                    args,
                    env: std::mem::take(&mut env),
                    inherit_stdin: true,
                    io: super::super::ProcessIo::CapturedTail,
                    timeout: Some(BOOTSTRAP_TIMEOUT),
                    stdout_limit: Some(MAX_LOG_BYTES),
                    stderr_limit: Some(MAX_LOG_BYTES),
                    ..ProcessRequest::default()
                },
                item_id,
                observer,
            )
            .await;
        let pacman_failed = provider == SysPackageProvider::Pacman
            && output
                .as_ref()
                .is_ok_and(|output| output.exit_code.is_some_and(|code| code != 0));
        let mut result = execution_result(
            output,
            format!("{provider:?} install failed"),
            provider == SysPackageProvider::Winget,
        )?;
        if pacman_failed {
            // Conditional advice, not classification of localized package-manager output.
            // Successful fallback, signals, spawn errors and typed timeouts retain their semantics.
            result.failure_detail.push_str(concat!(
                "; If downloads return HTTP 404, the package database may be outdated or a mirror out of sync: ",
                "review and complete pacman -Syu (use sudo if needed), then retry bootstrap; do not use pacman -Sy alone. ",
                "For lock or signature errors, follow pacman's specific diagnostics."
            ));
        }
        Ok(result)
    }

    pub(super) async fn run_sys_script(
        &self,
        request: &SysBootstrapRequest,
        script: &Path,
        root: &Path,
        item: &SysItem,
        admin: bool,
        observer: &mut impl RuntimeObserver,
    ) -> Result<ExecutionResult> {
        let (program, mut args) = match request.os_id.as_str() {
            "windows" => (
                "powershell.exe".to_string(),
                vec![
                    "-NoProfile".into(),
                    "-ExecutionPolicy".into(),
                    "Bypass".into(),
                    "-File".into(),
                ],
            ),
            "macos" => ("zsh".to_string(), Vec::new()),
            _ => ("bash".to_string(), Vec::new()),
        };
        args.push(script.display().to_string());
        let mut env = BTreeMap::from([
            ("SHINE_SYS_PRESET_ROOT".into(), root.display().to_string()),
            ("SHINE_SYS_SHELL".into(), request.sys_shell.clone()),
            (
                "SHINE_TARGET_HOME".into(),
                self.context().home_dir.display().to_string(),
            ),
        ]);
        for key in &item.required_env {
            if let Some(value) = self.context().env.get(key) {
                env.insert(key.clone(), value.clone());
            }
        }
        env.extend(self.context().proxy_env.clone());
        self.add_installer_display_env(&mut env);
        let (program, args) = if admin && !self.context().running_as_admin {
            let mut sudo = vec![
                "-n".to_string(),
                format!(
                    "--preserve-env={}",
                    env.keys().cloned().collect::<Vec<_>>().join(",")
                ),
                program,
            ];
            sudo.extend(args);
            ("sudo".to_string(), sudo)
        } else {
            (program, args)
        };
        let output = self
            .run_installer(
                ProcessRequest {
                    program,
                    args,
                    cwd: script.parent().map(Path::to_path_buf),
                    env,
                    inherit_stdin: true,
                    io: super::super::ProcessIo::CapturedTail,
                    timeout: Some(BOOTSTRAP_TIMEOUT),
                    stdout_limit: Some(MAX_LOG_BYTES),
                    stderr_limit: Some(MAX_LOG_BYTES),
                    ..ProcessRequest::default()
                },
                &item.id,
                observer,
            )
            .await;
        execution_result(
            output,
            format!("sys item `{}` install script failed", item.id),
            item.requires_admin && request.os_id == "windows",
        )
    }

    fn add_installer_display_env(&self, env: &mut BTreeMap<String, String>) {
        if self.installer_color && !self.context().env.contains_key("CARGO_TERM_COLOR") {
            env.entry("CARGO_TERM_COLOR".into())
                .or_insert_with(|| "always".into());
        }
        if self.installer_progress.0 && !self.context().env.contains_key("CARGO_TERM_PROGRESS_WHEN")
        {
            env.entry("CARGO_TERM_PROGRESS_WHEN".into())
                .or_insert_with(|| "always".into());
        }
        if let Some(width) = self.installer_progress.1
            && !self.context().env.contains_key("CARGO_TERM_PROGRESS_WIDTH")
        {
            env.entry("CARGO_TERM_PROGRESS_WIDTH".into())
                .or_insert_with(|| width.to_string());
        }
    }

    async fn run_installer(
        &self,
        request: ProcessRequest,
        item_id: &str,
        observer: &mut impl RuntimeObserver,
    ) -> Result<crate::runtime::ProcessOutput> {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
        let mut run = self.host().run_with_output(request, sender);
        let mut stdout = InstallerLines::default();
        let mut stderr = InstallerLines::default();
        let mut emit = |stream, text, code| {
            observer.emit(RuntimeEvent::ProcessOutput {
                code,
                target: item_id.to_string(),
                stream,
                text,
            })
        };
        let mut flush = tokio::time::interval(Duration::from_millis(100));
        flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let result = loop {
            tokio::select! {
                _ = flush.tick() => {
                    stdout.flush_partial("stdout", &mut emit);
                    stderr.flush_partial("stderr", &mut emit);
                    emit("stdout", String::new(), "sys_bootstrap_installer_tick");
                }
                result = &mut run => break result,
                Some(chunk) = receiver.recv() => {
                    let lines = if chunk.stream == "stdout" { &mut stdout } else { &mut stderr };
                    lines.push(&chunk.bytes, chunk.stream, &mut emit);
                }
            }
        };
        // The process can finish with chunks still in the bounded channel.
        while let Ok(chunk) = receiver.try_recv() {
            let lines = if chunk.stream == "stdout" {
                &mut stdout
            } else {
                &mut stderr
            };
            lines.push(&chunk.bytes, chunk.stream, &mut emit);
        }
        stdout.finish("stdout", &mut emit);
        stderr.finish("stderr", &mut emit);
        result
    }
}

/// Preserve line endings while bounding incomplete UTF-8 fragments.
#[derive(Default)]
struct InstallerLines {
    pending: Vec<u8>,
    carriage_return: bool,
}

impl InstallerLines {
    fn push(
        &mut self,
        bytes: &[u8],
        stream: &'static str,
        emit: &mut impl FnMut(&'static str, String, &'static str),
    ) {
        for &byte in bytes {
            if self.carriage_return {
                self.carriage_return = false;
                if byte == b'\n' {
                    self.emit_pending(stream, "sys_bootstrap_installer", emit);
                    continue;
                }
                self.emit_pending(stream, "sys_bootstrap_installer_progress", emit);
            }
            match byte {
                b'\r' => self.carriage_return = true,
                b'\n' => self.emit_pending(stream, "sys_bootstrap_installer", emit),
                _ => {
                    self.pending.push(byte);
                    if self.pending.len() >= 8192 {
                        self.flush_partial(stream, emit);
                    }
                }
            }
        }
    }

    fn flush_partial(
        &mut self,
        stream: &'static str,
        emit: &mut impl FnMut(&'static str, String, &'static str),
    ) {
        if self.carriage_return {
            self.carriage_return = false;
            self.emit_pending(stream, "sys_bootstrap_installer_progress", emit);
            return;
        }
        let end = match std::str::from_utf8(&self.pending) {
            Err(error) if error.error_len().is_none() => error.valid_up_to(),
            _ => self.pending.len(),
        };
        if end > 0 {
            let text = String::from_utf8_lossy(&self.pending[..end]).into_owned();
            self.pending.drain(..end);
            emit(stream, text, "sys_bootstrap_installer_partial");
        }
    }

    fn emit_pending(
        &mut self,
        stream: &'static str,
        code: &'static str,
        emit: &mut impl FnMut(&'static str, String, &'static str),
    ) {
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        emit(stream, text, code);
    }

    fn finish(
        &mut self,
        stream: &'static str,
        emit: &mut impl FnMut(&'static str, String, &'static str),
    ) {
        // EOF commits even a row whose bytes were already emitted by a timed flush.
        self.carriage_return = false;
        self.emit_pending(stream, "sys_bootstrap_installer", emit);
    }
}

fn execution_result(
    output: Result<crate::runtime::ProcessOutput>,
    prefix: String,
    windows_admin_hint: bool,
) -> Result<ExecutionResult> {
    match output {
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::TimedOut) =>
        {
            Ok(ExecutionResult {
                success: false,
                failure_detail: format!(
                    "{prefix}: timed out after {} minutes",
                    BOOTSTRAP_TIMEOUT.as_secs() / 60
                ),
                logs: Vec::new(),
            })
        }
        Err(error) => Err(error).context("running sys bootstrap installer"),
        Ok(output) => {
            let success = output.exit_code == Some(0);
            let mut detail = if success {
                String::new()
            } else {
                format!(
                    "{prefix} (exit {})",
                    output
                        .exit_code
                        .map_or_else(|| "signal".to_string(), |code| code.to_string())
                )
            };
            if !success && windows_admin_hint {
                detail.push_str("; retry from an elevated PowerShell");
            }
            Ok(ExecutionResult {
                success,
                failure_detail: detail,
                logs: bounded_logs(&output.stdout, &output.stderr, output.output_truncated),
            })
        }
    }
}
fn bounded_logs(stdout: &[u8], stderr: &[u8], output_truncated: bool) -> Vec<String> {
    let mut bytes = stdout.to_vec();
    if !stdout.is_empty() && !stderr.is_empty() {
        bytes.push(b'\n');
    }
    bytes.extend_from_slice(stderr);
    let truncated = bytes.len() > MAX_LOG_BYTES;
    if truncated {
        bytes = bytes[bytes.len() - MAX_LOG_BYTES..].to_vec();
    }
    let mut lines = String::from_utf8_lossy(&bytes)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if truncated || output_truncated {
        lines.insert(
            0,
            "installer output truncated at 64 KiB; showing final output".into(),
        );
    }
    lines
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{
        InMemoryHost, PresetSnapshot, PresetSourceKind, RuntimeContext, RuntimePlatform,
    };

    fn runtime() -> CoreRuntime<InMemoryHost> {
        let root = PathBuf::from("/shine-sys-bootstrap-test");
        CoreRuntime::new(
            InMemoryHost::new(),
            RuntimeContext::isolated(
                root.join("home"),
                root.join("state"),
                root.join("presets"),
                root.join("bin"),
                RuntimePlatform::Linux,
            ),
            PresetSnapshot::builder(PresetSourceKind::Embedded).build(),
        )
    }

    #[tokio::test]
    async fn color_fallback_is_opt_in_and_preserved_through_sudo() {
        let runtime = runtime().with_installer_color(true);
        runtime
            .run_package(
                "ubuntu",
                SysPackageProvider::Apt,
                "example",
                true,
                "example",
                &mut crate::runtime::NullObserver,
            )
            .await
            .unwrap();
        let requests = runtime.host().process_requests();
        assert_eq!(
            requests[0].env.get("CARGO_TERM_COLOR").map(String::as_str),
            Some("always")
        );
        assert!(
            requests[0]
                .args
                .windows(2)
                .any(|args| args == ["env", "CARGO_TERM_COLOR=always"])
        );
        let mut env = BTreeMap::from([("CARGO_TERM_COLOR".into(), "never".into())]);
        runtime.add_installer_display_env(&mut env);
        assert_eq!(env["CARGO_TERM_COLOR"], "never");
        let runtime = runtime.with_installer_color(false);
        let mut env = BTreeMap::new();
        runtime.add_installer_display_env(&mut env);
        assert!(env.is_empty());
    }

    #[test]
    fn installer_line_events_distinguish_crlf_progress_partial_and_eof() {
        let mut lines = InstallerLines::default();
        let mut events = Vec::new();
        let mut emit = |_stream, text, code| events.push((code, text));
        lines.push(b"log\r", "stdout", &mut emit);
        lines.push(b"\n10%\r20%", "stdout", &mut emit);
        lines.flush_partial("stdout", &mut emit);
        lines.push(b"\r\n", "stdout", &mut emit);
        lines.finish("stdout", &mut emit);
        assert_eq!(
            events,
            [
                ("sys_bootstrap_installer", "log".into()),
                ("sys_bootstrap_installer_progress", "10%".into()),
                ("sys_bootstrap_installer_partial", "20%".into()),
                ("sys_bootstrap_installer", String::new()),
                ("sys_bootstrap_installer", String::new()),
            ]
        );
    }

    #[test]
    fn progress_env_fallbacks_preserve_explicit_values() {
        let runtime = runtime().with_installer_progress(true, Some(64));
        let mut env = BTreeMap::new();
        runtime.add_installer_display_env(&mut env);
        assert_eq!(env["CARGO_TERM_PROGRESS_WHEN"], "always");
        assert_eq!(env["CARGO_TERM_PROGRESS_WIDTH"], "64");
        env.insert("CARGO_TERM_PROGRESS_WHEN".into(), "never".into());
        env.insert("CARGO_TERM_PROGRESS_WIDTH".into(), "40".into());
        runtime.add_installer_display_env(&mut env);
        assert_eq!(env["CARGO_TERM_PROGRESS_WHEN"], "never");
        assert_eq!(env["CARGO_TERM_PROGRESS_WIDTH"], "40");
    }

    #[test]
    fn installer_lines_handle_utf8_progress_partial_prompts_and_bounded_fragments() {
        let mut lines = InstallerLines::default();
        let mut output = Vec::new();
        let mut emit = |stream, text, _code| output.push((stream, text));
        lines.push(b"one\r\ntwo\r", "stdout", &mut emit);
        let chinese = "中文".as_bytes();
        lines.push(&chinese[..2], "stdout", &mut emit);
        lines.flush_partial("stdout", &mut emit);
        lines.push(&chinese[2..], "stdout", &mut emit);
        lines.push(b"\nPassword: ", "stdout", &mut emit);
        lines.flush_partial("stdout", &mut emit);
        lines.push(&vec![b'x'; 100_000], "stdout", &mut emit);
        assert!(lines.pending.len() < 8192);
        lines.finish("stdout", &mut emit);
        assert_eq!(
            &output[..4],
            &[
                ("stdout", "one".into()),
                ("stdout", "two".into()),
                ("stdout", "中文".into()),
                ("stdout", "Password: ".into()),
            ]
        );
        assert_eq!(
            output[4..]
                .iter()
                .map(|(_, text)| text.len())
                .sum::<usize>(),
            100_000
        );
        assert!(output.iter().all(|(_, text)| text.len() <= 8192));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn installer_streams_both_pipes_and_unterminated_prompt_before_exit() {
        use crate::runtime::RealHost;
        struct Observer {
            marker: PathBuf,
            events: Vec<(String, String)>,
        }
        impl RuntimeObserver for Observer {
            fn emit(&mut self, event: RuntimeEvent) {
                if let RuntimeEvent::ProcessOutput {
                    code,
                    target,
                    stream,
                    text,
                } = event
                {
                    if code == "sys_bootstrap_installer_tick" {
                        return;
                    }
                    assert_eq!(target, "fixture");
                    if text == "Continue? " {
                        std::fs::write(&self.marker, b"continue").unwrap();
                    }
                    self.events.push((stream.to_string(), text));
                }
            }
        }
        let temp =
            std::env::temp_dir().join(format!("shine-live-installer-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();
        let runtime = CoreRuntime::new(
            RealHost,
            RuntimeContext::isolated(
                temp.join("home"),
                temp.join("state"),
                temp.join("presets"),
                temp.join("bin"),
                RuntimePlatform::Linux,
            ),
            PresetSnapshot::builder(PresetSourceKind::Embedded).build(),
        );
        let mut observer = Observer {
            marker: temp.join("continue"),
            events: Vec::new(),
        };
        let result = runtime.run_installer(ProcessRequest {
            program: "bash".into(),
            args: vec!["-c".into(), "printf 'first\n'; printf 'warning\n' >&2; printf 'Continue? '; while [ ! -f continue ]; do sleep 0.02; done; printf 'done\n'; exit 7".into()],
            cwd: Some(temp.clone()), io: super::super::super::ProcessIo::CapturedTail,
            timeout: Some(Duration::from_secs(5)), stdout_limit: Some(64), stderr_limit: Some(64),
            ..ProcessRequest::default()
        }, "fixture", &mut observer).await.unwrap();
        assert_eq!(result.exit_code, Some(7));
        assert!(observer.events.contains(&("stdout".into(), "first".into())));
        assert!(
            observer
                .events
                .contains(&("stderr".into(), "warning".into()))
        );
        assert!(observer.events.contains(&("stdout".into(), "done".into())));
        assert!(result.stdout.ends_with(b"done\n"));
        assert_eq!(result.stderr, b"warning\n");
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[tokio::test]
    async fn installer_timeout_uses_error_kind_through_context_and_arbitrary_wording() {
        let runtime = runtime();
        runtime
            .host()
            .queue_process_output(Err(anyhow::Error::new(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "deadline reached",
            ))
            .context("waiting for installer")));
        let report = runtime
            .run_package(
                "ubuntu",
                SysPackageProvider::Apt,
                "example",
                false,
                "example",
                &mut crate::runtime::NullObserver,
            )
            .await
            .unwrap();
        assert!(!report.success);
        assert_eq!(
            report.failure_detail,
            "Apt install failed: timed out after 30 minutes"
        );
        assert!(report.logs.is_empty());
    }

    #[tokio::test]
    async fn installer_timeout_words_do_not_reclassify_other_failures() {
        for error in [
            anyhow::Error::new(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "timed out",
            )),
            anyhow::anyhow!("timed out"),
        ] {
            let runtime = runtime();
            runtime.host().queue_process_output(Err(error));
            let error = runtime
                .run_package(
                    "ubuntu",
                    SysPackageProvider::Apt,
                    "example",
                    false,
                    "example",
                    &mut crate::runtime::NullObserver,
                )
                .await
                .err()
                .expect("non-timeout errors must propagate");
            assert_eq!(error.to_string(), "running sys bootstrap installer");
            assert!(format!("{error:#}").contains("timed out"));
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn verbose_installer_finishes_and_keeps_failure_tail() {
        use crate::runtime::RealHost;
        let temp =
            std::env::temp_dir().join(format!("shine-verbose-installer-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();
        let root = temp.as_path();
        let runtime = CoreRuntime::new(
            RealHost,
            RuntimeContext::isolated(
                root.join("home"),
                root.join("state"),
                root.join("presets"),
                root.join("bin"),
                RuntimePlatform::Linux,
            ),
            PresetSnapshot::builder(PresetSourceKind::Embedded).build(),
        );
        let manifest: SysManifest = toml::from_str(
            "version = 2\n[[items]]\nid = 'fixture'\nlabel = 'Fixture'\ndetect = { kind = 'path', path = '~/.fixture' }\ninstall = { kind = 'script', path = 'install.sh' }\n"
        ).unwrap();
        let script = root.join("install.sh");
        for (stream, redirect) in [("stdout", ""), ("stderr", " >&2")] {
            for exit_code in [0, 7] {
                let marker = root.join("completed");
                std::fs::write(&script, format!(
                    "#!/bin/bash\nhead -c 200000 /dev/zero{redirect}\nprintf '\nfinal diagnostic\n'{redirect}\nprintf done > completed\nexit {exit_code}\n"
                )).unwrap();
                let result = runtime
                    .run_sys_script(
                        &SysBootstrapRequest {
                            os_id: "ubuntu".into(),
                            item_id: "fixture".into(),
                            sys_shell: "bash".into(),
                        },
                        &script,
                        root,
                        &manifest.items[0],
                        false,
                        &mut crate::runtime::NullObserver,
                    )
                    .await
                    .unwrap();
                assert_eq!(std::fs::read(&marker).unwrap(), b"done", "{stream}");
                std::fs::remove_file(marker).unwrap();
                assert_eq!(result.success, exit_code == 0);
                assert_eq!(
                    result.logs.first().unwrap(),
                    "installer output truncated at 64 KiB; showing final output"
                );
                assert_eq!(result.logs.last().unwrap(), "final diagnostic");
                if exit_code != 0 {
                    assert!(result.failure_detail.contains("exit 7"));
                }
            }
        }
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn installer_exit_failures_keep_logs_and_windows_admin_guidance() {
        let result = execution_result(
            Ok(crate::runtime::ProcessOutput {
                interrupted: false,
                output_truncated: false,
                exit_code: Some(5),
                stdout: b"installer output".to_vec(),
                stderr: b"failure detail".to_vec(),
            }),
            "Winget install failed".into(),
            true,
        )
        .unwrap();
        assert!(!result.success);
        assert_eq!(
            result.failure_detail,
            "Winget install failed (exit 5); retry from an elevated PowerShell"
        );
        assert_eq!(result.logs, ["installer output", "failure detail"]);
    }
}
