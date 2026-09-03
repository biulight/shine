use anyhow::Context;
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
    pub unix_mode: Option<u32>,
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

    pub fn is_not_directory(&self) -> bool {
        self.kind == std::io::ErrorKind::NotADirectory
    }

    pub fn into_anyhow(self, context: &'static str) -> anyhow::Error {
        self.error.context(context)
    }
}

/// Observation-only filesystem capability used by security planning and
/// read-oriented runtime paths.
pub trait FileSystemObservationHost {
    fn canonicalize<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<PathBuf, HostError>> + Send + 'a>>;

    fn read<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, HostError>> + Send + 'a>>;

    fn metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<FileMetadata, HostError>> + Send + 'a>>;

    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<PathBuf>, HostError>> + Send + 'a>>;

    fn read_link<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<PathBuf, HostError>> + Send + 'a>>;
}

/// Filesystem mutation capability. Security planners deliberately bind only
/// [`FileSystemObservationHost`].
pub trait FileSystemHost: FileSystemObservationHost {
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

    fn remove_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>>;

    fn set_executable<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>>;

    fn set_mode<'a>(
        &'a self,
        path: &'a Path,
        mode: u32,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>>;

    fn rename<'a>(
        &'a self,
        from: &'a Path,
        to: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>>;

    fn symlink<'a>(
        &'a self,
        target: &'a Path,
        link: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>>;
}

/// Minimal privileged filesystem primitives. Core performs ownership,
/// backup/conflict and receipt decisions before invoking these operations.
pub type PrivilegedOperationGuard = Box<dyn Send>;

pub trait PrivilegedFileSystemHost {
    /// Serializes the complete ownership-check/backup/mutation transaction.
    fn acquire_privileged_operation<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<PrivilegedOperationGuard>> + Send + 'a>>;

