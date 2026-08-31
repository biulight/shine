use super::host::{
    FileKind, FileMetadata, FileSystemHost, FileSystemObservationHost, HostError, HostOperation,
    PrivilegedFileSystemHost, ProcessHost, ProcessOutput, ProcessRequest, SplitDnsHost,
    SplitDnsObservationHost, SplitDnsRequest, SplitDnsState,
};
use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Eq, PartialEq)]
enum MemoryNode {
    Directory,
    File(Vec<u8>),
    Symlink(PathBuf),
}

#[derive(Default)]
struct MemoryState {
    nodes: BTreeMap<PathBuf, MemoryNode>,
    modes: BTreeMap<PathBuf, u32>,
    operations: Vec<HostOperation>,
    process_outputs: VecDeque<anyhow::Result<ProcessOutput>>,
    #[cfg(test)]
    write_failures: BTreeMap<PathBuf, usize>,
    #[cfg(test)]
    rename_failures: BTreeMap<(PathBuf, PathBuf), usize>,
}

#[derive(Clone, Default)]
pub struct InMemoryHost {
    state: Arc<Mutex<MemoryState>>,
}

impl InMemoryHost {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put_file(&self, path: impl Into<PathBuf>, bytes: Vec<u8>) {
        let path = path.into();
        let mut state = self.state.lock().expect("in-memory host lock");
        insert_parent_dirs(&mut state.nodes, &path);
        state.nodes.insert(path.clone(), MemoryNode::File(bytes));
        state.modes.insert(path, 0o100644);
    }

    pub fn queue_process_output(&self, output: anyhow::Result<ProcessOutput>) {
        self.state
            .lock()
            .expect("in-memory host lock")
            .process_outputs
            .push_back(output);
    }

    pub fn operations(&self) -> Vec<HostOperation> {
        self.state
            .lock()
            .expect("in-memory host lock")
            .operations
            .clone()
    }

    #[cfg(test)]
    pub fn fail_write_after(&self, path: impl Into<PathBuf>, successful_writes: usize) {
        self.state
            .lock()
            .expect("in-memory host lock")
            .write_failures
            .insert(path.into(), successful_writes);
    }

    #[cfg(test)]
    pub fn fail_rename_after(
        &self,
        from: impl Into<PathBuf>,
        to: impl Into<PathBuf>,
        successful_renames: usize,
    ) {
        self.state
            .lock()
            .expect("in-memory host lock")
            .rename_failures
            .insert((from.into(), to.into()), successful_renames);
    }
}

fn not_found(path: &Path) -> HostError {
    HostError::new(
        std::io::ErrorKind::NotFound,
        anyhow::anyhow!("in-memory path not found: {}", path.display()),
    )
}

fn insert_parent_dirs(nodes: &mut BTreeMap<PathBuf, MemoryNode>, path: &Path) {
    for parent in path.ancestors().skip(1) {
        if parent.as_os_str().is_empty() {
            continue;
        }
        nodes
            .entry(parent.to_path_buf())
            .or_insert(MemoryNode::Directory);
    }
}

