use super::*;
use crate::runtime::{FileSystemObservationHost, HostOperation, InMemoryHost, ProcessOutput};

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
        let host = InMemoryHost::new();
        host.put_file("/templates/profile.pre.sh", b"new\n".to_vec());
        host.put_file("/profile/linux-sys.pre.sh", b"user\n".to_vec());
        host.put_file("/profile/linux-sys.pre.base.sh", b"old\n".to_vec());
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
        assert_eq!(
            host.read(Path::new("/profile/linux-sys.pre.sh"))
                .await
                .unwrap(),
            b"user\n"
        );
        assert_eq!(
            host.read(Path::new("/profile/linux-sys.pre.base.sh"))
                .await
                .unwrap(),
            b"old\n"
        );
        assert_eq!(
            host.read(Path::new("/profile/linux-sys.pre.new.sh"))
                .await
                .unwrap(),
            b"new\n"
        );
        let conflict = host
            .read(Path::new("/profile/linux-sys.pre.merge.sh"))
            .await
            .unwrap();
        assert!(String::from_utf8(conflict).unwrap().contains("<<<<<<<"));
        assert_eq!(
            host.operations()
                .iter()
                .filter(|op| matches!(op, HostOperation::Run { program, .. } if program == "git"))
                .count(),
            1
        );
        assert_eq!(
            host.operations()
                .into_iter()
                .filter_map(|op| match op {
                    HostOperation::Write(path) => Some(path),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            [
                PathBuf::from("/profile/linux-sys.pre.new.sh"),
                PathBuf::from("/profile/linux-sys.pre.merge.sh")
            ]
        );
    }
}

#[tokio::test]
async fn fresh_profile_base_write_failure_stops_before_artifact_cleanup() {
    let host = InMemoryHost::new();
    host.put_file("/templates/profile.pre.sh", b"new\n".to_vec());
    host.put_file("/profile/linux-sys.pre.new.sh", b"pending\n".to_vec());
    host.put_file("/profile/linux-sys.pre.merge.sh", b"conflict\n".to_vec());
    host.fail_write_after("/profile/linux-sys.pre.base.sh", 0);
    let error = reconcile(&host).await.unwrap_err();
    assert_eq!(error.to_string(), "writing sys profile merge base");
    assert_eq!(
        host.read(Path::new("/profile/linux-sys.pre.sh"))
            .await
            .unwrap(),
        b"new\n"
    );
    assert_eq!(
        host.read(Path::new("/profile/linux-sys.pre.new.sh"))
            .await
            .unwrap(),
        b"pending\n"
    );
    assert_eq!(
        host.read(Path::new("/profile/linux-sys.pre.merge.sh"))
            .await
            .unwrap(),
        b"conflict\n"
    );
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
    host.put_file(
        "/profile/linux-sys.pre.sh",
        b"user\r\nkeep\r\nend\r\n".to_vec(),
    );
    host.put_file(
        "/profile/linux-sys.pre.base.sh",
        b"old\r\nkeep\r\nend\r\n".to_vec(),
    );
    let report = reconcile(&host).await.unwrap();
    assert!(report.updated && !report.needs_action);
    assert_eq!(
        host.read(Path::new("/profile/linux-sys.pre.sh"))
            .await
            .unwrap(),
        b"user\nkeep\nnew\n"
    );
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
        let host = InMemoryHost::new();
        host.put_file("/templates/profile.pre.sh", b"new\n".to_vec());
        host.put_file("/profile/linux-sys.pre.sh", b"user\n".to_vec());
        host.put_file("/profile/linux-sys.pre.base.sh", b"old\n".to_vec());
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
        let writes = host
            .operations()
            .into_iter()
            .filter_map(|op| match op {
                HostOperation::Write(path) => Some(path),
                _ => None,
            })
            .collect::<Vec<_>>();
        if let Some(code) = code {
            let report = result.unwrap();
            assert!(report.updated);
            assert_eq!(report.needs_action, code == 1);
            if code == 0 {
                assert_eq!(
                    host.read(Path::new("/profile/linux-sys.pre.sh"))
                        .await
                        .unwrap(),
                    b"git result\n"
                );
                assert_eq!(
                    host.read(Path::new("/profile/linux-sys.pre.base.sh"))
                        .await
                        .unwrap(),
                    b"new\n"
                );
                assert_eq!(
                    writes,
                    [
                        PathBuf::from("/profile/linux-sys.pre.sh"),
                        PathBuf::from("/profile/linux-sys.pre.base.sh")
                    ]
                );
            } else {
                assert_eq!(
                    host.read(Path::new("/profile/linux-sys.pre.merge.sh"))
                        .await
                        .unwrap(),
                    b"git result\n"
                );
                assert_eq!(
                    host.read(Path::new("/profile/linux-sys.pre.new.sh"))
                        .await
                        .unwrap(),
                    b"new\n"
                );
                assert_eq!(
                    writes,
                    [
                        PathBuf::from("/profile/linux-sys.pre.new.sh"),
                        PathBuf::from("/profile/linux-sys.pre.merge.sh")
                    ]
                );
            }
        } else {
            assert_eq!(result.unwrap_err().to_string(), "running git merge-file");
            assert!(writes.is_empty());
        }
        if code != Some(0) {
            assert_eq!(
                host.read(Path::new("/profile/linux-sys.pre.sh"))
                    .await
                    .unwrap(),
                b"user\n"
            );
            assert_eq!(
                host.read(Path::new("/profile/linux-sys.pre.base.sh"))
                    .await
                    .unwrap(),
                b"old\n"
            );
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