    fn write_privileged<'a>(
        &'a self,
        path: &'a Path,
        bytes: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;
    fn set_mode_privileged<'a>(
        &'a self,
        path: &'a Path,
        mode: u32,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;
    fn move_privileged<'a>(
        &'a self,
        from: &'a Path,
        to: &'a Path,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;
    fn remove_privileged<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProcessRequest {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
    pub stdin: Vec<u8>,
    pub inherit_stdin: bool,
    pub timeout: Option<Duration>,
    pub io: ProcessIo,
    pub stdout_limit: Option<usize>,
    pub stderr_limit: Option<usize>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProcessIo {
    #[default]
    Captured,
    Inherit,
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
pub struct SplitDnsRequest {
    pub os_id: String,
    pub item_id: String,
    pub domain: String,
    pub servers: Vec<String>,
    pub resource: PathBuf,
    pub content: Vec<u8>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SplitDnsState {
    pub exists: bool,
    pub content: Vec<u8>,
}

/// Platform resource port used by the Core split-DNS driver. Core owns
/// normalization, receipt, conflict and privileged-operation serialization;
/// the host owns only the platform-specific observation and mutation primitive.
pub trait SplitDnsObservationHost {
    fn inspect_split_dns<'a>(
        &'a self,
        request: &'a SplitDnsRequest,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<SplitDnsState>> + Send + 'a>>;
}

pub trait SplitDnsHost: SplitDnsObservationHost {
    fn apply_split_dns<'a>(
        &'a self,
        request: &'a SplitDnsRequest,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;

    fn remove_split_dns<'a>(
        &'a self,
        request: &'a SplitDnsRequest,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostOperation {
    AcquirePrivilegedOperation,
    WritePrivileged(PathBuf),
    SetModePrivileged { path: PathBuf, mode: u32 },
    MovePrivileged { from: PathBuf, to: PathBuf },
    RemovePrivileged(PathBuf),
    Read(PathBuf),
    Write(PathBuf),
    CreateDirectory(PathBuf),
    Remove(PathBuf),
    RemoveDirectory(PathBuf),
    SetExecutable(PathBuf),
    SetMode { path: PathBuf, mode: u32 },
    CreateSymlink { link: PathBuf, target: PathBuf },
    Run { program: String, args: Vec<String> },
    InspectSplitDns { domain: String },
    ApplySplitDns { domain: String },
    RemoveSplitDns { domain: String },
    Confirm { code: String },
    AuthorizeAdmin { item_count: usize },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeEvent {
    Section {
        domain: &'static str,
    },
    Progress {
        code: &'static str,
        target: String,
    },
    Warning {
        code: &'static str,
        target: Option<String>,
        detail: String,
    },
    ProcessOutput {
        code: &'static str,
        target: String,
        stream: &'static str,
        text: String,
    },
    Interaction {
        code: &'static str,
        target: String,
    },
    SysBootstrapSelection {
        os_id: String,
        shell: String,
        item_ids: Vec<String>,
        item_labels: BTreeMap<String, String>,
        source: super::SelectionSource,
    },
    SysBootstrapOutcome(super::SysItemOutcome),
    SysBootstrapItemStart {
        item_id: String,
        label: String,
        requires_admin: bool,
    },
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

    fn select_many(
        &mut self,
        code: &'static str,
        choices: &[String],
        defaults: &[String],
    ) -> anyhow::Result<Vec<String>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RealHost;

impl FileSystemObservationHost for RealHost {
    fn canonicalize<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<PathBuf, HostError>> + Send + 'a>> {
        Box::pin(async move { tokio::fs::canonicalize(path).await.map_err(HostError::io) })
    }

    fn read<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, HostError>> + Send + 'a>> {
        Box::pin(async move { tokio::fs::read(path).await.map_err(HostError::io) })
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
            #[cfg(unix)]
            let unix_mode = {
                use std::os::unix::fs::PermissionsExt;
                Some(metadata.permissions().mode())
            };
            #[cfg(windows)]
            let unix_mode = None;
            Ok(FileMetadata {
                kind,
                len: metadata.len(),
                unix_mode,
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

    fn read_link<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<PathBuf, HostError>> + Send + 'a>> {
        Box::pin(async move { tokio::fs::read_link(path).await.map_err(HostError::io) })
    }
}

impl FileSystemHost for RealHost {
    fn set_mode<'a>(
        &'a self,
        path: &'a Path,
        mode: u32,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        Box::pin(async move {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
                    .await
                    .map_err(HostError::io)?;
            }
            #[cfg(windows)]
            let _ = (path, mode);
            Ok(())
        })
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

    fn remove_dir_all<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        Box::pin(async move { tokio::fs::remove_dir_all(path).await.map_err(HostError::io) })
    }

    fn set_executable<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        Box::pin(async move {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let metadata = tokio::fs::metadata(path).await.map_err(HostError::io)?;
                let mut permissions = metadata.permissions();
                permissions.set_mode(permissions.mode() | 0o111);
                tokio::fs::set_permissions(path, permissions)
                    .await
                    .map_err(HostError::io)?;
            }
            #[cfg(windows)]
            let _ = path;
            Ok(())
        })
    }

    fn rename<'a>(
        &'a self,
        from: &'a Path,
        to: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<(), HostError>> + Send + 'a>> {
        Box::pin(async move { tokio::fs::rename(from, to).await.map_err(HostError::io) })
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
                    if request.inherit_stdin {
                        Stdio::inherit()
                    } else {
                        Stdio::null()
                    }
                } else {
                    Stdio::piped()
                });
            match request.io {
                ProcessIo::Captured => {
                    command.stdout(Stdio::piped()).stderr(Stdio::piped());
                }
                ProcessIo::Inherit => {
                    command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
                }
            }
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
            let stdout = enforce_output_limit(output.stdout, request.stdout_limit, "stdout")?;
            let stderr = enforce_output_limit(output.stderr, request.stderr_limit, "stderr")?;
            Ok(ProcessOutput {
                exit_code: output.status.code(),
                stdout,
                stderr,
            })
        })
    }
}

impl PrivilegedFileSystemHost for RealHost {
    fn acquire_privileged_operation<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<PrivilegedOperationGuard>> + Send + 'a>> {
        Box::pin(acquire_admin_operation_lock())
    }

    fn write_privileged<'a>(
        &'a self,
        path: &'a Path,
        bytes: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(privileged_write(path, bytes))
    }

    fn set_mode_privileged<'a>(
        &'a self,
        path: &'a Path,
        mode: u32,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(privileged_set_mode(path, mode))
    }
    fn move_privileged<'a>(
        &'a self,
        from: &'a Path,
        to: &'a Path,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if !cfg!(unix) || std::env::var("USER").is_ok_and(|user| user == "root") {
                tokio::fs::rename(from, to).await?;
                return Ok(());
            }
            let status = tokio::process::Command::new("sudo")
                .args(["-n", "mv", "--"])
                .arg(from)
                .arg(to)
                .status()
                .await?;
            if !status.success() {
                anyhow::bail!("administrator permission was not granted");
            }
            Ok(())
        })
    }
    fn remove_privileged<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(privileged_remove(path))
    }
}

fn enforce_output_limit(
    output: Vec<u8>,
    limit: Option<usize>,
    stream: &'static str,
) -> anyhow::Result<Vec<u8>> {
    if limit.is_some_and(|limit| output.len() > limit) {
        anyhow::bail!("process {stream} exceeded configured limit");
    }
    Ok(output)
}