impl FileSystemObservationHost for InMemoryHost {
    fn canonicalize<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<PathBuf, HostError>> + Send + 'a>> {
        Box::pin(async move {
            let state = self.state.lock().expect("in-memory host lock");
            if state.nodes.contains_key(path) {
                Ok(path.to_path_buf())
            } else {
                Err(not_found(path))
            }
        })
    }

    fn read<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, HostError>> + Send + 'a>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("in-memory host lock");
            state
                .operations
                .push(HostOperation::Read(path.to_path_buf()));
            match state.nodes.get(path) {
                Some(MemoryNode::File(bytes)) => Ok(bytes.clone()),
                Some(MemoryNode::Symlink(target)) => match state.nodes.get(target) {
                    Some(MemoryNode::File(bytes)) => Ok(bytes.clone()),
                    _ => Err(not_found(target)),
                },
                _ => Err(not_found(path)),
            }
        })
    }

    fn metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<FileMetadata, HostError>> + Send + 'a>> {
        Box::pin(async move {
            let state = self.state.lock().expect("in-memory host lock");
            match state.nodes.get(path) {
                Some(MemoryNode::Directory) => Ok(FileMetadata {
                    kind: FileKind::Directory,
                    len: 0,
                    unix_mode: None,
                }),
                Some(MemoryNode::File(bytes)) => Ok(FileMetadata {
                    kind: FileKind::File,
                    len: bytes.len() as u64,
                    unix_mode: state.modes.get(path).copied(),
                }),
                Some(MemoryNode::Symlink(target)) => Ok(FileMetadata {
                    kind: FileKind::Symlink,
                    len: target.as_os_str().len() as u64,
                    unix_mode: None,
                }),
                None => Err(not_found(path)),
            }
        })
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<PathBuf>, HostError>> + Send + 'a>> {
        Box::pin(async move {
            let state = self.state.lock().expect("in-memory host lock");
            if !matches!(state.nodes.get(path), Some(MemoryNode::Directory)) {
                return Err(not_found(path));
            }
            let mut entries = state
                .nodes
                .keys()
                .filter(|candidate| candidate.parent() == Some(path))
                .cloned()
                .collect::<Vec<_>>();
            entries.sort();
            Ok(entries)
        })
    }

    fn read_link<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<PathBuf, HostError>> + Send + 'a>> {
        Box::pin(async move {
            let state = self.state.lock().expect("in-memory host lock");
            match state.nodes.get(path) {
                Some(MemoryNode::Symlink(target)) => Ok(target.clone()),
                _ => Err(not_found(path)),
            }
        })
    }
}

