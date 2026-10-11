//! Targeted cleanup for services registered by older Shine binaries.
//! This entry point deliberately does not load or initialize Shine configuration.

use anyhow::{Context, Result, bail, ensure};
use shine_core::runtime::{
    FileKind, FileSystemHost, ProcessHost, ProcessOutput, ProcessRequest, RealHost,
};
use std::path::{Path, PathBuf};
use std::time::Duration;

const LAUNCHD_LABEL: &str = "top.biulight.shine.http";
const SYSTEMD_UNIT: &str = "shine-http.service";
const WINDOWS_TASK: &str = "Shine HTTP Server";

/// A cheap observation only; links also need user attention, but are never followed.
pub(crate) async fn has_registration(
    host: &impl FileSystemHost,
    platform: &str,
    home: &Path,
    xdg: Option<&Path>,
) -> Result<bool> {
    let path = match platform {
        "macos" => home
            .join("Library/LaunchAgents")
            .join(format!("{LAUNCHD_LABEL}.plist")),
        "linux" => xdg
            .filter(|path| path.is_absolute())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| home.join(".config"))
            .join("systemd/user")
            .join(SYSTEMD_UNIT),
        _ => return Ok(false),
    };
    match host.metadata(&path).await {
        Ok(_) => Ok(true),
        Err(error) if error.is_not_found() => Ok(false),
        Err(error) => Err(error.into_anyhow("observing legacy service registration")),
    }
}

/// Read-only native inspection. Never install, start, stop, or remove a service here.
pub(crate) async fn has_native_service(host: &impl ProcessHost, platform: &str) -> Result<bool> {
    let (program, args) = match platform {
        "macos" => ("launchctl", vec!["list".into()]),
        "linux" => (
            "systemctl",
            vec![
                "--user".into(),
                "show".into(),
                SYSTEMD_UNIT.into(),
                "--property=LoadState,ActiveState,UnitFileState".into(),
            ],
        ),
        "windows" => (
            "powershell.exe",
            vec![
                "-NoLogo".into(),
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                format!(
                    "$ErrorActionPreference = 'Stop'; $task = Get-ScheduledTask -TaskPath '\\' | Where-Object {{ $_.TaskName -ceq '{WINDOWS_TASK}' }}; if ($null -eq $task) {{ 'absent' }} else {{ 'present' }}"
                ),
            ],
        ),
        _ => return Ok(false),
    };
    let output = host
        .run(ProcessRequest {
            program: program.into(),
            args,
            timeout: Some(Duration::from_secs(10)),
            stdout_limit: Some(1024 * 1024),
            stderr_limit: Some(64 * 1024),
            ..ProcessRequest::default()
        })
        .await?;
    if platform != "linux" {
        ensure!(
            output.exit_code == Some(0),
            "native service inspection failed"
        );
    }
    let exit_code = output.exit_code;
    let output = String::from_utf8_lossy(&output.stdout);
    match platform {
        "macos" => Ok(output
            .lines()
            .any(|line| line.split_whitespace().last() == Some(LAUNCHD_LABEL))),
        "linux" => {
            let properties = output
                .lines()
                .filter_map(|line| line.split_once('='))
                .collect::<std::collections::BTreeMap<_, _>>();
            let load = properties
                .get("LoadState")
                .context("missing service load state")?;
            let active = properties
                .get("ActiveState")
                .context("missing service active state")?;
            ensure!(
                exit_code == Some(0)
                    || (*load == "not-found" && matches!(*active, "inactive" | "failed")),
                "native service inspection failed"
            );
            Ok(*load != "not-found" || !matches!(*active, "inactive" | "failed"))
        }
        "windows" => match output.trim() {
            "present" => Ok(true),
            "absent" => Ok(false),
            _ => bail!("unexpected native service inspection response"),
        },
        _ => Ok(false),
    }
}

pub async fn handle_uninstall() -> Result<()> {
    let home = crate::home::effective_home_dir();
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let removed = uninstall(&RealHost, std::env::consts::OS, &home, xdg.as_deref()).await?;
    crate::maintenance::invalidate_serve().await;
    println!(
        "{}",
        if removed {
            "Uninstalled legacy Shine HTTP service"
        } else {
            "Not installed"
        }
    );
    Ok(())
}