impl SplitDnsObservationHost for RealHost {
    fn inspect_split_dns<'a>(
        &'a self,
        request: &'a SplitDnsRequest,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<SplitDnsState>> + Send + 'a>> {
        Box::pin(async move {
            if request.os_id == "windows" {
                return inspect_windows_split_dns(request).await;
            }
            match tokio::fs::read(&request.resource).await {
                Ok(content) => Ok(SplitDnsState {
                    exists: true,
                    content,
                }),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    Ok(SplitDnsState::default())
                }
                Err(error) => Err(error.into()),
            }
        })
    }
}

impl SplitDnsHost for RealHost {
    fn apply_split_dns<'a>(
        &'a self,
        request: &'a SplitDnsRequest,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if request.os_id == "windows" {
                apply_windows_split_dns(request).await?;
            } else {
                privileged_write(&request.resource, &request.content).await?;
                if request.os_id == "ubuntu" {
                    restart_systemd_resolved().await?;
                }
            }
            Ok(())
        })
    }

    fn remove_split_dns<'a>(
        &'a self,
        request: &'a SplitDnsRequest,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if request.os_id == "windows" {
                remove_windows_split_dns(request).await?;
            } else {
                privileged_remove(&request.resource).await?;
                if request.os_id == "ubuntu" {
                    restart_systemd_resolved().await?;
                }
            }
            Ok(())
        })
    }
}

struct AdminOperationLock {
    path: PathBuf,
}

impl Drop for AdminOperationLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.path);
    }
}

async fn acquire_admin_operation_lock() -> anyhow::Result<PrivilegedOperationGuard> {
    let path = std::env::temp_dir().join("shine-admin.lock");
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        match tokio::fs::create_dir(&path).await {
            Ok(()) => return Ok(Box::new(AdminOperationLock { path })),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if std::time::Instant::now() >= deadline {
                    let _ = tokio::fs::remove_dir(&path).await;
                    continue;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(error) => {
                return Err(error).context("failed to acquire admin operation lock");
            }
        }
    }
}

fn split_dns_marker(item_id: &str) -> String {
    format!("Managed by shine: split-dns:{item_id}")
}

fn powershell_quote(value: &str) -> String {
    value.replace('\'', "''")
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct WindowsNrptRule {
    comment: String,
    namespace: Vec<String>,
    name_servers: Vec<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct WindowsNrptQuery {
    rules: Vec<WindowsNrptRule>,
}

async fn inspect_windows_split_dns(request: &SplitDnsRequest) -> anyhow::Result<SplitDnsState> {
    let marker = split_dns_marker(&request.item_id);
    let query = format!(
        "$rules=@(Get-DnsClientNrptRule | Where-Object {{$_.Comment -ceq '{}'}} | ForEach-Object {{[PSCustomObject]@{{Comment=$_.Comment;Namespace=@($_.Namespace | ForEach-Object {{$_.ToString()}});NameServers=@($_.NameServers | ForEach-Object {{$_.ToString()}})}}}}); [PSCustomObject]@{{Rules=@($rules)}} | ConvertTo-Json -Compress -Depth 3",
        powershell_quote(&marker),
    );
    let output = tokio::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &query])
        .output()
        .await?;
    if !output.status.success() {
        return Ok(SplitDnsState::default());
    }
    let query: WindowsNrptQuery = serde_json::from_slice(&output.stdout)?;
    if query.rules.is_empty() {
        return Ok(SplitDnsState::default());
    }
    let exact = query.rules.as_slice()
        == [WindowsNrptRule {
            comment: marker.clone(),
            namespace: vec![request.resource.display().to_string()],
            name_servers: request.servers.clone(),
        }];
    Ok(SplitDnsState {
        exists: true,
        content: if exact {
            request.content.clone()
        } else {
            marker.into_bytes()
        },
    })
}

impl PartialEq for WindowsNrptRule {
    fn eq(&self, other: &Self) -> bool {
        self.comment == other.comment
            && self.namespace == other.namespace
            && self.name_servers == other.name_servers
    }
}

async fn apply_windows_split_dns(request: &SplitDnsRequest) -> anyhow::Result<()> {
    let marker = split_dns_marker(&request.item_id);
    let servers = request
        .servers
        .iter()
        .map(|server| format!("'{}'", powershell_quote(server)))
        .collect::<Vec<_>>()
        .join(",");
    let script = format!(
        "$rules=@(Get-DnsClientNrptRule | Where-Object {{$_.Comment -eq '{marker}'}}); foreach($rule in $rules){{Remove-DnsClientNrptRule -Name $rule.Name -Force}}; Add-DnsClientNrptRule -Namespace '{namespace}' -NameServers @({servers}) -Comment '{marker}' | Out-Null",
        marker = powershell_quote(&marker),
        namespace = powershell_quote(&request.resource.display().to_string()),
    );
    elevated_powershell(&script).await
}

