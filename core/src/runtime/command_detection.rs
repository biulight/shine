use super::{
    FileKind, FileMetadata, FileSystemObservationHost, HostError, RuntimeContext, RuntimePlatform,
};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CommandCandidateObservation {
    pub source: Option<FileMetadata>,
    pub resolved: Option<ResolvedCommandCandidate>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedCommandCandidate {
    pub path: PathBuf,
    pub metadata: FileMetadata,
}

impl CommandCandidateObservation {
    pub(crate) fn is_executable(&self, platform: RuntimePlatform) -> bool {
        self.resolved.as_ref().is_some_and(|resolved| {
            resolved.metadata.kind == FileKind::File
                && (platform == RuntimePlatform::Windows
                    || resolved
                        .metadata
                        .unix_mode
                        .is_none_or(|mode| mode & 0o111 != 0))
        })
    }
}

pub(crate) async fn observe_command_candidate(
    host: &impl FileSystemObservationHost,
    path: &Path,
) -> Result<CommandCandidateObservation, HostError> {
    let source = match host.metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.is_not_found() => {
            return Ok(CommandCandidateObservation {
                source: None,
                resolved: None,
            });
        }
        Err(error) => return Err(error),
    };
    let resolved_path = if source.kind == FileKind::Symlink {
        match host.canonicalize(path).await {
            Ok(path) => path,
            Err(error) if error.is_not_found() => {
                return Ok(CommandCandidateObservation {
                    source: Some(source),
                    resolved: None,
                });
            }
            Err(error) => return Err(error),
        }
    } else {
        path.to_path_buf()
    };
    let resolved_metadata = match host.metadata(&resolved_path).await {
        Ok(metadata) => metadata,
        Err(error) if error.is_not_found() => {
            return Ok(CommandCandidateObservation {
                source: Some(source),
                resolved: None,
            });
        }
        Err(error) => return Err(error),
    };
    Ok(CommandCandidateObservation {
        source: Some(source),
        resolved: Some(ResolvedCommandCandidate {
            path: resolved_path,
            metadata: resolved_metadata,
        }),
    })
}

pub(crate) fn command_candidates(context: &RuntimeContext, command: &str) -> Vec<PathBuf> {
    let mut directories = context
        .path_env
        .as_deref()
        .map(std::env::split_paths)
        .map(Iterator::collect::<Vec<_>>)
        .unwrap_or_default();
    directories.extend([
        context.home_dir.join(".local/bin"),
        context.home_dir.join(".cargo/bin"),
        context.home_dir.join(".bun/bin"),
        context.home_dir.join(".local/share/pnpm"),
        context
            .home_dir
            .join("AppData/Local/Microsoft/WinGet/Links"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/home/linuxbrew/.linuxbrew/bin"),
    ]);
    directories
        .into_iter()
        .flat_map(|directory| {
            if context.platform == RuntimePlatform::Windows {
                vec![
                    directory.join(command),
                    directory.join(format!("{command}.exe")),
                    directory.join(format!("{command}.cmd")),
                    directory.join(format!("{command}.bat")),
                    directory.join(format!("{command}.ps1")),
                ]
            } else {
                vec![directory.join(command)]
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{FileSystemHost, InMemoryHost};

    #[tokio::test]
    async fn executable_symlink_resolves_to_its_target() {
        let host = InMemoryHost::new();
        host.put_file_with_mode("/usr/bin/batcat", b"binary".to_vec(), 0o100755);
        host.symlink(
            Path::new("/usr/bin/batcat"),
            Path::new("/home/test/.local/bin/bat"),
        )
        .await
        .unwrap();

        let observation = observe_command_candidate(&host, Path::new("/home/test/.local/bin/bat"))
            .await
            .unwrap();

        assert_eq!(
            observation.source.as_ref().map(|metadata| metadata.kind),
            Some(FileKind::Symlink)
        );
        assert_eq!(
            observation
                .resolved
                .as_ref()
                .map(|resolved| resolved.path.as_path()),
            Some(Path::new("/usr/bin/batcat"))
        );
        assert!(observation.is_executable(RuntimePlatform::Linux));
    }

    #[tokio::test]
    async fn broken_and_non_executable_symlinks_are_not_commands() {
        let host = InMemoryHost::new();
        host.symlink(Path::new("/missing"), Path::new("/bin/broken"))
            .await
            .unwrap();
        host.put_file_with_mode("/bin/plain-target", b"plain".to_vec(), 0o100644);
        host.symlink(Path::new("/bin/plain-target"), Path::new("/bin/plain"))
            .await
            .unwrap();

        let broken = observe_command_candidate(&host, Path::new("/bin/broken"))
            .await
            .unwrap();
        let plain = observe_command_candidate(&host, Path::new("/bin/plain"))
            .await
            .unwrap();

        assert!(broken.resolved.is_none());
        assert!(!broken.is_executable(RuntimePlatform::Linux));
        assert!(!plain.is_executable(RuntimePlatform::Linux));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn real_host_accepts_a_symlink_to_an_executable_file() {
        use crate::runtime::RealHost;
        use std::os::unix::fs::PermissionsExt;

        let root =
            std::env::temp_dir().join(format!("shine-command-detection-{}", uuid::Uuid::new_v4()));
        let target = root.join("batcat");
        let command = root.join("bat");
        tokio::fs::create_dir_all(&root).await.unwrap();
        tokio::fs::write(&target, b"binary").await.unwrap();
        tokio::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))
            .await
            .unwrap();
        tokio::fs::symlink(&target, &command).await.unwrap();

        let observation = observe_command_candidate(&RealHost, &command)
            .await
            .unwrap();

        assert_eq!(
            observation.source.as_ref().map(|metadata| metadata.kind),
            Some(FileKind::Symlink)
        );
        assert!(observation.is_executable(RuntimePlatform::Linux));
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}
