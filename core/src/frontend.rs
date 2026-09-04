//! Versioned, frontend-neutral application-service contracts.
//!
//! This module is public only so workspace adapters can share one service.
//! It is not a general remote API or a stability promise for [`CoreRuntime`].

use crate::install::AppManifest;
use crate::runtime::{
    CoreRuntime, FileSystemObservationHost, ShellManifest, SysRunManifest, command_path_for_name,
};
use anyhow::Error;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fmt;

pub const INVENTORY_REPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityKindV1 {
    App,
    Shell,
    Sys,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FrontendDiagnosticSeverityV1 {
    Error,
    Warning,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FrontendDiagnosticV1 {
    pub code: String,
    pub severity: FrontendDiagnosticSeverityV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

impl FrontendDiagnosticV1 {
    fn error(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            severity: FrontendDiagnosticSeverityV1::Error,
            target: None,
        }
    }

    fn warning(code: impl Into<String>, target: Option<String>) -> Self {
        Self {
            code: code.into(),
            severity: FrontendDiagnosticSeverityV1::Warning,
            target,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapabilityInventoryItemV1 {
    pub target: String,
    pub kind: CapabilityKindV1,
    pub available: bool,
    pub installed: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InventoryReportV1 {
    pub schema_version: u32,
    pub items: Vec<CapabilityInventoryItemV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<FrontendDiagnosticV1>,
}

impl Default for InventoryReportV1 {
    fn default() -> Self {
        Self {
            schema_version: INVENTORY_REPORT_SCHEMA_VERSION,
            items: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct InventoryRequest {
    kinds: BTreeSet<CapabilityKindV1>,
    include_available: bool,
    sys_os_id: Option<String>,
}

impl InventoryRequest {
    pub fn for_kind(kind: CapabilityKindV1) -> Self {
        Self {
            kinds: BTreeSet::from([kind]),
            include_available: true,
            sys_os_id: None,
        }
    }

    pub fn all() -> Self {
        Self {
            kinds: BTreeSet::from([
                CapabilityKindV1::App,
                CapabilityKindV1::Shell,
                CapabilityKindV1::Sys,
            ]),
            include_available: true,
            sys_os_id: None,
        }
    }

    pub fn installed_only(mut self) -> Self {
        self.include_available = false;
        self
    }

    pub fn with_sys_os_id(mut self, os_id: impl Into<String>) -> Self {
        self.sys_os_id = Some(os_id.into());
        self
    }

    fn includes(&self, kind: CapabilityKindV1) -> bool {
        self.kinds.contains(&kind)
    }
}

/// A safe stable diagnostic plus a local-only source error.
///
/// The source supports trusted local presentation and troubleshooting. It is
/// deliberately not serializable and must not be copied into a wire report.
#[derive(Debug)]
pub struct FrontendServiceError {
    diagnostic: FrontendDiagnosticV1,
    source: Error,
}

impl FrontendServiceError {
    fn new(code: &'static str, source: impl Into<Error>) -> Self {
        Self {
            diagnostic: FrontendDiagnosticV1::error(code),
            source: source.into(),
        }
    }

    pub fn diagnostic(&self) -> &FrontendDiagnosticV1 {
        &self.diagnostic
    }

    pub fn into_source(self) -> Error {
        self.source
    }
}

impl fmt::Display for FrontendServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.diagnostic.code)
    }
}

impl std::error::Error for FrontendServiceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

/// Frontend-neutral service over one fully captured Core runtime.
pub struct FrontendService<H> {
    runtime: CoreRuntime<H>,
}

impl<H> FrontendService<H> {
    pub fn new(runtime: CoreRuntime<H>) -> Self {
        Self { runtime }
    }

    pub fn runtime(&self) -> &CoreRuntime<H> {
        &self.runtime
    }

    pub fn into_runtime(self) -> CoreRuntime<H> {
        self.runtime
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct InventoryState {
    available: bool,
    installed: bool,
}

impl<H: FileSystemObservationHost> FrontendService<H> {
    /// Inventory capabilities without executing Preset code or mutating host state.
    pub async fn inventory(
        &self,
        request: InventoryRequest,
    ) -> Result<InventoryReportV1, FrontendServiceError> {
        let mut states = BTreeMap::<(CapabilityKindV1, String), InventoryState>::new();
        let mut diagnostics = Vec::new();

        if request.includes(CapabilityKindV1::App) {
            self.inventory_apps(request.include_available, &mut states, &mut diagnostics)
                .await?;
        }
        if request.includes(CapabilityKindV1::Shell) {
            self.inventory_shells(request.include_available, &mut states, &mut diagnostics)
                .await?;
        }
        if request.includes(CapabilityKindV1::Sys) {
            let os_id = request.sys_os_id.as_deref().ok_or_else(|| {
                FrontendServiceError::new(
                    "frontend_inventory_sys_os_required",
                    anyhow::anyhow!("a captured Sys OS identity is required for inventory"),
                )
            })?;
            self.inventory_sys(
                os_id,
                request.include_available,
                &mut states,
                &mut diagnostics,
            )
            .await?;
        }

        let items = states
            .into_iter()
            .filter_map(|((kind, target), state)| {
                (request.include_available || state.installed).then_some(
                    CapabilityInventoryItemV1 {
                        target,
                        kind,
                        available: state.available,
                        installed: state.installed,
                    },
                )
            })
            .collect();
        diagnostics.sort_by(|left, right| {
            left.severity
                .cmp(&right.severity)
                .then_with(|| left.target.cmp(&right.target))
                .then_with(|| left.code.cmp(&right.code))
        });
        diagnostics.dedup();
        Ok(InventoryReportV1 {
            schema_version: INVENTORY_REPORT_SCHEMA_VERSION,
            items,
            diagnostics,
        })
    }

    async fn inventory_apps(
        &self,
        include_available: bool,
        states: &mut BTreeMap<(CapabilityKindV1, String), InventoryState>,
        diagnostics: &mut Vec<FrontendDiagnosticV1>,
    ) -> Result<(), FrontendServiceError> {
        if include_available {
            let categories = self.runtime.app_categories(None).map_err(|error| {
                FrontendServiceError::new("frontend_inventory_app_source_failed", error)
            })?;
            for category in categories {
                mark_available(
                    states,
                    CapabilityKindV1::App,
                    format!("app/{}", category.name),
                );
            }
        }

        let manifest = AppManifest::load(self.runtime.host(), &self.runtime.context().shine_dir)
            .await
            .map_err(|error| {
                FrontendServiceError::new("frontend_inventory_app_state_failed", error)
            })?;
        for entry in manifest.entries {
            let Some(category) = app_manifest_category(&entry.source) else {
                diagnostics.push(FrontendDiagnosticV1::warning(
                    "frontend_inventory_receipt_target_invalid",
                    None,
                ));
                continue;
            };
            mark_installed(
                states,
                diagnostics,
                CapabilityKindV1::App,
                format!("app/{category}"),
                include_available,
            );
        }
        Ok(())
    }

    async fn inventory_shells(
        &self,
        include_available: bool,
        states: &mut BTreeMap<(CapabilityKindV1, String), InventoryState>,
        diagnostics: &mut Vec<FrontendDiagnosticV1>,
    ) -> Result<(), FrontendServiceError> {
        let categories = self.runtime.shell_categories(None).map_err(|error| {
            FrontendServiceError::new("frontend_inventory_shell_source_failed", error)
        })?;
        let manifest = ShellManifest::load(self.runtime.host(), &self.runtime.context().shine_dir)
            .await
            .map_err(|error| {
                FrontendServiceError::new("frontend_inventory_shell_state_failed", error)
            })?;

        for category in categories {
            for file in category.files {
                let target = format!("shell/{}/{}", category.name, file.command_name);
                if include_available {
                    mark_available(states, CapabilityKindV1::Shell, target.clone());
                }
                if manifest.find(&target).is_some()
                    || self
                        .runtime
                        .host()
                        .metadata(&command_path_for_name(
                            &self.runtime.context().bin_dir,
                            OsStr::new(&file.command_name),
                        ))
                        .await
                        .is_ok()
                {
                    mark_installed(
                        states,
                        diagnostics,
                        CapabilityKindV1::Shell,
                        target,
                        include_available,
                    );
                }
            }
        }
        for entry in manifest.entries {
            if !valid_segment(&entry.category) || !valid_segment(&entry.command) {
                diagnostics.push(FrontendDiagnosticV1::warning(
                    "frontend_inventory_receipt_target_invalid",
                    None,
                ));
                continue;
            }
            mark_installed(
                states,
                diagnostics,
                CapabilityKindV1::Shell,
                format!("shell/{}/{}", entry.category, entry.command),
                include_available,
            );
        }
        Ok(())
    }

    async fn inventory_sys(
        &self,
        os_id: &str,
        include_available: bool,
        states: &mut BTreeMap<(CapabilityKindV1, String), InventoryState>,
        diagnostics: &mut Vec<FrontendDiagnosticV1>,
    ) -> Result<(), FrontendServiceError> {
        if include_available {
            let preset = self.runtime.load_sys_preset(os_id).await.map_err(|error| {
                FrontendServiceError::new("frontend_inventory_sys_source_failed", error)
            })?;
            for item in preset.manifest.items {
                mark_available(states, CapabilityKindV1::Sys, format!("sys/{}", item.id));
            }
        }

        let manifest = SysRunManifest::load(self.runtime.host(), &self.runtime.context().shine_dir)
            .await
            .map_err(|error| {
                FrontendServiceError::new("frontend_inventory_sys_state_failed", error)
            })?;
        for entry in manifest
            .entries
            .into_iter()
            .filter(|entry| entry.os_id == os_id && entry.managed)
        {
            if !valid_segment(&entry.item_id) {
                diagnostics.push(FrontendDiagnosticV1::warning(
                    "frontend_inventory_receipt_target_invalid",
                    None,
                ));
                continue;
            }
            mark_installed(
                states,
                diagnostics,
                CapabilityKindV1::Sys,
                format!("sys/{}", entry.item_id),
                include_available,
            );
        }
        Ok(())
    }
}

fn mark_available(
    states: &mut BTreeMap<(CapabilityKindV1, String), InventoryState>,
    kind: CapabilityKindV1,
    target: String,
) {
    states.entry((kind, target)).or_default().available = true;
}

fn mark_installed(
    states: &mut BTreeMap<(CapabilityKindV1, String), InventoryState>,
    diagnostics: &mut Vec<FrontendDiagnosticV1>,
    kind: CapabilityKindV1,
    target: String,
    source_was_observed: bool,
) {
    let state = states.entry((kind, target.clone())).or_default();
    let newly_installed = !state.installed;
    state.installed = true;
    if newly_installed && source_was_observed && !state.available {
        diagnostics.push(FrontendDiagnosticV1::warning(
            "frontend_inventory_preset_missing",
            Some(target),
        ));
    }
}

fn app_manifest_category(source: &str) -> Option<&str> {
    let mut parts = source.splitn(3, '/');
    (parts.next()? == "app")
        .then(|| parts.next())
        .flatten()
        .filter(|category| valid_segment(category))
        .filter(|_| parts.next().is_some_and(|resource| !resource.is_empty()))
}

fn valid_segment(value: &str) -> bool {
    !value.is_empty() && !matches!(value, "." | "..") && !value.contains(['/', '\\'])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::{AppEntry, AppInstallStrategy};
    use crate::runtime::{
        InMemoryHost, PresetSnapshot, PresetSourceKind, RuntimeContext, RuntimePlatform,
        ShellManifestEntry, SysItemStatus, SysRunEntry,
    };
    use std::path::PathBuf;

    fn runtime(host: InMemoryHost, presets: PresetSnapshot) -> CoreRuntime<InMemoryHost> {
        CoreRuntime::new(
            host,
            RuntimeContext::isolated(
                PathBuf::from("/home/test"),
                PathBuf::from("/home/test/.shine"),
                PathBuf::from("/presets"),
                PathBuf::from("/home/test/.shine/bin"),
                RuntimePlatform::Linux,
            ),
            presets,
        )
    }

    fn snapshot() -> PresetSnapshot {
        PresetSnapshot::builder(PresetSourceKind::External)
            .file(
                "app/available/shine.toml",
                b"dest = \"~/.config/available\"\n[[files]]\nsource = \"config.toml\"\n"
                    .to_vec(),
            )
            .file("app/available/config.toml", b"value = true\n".to_vec())
            .file(
                "shell/tools/shine.toml",
                b"[[files]]\nsource = \"tool.sh\"\ntarget = \"tool\"\npermissions = { schema_version = 1 }\n"
                    .to_vec(),
            )
            .file("shell/tools/tool.sh", b"echo tool\n".to_vec())
            .file(
                "sys/ubuntu/shine.toml",
                b"version = 2\n[[items]]\nid = \"managed\"\nlabel = \"Managed\"\nmode = \"managed\"\ndriver = \"managed-file\"\npermissions = { schema_version = 1 }\n[items.config]\nsource = \"managed.txt\"\ntarget = \"~/.config/managed\"\n"
                    .to_vec(),
            )
            .file("sys/ubuntu/managed.txt", b"managed\n".to_vec())
            .build()
    }

    fn empty_snapshot() -> PresetSnapshot {
        PresetSnapshot::builder(PresetSourceKind::External).build()
    }

    async fn seed_manifests(host: &InMemoryHost) {
        let app = AppManifest {
            entries: vec![AppEntry {
                source: "app/orphan/config.toml".to_string(),
                destination: PathBuf::from("/private/orphan-config"),
                backup: None,
                content_hash: 1,
                install_strategy: AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            }],
            ..AppManifest::default()
        };
        app.save(host, &PathBuf::from("/home/test/.shine"))
            .await
            .unwrap();
        let shell = ShellManifest {
            entries: vec![ShellManifestEntry {
                category: "tools".to_string(),
                command: "tool".to_string(),
                mode: crate::runtime::ExternalShellMode::Snapshot,
                source_path: PathBuf::from("/private/tool.sh"),
                rendered_path: PathBuf::from("/private/rendered-tool.sh"),
                runtime: "native".to_string(),
                bun_dependencies: None,
                dependency_hash: None,
                transforms: Vec::new(),
                env: Vec::new(),
                needs_source: false,
                content_hash: 1,
            }],
            ..ShellManifest::default()
        };
        shell
            .save(host, &PathBuf::from("/home/test/.shine"))
            .await
            .unwrap();
        let mut sys = SysRunManifest::default();
        sys.entries.push(SysRunEntry {
            os_id: "ubuntu".to_string(),
            item_id: "orphan-sys".to_string(),
            label: "Orphan Sys".to_string(),
            status: SysItemStatus::Installed,
            detail: String::new(),
            updated_at: "2026-09-04T00:00:00Z".to_string(),
            managed: true,
            profile_enabled: true,
            receipt: None,
        });
        sys.save(host, &PathBuf::from("/home/test/.shine"))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn inventory_unifies_available_installed_and_manifest_only_targets() {
        let host = InMemoryHost::new();
        seed_manifests(&host).await;
        let service = FrontendService::new(runtime(host, snapshot()));

        let report = service
            .inventory(InventoryRequest::all().with_sys_os_id("ubuntu"))
            .await
            .unwrap();

        assert_eq!(report.schema_version, INVENTORY_REPORT_SCHEMA_VERSION);
        assert_eq!(
            report
                .items
                .iter()
                .map(|item| (&item.target, item.available, item.installed))
                .collect::<Vec<_>>(),
            vec![
                (&"app/available".to_string(), true, false),
                (&"app/orphan".to_string(), false, true),
                (&"shell/tools/tool".to_string(), true, true),
                (&"sys/managed".to_string(), true, false),
                (&"sys/orphan-sys".to_string(), false, true),
            ]
        );
        assert_eq!(
            report
                .diagnostics
                .iter()
                .filter_map(|diagnostic| diagnostic.target.as_deref())
                .collect::<Vec<_>>(),
            ["app/orphan", "sys/orphan-sys"]
        );
    }

    #[tokio::test]
    async fn installed_only_inventory_does_not_claim_source_observation() {
        let host = InMemoryHost::new();
        seed_manifests(&host).await;
        let service = FrontendService::new(runtime(host, empty_snapshot()));

        let report = service
            .inventory(
                InventoryRequest::for_kind(CapabilityKindV1::Sys)
                    .with_sys_os_id("ubuntu")
                    .installed_only(),
            )
            .await
            .unwrap();

        assert_eq!(report.items.len(), 1);
        assert!(!report.items[0].available);
        assert!(report.items[0].installed);
        assert!(report.diagnostics.is_empty());
    }

    #[tokio::test]
    async fn overlay_only_category_uses_the_effective_snapshot() {
        let presets = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .overlay_root("/private/overlay")
            .overlay_file(
                "app/overlay-only/shine.toml",
                b"dest = \"~/.config/overlay-only\"\n[[files]]\nsource = \"config.toml\"\n"
                    .to_vec(),
            )
            .overlay_file("app/overlay-only/config.toml", b"value = true\n".to_vec())
            .build();
        let service = FrontendService::new(runtime(InMemoryHost::new(), presets));

        let report = service
            .inventory(InventoryRequest::for_kind(CapabilityKindV1::App))
            .await
            .unwrap();

        assert_eq!(report.items.len(), 1);
        assert_eq!(report.items[0].target, "app/overlay-only");
        assert!(report.items[0].available);
        assert!(!report.items[0].installed);
        assert!(
            !serde_json::to_string(&report)
                .unwrap()
                .contains("/private/overlay")
        );
    }

    #[tokio::test]
    async fn empty_inventory_is_stable() {
        let service = FrontendService::new(runtime(InMemoryHost::new(), empty_snapshot()));

        let report = service
            .inventory(InventoryRequest::for_kind(CapabilityKindV1::App))
            .await
            .unwrap();

        assert_eq!(report, InventoryReportV1::default());
    }

    #[test]
    fn inventory_json_contract_is_stable_and_redacted() {
        let report = InventoryReportV1 {
            schema_version: INVENTORY_REPORT_SCHEMA_VERSION,
            items: vec![CapabilityInventoryItemV1 {
                target: "app/demo".to_string(),
                kind: CapabilityKindV1::App,
                available: false,
                installed: true,
            }],
            diagnostics: vec![FrontendDiagnosticV1::warning(
                "frontend_inventory_preset_missing",
                Some("app/demo".to_string()),
            )],
        };

        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            serde_json::json!({
                "schema_version": 1,
                "items": [{
                    "target": "app/demo",
                    "kind": "app",
                    "available": false,
                    "installed": true
                }],
                "diagnostics": [{
                    "code": "frontend_inventory_preset_missing",
                    "severity": "warning",
                    "target": "app/demo"
                }]
            })
        );
        let encoded = serde_json::to_string(&report).unwrap();
        for private in [
            "/home/test",
            "/private/orphan-config",
            "rendered-tool.sh",
            "SECRET=value",
            "--password",
        ] {
            assert!(!encoded.contains(private));
        }
    }

    #[test]
    fn service_error_serializes_only_through_its_safe_diagnostic() {
        let error = FrontendServiceError::new(
            "frontend_inventory_app_state_failed",
            anyhow::anyhow!("failed at /private/app-manifest.toml with SECRET=value"),
        );

        let encoded = serde_json::to_string(error.diagnostic()).unwrap();
        assert!(encoded.contains("frontend_inventory_app_state_failed"));
        assert!(!encoded.contains("/private"));
        assert!(!encoded.contains("SECRET"));
    }
}
