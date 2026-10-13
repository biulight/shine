use super::*;
use std::{fs::File, io::Read, path::Component, process::Command};

pub(super) fn resolve(root: &Path, pattern: &str, mode: &str) -> Result<PathBuf> {
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

pub(super) fn check_path(root: &Path, path: &Path) -> Result<()> {
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
                    let file = platform::open_read(&current)?;
                    platform::identity(&file)?;
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

pub(super) struct CapturedFile {
    pub path: PathBuf,
    pub bytes: Zeroizing<Vec<u8>>,
    file: File,
    identity: platform::Identity,
    limit: usize,
}

pub(super) fn read_file(path: &Path, limit: usize) -> Result<(File, Zeroizing<Vec<u8>>)> {
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

pub(super) fn verify_optional(
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

pub(super) fn check_git(root: &Path, target: &Path) -> Result<()> {
    let git = || {
        let mut cmd = Command::new("git");
        cmd.current_dir(root)
            .args(["--no-optional-locks", "-c", "core.fsmonitor=false"]);
        for (key, _) in
            std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("GIT_"))
        {
            cmd.env_remove(key);
        }
        cmd
    };
    let discovery = git().args(["rev-parse", "--is-inside-work-tree"]).output();
    let in_git = match discovery {
        Ok(output) if output.status.success() => output.stdout == b"true\n",
        Ok(output) if output.status.code() == Some(128) => false,
        Ok(_) => bail!("cannot inspect Git repository"),
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                && !root.ancestors().any(|p| p.join(".git").exists()) =>
        {
            false
        }
        Err(error) => return Err(error).context("Git is required to verify plaintext exclusions"),
    };
    if !in_git {
        return Ok(());
    }
    let relative = target.strip_prefix(root)?;
    let tracked = git()
        .arg("--literal-pathspecs")
        .args(["ls-files", "--error-unmatch", "--"])
        .arg(relative)
        .output()?;
    ensure!(
        tracked.status.code() == Some(1),
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

pub(super) struct PathLocks(Vec<File>);
impl PathLocks {
    pub fn acquire(root: &Path, workspace: &Path, mappings: &[(PathBuf, PathBuf)]) -> Result<Self> {
        use fs2::FileExt;
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
            keys.insert(format!("{:x}", Sha256::digest(platform::path_key(path))));
        }
        let mut files = Vec::new();
        for key in keys {
            let path = base.join(key);
            let file = platform::lock_file(&path)?;
            file.try_lock_exclusive()
                .context("another Shine file-group operation is active; retry when it finishes")?;
            files.push(file);
        }
        Ok(Self(files))
    }
}
impl Drop for PathLocks {
    fn drop(&mut self) {
        for file in &self.0 {
            let _ = fs2::FileExt::unlock(file);
        }
    }
}

struct OwnedFile {
    path: PathBuf,
    file: File,
    identity: platform::Identity,
}
pub(super) struct Materialized {
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
    pub fn cleanup(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        for owned in self.files.drain(..).rev() {
            let remove = (|| {
                check_path(&self.root, &owned.path)?;
                let current = platform::open_read(&owned.path)?;
                ensure!(
                    platform::identity(&current)? == owned.identity
                        && platform::identity(&owned.file)? == owned.identity,
                    "plaintext target was replaced"
                );
                fs::remove_file(&owned.path)?;
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
            eprintln!("File-group cleanup failed: {error}");
        }
    }
}

pub(super) async fn write_cipher(
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
