use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileKind {
    File,
    Directory,
    Symlink,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileMetadata {
    pub kind: FileKind,
    pub len: u64,
}

#[derive(Debug)]
pub struct HostError {
    kind: std::io::ErrorKind,
    error: anyhow::Error,
}

impl HostError {
    pub fn new(kind: std::io::ErrorKind, error: impl Into<anyhow::Error>) -> Self {
        Self {
            kind,
            error: error.into(),
        }
    }

    pub fn io(error: std::io::Error) -> Self {
        Self::new(error.kind(), error)
    }

    pub fn is_not_found(&self) -> bool {
        self.kind == std::io::ErrorKind::NotFound
    }

    pub fn into_anyhow(self, context: &'static str) -> anyhow::Error {
        self.error.context(context)
    }
}

pub trait FileSystemHost {
    fn read<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, HostError>> + Send + 'a>>;

    fn write_atomic<'a>(
        &'a self,
        path: &'a Path,
        bytes: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>>;

    fn write<'a>(
        &'a self,
        path: &'a Path,
        bytes: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>>;

    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>>;

    fn remove_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>>;

    fn rename<'a>(
        &'a self,
        from: &'a Path,
        to: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>>;

    fn metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<FileMetadata, HostError>> + Send + 'a>>;

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<PathBuf>, HostError>> + Send + 'a>>;

    fn symlink<'a>(
        &'a self,
        target: &'a Path,
        link: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>>;

    fn read_link<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<PathBuf, HostError>> + Send + 'a>>;
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProcessRequest {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
    pub stdin: Vec<u8>,
    pub timeout: Option<Duration>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProcessOutput {
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub trait ProcessHost {
    fn run<'a>(
        &'a self,
        request: ProcessRequest,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ProcessOutput>> + Send + 'a>>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostOperation {
    Read(PathBuf),
    Write(PathBuf),
    CreateDirectory(PathBuf),
    Remove(PathBuf),
    CreateSymlink { link: PathBuf, target: PathBuf },
    Run { program: String, args: Vec<String> },
    Confirm { code: String },
    AuthorizeAdmin { item_count: usize },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeEvent {
    Section { domain: &'static str },
    Progress { code: &'static str, target: String },
    Warning { code: &'static str, detail: String },
}

pub trait RuntimeObserver {
    fn emit(&mut self, event: RuntimeEvent);
}

#[derive(Default)]
pub struct NullObserver;

impl RuntimeObserver for NullObserver {
    fn emit(&mut self, _event: RuntimeEvent) {}
}

pub trait RuntimeInteraction {
    fn confirm(&mut self, code: &'static str, default: bool) -> anyhow::Result<bool>;

    fn authorize_admin<'a>(
        &'a mut self,
        item_count: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send + 'a>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RealHost;

impl FileSystemHost for RealHost {
    fn read<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, HostError>> + Send + 'a>> {
        Box::pin(async move { tokio::fs::read(path).await.map_err(HostError::io) })
    }

    fn write_atomic<'a>(
        &'a self,
        path: &'a Path,
        bytes: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        Box::pin(async move {
            let parent = path.parent().unwrap_or_else(|| Path::new("."));
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(HostError::io)?;
            let temp = parent.join(format!(".shine-core-write-{}", uuid::Uuid::new_v4()));
            if let Err(error) = tokio::fs::write(&temp, bytes).await {
                return Err(HostError::io(error));
            }
            #[cfg(windows)]
            if path.exists() {
                tokio::fs::remove_file(path).await.map_err(HostError::io)?;
            }
            if let Err(error) = tokio::fs::rename(&temp, path).await {
                let _ = tokio::fs::remove_file(&temp).await;
                return Err(HostError::io(error));
            }
            Ok(())
        })
    }

    fn write<'a>(
        &'a self,
        path: &'a Path,
        bytes: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        Box::pin(async move { tokio::fs::write(path, bytes).await.map_err(HostError::io) })
    }

    fn create_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        Box::pin(async move { tokio::fs::create_dir_all(path).await.map_err(HostError::io) })
    }

    fn remove_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        Box::pin(async move { tokio::fs::remove_file(path).await.map_err(HostError::io) })
    }

    fn rename<'a>(
        &'a self,
        from: &'a Path,
        to: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        Box::pin(async move { tokio::fs::rename(from, to).await.map_err(HostError::io) })
    }

    fn metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<FileMetadata, HostError>> + Send + 'a>> {
        Box::pin(async move {
            let metadata = tokio::fs::symlink_metadata(path)
                .await
                .map_err(HostError::io)?;
            let kind = if metadata.file_type().is_symlink() {
                FileKind::Symlink
            } else if metadata.is_dir() {
                FileKind::Directory
            } else {
                FileKind::File
            };
            Ok(FileMetadata {
                kind,
                len: metadata.len(),
            })
        })
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<PathBuf>, HostError>> + Send + 'a>> {
        Box::pin(async move {
            let mut entries = tokio::fs::read_dir(path).await.map_err(HostError::io)?;
            let mut paths = Vec::new();
            while let Some(entry) = entries.next_entry().await.map_err(HostError::io)? {
                paths.push(entry.path());
            }
            paths.sort();
            Ok(paths)
        })
    }

    fn symlink<'a>(
        &'a self,
        target: &'a Path,
        link: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        Box::pin(async move {
            #[cfg(unix)]
            {
                tokio::fs::symlink(target, link)
                    .await
                    .map_err(HostError::io)
            }
            #[cfg(windows)]
            {
                tokio::fs::symlink_file(target, link)
                    .await
                    .map_err(HostError::io)
            }
        })
    }

    fn read_link<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<PathBuf, HostError>> + Send + 'a>> {
        Box::pin(async move { tokio::fs::read_link(path).await.map_err(HostError::io) })
    }
}

impl ProcessHost for RealHost {
    fn run<'a>(
        &'a self,
        request: ProcessRequest,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ProcessOutput>> + Send + 'a>> {
        Box::pin(async move {
            let mut command = tokio::process::Command::new(&request.program);
            command
                .args(&request.args)
                .envs(&request.env)
                .stdin(if request.stdin.is_empty() {
                    Stdio::null()
                } else {
                    Stdio::piped()
                })
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if let Some(cwd) = &request.cwd {
                command.current_dir(cwd);
            }
            let mut child = command.spawn()?;
            if !request.stdin.is_empty()
                && let Some(mut stdin) = child.stdin.take()
            {
                stdin.write_all(&request.stdin).await?;
                stdin.shutdown().await?;
            }
            let output = if let Some(timeout) = request.timeout {
                tokio::time::timeout(timeout, child.wait_with_output())
                    .await
                    .map_err(|_| anyhow::anyhow!("process timed out"))??
            } else {
                child.wait_with_output().await?
            };
            Ok(ProcessOutput {
                exit_code: output.status.code(),
                stdout: output.stdout,
                stderr: output.stderr,
            })
        })
    }
}
