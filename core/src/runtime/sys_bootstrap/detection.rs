//! Presence probes through captured context and Host observations/processes.
use super::*;

pub(super) enum DetectionResult {
    Present(String),
    Missing,
}

impl<H: FileSystemHost + ProcessHost> CoreRuntime<H> {
    pub(super) async fn detect_sys_item(
        &self,
        detection: &SysDetection,
    ) -> Result<DetectionResult> {
        match detection {
            SysDetection::Command {
                command,
                version_args,
            } => self.detect_command(command, version_args).await,
            SysDetection::Path { path } => self.detect_path(path).await,
            SysDetection::Any { probes } => {
                for probe in probes {
                    let result = match probe {
                        SysDetectionProbe::Command { command } => {
                            self.detect_command(command, &[]).await?
                        }
                        SysDetectionProbe::Path { path } => self.detect_path(path).await?,
                    };
                    if matches!(result, DetectionResult::Present(_)) {
                        return Ok(result);
                    }
                }
                Ok(DetectionResult::Missing)
            }
        }
    }

    async fn detect_path(&self, raw: &str) -> Result<DetectionResult> {
        let path = captured_expand(raw, &self.context().home_dir);
        match self.host().metadata(&path).await {
            Ok(_) => Ok(DetectionResult::Present(path.display().to_string())),
            Err(error) if error.is_not_found() => Ok(DetectionResult::Missing),
            Err(error) => Err(error.into_anyhow("inspecting Sys detection path")),
        }
    }

    async fn detect_command(
        &self,
        command: &str,
        version_args: &[String],
    ) -> Result<DetectionResult> {
        let Some(program) = self.find_command(command).await? else {
            return Ok(DetectionResult::Missing);
        };
        if version_args.is_empty() {
            return Ok(DetectionResult::Present(program.display().to_string()));
        }
        let output = self
            .host()
            .run(ProcessRequest {
                program: program.display().to_string(),
                args: version_args.to_vec(),
                timeout: Some(std::time::Duration::from_secs(3)),
                detach_terminal: true,
                stdout_limit: Some(MAX_LOG_BYTES),
                stderr_limit: Some(MAX_LOG_BYTES),
                ..ProcessRequest::default()
            })
            .await;
        let detail = output
            .ok()
            .filter(|output| output.exit_code == Some(0))
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .and_then(|stdout| stdout.lines().next().map(str::to_string))
            .filter(|line| !line.trim().is_empty())
            .unwrap_or_else(|| program.display().to_string());
        Ok(DetectionResult::Present(detail))
    }

    async fn find_command(&self, command: &str) -> Result<Option<PathBuf>> {
        for candidate in command_candidates(self.context(), command) {
            let observation = observe_command_candidate(self.host(), &candidate)
                .await
                .map_err(|error| error.into_anyhow("inspecting Sys detection command"))?;
            if observation.is_executable(self.context().platform) {
                return Ok(Some(candidate));
            }
        }
        Ok(None)
    }
}