async fn uninstall(
    host: &(impl FileSystemHost + ProcessHost),
    platform: &str,
    home: &Path,
    xdg: Option<&Path>,
) -> Result<bool> {
    match platform {
        "macos" => {
            let path = home
                .join("Library/LaunchAgents")
                .join(format!("{LAUNCHD_LABEL}.plist"));
            let exists = registration_exists(host, &path).await?;
            let listed = run(host, "launchctl", &["list".into()]).await?;
            let loaded = String::from_utf8_lossy(&listed.stdout)
                .lines()
                .any(|line| line.split_whitespace().last() == Some(LAUNCHD_LABEL));
            if loaded {
                if exists {
                    run(
                        host,
                        "launchctl",
                        &["unload".into(), "-w".into(), path_arg(&path)?],
                    )
                    .await?;
                } else {
                    run(host, "launchctl", &["remove".into(), LAUNCHD_LABEL.into()]).await?;
                }
            }
            if exists {
                remove_registration(host, &path).await?;
            }
            Ok(exists || loaded)
        }
        "linux" => {
            let config_home = xdg
                .filter(|path| path.is_absolute())
                .map(Path::to_path_buf)
                .unwrap_or_else(|| home.join(".config"));
            let path = config_home.join("systemd/user").join(SYSTEMD_UNIT);
            if !registration_exists(host, &path).await? {
                if !has_native_service(host, platform).await? {
                    return Ok(false);
                }
                run(
                    host,
                    "systemctl",
                    &["--user".into(), "stop".into(), SYSTEMD_UNIT.into()],
                )
                .await?;
                run(
                    host,
                    "systemctl",
                    &["--user".into(), "daemon-reload".into()],
                )
                .await?;
                return Ok(true);
            }
            run(
                host,
                "systemctl",
                &[
                    "--user".into(),
                    "disable".into(),
                    "--now".into(),
                    SYSTEMD_UNIT.into(),
                ],
            )
            .await?;
            remove_registration(host, &path).await?;
            run(
                host,
                "systemctl",
                &["--user".into(), "daemon-reload".into()],
            )
            .await?;
            Ok(true)
        }
        "windows" => {
            // Enumerate first: Get-ScheduledTask errors remain failures, while an absent task
            // is an explicit no-op independent of localized schtasks error messages.
            let script = format!(
                "$ErrorActionPreference = 'Stop'; $task = Get-ScheduledTask -TaskPath '\\' | Where-Object {{ $_.TaskName -ceq '{WINDOWS_TASK}' }}; if ($null -eq $task) {{ 'absent' }} else {{ $task | Stop-ScheduledTask; $task | Unregister-ScheduledTask -Confirm:$false; 'removed' }}"
            );
            let output = run(
                host,
                "powershell.exe",
                &[
                    "-NoLogo".into(),
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-Command".into(),
                    script,
                ],
            )
            .await?;
            match String::from_utf8_lossy(&output.stdout).trim() {
                "absent" => Ok(false),
                "removed" => Ok(true),
                _ => bail!("unexpected legacy scheduled-task cleanup response"),
            }
        }
        os => bail!("shine clear serve is not supported on {os}"),
    }
}

async fn registration_exists(host: &impl FileSystemHost, path: &Path) -> Result<bool> {
    match host.metadata(path).await {
        Ok(metadata) => {
            ensure!(
                metadata.kind == FileKind::File,
                "legacy service registration is not a regular file: {}",
                path.display()
            );
            Ok(true)
        }
        Err(error) if error.is_not_found() => Ok(false),
        Err(error) => Err(error.into_anyhow("reading legacy service registration")),
    }
}

fn path_arg(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_string)
        .context("legacy service path is not valid UTF-8")
}

async fn remove_registration(host: &impl FileSystemHost, path: &Path) -> Result<()> {
    host.remove_file(path)
        .await
        .map_err(|error| error.into_anyhow("removing legacy service registration"))
}

