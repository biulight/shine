use super::host::{
    FileKind, FileMetadata, FileSystemHost, HostError, HostOperation, ProcessHost, ProcessOutput,
    ProcessRequest,
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
    operations: Vec<HostOperation>,
    process_outputs: VecDeque<anyhow::Result<ProcessOutput>>,
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
        state.nodes.insert(path, MemoryNode::File(bytes));
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

impl FileSystemHost for InMemoryHost {
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

    fn write_atomic<'a>(
        &'a self,
        path: &'a Path,
        bytes: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("in-memory host lock");
            insert_parent_dirs(&mut state.nodes, path);
            state
                .nodes
                .insert(path.to_path_buf(), MemoryNode::File(bytes.to_vec()));
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
            state
                .operations
                .push(HostOperation::Remove(path.to_path_buf()));
            Ok(())
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
                }),
                Some(MemoryNode::File(bytes)) => Ok(FileMetadata {
                    kind: FileKind::File,
                    len: bytes.len() as u64,
                }),
                Some(MemoryNode::Symlink(target)) => Ok(FileMetadata {
                    kind: FileKind::Symlink,
                    len: target.as_os_str().len() as u64,
                }),
                None => Err(not_found(path)),
            }
        })
    }

    fn rename<'a>(
        &'a self,
        from: &'a Path,
        to: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("in-memory host lock");
            let Some(node) = state.nodes.remove(from) else {
                return Err(not_found(from));
            };
            insert_parent_dirs(&mut state.nodes, to);
            state.nodes.insert(to.to_path_buf(), node);
            state
                .operations
                .push(HostOperation::Remove(from.to_path_buf()));
            state
                .operations
                .push(HostOperation::Write(to.to_path_buf()));
            Ok(())
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