fn captured_expand(raw: &str, home: &Path) -> PathBuf {
    if raw == "~" || raw == "$HOME" {
        home.to_path_buf()
    } else if let Some(rest) = raw
        .strip_prefix("~/")
        .or_else(|| raw.strip_prefix("$HOME/"))
    {
        home.join(rest)
    } else {
        PathBuf::from(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{
        InMemoryHost, PresetSnapshot, PresetSourceKind, ProcessOutput, RuntimeContext,
        RuntimePlatform,
    };

    #[cfg(unix)]
    #[tokio::test]
    async fn version_probe_is_bounded_and_detached_from_terminal() {
        use crate::runtime::RealHost;
        use std::os::unix::fs::PermissionsExt;
        use std::time::Duration;

        const FIXTURE: &str = "SHINE_VERSION_PROBE_PTY";
        if std::env::var_os(FIXTURE).is_some() {
            let original = unsafe { libc::tcgetpgrp(libc::STDIN_FILENO) };
            assert_eq!(original, unsafe { libc::getpgrp() });
            let tty = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/tty")
                .expect("PTY fixture must have an accessible controlling terminal");
            drop(tty);
            let root =
                std::env::temp_dir().join(format!("shine-version-probe-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            let binary = root.as_path().join("probe");
            let runtime = CoreRuntime::new(
                RealHost,
                RuntimeContext::isolated(
                    root.as_path().into(),
                    root.as_path().join("state"),
                    root.as_path().join("presets"),
                    root.as_path().join("bin"),
                    RuntimePlatform::Linux,
                ),
                PresetSnapshot::builder(PresetSourceKind::Embedded).build(),
            );
            // Like Yazi, touch /dev/tty even with redirected stdin/stdout.
            std::fs::write(
                &binary,
                r#"#!/usr/bin/env python3
import os, signal, termios
signal.signal(signal.SIGTTOU, signal.SIG_DFL)
try:
    fd = os.open('/dev/tty', os.O_RDWR)
except OSError:
    print('probe 1.0 detached')
else:
    termios.tcsetattr(fd, termios.TCSANOW, termios.tcgetattr(fd))
    print('unexpected controlling terminal')
"#,
            )
            .unwrap();
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
            let result = tokio::time::timeout(
                Duration::from_secs(8),
                runtime.detect_command(binary.to_str().unwrap(), &["--version".into()]),
            )
            .await
            .expect("version probe hung on terminal access")
            .unwrap();
            assert!(
                matches!(result, DetectionResult::Present(detail) if detail == "probe 1.0 detached")
            );
            assert_eq!(unsafe { libc::tcgetpgrp(libc::STDIN_FILENO) }, original);
            std::fs::remove_dir_all(&root).unwrap();
            return;
        }
        let output = tokio::task::spawn_blocking(|| {
            std::process::Command::new("python3").args(["-c", r#"
import os, pty, select, signal, sys, time
pid, fd = pty.fork()
if pid == 0:
    os.environ['SHINE_VERSION_PROBE_PTY'] = '1'
    os.execv(sys.argv[1], [sys.argv[1], '--exact', 'runtime::sys_bootstrap::detection::tests::version_probe_is_bounded_and_detached_from_terminal', '--nocapture'])
output = bytearray()
status = None
deadline = time.monotonic() + 12
while time.monotonic() < deadline:
    if select.select([fd], [], [], 0.05)[0]:
        try: chunk = os.read(fd, 4096)
        except OSError: break
        if not chunk: break
        output.extend(chunk)
    done, value = os.waitpid(pid, os.WNOHANG)
    if done:
        status = value
        break
if status is None:
    done, value = os.waitpid(pid, os.WNOHANG)
    if done: status = value
    else:
        os.kill(pid, signal.SIGKILL)
        _, status = os.waitpid(pid, 0)
os.close(fd)
sys.stdout.buffer.write(output)
sys.exit(os.waitstatus_to_exitcode(status))
"#]).arg(std::env::current_exe().unwrap()).output().unwrap()
        }).await.unwrap();
        assert!(
            output.status.success(),
            "PTY probe failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stalled_version_probe_falls_back_to_existing_executable() {
        use crate::runtime::RealHost;
        use std::os::unix::fs::PermissionsExt;
        use std::time::Duration;
        let root =
            std::env::temp_dir().join(format!("shine-version-probe-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let binary = root.as_path().join("probe");
        std::fs::write(&binary, "#!/bin/sh\nsleep 30\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        let runtime = CoreRuntime::new(
            RealHost,
            RuntimeContext::isolated(
                root.as_path().into(),
                root.as_path().join("state"),
                root.as_path().join("presets"),
                root.as_path().join("bin"),
                RuntimePlatform::Linux,
            ),
            PresetSnapshot::builder(PresetSourceKind::Embedded).build(),
        );
        let result = tokio::time::timeout(
            Duration::from_secs(8),
            runtime.detect_command(binary.to_str().unwrap(), &["--version".into()]),
        )
        .await
        .expect("version probe has no deadline")
        .unwrap();
        assert!(
            matches!(result, DetectionResult::Present(detail) if detail == binary.display().to_string())
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn command_detection_finds_new_installs_without_refreshing_captured_path() {
        for (platform, directory, command) in [
            (RuntimePlatform::Linux, ".atuin/bin", "atuin"),
            (RuntimePlatform::Linux, ".local/share/pnpm", "pnpm"),
            (RuntimePlatform::Linux, ".local/share/pnpm/bin", "pnpm"),
            (RuntimePlatform::Macos, "/opt/homebrew/bin", "atuin"),
            (RuntimePlatform::Macos, "/usr/local/bin", "atuin"),
            (
                RuntimePlatform::Windows,
                "AppData/Local/Microsoft/WinGet/Links",
                "atuin.exe",
            ),
        ] {
            let home = std::env::temp_dir().join("shine-command-detection");
            let context = RuntimeContext::isolated(
                home.clone(),
                home.join("state"),
                home.join("presets"),
                home.join("bin"),
                platform,
            );
            let host = InMemoryHost::new();
            let runtime = CoreRuntime::new(
                host,
                context,
                PresetSnapshot::builder(PresetSourceKind::Embedded).build(),
            );
            let detection = SysDetection::Command {
                command: command.trim_end_matches(".exe").into(),
                version_args: vec!["--version".into()],
            };
            assert!(matches!(
                runtime.detect_sys_item(&detection).await.unwrap(),
                DetectionResult::Missing
            ));
            let binary = if directory.starts_with('/') {
                PathBuf::from(directory)
            } else {
                home.join(directory)
            }
            .join(command);
            if platform != RuntimePlatform::Windows {
                runtime
                    .host()
                    .put_file_with_mode(&binary, b"not executable".to_vec(), 0o100644);
                assert!(matches!(
                    runtime.detect_sys_item(&detection).await.unwrap(),
                    DetectionResult::Missing
                ));
            }
            runtime
                .host()
                .put_file_with_mode(&binary, b"installed".to_vec(), 0o100755);
            runtime.host().queue_process_output(Ok(ProcessOutput {
                interrupted: false,
                output_truncated: false,
                exit_code: Some(0),
                stdout: b"command fixture\n".to_vec(),
                stderr: Vec::new(),
            }));
            assert!(
                matches!(runtime.detect_sys_item(&detection).await.unwrap(), DetectionResult::Present(detail) if detail == "command fixture")
            );
            let requests = runtime.host().process_requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(Path::new(&requests[0].program), binary.as_path());
            assert_eq!(requests[0].args, ["--version"]);
            assert!(runtime.context().path_env.is_none());
        }
    }
}
