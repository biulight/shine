//! CLI assembly adapter for the frontend-neutral Core runtime.
//!
//! `rust-embed` and CLI configuration stay in this crate. Shared bootstrap
//! discovers external/overlay trees through the host port; every input used by
//! domain logic is captured before `CoreRuntime` is constructed.

use crate::config::Config;
use anyhow::{Context, Result};
use directories::BaseDirs;
use shine_core::runtime::{
    CoreRuntime, PresetSnapshotRequest, PresetSnapshotSource, RealHost, RuntimeContext,
    RuntimePlatform, capture_embedded_preset_snapshot,
};

pub(crate) async fn frontend_from_config(
    config: &Config,
) -> Result<shine_core::frontend::FrontendService<RealHost>> {
    from_config(config)
        .await
        .map(shine_core::frontend::FrontendService::new)
}

pub(crate) async fn from_config(config: &Config) -> Result<CoreRuntime<RealHost>> {
    from_config_with_preset_mode(config, config.is_external_presets).await
}

pub(crate) async fn from_installed_presets(config: &Config) -> Result<CoreRuntime<RealHost>> {
    from_config_with_preset_mode(config, true).await
}

async fn from_config_with_preset_mode(
    config: &Config,
    is_external_presets: bool,
) -> Result<CoreRuntime<RealHost>> {
    let bases = BaseDirs::new().context("resolving platform runtime directories")?;
    // Resolve lexical paths before any child changes its working directory.
    // Do not canonicalize: missing paths and symlink dispatch identities are valid.
    let absolute = |path: &std::path::Path| {
        std::path::absolute(path).context("resolving absolute runtime path")
    };
    let presets_dir = absolute(config.presets_dir())?;
    let overlay_root = config
        .active_presets_overlay_dir()
        .map(absolute)
        .transpose()?;
    let source = if is_external_presets {
        PresetSnapshotSource::External(presets_dir.clone())
    } else {
        PresetSnapshotSource::Embedded(embedded_preset_files())
    };
    let context = RuntimeContext {
        home_dir: absolute(&config.home_dir)?,
        shine_dir: absolute(config.shine_dir())?,
        presets_dir,
        bin_dir: absolute(config.bin_dir())?,
        cache_dir: bases.cache_dir().to_path_buf(),
        data_dir: bases.data_dir().to_path_buf(),
        app_default_dest_root: absolute(&config.app_default_dest_root())?,
        overlay_dir: overlay_root.clone(),
        platform: RuntimePlatform::current(),
        shell: config.shell_type,
        shell_config_paths: crate::shells::shell_config_paths_for_core(
            &config.shell_type,
            &config.home_dir,
        )?
        .iter()
        .map(|path| absolute(path))
        .collect::<Result<Vec<_>>>()?,
        external_shell_mode: config.external_shell_mode,
        is_external_presets,
        trust_grants: crate::trust::load_store(config).await?.grants,
        linux_split_dns_ready: systemd_resolved_stub_active(),
        running_as_admin: cfg!(windows) || std::env::var("USER").is_ok_and(|user| user == "root"),
        captured_unix_time: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        env: config.env.clone(),
        path_env: std::env::var("PATH").ok(),
        proxy_env: proxy_env(&config.env),
    };
    shine_core::frontend::FrontendService::capture(
        RealHost,
        context,
        PresetSnapshotRequest {
            source,
            overlay_root,
        },
    )
    .await
    .map(shine_core::frontend::FrontendService::into_runtime)
    .map_err(shine_core::frontend::FrontendServiceError::into_source)
}

pub(crate) fn from_embedded_presets() -> CoreRuntime<RealHost> {
    from_embedded_presets_for_platform(RuntimePlatform::current())
}

pub(crate) fn from_embedded_presets_for_platform(
    platform: RuntimePlatform,
) -> CoreRuntime<RealHost> {
    let snapshot = capture_embedded_preset_snapshot(embedded_preset_files());
    let root = std::path::PathBuf::from(".shine-core-embedded");
    let mut context = RuntimeContext::isolated(
        root.join("home"),
        root.join("home/.shine"),
        root.join("presets"),
        root.join("home/.shine/bin"),
        platform,
    );
    context.shell = if platform == RuntimePlatform::Windows {
        shine_core::runtime::ShellType::PowerShell
    } else {
        shine_core::runtime::ShellType::Zsh
    };
    context.shell_config_paths =
        crate::shells::shell_config_paths_for_core(&context.shell, &context.home_dir)
            .expect("built-in shell config paths");
    CoreRuntime::new(RealHost, context, snapshot)
}

fn systemd_resolved_stub_active() -> bool {
    if !cfg!(target_os = "linux") {
        return true;
    }
    std::fs::read_to_string("/run/systemd/resolve/stub-resolv.conf")
        .map(|content| {
            content
                .lines()
                .any(|line| line.trim_start().starts_with("nameserver 127.0.0.53"))
        })
        .unwrap_or(true)
}

