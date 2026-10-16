use crate::runtime::{
    BunDependencyMode, FileKind, FileSystemHost, FileSystemObservationHost, LinkRuntime,
};
use anyhow::Result;
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

const LINKABLE_SCRIPT_EXTENSIONS: &[&str] = &["sh", "bash", "zsh", "fish", "ps1"];

/// Script extensions runnable through the `bun` runtime launcher.
const BUN_SCRIPT_EXTENSIONS: &[&str] = &["ts", "js", "mts", "mjs"];

#[cfg(not(unix))]
const EXECUTABLE_EXTENSIONS: &[&str] = &["sh", "ps1"];

// Marker lines identifying a shine-managed launcher. Unix and PowerShell use `#`;
// cmd uses its native `REM` form. Ownership and current-ness detection accept both.
const SHIM_MANAGED_MARKER: &str = "# shine-managed";
const CMD_SHIM_MANAGED_MARKER: &str = "REM shine-managed";
const SHIM_TARGET_PREFIX: &str = "# shine-target: ";

/// Runtime used to invoke a linked command.
///
/// `Native` is the historical behavior: a Unix symlink or a Windows bash/PowerShell
/// shim pointing directly at the script. `Bun` wraps the script in a generated
/// launcher that runs `bun <script> "$@"` — a real regular file on Unix (not a
/// symlink) carrying the managed marker, and a Bun shim on Windows.
/// Whether an existing on-disk launcher/shim is a current, stale, or foreign file.
///
/// Shared by the Unix bun-launcher path and the Windows shim path. `NotManaged`
/// protects user files: it means the file lacks the managed marker (or points at a
/// different source), so it is treated as a conflict, never silently replaced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LauncherStatus {
    Current,
    Stale,
    NotManaged,
}

