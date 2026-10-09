//! Sys support.

use super::*;

pub(super) fn validate_sys_bootstrap_request(request: &SysBootstrapPlanRequest) -> Result<()> {
    if request.os_id.is_empty()
        || request.os_id.contains(['/', '\\'])
        || request.os_id.contains("..")
    {
        bail!("invalid Sys bootstrap os id");
    }
    if request.sys_shell.is_empty() {
        bail!("Sys bootstrap shell identity must not be empty");
    }
    Ok(())
}

pub(super) fn validate_sys_profile_request(request: &SysProfilePlanRequest) -> Result<()> {
    if request.os_id.is_empty()
        || request.os_id.contains(['/', '\\'])
        || request.os_id.contains("..")
    {
        bail!("invalid Sys profile os id");
    }
    if request.item_id.is_empty()
        || request.item_id.contains(['/', '\\'])
        || request.item_id.contains("..")
    {
        bail!("invalid Sys profile item id");
    }
    Ok(())
}

pub(super) fn capture_proxy_env(
    context: &crate::runtime::RuntimeContext,
    state: &mut StateCapture,
) -> Result<()> {
    for (name, value) in &context.proxy_env {
        state.public(
            format!("proxy-env:{name}"),
            format!("plain:{}", sha256_hex(value.as_bytes())),
        )?;
    }
    Ok(())
}

pub(super) async fn observe_sys_detection<H: FileSystemObservationHost>(
    runtime: &CoreRuntime<H>,
    detection: &SysDetection,
    state: &mut StateCapture,
    target: &str,
    permissions: &mut PermissionAccumulator,
) -> Result<bool> {
    match detection {
        SysDetection::Command {
            command,
            version_args,
        } => {
            if !version_args.is_empty() {
                permissions.implicit(PermissionV1::Command {
                    program: command.clone(),
                });
            }
            observe_command_presence(runtime, command, state, target).await
        }
        SysDetection::Path { path } => {
            let resolved = captured_sys_path(path, &runtime.context().home_dir)?;
            observe_presence(
                runtime.host(),
                state,
                format!("detection:{target}:path"),
                &resolved,
            )
            .await
        }
        SysDetection::Any { probes } => {
            let mut present = false;
            for (index, probe) in probes.iter().enumerate() {
                let found = match probe {
                    SysDetectionProbe::Command { command } => {
                        observe_command_presence(
                            runtime,
                            command,
                            state,
                            &format!("{target}:probe:{index}"),
                        )
                        .await?
                    }
                    SysDetectionProbe::Path { path } => {
                        let resolved = captured_sys_path(path, &runtime.context().home_dir)?;
                        observe_presence(
                            runtime.host(),
                            state,
                            format!("detection:{target}:probe:{index}:path"),
                            &resolved,
                        )
                        .await?
                    }
                };
                present |= found;
            }
            Ok(present)
        }
    }
}

pub(super) async fn observe_command_presence<H: FileSystemObservationHost>(
    runtime: &CoreRuntime<H>,
    command: &str,
    state: &mut StateCapture,
    target: &str,
) -> Result<bool> {
    let mut present = false;
    for (index, candidate) in command_candidates(runtime.context(), command)
        .into_iter()
        .enumerate()
    {
        let observation = observe_command_candidate(runtime.host(), &candidate)
            .await
            .map_err(|error| error.into_anyhow("observing Sys detection command"))?;
        let mut value = observation.source.as_ref().map_or_else(
            || "missing".to_string(),
            |metadata| {
                format!(
                    "{:?}:{}:{}",
                    metadata.kind,
                    metadata.len,
                    metadata.unix_mode.unwrap_or_default()
                )
            },
        );
        if observation
            .source
            .as_ref()
            .is_some_and(|metadata| metadata.kind == FileKind::Symlink)
        {
            match &observation.resolved {
                Some(resolved) => value.push_str(&format!(
                    ":resolved:{}:{:?}:{}:{}",
                    sha256_hex(resolved.path.as_os_str().as_encoded_bytes()),
                    resolved.metadata.kind,
                    resolved.metadata.len,
                    resolved.metadata.unix_mode.unwrap_or_default(),
                )),
                None => value.push_str(":resolved:missing"),
            }
        }
        state.public(format!("detection:{target}:candidate:{index}"), value)?;
        present |= observation.is_executable(runtime.context().platform);
    }
    Ok(present)
}

