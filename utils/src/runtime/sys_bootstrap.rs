use super::{
    CoreRuntime, FileKind, FileSystemHost, ProcessHost, ProcessRequest, ResolvedSelection,
    RuntimeEvent, RuntimeInteraction, RuntimeObserver, SelectionSource, SysDetection,
    SysDetectionProbe, SysInstall, SysItem, SysItemMode, SysItemOutcome, SysItemStatus,
    SysManifest, SysPackageProvider, SysRunEntry, SystemReceipt,
};
use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_LOG_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SysBootstrapRequest {
    pub os_id: String,
    pub item_id: String,
    pub sys_shell: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SysBootstrapReport {
    pub outcomes: Vec<SysItemOutcome>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SysBootstrapBatchRequest {
    pub os_id: String,
    pub requested: Vec<String>,
    pub preset: Option<String>,
    pub interactive: bool,
    pub sys_shell: String,
    pub dry_run: bool,
    pub force_profile: bool,
}

#[derive(Clone, Debug)]
pub struct SysBootstrapBatchReport {
    pub loaded: super::LoadedSysPreset,
    pub selection: ResolvedSelection,
    pub previews: Vec<String>,
    pub outcomes: Vec<SysItemOutcome>,
}

enum DetectionResult {
    Present(String),
    Missing,
}

struct ExecutionResult {
    success: bool,
    failure_detail: String,
    logs: Vec<String>,
}

impl<H: FileSystemHost + ProcessHost> CoreRuntime<H> {
    pub async fn run_sys_bootstrap_batch(
        &self,
        request: SysBootstrapBatchRequest,
        interaction: &mut impl RuntimeInteraction,
        observer: &mut impl RuntimeObserver,
    ) -> Result<SysBootstrapBatchReport> {
        let loaded = self.load_sys_preset(&request.os_id).await?;
        let selection = resolve_selection(
            &loaded.manifest,
            &request.requested,
            request.preset.as_deref(),
            request.interactive,
            interaction,
            observer,
        )?;
        let previews = selection
            .item_ids
            .iter()
            .map(|item_id| {
                let item = loaded
                    .manifest
                    .items
                    .iter()
                    .find(|item| item.id == *item_id)
                    .expect("Core selection references a manifest item");
                self.sys_install_preview(&request.os_id, &loaded.root, item)
            })
            .collect::<Result<Vec<_>>>()?;
        if request.dry_run || selection.item_ids.is_empty() {
            return Ok(SysBootstrapBatchReport {
                loaded,
                selection,
                previews,
                outcomes: Vec::new(),
            });
        }

        // Load and version-gate persistent state, and compose every selected profile input,
        // before the first installer or administrator prompt.
        let mut run_manifest =
            super::sys::load_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        for item_id in &selection.item_ids {
            let item = loaded
                .manifest
                .items
                .iter()
                .find(|item| item.id == *item_id)
                .expect("Core selection references a manifest item");
            self.preflight_sys_item(&request.os_id, &loaded.root, item)?;
        }
        let mut enabled =
            self.enabled_sys_profile_items(&loaded.manifest, &run_manifest.entries, &request.os_id);
        for item_id in &selection.item_ids {
            if loaded
                .manifest
                .items
                .iter()
                .find(|item| item.id == *item_id)
                .is_some_and(|item| !item.shell.is_empty())
            {
                enabled.insert(item_id.clone());
            }
        }
        self.preflight_composed_sys_profile(&request.os_id, &loaded, &enabled, &request.sys_shell)
            .await?;
        observer.emit(RuntimeEvent::SysBootstrapSelection {
            os_id: request.os_id.clone(),
            shell: request.sys_shell.clone(),
            item_ids: selection.item_ids.clone(),
            item_labels: loaded
                .manifest
                .items
                .iter()
                .map(|item| (item.id.clone(), item.label.clone()))
                .collect(),
            source: selection.source.clone(),
        });

        let mut outcomes = Vec::new();
        for item_id in &selection.item_ids {
            let report = self
                .run_sys_bootstrap(
                    SysBootstrapRequest {
                        os_id: request.os_id.clone(),
                        item_id: item_id.clone(),
                        sys_shell: request.sys_shell.clone(),
                    },
                    interaction,
                    observer,
                )
                .await?;
            let outcome = report
                .outcomes
                .into_iter()
                .next()
                .context("Core bootstrap returned no item outcome")?;
            let failed = outcome.status == SysItemStatus::Failed;
            observer.emit(RuntimeEvent::SysBootstrapOutcome(outcome.clone()));
            outcomes.push(outcome);
            if failed {
                break;
            }
        }

        if outcomes
            .iter()
            .any(|outcome| outcome.status != SysItemStatus::Failed)
        {
            let mut profile_enabled = self.enabled_sys_profile_items(
                &loaded.manifest,
                &run_manifest.entries,
                &request.os_id,
            );
            for outcome in &outcomes {
                if outcome.status != SysItemStatus::Failed
                    && loaded
                        .manifest
                        .items
                        .iter()
                        .find(|item| item.id == outcome.item_id)
                        .is_some_and(|item| !item.shell.is_empty())
                {
                    profile_enabled.insert(outcome.item_id.clone());
                }
            }
            let profile = self
                .install_composed_sys_profile(
                    &request.os_id,
                    &loaded,
                    &profile_enabled,
                    &request.sys_shell,
                    request.force_profile,
                )
                .await?;
            observer.emit(RuntimeEvent::SysBootstrapOutcome(profile.clone()));
            outcomes.push(profile);
        }

        for outcome in outcomes.iter().filter(|outcome| {
            outcome.item_id != "profile" && outcome.status != SysItemStatus::Failed
        }) {
            let item = loaded
                .manifest
                .items
                .iter()
                .find(|item| item.id == outcome.item_id);
            run_manifest.upsert(SysRunEntry {
                os_id: request.os_id.clone(),
                item_id: outcome.item_id.clone(),
                label: item
                    .map(|item| item.label.clone())
                    .unwrap_or_else(|| outcome.label.clone()),
                status: outcome.status,
                detail: outcome.detail.clone(),
                updated_at: self.context().captured_unix_time.to_string(),
                managed: item.is_some_and(|item| item.mode == SysItemMode::Managed),
                profile_enabled: item
                    .is_some_and(|item| item.mode == SysItemMode::Init && !item.shell.is_empty()),
                receipt: item
                    .filter(|item| item.mode == SysItemMode::Managed)
                    .map(|_| SystemReceipt::script()),
            });
        }
        super::sys::save_manifest_with_host(self.host(), &self.context().shine_dir, &run_manifest)
            .await?;
        Ok(SysBootstrapBatchReport {
            loaded,
            selection,
            previews,
            outcomes,
        })
    }

    pub async fn sys_item_is_present(&self, item: &SysItem) -> Result<bool> {
        let detect = item
            .detect
            .as_ref()
            .with_context(|| format!("sys item `{}` has no standard detection", item.id))?;
        Ok(matches!(
            self.detect_sys_item(detect).await?,
            DetectionResult::Present(_)
        ))
    }

    pub async fn run_sys_bootstrap(
        &self,
        request: SysBootstrapRequest,
        interaction: &mut impl RuntimeInteraction,
        observer: &mut impl RuntimeObserver,
    ) -> Result<SysBootstrapReport> {
        let loaded = self.load_sys_preset(&request.os_id).await?;
        let item = loaded
            .manifest
            .items
            .iter()
            .find(|item| item.id == request.item_id)
            .with_context(|| format!("sys item not found: {}", request.item_id))?;
        let detect = item
            .detect
            .as_ref()
            .with_context(|| format!("sys item `{}` has no standard detection", item.id))?;
        let install = item
            .install
            .as_ref()
            .with_context(|| format!("sys item `{}` has no standard installer", item.id))?;
        self.preflight_sys_item(&request.os_id, &loaded.root, item)?;
        if let DetectionResult::Present(detail) = self.detect_sys_item(detect).await? {
            return Ok(SysBootstrapReport {
                outcomes: vec![sys_outcome(
                    item,
                    SysItemStatus::AlreadyInstalled,
                    detail,
                    Vec::new(),
                )],
            });
        }
        let requires_admin = install_requires_admin(&request.os_id, install, item)?;
        observer.emit(RuntimeEvent::SysBootstrapItemStart {
            item_id: item.id.clone(),
            label: item.label.clone(),
            requires_admin,
        });
        if requires_admin
            && !self.context().running_as_admin
            && !interaction.authorize_admin(1).await?
        {
            return Ok(SysBootstrapReport {
                outcomes: vec![sys_outcome(
                    item,
                    SysItemStatus::Failed,
                    "administrator authorization was not granted".to_string(),
                    Vec::new(),
                )],
            });
        }
        let execution = match install {
            SysInstall::Package {
                provider, package, ..
            } => {
                self.run_package(&request.os_id, *provider, package, requires_admin)
                    .await?
            }
            SysInstall::Script { path, .. } => {
                let script = loaded.root.join(path);
                self.run_sys_script(
                    &request.os_id,
                    &script,
                    &loaded.root,
                    &request.sys_shell,
                    item,
                    requires_admin,
                )
                .await?
            }
        };
        if !execution.success {
            return Ok(SysBootstrapReport {
                outcomes: vec![sys_outcome(
                    item,
                    SysItemStatus::Failed,
                    execution.failure_detail,
                    execution.logs,
                )],
            });
        }
        let detected = self.detect_sys_item(detect).await?;
        if matches!(detected, DetectionResult::Missing) {
            return Ok(SysBootstrapReport {
                outcomes: vec![sys_outcome(
                    item,
                    SysItemStatus::Failed,
                    "installer succeeded but the declared detection is still missing".to_string(),
                    execution.logs,
                )],
            });
        }
        let mut detail = install.success_hint().to_string();
        if detail.is_empty()
            && let DetectionResult::Present(value) = detected
        {
            detail = value;
        }
        Ok(SysBootstrapReport {
            outcomes: vec![sys_outcome(
                item,
                install.success_status(),
                detail,
                execution.logs,
            )],
        })
    }

    pub fn preflight_sys_item(&self, _os_id: &str, root: &Path, item: &SysItem) -> Result<()> {
        for key in &item.required_env {
            if self
                .context()
                .env
                .get(key)
                .is_none_or(|value| value.trim().is_empty())
            {
                bail!(
                    "sys item `{}` requires environment variable `{key}`",
                    item.id
                );
            }
        }
        if let Some(SysInstall::Script { path, .. }) = &item.install {
            let script = safe_sys_path(root, path)?;
            let logical = format!(
                "sys/{}/{}",
                root.file_name().unwrap_or_default().to_string_lossy(),
                path.replace('\\', "/")
            );
            let external =
                self.context().is_external_presets || self.presets().is_overlay(&logical);
            if external && !self.context().allow_sys_code {
                bail!(
                    "executable sys install script is blocked because external preset code is active; set allow_sys_code = true after reviewing the source: {}",
                    script.display()
                );
            }
            if self.presets().get(&logical).is_none() && !script.is_file() {
                bail!(
                    "sys item `{}` install script is missing: {}",
                    item.id,
                    script.display()
                );
            }
        }
        Ok(())
    }

    pub fn sys_install_preview(&self, os_id: &str, root: &Path, item: &SysItem) -> Result<String> {
        let install = item
            .install
            .as_ref()
            .with_context(|| format!("sys item `{}` has no standard installer", item.id))?;
        match install {
            SysInstall::Package {
                provider, package, ..
            } => package_preview(os_id, *provider, package),
            SysInstall::Script { path, .. } => {
                let script = safe_sys_path(root, path)?;
                let external = self.context().is_external_presets
                    || self
                        .context()
                        .overlay_dir
                        .as_ref()
                        .is_some_and(|root| script.starts_with(root));
                Ok(format!(
                    "{}{}",
                    script.display(),
                    if external && !self.context().allow_sys_code {
                        " (requires allow_sys_code = true)"
                    } else {
                        ""
                    }
                ))
            }
        }
    }

    async fn detect_sys_item(&self, detection: &SysDetection) -> Result<DetectionResult> {
        match detection {
            SysDetection::Command {
                command,
                version_args,
            } => self.detect_command(command, version_args).await,
            SysDetection::Path { path } => self.detect_path(path).await,
            SysDetection::Any { probes } => {
                for probe in probes {
                    let result = match probe {
                        SysDetectionProbe::Command { command } => {
                            self.detect_command(command, &[]).await?
                        }
                        SysDetectionProbe::Path { path } => self.detect_path(path).await?,
                    };
                    if matches!(result, DetectionResult::Present(_)) {
                        return Ok(result);
                    }
                }
                Ok(DetectionResult::Missing)
            }
        }
    }

    async fn detect_path(&self, raw: &str) -> Result<DetectionResult> {
        let path = captured_expand(raw, &self.context().home_dir);
        match self.host().metadata(&path).await {
            Ok(_) => Ok(DetectionResult::Present(path.display().to_string())),
            Err(error) if error.is_not_found() => Ok(DetectionResult::Missing),
            Err(error) => Err(error.into_anyhow("inspecting Sys detection path")),
        }
    }

    async fn detect_command(
        &self,
        command: &str,
        version_args: &[String],
    ) -> Result<DetectionResult> {
        let Some(program) = self.find_command(command).await? else {
            return Ok(DetectionResult::Missing);
        };
        if version_args.is_empty() {
            return Ok(DetectionResult::Present(program.display().to_string()));
        }
        let output = self
            .host()
            .run(ProcessRequest {
                program: program.display().to_string(),
                args: version_args.to_vec(),
                stdout_limit: Some(MAX_LOG_BYTES),
                stderr_limit: Some(MAX_LOG_BYTES),
                ..ProcessRequest::default()
            })
            .await;
        let detail = output
            .ok()
            .filter(|output| output.exit_code == Some(0))
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .and_then(|stdout| stdout.lines().next().map(str::to_string))
            .filter(|line| !line.trim().is_empty())
            .unwrap_or_else(|| program.display().to_string());
        Ok(DetectionResult::Present(detail))
    }

    async fn find_command(&self, command: &str) -> Result<Option<PathBuf>> {
        let mut directories = self
            .context()
            .path_env
            .as_deref()
            .map(std::env::split_paths)
            .map(Iterator::collect::<Vec<_>>)
            .unwrap_or_default();
        directories.extend([
            self.context().home_dir.join(".local/bin"),
            self.context().home_dir.join(".cargo/bin"),
            self.context().home_dir.join(".bun/bin"),
            self.context().home_dir.join(".local/share/pnpm"),
            self.context()
                .home_dir
                .join("AppData/Local/Microsoft/WinGet/Links"),
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/home/linuxbrew/.linuxbrew/bin"),
        ]);
        for directory in directories {
            let candidates = if self.context().platform == super::RuntimePlatform::Windows {
                vec![
                    directory.join(command),
                    directory.join(format!("{command}.exe")),
                    directory.join(format!("{command}.cmd")),
                    directory.join(format!("{command}.bat")),
                    directory.join(format!("{command}.ps1")),
                ]
            } else {
                vec![directory.join(command)]
            };
            for candidate in candidates {
                if let Ok(metadata) = self.host().metadata(&candidate).await
                    && metadata.kind == FileKind::File
                    && (self.context().platform == super::RuntimePlatform::Windows
                        || metadata.unix_mode.is_none_or(|mode| mode & 0o111 != 0))
                {
                    return Ok(Some(candidate));
                }
            }
        }
        Ok(None)
    }

    async fn run_package(
        &self,
        os_id: &str,
        provider: SysPackageProvider,
        package: &str,
        admin: bool,
    ) -> Result<ExecutionResult> {
        let (program, mut args, _) =
            package_command(os_id, provider, package, &self.context().proxy_env)?;
        let mut env = self.context().proxy_env.clone();
        let program = if admin && !self.context().running_as_admin {
            let mut sudo_args = vec!["-n".to_string()];
            if !env.is_empty() {
                sudo_args.push("env".to_string());
                sudo_args.extend(env.iter().map(|(key, value)| format!("{key}={value}")));
            }
            sudo_args.push(program);
            sudo_args.append(&mut args);
            args = sudo_args;
            "sudo".to_string()
        } else {
            program
        };
        let output = self
            .host()
            .run(ProcessRequest {
                program,
                args,
                env: std::mem::take(&mut env),
                inherit_stdin: true,
                timeout: Some(BOOTSTRAP_TIMEOUT),
                stdout_limit: Some(MAX_LOG_BYTES),
                stderr_limit: Some(MAX_LOG_BYTES),
                ..ProcessRequest::default()
            })
            .await;
        execution_result(
            output,
            format!("{provider:?} install failed"),
            provider == SysPackageProvider::Winget,
        )
    }

    async fn run_sys_script(
        &self,
        os_id: &str,
        script: &Path,
        root: &Path,
        shell: &str,
        item: &SysItem,
        admin: bool,
    ) -> Result<ExecutionResult> {
        let (program, mut args) = match os_id {
            "windows" => (
                "powershell.exe".to_string(),
                vec![
                    "-NoProfile".into(),
                    "-ExecutionPolicy".into(),
                    "Bypass".into(),
                    "-File".into(),
                ],
            ),
            "macos" => ("zsh".to_string(), Vec::new()),
            _ => ("bash".to_string(), Vec::new()),
        };
        args.push(script.display().to_string());
        let mut env = BTreeMap::from([
            ("SHINE_SYS_PRESET_ROOT".into(), root.display().to_string()),
            ("SHINE_SYS_SHELL".into(), shell.to_string()),
            (
                "SHINE_TARGET_HOME".into(),
                self.context().home_dir.display().to_string(),
            ),
        ]);
        for key in &item.required_env {
            if let Some(value) = self.context().env.get(key) {
                env.insert(key.clone(), value.clone());
            }
        }
        env.extend(self.context().proxy_env.clone());
        let (program, args) = if admin && !self.context().running_as_admin {
            let mut sudo = vec![
                "-n".to_string(),
                format!(
                    "--preserve-env={}",
                    env.keys().cloned().collect::<Vec<_>>().join(",")
                ),
                program,
            ];
            sudo.extend(args);
            ("sudo".to_string(), sudo)
        } else {
            (program, args)
        };
        let output = self
            .host()
            .run(ProcessRequest {
                program,
                args,
                cwd: script.parent().map(Path::to_path_buf),
                env,
                inherit_stdin: true,
                timeout: Some(BOOTSTRAP_TIMEOUT),
                stdout_limit: Some(MAX_LOG_BYTES),
                stderr_limit: Some(MAX_LOG_BYTES),
                ..ProcessRequest::default()
            })
            .await;
        execution_result(
            output,
            format!("sys item `{}` install script failed", item.id),
            item.requires_admin && os_id == "windows",
        )
    }
}

fn resolve_selection(
    manifest: &SysManifest,
    requested: &[String],
    preset: Option<&str>,
    interactive: bool,
    interaction: &mut impl RuntimeInteraction,
    observer: &mut impl RuntimeObserver,
) -> Result<ResolvedSelection> {
    if !requested.is_empty() {
        if preset.is_some() {
            bail!("explicit sys bootstrap items cannot be combined with `--preset`");
        }
        let mut seen = BTreeSet::new();
        let mut item_ids = Vec::new();
        for item_id in requested {
            let item = manifest
                .items
                .iter()
                .find(|item| item.id == *item_id)
                .with_context(|| format!("unknown sys bootstrap item `{item_id}`"))?;
            if item.mode == SysItemMode::Managed {
                bail!("`{item_id}` is a managed system resource; use `shine sys apply {item_id}`");
            }
            if seen.insert(item_id.as_str()) {
                item_ids.push(item_id.clone());
            }
        }
        return Ok(ResolvedSelection {
            item_ids,
            source: SelectionSource::Items,
        });
    }
    if let Some(name) = preset {
        let profile = manifest
            .profiles
            .get(name)
            .with_context(|| format!("unknown sys bootstrap profile `{name}`"))?;
        return Ok(ResolvedSelection {
            item_ids: profile.items.clone(),
            source: SelectionSource::Profile(name.to_string()),
        });
    }
    let init_items = manifest
        .items
        .iter()
        .filter(|item| item.mode == SysItemMode::Init)
        .collect::<Vec<_>>();
    if init_items.is_empty() {
        return Ok(ResolvedSelection {
            item_ids: Vec::new(),
            source: SelectionSource::NoItems,
        });
    }
    if interactive {
        observer.emit(RuntimeEvent::Interaction {
            code: "sys_bootstrap_selection",
            target: manifest.default_profile.clone().unwrap_or_default(),
        });
        let choices = init_items
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>();
        let defaults = default_selection(manifest);
        let item_ids = interaction.select_many("Select system init items", &choices, &defaults)?;
        return Ok(ResolvedSelection {
            item_ids,
            source: SelectionSource::Interactive,
        });
    }
    let name = manifest
        .default_profile
        .as_deref()
        .context("sys bootstrap requires `default_profile` for non-interactive runs")?;
    let profile = manifest
        .profiles
        .get(name)
        .with_context(|| format!("unknown sys bootstrap profile `{name}`"))?;
    Ok(ResolvedSelection {
        item_ids: profile.items.clone(),
        source: SelectionSource::DefaultProfile(name.to_string()),
    })
}

fn default_selection(manifest: &SysManifest) -> Vec<String> {
    if let Some(name) = manifest.default_profile.as_deref()
        && let Some(profile) = manifest.profiles.get(name)
    {
        return profile.items.clone();
    }
    manifest
        .items
        .iter()
        .filter(|item| item.mode == SysItemMode::Init && item.default)
        .map(|item| item.id.clone())
        .collect()
}

fn safe_sys_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        bail!("sys preset path escapes its root: {relative}");
    }
    Ok(root.join(path))
}
fn captured_expand(raw: &str, home: &Path) -> PathBuf {
    if raw == "~" || raw == "$HOME" {
        home.to_path_buf()
    } else if let Some(rest) = raw
        .strip_prefix("~/")
        .or_else(|| raw.strip_prefix("$HOME/"))
    {
        home.join(rest)
    } else {
        PathBuf::from(raw)
    }
}
pub fn sys_install_requires_admin(
    os_id: &str,
    install: &SysInstall,
    item: &SysItem,
) -> Result<bool> {
    match install {
        SysInstall::Package {
            provider, package, ..
        } => Ok(package_command(os_id, *provider, package, &BTreeMap::new())?.2),
        SysInstall::Script { .. } => Ok(item.requires_admin && os_id != "windows"),
    }
}
fn install_requires_admin(os_id: &str, install: &SysInstall, item: &SysItem) -> Result<bool> {
    sys_install_requires_admin(os_id, install, item)
}
fn package_preview(os_id: &str, provider: SysPackageProvider, package: &str) -> Result<String> {
    let (program, args, admin) = package_command(os_id, provider, package, &BTreeMap::new())?;
    Ok(format!(
        "{}{} {}",
        if admin { "sudo " } else { "" },
        program,
        args.join(" ")
    ))
}
fn package_command(
    os_id: &str,
    provider: SysPackageProvider,
    package: &str,
    proxy: &BTreeMap<String, String>,
) -> Result<(String, Vec<String>, bool)> {
    match provider {
        SysPackageProvider::Homebrew | SysPackageProvider::HomebrewCask => {
            if os_id == "windows" {
                bail!("Homebrew sys provider is unavailable on Windows");
            }
            let mut args = vec!["install".to_string()];
            if provider == SysPackageProvider::HomebrewCask {
                args.push("--cask".into());
            }
            args.push(package.into());
            Ok(("brew".into(), args, false))
        }
        SysPackageProvider::Apt => {
            if os_id != "ubuntu" {
                bail!("APT sys provider is supported only on Ubuntu presets");
            }
            Ok((
                "apt-get".into(),
                vec!["install".into(), "-y".into(), package.into()],
                true,
            ))
        }
        SysPackageProvider::Winget => {
            if os_id != "windows" {
                bail!("Winget sys provider is supported only on Windows presets");
            }
            let mut args = vec![
                "install".into(),
                "--exact".into(),
                "--id".into(),
                package.into(),
                "--accept-package-agreements".into(),
                "--accept-source-agreements".into(),
            ];
            if let Some(proxy) = proxy.get("SHINE_SYS_PROXY") {
                args.extend(["--proxy".into(), proxy.clone()]);
            }
            Ok(("winget".into(), args, false))
        }
    }
}
fn execution_result(
    output: Result<super::ProcessOutput>,
    prefix: String,
    windows_admin_hint: bool,
) -> Result<ExecutionResult> {
    match output {
        Err(error) if error.to_string().contains("timed out") => Ok(ExecutionResult {
            success: false,
            failure_detail: format!("{prefix}: timed out after 30 minutes"),
            logs: Vec::new(),
        }),
        Err(error) => Err(error).context("running sys bootstrap installer"),
        Ok(output) => {
            let success = output.exit_code == Some(0);
            let mut detail = if success {
                String::new()
            } else {
                format!(
                    "{prefix} (exit {})",
                    output
                        .exit_code
                        .map_or_else(|| "signal".to_string(), |code| code.to_string())
                )
            };
            if !success && windows_admin_hint {
                detail.push_str("; retry from an elevated PowerShell");
            }
            Ok(ExecutionResult {
                success,
                failure_detail: detail,
                logs: bounded_logs(&output.stdout, &output.stderr),
            })
        }
    }
}
fn bounded_logs(stdout: &[u8], stderr: &[u8]) -> Vec<String> {
    let mut bytes = stdout.to_vec();
    if !stdout.is_empty() && !stderr.is_empty() {
        bytes.push(b'\n');
    }
    bytes.extend_from_slice(stderr);
    let truncated = bytes.len() > MAX_LOG_BYTES;
    if truncated {
        bytes = bytes[bytes.len() - MAX_LOG_BYTES..].to_vec();
    }
    let mut lines = String::from_utf8_lossy(&bytes)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if truncated {
        lines.insert(
            0,
            "installer output truncated at 64 KiB; showing final output".into(),
        );
    }
    lines
}
fn sys_outcome(
    item: &SysItem,
    status: SysItemStatus,
    detail: String,
    logs: Vec<String>,
) -> SysItemOutcome {
    SysItemOutcome {
        item_id: item.id.clone(),
        label: format!("sys/{}", item.id),
        status,
        detail,
        logs,
    }
}
