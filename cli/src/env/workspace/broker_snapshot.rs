//! Broker snapshot for workspace environments.
use super::sources;
use super::{
    BTreeMap, BTreeSet, Binding, Config, Context, Path, Result, SecretState, SensitiveValues,
    SourceFile, SourceSnapshot, WorkspaceSnapshot, Zeroize, bail, decrypt_source_payload,
    effective_bindings, find_workspace_optional, parse_source, parse_workspace, resolve_sources,
    validate_mode,
};

/// Reads one workspace/mode exactly once for SSH broker hashing and execution.
/// The returned bytes are retained by the remote runner until the child starts,
/// so a successful authorization never re-reads mutable files.
pub async fn snapshot_for_broker(
    workspace_arg: Option<&Path>,
    mode: &str,
) -> Result<WorkspaceSnapshot> {
    validate_mode(mode)?;
    let workspace_path = find_workspace_optional(workspace_arg)
        .await?
        .context("shine.workspace.toml was not found; pass --workspace")?;
    let workspace_contents = tokio::fs::read_to_string(&workspace_path)
        .await
        .with_context(|| format!("reading {}", workspace_path.display()))?;
    let workspace = parse_workspace(&workspace_path, &workspace_contents)?;
    let source_paths = resolve_sources(&workspace_path, &workspace.env.files, mode)?;
    let root = workspace_path
        .parent()
        .context("workspace path has no parent directory")?;
    let mut sources = Vec::new();
    for source_path in source_paths {
        if !source_path.is_file() {
            continue;
        }
        let contents = tokio::fs::read_to_string(&source_path)
            .await
            .with_context(|| format!("reading {}", source_path.display()))?;
        // Parse now so malformed/unsealed metadata never reaches the broker.
        let source = parse_source(&source_path, &contents)?;
        for (key, state) in &source.secret {
            if !matches!(state, SecretState::Sealed(true)) {
                bail!(
                    "{key} in {} is not sealed; run `shine env secret seal`",
                    source_path.display()
                );
            }
        }
        let display_path = source_path
            .strip_prefix(root)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| source_path.clone())
            .to_string_lossy()
            .into_owned();
        sources.push(SourceSnapshot {
            path: display_path,
            contents,
        });
    }
    if sources.is_empty() {
        bail!("none of the configured environment source files exist");
    }
    Ok(WorkspaceSnapshot {
        workspace_path: workspace_path.to_string_lossy().into_owned(),
        workspace_contents,
        mode: mode.to_string(),
        override_process_env: workspace.env.override_process_env,
        sources,
    })
}

pub(crate) fn declared_secrets_from_source(path: &str, contents: &str) -> Result<Vec<String>> {
    let source = parse_source(Path::new(path), contents)?;
    let mut keys = source.secret.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    Ok(keys)
}

pub(super) fn parse_broker_sources(snapshot: &WorkspaceSnapshot) -> Result<Vec<SourceFile>> {
    snapshot
        .sources
        .iter()
        .map(|source| parse_source(Path::new(&source.path), &source.contents))
        .collect()
}

pub fn plain_values_from_broker_snapshot(
    snapshot: &WorkspaceSnapshot,
) -> Result<BTreeMap<String, String>> {
    let sources = parse_broker_sources(snapshot)?;
    let bindings = effective_bindings(&sources);
    Ok(sources::plain_values(&sources, &bindings).into_map())
}

pub(super) fn merge_broker_values(
    snapshot: &WorkspaceSnapshot,
    secrets: BTreeMap<String, String>,
) -> Result<SensitiveValues> {
    let secrets = SensitiveValues::from(secrets);
    let sources = parse_broker_sources(snapshot)?;
    let bindings = effective_bindings(&sources);
    let mut values = sources::plain_values(&sources, &bindings);
    // Older brokers can return a secret shadowed by later plain data.
    for (key, mut value) in secrets {
        if matches!(bindings.get(&key), Some(Binding::Secret(_))) {
            values.insert(key, value);
        } else {
            value.zeroize();
        }
    }
    Ok(values)
}

pub async fn decrypt_broker_snapshot(
    config: &Config,
    snapshot: &WorkspaceSnapshot,
    release: &[String],
) -> Result<BTreeMap<String, String>> {
    let release = release.iter().cloned().collect::<BTreeSet<_>>();
    let parsed_sources = parse_broker_sources(snapshot)?;
    let declared = parsed_sources
        .iter()
        .flat_map(|source| source.secret.keys().cloned())
        .collect::<BTreeSet<_>>();
    if !release.is_subset(&declared) {
        bail!("broker release contains an undeclared secret key");
    }
    let bindings = effective_bindings(&parsed_sources);
    let expected_release = release
        .iter()
        .filter(|key| matches!(bindings.get(*key), Some(Binding::Secret(_))))
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut values = SensitiveValues::default();
    for (index, (source, parsed)) in snapshot.sources.iter().zip(&parsed_sources).enumerate() {
        let path = Path::new(&source.path);
        for (key, state) in &parsed.secret {
            if !matches!(state, SecretState::Sealed(true)) {
                bail!("{key} in {} is not sealed", source.path);
            }
        }
        let secrets = decrypt_source_payload(path, parsed, config).await?;
        let expected = parsed.secret.keys().cloned().collect::<BTreeSet<_>>();
        let actual = secrets.keys().cloned().collect::<BTreeSet<_>>();
        if expected != actual {
            bail!(
                "secret key list does not match encrypted payload in {}",
                source.path
            );
        }
        for (key, mut value) in secrets {
            if release.contains(&key) && bindings.get(&key) == Some(&Binding::Secret(index)) {
                values.insert(key, value);
            } else {
                value.zeroize();
            }
        }
    }
    if values.keys().cloned().collect::<BTreeSet<_>>() != expected_release {
        bail!("broker response does not contain every released secret key");
    }
    Ok(values.into_map())
}
