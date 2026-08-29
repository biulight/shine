use crate::runtime::{BunDependencyMode, FileKind, FileSystemHost, LinkRuntime};
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

const LINKABLE_SCRIPT_EXTENSIONS: &[&str] = &["sh", "bash", "zsh", "fish", "ps1"];

/// Script extensions runnable through the `bun` runtime launcher.
const BUN_SCRIPT_EXTENSIONS: &[&str] = &["ts", "js", "mts", "mjs"];

#[cfg(not(unix))]
const EXECUTABLE_EXTENSIONS: &[&str] = &["sh", "ps1"];

// Marker lines identifying a shine-managed launcher. The Unix bun launcher script
// and the Windows `.ps1`/`.cmd` shims all use the same convention so ownership
// (`unlink_managed`) and current-ness detection are shared across platforms.
const SHIM_MANAGED_MARKER: &str = "# shine-managed";
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

#[derive(Clone)]
pub struct LinkSpec {
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

async fn host_launcher_target(host: &impl FileSystemHost, path: &Path) -> Result<Option<PathBuf>> {
    let content = match host.read(path).await {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(content) => content,
            Err(_) => return Ok(None),
        },
        Err(_) => return Ok(None),
    };
    if !content.contains(SHIM_MANAGED_MARKER) {
        return Ok(None);
    }
    Ok(shim_target_from_content(&content))
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

async fn host_create_link(
    host: &impl FileSystemHost,
    source: &Path,
    link_path: &Path,
    runtime: LinkRuntime,
    bun_dependencies: BunDependencyMode,
    env: &[String],
    render_target: Option<&str>,
) -> Result<()> {
    if let Some(parent) = link_path.parent() {
        host.create_dir_all(parent)
            .await
            .map_err(|error| error.into_anyhow("creating shell launcher directory"))?;
    }
    #[cfg(unix)]
    {
        let content = if let Some(target) = render_target {
            Some(unix_live_launcher_content(
                source,
                &launcher_command_name(link_path),
                runtime,
                bun_dependencies,
                env,
                target,
            ))
        } else if runtime == LinkRuntime::Bun {
            Some(unix_bun_launcher_content(
                source,
                &launcher_command_name(link_path),
                bun_dependencies,
                env,
            ))
        } else {
            None
        };
        if let Some(content) = content {
            host.write_atomic(link_path, content.as_bytes())
                .await
                .map_err(|error| error.into_anyhow("writing shell launcher"))?;
            host.set_mode(link_path, 0o755)
                .await
                .map_err(|error| error.into_anyhow("setting shell launcher permissions"))?;
        } else {
            host.symlink(source, link_path)
                .await
                .map_err(|error| error.into_anyhow("creating shell symlink"))?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let name = launcher_command_name(link_path);
        let ps1 =
            powershell_shim_content(source, runtime, &name, bun_dependencies, env, render_target);
        let cmd = cmd_shim_content(source, runtime, &name, bun_dependencies, env, render_target);
        host.write_atomic(link_path, ps1.as_bytes())
            .await
            .map_err(|error| error.into_anyhow("writing PowerShell shim"))?;
        host.write_atomic(&link_path.with_extension("cmd"), cmd.as_bytes())
            .await
            .map_err(|error| error.into_anyhow("writing cmd shim"))?;
        Ok(())
    }
}

async fn host_launcher_status(
    host: &impl FileSystemHost,
    link_path: &Path,
    source: &Path,
    runtime: LinkRuntime,
    bun_dependencies: BunDependencyMode,
    env: &[String],
    render_target: Option<&str>,
) -> Result<LauncherStatus> {
    let content = match host.read(link_path).await {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(content) => content,
            Err(_) => return Ok(LauncherStatus::NotManaged),
        },
        Err(_) => return Ok(LauncherStatus::NotManaged),
    };
    if !content.contains(SHIM_MANAGED_MARKER) {
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
        let expected_cmd =
            cmd_shim_content(source, runtime, &name, bun_dependencies, env, render_target);
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
    host: &impl FileSystemHost,
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
    let path = command_path_for_name(bin_dir, command);
    let mut report = UnlinkReport {
        removed: Vec::new(),
        skipped: Vec::new(),
    };
    let metadata = match host.metadata(&path).await {
        Ok(metadata) => metadata,
        Err(error) if error.is_not_found() => return Ok(report),
        Err(error) => return Err(error.into_anyhow("inspecting shell launcher")),
    };
    let target = if metadata.kind == FileKind::Symlink {
        host.read_link(&path).await.ok()
    } else {
        host_launcher_target(host, &path).await?
    };
    let managed = target.is_some_and(|target| {
        managed_roots
            .iter()
            .any(|root| target_is_managed(&target, root, bin_dir))
    });
    if !managed {
        report.skipped.push(path);
        return Ok(report);
    }
    if !dry_run {
        host_remove_link(host, &path).await?;
    }
    report.removed.push(path);
    Ok(report)
}

/// Remove symlinks in `bin_dir` whose link target starts with `managed_root`.
///
/// Non-symlinks and symlinks pointing outside `managed_root` are untouched.
/// Missing `bin_dir` is treated as a no-op (returns empty report).
/// When `dry_run` is true, nothing is removed.
pub async fn unlink_managed(
    bin_dir: &Path,
    managed_root: &Path,
    dry_run: bool,
) -> Result<UnlinkReport> {
    let mut report = UnlinkReport {
        removed: Vec::new(),
        skipped: Vec::new(),
    };

    let mut read_dir = match tokio::fs::read_dir(bin_dir).await {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(report),
        Err(e) => return Err(e).with_context(|| format!("reading bin dir: {bin_dir:?}")),
    };

    while let Some(entry) = read_dir
        .next_entry()
        .await
        .with_context(|| format!("iterating bin dir: {bin_dir:?}"))?
    {
        let path = entry.path();
        let meta = match tokio::fs::symlink_metadata(&path).await {
            Ok(m) => m,
            Err(_) => continue,
        };

        // Regular files are shine-managed only when they carry the managed marker
        // and record a target under `managed_root`. On Windows these are the
        // `.ps1`/`.cmd` shims; on Unix they are the generated bun launcher scripts.
        // User files (no marker, foreign target, or unreadable) are always skipped —
        // this is the "uninstall never touches user files" invariant.
        if !meta.file_type().is_symlink() {
            match launcher_target(&path).await {
                Ok(Some(target)) if target_is_managed(&target, managed_root, bin_dir) => {
                    if !dry_run {
                        remove_link(&path).await?;
                    }
                    report.removed.push(path);
                }
                _ => report.skipped.push(path),
            }
            continue;
        }

        let target = match tokio::fs::read_link(&path).await {
            Ok(t) => t,
            Err(_) => {
                report.skipped.push(path);
                continue;
            }
        };

        // Lexical prefix check — works even if the target file no longer exists.
        if target_is_managed(&target, managed_root, bin_dir) {
            if !dry_run {
                tokio::fs::remove_file(&path)
                    .await
                    .with_context(|| format!("removing symlink: {path:?}"))?;
            }
            report.removed.push(path);
        } else {
            report.skipped.push(path);
        }
    }

    Ok(report)
}

/// Remove one command entry only when it is owned by Shine and points below one
/// of `managed_roots`. This is the command-scoped counterpart to
/// [`unlink_managed`]; foreign files and links are reported as skipped.
pub async fn unlink_managed_command(
    bin_dir: &Path,
    command: &OsStr,
    managed_roots: &[PathBuf],
    dry_run: bool,
) -> Result<UnlinkReport> {
    let path = command_path_for_name(bin_dir, command);
    let mut report = UnlinkReport {
        removed: Vec::new(),
        skipped: Vec::new(),
    };
    let meta = match tokio::fs::symlink_metadata(&path).await {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(report),
        Err(error) => return Err(error).with_context(|| format!("stat failed: {path:?}")),
    };

    let target = if meta.file_type().is_symlink() {
        tokio::fs::read_link(&path).await.ok()
    } else {
        launcher_target(&path).await?
    };
    let managed = target.is_some_and(|target| {
        managed_roots
            .iter()
            .any(|root| target_is_managed(&target, root, bin_dir))
    });
    if !managed {
        report.skipped.push(path);
        return Ok(report);
    }

    if !dry_run {
        remove_link(&path).await?;
    }
    report.removed.push(path);
    Ok(report)
}

/// Create flat symlinks in `bin_dir` for each executable file in `sources`.
///
/// - Existing correct symlinks are skipped (idempotent).
/// - Conflicting entries (wrong target or regular file) are recorded and skipped
///   unless `overwrite` is true.
/// - Two sources sharing the same filename → second is recorded as a conflict.
#[cfg(test)]
pub async fn link_executables(
    bin_dir: &Path,
    sources: &[PathBuf],
    overwrite: bool,
) -> Result<LinkReport> {
    let specs: Vec<_> = sources
        .iter()
        .map(|source| LinkSpec {
            source: source.clone(),
            link_name: link_stem(source),
            runtime: LinkRuntime::Native,
            bun_dependencies: BunDependencyMode::Disabled,
            env: Vec::new(),
            render_target: None,
        })
        .collect();
    link_executables_with_names(bin_dir, &specs, overwrite).await
}

pub async fn link_executables_with_names(
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

    let mut seen: HashSet<OsString> = HashSet::new();

    for spec in specs {
        // Native links require a runnable/linkable source; bun launchers wrap any
        // declared bun script, so they bypass the executable/extension gate.
        if spec.runtime == LinkRuntime::Native && !is_linkable_source(&spec.source) {
            continue;
        }

        if spec.source.file_name().is_none() {
            continue;
        }
        let stem = spec.link_name.clone();

        if !seen.insert(stem.clone()) {
            report.conflicts.push(LinkConflict {
                link_path: command_path_for_name(bin_dir, &stem),
                source: spec.source.clone(),
                kind: LinkConflictKind::DuplicateName,
            });
            continue;
        }

        let link_path = command_path_for_name(bin_dir, &stem);

        match tokio::fs::symlink_metadata(&link_path).await {
            Ok(meta) if meta.file_type().is_symlink() => {
                match tokio::fs::read_link(&link_path).await {
                    Ok(existing) if existing == spec.source && spec.render_target.is_none() => {
                        report.skipped.push(link_path);
                    }
                    _ => {
                        if overwrite {
                            tokio::fs::remove_file(&link_path).await.with_context(|| {
                                format!("removing stale symlink: {link_path:?}")
                            })?;
                            create_link(
                                &spec.source,
                                &link_path,
                                spec.runtime,
                                spec.bun_dependencies,
                                &spec.env,
                                spec.render_target.as_deref(),
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
                }
            }
            Ok(_) => {
                match launcher_status(
                    &link_path,
                    &spec.source,
                    spec.runtime,
                    spec.bun_dependencies,
                    &spec.env,
                    spec.render_target.as_deref(),
                )
                .await?
                {
                    LauncherStatus::Current => {
                        report.skipped.push(link_path);
                        continue;
                    }
                    LauncherStatus::Stale => {
                        remove_link(&link_path).await?;
                        create_link(
                            &spec.source,
                            &link_path,
                            spec.runtime,
                            spec.bun_dependencies,
                            &spec.env,
                            spec.render_target.as_deref(),
                        )
                        .await?;
                        report.overwritten.push(link_path);
                        continue;
                    }
                    LauncherStatus::NotManaged => {}
                }

                if overwrite {
                    remove_link(&link_path).await?;
                    create_link(
                        &spec.source,
                        &link_path,
                        spec.runtime,
                        spec.bun_dependencies,
                        &spec.env,
                        spec.render_target.as_deref(),
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
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                create_link(
                    &spec.source,
                    &link_path,
                    spec.runtime,
                    spec.bun_dependencies,
                    &spec.env,
                    spec.render_target.as_deref(),
                )
                .await?;
                report.created.push(link_path);
            }
            Err(e) => {
                return Err(e).with_context(|| format!("stat failed: {link_path:?}"));
            }
        }
    }

    Ok(report)
}

/// Return whether an installed command exactly matches its expected source, runtime, and
/// runtime environment declaration.
///
/// Status surfaces use the same current-ness rules as install/upgrade so an existing command
/// from an older source or runtime is reported as an available update.
pub async fn link_is_current(
    link_path: &Path,
    source: &Path,
    runtime: LinkRuntime,
    bun_dependencies: BunDependencyMode,
    env: &[String],
    render_target: Option<&str>,
) -> Result<bool> {
    match tokio::fs::symlink_metadata(link_path).await {
        Ok(meta) if meta.file_type().is_symlink() => {
            if runtime != LinkRuntime::Native || render_target.is_some() {
                return Ok(false);
            }
            Ok(tokio::fs::read_link(link_path)
                .await
                .is_ok_and(|target| target == source))
        }
        Ok(_) => Ok(matches!(
            launcher_status(
                link_path,
                source,
                runtime,
                bun_dependencies,
                env,
                render_target,
            )
            .await?,
            LauncherStatus::Current
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("stat failed: {link_path:?}")),
    }
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

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|ext| EXECUTABLE_EXTENSIONS.contains(&ext))
            .unwrap_or(false)
    }
}

fn is_linkable_source(path: &Path) -> bool {
    is_executable(path) || has_linkable_script_extension(path)
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

/// True when `target` (from a launcher's `# shine-target:` line or a symlink)
/// lexically resolves under `managed_root`. Relative targets are resolved against
/// `bin_dir`. Works even if the target file no longer exists.
fn target_is_managed(target: &Path, managed_root: &Path, bin_dir: &Path) -> bool {
    if target.is_absolute() {
        target.starts_with(managed_root)
    } else {
        bin_dir.join(target).starts_with(managed_root)
    }
}

/// The command name a launcher exposes — the link path's file stem.
fn launcher_command_name(link_path: &Path) -> String {
    link_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Read a launcher/shim's recorded `# shine-target:` path, or `None` if the file
/// is not a shine-managed launcher (missing marker) or is unreadable. Any read
/// error yields `None` so a user file is never mistaken for a managed launcher.
async fn launcher_target(path: &Path) -> Result<Option<PathBuf>> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(content) => content,
        Err(_) => return Ok(None),
    };
    if !content.contains(SHIM_MANAGED_MARKER) {
        return Ok(None);
    }
    Ok(shim_target_from_content(&content))
}

fn shim_target_from_content(content: &str) -> Option<PathBuf> {
    content.lines().find_map(|line| {
        line.strip_prefix(SHIM_TARGET_PREFIX)
            .or_else(|| line.strip_prefix("REM shine-target: "))
            .map(PathBuf::from)
    })
}

async fn create_link(
    source: &Path,
    link_path: &Path,
    runtime: LinkRuntime,
    bun_dependencies: BunDependencyMode,
    env: &[String],
    render_target: Option<&str>,
) -> Result<()> {
    #[cfg(unix)]
    {
        if let Some(target) = render_target {
            return write_unix_live_launcher(
                source,
                link_path,
                runtime,
                bun_dependencies,
                env,
                target,
            )
            .await;
        }
        match runtime {
            LinkRuntime::Native => tokio::fs::symlink(source, link_path)
                .await
                .with_context(|| format!("creating symlink {link_path:?} -> {source:?}")),
            LinkRuntime::Bun => {
                write_unix_bun_launcher(source, link_path, bun_dependencies, env).await
            }
        }
    }
    #[cfg(not(unix))]
    {
        create_windows_shims(
            source,
            link_path,
            runtime,
            bun_dependencies,
            env,
            render_target,
        )
        .await
    }
}

async fn remove_link(link_path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        tokio::fs::remove_file(link_path)
            .await
            .with_context(|| format!("removing existing file: {link_path:?}"))
    }
    #[cfg(not(unix))]
    {
        remove_windows_shims(link_path).await
    }
}

/// Whether the existing regular file at `link_path` is a current/stale/foreign
/// launcher for `source` under `runtime`. Native runtime on Unix has no managed
/// regular-file form (its links are symlinks), so any regular file is `NotManaged`
/// (a user-file conflict).
async fn launcher_status(
    link_path: &Path,
    source: &Path,
    runtime: LinkRuntime,
    bun_dependencies: BunDependencyMode,
    env: &[String],
    render_target: Option<&str>,
) -> Result<LauncherStatus> {
    #[cfg(unix)]
    {
        if let Some(target) = render_target {
            return unix_live_launcher_status(
                link_path,
                source,
                runtime,
                bun_dependencies,
                env,
                target,
            )
            .await;
        }
        match runtime {
            LinkRuntime::Bun => {
                unix_launcher_status(link_path, source, bun_dependencies, env).await
            }
            LinkRuntime::Native => Ok(LauncherStatus::NotManaged),
        }
    }
    #[cfg(not(unix))]
    {
        windows_shim_status(
            link_path,
            source,
            runtime,
            bun_dependencies,
            env,
            render_target,
        )
        .await
    }
}

#[cfg(unix)]
fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Deterministic content of a Unix bun launcher. Regenerated byte-for-byte by
/// `unix_launcher_status` to detect staleness, so any change here is a format
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

#[cfg(unix)]
async fn write_unix_live_launcher(
    source: &Path,
    link_path: &Path,
    runtime: LinkRuntime,
    bun_dependencies: BunDependencyMode,
    env: &[String],
    render_target: &str,
) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(parent) = link_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let name = launcher_command_name(link_path);
    let content =
        unix_live_launcher_content(source, &name, runtime, bun_dependencies, env, render_target);
    crate::persist::atomic_write(link_path, content.as_bytes()).await?;
    tokio::fs::set_permissions(link_path, std::fs::Permissions::from_mode(0o755)).await?;
    Ok(())
}

#[cfg(unix)]
async fn unix_live_launcher_status(
    link_path: &Path,
    source: &Path,
    runtime: LinkRuntime,
    bun_dependencies: BunDependencyMode,
    env: &[String],
    render_target: &str,
) -> Result<LauncherStatus> {
    let content = match tokio::fs::read_to_string(link_path).await {
        Ok(content) => content,
        Err(_) => return Ok(LauncherStatus::NotManaged),
    };
    if !content.contains(SHIM_MANAGED_MARKER) {
        return Ok(LauncherStatus::NotManaged);
    }
    let Some(target) = shim_target_from_content(&content) else {
        return Ok(LauncherStatus::Stale);
    };
    if target.as_os_str() != source.as_os_str() {
        return Ok(LauncherStatus::NotManaged);
    }
    let name = launcher_command_name(link_path);
    if content
        == unix_live_launcher_content(source, &name, runtime, bun_dependencies, env, render_target)
    {
        Ok(LauncherStatus::Current)
    } else {
        Ok(LauncherStatus::Stale)
    }
}

#[cfg(unix)]
async fn write_unix_bun_launcher(
    source: &Path,
    link_path: &Path,
    bun_dependencies: BunDependencyMode,
    env: &[String],
) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(parent) = link_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating bin dir: {parent:?}"))?;
    }
    let name = launcher_command_name(link_path);
    tokio::fs::write(
        link_path,
        unix_bun_launcher_content(source, &name, bun_dependencies, env),
    )
    .await
    .with_context(|| format!("writing bun launcher: {link_path:?}"))?;
    tokio::fs::set_permissions(link_path, std::fs::Permissions::from_mode(0o755))
        .await
        .with_context(|| format!("setting bun launcher permissions: {link_path:?}"))?;
    Ok(())
}

#[cfg(unix)]
async fn unix_launcher_status(
    link_path: &Path,
    source: &Path,
    bun_dependencies: BunDependencyMode,
    env: &[String],
) -> Result<LauncherStatus> {
    let content = match tokio::fs::read_to_string(link_path).await {
        Ok(content) => content,
        // Missing, non-UTF-8, or otherwise unreadable → treat as a user file.
        Err(_) => return Ok(LauncherStatus::NotManaged),
    };
    if !content.contains(SHIM_MANAGED_MARKER) {
        return Ok(LauncherStatus::NotManaged);
    }
    let Some(target) = shim_target_from_content(&content) else {
        return Ok(LauncherStatus::Stale);
    };
    if target.as_os_str() != source.as_os_str() {
        return Ok(LauncherStatus::NotManaged);
    }
    let name = launcher_command_name(link_path);
    // Byte comparison against the regenerated content — which embeds the ordered
    // `env` spec — so an added/removed/reordered declaration is detected as stale.
    if content == unix_bun_launcher_content(source, &name, bun_dependencies, env) {
        Ok(LauncherStatus::Current)
    } else {
        Ok(LauncherStatus::Stale)
    }
}

#[cfg(not(unix))]
async fn create_windows_shims(
    source: &Path,
    ps1_path: &Path,
    runtime: LinkRuntime,
    bun_dependencies: BunDependencyMode,
    env: &[String],
    render_target: Option<&str>,
) -> Result<()> {
    let cmd_path = ps1_path.with_extension("cmd");
    if let Some(parent) = ps1_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating bin dir: {parent:?}"))?;
    }
    let name = launcher_command_name(ps1_path);
    tokio::fs::write(
        ps1_path,
        powershell_shim_content(source, runtime, &name, bun_dependencies, env, render_target),
    )
    .await
    .with_context(|| format!("writing PowerShell shim: {ps1_path:?}"))?;
    tokio::fs::write(
        &cmd_path,
        cmd_shim_content(source, runtime, &name, bun_dependencies, env, render_target),
    )
    .await
    .with_context(|| format!("writing cmd shim: {cmd_path:?}"))?;
    Ok(())
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

#[cfg(not(unix))]
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

#[cfg(not(unix))]
fn bash_compatible_path(path: &Path) -> String {
    windows_native_path(path).replace('\\', "/")
}

#[cfg(not(unix))]
fn windows_native_path(path: &Path) -> String {
    strip_windows_verbatim_prefix(&path.display().to_string())
}

#[cfg(not(unix))]
fn strip_windows_verbatim_prefix(value: &str) -> String {
    value
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .or_else(|| value.strip_prefix(r"\\?\").map(str::to_string))
        .unwrap_or_else(|| value.to_string())
}

#[cfg(not(unix))]
async fn windows_shim_status(
    link_path: &Path,
    source: &Path,
    runtime: LinkRuntime,
    bun_dependencies: BunDependencyMode,
    env: &[String],
    render_target: Option<&str>,
) -> Result<LauncherStatus> {
    let content = match tokio::fs::read_to_string(link_path).await {
        Ok(content) => content,
        // Missing or unreadable (e.g. non-UTF-8 user file) → treat as a user file.
        Err(_) => return Ok(LauncherStatus::NotManaged),
    };
    if !content.contains(SHIM_MANAGED_MARKER) {
        return Ok(LauncherStatus::NotManaged);
    }

    let Some(target) = shim_target_from_content(&content) else {
        return Ok(LauncherStatus::Stale);
    };
    if windows_path_key(&target) != windows_path_key(source) {
        return Ok(LauncherStatus::NotManaged);
    }

    let name = launcher_command_name(link_path);
    let expected_ps1 =
        powershell_shim_content(source, runtime, &name, bun_dependencies, env, render_target);
    let expected_cmd =
        cmd_shim_content(source, runtime, &name, bun_dependencies, env, render_target);
    let cmd_path = link_path.with_extension("cmd");
    let cmd_content = tokio::fs::read_to_string(&cmd_path).await.ok();
    if content == expected_ps1 && cmd_content.as_deref() == Some(expected_cmd.as_str()) {
        Ok(LauncherStatus::Current)
    } else {
        Ok(LauncherStatus::Stale)
    }
}

#[cfg(not(unix))]
fn windows_path_key(path: &Path) -> String {
    windows_native_path(path)
        .replace('\\', "/")
        .to_ascii_lowercase()
}

#[cfg(not(unix))]
async fn remove_windows_shims(ps1_path: &Path) -> Result<()> {
    let cmd_path = ps1_path.with_extension("cmd");
    match tokio::fs::remove_file(ps1_path).await {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).with_context(|| format!("removing shim: {ps1_path:?}")),
    }
    match tokio::fs::remove_file(&cmd_path).await {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).with_context(|| format!("removing shim: {cmd_path:?}")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use tokio::fs;

    #[cfg(unix)]
    async fn make_dirs() -> (PathBuf, PathBuf) {
        let id = uuid::Uuid::new_v4();
        let src_dir = std::env::temp_dir().join(format!("shine-bl-src-{id}"));
        let bin_dir = std::env::temp_dir().join(format!("shine-bl-bin-{id}"));
        fs::create_dir_all(&src_dir).await.unwrap();
        fs::create_dir_all(&bin_dir).await.unwrap();
        (src_dir, bin_dir)
    }

    /// Write a file and set the executable bit so `is_executable` returns true.
    #[cfg(unix)]
    async fn make_executable(dir: &Path, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        fs::write(&path, b"#!/bin/sh\n").await.unwrap();
        let mut perms = fs::metadata(&path).await.unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).await.unwrap();
        path
    }

    #[cfg(unix)]
    async fn make_plain(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, b"data").await.unwrap();
        path
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn creates_symlink_for_executable_source() {
        let (src, bin) = make_dirs().await;
        let exe = make_executable(&src, "run.sh").await;

        let report = link_executables(&bin, std::slice::from_ref(&exe), false)
            .await
            .unwrap();

        assert_eq!(report.created.len(), 1);
        let link = &report.created[0];
        assert!(link.is_symlink());
        assert_eq!(fs::read_link(link).await.unwrap(), exe);
        // symlink name is the stem, not the full filename
        assert_eq!(link.file_name().unwrap(), "run");

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn skips_non_executable_source() {
        let (src, bin) = make_dirs().await;
        let plain = make_plain(&src, "readme.txt").await;

        let report = link_executables(&bin, &[plain], false).await.unwrap();

        assert!(report.created.is_empty());
        assert!(report.skipped.is_empty());

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn skips_when_correct_symlink_already_exists() {
        let (src, bin) = make_dirs().await;
        let exe = make_executable(&src, "run.sh").await;
        tokio::fs::symlink(&exe, bin.join("run")).await.unwrap();

        let report = link_executables(&bin, std::slice::from_ref(&exe), false)
            .await
            .unwrap();

        assert!(report.created.is_empty());
        assert_eq!(report.skipped.len(), 1);

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reports_conflict_when_regular_file_exists() {
        let (src, bin) = make_dirs().await;
        let exe = make_executable(&src, "run.sh").await;
        make_plain(&bin, "run").await;

        let report = link_executables(&bin, std::slice::from_ref(&exe), false)
            .await
            .unwrap();

        assert!(report.created.is_empty());
        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(report.conflicts[0].link_path, bin.join("run"));
        assert_eq!(report.conflicts[0].source, exe);
        assert_eq!(report.conflicts[0].kind, LinkConflictKind::ExistingEntry);

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn overwrites_stale_symlink_when_overwrite_true() {
        let (src, bin) = make_dirs().await;
        let exe = make_executable(&src, "run.sh").await;
        let other = make_executable(&src, "other.sh").await;
        tokio::fs::symlink(&other, bin.join("run")).await.unwrap();

        let report = link_executables(&bin, std::slice::from_ref(&exe), true)
            .await
            .unwrap();

        assert_eq!(report.overwritten.len(), 1);
        assert_eq!(fs::read_link(bin.join("run")).await.unwrap(), exe);

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn flattens_nested_preset_path_into_bin_dir() {
        let (src, bin) = make_dirs().await;
        let sub = src.join("shell").join("proxy");
        fs::create_dir_all(&sub).await.unwrap();
        let exe = {
            use std::os::unix::fs::PermissionsExt;
            let path = sub.join("set_proxy.sh");
            fs::write(&path, b"#!/bin/sh\n").await.unwrap();
            let mut perms = fs::metadata(&path).await.unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&path, perms).await.unwrap();
            path
        };

        let report = link_executables(&bin, &[exe], false).await.unwrap();

        assert_eq!(report.created.len(), 1);
        assert!(bin.join("set_proxy").exists());

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reports_collision_when_two_sources_share_basename() {
        let (src, bin) = make_dirs().await;
        let sub1 = src.join("a");
        let sub2 = src.join("b");
        fs::create_dir_all(&sub1).await.unwrap();
        fs::create_dir_all(&sub2).await.unwrap();
        let exe1 = make_executable(&sub1, "run.sh").await;
        let exe2 = make_executable(&sub2, "run.sh").await;

        let report = link_executables(&bin, &[exe1, exe2.clone()], false)
            .await
            .unwrap();

        assert_eq!(report.created.len(), 1);
        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(report.conflicts[0].link_path, bin.join("run"));
        assert_eq!(report.conflicts[0].source, exe2);
        assert_eq!(report.conflicts[0].kind, LinkConflictKind::DuplicateName);

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn creates_symlink_with_explicit_link_name() {
        let (src, bin) = make_dirs().await;
        let exe = make_executable(&src, "set_proxy.sh").await;
        let specs = [LinkSpec {
            source: exe.clone(),
            link_name: OsString::from("setproxy"),
            runtime: LinkRuntime::Native,
            bun_dependencies: BunDependencyMode::Disabled,
            env: Vec::new(),
            render_target: None,
        }];

        let report = link_executables_with_names(&bin, &specs, false)
            .await
            .unwrap();

        assert_eq!(report.created.len(), 1);
        assert!(bin.join("setproxy").exists());
        assert!(!bin.join("set_proxy").exists());
        assert_eq!(fs::read_link(bin.join("setproxy")).await.unwrap(), exe);

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn links_non_executable_shell_script_source() {
        let (src, bin) = make_dirs().await;
        let script = src.join("set_proxy.sh");
        fs::write(&script, b"#!/bin/sh\n").await.unwrap();
        let specs = [LinkSpec {
            source: script.clone(),
            link_name: OsString::from("setproxy"),
            runtime: LinkRuntime::Native,
            bun_dependencies: BunDependencyMode::Disabled,
            env: Vec::new(),
            render_target: None,
        }];

        let report = link_executables_with_names(&bin, &specs, false)
            .await
            .unwrap();

        assert_eq!(report.created.len(), 1);
        assert!(bin.join("setproxy").exists());
        assert_eq!(fs::read_link(bin.join("setproxy")).await.unwrap(), script);

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn skips_non_executable_non_script_source_with_custom_name() {
        let (src, bin) = make_dirs().await;
        let plain = make_plain(&src, "proxy.txt").await;
        let specs = [LinkSpec {
            source: plain,
            link_name: OsString::from("setproxy"),
            runtime: LinkRuntime::Native,
            bun_dependencies: BunDependencyMode::Disabled,
            env: Vec::new(),
            render_target: None,
        }];

        let report = link_executables_with_names(&bin, &specs, false)
            .await
            .unwrap();

        assert!(report.created.is_empty());
        assert!(!bin.join("setproxy").exists());

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    // --- unlink_managed tests ---

    #[cfg(unix)]
    #[tokio::test]
    async fn unlink_removes_symlink_pointing_into_managed_root() {
        let (src, bin) = make_dirs().await;
        let exe = make_executable(&src, "run.sh").await;
        tokio::fs::symlink(&exe, bin.join("run.sh")).await.unwrap();

        let report = unlink_managed(&bin, &src, false).await.unwrap();

        assert_eq!(report.removed.len(), 1);
        assert!(!bin.join("run.sh").exists());

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unlink_skips_symlink_outside_managed_root() {
        let (src, bin) = make_dirs().await;
        let outside = std::env::temp_dir().join(format!("shine-bl-out-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&outside).await.unwrap();
        let exe = make_executable(&outside, "run.sh").await;
        tokio::fs::symlink(&exe, bin.join("run.sh")).await.unwrap();

        let report = unlink_managed(&bin, &src, false).await.unwrap();

        assert_eq!(report.skipped.len(), 1);
        assert!(bin.join("run.sh").is_symlink());

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
        fs::remove_dir_all(&outside).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unlink_skips_regular_files_in_bin_dir() {
        let (src, bin) = make_dirs().await;
        make_plain(&bin, "user_script.sh").await;

        let report = unlink_managed(&bin, &src, false).await.unwrap();

        assert!(report.removed.is_empty());
        assert_eq!(report.skipped.len(), 1);
        assert!(bin.join("user_script.sh").exists());

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unlink_dry_run_reports_but_does_not_remove() {
        let (src, bin) = make_dirs().await;
        let exe = make_executable(&src, "run.sh").await;
        tokio::fs::symlink(&exe, bin.join("run.sh")).await.unwrap();

        let report = unlink_managed(&bin, &src, true).await.unwrap();

        assert_eq!(report.removed.len(), 1);
        assert!(bin.join("run.sh").is_symlink(), "dry-run must not remove");

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unlink_is_idempotent_on_empty_bin_dir() {
        let (src, bin) = make_dirs().await;

        let r1 = unlink_managed(&bin, &src, false).await.unwrap();
        let r2 = unlink_managed(&bin, &src, false).await.unwrap();

        assert!(r1.removed.is_empty());
        assert!(r2.removed.is_empty());

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[tokio::test]
    async fn unlink_returns_empty_report_when_bin_dir_missing() {
        let missing = std::env::temp_dir().join(format!("shine-bl-miss-{}", uuid::Uuid::new_v4()));
        let managed = std::env::temp_dir().join(format!("shine-bl-mgd-{}", uuid::Uuid::new_v4()));

        let report = unlink_managed(&missing, &managed, false).await.unwrap();

        assert!(report.removed.is_empty());
        assert!(report.skipped.is_empty());
    }

    #[test]
    fn link_stem_strips_bun_extensions() {
        assert_eq!(link_stem(Path::new("tool.ts")), OsString::from("tool"));
        assert_eq!(link_stem(Path::new("tool.js")), OsString::from("tool"));
        assert_eq!(link_stem(Path::new("tool.mts")), OsString::from("tool"));
        assert_eq!(link_stem(Path::new("tool.mjs")), OsString::from("tool"));
    }

    #[cfg(unix)]
    fn bun_spec(source: &Path, name: &str) -> LinkSpec {
        bun_spec_with_env(source, name, Vec::new())
    }

    #[cfg(unix)]
    fn locked_bun_spec(source: &Path, name: &str) -> LinkSpec {
        LinkSpec {
            source: source.to_path_buf(),
            link_name: OsString::from(name),
            runtime: LinkRuntime::Bun,
            bun_dependencies: BunDependencyMode::Locked,
            env: Vec::new(),
            render_target: None,
        }
    }

    #[cfg(unix)]
    fn bun_spec_with_env(source: &Path, name: &str, env: Vec<String>) -> LinkSpec {
        LinkSpec {
            source: source.to_path_buf(),
            link_name: OsString::from(name),
            runtime: LinkRuntime::Bun,
            bun_dependencies: BunDependencyMode::Disabled,
            env,
            render_target: None,
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn creates_bun_launcher_as_marked_executable_regular_file() {
        use std::os::unix::fs::PermissionsExt;
        let (src, bin) = make_dirs().await;
        let script = src.join("tool.ts");
        fs::write(&script, b"console.log('hi')\n").await.unwrap();

        let report = link_executables_with_names(&bin, &[bun_spec(&script, "tool")], false)
            .await
            .unwrap();

        assert_eq!(report.created.len(), 1);
        let launcher = bin.join("tool");
        assert!(launcher.exists());
        assert!(
            !launcher.is_symlink(),
            "bun launcher must be a regular file, not a symlink"
        );
        let content = fs::read_to_string(&launcher).await.unwrap();
        assert!(content.contains("# shine-managed"));
        assert!(content.contains(&format!("# shine-target: {}", script.display())));
        assert!(content.contains("command -v bun"));
        assert!(content.contains("exit 127"));
        assert!(content.contains(&format!(
            "exec bun --no-install '{}' \"$@\"",
            script.display()
        )));
        let mode = fs::metadata(&launcher).await.unwrap().permissions().mode();
        assert!(mode & 0o111 != 0, "launcher must be executable");

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bun_launcher_is_idempotent_and_refreshes_when_stale() {
        let (src, bin) = make_dirs().await;
        let script = src.join("tool.ts");
        fs::write(&script, b"console.log('hi')\n").await.unwrap();

        link_executables_with_names(&bin, &[bun_spec(&script, "tool")], false)
            .await
            .unwrap();
        let again = link_executables_with_names(&bin, &[bun_spec(&script, "tool")], false)
            .await
            .unwrap();
        assert_eq!(
            again.skipped.len(),
            1,
            "identical launcher should be skipped"
        );
        assert!(again.created.is_empty());
        assert!(again.overwritten.is_empty());

        // Same marker + target but different body → stale, refreshed without --force.
        let launcher = bin.join("tool");
        fs::write(
            &launcher,
            format!(
                "#!/usr/bin/env bash\n# shine-managed\n# shine-target: {}\necho stale\n",
                script.display()
            ),
        )
        .await
        .unwrap();
        let refreshed = link_executables_with_names(&bin, &[bun_spec(&script, "tool")], false)
            .await
            .unwrap();
        assert_eq!(
            refreshed.overwritten.len(),
            1,
            "stale launcher should refresh"
        );
        assert!(
            fs::read_to_string(&launcher)
                .await
                .unwrap()
                .contains("exec bun")
        );

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bun_launcher_conflicts_with_user_file_unless_forced() {
        let (src, bin) = make_dirs().await;
        let script = src.join("tool.ts");
        fs::write(&script, b"console.log('hi')\n").await.unwrap();
        // A user's own file at the same command name, no managed marker.
        make_plain(&bin, "tool").await;

        let report = link_executables_with_names(&bin, &[bun_spec(&script, "tool")], false)
            .await
            .unwrap();
        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(report.conflicts[0].kind, LinkConflictKind::ExistingEntry);
        assert_eq!(fs::read_to_string(bin.join("tool")).await.unwrap(), "data");

        let forced = link_executables_with_names(&bin, &[bun_spec(&script, "tool")], true)
            .await
            .unwrap();
        assert_eq!(forced.overwritten.len(), 1);
        assert!(
            fs::read_to_string(bin.join("tool"))
                .await
                .unwrap()
                .contains("exec bun")
        );

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unlink_removes_managed_bun_launcher_but_skips_user_file() {
        let (src, bin) = make_dirs().await;
        let script = src.join("tool.ts");
        fs::write(&script, b"console.log('hi')\n").await.unwrap();
        link_executables_with_names(&bin, &[bun_spec(&script, "tool")], false)
            .await
            .unwrap();
        // A user's own regular file that must survive uninstall.
        make_plain(&bin, "user_tool").await;

        let report = unlink_managed(&bin, &src, false).await.unwrap();

        assert!(report.removed.iter().any(|p| p.ends_with("tool")));
        assert!(
            !bin.join("tool").exists(),
            "managed launcher should be removed"
        );
        assert!(
            bin.join("user_tool").exists(),
            "user file must be preserved"
        );
        assert!(report.skipped.iter().any(|p| p.ends_with("user_tool")));

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bun_launcher_without_env_has_no_shine_dependency() {
        let (src, bin) = make_dirs().await;
        let script = src.join("tool.ts");
        fs::write(&script, b"console.log('hi')\n").await.unwrap();

        link_executables_with_names(&bin, &[bun_spec(&script, "tool")], false)
            .await
            .unwrap();

        let content = fs::read_to_string(bin.join("tool")).await.unwrap();
        assert!(
            content.contains(&format!(
                "exec bun --no-install '{}' \"$@\"",
                script.display()
            )),
            "no-env launcher must run bun directly: {content}"
        );
        assert!(
            !content.contains("shine env run"),
            "no-env launcher must not depend on shine: {content}"
        );

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bun_launcher_with_env_wraps_shine_env_run() {
        let (src, bin) = make_dirs().await;
        let script = src.join("tool.ts");
        fs::write(&script, b"console.log('hi')\n").await.unwrap();

        let env = vec!["API_URL".to_string(), "SERVICE_TOKEN=API_TOKEN".to_string()];
        link_executables_with_names(&bin, &[bun_spec_with_env(&script, "tool", env)], false)
            .await
            .unwrap();

        let content = fs::read_to_string(bin.join("tool")).await.unwrap();
        // Both prerequisites are checked with a 127 exit.
        assert!(content.contains("command -v bun"));
        assert!(content.contains("command -v shine"));
        assert_eq!(content.matches("exit 127").count(), 2);
        // The child runs through shine env run with the declared, ordered specs.
        assert!(content.contains(&format!(
            "exec shine env run --no-workspace --with 'API_URL' --with 'SERVICE_TOKEN=API_TOKEN' -- bun --no-install '{}' \"$@\"",
            script.display()
        )));

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn locked_bun_launcher_uses_fallback_and_refreshes_disabled_launcher() {
        let (src, bin) = make_dirs().await;
        let script = src.join("tool.ts");
        fs::write(&script, b"import 'zod'\n").await.unwrap();

        link_executables_with_names(&bin, &[bun_spec(&script, "tool")], false)
            .await
            .unwrap();
        let refreshed =
            link_executables_with_names(&bin, &[locked_bun_spec(&script, "tool")], false)
                .await
                .unwrap();
        assert_eq!(refreshed.overwritten.len(), 1);
        let content = fs::read_to_string(bin.join("tool")).await.unwrap();
        assert!(content.contains("exec bun --install=fallback"));

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bun_launcher_refreshes_when_env_declaration_changes() {
        let (src, bin) = make_dirs().await;
        let script = src.join("tool.ts");
        fs::write(&script, b"console.log('hi')\n").await.unwrap();

        // Install with no env, then replace the same source with a declaration.
        link_executables_with_names(&bin, &[bun_spec(&script, "tool")], false)
            .await
            .unwrap();
        let changed = link_executables_with_names(
            &bin,
            &[bun_spec_with_env(
                &script,
                "tool",
                vec!["API_URL".to_string()],
            )],
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            changed.overwritten.len(),
            1,
            "adding an env declaration must refresh the launcher without --force"
        );

        // Re-running with the same declaration is a no-op (byte-identical).
        let again = link_executables_with_names(
            &bin,
            &[bun_spec_with_env(
                &script,
                "tool",
                vec!["API_URL".to_string()],
            )],
            false,
        )
        .await
        .unwrap();
        assert_eq!(again.skipped.len(), 1);
        assert!(again.overwritten.is_empty());

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(not(unix))]
    #[test]
    fn shell_shims_pass_bash_compatible_paths_on_windows() {
        let source = PathBuf::from(r"C:\Users\me\.shine\rendered\shell\utils\copyfile.sh");

        let ps1 = powershell_shim_content(
            &source,
            LinkRuntime::Native,
            "copyfile",
            BunDependencyMode::Disabled,
            &[],
            None,
        );
        let cmd = cmd_shim_content(
            &source,
            LinkRuntime::Native,
            "copyfile",
            BunDependencyMode::Disabled,
            &[],
            None,
        );

        assert!(ps1.contains("C:/Users/me/.shine/rendered/shell/utils/copyfile.sh"));
        assert!(cmd.contains("C:/Users/me/.shine/rendered/shell/utils/copyfile.sh"));
        assert!(!ps1.contains(r"& bash 'C:\Users\me"));
    }
}
