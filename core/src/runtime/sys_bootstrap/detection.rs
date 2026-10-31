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

    #[tokio::test]
    async fn atuin_detection_finds_new_installs_without_refreshing_captured_path() {
        for (platform, directory, command) in [
            (RuntimePlatform::Linux, ".atuin/bin", "atuin"),
            (RuntimePlatform::Macos, "/opt/homebrew/bin", "atuin"),
            (RuntimePlatform::Macos, "/usr/local/bin", "atuin"),
            (
                RuntimePlatform::Windows,
                "AppData/Local/Microsoft/WinGet/Links",
                "atuin.exe",
            ),
        ] {
            let home = std::env::temp_dir().join("shine-atuin-detection");
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
                command: "atuin".into(),
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
                exit_code: Some(0),
                stdout: b"atuin fixture\n".to_vec(),
                stderr: Vec::new(),
            }));
            assert!(
                matches!(runtime.detect_sys_item(&detection).await.unwrap(), DetectionResult::Present(detail) if detail == "atuin fixture")
            );
            let requests = runtime.host().process_requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(Path::new(&requests[0].program), binary.as_path());
            assert_eq!(requests[0].args, ["--version"]);
            assert!(runtime.context().path_env.is_none());
        }
    }
}