impl FileSystemHost for InMemoryHost {
    fn write_atomic<'a>(
        &'a self,
        path: &'a Path,
        bytes: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("in-memory host lock");
            #[cfg(test)]
            if let Some(remaining) = state.write_failures.get_mut(path) {
                if *remaining == 0 {
                    state.write_failures.remove(path);
                    return Err(HostError::new(
                        std::io::ErrorKind::Other,
                        anyhow::anyhow!("injected in-memory write failure: {}", path.display()),
                    ));
                }
                *remaining -= 1;
            }
            insert_parent_dirs(&mut state.nodes, path);
            state
                .nodes
                .insert(path.to_path_buf(), MemoryNode::File(bytes.to_vec()));
            state.modes.insert(path.to_path_buf(), 0o100644);
            state
                .operations
                .push(HostOperation::Write(path.to_path_buf()));
            Ok(())
        })
    }

    fn write<'a>(
        &'a self,
        path: &'a Path,
        bytes: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        self.write_atomic(path, bytes)
    }

    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("in-memory host lock");
            insert_parent_dirs(&mut state.nodes, path);
            state
                .nodes
                .insert(path.to_path_buf(), MemoryNode::Directory);
            state
                .operations
                .push(HostOperation::CreateDirectory(path.to_path_buf()));
            Ok(())
        })
    }

    fn remove_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("in-memory host lock");
            if state.nodes.remove(path).is_none() {
                return Err(not_found(path));
            }
            state.modes.remove(path);
            state
                .operations
                .push(HostOperation::Remove(path.to_path_buf()));
            Ok(())
        })
    }

    fn remove_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("in-memory host lock");
            if !state.nodes.contains_key(path) {
                return Err(not_found(path));
            }
            state
                .nodes
                .retain(|candidate, _| candidate != path && !candidate.starts_with(path));
            state
                .modes
                .retain(|candidate, _| candidate != path && !candidate.starts_with(path));
            state
                .operations
                .push(HostOperation::RemoveDirectory(path.to_path_buf()));
            Ok(())
        })
    }

    fn set_executable<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("in-memory host lock");
            if !matches!(state.nodes.get(path), Some(MemoryNode::File(_))) {
                return Err(not_found(path));
            }
            state
                .operations
                .push(HostOperation::SetExecutable(path.to_path_buf()));
            let mode = state.modes.entry(path.to_path_buf()).or_insert(0o100644);
            *mode |= 0o111;
            Ok(())
        })
    }

    fn set_mode<'a>(
        &'a self,
        path: &'a Path,
        mode: u32,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("in-memory host lock");
            if !matches!(state.nodes.get(path), Some(MemoryNode::File(_))) {
                return Err(not_found(path));
            }
            state.modes.insert(path.to_path_buf(), mode);
            state.operations.push(HostOperation::SetMode {
                path: path.to_path_buf(),
                mode,
            });
            Ok(())
        })
    }

    fn rename<'a>(
        &'a self,
        from: &'a Path,
        to: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("in-memory host lock");
            #[cfg(test)]
            if let Some(remaining) = state
                .rename_failures
                .get_mut(&(from.to_path_buf(), to.to_path_buf()))
            {
                if *remaining == 0 {
                    state
                        .rename_failures
                        .remove(&(from.to_path_buf(), to.to_path_buf()));
                    return Err(HostError::new(
                        std::io::ErrorKind::Other,
                        anyhow::anyhow!(
                            "injected in-memory rename failure: {} -> {}",
                            from.display(),
                            to.display()
                        ),
                    ));
                }
                *remaining -= 1;
            }
            let Some(node) = state.nodes.remove(from) else {
                return Err(not_found(from));
            };
            let mode = state.modes.remove(from);
            let descendants = if matches!(node, MemoryNode::Directory) {
                state
                    .nodes
                    .keys()
                    .filter(|path| path.starts_with(from))
                    .cloned()
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            let moved_descendants = descendants
                .into_iter()
                .filter_map(|path| {
                    let relative = path.strip_prefix(from).ok()?.to_path_buf();
                    let node = state.nodes.remove(&path)?;
                    let mode = state.modes.remove(&path);
                    Some((to.join(relative), node, mode))
                })
                .collect::<Vec<_>>();
            insert_parent_dirs(&mut state.nodes, to);
            state.nodes.insert(to.to_path_buf(), node);
            if let Some(mode) = mode {
                state.modes.insert(to.to_path_buf(), mode);
            }
            for (path, node, mode) in moved_descendants {
                state.nodes.insert(path.clone(), node);
                if let Some(mode) = mode {
                    state.modes.insert(path, mode);
                }
            }
            state
                .operations
                .push(HostOperation::Remove(from.to_path_buf()));
            state
                .operations
                .push(HostOperation::Write(to.to_path_buf()));
            Ok(())
        })
    }

    fn symlink<'a>(
        &'a self,
        target: &'a Path,
        link: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("in-memory host lock");
            insert_parent_dirs(&mut state.nodes, link);
            state.nodes.insert(
                link.to_path_buf(),
                MemoryNode::Symlink(target.to_path_buf()),
            );
            state.operations.push(HostOperation::CreateSymlink {
                link: link.to_path_buf(),
                target: target.to_path_buf(),
            });
            Ok(())
        })
    }
}

impl PrivilegedFileSystemHost for InMemoryHost {
    fn acquire_privileged_operation<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<super::PrivilegedOperationGuard>> + Send + 'a>>
    {
        Box::pin(async move {
            self.state
                .lock()
                .expect("in-memory host lock")
                .operations
                .push(HostOperation::AcquirePrivilegedOperation);
            Ok(Box::new(()) as super::PrivilegedOperationGuard)
        })
    }

