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
    ) -> Result<ExecutionResult> {
        let ProviderInvocation {
            program, mut args, ..
        } = package_command(os_id, provider, package, &self.context().proxy_env)?;
        let mut env = self.context().proxy_env.clone();
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
            .host()
            .run(ProcessRequest {
                program,
                args,
                env: std::mem::take(&mut env),
                inherit_stdin: true,
                timeout: Some(BOOTSTRAP_TIMEOUT),
                stdout_limit: Some(MAX_LOG_BYTES),
                stderr_limit: Some(MAX_LOG_BYTES),
                ..ProcessRequest::default()
            })
            .await;
        execution_result(
            output,
            format!("{provider:?} install failed"),
            provider == SysPackageProvider::Winget,
        )
    }

    pub(super) async fn run_sys_script(
        &self,
        os_id: &str,
        script: &Path,
        root: &Path,
        shell: &str,
        item: &SysItem,
        admin: bool,
    ) -> Result<ExecutionResult> {
        let (program, mut args) = match os_id {
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
            ("SHINE_SYS_SHELL".into(), shell.to_string()),
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
            .host()
            .run(ProcessRequest {
                program,
                args,
                cwd: script.parent().map(Path::to_path_buf),
                env,
                inherit_stdin: true,
                timeout: Some(BOOTSTRAP_TIMEOUT),
                stdout_limit: Some(MAX_LOG_BYTES),
                stderr_limit: Some(MAX_LOG_BYTES),
                ..ProcessRequest::default()
            })
            .await;
        execution_result(
            output,
            format!("sys item `{}` install script failed", item.id),
            item.requires_admin && os_id == "windows",
        )
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
                logs: bounded_logs(&output.stdout, &output.stderr),
            })
        }
    }
}
fn bounded_logs(stdout: &[u8], stderr: &[u8]) -> Vec<String> {
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
    if truncated {
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
            .run_package("ubuntu", SysPackageProvider::Apt, "example", false)
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
                .run_package("ubuntu", SysPackageProvider::Apt, "example", false)
                .await
                .err()
                .expect("non-timeout errors must propagate");
            assert_eq!(error.to_string(), "running sys bootstrap installer");
            assert!(format!("{error:#}").contains("timed out"));
        }
    }

    #[test]
    fn installer_exit_failures_keep_logs_and_windows_admin_guidance() {
        let result = execution_result(
            Ok(crate::runtime::ProcessOutput {
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
