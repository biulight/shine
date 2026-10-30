//! CLI adapters for platform-private files and configured local/broker decryption.
use crate::env::workspace::file_groups::{filesystem, platform};
use anyhow::{Context, Result, ensure};
use shine_core::runtime::secret_files::*;
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};
use zeroize::Zeroizing;
#[cfg(all(test, unix))]
mod tests;

pub(crate) struct Provider {
    config: crate::config::Config,
}
impl Provider {
    pub fn new(config: crate::config::Config) -> Self {
        Self { config }
    }
}

type Keys = Mutex<BTreeMap<PathBuf, Arc<Zeroizing<Vec<u8>>>>>;
static KEYS: OnceLock<Keys> = OnceLock::new();
impl Provider {
    fn key_path(&self) -> PathBuf {
        self.config.shine_dir().join("app-secret-fingerprint.key")
    }
    fn key(&self) -> Result<Arc<Zeroizing<Vec<u8>>>> {
        let path = self.key_path();
        let mut keys = KEYS
            .get_or_init(Mutex::default)
            .lock()
            .map_err(|_| anyhow::anyhow!("fingerprint key lock poisoned"))?;
        if let Some(key) = keys.get(&path) {
            if let Some(bytes) = private_read(&path)? {
                ensure!(
                    bytes.as_slice() == key.as_slice(),
                    "App fingerprint key changed; retry with fresh state"
                );
            } else {
                ensure!(
                    !self
                        .config
                        .shine_dir()
                        .join("app-secret-manifest.json")
                        .try_exists()?,
                    "App fingerprint key missing; restore the key before continuing"
                );
            }
            return Ok(key.clone());
        }
        let bytes = match private_read(&path)? {
            Some(bytes) => {
                ensure!(bytes.len() == 32, "invalid App secret fingerprint key");
                bytes
            }
            None => {
                ensure!(
                    !self
                        .config
                        .shine_dir()
                        .join("app-secret-manifest.json")
                        .try_exists()?,
                    "App secret fingerprint key missing; preserve state and restore the key"
                );
                let mut key = Zeroizing::new(vec![0; 32]);
                getrandom::getrandom(&mut key)
                    .map_err(|_| anyhow::anyhow!("cannot generate fingerprint key"))?;
                key
            }
        };
        let key = Arc::new(bytes);
        keys.insert(path, key.clone());
        Ok(key)
    }
}

fn root_for(path: &Path) -> Result<&Path> {
    ensure!(path.is_absolute(), "secret path must be absolute");
    path.ancestors().last().context("secret path root missing")
}
fn check(path: &Path) -> Result<()> {
    filesystem::check_path(root_for(path)?, path)
}
fn private_read(path: &Path) -> Result<Option<Zeroizing<Vec<u8>>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    check(path)?;
    #[cfg(windows)]
    platform::verify_acl(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0,
            "App secret state must be owned by the current user and private"
        );
    }
    let _ = metadata;
    Ok(Some(filesystem::read_file(path, 8 * 1024 * 1024)?.1))
}
fn parents(path: &Path) -> Result<()> {
    let parent = path.parent().context("secret target parent missing")?;
    if parent.try_exists()? {
        return Ok(());
    }
    parents(parent)?;
    platform::private_directory(parent)?;
    Ok(())
}

fn check_parent_access(path: &Path, permissions: &SecretFilePermissions) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let uid = permissions.uid.unwrap_or(unsafe { libc::geteuid() });
        if uid == unsafe { libc::geteuid() } || uid == 0 {
            return Ok(());
        }
        let gid = permissions.gid.unwrap_or(unsafe { libc::getegid() });
        for parent in path.ancestors().skip(1) {
            let metadata = fs::symlink_metadata(parent).with_context(|| {
                format!(
                    "pre-create secret parent directories accessible to uid {uid}: {}",
                    parent.display()
                )
            })?;
            let execute = if metadata.uid() == uid {
                0o100
            } else if metadata.gid() == gid {
                0o010
            } else {
                0o001
            };
            ensure!(
                metadata.is_dir() && metadata.mode() & execute != 0,
                "secret parent must grant Unix directory traversal to uid {uid}, gid {gid}: {}",
                parent.display()
            );
        }
    }
    #[cfg(not(unix))]
    let _ = (path, permissions);
    Ok(())
}