    fn write_privileged<'a>(
        &'a self,
        path: &'a Path,
        bytes: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            self.write_atomic(path, bytes)
                .await
                .map_err(|error| error.into_anyhow("privileged write"))?;
            self.state
                .lock()
                .expect("in-memory host lock")
                .operations
                .push(HostOperation::WritePrivileged(path.to_path_buf()));
            Ok(())
        })
    }
    fn set_mode_privileged<'a>(
        &'a self,
        path: &'a Path,
        mode: u32,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            self.set_mode(path, mode)
                .await
                .map_err(|error| error.into_anyhow("privileged set mode"))?;
            self.state
                .lock()
                .expect("in-memory host lock")
                .operations
                .push(HostOperation::SetModePrivileged {
                    path: path.to_path_buf(),
                    mode,
                });
            Ok(())
        })
    }
    fn move_privileged<'a>(
        &'a self,
        from: &'a Path,
        to: &'a Path,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            self.rename(from, to)
                .await
                .map_err(|error| error.into_anyhow("privileged move"))?;
            self.state
                .lock()
                .expect("in-memory host lock")
                .operations
                .push(HostOperation::MovePrivileged {
                    from: from.to_path_buf(),
                    to: to.to_path_buf(),
                });
            Ok(())
        })
    }
    fn remove_privileged<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let removed = match self.remove_file(path).await {
                Ok(()) => Ok(()),
                Err(error) if error.is_not_found() => Ok(()),
                Err(error) => Err(error.into_anyhow("privileged remove")),
            };
            removed?;
            self.state
                .lock()
                .expect("in-memory host lock")
                .operations
                .push(HostOperation::RemovePrivileged(path.to_path_buf()));
            Ok(())
        })
    }
}

impl ProcessHost for InMemoryHost {
    fn run<'a>(
        &'a self,
        request: ProcessRequest,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ProcessOutput>> + Send + 'a>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("in-memory host lock");
            state.operations.push(HostOperation::Run {
                program: request.program,
                args: request.args,
            });
            state
                .process_outputs
                .pop_front()
                .unwrap_or_else(|| Ok(ProcessOutput::default()))
        })
    }
}

impl SplitDnsObservationHost for InMemoryHost {
    fn inspect_split_dns<'a>(
        &'a self,
        request: &'a SplitDnsRequest,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<SplitDnsState>> + Send + 'a>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("in-memory host lock");
            state.operations.push(HostOperation::InspectSplitDns {
                domain: request.domain.clone(),
            });
            Ok(match state.nodes.get(&request.resource) {
                Some(MemoryNode::File(content)) => SplitDnsState {
                    exists: true,
                    content: content.clone(),
                },
                _ => SplitDnsState::default(),
            })
        })
    }
}

impl SplitDnsHost for InMemoryHost {
    fn apply_split_dns<'a>(
        &'a self,
        request: &'a SplitDnsRequest,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("in-memory host lock");
            insert_parent_dirs(&mut state.nodes, &request.resource);
            state.nodes.insert(
                request.resource.clone(),
                MemoryNode::File(request.content.clone()),
            );
            state.operations.push(HostOperation::ApplySplitDns {
                domain: request.domain.clone(),
            });
            Ok(())
        })
    }

    fn remove_split_dns<'a>(
        &'a self,
        request: &'a SplitDnsRequest,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("in-memory host lock");
            state.nodes.remove(&request.resource);
            state.operations.push(HostOperation::RemoveSplitDns {
                domain: request.domain.clone(),
            });
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn records_virtual_file_and_process_operations() {
        let host = InMemoryHost::new();
        host.write_atomic(Path::new("/state/a"), b"a")
            .await
            .unwrap();
        host.symlink(Path::new("/state/a"), Path::new("/bin/a"))
            .await
            .unwrap();
        let output = host
            .run(ProcessRequest {
                program: "tool".into(),
                args: vec!["arg".into()],
                ..ProcessRequest::default()
            })
            .await
            .unwrap();

        assert_eq!(host.read(Path::new("/bin/a")).await.unwrap(), b"a");
        assert_eq!(output, ProcessOutput::default());
        assert!(host.operations().iter().any(|operation| matches!(
            operation,
            HostOperation::Run { program, .. } if program == "tool"
        )));
    }
}