async fn remove_windows_split_dns(request: &SplitDnsRequest) -> anyhow::Result<()> {
    let marker = powershell_quote(&split_dns_marker(&request.item_id));
    elevated_powershell(&format!("$rules=@(Get-DnsClientNrptRule | Where-Object {{$_.Comment -eq '{marker}'}}); foreach($rule in $rules){{Remove-DnsClientNrptRule -Name $rule.Name -Force}}")).await
}

async fn elevated_powershell(script: &str) -> anyhow::Result<()> {
    let id = uuid::Uuid::new_v4();
    let script_path = std::env::temp_dir().join(format!("shine-system-{id}.ps1"));
    let result_path = std::env::temp_dir().join(format!("shine-system-{id}.result"));
    let body = format!(
        "$ErrorActionPreference='Stop'\ntry {{\n{script}\nSet-Content -LiteralPath '{}' -Value 'ok'\nexit 0\n}} catch {{\nSet-Content -LiteralPath '{}' -Value $_.Exception.Message\nexit 1\n}}\n",
        powershell_quote(&result_path.display().to_string()),
        powershell_quote(&result_path.display().to_string())
    );
    tokio::fs::write(&script_path, body).await?;
    let arguments = format!(
        "@('-NoProfile','-ExecutionPolicy','Bypass','-File','\"{}\"')",
        powershell_quote(&script_path.display().to_string())
    );
    let wrapper = format!(
        "$p=Start-Process powershell.exe -Verb RunAs -Wait -PassThru -ArgumentList {arguments}; exit $p.ExitCode"
    );
    let status = tokio::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &wrapper])
        .stdin(Stdio::inherit())
        .status()
        .await?;
    let result = tokio::fs::read_to_string(&result_path)
        .await
        .unwrap_or_else(|_| "elevated process did not return a result".to_string());
    let _ = tokio::fs::remove_file(&script_path).await;
    let _ = tokio::fs::remove_file(&result_path).await;
    if !status.success() {
        anyhow::bail!("elevated PowerShell operation failed: {}", result.trim());
    }
    Ok(())
}

async fn privileged_write(path: &Path, content: &[u8]) -> anyhow::Result<()> {
    if !cfg!(unix) || std::env::var("USER").is_ok_and(|user| user == "root") {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(path, content).await?;
        return Ok(());
    }
    let temp = std::env::temp_dir().join(format!("shine-system-{}", uuid::Uuid::new_v4()));
    tokio::fs::write(&temp, content).await?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("split DNS resource has no parent"))?;
    let mkdir = tokio::process::Command::new("sudo")
        .args(["-n", "mkdir", "-p"])
        .arg(parent)
        .status()
        .await?;
    if !mkdir.success() {
        let _ = tokio::fs::remove_file(&temp).await;
        anyhow::bail!("administrator permission was not granted");
    }
    let install = tokio::process::Command::new("sudo")
        .args(["-n", "install", "-m", "0644", "--"])
        .arg(&temp)
        .arg(path)
        .status()
        .await?;
    let _ = tokio::fs::remove_file(&temp).await;
    if !install.success() {
        anyhow::bail!("failed to install privileged split DNS resource");
    }
    Ok(())
}

async fn privileged_set_mode(path: &Path, mode: u32) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        let _ = (path, mode);
        return Ok(());
    }
    #[cfg(unix)]
    {
        if std::env::var("USER").is_ok_and(|user| user == "root") {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).await?;
            return Ok(());
        }
        let status = tokio::process::Command::new("sudo")
            .args(["-n", "chmod"])
            .arg(format!("{:o}", mode & 0o7777))
            .arg(path)
            .status()
            .await?;
        if !status.success() {
            anyhow::bail!("administrator permission was not granted");
        }
        Ok(())
    }
}

async fn privileged_remove(path: &Path) -> anyhow::Result<()> {
    if !cfg!(unix) || std::env::var("USER").is_ok_and(|user| user == "root") {
        match tokio::fs::remove_file(path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        return Ok(());
    }
    let status = tokio::process::Command::new("sudo")
        .args(["-n", "rm", "-f", "--"])
        .arg(path)
        .status()
        .await?;
    if !status.success() {
        anyhow::bail!("administrator permission was not granted");
    }
    Ok(())
}

async fn restart_systemd_resolved() -> anyhow::Result<()> {
    let status = if std::env::var("USER").is_ok_and(|user| user == "root") {
        tokio::process::Command::new("systemctl")
            .args(["restart", "systemd-resolved"])
            .status()
            .await?
    } else {
        tokio::process::Command::new("sudo")
            .args(["-n", "systemctl", "restart", "systemd-resolved"])
            .status()
            .await?
    };
    if !status.success() {
        anyhow::bail!("failed to restart systemd-resolved");
    }
    Ok(())
}