async fn run(host: &impl ProcessHost, program: &str, args: &[String]) -> Result<ProcessOutput> {
    let output = host
        .run(ProcessRequest {
            program: program.into(),
            args: args.to_vec(),
            timeout: Some(Duration::from_secs(30)),
            ..ProcessRequest::default()
        })
        .await
        .with_context(|| format!("running {program} for legacy service cleanup"))?;
    ensure!(
        output.exit_code == Some(0),
        "{program} failed during legacy service cleanup: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shine_core::runtime::{FileSystemObservationHost, HostOperation, InMemoryHost};

    fn queue(host: &InMemoryHost, code: i32, stdout: &str) {
        host.queue_process_output(Ok(ProcessOutput {
            exit_code: Some(code),
            stdout: stdout.as_bytes().to_vec(),
            stderr: b"service-manager diagnostic".to_vec(),
        }));
    }

    fn runs(host: &InMemoryHost) -> Vec<(String, Vec<String>)> {
        host.operations()
            .into_iter()
            .filter_map(|operation| {
                if let HostOperation::Run { program, args } = operation {
                    Some((program, args))
                } else {
                    None
                }
            })
            .collect()
    }

    #[tokio::test]
    async fn native_inspection_distinguishes_absence_failures_and_loaded_orphans() {
        for (platform, code, output, expected) in [
            (
                "macos",
                0,
                "- 0 top.biulight.shine.http.other\n",
                Some(false),
            ),
            ("macos", 0, "- 0 top.biulight.shine.http\n", Some(true)),
            ("macos", 1, "", None),
            (
                "linux",
                0,
                "LoadState=loaded\nActiveState=active\n",
                Some(true),
            ),
            (
                "linux",
                0,
                "LoadState=not-found\nActiveState=active\n",
                Some(true),
            ),
            (
                "linux",
                4,
                "LoadState=not-found\nActiveState=inactive\n",
                Some(false),
            ),
            ("linux", 1, "", None),
            ("windows", 0, "present\r\n", Some(true)),
            ("windows", 0, "absent\r\n", Some(false)),
            ("windows", 1, "absent\r\n", None),
        ] {
            let host = InMemoryHost::new();
            queue(&host, code, output);
            let result = has_native_service(&host, platform).await;
            match expected {
                Some(expected) => assert_eq!(result.unwrap(), expected),
                None => assert!(result.is_err()),
            }
            for (_, args) in runs(&host) {
                let args = args.join(" ");
                for mutation in [
                    "unload",
                    "remove",
                    "disable",
                    "--now",
                    "Stop-ScheduledTask",
                    "Unregister-ScheduledTask",
                ] {
                    assert!(!args.contains(mutation), "{args}");
                }
            }
        }
    }

    #[tokio::test]
    async fn linux_cleanup_stops_loaded_service_even_when_unit_file_is_missing() {
        let host = InMemoryHost::new();
        queue(&host, 0, "LoadState=not-found\nActiveState=active\n");
        queue(&host, 0, "");
        queue(&host, 0, "");
        assert!(
            uninstall(&host, "linux", Path::new("unused"), None)
                .await
                .unwrap()
        );
        assert_eq!(runs(&host)[1].1, ["--user", "stop", SYSTEMD_UNIT]);
    }

    #[tokio::test]
    async fn macos_cleanup_removes_loaded_registration_and_is_repeatable() {
        let host = InMemoryHost::new();
        let home = std::env::temp_dir().join("shine-legacy-home");
        let path = home.join("Library/LaunchAgents/top.biulight.shine.http.plist");
        let retained = home.join(".shine/http/app/demo/keep.txt");
        host.put_file(&path, b"legacy registration".to_vec());
        host.put_file(&retained, b"user resource".to_vec());
        queue(&host, 0, "-\t0\ttop.biulight.shine.http\n");
        queue(&host, 0, "");
        assert!(uninstall(&host, "macos", &home, None).await.unwrap());
        assert!(host.metadata(&path).await.unwrap_err().is_not_found());
        assert_eq!(host.read(&retained).await.unwrap(), b"user resource");
        assert_eq!(runs(&host)[1].1, ["unload", "-w", path.to_str().unwrap()]);
        queue(&host, 0, "-\t0\ttop.biulight.shine.http.other\n");
        assert!(!uninstall(&host, "macos", &home, None).await.unwrap());
        assert_eq!(runs(&host).len(), 3);
    }

    #[tokio::test]
    async fn macos_cleanup_handles_loaded_service_without_plist() {
        let host = InMemoryHost::new();
        queue(&host, 0, "-\t0\ttop.biulight.shine.http\n");
        queue(&host, 0, "");
        assert!(
            uninstall(&host, "macos", Path::new("unused"), None)
                .await
                .unwrap()
        );
        assert_eq!(runs(&host)[1].1, ["remove", LAUNCHD_LABEL]);
    }

    #[tokio::test]
    async fn macos_stop_failure_keeps_registration_for_retry() {
        let host = InMemoryHost::new();
        let home = std::env::temp_dir().join("shine-legacy-home");
        let path = home.join("Library/LaunchAgents/top.biulight.shine.http.plist");
        host.put_file(&path, b"legacy registration".to_vec());
        queue(&host, 0, "-\t0\ttop.biulight.shine.http\n");
        queue(&host, 1, "");
        assert!(uninstall(&host, "macos", &home, None).await.is_err());
        assert_eq!(host.read(&path).await.unwrap(), b"legacy registration");
    }

    #[tokio::test]
    async fn cleanup_rejects_linked_registration_before_running_service_manager() {
        let host = InMemoryHost::new();
        let home = std::env::temp_dir().join("shine-legacy-home");
        let target = home.join("unrelated.conf");
        let path = home.join("Library/LaunchAgents/top.biulight.shine.http.plist");
        host.put_file(&target, b"user data".to_vec());
        host.symlink(&target, &path).await.unwrap();
        assert!(uninstall(&host, "macos", &home, None).await.is_err());
        assert_eq!(host.read(&target).await.unwrap(), b"user data");
        assert_eq!(host.metadata(&path).await.unwrap().kind, FileKind::Symlink);
        assert!(runs(&host).is_empty());
    }

    #[tokio::test]
    async fn linux_cleanup_uses_xdg_and_preserves_unrelated_files() {
        let host = InMemoryHost::new();
        let home = std::env::temp_dir().join("shine-legacy-home");
        let xdg = home.join("custom config");
        let path = xdg.join("systemd/user/shine-http.service");
        let other = xdg.join("systemd/user/other.service");
        host.put_file(&path, b"legacy registration".to_vec());
        host.put_file(&other, b"other registration".to_vec());
        queue(&host, 0, "");
        queue(&host, 0, "");
        assert!(uninstall(&host, "linux", &home, Some(&xdg)).await.unwrap());
        assert!(host.metadata(&path).await.unwrap_err().is_not_found());
        assert_eq!(host.read(&other).await.unwrap(), b"other registration");
        assert_eq!(
            runs(&host),
            [
                (
                    "systemctl".into(),
                    vec![
                        "--user".into(),
                        "disable".into(),
                        "--now".into(),
                        SYSTEMD_UNIT.into()
                    ]
                ),
                (
                    "systemctl".into(),
                    vec!["--user".into(), "daemon-reload".into()]
                ),
            ]
        );
        queue(&host, 0, "LoadState=not-found\nActiveState=inactive\n");
        assert!(!uninstall(&host, "linux", &home, Some(&xdg)).await.unwrap());
        assert_eq!(runs(&host).len(), 3);
    }

    #[tokio::test]
    async fn linux_stop_failure_keeps_registration_for_retry() {
        let host = InMemoryHost::new();
        let home = std::env::temp_dir().join("shine-legacy-home");
        let path = home.join(".config/systemd/user/shine-http.service");
        host.put_file(&path, b"legacy registration".to_vec());
        queue(&host, 1, "");
        assert!(
            uninstall(&host, "linux", &home, Some(Path::new("relative")))
                .await
                .is_err()
        );
        assert_eq!(host.read(&path).await.unwrap(), b"legacy registration");
        assert_eq!(runs(&host).len(), 1);
    }

    #[tokio::test]
    async fn windows_cleanup_distinguishes_absence_from_failure() {
        let host = InMemoryHost::new();
        for (code, output, expected) in [
            (0, "removed\r\n", Some(true)),
            (0, "absent\r\n", Some(false)),
            (1, "", None),
        ] {
            queue(&host, code, output);
            let result = uninstall(&host, "windows", Path::new("unused"), None).await;
            match expected {
                Some(value) => assert_eq!(result.unwrap(), value),
                None => assert!(result.is_err()),
            }
        }
        for (program, args) in runs(&host) {
            assert_eq!(program, "powershell.exe");
            let script = args.last().unwrap();
            assert!(script.contains("-TaskPath '\\'"));
            assert!(script.contains("-ceq 'Shine HTTP Server'"));
            assert!(script.contains("Stop-ScheduledTask"));
            assert!(script.contains("Unregister-ScheduledTask -Confirm:$false"));
        }
    }
}