pub struct LinkReport {
    pub created: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
    pub conflicts: Vec<LinkConflict>,
    pub overwritten: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkConflictKind {
    ExistingEntry,
    DuplicateName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkConflict {
    pub link_path: PathBuf,
    pub source: PathBuf,
    pub kind: LinkConflictKind,
}

pub struct UnlinkReport {
    pub removed: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
}

pub(crate) struct ManagedLauncherProbe {
    pub resources: Vec<PreparedLauncherResource>,
    pub conflicts: Vec<PathBuf>,
}

#[derive(Clone)]
pub struct LinkSpec {
    /// Corrected native CMD path format; false reconstructs legacy receipt bytes.
    pub native_cmd_literal_path: bool,
    /// Explicit installation root for the single-process Live Bun format.
    pub live_launch_config: Option<PathBuf>,
    pub source: PathBuf,
    pub link_name: OsString,
    pub runtime: LinkRuntime,
    /// Whether Bun may resolve locked third-party dependencies for this entry.
    pub bun_dependencies: BunDependencyMode,
    /// For `LinkRuntime::Bun`: ordered `--with` argument tokens (`KEY` or
    /// `SOURCE=TARGET`) injected at launch through `shine env run`. When empty,
    /// the launcher invokes Bun directly without a `shine env run` dependency.
    pub env: Vec<String>,
    /// Canonical installed target to lazily render before execution in external live mode.
    pub render_target: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PreparedLauncherResource {
    Symlink {
        destination: PathBuf,
        target: PathBuf,
    },
    File {
        destination: PathBuf,
        bytes: Vec<u8>,
        unix_mode: Option<u32>,
    },
}

impl PreparedLauncherResource {
    pub(crate) fn destination(&self) -> &Path {
        match self {
            Self::Symlink { destination, .. } | Self::File { destination, .. } => destination,
        }
    }
}

pub(crate) fn prepare_launcher_resources(
    bin_dir: &Path,
    spec: &LinkSpec,
) -> Vec<PreparedLauncherResource> {
    let link_path = command_path_for_name(bin_dir, &spec.link_name);
    #[cfg(unix)]
    {
        let content =
            if let (Some(config), Some(target)) = (&spec.live_launch_config, &spec.render_target) {
                Some(unix_live_bun_launch_content(&spec.source, config, target))
            } else if let Some(target) = spec.render_target.as_deref() {
                Some(unix_live_launcher_content(
                    &spec.source,
                    &launcher_command_name(&link_path),
                    spec.runtime,
                    spec.bun_dependencies,
                    &spec.env,
                    target,
                ))
            } else if spec.runtime == LinkRuntime::Bun {
                Some(unix_bun_launcher_content(
                    &spec.source,
                    &launcher_command_name(&link_path),
                    spec.bun_dependencies,
                    &spec.env,
                ))
            } else {
                None
            };
        match content {
            Some(content) => vec![PreparedLauncherResource::File {
                destination: link_path,
                bytes: content.into_bytes(),
                unix_mode: Some(0o755),
            }],
            None => vec![PreparedLauncherResource::Symlink {
                destination: link_path,
                target: spec.source.clone(),
            }],
        }
    }
    #[cfg(not(unix))]
    {
        let name = launcher_command_name(&link_path);
        let ps1 =
            if let (Some(config), Some(target)) = (&spec.live_launch_config, &spec.render_target) {
                powershell_live_bun_launch_content(&spec.source, config, target)
            } else {
                powershell_shim_content(
                    &spec.source,
                    spec.runtime,
                    &name,
                    spec.bun_dependencies,
                    &spec.env,
                    spec.render_target.as_deref(),
                )
            };
        let cmd =
            if let (Some(config), Some(target)) = (&spec.live_launch_config, &spec.render_target) {
                cmd_live_bun_launch_content(&spec.source, config, target)
            } else {
                let content = cmd_shim_content(
                    &spec.source,
                    spec.runtime,
                    &name,
                    spec.bun_dependencies,
                    &spec.env,
                    spec.render_target.as_deref(),
                );
                if spec.native_cmd_literal_path {
                    correct_native_cmd_path(content, &spec.source)
                } else {
                    content
                }
            };
        vec![
            PreparedLauncherResource::File {
                destination: link_path.clone(),
                bytes: ps1.into_bytes(),
                unix_mode: None,
            },
            PreparedLauncherResource::File {
                destination: link_path.with_extension("cmd"),
                bytes: cmd.into_bytes(),
                unix_mode: None,
            },
        ]
    }
}

pub(crate) async fn apply_prepared_launcher_resource(
    host: &impl FileSystemHost,
    resource: &PreparedLauncherResource,
) -> Result<()> {
    let destination = resource.destination();
    if let Some(parent) = destination.parent() {
        host.create_dir_all(parent)
            .await
            .map_err(|error| error.into_anyhow("creating shell launcher directory"))?;
    }
    match resource {
        PreparedLauncherResource::Symlink {
            destination,
            target,
        } => host
            .symlink(target, destination)
            .await
            .map_err(|error| error.into_anyhow("creating shell symlink")),
        PreparedLauncherResource::File {
            destination,
            bytes,
            unix_mode,
        } => {
            host.write_atomic(destination, bytes)
                .await
                .map_err(|error| error.into_anyhow("writing shell launcher"))?;
            if let Some(mode) = unix_mode {
                host.set_mode(destination, *mode)
                    .await
                    .map_err(|error| error.into_anyhow("setting shell launcher permissions"))?;
            }
            Ok(())
        }
    }
}

pub(crate) async fn prepared_launcher_resource_is_exact(
    host: &impl FileSystemObservationHost,
    resource: &PreparedLauncherResource,
) -> Result<bool> {
    let metadata = match host.metadata(resource.destination()).await {
        Ok(metadata) => metadata,
        Err(error) if error.is_not_found() => return Ok(false),
        Err(error) => return Err(error.into_anyhow("observing prepared Shell launcher")),
    };
    Ok(match resource {
        PreparedLauncherResource::Symlink { target, .. } => {
            metadata.kind == FileKind::Symlink
                && host
                    .read_link(resource.destination())
                    .await
                    .is_ok_and(|current| current == *target)
        }
        PreparedLauncherResource::File {
            bytes, unix_mode, ..
        } => {
            metadata.kind == FileKind::File
                && host
                    .read(resource.destination())
                    .await
                    .is_ok_and(|current| current == *bytes)
                && unix_mode.is_none_or(|expected| {
                    metadata
                        .unix_mode
                        .is_some_and(|mode| mode & 0o777 == expected)
                })
        }
    })
}

async fn host_remove_link(host: &impl FileSystemHost, link_path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        host.remove_file(link_path)
            .await
            .map_err(|error| error.into_anyhow("removing existing shell launcher"))
    }
    #[cfg(not(unix))]
    {
        for path in [link_path.to_path_buf(), link_path.with_extension("cmd")] {
            match host.remove_file(&path).await {
                Ok(()) => {}
                Err(error) if error.is_not_found() => {}
                Err(error) => {
                    return Err(error.into_anyhow("removing existing shell shim"));
                }
            }
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
async fn host_create_link(
    host: &impl FileSystemHost,
    source: &Path,
    link_path: &Path,
    runtime: LinkRuntime,
    bun_dependencies: BunDependencyMode,
    env: &[String],
    render_target: Option<&str>,
    live_launch_config: Option<&Path>,
) -> Result<()> {
    let spec = LinkSpec {
        native_cmd_literal_path: uses_native_cmd_literal_path(runtime, source),
        live_launch_config: live_launch_config.map(Path::to_path_buf),

        source: source.to_path_buf(),
        link_name: {
            #[cfg(unix)]
            let name = link_path.file_name();
            #[cfg(not(unix))]
            let name = link_path.file_stem();
            name.unwrap_or_default().to_os_string()
        },
        runtime,
        bun_dependencies,
        env: env.to_vec(),
        render_target: render_target.map(str::to_string),
    };
    for resource in
        prepare_launcher_resources(link_path.parent().unwrap_or_else(|| Path::new("")), &spec)
    {
        apply_prepared_launcher_resource(host, &resource).await?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn host_launcher_status(
    host: &impl FileSystemObservationHost,
    link_path: &Path,
    source: &Path,
    runtime: LinkRuntime,
    bun_dependencies: BunDependencyMode,
    env: &[String],
    render_target: Option<&str>,
    live_launch_config: Option<&Path>,
) -> Result<LauncherStatus> {
    let content = match host.read(link_path).await {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(content) => content,
            Err(_) => return Ok(LauncherStatus::NotManaged),
        },
        Err(_) => return Ok(LauncherStatus::NotManaged),
    };
    if !has_shim_managed_marker(&content) {
        return Ok(LauncherStatus::NotManaged);
    }
    let Some(target) = shim_target_from_content(&content) else {
        return Ok(LauncherStatus::Stale);
    };
    #[cfg(unix)]
    if target.as_os_str() != source.as_os_str() {
        return Ok(LauncherStatus::NotManaged);
    }
    #[cfg(not(unix))]
    if windows_path_key(&target) != windows_path_key(source) {
        return Ok(LauncherStatus::NotManaged);
    }

    if let Some(config) = live_launch_config {
        let spec = LinkSpec {
            source: source.to_path_buf(),
            link_name: launcher_command_name(link_path).into(),
            runtime,
            bun_dependencies,
            env: env.to_vec(),
            render_target: render_target.map(str::to_string),
            native_cmd_literal_path: false,
            live_launch_config: Some(config.to_path_buf()),
        };
        let resources =
            prepare_launcher_resources(link_path.parent().unwrap_or_else(|| Path::new("")), &spec);
        for resource in resources {
            if !prepared_launcher_resource_is_exact(host, &resource).await? {
                return Ok(LauncherStatus::Stale);
            }
        }
        return Ok(LauncherStatus::Current);
    }
    let name = launcher_command_name(link_path);
    #[cfg(unix)]
    let current = if let Some(target) = render_target {
        content == unix_live_launcher_content(source, &name, runtime, bun_dependencies, env, target)
    } else {
        runtime == LinkRuntime::Bun
            && content == unix_bun_launcher_content(source, &name, bun_dependencies, env)
    };
    #[cfg(not(unix))]
    let current = {
        let expected_ps1 =
            powershell_shim_content(source, runtime, &name, bun_dependencies, env, render_target);
        let mut expected_cmd =
            cmd_shim_content(source, runtime, &name, bun_dependencies, env, render_target);
        if uses_native_cmd_literal_path(runtime, source) {
            expected_cmd = correct_native_cmd_path(expected_cmd, source);
        }
        let cmd_current = host
            .read(&link_path.with_extension("cmd"))
            .await
            .ok()
            .is_some_and(|bytes| bytes == expected_cmd.as_bytes());
        content == expected_ps1 && cmd_current
    };
    Ok(if current {
        LauncherStatus::Current
    } else {
        LauncherStatus::Stale
    })
}

async fn host_source_is_linkable(host: &impl FileSystemHost, path: &Path) -> bool {
    if has_linkable_script_extension(path) {
        return true;
    }
    #[cfg(unix)]
    return host
        .metadata(path)
        .await
        .ok()
        .and_then(|metadata| metadata.unix_mode)
        .is_some_and(|mode| mode & 0o111 != 0);
    #[cfg(not(unix))]
    return path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| EXECUTABLE_EXTENSIONS.contains(&extension));
}

pub async fn link_executables_with_host(
    host: &impl FileSystemHost,
    bin_dir: &Path,
    specs: &[LinkSpec],
    overwrite: bool,
) -> Result<LinkReport> {
    let mut report = LinkReport {
        created: Vec::new(),
        skipped: Vec::new(),
        conflicts: Vec::new(),
        overwritten: Vec::new(),
    };
    let mut seen = HashSet::new();
    for spec in specs {
        if spec.runtime == LinkRuntime::Native && !host_source_is_linkable(host, &spec.source).await
        {
            continue;
        }
        if spec.source.file_name().is_none() {
            continue;
        }
        if !seen.insert(spec.link_name.clone()) {
            report.conflicts.push(LinkConflict {
                link_path: command_path_for_name(bin_dir, &spec.link_name),
                source: spec.source.clone(),
                kind: LinkConflictKind::DuplicateName,
            });
            continue;
        }
        let link_path = command_path_for_name(bin_dir, &spec.link_name);
        match host.metadata(&link_path).await {
            Ok(metadata) if metadata.kind == FileKind::Symlink => {
                let current = host
                    .read_link(&link_path)
                    .await
                    .is_ok_and(|target| target == spec.source && spec.render_target.is_none());
                if current {
                    report.skipped.push(link_path);
                } else if overwrite {
                    host_remove_link(host, &link_path).await?;
                    host_create_link(
                        host,
                        &spec.source,
                        &link_path,
                        spec.runtime,
                        spec.bun_dependencies,
                        &spec.env,
                        spec.render_target.as_deref(),
                        spec.live_launch_config.as_deref(),
                    )
                    .await?;
                    report.overwritten.push(link_path);
                } else {
                    report.conflicts.push(LinkConflict {
                        link_path,
                        source: spec.source.clone(),
                        kind: LinkConflictKind::ExistingEntry,
                    });
                }
            }
            Ok(_) => match host_launcher_status(
                host,
                &link_path,
                &spec.source,
                spec.runtime,
                spec.bun_dependencies,
                &spec.env,
                spec.render_target.as_deref(),
                spec.live_launch_config.as_deref(),
            )
            .await?
            {
                LauncherStatus::Current => report.skipped.push(link_path),
                LauncherStatus::Stale => {
                    host_remove_link(host, &link_path).await?;
                    host_create_link(
                        host,
                        &spec.source,
                        &link_path,
                        spec.runtime,
                        spec.bun_dependencies,
                        &spec.env,
                        spec.render_target.as_deref(),
                        spec.live_launch_config.as_deref(),
                    )
                    .await?;
                    report.overwritten.push(link_path);
                }
                LauncherStatus::NotManaged if overwrite => {
                    host_remove_link(host, &link_path).await?;
                    host_create_link(
                        host,
                        &spec.source,
                        &link_path,
                        spec.runtime,
                        spec.bun_dependencies,
                        &spec.env,
                        spec.render_target.as_deref(),
                        spec.live_launch_config.as_deref(),
                    )
                    .await?;
                    report.overwritten.push(link_path);
                }
                LauncherStatus::NotManaged => report.conflicts.push(LinkConflict {
                    link_path,
                    source: spec.source.clone(),
                    kind: LinkConflictKind::ExistingEntry,
                }),
            },
            Err(error) if error.is_not_found() => {
                host_create_link(
                    host,
                    &spec.source,
                    &link_path,
                    spec.runtime,
                    spec.bun_dependencies,
                    &spec.env,
                    spec.render_target.as_deref(),
                    spec.live_launch_config.as_deref(),
                )
                .await?;
                report.created.push(link_path);
            }
            Err(error) => return Err(error.into_anyhow("inspecting shell launcher")),
        }
    }
    Ok(report)
}

pub async fn link_is_current_with_host(
    host: &impl FileSystemObservationHost,
    link_path: &Path,
    source: &Path,
    runtime: LinkRuntime,
    bun_dependencies: BunDependencyMode,
    env: &[String],
    render_target: Option<&str>,
) -> Result<bool> {
    match host.metadata(link_path).await {
        Ok(metadata) if metadata.kind == FileKind::Symlink => {
            if runtime != LinkRuntime::Native || render_target.is_some() {
                return Ok(false);
            }
            Ok(host
                .read_link(link_path)
                .await
                .is_ok_and(|target| target == source))
        }
        Ok(_) => Ok(matches!(
            host_launcher_status(
                host,
                link_path,
                source,
                runtime,
                bun_dependencies,
                env,
                render_target,
                None,
            )
            .await?,
            LauncherStatus::Current
        )),
        Err(error) if error.is_not_found() => Ok(false),
        Err(error) => Err(error.into_anyhow("inspecting shell launcher")),
    }
}

pub async fn unlink_managed_command_with_host(
    host: &impl FileSystemHost,
    bin_dir: &Path,
    command: &OsStr,
    managed_roots: &[PathBuf],
    dry_run: bool,
) -> Result<UnlinkReport> {
    let probe = probe_managed_command_with_host(host, bin_dir, command, managed_roots).await?;
    if !probe.conflicts.is_empty() {
        return Ok(UnlinkReport {
            removed: Vec::new(),
            skipped: probe.conflicts,
        });
    }
    let removed = probe
        .resources
        .iter()
        .map(|resource| resource.destination().to_path_buf())
        .collect::<Vec<_>>();
    if !dry_run {
        for path in &removed {
            host.remove_file(path)
                .await
                .map_err(|error| error.into_anyhow("removing managed shell launcher"))?;
        }
    }
    Ok(UnlinkReport {
        removed,
        skipped: Vec::new(),
    })
}

pub(crate) async fn probe_managed_command_with_host(
    host: &impl FileSystemObservationHost,
    bin_dir: &Path,
    command: &OsStr,
    managed_roots: &[PathBuf],
) -> Result<ManagedLauncherProbe> {
    let primary = command_path_for_name(bin_dir, command);
    #[cfg(unix)]
    let candidates = vec![primary];
    #[cfg(not(unix))]
    let candidates = vec![primary.clone(), primary.with_extension("cmd")];

    let mut resources = Vec::new();
    let mut conflicts = Vec::new();
    for path in candidates {
        let metadata = match host.metadata(&path).await {
            Ok(metadata) => metadata,
            Err(error) if error.is_not_found() => continue,
            Err(error) => return Err(error.into_anyhow("inspecting shell launcher")),
        };
        let (target, resource) = if metadata.kind == FileKind::Symlink {
            let target = host
                .read_link(&path)
                .await
                .map_err(|error| error.into_anyhow("reading shell launcher target"))?;
            (
                target.clone(),
                PreparedLauncherResource::Symlink {
                    destination: path.clone(),
                    target,
                },
            )
        } else if metadata.kind == FileKind::File {
            let bytes = host
                .read(&path)
                .await
                .map_err(|error| error.into_anyhow("reading shell launcher"))?;
            let target = String::from_utf8(bytes.clone())
                .ok()
                .filter(|content| has_shim_managed_marker(content))
                .and_then(|content| shim_target_from_content(&content));
            let Some(target) = target else {
                conflicts.push(path);
                continue;
            };
            (
                target,
                PreparedLauncherResource::File {
                    destination: path.clone(),
                    bytes,
                    unix_mode: metadata.unix_mode.map(|mode| mode & 0o777),
                },
            )
        } else {
            conflicts.push(path);
            continue;
        };
        if managed_roots
            .iter()
            .any(|root| target_is_managed(&target, root, bin_dir))
        {
            resources.push(resource);
        } else {
            conflicts.push(path);
        }
    }
    if !conflicts.is_empty() {
        resources.clear();
    }
    Ok(ManagedLauncherProbe {
        resources,
        conflicts,
    })
}

pub fn command_path_for_name(bin_dir: &Path, stem: &OsStr) -> PathBuf {
    #[cfg(unix)]
    {
        bin_dir.join(stem)
    }
    #[cfg(not(unix))]
    {
        let mut name = stem.to_os_string();
        name.push(".ps1");
        bin_dir.join(name)
    }
}

pub fn link_stem(path: &Path) -> std::ffi::OsString {
    if has_linkable_script_extension(path) || has_bun_script_extension(path) {
        path.file_stem().map(|s| s.to_owned()).unwrap_or_default()
    } else {
        path.file_name().map(|n| n.to_owned()).unwrap_or_default()
    }
}

fn has_linkable_script_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| LINKABLE_SCRIPT_EXTENSIONS.contains(&ext))
        .unwrap_or(false)
}

fn has_bun_script_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| BUN_SCRIPT_EXTENSIONS.contains(&ext))
        .unwrap_or(false)
}

/// True when `target` (from a launcher's native target marker or a symlink) lexically
/// resolves under `managed_root`. Relative targets are resolved against `bin_dir`.
/// Works even if the target file no longer exists.
fn target_is_managed(target: &Path, managed_root: &Path, bin_dir: &Path) -> bool {
    if target.is_absolute() {
        target.starts_with(managed_root)
    } else {
        bin_dir.join(target).starts_with(managed_root)
    }
}

fn has_shim_managed_marker(content: &str) -> bool {
    content.contains(SHIM_MANAGED_MARKER) || content.contains(CMD_SHIM_MANAGED_MARKER)
}

/// The command name a launcher exposes — the link path's file stem.
fn launcher_command_name(link_path: &Path) -> String {
    link_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn shim_target_from_content(content: &str) -> Option<PathBuf> {
    content.lines().find_map(|line| {
        line.strip_prefix(SHIM_TARGET_PREFIX)
            .or_else(|| line.strip_prefix("REM shine-target: "))
            .map(PathBuf::from)
    })
}

#[cfg(unix)]
fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Deterministic content of a Unix bun launcher. Regenerated byte-for-byte by
/// `host_launcher_status` to detect staleness, so any change here is a format
/// change that will refresh installed launchers on upgrade.
#[cfg(unix)]
fn unix_bun_launcher_content(
    source: &Path,
    name: &str,
    bun_dependencies: BunDependencyMode,
    env: &[String],
) -> String {
    let target = source.display().to_string();
    let quoted_target = shell_single_quote(&target);
    let quoted_name = shell_single_quote(name);
    let install_arg = bun_dependencies.install_arg();
    // Empty `env` reproduces the v1 launcher byte-for-byte (no `shine` dependency);
    // a declared `env` adds a `shine` presence check and runs the child through
    // `shine env run --no-workspace` so values reach Bun via `Bun.env`.
    let (shine_check, runner) = if env.is_empty() {
        (
            String::new(),
            format!("exec bun {install_arg} {quoted_target} \"$@\"\n"),
        )
    } else {
        let with_args = env
            .iter()
            .map(|token| format!("--with {}", shell_single_quote(token)))
            .collect::<Vec<_>>()
            .join(" ");
        (
            format!(
                "if ! command -v shine >/dev/null 2>&1; then\n  \
                 printf 'shine: %s requires the shine command, which was not found on PATH.\\n' {quoted_name} >&2\n  \
                 exit 127\nfi\n"
            ),
            format!(
                "exec shine env run --no-workspace {with_args} -- bun {install_arg} {quoted_target} \"$@\"\n"
            ),
        )
    };
    format!(
        "#!/usr/bin/env bash\n\
         {SHIM_MANAGED_MARKER}\n\
         {SHIM_TARGET_PREFIX}{target}\n\
         if ! command -v bun >/dev/null 2>&1; then\n  \
         printf 'shine: %s requires Bun, which was not found on PATH.\\n' {quoted_name} >&2\n  \
         printf 'shine: install Bun from https://bun.sh, then re-run %s.\\n' {quoted_name} >&2\n  \
         exit 127\nfi\n\
         {shine_check}{runner}"
    )
}

#[cfg(unix)]
fn unix_live_launcher_content(
    source: &Path,
    name: &str,
    runtime: LinkRuntime,
    bun_dependencies: BunDependencyMode,
    env: &[String],
    render_target: &str,
) -> String {
    let target = source.display().to_string();
    let quoted_source = shell_single_quote(&target);
    let quoted_name = shell_single_quote(name);
    let quoted_render_target = shell_single_quote(render_target);
    let config_dir = live_config_dir(source);
    let config_arg = if config_dir.file_name() == Some(OsStr::new(".shine")) {
        String::new()
    } else {
        format!(
            "--config-dir {} ",
            shell_single_quote(&config_dir.display().to_string())
        )
    };
    let render = format!(
        "if ! command -v shine >/dev/null 2>&1; then\n  \
         printf 'shine: %s requires the shine command, which was not found on PATH.\\n' {quoted_name} >&2\n  \
         return 127 2>/dev/null || exit 127\nfi\n\
         shine {config_arg}__shell-render {quoted_render_target} || {{ _shine_code=$?; return $_shine_code 2>/dev/null || exit $_shine_code; }}\n"
    );
    let runner = match runtime {
        LinkRuntime::Native => format!(
            "_shine_sourced=false\n\
             case \"$ZSH_EVAL_CONTEXT\" in *:file|*:file:*) _shine_sourced=true ;; esac\n\
             if [ -n \"$BASH_VERSION\" ] && [ \"$BASH_SOURCE\" != \"$0\" ]; then _shine_sourced=true; fi\n\
             if [ \"$_shine_sourced\" = true ]; then\n  . {quoted_source} \"$@\"\n  return $?\nfi\n\
             exec {quoted_source} \"$@\"\n"
        ),
        LinkRuntime::Bun => {
            let install_arg = bun_dependencies.install_arg();
            let bun_check = format!(
                "if ! command -v bun >/dev/null 2>&1; then\n  \
                 printf 'shine: %s requires Bun, which was not found on PATH.\\n' {quoted_name} >&2\n  \
                 exit 127\nfi\n"
            );
            if env.is_empty() {
                format!("{bun_check}exec bun {install_arg} {quoted_source} \"$@\"\n")
            } else {
                let with_args = env
                    .iter()
                    .map(|token| format!("--with {}", shell_single_quote(token)))
                    .collect::<Vec<_>>()
                    .join(" ");
                format!(
                    "{bun_check}exec shine env run --no-workspace {with_args} -- bun {install_arg} {quoted_source} \"$@\"\n"
                )
            }
        }
    };
    format!(
        "#!/usr/bin/env bash\n{SHIM_MANAGED_MARKER}\n{SHIM_TARGET_PREFIX}{target}\n{render}{runner}"
    )
}

#[cfg(unix)]
fn unix_live_bun_launch_content(source: &Path, config: &Path, target: &str) -> String {
    format!(
        "#!/usr/bin/env bash\n{SHIM_MANAGED_MARKER}\n{SHIM_TARGET_PREFIX}{}\n\
         if ! command -v shine >/dev/null 2>&1; then\n  printf 'shine: live command requires shine on PATH.\\n' >&2\n  exit 127\nfi\n\
         exec shine --config-dir {} __shell-launch {} -- \"$@\"\n",
        source.display(),
        shell_single_quote(&config.display().to_string()),
        shell_single_quote(target),
    )
}

#[cfg(any(not(unix), test))]
fn powershell_live_bun_launch_content(source: &Path, config: &Path, target: &str) -> String {
    let config = windows_native_path(config).replace('\'', "''");
    let target = target.replace('\'', "''");
    format!(
        // PowerShell 5.1's native binder drops empty args and consumes `--`.
        // Encode the Windows argv explicitly and inherit the caller's stdio.
        r#"{SHIM_MANAGED_MARKER}
{SHIM_TARGET_PREFIX}{}
$shine = Get-Command shine -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $shine) {{ exit 127 }}
function ConvertTo-ShineArgument([string]$value) {{
  $escaped = [regex]::Replace($value, '(\\*)"', '$1$1\"')
  $escaped = [regex]::Replace($escaped, '(\\+)$', '$1$1')
  return '"' + $escaped + '"'
}}
$argv = @('--config-dir', '{config}', '__shell-launch', '{target}', '--') + @($args)
$start = New-Object System.Diagnostics.ProcessStartInfo
$start.FileName = $shine.Source
$start.UseShellExecute = $false
$start.Arguments = ($argv | ForEach-Object {{ ConvertTo-ShineArgument $_ }}) -join ' '
$process = New-Object System.Diagnostics.Process
$process.StartInfo = $start
try {{
  [void]$process.Start()
  $process.WaitForExit()
  $code = $process.ExitCode
}} finally {{ $process.Dispose() }}
exit $code
"#,
        source.display(),
    )
}

#[cfg(any(not(unix), test))]
fn cmd_live_bun_launch_content(source: &Path, config: &Path, target: &str) -> String {
    // Percent signs in batch-file literals need doubling; delayed expansion
    // must be off so literal exclamation marks survive. User argv stays %*.
    // Skip raw ownership metadata so path metacharacters never become commands.
    let config = windows_native_path(config).replace('%', "%%");
    let target = target.replace('%', "%%");
    format!(
        "@echo off\r\ngoto :shine_launch\r\nREM shine-managed\r\nREM shine-target: {}\r\n:shine_launch\r\n\
         setlocal DisableDelayedExpansion\r\nwhere shine >nul 2>nul\r\nif errorlevel 1 exit /b 127\r\n\
         shine --config-dir \"{config}\" __shell-launch \"{target}\" -- %*\r\nexit /b %errorlevel%\r\n",
        source.display(),
    )
}

fn live_config_dir(rendered_source: &Path) -> PathBuf {
    rendered_source
        .ancestors()
        .find(|path| path.file_name() == Some(OsStr::new("rendered")))
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| {
            rendered_source
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf()
        })
}