pub(crate) fn embedded_preset_files() -> Vec<(String, Vec<u8>)> {
    crate::presets::embedded_asset_paths("")
        .into_iter()
        .filter_map(|logical| {
            crate::presets::read_embedded_asset_bytes(&logical).map(|bytes| (logical, bytes))
        })
        .collect()
}

fn proxy_env(
    env: &std::collections::BTreeMap<String, String>,
) -> std::collections::BTreeMap<String, String> {
    ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY"]
        .into_iter()
        .filter_map(|key| env.get(key).map(|value| (key.to_string(), value.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shine_core::runtime::{PlanningInputVersions, SysBootstrapPlanRequest};

    #[cfg(unix)]
    #[tokio::test]
    async fn relative_config_directory_runs_captured_app_generator() {
        use shine_core::install::{AppEntry, AppInstallStrategy, AppManifest, hash_content};
        use shine_core::plan::PlanApprovalV1;
        use shine_core::runtime::*;
        struct Interaction;
        impl RuntimeInteraction for Interaction {
            fn confirm(&mut self, _: &'static str, default: bool) -> Result<bool> {
                Ok(default)
            }
            fn authorize_admin<'a>(
                &'a mut self,
                _: usize,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool>> + Send + 'a>>
            {
                Box::pin(async { Ok(false) })
            }
            fn select_many(
                &mut self,
                _: &'static str,
                _: &[String],
                defaults: &[String],
            ) -> Result<Vec<String>> {
                Ok(defaults.to_vec())
            }
        }
        let relative =
            std::path::PathBuf::from(format!("target/relative-app-{}", uuid::Uuid::new_v4()));
        let root = std::path::absolute(&relative).unwrap();
        let config = Config::new_for_test(&relative);
        let context = from_config(&config).await.unwrap().context().clone();
        for path in [
            &context.home_dir,
            &context.shine_dir,
            &context.presets_dir,
            &context.bin_dir,
            &context.app_default_dest_root,
        ] {
            assert!(path.is_absolute(), "{}", path.display());
        }
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                br#"metadata_schema_version = 2
dest = '~/.config/demo'
[permissions]
schema_version = 1
environment = [{ name = 'SOURCE', sensitivity = 'plain' }]
[[files]]
source = 'generated.txt'
generator = { script = 'gen.sh', env = ['SOURCE'], when_env = 'SOURCE', auto = false }
"#
                .to_vec(),
            )
            .file("app/demo/generated.txt", b"fallback".to_vec())
            .file("app/demo/gen.sh", b"#!/bin/sh\nprintf generated\n".to_vec())
            .build();
        let mut context = context;
        context.env.insert("SOURCE".into(), "yes".into());
        let runtime = CoreRuntime::new(RealHost, context, snapshot);
        let destination = root.join(".config/demo/generated.txt");
        runtime
            .host()
            .write_atomic(&destination, b"installed")
            .await
            .unwrap();
        AppManifest {
            schema_version: 1,
            entries: vec![AppEntry {
                source: "app/demo/generated.txt".into(),
                destination: destination.clone(),
                backup: None,
                content_hash: hash_content(b"installed"),
                install_strategy: AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            }],
        }
        .save(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
        let request = AppRefreshPlanRequest {
            category: "demo".into(),
            file: None,
            force: false,
            input_versions: PlanningInputVersions::default(),
        };
        let plan = runtime.plan_app_refresh(request.clone()).await.unwrap();
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        let report = runtime
            .refresh_app_generators_approved(
                request,
                &approval,
                &mut NullObserver,
                &mut Interaction,
            )
            .await
            .unwrap();
        assert!(
            report.files.iter().all(|file| file.error.is_none()),
            "{report:?}"
        );
        assert_eq!(
            runtime.host().read(&destination).await.unwrap(),
            b"generated"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn adapter_captures_embedded_presets_without_exposing_rust_embed_to_core() {
        let root =
            std::env::temp_dir().join(format!("shine-core-adapter-{}", uuid::Uuid::new_v4()));
        let config = Config::new_for_test(&root);
        let runtime = from_config(&config).await.unwrap();
        assert!(runtime.presets().get("app/ghostty/shine.toml").is_some());
        assert_eq!(runtime.context().home_dir, root);
    }

    #[tokio::test]
    async fn built_in_sys_bootstrap_plan_is_ready_for_each_platform() {
        for (platform, os_id, shell) in [
            (RuntimePlatform::Macos, "macos", "zsh"),
            (RuntimePlatform::Linux, "ubuntu", "zsh"),
            (RuntimePlatform::Windows, "windows", "powershell"),
        ] {
            let runtime = from_embedded_presets_for_platform(platform);
            let plan = runtime
                .plan_sys_bootstrap(SysBootstrapPlanRequest {
                    os_id: os_id.to_string(),
                    item_ids: vec!["rust".to_string()],
                    sys_shell: shell.to_string(),
                    force_profile: false,
                    input_versions: PlanningInputVersions::default(),
                })
                .await
                .unwrap();
            assert!(
                plan.is_ready(),
                "blocked built-in Plan for {os_id}: {plan:#?}"
            );
        }
    }
}