fn write_private(
    path: &Path,
    bytes: &[u8],
    permissions: &SecretFilePermissions,
    replace: bool,
) -> Result<()> {
    check(path)?;
    check_parent_access(path, permissions)?;
    parents(path)?;
    let temp = path.with_file_name(format!(".shine-secret-{}", uuid::Uuid::new_v4()));
    #[cfg(not(unix))]
    ensure!(
        permissions.uid.is_none() && permissions.gid.is_none(),
        "Unix ownership is unsupported on this platform"
    );
    let result = (|| {
        let mut file = platform::create_private(&temp)?;
        file.write_all(bytes)?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            use std::os::unix::fs::PermissionsExt;
            let uid = permissions.uid.unwrap_or(unsafe { libc::geteuid() });
            let gid = permissions.gid.unwrap_or(unsafe { libc::getegid() });
            ensure!(
                unsafe { libc::fchown(file.as_raw_fd(), uid, gid) } == 0,
                "cannot set secret file owner/group"
            );
            file.set_permissions(fs::Permissions::from_mode(permissions.mode))?;
        }
        file.sync_all()?;
        check(path)?;
        if replace {
            fs::rename(&temp, path)?;
        } else {
            platform::publish_new(&temp, path)?;
        }
        platform::sync_cipher_parents(root_for(path)?, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
impl SecretFileIo for Provider {
    fn resolve_permissions(&self, permissions: &SecretFilePermissions) -> SecretFilePermissions {
        #[cfg(unix)]
        {
            SecretFilePermissions {
                uid: Some(permissions.uid.unwrap_or(unsafe { libc::geteuid() })),
                gid: Some(permissions.gid.unwrap_or(unsafe { libc::getegid() })),
                mode: permissions.mode,
            }
        }
        #[cfg(not(unix))]
        {
            permissions.clone()
        }
    }
    fn observe(&self, path: &Path) -> Result<Option<SecretObservation>> {
        check(path)?;
        match fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
            _ => (),
        }
        let (file, bytes) = filesystem::read_file(path, MAX_FILE)?;
        #[cfg(unix)]
        let metadata = file.metadata()?;
        #[cfg(unix)]
        let (permissions, identity) = {
            use std::os::unix::fs::MetadataExt;
            (
                SecretFilePermissions {
                    uid: Some(metadata.uid()),
                    gid: Some(metadata.gid()),
                    mode: metadata.mode() & 0o7777,
                },
                format!("{}:{}", metadata.dev(), metadata.ino()),
            )
        };
        #[cfg(windows)]
        let (permissions, identity) = (
            SecretFilePermissions {
                uid: None,
                gid: None,
                mode: if platform::verify_acl(path).is_ok() {
                    0o600
                } else {
                    0o777
                },
            },
            format!("{:?}", platform::identity(&file)?),
        );
        Ok(Some(SecretObservation {
            fingerprint: self.fingerprint(&bytes)?,
            permissions,
            identity,
        }))
    }
    fn fingerprint(&self, bytes: &[u8]) -> Result<String> {
        keyed_fingerprint(&self.key()?, bytes)
    }
    fn state(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        ensure!(
            matches!(
                name,
                "app-secret-manifest.json" | "app-secret-operation.json"
            ),
            "invalid secret state name"
        );
        private_read(&self.config.shine_dir().join(name))
    }
    fn save_state(&self, name: &str, bytes: Option<&[u8]>) -> Result<()> {
        self.state(name)?;
        let path = self.config.shine_dir().join(name);
        match bytes {
            Some(bytes) => write_private(
                &path,
                bytes,
                &SecretFilePermissions {
                    uid: None,
                    gid: None,
                    mode: 0o600,
                },
                true,
            ),
            None => {
                fs::remove_file(&path)?;
                platform::sync_cipher_parents(root_for(&path)?, &path)
            }
        }
    }
    fn prepare_key(&self) -> Result<()> {
        let key = self.key()?;
        let path = self.key_path();
        match private_read(&path)? {
            Some(bytes) => ensure!(
                bytes.as_slice() == key.as_slice(),
                "fingerprint key changed"
            ),
            None => write_private(
                &path,
                &key,
                &SecretFilePermissions {
                    uid: None,
                    gid: None,
                    mode: 0o600,
                },
                false,
            )?,
        }
        Ok(())
    }
    fn lock_paths(&self, paths: &[PathBuf]) -> Result<Box<dyn Send>> {
        if paths.is_empty() {
            return Ok(Box::new(()));
        }
        let root = root_for(&paths[0])?;
        Ok(Box::new(filesystem::PathLocks::acquire(
            root,
            &paths[0],
            &paths
                .iter()
                .map(|p| (p.clone(), p.clone()))
                .collect::<Vec<_>>(),
        )?))
    }
    fn create(&self, path: &Path, bytes: &[u8], permissions: &SecretFilePermissions) -> Result<()> {
        self.check_target(path, permissions)?;
        write_private(path, bytes, permissions, false)
    }
    fn set_permissions(&self, path: &Path, permissions: &SecretFilePermissions) -> Result<()> {
        check(path)?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            use std::os::unix::fs::PermissionsExt;
            let file = platform::open_read(path)?;
            platform::identity(&file)?;
            // Restrict access before changing ownership; the final mode is the approved one.
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
            ensure!(
                unsafe {
                    libc::fchown(
                        file.as_raw_fd(),
                        permissions.uid.unwrap_or(libc::geteuid()),
                        permissions.gid.unwrap_or(libc::getegid()),
                    )
                } == 0,
                "cannot set secret rollback ownership"
            );
            file.set_permissions(fs::Permissions::from_mode(permissions.mode))?;
            file.sync_all()?;
        }
        #[cfg(windows)]
        {
            let _ = permissions;
            platform::verify_acl(path)?;
        }
        Ok(())
    }
    fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        check(from)?;
        check(to)?;
        ensure!(!to.try_exists()?, "secret rename destination occupied");
        fs::rename(from, to)?;
        platform::sync_cipher_parents(root_for(to)?, to)
    }
    fn remove(&self, path: &Path) -> Result<()> {
        check(path)?;
        fs::remove_file(path)?;
        platform::sync_cipher_parents(root_for(path)?, path)
    }
    fn check_target(&self, path: &Path, permissions: &SecretFilePermissions) -> Result<()> {
        filesystem::check_git(root_for(path)?, path)?;
        check_parent_access(path, permissions)
    }
}
impl SecretFileProvider for Provider {
    fn decrypt<'a>(&'a self, request: &'a FileReleaseRequest) -> SecretFuture<'a> {
        Box::pin(platform::prepare(async move {
            if crate::ssh::file_broker::enabled().await? {
                crate::ssh::file_broker::request(request.clone(), false).await
            } else {
                crate::ssh::file_broker::decrypt(&self.config, request).await
            }
        }))
    }
}