#[cfg(not(unix))]
fn powershell_shim_content(
    source: &Path,
    runtime: LinkRuntime,
    name: &str,
    bun_dependencies: BunDependencyMode,
    env: &[String],
    render_target: Option<&str>,
) -> String {
    let target = windows_native_path(source);
    let escaped = target.replace('\'', "''");
    let render = render_target.map_or_else(String::new, |render_target| {
        let render_target = render_target.replace('\'', "''");
        let config_dir = windows_native_path(&live_config_dir(source)).replace('\'', "''");
        let config_arg = if Path::new(&config_dir).file_name() == Some(OsStr::new(".shine")) {
            String::new()
        } else {
            format!("--config-dir '{config_dir}' ")
        };
        format!(
            "$shineDotSourced = $MyInvocation.InvocationName -eq '.'\nif (-not (Get-Command shine -ErrorAction SilentlyContinue)) {{\n  [Console]::Error.WriteLine('shine: live transformed command requires shine on PATH.')\n  if ($shineDotSourced) {{ return }} else {{ exit 127 }}\n}}\n& shine {config_arg}__shell-render '{render_target}'\nif ($LASTEXITCODE -ne 0) {{\n  $shineRenderCode = $LASTEXITCODE\n  if ($shineDotSourced) {{ return }} else {{ exit $shineRenderCode }}\n}}\n"
        )
    });
    match runtime {
        LinkRuntime::Bun => {
            let install_arg = bun_dependencies.install_arg();
            let name_escaped = name.replace('\'', "''");
            let bun_check = format!(
                "if (-not (Get-Command bun -ErrorAction SilentlyContinue)) {{\n  [Console]::Error.WriteLine('shine: {name_escaped} requires Bun, which was not found on PATH. Install from https://bun.sh')\n  exit 127\n}}\n"
            );
            if env.is_empty() {
                format!(
                    "{SHIM_MANAGED_MARKER}\n{SHIM_TARGET_PREFIX}{target}\n{render}{bun_check}& bun {install_arg} '{escaped}' @args\nexit $LASTEXITCODE\n"
                )
            } else {
                let with_args = env
                    .iter()
                    .map(|token| format!("--with '{}'", token.replace('\'', "''")))
                    .collect::<Vec<_>>()
                    .join(" ");
                format!(
                    "{SHIM_MANAGED_MARKER}\n{SHIM_TARGET_PREFIX}{target}\n{render}{bun_check}if (-not (Get-Command shine -ErrorAction SilentlyContinue)) {{\n  [Console]::Error.WriteLine('shine: {name_escaped} requires the shine command, which was not found on PATH.')\n  exit 127\n}}\n& shine env run --no-workspace {with_args} -- bun {install_arg} '{escaped}' @args\nexit $LASTEXITCODE\n"
                )
            }
        }
        LinkRuntime::Native => {
            let bash_target = bash_compatible_path(source);
            let bash_escaped = bash_target.replace('\'', "''");
            match source.extension().and_then(|e| e.to_str()) {
                Some("ps1") => format!(
                    "{SHIM_MANAGED_MARKER}\n{SHIM_TARGET_PREFIX}{target}\n{render}if ($MyInvocation.InvocationName -eq '.') {{\n  . '{escaped}' @args\n}} else {{\n  & '{escaped}' @args\n  exit $LASTEXITCODE\n}}\n"
                ),
                _ => format!(
                    "{SHIM_MANAGED_MARKER}\n{SHIM_TARGET_PREFIX}{target}\n{render}& bash '{bash_escaped}' @args\nexit $LASTEXITCODE\n"
                ),
            }
        }
    }
}

