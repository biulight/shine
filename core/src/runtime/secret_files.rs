//! Explicit, snapshot-bound App secret files. Ciphertext is never a plaintext fallback.
use super::{AppFile, PresetSnapshot, RuntimePlatform};
use crate::install::AppInstallStrategy;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
pub(crate) mod managed;
pub use managed::{SecretEntry, SecretFileIo, SecretObservation, SecretStore};

pub type SecretFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<Vec<zeroize::Zeroizing<Vec<u8>>>>> + Send + 'a>,
>;

pub trait SecretFileProvider: SecretFileIo {
    fn decrypt<'a>(&'a self, request: &'a FileReleaseRequest) -> SecretFuture<'a>;
}

pub const MAX_FILE: usize = 16 * 1024 * 1024;
pub const MAX_TOTAL: usize = 64 * 1024 * 1024;
pub const MAX_CIPHER: usize = 32 * 1024 * 1024;
pub const MAX_CIPHER_TOTAL: usize = 128 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecretFileDeclaration {
    pub workspace: String,
    pub file_group: String,
    pub mode: String,
    #[serde(default)]
    pub auto_decrypt: bool,
    pub unix_uid: Option<u32>,
    pub unix_gid: Option<u32>,
    pub unix_mode: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SecretFilePermissions {
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub mode: u32,
}

impl SecretFileDeclaration {
    pub fn permissions(&self, platform: RuntimePlatform) -> Result<SecretFilePermissions> {
        ensure!(
            platform != RuntimePlatform::Windows
                || (self.unix_uid.is_none() && self.unix_gid.is_none() && self.unix_mode.is_none()),
            "Unix secret file permissions are not supported on Windows"
        );
        let mode = u32::from_str_radix(self.unix_mode.as_deref().unwrap_or("0600"), 8)
            .context("invalid secret file Unix mode")?;
        ensure!(
            matches!(mode, 0o400 | 0o600),
            "secret file mode must be 0400 or 0600"
        );
        Ok(SecretFilePermissions {
            uid: self.unix_uid,
            gid: self.unix_gid,
            mode,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AppSecretFile {
    pub workspace: PathBuf,
    pub group: String,
    pub mode: String,
    pub source: PathBuf,
    pub target: PathBuf,
    pub auto_decrypt: bool,
    pub permissions: SecretFilePermissions,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileMapping {
    pub source: String,
    pub target: String,
}

#[derive(Deserialize)]
struct Workspace {
    version: u32,
    #[serde(default)]
    env: WorkspaceEnv,
    #[serde(default)]
    file_groups: BTreeMap<String, FileGroup>,
}
#[derive(Default, Deserialize)]
struct WorkspaceEnv {
    #[serde(default)]
    modes: Vec<String>,
    default_mode: Option<String>,
}
#[derive(Deserialize)]
struct FileGroup {
    files: Vec<FileMapping>,
}

/// This portable grammar rejects Windows paths even when validated on Unix.
pub fn relative_path(value: &str) -> Result<PathBuf> {
    ensure!(
        !value.is_empty()
            && !value.contains(['\\', ':', '\0', '{', '}'])
            && !value.chars().any(char::is_control),
        "invalid secret file relative path"
    );
    let path = Path::new(value);
    ensure!(
        path.components().all(|c| matches!(c, Component::Normal(_))),
        "secret file path must stay inside its workspace"
    );
    Ok(path.to_path_buf())
}

pub fn selected_mappings(
    contents: &[u8],
    mode: &str,
    groups: &[String],
) -> Result<Vec<FileMapping>> {
    ensure!(
        contents.len() <= 1024 * 1024,
        "secret workspace exceeds 1 MiB"
    );
    ensure!(
        !mode.is_empty()
            && mode
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')),
        "invalid secret file mode name"
    );
    let workspace: Workspace =
        toml::from_slice(contents).map_err(|_| anyhow::anyhow!("invalid secret file workspace"))?;
    ensure!(
        workspace.version == 3,
        "secret files require workspace version 3"
    );
    ensure!(
        workspace.env.modes.iter().any(|m| m == mode)
            || workspace.env.default_mode.as_deref() == Some(mode),
        "undeclared secret file mode"
    );
    ensure!(!groups.is_empty(), "select at least one file group");
    let mut mappings = Vec::new();
    for group in groups.iter().collect::<BTreeSet<_>>() {
        ensure!(
            !group.is_empty()
                && group
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')),
            "invalid secret file group"
        );
        let files = &workspace
            .file_groups
            .get(group)
            .context("unknown secret file group")?
            .files;
        ensure!(!files.is_empty(), "empty secret file group");
        for file in files {
            let source = file.source.replace("{mode}", mode);
            let target = file.target.replace("{mode}", mode);
            relative_path(&source)?;
            relative_path(&target)?;
            mappings.push(FileMapping { source, target });
        }
    }
    ensure!(mappings.len() <= 128, "too many secret files");
    Ok(mappings)
}

pub(super) fn resolve_declarations(
    snapshot: &PresetSnapshot,
    category: &str,
    version: u32,
    platform: RuntimePlatform,
    declarations: &[SecretFileDeclaration],
    files: &mut Vec<AppFile>,
    requires_admin: bool,
) -> Result<Vec<AppSecretFile>> {
    ensure!(
        declarations.is_empty() || version == 3,
        "secret_files requires App metadata version 3"
    );
    let prefix = format!("app/{category}/");
    let mut secrets = Vec::new();
    let mut auxiliary = BTreeSet::new();
    for declaration in declarations {
        let workspace = relative_path(&declaration.workspace)?;
        let parent = workspace.parent().context("workspace parent required")?;
        let bytes = snapshot
            .get(&format!(
                "{prefix}{}",
                crate::runtime::path_text::preset_path(&workspace)
            ))
            .context("secret workspace not found in preset snapshot")?;
        auxiliary.insert(workspace.clone());
        let permissions = declaration.permissions(platform)?;
        for mapping in selected_mappings(
            bytes,
            &declaration.mode,
            std::slice::from_ref(&declaration.file_group),
        )? {
            let source = parent.join(relative_path(&mapping.source)?);
            let target = parent.join(relative_path(&mapping.target)?);
            ensure!(
                !snapshot
                    .files()
                    .keys()
                    .filter_map(|p| p.strip_prefix(&prefix))
                    .any(|p| path_key(Path::new(p)) == path_key(&target)),
                "plaintext secret target is present in the preset source; remove it before installing or packing"
            );
            let cipher = snapshot
                .get(&format!(
                    "{prefix}{}",
                    crate::runtime::path_text::preset_path(&source)
                ))
                .context("secret ciphertext not found in preset snapshot")?;
            ensure!(
                cipher.len() <= MAX_CIPHER,
                "secret ciphertext exceeds 32 MiB"
            );
            auxiliary.insert(source.clone());
            secrets.push(AppSecretFile {
                workspace: workspace.clone(),
                group: declaration.file_group.clone(),
                mode: declaration.mode.clone(),
                source,
                target,
                auto_decrypt: declaration.auto_decrypt,
                permissions: permissions.clone(),
            });
        }
    }
    // Auxiliary files have exactly the same relative layout in the installed workspace.
    for path in auxiliary {
        if files
            .iter()
            .any(|f| f.target_rel == path || f.source_rel == path)
        {
            bail!("secret auxiliary file must not also be declared as an ordinary App file");
        }
        files.push(AppFile {
            source_rel: path.clone(),
            target_rel: path,
            destination_root: None,
            description: Some("Secret workspace/ciphertext".into()),
            display_name: None,
            legacy_dest_annotation: None,
            transforms: Vec::new(),
            install_strategy: AppInstallStrategy::Copy,
            requires_admin,
            restart_hint: None,
            generator: None,
        });
    }
    let mut paths = files
        .iter()
        .map(|f| f.target_rel.clone())
        .collect::<Vec<_>>();
    for secret in &secrets {
        for path in &paths {
            let a = super::secret_files::path_key(path);
            let b = super::secret_files::path_key(&secret.target);
            ensure!(
                !a.starts_with(&b) && !b.starts_with(&a),
                "secret file target overlaps another resource"
            );
        }
        paths.push(secret.target.clone());
    }
    Ok(secrets)
}

pub fn path_key(path: &Path) -> PathBuf {
    // Match the workspace's conservative case and Unicode alias policy.
    use unicode_normalization::UnicodeNormalization;
    PathBuf::from(
        path.to_string_lossy()
            .nfd()
            .flat_map(char::to_lowercase)
            .collect::<String>(),
    )
}

/// One immutable release request, shared by App preparation and explicit export.
/// Contains ciphertext only; the response must never be serialized through this type.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileReleaseRequest {
    pub operation: String,
    pub workspace_path: String,
    pub workspace_contents: String,
    pub mode: String,
    pub groups: Vec<String>,
    pub release: Vec<String>,
    pub ciphertexts: BTreeMap<String, String>,
    pub permissions: BTreeMap<String, SecretFilePermissions>,
    pub app: Option<AppReleaseBinding>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppReleaseBinding {
    pub category: String,
    pub metadata: String,
    pub snapshot_sha256: String,
}

impl FileReleaseRequest {
    pub fn released_mappings(&self) -> Result<Vec<FileMapping>> {
        let mappings = self.mappings()?;
        let release = self.release.iter().collect::<BTreeSet<_>>();
        ensure!(
            !release.is_empty()
                && release.len() == self.release.len()
                && release
                    .iter()
                    .all(|s| mappings.iter().any(|m| &m.source == *s)),
            "invalid file release selection"
        );
        Ok(mappings
            .into_iter()
            .filter(|m| release.contains(&m.source))
            .collect())
    }
    pub fn mappings(&self) -> Result<Vec<FileMapping>> {
        ensure!(
            matches!(self.operation.as_str(), "export" | "install" | "upgrade"),
            "invalid file release operation"
        );
        ensure!(
            self.workspace_path.len() <= 4096 && !self.workspace_path.chars().any(char::is_control),
            "invalid remote workspace path"
        );
        let workspace = Path::new(&self.workspace_path);
        ensure!(
            workspace.is_absolute()
                && workspace
                    .components()
                    .all(|c| !matches!(c, Component::ParentDir)),
            "remote workspace must be absolute"
        );
        ensure!(self.groups.len() <= 128, "too many file groups");
        let mappings =
            selected_mappings(self.workspace_contents.as_bytes(), &self.mode, &self.groups)?;
        let sources = mappings
            .iter()
            .map(|m| m.source.clone())
            .collect::<BTreeSet<_>>();
        ensure!(
            sources == self.ciphertexts.keys().cloned().collect(),
            "file ciphertext set does not match mappings"
        );
        let mut total = 0usize;
        let mut paths = vec![relative_path(
            workspace
                .file_name()
                .context("workspace filename required")?
                .to_str()
                .context("workspace filename must be UTF-8")?,
        )?];
        for mapping in &mappings {
            let cipher = &self.ciphertexts[&mapping.source];
            ensure!(
                !cipher.is_empty() && cipher.len() <= MAX_CIPHER,
                "invalid ciphertext size"
            );
            total += cipher.len();
            for value in [&mapping.source, &mapping.target] {
                let key = path_key(&relative_path(value)?);
                ensure!(
                    paths
                        .iter()
                        .all(|p| !path_key(p).starts_with(&key) && !key.starts_with(path_key(p))),
                    "overlapping file release paths"
                );
                paths.push(key);
            }
        }
        ensure!(
            total <= MAX_CIPHER_TOTAL,
            "file ciphertext total exceeds limit"
        );
        if self.operation == "export" {
            ensure!(
                self.app.is_none() && self.permissions.is_empty(),
                "export cannot claim App authority"
            );
        } else {
            let app = self.app.as_ref().context("App release binding required")?;
            ensure!(
                !app.category.is_empty()
                    && app.category.len() <= 128
                    && app
                        .category
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')),
                "invalid App category"
            );
            ensure!(
                app.metadata.len() <= 1024 * 1024
                    && app.snapshot_sha256.len() == 64
                    && app.snapshot_sha256.bytes().all(|c| c.is_ascii_hexdigit()),
                "invalid App snapshot binding"
            );
            ensure!(
                self.permissions.keys().cloned().collect::<BTreeSet<_>>()
                    == mappings.iter().map(|m| m.target.clone()).collect(),
                "App permission set does not match file mappings"
            );
            ensure!(
                self.permissions
                    .values()
                    .all(|p| matches!(p.mode, 0o400 | 0o600)),
                "invalid secret file permissions"
            );
        }
        Ok(mappings)
    }

    /// Public, reviewable binding. Ciphertext hashes are safe; plaintext hashes are not.
    pub fn descriptor(&self) -> Result<serde_json::Value> {
        let mappings = self.mappings()?;
        self.released_mappings()?;
        Ok(serde_json::json!({
            "operation": self.operation, "workspace": self.workspace_path,
            "workspace_sha256": crate::digest::sha256_hex(self.workspace_contents.as_bytes()),
            "mode": self.mode, "groups": self.groups, "mappings": mappings, "release": self.release,
            "ciphertexts": self.ciphertexts.iter().map(|(k,v)| (k.clone(), crate::digest::sha256_hex(v.as_bytes()))).collect::<BTreeMap<_,_>>(),
            "permissions": self.permissions, "app": self.app,
        }))
    }
}

pub fn keyed_fingerprint(key: &[u8], bytes: &[u8]) -> Result<String> {
    use hmac::{Hmac, Mac};
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(key).context("invalid fingerprint key")?;
    mac.update(b"shine-app-secret-file-v1\0");
    mac.update(bytes);
    Ok(mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> FileReleaseRequest {
        FileReleaseRequest { operation: "export".into(), workspace_path: "/srv/service/shine.workspace.toml".into(),
            workspace_contents: "version=3\n[env]\nmodes=['prod']\n[[file_groups.keys.files]]\nsource='keys.enc'\ntarget='private/keys.bin'\n".into(),
            mode: "prod".into(), groups: vec!["keys".into()], release: vec!["keys.enc".into()],
            ciphertexts: BTreeMap::from([("keys.enc".into(), "ciphertext".into())]), permissions: BTreeMap::new(), app: None }
    }
    #[test]
    fn plaintext_source_alias_is_rejected_before_auxiliary_install() {
        for target in ["private/keys.bin", "PRIVATE/KEYS.BIN"] {
            let req = request();
            let snapshot = PresetSnapshot::builder(crate::runtime::PresetSourceKind::Embedded)
                .file(
                    "app/demo/shine.workspace.toml",
                    req.workspace_contents.into_bytes(),
                )
                .file("app/demo/keys.enc", b"cipher".to_vec())
                .file(format!("app/demo/{target}"), b"must-not-cache".to_vec())
                .build();
            let declaration = SecretFileDeclaration {
                workspace: "shine.workspace.toml".into(),
                file_group: "keys".into(),
                mode: "prod".into(),
                auto_decrypt: true,
                unix_uid: None,
                unix_gid: None,
                unix_mode: None,
            };
            let result = resolve_declarations(
                &snapshot,
                "demo",
                3,
                RuntimePlatform::Linux,
                &[declaration],
                &mut Vec::new(),
                false,
            );
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("plaintext secret target")
            );
        }
    }
    #[test]
    fn secret_file_request_rejects_extra_sources_and_path_aliases() {
        let valid = request();
        assert!(valid.descriptor().is_ok());
        let mut extra = valid.clone();
        extra.ciphertexts.insert("extra".into(), "opaque".into());
        assert!(extra.descriptor().is_err());
        for target in [
            "../escape",
            "/absolute",
            "C:\\absolute",
            "KEYS.ENC",
            "shine.workspace.toml",
            "private/{unknown}",
        ] {
            let mut invalid = valid.clone();
            invalid.workspace_contents = invalid
                .workspace_contents
                .replace("private/keys.bin", target);
            assert!(invalid.descriptor().is_err(), "{target}");
        }
    }
    #[test]
    fn secret_file_release_is_explicit_and_permissions_are_scoped() {
        let mut invalid = request();
        invalid.release.clear();
        assert!(invalid.descriptor().is_err());
        invalid.release = vec!["other.enc".into()];
        assert!(invalid.descriptor().is_err());
        invalid = request();
        invalid.permissions.insert(
            "private/keys.bin".into(),
            SecretFilePermissions {
                uid: None,
                gid: None,
                mode: 0o600,
            },
        );
        assert!(invalid.descriptor().is_err());
        invalid = request();
        invalid.operation = "install".into();
        assert!(invalid.descriptor().is_err());
    }
    #[test]
    fn secret_permissions_reject_broad_modes_and_windows_unix_fields() {
        let mut declaration = SecretFileDeclaration {
            workspace: "shine.workspace.toml".into(),
            file_group: "keys".into(),
            mode: "prod".into(),
            auto_decrypt: false,
            unix_uid: None,
            unix_gid: None,
            unix_mode: None,
        };
        assert_eq!(
            declaration
                .permissions(RuntimePlatform::Linux)
                .unwrap()
                .mode,
            0o600
        );
        for mode in ["0644", "0777", "04700", "invalid"] {
            declaration.unix_mode = Some(mode.into());
            assert!(declaration.permissions(RuntimePlatform::Linux).is_err());
        }
        declaration.unix_mode = Some("0400".into());
        assert!(declaration.permissions(RuntimePlatform::Windows).is_err());
        declaration.unix_mode = None;
        assert!(declaration.permissions(RuntimePlatform::Windows).is_ok());
    }
    #[test]
    fn secret_fingerprint_is_keyed_and_binary_exact() {
        assert_ne!(
            keyed_fingerprint(b"one", b"password").unwrap(),
            keyed_fingerprint(b"two", b"password").unwrap()
        );
        assert_ne!(
            keyed_fingerprint(b"one", b"password").unwrap(),
            keyed_fingerprint(b"one", b"password\0").unwrap()
        );
    }
}
