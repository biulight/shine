//! Workspace-internal, frontend-neutral runtime seams.
//!
//! These APIs are public only so the `shine-cli` package can consume them.
//! They are not a stable third-party API in Roadmap Phase 2.

mod app;
mod host;
mod memory;
mod preset;
mod shell;
mod sys;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use host::{
    FileKind, FileMetadata, FileSystemHost, HostError, HostOperation, NullObserver, ProcessHost,
    ProcessOutput, ProcessRequest, RealHost, RuntimeEvent, RuntimeInteraction, RuntimeObserver,
};
pub use memory::InMemoryHost;
pub use preset::{PresetSnapshot, PresetSourceKind, PresetValidationIssue, PresetValidationReport};
pub use shell::{
    ExternalShellMode, LinkRuntime, SHELL_MANIFEST_FILE, SHELL_MANIFEST_SCHEMA_VERSION,
    ShellCategory, ShellFile, ShellManifest, ShellManifestEntry, ShellTarget, ShellType,
    parse_shell_lifecycle_target,
};
pub use sys::{
    ManagedFileReceipt, ManagedFileRemoveRequest, ManagedFileRequest, RECEIPT_VERSION,
    ResourceConflict, ResourceOutcome, ResourcePlan, SYS_MANIFEST_FILE,
    SYS_MANIFEST_SCHEMA_VERSION, SplitDnsReceipt, SysDriverKind, SysItemStatus, SysRunEntry,
    SysRunManifest, SystemReceipt,
};

/// Fully resolved runtime inputs. Domain code must not rediscover these from
/// ambient process state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeContext {
    pub home_dir: PathBuf,
    pub shine_dir: PathBuf,
    pub presets_dir: PathBuf,
    pub bin_dir: PathBuf,
    pub platform: RuntimePlatform,
    pub env: BTreeMap<String, String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> RuntimeContext {
        RuntimeContext {
            home_dir: PathBuf::from("/home/test"),
            shine_dir: PathBuf::from("/home/test/.shine"),
            presets_dir: PathBuf::from("/home/test/.shine/presets"),
            bin_dir: PathBuf::from("/home/test/.shine/bin"),
            platform: RuntimePlatform::Linux,
            env: BTreeMap::new(),
        }
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
pub use app::{
    AppArtifact, AppCategory, AppDestinationRoot, AppFile, AppGenerator, AppHook,
    AppInstallRequest, AppListMode, AppUninstallRequest, ArtifactRuntime, PreparedAppFile,
};
