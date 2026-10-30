//! Bounded binary-file release over the existing authenticated control channel.
use super::protocol;
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
pub use shine_core::runtime::secret_files::FileReleaseRequest;
use shine_core::runtime::secret_files::{MAX_FILE, MAX_TOTAL};
use tokio::io::{AsyncRead, AsyncWrite};
use zeroize::Zeroizing;

const CHUNK: usize = 256 * 1024;
pub const MAX_REQUEST: usize = 192 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FilePolicy {
    pub name: String,
    pub ssh_target: String,
    /// Canonical JSON containing source hashes and exact public release facts.
    pub descriptor: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Chunk {
    nonce: String,
    index: usize,
    offset: usize,
    length: usize,
    #[serde(
        serialize_with = "serialize_chunk_data",
        deserialize_with = "deserialize_chunk_data"
    )]
    data: Zeroizing<String>,
}
fn serialize_chunk_data<S: serde::Serializer>(
    data: &Zeroizing<String>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.serialize_str(data)
}
fn deserialize_chunk_data<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Zeroizing<String>, D::Error> {
    String::deserialize(deserializer).map(Zeroizing::new)
}

pub async fn send_blob<W: AsyncWrite + Unpin>(
    writer: &mut W,
    nonce: &str,
    index: usize,
    bytes: &[u8],
) -> Result<()> {
    if bytes.is_empty() {
        return protocol::write_message(
            writer,
            &Chunk {
                nonce: nonce.into(),
                index,
                offset: 0,
                length: 0,
                data: Zeroizing::new(String::new()),
            },
        )
        .await;
    }
    for (i, part) in bytes.chunks(CHUNK).enumerate() {
        protocol::write_message(
            writer,
            &Chunk {
                nonce: nonce.into(),
                index,
                offset: i * CHUNK,
                length: bytes.len(),
                data: Zeroizing::new(BASE64.encode(part)),
            },
        )
        .await?;
    }
    Ok(())
}

pub async fn receive_blob<R: AsyncRead + Unpin>(
    reader: &mut R,
    nonce: &str,
    index: usize,
    limit: usize,
) -> Result<Zeroizing<Vec<u8>>> {
    let mut bytes = Zeroizing::new(Vec::new());
    let mut length = None;
    loop {
        let chunk: Chunk = tokio::time::timeout(
            std::time::Duration::from_secs(300),
            protocol::read_message(reader),
        )
        .await
        .context("file broker timed out")??;
        ensure!(
            chunk.nonce == nonce && chunk.index == index && chunk.offset == bytes.len(),
            "file broker chunk association/order mismatch"
        );
        ensure!(
            chunk.length <= limit && chunk.data.len() <= CHUNK.div_ceil(3) * 4,
            "file broker size limit exceeded"
        );
        ensure!(
            length.is_none_or(|l| l == chunk.length),
            "file broker length changed"
        );
        length = Some(chunk.length);
        let mut decoded = Zeroizing::new(Vec::new());
        BASE64
            .decode_vec(chunk.data.as_bytes(), &mut decoded)
            .context("invalid file broker encoding")?;
        ensure!(
            decoded.len() <= CHUNK && decoded.len() <= chunk.length - bytes.len(),
            "invalid file broker chunk length"
        );
        ensure!(
            !decoded.is_empty() || chunk.length == 0,
            "empty intermediate file broker chunk"
        );
        bytes.extend_from_slice(&decoded);
        if bytes.len() == chunk.length {
            return Ok(bytes);
        }
    }
}