pub(super) async fn observe_presence(
    host: &impl FileSystemObservationHost,
    state: &mut StateCapture,
    label: String,
    path: &Path,
) -> Result<bool> {
    match host.metadata(path).await {
        Ok(metadata) => {
            state.public(
                label,
                format!(
                    "{:?}:{}:{}",
                    metadata.kind,
                    metadata.len,
                    metadata.unix_mode.unwrap_or_default()
                ),
            )?;
            Ok(true)
        }
        Err(error) if error.is_not_found() => {
            state.public(label, "missing")?;
            Ok(false)
        }
        Err(error) => Err(error.into_anyhow("observing Sys detection path")),
    }
}

pub(super) fn sys_bootstrap_code_blocked<H>(
    runtime: &CoreRuntime<H>,
    os_id: &str,
    item: &SysItem,
) -> Result<bool> {
    let Some(SysInstall::Script { path, .. }) = &item.install else {
        return Ok(false);
    };
    let logical = format!("sys/{os_id}/{}", path.replace('\\', "/"));
    if runtime
        .presets()
        .origin(&logical)
        .is_none_or(|origin| origin.source_kind == crate::runtime::PresetSourceKind::Embedded)
    {
        return Ok(false);
    }
    Ok(!runtime.sys_capability_trusted(os_id, item, TrustCapabilityV1::SysBootstrapScript)?)
}

pub(super) fn sys_profile_code_blocked<H>(
    runtime: &CoreRuntime<H>,
    os_id: &str,
    manifest: &SysManifest,
    selected: &[&SysItem],
    run_manifest: &SysRunManifest,
) -> Result<bool> {
    let enabled = run_manifest
        .entries
        .iter()
        .filter(|entry| entry.os_id == os_id && !entry.managed && entry.profile_enabled)
        .map(|entry| entry.item_id.as_str())
        .chain(selected.iter().map(|item| item.id.as_str()))
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    sys_profile_code_blocked_for_enabled(runtime, os_id, manifest, &enabled)
}