#[cfg(any(not(unix), test))]
fn cmd_shim_content(
    source: &Path,
    runtime: LinkRuntime,
    name: &str,
    bun_dependencies: BunDependencyMode,
    env: &[String],
    render_target: Option<&str>,
) -> String {
    let target = windows_native_path(source);
    let render = render_target.map_or_else(String::new, |render_target| {
        let config_dir = windows_native_path(&live_config_dir(source));
        let config_arg = if Path::new(&config_dir).file_name() == Some(OsStr::new(".shine")) {
            String::new()
        } else {
            format!("--config-dir \"{config_dir}\" ")
        };
        format!(
            "where shine >nul 2>nul\r\nif errorlevel 1 exit /b 127\r\nshine {config_arg}__shell-render \"{render_target}\"\r\nif errorlevel 1 exit /b %errorlevel%\r\n"
        )
    });
    match runtime {
        LinkRuntime::Bun => {
            let install_arg = bun_dependencies.install_arg();
            let bun_check = format!(
                "where bun >nul 2>nul\r\nif errorlevel 1 (\r\n  echo shine: {name} requires Bun, which was not found on PATH. Install from https://bun.sh 1>&2\r\n  exit /b 127\r\n)\r\n"
            );
            if env.is_empty() {
                format!(
                    "@echo off\r\nREM shine-managed\r\nREM shine-target: {target}\r\n{render}{bun_check}bun {install_arg} \"{target}\" %*\r\n"
                )
            } else {
                let with_args = env
                    .iter()
                    .map(|token| format!("--with {token}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                format!(
                    "@echo off\r\nREM shine-managed\r\nREM shine-target: {target}\r\n{render}{bun_check}where shine >nul 2>nul\r\nif errorlevel 1 (\r\n  echo shine: {name} requires the shine command, which was not found on PATH. 1>&2\r\n  exit /b 127\r\n)\r\nshine env run --no-workspace {with_args} -- bun {install_arg} \"{target}\" %*\r\n"
                )
            }
        }
        LinkRuntime::Native => {
            let escaped = target.replace('\'', "''");
            let bash_target = bash_compatible_path(source);
            match source.extension().and_then(|e| e.to_str()) {
                Some("ps1") => format!(
                    "@echo off\r\nREM shine-managed\r\nREM shine-target: {target}\r\n{render}powershell.exe -NoProfile -ExecutionPolicy Bypass -File \"{escaped}\" %*\r\n"
                ),
                _ => format!(
                    "@echo off\r\nREM shine-managed\r\nREM shine-target: {target}\r\n{render}bash \"{bash_target}\" %*\r\n"
                ),
            }
        }
    }
}

pub(crate) fn uses_native_cmd_literal_path(runtime: LinkRuntime, source: &Path) -> bool {
    cfg!(not(unix))
        && runtime == LinkRuntime::Native
        && source.extension() == Some(OsStr::new("ps1"))
}

#[cfg(any(not(unix), test))]
fn correct_native_cmd_path(content: String, source: &Path) -> String {
    // Keep the legacy generator byte-identical for receipt reconstruction; change
    // only the native invocation argument, retaining the original ownership marker.
    let target = windows_native_path(source);
    content.replace(
        &format!("-File \"{}\"", target.replace('\'', "''")),
        &format!("-File \"{target}\""),
    )
}

#[cfg(any(not(unix), test))]
fn bash_compatible_path(path: &Path) -> String {
    windows_native_path(path).replace('\\', "/")
}

#[cfg(any(not(unix), test))]
fn windows_native_path(path: &Path) -> String {
    strip_windows_verbatim_prefix(&path.display().to_string())
}

#[cfg(any(not(unix), test))]
fn strip_windows_verbatim_prefix(value: &str) -> String {
    value
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .or_else(|| value.strip_prefix(r"\\?\").map(str::to_string))
        .unwrap_or_else(|| value.to_string())
}

#[cfg(not(unix))]
fn windows_path_key(path: &Path) -> String {
    windows_native_path(path)
        .replace('\\', "/")
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests;
