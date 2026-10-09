//! Observations.

use super::*;

pub(super) fn add_sys_bootstrap_install_permissions<H>(
    runtime: &CoreRuntime<H>,
    os_id: &str,
    item: &SysItem,
    install: &SysInstall,
    permissions: &mut PermissionAccumulator,
    shared_permissions: &mut PermissionAccumulator,
) -> Result<()> {
    match install {
        SysInstall::Package { provider, .. } => {
            let program = match provider {
                SysPackageProvider::Homebrew | SysPackageProvider::HomebrewCask => "brew",
                SysPackageProvider::Apt => "apt-get",
                SysPackageProvider::Winget => "winget",
            };
            permissions.implicit(PermissionV1::Command {
                program: program.to_string(),
            });
            permissions.implicit(PermissionV1::Network {
                scope: NetworkScopeV1::Any,
            });
        }
        SysInstall::Script { path, .. } => {
            permissions.require(PermissionV1::Filesystem {
                access: FilesystemAccessV1::Execute,
                path: format!("preset:{}", path.replace('\\', "/")),
            });
            permissions.implicit(PermissionV1::Command {
                program: match os_id {
                    "windows" => "powershell.exe",
                    "macos" => "zsh",
                    _ => "bash",
                }
                .to_string(),
            });
            add_shine_write_permission(
                runtime.context(),
                shared_permissions,
                &runtime.context().shine_dir.join("runtime/sys").join(os_id),
                FilesystemPurposeV1::Maintenance,
                "sys/bootstrap",
            );
            shared_permissions.implicit_for(
                PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Remove,
                    path: review_path(
                        runtime.context(),
                        &runtime.context().shine_dir.join("runtime/sys").join(os_id),
                    ),
                },
                FilesystemPurposeV1::Maintenance,
                "sys/bootstrap",
            );
        }
    }
    if crate::runtime::sys_install_requires_admin(os_id, install, item)? {
        permissions.implicit(PermissionV1::Administrator);
    }
    for name in runtime.context().proxy_env.keys() {
        permissions.implicit(PermissionV1::Environment {
            name: name.clone(),
            sensitivity: EnvironmentSensitivityV1::Plain,
        });
    }
    Ok(())
}

pub(super) fn validate_app_request(request: &AppPlanRequest) -> Result<()> {
    if request.purge && request.operation != LifecycleOperation::Uninstall {
        bail!("App purge is valid only for uninstall Plans");
    }
    if request.prune_stale && request.operation != LifecycleOperation::Upgrade {
        bail!("App stale pruning is valid only for upgrade Plans");
    }
    if request.force
        && !matches!(
            request.operation,
            LifecycleOperation::Install | LifecycleOperation::Uninstall
        )
    {
        bail!("App force is valid only for install or uninstall Plans");
    }
    Ok(())
}

pub(super) fn validate_shell_request(request: &ShellPlanRequest) -> Result<()> {
    if request.purge && request.operation != LifecycleOperation::Uninstall {
        bail!("Shell purge is valid only for uninstall Plans");
    }
    if request.force && request.operation != LifecycleOperation::Install {
        bail!("Shell force is valid only for install Plans");
    }
    Ok(())
}

pub(super) fn capture_context(
    state: &mut StateCapture,
    context: &crate::runtime::RuntimeContext,
) -> Result<()> {
    state.public("platform", context.platform.as_str())?;
    state.public("external-presets", context.is_external_presets.to_string())?;
    state.public(
        "external-shell-mode",
        match context.external_shell_mode {
            ExternalShellMode::Snapshot => "snapshot",
            ExternalShellMode::Live => "live",
        },
    )?;
    state.public(
        "home",
        sha256_hex(context.home_dir.as_os_str().as_encoded_bytes()),
    )?;
    state.public(
        "shine",
        sha256_hex(context.shine_dir.as_os_str().as_encoded_bytes()),
    )?;
    state.public(
        "trust-grants",
        sha256_hex(&serde_json::to_vec(&context.trust_grants)?),
    )?;
    Ok(())
}

