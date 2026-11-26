use super::*;
use crate::runtime::{FileSystemObservationHost, HostOperation, InMemoryHost, ProcessOutput};

#[tokio::test]
async fn composed_profiles_migrate_without_legacy_source_templates() {
    for (os_id, ext, setting, user_setting) in [
        (
            "ubuntu",
            "sh",
            "export TOOL_ENABLED=1\n",
            "export USER_SETTING=1\n",
        ),
        (
            "macos",
            "sh",
            "export TOOL_ENABLED=1\n",
            "export USER_SETTING=1\n",
        ),
        (
            "windows",
            "ps1",
            "$env:TOOL_ENABLED = '1'\n",
            "$env:USER_SETTING = '1'\n",
        ),
    ] {
        for phase in SYS_PROFILE_PHASES {
            let base = format!("{setting}# user settings\n").into_bytes();
            let template = format!("# shine sys/tool\n{setting}# user settings\n").into_bytes();
            for with_base in [true, false] {
                let host = InMemoryHost::new();
                let profile_dir = Path::new("/profile");
                let phase_name = phase.as_str();
                let active = profile_dir.join(format!("{os_id}-sys.{phase_name}.{ext}"));
                let base_path = profile_dir.join(format!("{os_id}-sys.{phase_name}.base.{ext}"));
                let new_path = profile_dir.join(format!("{os_id}-sys.{phase_name}.new.{ext}"));
                let user = [base.as_slice(), user_setting.as_bytes()].concat();
                host.put_file(&active, user.clone());
                if with_base {
                    host.put_file(&base_path, base.to_vec());
                }
                // No legacy source template exists; migration uses the saved merge base.
                let report = install_sys_profile_phase_with_template(
                    &host,
                    ProfilePhaseInstall {
                        profile_dir,
                        os_id,
                        script_dir: Path::new("/missing-templates"),
                        phase,
                        ext,
                        force_profile: false,
                        template_override: Some(&template),
                    },
                )
                .await
                .unwrap();
                assert!(report.updated);
                assert_eq!(report.needs_action, !with_base);
                if with_base {
                    assert_eq!(
                        host.read(&active).await.unwrap(),
                        [template.as_slice(), user_setting.as_bytes()].concat()
                    );
                    assert_eq!(host.read(&base_path).await.unwrap(), template);
                } else {
                    assert_eq!(host.read(&active).await.unwrap(), user);
                    assert_eq!(host.read(&new_path).await.unwrap(), template);
                    assert!(host.read(&base_path).await.is_err());
                }
                assert!(
                    !host
                        .operations()
                        .iter()
                        .any(|op| matches!(op, HostOperation::Run { .. }))
                );
            }
        }
    }
}

const ACTIVE: &str = "/profile/linux-sys.pre.sh";
const BASE: &str = "/profile/linux-sys.pre.base.sh";
const NEW: &str = "/profile/linux-sys.pre.new.sh";
const MERGE: &str = "/profile/linux-sys.pre.merge.sh";

fn divergent_profile() -> InMemoryHost {
    let host = InMemoryHost::new();
    host.put_file("/templates/profile.pre.sh", b"new\n".to_vec());
    host.put_file(ACTIVE, b"user\n".to_vec());
    host.put_file(BASE, b"old\n".to_vec());
    host
}

async fn assert_file_contents(host: &InMemoryHost, expected: &[(&str, &[u8])]) {
    for &(path, bytes) in expected {
        assert_eq!(host.read(Path::new(path)).await.unwrap(), bytes, "{path}");
    }
}

fn written_paths(host: &InMemoryHost) -> Vec<PathBuf> {
    host.operations()
        .into_iter()
        .filter_map(|op| match op {
            HostOperation::Write(path) => Some(path),
            _ => None,
        })
        .collect()
}

async fn reconcile(host: &InMemoryHost) -> Result<SysProfileFileUpdate> {
    install_sys_profile_phase_with_template(
        host,
        ProfilePhaseInstall {
            profile_dir: Path::new("/profile"),
            os_id: "linux",
            script_dir: Path::new("/templates"),
            phase: SysProfilePhase::Pre,
            ext: "sh",
            force_profile: false,
            template_override: None,
        },
    )
    .await
}

