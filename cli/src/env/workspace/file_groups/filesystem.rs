use super::*;
use std::{fs::File, io::Read, path::Component, process::Command};

pub(crate) fn resolve(root: &Path, pattern: &str, mode: &str) -> Result<PathBuf> {
    let expanded = pattern.replace("{mode}", mode);
    ensure!(
        !expanded.is_empty() && !expanded.contains(['{', '}', '\\', ':']),
        "invalid file-group path"
    );
    let path = Path::new(&expanded);
    ensure!(
        path.components().all(|c| matches!(c, Component::Normal(_))),
        "file-group paths must be relative without parent traversal"
    );
    let path = root.join(path);
    check_path(root, &path)?;
    Ok(path)
}

pub(crate) fn check_path(root: &Path, path: &Path) -> Result<()> {
    let root_metadata = fs::symlink_metadata(root)?;
    ensure!(
        root_metadata.is_dir() && !platform::is_link(&root_metadata),
        "workspace root was replaced"
    );
    let relative = path
        .strip_prefix(root)
        .context("file-group path outside workspace")?;
    let mut current = root.to_owned();
    for component in relative.components() {
        ensure!(
            matches!(component, Component::Normal(_)),
            "invalid path component"
        );
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                ensure!(
                    !platform::is_link(&metadata),
                    "linked file-group path rejected: {}",
                    current.display()
                );
                if current == path {
                    ensure!(
                        metadata.is_file(),
                        "mapped path is not a regular file: {}",
                        current.display()
                    );
                    platform::path_identity(&current)?;
                } else {
                    ensure!(
                        metadata.is_dir(),
                        "file-group parent is not a directory: {}",
                        current.display()
                    );
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub(crate) struct CapturedFile {
    pub path: PathBuf,
    pub bytes: Zeroizing<Vec<u8>>,
    file: File,
    identity: platform::Identity,
    limit: usize,
}

pub(crate) fn read_file(path: &Path, limit: usize) -> Result<(File, Zeroizing<Vec<u8>>)> {
    let mut file = platform::open_read(path)?;
    platform::identity(&file)?;
    ensure!(
        file.metadata()?.len() <= limit as u64,
        "file exceeds size limit: {}",
        path.display()
    );
    let mut bytes = Zeroizing::new(Vec::new());
    (&mut file)
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "file exceeds size limit: {}",
        path.display()
    );
    Ok((file, bytes))
}

impl CapturedFile {
    pub fn read(root: &Path, path: &Path, limit: usize) -> Result<Self> {
        check_path(root, path)?;
        let (file, bytes) = read_file(path, limit)?;
        let identity = platform::identity(&file)?;
        Ok(Self {
            path: path.into(),
            bytes,
            file,
            identity,
            limit,
        })
    }
    pub fn optional(root: &Path, path: &Path, limit: usize) -> Result<Option<Self>> {
        check_path(root, path)?;
        if path.try_exists()? {
            Ok(Some(Self::read(root, path, limit)?))
        } else {
            Ok(None)
        }
    }
    pub fn verify(&self, root: &Path) -> Result<()> {
        let current = Self::read(root, &self.path, self.limit)?;
        ensure!(
            platform::identity(&self.file)? == self.identity
                && current.identity == self.identity
                && current.bytes == self.bytes,
            "file changed during operation: {}",
            self.path.display()
        );
        Ok(())
    }
}

pub(crate) fn verify_optional(
    root: &Path,
    path: &Path,
    original: Option<&CapturedFile>,
    limit: usize,
) -> Result<()> {
    match original {
        Some(original) => original.verify(root),
        None => {
            ensure!(
                CapturedFile::optional(root, path, limit)?.is_none(),
                "file appeared during operation: {}",
                path.display()
            );
            Ok(())
        }
    }
}

fn git_reports_no_repository(stderr: &[u8]) -> bool {
    let Ok(message) = std::str::from_utf8(stderr) else {
        return false;
    };
    let message = message.trim();
    if message == "fatal: not a git repository (or any of the parent directories): .git" {
        return true;
    }
    // Git stops discovery at mount points by default. This is also confirmed absence,
    // but only when the caller found no repository marker in any ancestor.
    let mut lines = message.lines();
    lines.next().is_some_and(|line| {
        line.strip_prefix("fatal: not a git repository (or any parent up to mount point ")
            .and_then(|path| path.strip_suffix(')'))
            .is_some_and(|path| !path.is_empty())
    }) && lines.next()
        == Some("Stopping at filesystem boundary (GIT_DISCOVERY_ACROSS_FILESYSTEM not set).")
        && lines.next().is_none()
}

pub(crate) fn check_git(root: &Path, target: &Path) -> Result<()> {
    // Targets may belong to a nested repository, and their parents may not exist yet.
    check_path(root, target)?;
    let mut directory = target.parent().context("plaintext target has no parent")?;
    while !directory.try_exists()? {
        directory = directory.parent().context("no existing plaintext parent")?;
    }
    let mut repository_marker = false;
    for ancestor in directory.ancestors() {
        match fs::symlink_metadata(ancestor.join(".git")) {
            Ok(_) => repository_marker = true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error).context("cannot inspect Git repository marker"),
        }
    }
    let git = || {
        let mut cmd = Command::new("git");
        cmd.current_dir(directory)
            .args(["--no-optional-locks", "-c", "core.fsmonitor=false"])
            .env("LC_ALL", "C");
        for (key, _) in
            std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("GIT_"))
        {
            cmd.env_remove(key);
        }
        cmd
    };
    let discovery = git().args(["rev-parse", "--is-inside-work-tree"]).output();
    let in_git = match discovery {
        Ok(output) if output.status.success() && output.stdout == b"true\n" => true,
        // Exit 128 alone is ambiguous (bad config, unsafe ownership, broken worktree, etc.).
        Ok(output)
            if !repository_marker
                && output.status.code() == Some(128)
                && output.stdout.is_empty()
                && git_reports_no_repository(&output.stderr) =>
        {
            false
        }
        Ok(_) => bail!("cannot inspect Git repository"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !repository_marker => false,
        Err(error) => return Err(error).context("Git is required to verify plaintext exclusions"),
    };
    if !in_git {
        return Ok(());
    }
    let relative = target.strip_prefix(directory)?;
    let prefix = git().args(["rev-parse", "--show-prefix"]).output()?;
    ensure!(
        prefix.status.success(),
        "cannot inspect Git repository prefix"
    );
    let prefix = std::str::from_utf8(&prefix.stdout)
        .context("invalid Git repository prefix")?
        .strip_suffix('\n')
        .context("invalid Git repository prefix")?;
    let target_key = platform::path_key(&Path::new(prefix).join(relative));
    // A literal pathspec misses case/Unicode aliases, especially when the tracked
    // working file is absent. Enumerate the entire index, not only the current
    // directory: even a parent directory can have a different index spelling.
    let tracked = git()
        .args(["ls-files", "--full-name", "-z", "--", ":/"])
        .output()?;
    ensure!(tracked.status.success(), "cannot inspect Git index");
    let mut is_tracked = false;
    for path in tracked
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let path = std::str::from_utf8(path).context("invalid Git index path")?;
        is_tracked |= platform::path_key(Path::new(path)) == target_key;
    }
    ensure!(
        !is_tracked,
        "plaintext target is tracked or Git inspection failed: {}",
        target.display()
    );
    let ignored = git()
        .args(["check-ignore", "--quiet", "--no-index", "--"])
        .arg(relative)
        .output()?;
    ensure!(
        ignored.status.success(),
        "plaintext target must be ignored by Git: {}",
        target.display()
    );
    Ok(())
}

