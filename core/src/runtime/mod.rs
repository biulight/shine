//! Workspace-internal, frontend-neutral runtime seams.
//!
//! These APIs are public only so the `shine-cli` package can consume them.
//! They are not a stable third-party API in Roadmap Phases 2 and 3.

mod app;
mod app_metadata;
mod bootstrap;
mod host;
mod inspection;
mod launcher;
mod memory;
mod planner;
mod preset;
mod profile;
mod shell;
mod sys;
mod sys_bootstrap;
mod sys_manifest;
mod sys_model;
mod sys_profile;
mod validation;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use host::{
    FileKind, FileMetadata, FileSystemHost, FileSystemObservationHost, HostError, HostOperation,
    NullObserver, PrivilegedFileSystemHost, PrivilegedOperationGuard, ProcessHost, ProcessIo,
    ProcessOutput, ProcessRequest, RealHost, RuntimeEvent, RuntimeInteraction, RuntimeObserver,
    SplitDnsHost, SplitDnsObservationHost, SplitDnsRequest, SplitDnsState,
};
pub use inspection::{
    AppFileInspection, DomainInspectionReport, InspectionChange, InspectionFileStatus,
    ShellFileInspection,
};
pub use launcher::{
    LinkConflict, LinkConflictKind, LinkReport, LinkSpec, UnlinkReport, command_path_for_name,
    link_executables_with_host, link_is_current_with_host, link_stem,
    unlink_managed_command_with_host,
};
pub use memory::InMemoryHost;
pub use planner::{
    AppPlanRequest, OpaqueSecretVersion, PlanningInputVersions, ShellPlanRequest,
    SysManagedPlanRequest,
};
pub use preset::{
    PresetFile, PresetFileOrigin, PresetSnapshot, PresetSourceKind, PresetValidationIssue,
    PresetValidationReport,
};
pub use profile::{
    PathUpdateStatus, SHELL_SENTINEL_END, SHELL_SENTINEL_START, ShellConfigUpdate,
    ShellProfileRemoval, managed_profile_snippet, managed_shell_profile_path,
    powershell_bin_assignment, powershell_quote, remove_shell_sentinel, shell_config_snippet,
    shell_source_command, supports_completion_registration,
};
pub use shell::{
    BunDependencyMode, BunRuntimeSpec, ExternalShellMode, LinkRuntime, SHELL_MANIFEST_FILE,
    SHELL_MANIFEST_SCHEMA_VERSION, ShellCacheReport, ShellCacheRequest, ShellCategory,
    ShellCompletionReport, ShellFile, ShellLifecycleReport, ShellLifecycleRequest, ShellManifest,
    ShellManifestEntry, ShellManifestUpdateScope, ShellScriptTemplate, ShellTarget,
    ShellTemplateReport, ShellType, ShellUninstallReport, ShellUninstallRequest,
    ShellUpgradeLifecycleReport, ShellUpgradeRequest, parse_shell_lifecycle_target,
};
pub use sys::{
    ManagedFileReceipt, ManagedFileRemoveRequest, ManagedFileRequest, RECEIPT_VERSION,
    ResourceConflict, ResourceOutcome, ResourcePlan, SYS_MANIFEST_FILE,
    SYS_MANIFEST_SCHEMA_VERSION, SplitDnsDomainRequest, SplitDnsReceipt, SysDriverKind,
    SysItemStatus, SysManagedAction, SysManagedReport, SysManagedRequest, SysRunEntry,
    SysRunManifest, SystemReceipt, remove_split_dns_with_host, split_dns_receipt,
};
pub use sys_bootstrap::{
    SysBootstrapBatchReport, SysBootstrapBatchRequest, SysBootstrapReport, SysBootstrapRequest,
    sys_install_requires_admin,
};
pub use sys_manifest::{parse_sys_manifest, validate_sys_manifest};
pub use sys_model::{
    LoadedSysPreset, ResolvedSelection, SYS_PROFILE_PHASES, SelectionSource,
    ShellProfileBlockPosition, SysDetection, SysDetectionProbe, SysInstall, SysInstalledRow,
    SysItem, SysItemMode, SysItemOutcome, SysManifest, SysPackageProvider, SysProfile,
    SysProfilePhase, SysShellIntegration, SysShellKind, SysUpdateRow, SysUpgradeReport,
};
pub use sys_profile::{SysProfileStateReport, SysProfileStateRequest};
pub use validation::{
    PRESET_VALIDATION_SCHEMA_VERSION, PresetCategoryValidation, PresetDiagnostic,
    PresetDiagnosticSeverity, PresetValidationReportV1, PresetValidationSummary,
    validate_preset_path,
};

/// Fully resolved runtime inputs. Domain code must not rediscover these from
/// ambient process state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeContext {
    pub home_dir: PathBuf,
    pub shine_dir: PathBuf,
    pub presets_dir: PathBuf,
    pub bin_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub data_dir: PathBuf,
    pub app_default_dest_root: PathBuf,
    pub overlay_dir: Option<PathBuf>,
    pub platform: RuntimePlatform,
    pub shell: ShellType,
    pub shell_config_paths: Vec<PathBuf>,
    pub external_shell_mode: ExternalShellMode,
    pub is_external_presets: bool,
    pub allow_app_hooks: bool,
    pub allow_sys_code: bool,
    pub linux_split_dns_ready: bool,
    pub running_as_admin: bool,
    pub captured_unix_time: u64,
    pub env: BTreeMap<String, String>,
    pub path_env: Option<String>,
    pub proxy_env: BTreeMap<String, String>,
}