pub(super) fn capture_request_mode(
    state: &mut StateCapture,
    target: Option<&str>,
    force: bool,
    purge: bool,
    prune_stale: bool,
) -> Result<()> {
    state.public("target", target.unwrap_or("all"))?;
    state.public("force", force.to_string())?;
    state.public("purge", purge.to_string())?;
    state.public("prune-stale", prune_stale.to_string())?;
    Ok(())
}

pub(super) fn capture_manifest_selection<T: serde::Serialize>(
    state: &mut StateCapture,
    label: &str,
    present: bool,
    schema_version: u32,
    entries: &T,
) -> Result<()> {
    state.public(format!("{label}:present"), present.to_string())?;
    state.public(format!("{label}:schema"), schema_version.to_string())?;
    let encoded = serde_json::to_vec(entries)?;
    state.bytes(format!("{label}:entries"), Some(&encoded))
}

pub(super) async fn capture_path_state(
    host: &impl FileSystemObservationHost,
    state: &mut StateCapture,
    label: String,
    path: &Path,
) -> Result<()> {
    match host.metadata(path).await {
        Ok(metadata) => {
            let fingerprint = match metadata.kind {
                FileKind::File => read_optional(host, path)
                    .await?
                    .as_deref()
                    .map(sha256_hex)
                    .unwrap_or_else(|| "none".to_string()),
                FileKind::Symlink => host
                    .read_link(path)
                    .await
                    .map(|target| sha256_hex(target.as_os_str().as_encoded_bytes()))
                    .unwrap_or_else(|_| "unreadable".to_string()),
                FileKind::Directory => "none".to_string(),
            };
            let value = format!(
                "{:?}:{}:{}",
                metadata.kind,
                metadata.unix_mode.unwrap_or_default(),
                fingerprint
            );
            state.public(label, value)
        }
        Err(error) if error.is_not_found() => state.public(label, "missing"),
        Err(error) => Err(error.into_anyhow("observing planned resource")),
    }
}

pub(super) async fn capture_tree_state(
    host: &impl FileSystemObservationHost,
    state: &mut StateCapture,
    label: String,
    root: &Path,
) -> Result<()> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let relative = path.strip_prefix(root).unwrap_or(&path);
        let resource = if relative.as_os_str().is_empty() {
            "root".to_string()
        } else {
            logical_path(relative)
        };
        capture_path_state(host, state, format!("{label}:{resource}"), &path).await?;
        match host.metadata(&path).await {
            Ok(metadata) if metadata.kind == FileKind::Directory => {
                let mut children = host
                    .read_dir(&path)
                    .await
                    .map_err(|error| error.into_anyhow("observing planned resource tree"))?;
                children.sort();
                pending.extend(children.into_iter().rev());
            }
            Ok(_) => {}
            Err(error) if error.is_not_found() => {}
            Err(error) => return Err(error.into_anyhow("observing planned resource tree")),
        }
    }
    Ok(())
}

pub(super) async fn path_exists(
    host: &impl FileSystemObservationHost,
    path: &Path,
) -> Result<bool> {
    match host.metadata(path).await {
        Ok(_) => Ok(true),
        Err(error) if error.is_not_found() => Ok(false),
        Err(error) => Err(error.into_anyhow("observing planned resource")),
    }
}

pub(in crate::runtime) async fn shell_snapshot_tree_current(
    host: &impl FileSystemObservationHost,
    root: &Path,
    expected: &BTreeMap<PathBuf, (u64, bool)>,
) -> Result<bool> {
    let metadata = match host.metadata(root).await {
        Ok(metadata) => metadata,
        Err(error) if error.is_not_found() => return Ok(false),
        Err(error) => return Err(error.into_anyhow("observing planned Shell snapshot")),
    };
    if metadata.kind != FileKind::Directory {
        return Ok(false);
    }
    let mut actual = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let mut children = host
            .read_dir(&directory)
            .await
            .map_err(|error| error.into_anyhow("observing planned Shell snapshot tree"))?;
        children.sort();
        for path in children {
            let metadata = host
                .metadata(&path)
                .await
                .map_err(|error| error.into_anyhow("observing planned Shell snapshot entry"))?;
            match metadata.kind {
                FileKind::Directory => pending.push(path),
                FileKind::File => {
                    let bytes = host.read(&path).await.map_err(|error| {
                        error.into_anyhow("reading planned Shell snapshot entry")
                    })?;
                    actual.insert(
                        path.strip_prefix(root)
                            .context("planned Shell snapshot escaped its root")?
                            .to_path_buf(),
                        (
                            crate::install::hash_content(&bytes),
                            cfg!(unix) && metadata.unix_mode.is_some_and(|mode| mode & 0o111 != 0),
                        ),
                    );
                }
                FileKind::Symlink => return Ok(false),
            }
        }
    }
    Ok(actual == *expected)
}