pub(crate) struct PathLocks(Vec<File>);
impl PathLocks {
    pub fn acquire(root: &Path, workspace: &Path, mappings: &[(PathBuf, PathBuf)]) -> Result<Self> {
        let base = fs::canonicalize(std::env::temp_dir())?.join(platform::lock_directory()?);
        platform::private_directory(&base)?;
        let paths = std::iter::once(workspace).chain(
            mappings
                .iter()
                .flat_map(|(s, t)| [s.as_path(), t.as_path()]),
        );
        let mut keys = BTreeSet::new();
        for path in paths {
            check_path(root, path)?;
            keys.insert(shine_core::digest::sha256_hex(&platform::path_key(path)));
        }
        let mut files = Vec::new();
        for key in keys {
            let path = base.join(key);
            let file = platform::lock_file(&path)?;
            file.try_lock()
                .context("another Shine file-group operation is active; retry when it finishes")?;
            files.push(file);
        }
        Ok(Self(files))
    }
}
impl Drop for PathLocks {
    fn drop(&mut self) {
        for file in &self.0 {
            let _ = file.unlock();
        }
    }
}

struct OwnedFile {
    path: PathBuf,
    file: File,
    identity: platform::Identity,
}
pub(crate) struct Materialized {
    root: PathBuf,
    files: Vec<OwnedFile>,
    directories: Vec<(PathBuf, File, platform::Identity)>,
}
impl Materialized {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.into(),
            files: Vec::new(),
            directories: Vec::new(),
        }
    }
    fn parents(&mut self, path: &Path) -> Result<()> {
        let mut current = self.root.clone();
        for component in path
            .parent()
            .context("parent required")?
            .strip_prefix(&self.root)?
            .components()
        {
            current.push(component);
            if !current.try_exists()? && platform::private_directory(&current)? {
                let file = platform::open_directory(&current)?;
                let identity = platform::directory_identity(&file)?;
                self.directories.push((current.clone(), file, identity));
            }
            ensure!(
                !platform::is_link(&fs::symlink_metadata(&current)?) && current.is_dir(),
                "invalid file-group parent"
            );
        }
        Ok(())
    }
    pub fn create(&mut self, path: &Path, bytes: &[u8]) -> Result<()> {
        use std::io::Write;
        check_path(&self.root, path)?;
        self.parents(path)?;
        check_path(&self.root, path)?;
        let file = platform::create_private(path)?;
        let identity = platform::identity(&file)?;
        self.files.push(OwnedFile {
            path: path.into(),
            file,
            identity,
        });
        let owned = self.files.last_mut().expect("just pushed");
        owned.file.write_all(bytes)?;
        owned.file.sync_all()?;
        Ok(())
    }
    /// Transfer the completed files and directories to the caller; Drop no longer removes them.
    pub fn persist(&mut self) -> Result<()> {
        for owned in &self.files {
            check_path(&self.root, &owned.path)?;
            ensure!(
                platform::path_identity(&owned.path)? == owned.identity
                    && platform::identity(&owned.file)? == owned.identity,
                "plaintext target was replaced before export completed: {}",
                owned.path.display()
            );
        }
        self.files.clear();
        self.directories.clear();
        Ok(())
    }

    pub fn cleanup(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        for owned in self.files.drain(..).rev() {
            let remove = (|| {
                check_path(&self.root, &owned.path)?;
                let current = match platform::path_identity(&owned.path) {
                    Ok(identity) => identity,
                    // A child may consume its credential file (and its parent).
                    // Check boundaries first so dangling replacement links still fail.
                    Err(error)
                        if error
                            .downcast_ref::<std::io::Error>()
                            .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
                    {
                        return Ok(());
                    }
                    Err(error) => return Err(error),
                };
                ensure!(
                    current == owned.identity && platform::identity(&owned.file)? == owned.identity,
                    "plaintext target was replaced"
                );
                match fs::remove_file(&owned.path) {
                    Ok(()) => (),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                    Err(error) => return Err(error.into()),
                }
                Ok::<_, anyhow::Error>(())
            })();
            if let Err(error) = remove {
                errors.push(format!("{}: {error}", owned.path.display()));
            }
        }
        for (path, file, identity) in self.directories.drain(..).rev() {
            // Recheck ancestors before touching a directory; never follow a substituted link.
            let same = path.parent().is_some_and(|parent| {
                let probe = parent.join(".shine-directory-probe");
                check_path(&self.root, &probe).is_ok()
            }) && platform::open_directory(&path)
                .and_then(|f| platform::directory_identity(&f))
                .is_ok_and(|id| id == identity)
                && platform::directory_identity(&file).is_ok_and(|id| id == identity);
            if same
                && let Err(error) = fs::remove_dir(&path)
                && !matches!(
                    error.kind(),
                    std::io::ErrorKind::DirectoryNotEmpty | std::io::ErrorKind::NotFound
                )
            {
                errors.push(format!("{}: {error}", path.display()));
            }
        }
        ensure!(errors.is_empty(), "{}", errors.join("; "));
        Ok(())
    }
}
impl Drop for Materialized {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            eprintln!(
                "File-group cleanup failed: {}",
                crate::output::escape_controls(&error.to_string())
            );
        }
    }
}

pub(crate) async fn write_cipher(
    root: &Path,
    path: &Path,
    bytes: &[u8],
    previous: Option<CapturedFile>,
) -> Result<()> {
    let mut parents = Materialized::new(root);
    parents.parents(path)?;
    let temp = path
        .parent()
        .context("parent required")?
        .join(format!(".shine-cipher-{}", uuid::Uuid::new_v4()));
    let result = async {
        use std::io::Write;
        let mut file = platform::create_private(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        verify_optional(root, path, previous.as_ref(), MAX_CIPHER)?;
        let replacing = previous.is_some();
        // MoveFileExW cannot replace an open destination, even when its snapshot handle
        // shares deletion. Consume that handle only after the final snapshot check.
        // Plaintext ownership handles remain open until verification and cleanup finish.
        drop(previous);
        if replacing {
            crate::persist::finalize_temp(&temp, path).await?;
        } else {
            platform::publish_new(&temp, path)?;
        }
        platform::sync_cipher_parents(root, path)?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    let _ = fs::remove_file(&temp);
    // Keep newly created ciphertext parents; they contain no plaintext.
    parents.directories.clear();
    result
}