pub async fn decrypt(
    config: &crate::config::Config,
    request: &FileReleaseRequest,
) -> Result<Vec<Zeroizing<Vec<u8>>>> {
    let mappings = request.released_mappings()?;
    let mut config = config.clone();
    for cipher in request.ciphertexts.values() {
        crate::secret::validate_file_ciphertext(cipher)?;
    }
    if request
        .ciphertexts
        .values()
        .any(|v| v.starts_with("hybrid:"))
        && config.hybrid_decrypt_backend.is_none()
    {
        let backend =
            crate::secret::hybrid::select(&config.resolved_age_identities(), None).await?;
        config.hybrid_decrypt_backend = Some(
            match backend {
                crate::secret::BackendKind::Gpg => "gpg",
                _ => "age",
            }
            .into(),
        );
    }
    let mut total = 0;
    let mut result = Vec::new();
    for mapping in mappings {
        let bytes = crate::secret::decrypt_bytes(
            &request.ciphertexts[&mapping.source],
            &config,
            MAX_FILE.min(MAX_TOTAL - total),
        )
        .await
        .map_err(|_| anyhow::anyhow!("file decryption failed or was cancelled"))?;
        total += bytes.len();
        ensure!(total <= MAX_TOTAL, "plaintext total limit exceeded");
        result.push(bytes);
    }
    Ok(result)
}

#[cfg(unix)]
pub async fn request(
    request: FileReleaseRequest,
    describe: bool,
) -> Result<Vec<Zeroizing<Vec<u8>>>> {
    super::remote_client::request_files(request, describe).await
}
#[cfg(not(unix))]
pub async fn request(_: FileReleaseRequest, _: bool) -> Result<Vec<Zeroizing<Vec<u8>>>> {
    anyhow::bail!("file broker requires a POSIX remote")
}

pub async fn enabled() -> Result<bool> {
    if !super::broker_session_available() {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        super::remote_client::file_broker_enabled().await
    }
    #[cfg(not(unix))]
    {
        Ok(false)
    }
}

pub fn descriptor(request: &FileReleaseRequest) -> Result<String> {
    Ok(serde_json::to_string(&request.descriptor()?)?)
}