#[tokio::test]
async fn profile_git_failure_falls_back_and_conflicts_preserve_active_and_base() {
    for missing_git in [false, true] {
        let host = divergent_profile();
        if missing_git {
            host.queue_process_output(Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "git absent",
            )
            .into()));
        } else {
            host.queue_process_output(Ok(ProcessOutput {
                exit_code: Some(2),
                ..Default::default()
            }));
        }
        let report = reconcile(&host).await.unwrap();
        assert!(report.updated && report.needs_action);
        assert_file_contents(
            &host,
            &[(ACTIVE, b"user\n"), (BASE, b"old\n"), (NEW, b"new\n")],
        )
        .await;
        let conflict = host.read(Path::new(MERGE)).await.unwrap();
        assert!(String::from_utf8(conflict).unwrap().contains("<<<<<<<"));
        assert_eq!(
            host.operations()
                .iter()
                .filter(|op| matches!(op, HostOperation::Run { program, .. } if program == "git"))
                .count(),
            1
        );
        assert_eq!(
            written_paths(&host),
            [
                // Empty preparation precedes permission setup and content publication.
                PathBuf::from(NEW),
                PathBuf::from(NEW),
                PathBuf::from(MERGE),
                PathBuf::from(MERGE)
            ]
        );
    }
}

#[tokio::test]
async fn fresh_profile_base_write_failure_stops_before_artifact_cleanup() {
    let host = InMemoryHost::new();
    host.put_file("/templates/profile.pre.sh", b"new\n".to_vec());
    host.put_file(NEW, b"pending\n".to_vec());
    host.put_file(MERGE, b"conflict\n".to_vec());
    host.fail_write_after(BASE, 0);
    let error = reconcile(&host).await.unwrap_err();
    assert_eq!(error.to_string(), "writing sys profile merge base");
    assert_file_contents(
        &host,
        &[
            (ACTIVE, b"new\n"),
            (NEW, b"pending\n"),
            (MERGE, b"conflict\n"),
        ],
    )
    .await;
    assert!(
        !host
            .operations()
            .iter()
            .any(|op| matches!(op, HostOperation::Remove(_) | HostOperation::Run { .. }))
    );
}

#[tokio::test]
async fn crlf_profile_reconciliation_uses_normalized_merge_without_git() {
    let host = InMemoryHost::new();
    host.put_file("/templates/profile.pre.sh", b"old\nkeep\nnew\n".to_vec());
    host.put_file(ACTIVE, b"user\r\nkeep\r\nend\r\n".to_vec());
    host.put_file(BASE, b"old\r\nkeep\r\nend\r\n".to_vec());
    let report = reconcile(&host).await.unwrap();
    assert!(report.updated && !report.needs_action);
    assert_file_contents(&host, &[(ACTIVE, b"user\nkeep\nnew\n")]).await;
    assert!(
        !host
            .operations()
            .iter()
            .any(|op| matches!(op, HostOperation::Run { .. }))
    );
}

#[tokio::test]
async fn profile_git_results_keep_clean_conflict_and_spawn_error_write_boundaries() {
    for code in [Some(0), Some(1), None] {
        let host = divergent_profile();
        host.queue_process_output(match code {
            Some(code) => Ok(ProcessOutput {
                exit_code: Some(code),
                stdout: b"git result\n".to_vec(),
                ..Default::default()
            }),
            None => {
                Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "git denied").into())
            }
        });
        let result = reconcile(&host).await;
        let writes = written_paths(&host);
        if let Some(code) = code {
            let report = result.unwrap();
            assert!(report.updated);
            assert_eq!(report.needs_action, code == 1);
            if code == 0 {
                assert_file_contents(&host, &[(ACTIVE, b"git result\n"), (BASE, b"new\n")]).await;
                assert_eq!(writes, [PathBuf::from(ACTIVE), PathBuf::from(BASE)]);
            } else {
                assert_file_contents(&host, &[(MERGE, b"git result\n"), (NEW, b"new\n")]).await;
                assert_eq!(
                    writes,
                    [
                        // Empty preparation precedes permission setup and content publication.
                        PathBuf::from(NEW),
                        PathBuf::from(NEW),
                        PathBuf::from(MERGE),
                        PathBuf::from(MERGE)
                    ]
                );
            }
        } else {
            assert_eq!(result.unwrap_err().to_string(), "running git merge-file");
            assert!(writes.is_empty());
        }
        if code != Some(0) {
            assert_file_contents(&host, &[(ACTIVE, b"user\n"), (BASE, b"old\n")]).await;
        }
        assert_eq!(
            host.operations()
                .iter()
                .filter(|op| matches!(op, HostOperation::Run { .. }))
                .count(),
            1
        );
    }
}