pub(super) fn sys_profile_code_blocked_for_enabled<H>(
    runtime: &CoreRuntime<H>,
    os_id: &str,
    manifest: &SysManifest,
    enabled: &BTreeSet<String>,
) -> Result<bool> {
    for item in manifest
        .items
        .iter()
        .filter(|item| enabled.contains(&item.id))
    {
        if !runtime.sys_capability_trusted(os_id, item, TrustCapabilityV1::SysProfileCode)? {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn sys_item_has_executable_profile_code(item: &SysItem) -> bool {
    item.shell.iter().any(|integration| {
        !integration.eval_argv.is_empty()
            || integration.source.is_some()
            || integration.fragment.is_some()
    })
}

pub(super) fn sys_profile_base_code_present<H>(runtime: &CoreRuntime<H>, os_id: &str) -> bool {
    let ext = if os_id == "windows" { "ps1" } else { "sh" };
    ["pre", "post"].into_iter().any(|phase| {
        runtime
            .presets()
            .get(&format!("sys/{os_id}/profile/base.{phase}.{ext}"))
            .is_some()
    })
}

pub(super) async fn capture_sys_profile_state<H: FileSystemObservationHost>(
    runtime: &CoreRuntime<H>,
    os_id: &str,
    sys_shell: &str,
    state: &mut StateCapture,
    permissions: &mut PermissionAccumulator,
) -> Result<()> {
    let ext = if os_id == "windows" { "ps1" } else { "sh" };
    for phase in ["pre", "post"] {
        let path = runtime
            .context()
            .home_dir
            .join(".shine/profile")
            .join(format!("{os_id}.{phase}.{ext}"));
        capture_path_state(runtime.host(), state, format!("profile:{phase}"), &path).await?;
        add_shine_write_permission(
            runtime.context(),
            permissions,
            &path,
            FilesystemPurposeV1::Installation,
            "sys/profile",
        );
    }
    for (index, path) in runtime.context().shell_config_paths.iter().enumerate() {
        capture_path_state(
            runtime.host(),
            state,
            format!("shell-profile:{sys_shell}:{index}"),
            path,
        )
        .await?;
        add_shine_write_permission(
            runtime.context(),
            permissions,
            path,
            FilesystemPurposeV1::UserTarget,
            &review_path(runtime.context(), path),
        );
    }
    permissions.implicit(PermissionV1::Command {
        program: "git".to_string(),
    });
    Ok(())
}

pub(super) fn sys_profile_block_paths(
    context: &crate::runtime::RuntimeContext,
    os_id: &str,
) -> Vec<PathBuf> {
    match os_id {
        "macos" => vec![context.home_dir.join(".zshrc")],
        "ubuntu" => vec![
            context.home_dir.join(".bashrc"),
            context.home_dir.join(".zshrc"),
        ],
        "windows" => vec![
            context
                .home_dir
                .join("Documents/PowerShell/Microsoft.PowerShell_profile.ps1"),
            context
                .home_dir
                .join("Documents/WindowsPowerShell/Microsoft.PowerShell_profile.ps1"),
        ],
        _ => context.shell_config_paths.clone(),
    }
}

pub(super) fn validate_sys_request(request: &SysManagedPlanRequest) -> Result<()> {
    if request.os_id.is_empty()
        || request.os_id.contains(['/', '\\'])
        || request.os_id.contains("..")
    {
        bail!("invalid managed Sys os id");
    }
    Ok(())
}

pub(super) async fn sys_receipt_modified<H: FileSystemObservationHost + SplitDnsObservationHost>(
    runtime: &CoreRuntime<H>,
    receipt: &SystemReceipt,
    state: &mut StateCapture,
    target: &str,
) -> Result<bool> {
    match receipt {
        SystemReceipt::ManagedFile(receipt) => {
            capture_path_state(
                runtime.host(),
                state,
                format!("receipt-resource:{target}"),
                &receipt.destination,
            )
            .await?;
            let current = read_optional(runtime.host(), &receipt.destination).await?;
            Ok(current
                .as_deref()
                .map(crate::install::hash_content)
                .is_some_and(|hash| hash != receipt.content_hash))
        }
        SystemReceipt::SplitDns(receipt) => {
            let request = SplitDnsRequest {
                os_id: receipt.os_id.clone(),
                item_id: receipt.item_id.clone(),
                domain: receipt.domain.clone(),
                servers: receipt.servers.clone(),
                resource: PathBuf::from(&receipt.resource),
                content: Vec::new(),
            };
            let observed = runtime.host().inspect_split_dns(&request).await?;
            state.bytes(
                format!("receipt-resource:{target}"),
                observed.exists.then_some(observed.content.as_slice()),
            )?;
            Ok(observed.exists
                && !String::from_utf8_lossy(&observed.content)
                    .contains(&format!("split-dns:{}", receipt.item_id)))
        }
        SystemReceipt::Script { .. } => Ok(true),
    }
}

pub(super) async fn sys_item_current<H: FileSystemObservationHost + SplitDnsObservationHost>(
    runtime: &CoreRuntime<H>,
    os_id: &str,
    item: &SysItem,
    previous: Option<&SystemReceipt>,
    state: &mut StateCapture,
    target: &str,
    permissions: &mut PermissionAccumulator,
) -> Result<bool> {
    match item.driver {
        SysDriverKind::SplitDns => {
            permissions.implicit(PermissionV1::System {
                capability: "split-dns".to_string(),
                resource: Some("private-domain".to_string()),
            });
            let domain_key = sys_config_string(&item.config, "domain_env")?;
            let servers_key = sys_config_string(&item.config, "servers_env")?;
            let desired = split_dns_receipt(&crate::runtime::SplitDnsDomainRequest {
                os_id: os_id.to_string(),
                item_id: item.id.clone(),
                domain: runtime
                    .context()
                    .env
                    .get(&domain_key)
                    .cloned()
                    .context("missing split DNS domain")?,
                servers: runtime
                    .context()
                    .env
                    .get(&servers_key)
                    .cloned()
                    .context("missing split DNS servers")?,
                dry_run: true,
            })?;
            let request = SplitDnsRequest {
                os_id: desired.os_id.clone(),
                item_id: desired.item_id.clone(),
                domain: desired.domain.clone(),
                servers: desired.servers.clone(),
                resource: PathBuf::from(&desired.resource),
                content: split_dns_content_for_plan(&desired),
            };
            let observed = runtime.host().inspect_split_dns(&request).await?;
            state.bytes(
                format!("resource:{target}"),
                observed.exists.then_some(observed.content.as_slice()),
            )?;
            Ok(
                previous.is_some_and(|previous| split_dns_receipt_matches(previous, &desired))
                    && observed.exists
                    && observed.content == request.content,
            )
        }
        SysDriverKind::ManagedFile => {
            let source = sys_config_string(&item.config, "source")?;
            let logical = format!("sys/{os_id}/{}", source.trim_start_matches('/'));
            let raw = runtime
                .presets()
                .get(&logical)
                .with_context(|| format!("missing {logical}"))?;
            let transforms = item
                .config
                .get("transforms")
                .and_then(toml::Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(toml::Value::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let desired =
                crate::install::transforms::apply(&transforms, raw, &runtime.context().env)?;
            let destination = captured_sys_path(
                &sys_config_string(&item.config, "target")?,
                &runtime.context().home_dir,
            )?;
            permissions.implicit(PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: review_path(runtime.context(), &destination),
            });
            capture_path_state(
                runtime.host(),
                state,
                format!("resource:{target}"),
                &destination,
            )
            .await?;
            let current = read_optional(runtime.host(), &destination).await?;
            let desired_hash = crate::install::hash_content(&desired);
            Ok(
                matches!(previous, Some(SystemReceipt::ManagedFile(receipt)) if receipt.destination == destination && receipt.content_hash == desired_hash)
                    && current.as_deref() == Some(desired.as_slice()),
            )
        }
        SysDriverKind::Script => Ok(false),
    }
}

pub(super) fn split_dns_content_for_plan(receipt: &crate::runtime::SplitDnsReceipt) -> Vec<u8> {
    let marker = format!("Managed by shine: split-dns:{}", receipt.item_id);
    match receipt.os_id.as_str() {
        "macos" => format!(
            "# {marker}\n{}\n",
            receipt
                .servers
                .iter()
                .map(|server| format!("nameserver {server}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
        .into_bytes(),
        "ubuntu" => format!(
            "# {marker}\n[Resolve]\nDNS={}\nDomains=~{}\n",
            receipt.servers.join(" "),
            receipt.domain
        )
        .into_bytes(),
        _ => format!(
            "{marker}\n{}\n{}",
            receipt.resource,
            receipt.servers.join(",")
        )
        .into_bytes(),
    }
}

pub(super) fn sys_config_string(config: &toml::Table, key: &str) -> Result<String> {
    config
        .get(key)
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .with_context(|| format!("managed Sys config `{key}` must be a string"))
}

pub(super) fn captured_sys_path(raw: &str, home: &Path) -> Result<PathBuf> {
    if raw == "$HOME" || raw == "~" {
        return Ok(home.to_path_buf());
    }
    if let Some(rest) = raw
        .strip_prefix("$HOME/")
        .or_else(|| raw.strip_prefix("~/"))
    {
        if rest
            .split(['/', '\\'])
            .any(|part| matches!(part, "" | "." | ".."))
        {
            bail!("invalid managed Sys target");
        }
        return Ok(home.join(rest));
    }
    let path = PathBuf::from(raw);
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        bail!("managed Sys target must be absolute or HOME-relative");
    }
    Ok(path)
}

pub(super) fn split_dns_receipt_matches(
    previous: &SystemReceipt,
    desired: &crate::runtime::SplitDnsReceipt,
) -> bool {
    matches!(previous, SystemReceipt::SplitDns(previous)
        if previous.version == desired.version
            && previous.os_id == desired.os_id
            && previous.item_id == desired.item_id
            && previous.domain == desired.domain
            && previous.servers == desired.servers
            && previous.resource == desired.resource)
}