pub async fn describe(
    config: &crate::config::Config,
    workspace: Option<&std::path::Path>,
    mode: &str,
    groups: &[String],
    app: Option<&str>,
    operation: Option<&str>,
) -> Result<()> {
    let request = capture(config, workspace, mode, groups, app, operation, None).await?;
    if super::broker_session_available() {
        self::request(request, true).await?;
    } else {
        println!("{}", serde_json::to_string_pretty(&request.descriptor()?)?);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn capture(
    config: &crate::config::Config,
    workspace: Option<&std::path::Path>,
    mode: &str,
    groups: &[String],
    app: Option<&str>,
    operation: Option<&str>,
    remote: Option<&str>,
) -> Result<FileReleaseRequest> {
    if let Some(category) = app {
        use shine_core::runtime::{CoreRuntime, RealHost, RuntimePlatform};
        let runtime = crate::core_runtime::from_config(config).await?;
        let platforms = if remote.is_some() {
            vec![RuntimePlatform::Linux, RuntimePlatform::Macos]
        } else {
            vec![runtime.context().platform]
        };
        let mut candidates = Vec::new();
        for platform in platforms {
            let mut context = runtime.context().clone();
            context.platform = platform;
            let runtime = CoreRuntime::new(RealHost, context, runtime.presets().clone());
            for category in runtime.app_categories(Some(category))? {
                for request in
                    runtime.app_file_release_requests(&category, operation.unwrap_or("install"))?
                {
                    if request.mode == mode
                        && request
                            .groups
                            .iter()
                            .collect::<std::collections::BTreeSet<_>>()
                            == groups.iter().collect()
                        && remote.is_none_or(|p| p == request.workspace_path)
                        && !candidates.contains(&request)
                    {
                        candidates.push(request);
                    }
                }
            }
        }
        ensure!(
            candidates.len() == 1,
            "file policy requires one matching App declaration and remote workspace"
        );
        let request = candidates.remove(0);
        if let Some(path) = workspace {
            ensure!(
                std::fs::read(path)? == request.workspace_contents.as_bytes(),
                "local workspace differs from the effective App snapshot"
            );
        }
        return Ok(request);
    }
    ensure!(
        operation.is_none_or(|s| s == "export"),
        "install/upgrade file policies require --app"
    );
    let mut request =
        crate::env::workspace::file_groups::capture_release(workspace, mode, groups).await?;
    if let Some(remote) = remote {
        request.workspace_path = remote.into();
    }
    request.mappings()?;
    Ok(request)
}

pub async fn save_policy(
    config: &crate::config::Config,
    input: &crate::commands::EnvBrokerPolicyInput,
    update: bool,
) -> Result<()> {
    use crate::env::broker;
    use shine_core::runtime::{FileSystemHost, RealHost};
    ensure!(
        !input.name.is_empty()
            && input
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')),
        "invalid file policy name"
    );
    ensure!(
        input.command.is_empty(),
        "file policies bind a typed operation; do not supply a child command"
    );
    let remote = input
        .remote_workspace
        .as_deref()
        .context("file policies require --remote-workspace")?;
    let request = capture(
        config,
        Some(&input.workspace),
        &input.mode,
        &input.file_groups,
        input.app.as_deref(),
        input.operation.as_deref(),
        Some(remote),
    )
    .await?;
    let candidate = FilePolicy {
        name: input.name.clone(),
        ssh_target: input.ssh_target.clone(),
        descriptor: descriptor(&request)?,
    };
    let original = broker::load_store(config).await?;
    ensure!(
        original.version == 2 || input.migrate_file_policy,
        "adding file policies requires --migrate-file-policy to explicitly migrate the store to version 2"
    );
    let previous = original
        .file_policies
        .iter()
        .find(|p| p.name == input.name)
        .cloned();
    ensure!(
        previous.is_some() == update,
        "file policy already exists or update target is missing"
    );
    if update {
        let details = format!(
            "Replace file policy {}?\nOld: {}\nNew: {}",
            input.name,
            previous
                .as_ref()
                .context("previous file policy missing")?
                .descriptor,
            candidate.descriptor
        );
        if !dialoguer::Confirm::new()
            .with_prompt(crate::output::escape_controls(&details))
            .default(false)
            .interact()?
        {
            anyhow::bail!("file policy update cancelled");
        }
    }
    let _lock = RealHost
        .acquire_operation_lock(&config.shine_dir().join("ssh-secret-broker.lock"))
        .await?;
    let mut current = broker::load_store(config).await?;
    ensure!(
        current.file_policies.iter().find(|p| p.name == input.name) == previous.as_ref(),
        "file policy changed after review"
    );
    current.version = 2;
    current.file_policies.retain(|p| p.name != input.name);
    current.file_policies.push(candidate);
    broker::store::save_store(config, &current).await?;
    println!("saved file policy {}", input.name);
    Ok(())
}

pub fn policy_matches(policy: &str, request: &str) -> bool {
    let (Ok(mut policy), Ok(mut request)) = (
        serde_json::from_str::<serde_json::Value>(policy),
        serde_json::from_str::<serde_json::Value>(request),
    ) else {
        return false;
    };
    let (Some(policy_release), Some(request_release)) = (
        policy.get("release").and_then(|v| v.as_array()).cloned(),
        request.get("release").and_then(|v| v.as_array()).cloned(),
    ) else {
        return false;
    };
    if request_release.is_empty() || !request_release.iter().all(|r| policy_release.contains(r)) {
        return false;
    }
    if let Some(p) = policy.as_object_mut() {
        p.remove("release");
    }
    if let Some(r) = request.as_object_mut() {
        r.remove("release");
    }
    policy == request
}

#[allow(clippy::too_many_arguments)]
pub async fn diff_policy(
    config: &crate::config::Config,
    name: &str,
    workspace: &std::path::Path,
    mode: &str,
    groups: &[String],
    app: Option<&str>,
    operation: Option<&str>,
) -> Result<()> {
    let store = crate::env::broker::load_store(config).await?;
    let existing = store
        .file_policies
        .iter()
        .find(|p| p.name == name)
        .context("file policy does not exist")?;
    let old: serde_json::Value = serde_json::from_str(&existing.descriptor)?;
    let remote = old
        .get("workspace")
        .and_then(|v| v.as_str())
        .context("file policy workspace missing")?;
    let candidate = capture(
        config,
        Some(workspace),
        mode,
        groups,
        app,
        operation,
        Some(remote),
    )
    .await?;
    let new = candidate.descriptor()?;
    println!(
        "{}",
        crate::output::colorize_unified_diff(&crate::output::unified_text_diff(
            &serde_json::to_string_pretty(&old)?,
            &serde_json::to_string_pretty(&new)?,
            None
        ))
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn binary_chunks_preserve_empty_and_large_files() {
        for bytes in [Vec::new(), vec![0xff; CHUNK * 9 + 1]] {
            let mut wire = Vec::new();
            send_blob(&mut wire, "nonce", 2, &bytes).await.unwrap();
            let received = receive_blob(&mut std::io::Cursor::new(wire), "nonce", 2, MAX_FILE)
                .await
                .unwrap();
            assert_eq!(*received, bytes);
        }
    }
    #[tokio::test]
    async fn rejects_wrong_association_and_limits() {
        let mut wire = Vec::new();
        send_blob(&mut wire, "nonce", 0, b"test").await.unwrap();
        for (nonce, index, limit) in [("other", 0, 10), ("nonce", 1, 10), ("nonce", 0, 3)] {
            assert!(
                receive_blob(&mut std::io::Cursor::new(&wire), nonce, index, limit)
                    .await
                    .is_err()
            );
        }
    }

    #[test]
    fn file_policy_allows_only_explicit_subset_of_unchanged_bindings() {
        let policy = serde_json::json!({"workspace":"/srv/workspace.toml", "mappings":["one","two"], "ciphertexts":{"one":"hash1","two":"hash2"}, "release":["one","two"], "operation":"install", "permissions":{"mode":"0400"}});
        let mut request = policy.clone();
        request["release"] = serde_json::json!(["two"]);
        assert!(policy_matches(&policy.to_string(), &request.to_string()));
        for (key, value) in [
            ("operation", serde_json::json!("upgrade")),
            ("workspace", serde_json::json!("/other/workspace.toml")),
            (
                "ciphertexts",
                serde_json::json!({"one":"changed","two":"hash2"}),
            ),
            ("release", serde_json::json!(["new-file"])),
            ("permissions", serde_json::json!({"mode":"0644"})),
        ] {
            let mut changed = request.clone();
            changed[key] = value;
            assert!(!policy_matches(&policy.to_string(), &changed.to_string()));
        }
    }

    #[tokio::test]
    async fn file_chunks_reject_reordered_empty_and_changed_length_frames() {
        for chunk in [
            Chunk {
                nonce: "n".into(),
                index: 0,
                offset: 1,
                length: 2,
                data: Zeroizing::new("YQ==".into()),
            },
            Chunk {
                nonce: "n".into(),
                index: 0,
                offset: 0,
                length: 2,
                data: Zeroizing::new(String::new()),
            },
            Chunk {
                nonce: "n".into(),
                index: 0,
                offset: 0,
                length: 1,
                data: Zeroizing::new("YWJj".into()),
            },
        ] {
            let mut wire = Vec::new();
            protocol::write_message(&mut wire, &chunk).await.unwrap();
            assert!(
                receive_blob(&mut std::io::Cursor::new(wire), "n", 0, 10)
                    .await
                    .is_err()
            );
        }
        let mut wire = Vec::new();
        for (offset, length) in [(0, 2), (1, 3)] {
            protocol::write_message(
                &mut wire,
                &Chunk {
                    nonce: "n".into(),
                    index: 0,
                    offset,
                    length,
                    data: Zeroizing::new("YQ==".into()),
                },
            )
            .await
            .unwrap();
        }
        assert!(
            receive_blob(&mut std::io::Cursor::new(wire), "n", 0, 10)
                .await
                .is_err()
        );
    }
}