impl RuntimeContext {
    /// Construct the deterministic defaults used by Core-only tests and
    /// embedders that do not need distribution policy.
    pub fn isolated(
        home_dir: PathBuf,
        shine_dir: PathBuf,
        presets_dir: PathBuf,
        bin_dir: PathBuf,
        platform: RuntimePlatform,
    ) -> Self {
        Self {
            cache_dir: shine_dir.join("cache"),
            data_dir: home_dir.join(".local/share"),
            app_default_dest_root: home_dir.join(".config"),
            home_dir: home_dir.clone(),
            shine_dir,
            presets_dir,
            bin_dir,
            overlay_dir: None,
            platform,
            shell: ShellType::default(),
            shell_config_paths: vec![home_dir.join(".zshrc")],
            external_shell_mode: ExternalShellMode::Snapshot,
            is_external_presets: false,
            allow_app_hooks: false,
            allow_sys_code: false,
            linux_split_dns_ready: true,
            running_as_admin: false,
            captured_unix_time: 0,
            env: BTreeMap::new(),
            path_env: None,
            proxy_env: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RuntimePlatform {
    Macos,
    Linux,
    Windows,
}

impl RuntimePlatform {
    pub const ALL: [Self; 3] = [Self::Macos, Self::Linux, Self::Windows];

    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else {
            Self::Linux
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Macos => "macos",
            Self::Linux => "linux",
            Self::Windows => "windows",
        }
    }

    pub const fn is_unix(self) -> bool {
        matches!(self, Self::Macos | Self::Linux)
    }
}

/// Existing lifecycle request inputs. This is deliberately not a Phase 3
/// reviewable Plan.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LifecycleRequest {
    pub target: Option<String>,
    pub dry_run: bool,
    pub force: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceInspection {
    pub logical_path: String,
    pub installed: bool,
    pub matches_snapshot: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RuntimeInspection {
    pub resources: Vec<ResourceInspection>,
}

/// Frontend-neutral runtime facade. Domain executors are added behind this
/// facade as their Phase 2 slices migrate.
pub struct CoreRuntime<H> {
    host: H,
    context: RuntimeContext,
    presets: PresetSnapshot,
}

impl<H> CoreRuntime<H> {
    pub fn new(host: H, context: RuntimeContext, presets: PresetSnapshot) -> Self {
        Self {
            host,
            context,
            presets,
        }
    }

    pub fn context(&self) -> &RuntimeContext {
        &self.context
    }

    /// Frontend adapter hook used while legacy configuration loading is being
    /// collapsed into one captured context. Domain executors never call this.
    pub fn context_mut_for_cli(&mut self) -> &mut RuntimeContext {
        &mut self.context
    }

    pub fn presets(&self) -> &PresetSnapshot {
        &self.presets
    }

    pub fn host(&self) -> &H {
        &self.host
    }

    pub fn into_host(self) -> H {
        self.host
    }

    pub fn validate(&self) -> PresetValidationReport {
        self.presets.validate()
    }
}

impl<H: FileSystemHost> CoreRuntime<H> {
    /// Inspect snapshot resources against a Shine-owned root. This generic
    /// harness seam is used without loading CLI configuration or real HOME.
    pub async fn inspect_snapshot(
        &self,
        installed_root: &Path,
    ) -> anyhow::Result<RuntimeInspection> {
        let mut inspection = RuntimeInspection::default();
        for (logical_path, expected) in self.presets.files() {
            let installed_path = installed_root.join(logical_path);
            let current = self.host.read(&installed_path).await;
            let (installed, matches_snapshot) = match current {
                Ok(current) => (true, current == *expected),
                Err(error) if error.is_not_found() => (false, false),
                Err(error) => return Err(error.into_anyhow("reading snapshot resource")),
            };
            inspection.resources.push(ResourceInspection {
                logical_path: logical_path.clone(),
                installed,
                matches_snapshot,
            });
        }
        Ok(inspection)
    }
}

pub use app::{
    AppArtifact, AppArtifactAction, AppArtifactRequest, AppCacheRequest, AppCategory,
    AppDestinationRoot, AppFile, AppFileAction, AppFileLifecycleReport, AppGenerator,
    AppGeneratorRequest, AppHook, AppHookPhase, AppHookReport, AppHookRequest, AppLifecycleReport,
    AppLifecycleRequest, AppListMode, AppRefreshRequest, AppUninstallLifecycleRequest,
    AppUpgradeLifecycleReport, AppUpgradeRequest, ArtifactRuntime,
};
pub use bootstrap::{
    PresetSnapshotRequest, PresetSnapshotSource, capture_embedded_preset_snapshot,
    capture_preset_snapshot,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> RuntimeContext {
        RuntimeContext::isolated(
            PathBuf::from("/home/test"),
            PathBuf::from("/home/test/.shine"),
            PathBuf::from("/home/test/.shine/presets"),
            PathBuf::from("/home/test/.shine/bin"),
            RuntimePlatform::Linux,
        )
    }

    #[tokio::test]
    async fn core_only_harness_validates_and_inspects_without_real_host_access() {
        let host = InMemoryHost::new();
        host.put_file("/installed/app/demo/config.toml", b"current".to_vec());
        let presets = PresetSnapshot::builder(PresetSourceKind::External)
            .file(
                "app/demo/shine.toml",
                b"[[files]]\nsource = \"config.toml\"\n".to_vec(),
            )
            .file("app/demo/config.toml", b"desired".to_vec())
            .build();
        let runtime = CoreRuntime::new(host, context(), presets);

        assert!(runtime.validate().valid);
        let inspection = runtime
            .inspect_snapshot(Path::new("/installed"))
            .await
            .unwrap();
        assert_eq!(inspection.resources.len(), 2);
        assert!(inspection.resources.iter().any(|row| {
            row.logical_path == "app/demo/config.toml" && row.installed && !row.matches_snapshot
        }));
    }
}