pub(super) async fn read_optional(
    host: &impl FileSystemObservationHost,
    path: &Path,
) -> Result<Option<Vec<u8>>> {
    match host.read(path).await {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.is_not_found() => Ok(None),
        Err(error) => Err(error.into_anyhow("reading planned state")),
    }
}

pub(super) async fn load_app_manifest(
    host: &impl FileSystemObservationHost,
    shine_dir: &Path,
) -> Result<(AppManifest, Option<Vec<u8>>)> {
    let bytes = read_optional(host, &shine_dir.join("app-manifest.toml")).await?;
    let mut manifest: AppManifest = bytes
        .as_deref()
        .map(toml::from_slice)
        .transpose()?
        .unwrap_or_default();
    match manifest.schema_version {
        0 => manifest.schema_version = APP_MANIFEST_SCHEMA_VERSION,
        APP_MANIFEST_SCHEMA_VERSION => {}
        version => bail!(
            "app manifest schema version {version} is newer than this Shine supports ({APP_MANIFEST_SCHEMA_VERSION})"
        ),
    }
    Ok((manifest, bytes))
}

pub(super) async fn load_shell_manifest(
    host: &impl FileSystemObservationHost,
    shine_dir: &Path,
) -> Result<(ShellManifest, Option<Vec<u8>>)> {
    let bytes = read_optional(host, &shine_dir.join("shell-manifest.toml")).await?;
    let mut manifest: ShellManifest = bytes
        .as_deref()
        .map(toml::from_slice)
        .transpose()?
        .unwrap_or_default();
    crate::runtime::shell::normalize_shell_manifest(&mut manifest)?;
    Ok((manifest, bytes))
}

pub(super) async fn load_sys_manifest(
    host: &impl FileSystemObservationHost,
    shine_dir: &Path,
) -> Result<(SysRunManifest, Option<Vec<u8>>)> {
    let bytes = read_optional(host, &shine_dir.join("sys-manifest.toml")).await?;
    let mut manifest: SysRunManifest = bytes
        .as_deref()
        .map(toml::from_slice)
        .transpose()?
        .unwrap_or_default();
    match manifest.schema_version {
        0 => manifest.schema_version = crate::runtime::SYS_MANIFEST_SCHEMA_VERSION,
        crate::runtime::SYS_MANIFEST_SCHEMA_VERSION => {}
        version => bail!(
            "sys manifest schema version {version} is newer than this Shine supports ({})",
            crate::runtime::SYS_MANIFEST_SCHEMA_VERSION
        ),
    }
    Ok((manifest, bytes))
}

pub(super) fn shell_entry_selected(
    entry: &ShellManifestEntry,
    selection: Option<&crate::runtime::ShellTarget<'_>>,
) -> bool {
    selection.is_none_or(|target| {
        entry.category == target.category
            && target
                .command
                .is_none_or(|command| entry.command == command)
    })
}

pub(super) fn logical_app_source(category: &AppCategory, file: &AppFile) -> String {
    format!("app/{}/{}", category.name, logical_path(&file.source_rel))
}

pub(super) fn logical_app_source_for(category: &str, file: &AppFile) -> String {
    format!("app/{category}/{}", logical_path(&file.source_rel))
}

pub(super) fn app_source_parts(source: &str) -> Option<(&str, &str)> {
    let mut parts = source.splitn(3, '/');
    (parts.next()? == "app").then_some((parts.next()?, parts.next()?))
}

pub(super) fn operation_name(operation: PlanOperationV1) -> &'static str {
    operation.as_str()
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
