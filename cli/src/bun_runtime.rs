use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::path::Path;

const PACKAGE_JSON: &str = "package.json";
const LOCK_FILE: &str = "bun.lock";

pub use utils::runtime::{BunDependencyMode, BunRuntimeSpec};

/// Resolve the Bun dependency policy for one physical preset category.
///
/// Built-in scripts always disable package installation, even if an overlay adds
/// unrelated package files. External or overlay scripts may opt in by providing
/// both package.json and bun.lock at their category root.
pub fn resolve(category_root: &Path, allow_external_dependencies: bool) -> Result<BunRuntimeSpec> {
    if !allow_external_dependencies {
        return Ok(BunRuntimeSpec::default());
    }

    let package_path = category_root.join(PACKAGE_JSON);
    let lock_path = category_root.join(LOCK_FILE);
    let has_package = package_path.is_file();
    let has_lock = lock_path.is_file();
    match (has_package, has_lock) {
        (false, false) => return Ok(BunRuntimeSpec::default()),
        (true, false) => bail!(
            "external Bun preset dependency declaration requires {} beside {}",
            lock_path.display(),
            package_path.display()
        ),
        (false, true) => bail!(
            "external Bun preset dependency lock requires {} beside {}",
            package_path.display(),
            lock_path.display()
        ),
        (true, true) => {}
    }

    let package = std::fs::read(&package_path)
        .with_context(|| format!("reading Bun preset package: {}", package_path.display()))?;
    let parsed: Value = serde_json::from_slice(&package)
        .with_context(|| format!("parsing Bun preset package: {}", package_path.display()))?;
    if parsed.get("trustedDependencies").is_some() {
        bail!(
            "external Bun preset package must not declare trustedDependencies: {}",
            package_path.display()
        );
    }

    let lock = std::fs::read(&lock_path)
        .with_context(|| format!("reading Bun preset lock: {}", lock_path.display()))?;
    let mut bytes = Vec::with_capacity(package.len() + lock.len() + 1);
    bytes.extend_from_slice(&package);
    bytes.push(0);
    bytes.extend_from_slice(&lock);
    Ok(BunRuntimeSpec {
        dependency_mode: BunDependencyMode::Locked,
        dependency_hash: Some(crate::install_core::hash_content(&bytes)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    async fn temp_dir(name: &str) -> PathBuf {
        crate::test_support::make_temp_dir(name).await
    }

    #[tokio::test]
    async fn built_in_scripts_ignore_package_files() {
        let dir = temp_dir("bun-runtime-built-in").await;
        std::fs::write(
            dir.join(PACKAGE_JSON),
            r#"{"dependencies":{"zod":"4.0.0"}}"#,
        )
        .unwrap();
        let spec = resolve(&dir, false).unwrap();
        assert_eq!(spec, BunRuntimeSpec::default());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn external_scripts_require_a_complete_locked_pair() {
        let dir = temp_dir("bun-runtime-pair").await;
        std::fs::write(
            dir.join(PACKAGE_JSON),
            r#"{"dependencies":{"zod":"4.0.0"}}"#,
        )
        .unwrap();
        let error = resolve(&dir, true).unwrap_err().to_string();
        assert!(error.contains("bun.lock"));
        std::fs::remove_file(dir.join(PACKAGE_JSON)).unwrap();
        std::fs::write(dir.join(LOCK_FILE), "{}").unwrap();
        let error = resolve(&dir, true).unwrap_err().to_string();
        assert!(error.contains("package.json"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn external_scripts_accept_locked_dependencies_and_hash_both_files() {
        let dir = temp_dir("bun-runtime-locked").await;
        std::fs::write(
            dir.join(PACKAGE_JSON),
            r#"{"dependencies":{"zod":"4.0.0"}}"#,
        )
        .unwrap();
        std::fs::write(dir.join(LOCK_FILE), "lockfileVersion = 1\n").unwrap();
        let first = resolve(&dir, true).unwrap();
        assert_eq!(first.dependency_mode, BunDependencyMode::Locked);
        assert!(first.dependency_hash.is_some());
        std::fs::write(dir.join(LOCK_FILE), "lockfileVersion = 1\n# changed\n").unwrap();
        let second = resolve(&dir, true).unwrap();
        assert_ne!(first.dependency_hash, second.dependency_hash);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn external_scripts_reject_trusted_dependencies() {
        let dir = temp_dir("bun-runtime-trusted").await;
        std::fs::write(dir.join(PACKAGE_JSON), r#"{"trustedDependencies":[]}"#).unwrap();
        std::fs::write(dir.join(LOCK_FILE), "{}").unwrap();
        let error = resolve(&dir, true).unwrap_err().to_string();
        assert!(error.contains("trustedDependencies"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
