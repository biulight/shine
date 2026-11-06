//! Only copies actually published by self install have binary removal authority.
use crate::{config::Config, output::escape_controls};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use shine_core::runtime::{FileSystemHost, PrivilegedOperationGuard, RealHost};
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
};

const FILE: &str = "self-installed.json";

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CopyReceipt {
    pub path: PathBuf,
    canonical: PathBuf,
    identity: String,
    digest: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Copies {
    version: u32,
    copies: Vec<CopyReceipt>,
}

pub(super) async fn lock(config: &Config) -> Result<PrivilegedOperationGuard> {
    RealHost
        .acquire_operation_lock(&config.shine_dir().join("self-install.lock"))
        .await
}

fn digest(file: &mut File) -> Result<String> {
    let mut hash = Sha256::new();
    io::copy(file, &mut hash)?;
    Ok(shine_core::digest::encode_sha256(&hash.finalize().into()))
}

fn observe(path: &Path) -> Result<Option<CopyReceipt>> {
    let stat = match std::fs::symlink_metadata(path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    ensure!(
        path.is_absolute() && stat.is_file(),
        "expected a regular installed program"
    );
    let file = crate::env::workspace::file_groups::platform::open_read(path)?;
    observe_open_file(path, file).map(Some)
}

fn observe_open_file(path: &Path, mut file: File) -> Result<CopyReceipt> {
    use crate::env::workspace::file_groups::platform;
    let identity = platform::identity(&file)?;
    let canonical = path.canonicalize()?;
    let digest = digest(&mut file)?;
    // A rename can replace the directory entry while a large executable is being hashed.
    ensure!(
        platform::path_identity(path)? == identity,
        "installed program changed while hashing {}; retry",
        escape_controls(&path.display().to_string())
    );
    Ok(CopyReceipt {
        path: path.into(),
        canonical,
        identity: identity.binary_identity(),
        digest,
    })
}

pub(super) fn load(config: &Config) -> Result<Vec<CopyReceipt>> {
    let path = config.shine_dir().join(FILE);
    let stat = match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
        other => other?,
    };
    ensure!(
        stat.is_file() && stat.len() <= 1024 * 1024,
        "invalid self-install receipt"
    );
    let file = crate::env::workspace::file_groups::platform::open_read(&path)?;
    let store: Copies = serde_json::from_reader(std::io::Read::take(file, 1024 * 1024 + 1))?;
    ensure!(store.version == 1, "unsupported self-install receipt");
    let mut seen = std::collections::BTreeSet::new();
    for copy in &store.copies {
        ensure!(
            copy.path.is_absolute() && copy.canonical.is_absolute() && seen.insert(&copy.canonical),
            "invalid self-install receipt paths"
        );
    }
    Ok(store.copies)
}

/// Carry ownership across an actual upgrade only if the pre-upgrade copy still matches.
pub(super) fn owned_upgrade_path(config: &Config, running: &Path) -> Result<Option<PathBuf>> {
    let canonical = running.canonicalize()?;
    for copy in load(config)? {
        if copy.canonical == canonical && observe(&copy.path)?.as_ref() == Some(&copy) {
            return Ok(Some(copy.path));
        }
    }
    Ok(None)
}

pub(super) async fn record(config: &Config, src: &Path, dest: &Path) -> Result<()> {
    let mut copies = load(config)?;
    let copy = observe(dest)?.context("installed program disappeared")?;
    ensure!(
        copy.digest == digest(&mut File::open(src)?)?,
        "installed program changed before receipt publication"
    );
    copies.retain(|c| c.path != copy.path && c.canonical != copy.canonical);
    copies.push(copy);
    crate::persist::atomic_write_private(
        &config.shine_dir().join(FILE),
        &serde_json::to_vec(&Copies { version: 1, copies })?,
    )
    .await
}

pub(super) fn verify(copies: &[CopyReceipt]) -> Result<()> {
    if copies.is_empty() {
        return Ok(());
    }
    static RUNNING_HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let executable_hash = if let Some(hash) = RUNNING_HASH.get() {
        hash
    } else {
        let hash = digest(&mut File::open(std::env::current_exe()?)?)?;
        RUNNING_HASH.get_or_init(|| hash)
    };
    for receipt in copies {
        let current = observe(&receipt.path)?;
        ensure!(
            current.is_none() || &receipt.digest == executable_hash,
            "binary at {} differs from this Shine build; inspect the other installation before retrying",
            escape_controls(&receipt.path.display().to_string())
        );
        ensure!(
            current.as_ref().is_none_or(|r| r == receipt),
            "installed program changed; preserving {}",
            escape_controls(&receipt.path.display().to_string())
        );
    }
    Ok(())
}

pub(super) fn remove(config: &Config, copies: &[CopyReceipt]) -> Result<()> {
    ensure!(
        load(config)? == copies,
        "self-install receipts changed; retry"
    );
    let running = std::env::current_exe()?.canonicalize()?;
    let mut ordered = copies.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|copy| copy.canonical == running);
    for receipt in ordered {
        if let Some(current) = observe(&receipt.path)? {
            ensure!(&current == receipt, "installed program changed; retry");
            std::fs::remove_file(&receipt.path).with_context(|| format!(
                "could not remove {}; close running instances and retry from another Shine copy with the same configuration",
                escape_controls(&receipt.path.display().to_string())))?;
        }
    }
    if !copies.is_empty() {
        std::fs::remove_file(config.shine_dir().join(FILE))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn receipt_rejects_a_replacement_after_open_even_when_bytes_match() {
        let dir = crate::test_support::make_temp_dir("shine-receipt-path-replaced").await;
        let path = dir.join("program");
        std::fs::write(&path, b"same").unwrap();
        let file = crate::env::workspace::file_groups::platform::open_read(&path).unwrap();
        std::fs::rename(&path, dir.join("previous")).unwrap();
        std::fs::write(&path, b"same").unwrap();
        assert!(
            observe_open_file(&path, file)
                .unwrap_err()
                .to_string()
                .contains("changed")
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"same");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn upgrade_retains_only_proven_program_ownership() {
        let dir = crate::test_support::make_temp_dir("shine-self-upgrade-receipt").await;
        let config = crate::test_support::test_config(&dir);
        let path = dir.join("program");
        std::fs::write(&path, b"version one").unwrap();
        assert!(owned_upgrade_path(&config, &path).unwrap().is_none());
        record(&config, &path, &path).await.unwrap();
        assert_eq!(
            owned_upgrade_path(&config, &path).unwrap(),
            Some(path.clone())
        );
        std::fs::rename(&path, dir.join("previous")).unwrap();
        std::fs::write(&path, b"version one").unwrap();
        assert!(owned_upgrade_path(&config, &path).unwrap().is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
