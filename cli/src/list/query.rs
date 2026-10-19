//! Ordered CLI observations. Preserve capture count, error contexts and generator opt-in.
use super::*;
use shine_core::{
    lifecycle::LifecycleResultV1,
    runtime::{AppFileInspection, SysUpdateRow},
};

pub(super) struct UpdateInputs {
    pub shell_rows: Vec<ShellRow>,
    pub shell_lifecycle: LifecycleResultV1,
    pub app_rows: Vec<AppRow>,
    pub app_lifecycle: LifecycleResultV1,
    pub app_inspections: Vec<AppFileInspection>,
    pub update_sys: Vec<SysUpdateRow>,
    pub missing_sources: Vec<(String, String)>,
}
pub(super) struct StatusInputs {
    pub shell_rows: Vec<ShellRow>,
    pub app_rows: Vec<AppRow>,
    pub app_inspections: Vec<AppFileInspection>,
    pub sys: shine_core::frontend::SysInspection,
    pub missing_sources: Vec<(String, String)>,
}
pub(super) struct InventoryInputs {
    pub installed_shell: Vec<String>,
    pub installed_app: Vec<String>,
    pub installed_sys: Vec<String>,
    pub missing_sources: Vec<(String, String)>,
}

async fn build_update_app_rows(
    config: &Config,
    categories: &[crate::apps::AppCategory],
    run_generators: bool,
) -> Result<(
    Vec<AppRow>,
    shine_core::lifecycle::LifecycleResultV1,
    Vec<shine_core::runtime::AppFileInspection>,
)> {
    if !run_generators {
        return build_app_rows_with_lifecycle_options(config, categories, false).await;
    }
    let (static_rows, static_lifecycle, static_inspections) =
        build_app_rows_with_lifecycle_options(config, categories, false).await?;
    let installed = static_rows
        .iter()
        .filter(|row| row.file_status != FileStatus::NotInstalled)
        .map(|row| row.category.as_str())
        .collect::<BTreeSet<_>>();
    let selected = categories
        .iter()
        .filter(|category| installed.contains(category.name.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Ok((static_rows, static_lifecycle, static_inspections));
    }
    build_app_rows_with_lifecycle_options(config, &selected, true).await
}

async fn missing_app_sources(config: &Config) -> Result<Vec<(String, String)>> {
    crate::core_runtime::frontend_from_config(config)
        .await?
        .missing_app_sources()
        .await
        .map_err(FrontendServiceError::into_source)
}

pub(super) async fn collect_update(config: &Config, run_generators: bool) -> Result<UpdateInputs> {
    let shell_rows = build_shell_rows(config).await?;
    // Retain the existing second capture used for lifecycle pending targets.
    let shell_lifecycle = crate::shells::collect_update_lifecycle_result(config).await?;
    let cats = load_active_categories(config, None)
        .await
        .context("loading active App Presets for update")?;
    let (app_rows, app_lifecycle, app_inspections) =
        build_update_app_rows(config, &cats, run_generators).await?;
    let update_sys = sys::managed_updates(config)
        .await
        .context("checking managed Sys Presets for update")?;
    let missing_sources = missing_app_sources(config).await?;
    Ok(UpdateInputs {
        shell_rows,
        shell_lifecycle,
        app_rows,
        app_lifecycle,
        app_inspections,
        update_sys,
        missing_sources,
    })
}

// Status prints Shell attention before loading App/Sys; the facade owns that boundary.
pub(super) async fn collect_status(
    config: &Config,
    shell_rows: Vec<ShellRow>,
    run_generators: bool,
) -> Result<StatusInputs> {
    let cats = load_active_categories(config, None)
        .await
        .context("loading active App Presets for status")?;
    let (app_rows, _, app_inspections) =
        build_update_app_rows(config, &cats, run_generators).await?;
    let sys = sys::managed_inspection(config)
        .await
        .context("checking managed Sys Presets for status")?;
    let missing_sources = missing_app_sources(config).await?;
    Ok(StatusInputs {
        shell_rows,
        app_rows,
        app_inspections,
        sys,
        missing_sources,
    })
}

pub(super) async fn collect_diffs(
    config: &Config,
    app_inspections: Vec<AppFileInspection>,
) -> Result<UpdateDiffs> {
    UpdateDiffs::collect_with_app_inspections(config, app_inspections).await
}

pub(super) async fn collect_inventory(config: &Config) -> Result<InventoryInputs> {
    let service = crate::core_runtime::frontend_from_config(config).await?;
    let shell_inventory = service
        .inventory(InventoryRequest::for_kind(CapabilityKindV1::Shell))
        .await
        .map_err(FrontendServiceError::into_source)?;
    let installed_shell = installed_shell_categories_from_inventory(&shell_inventory);

    let installed_app = match service
        .inventory(InventoryRequest::for_kind(CapabilityKindV1::App))
        .await
    {
        Ok(report) => installed_app_categories_from_inventory(&report),
        Err(error) if error.diagnostic().code == "frontend_inventory_app_source_failed" => {
            Vec::new()
        }
        Err(error) => return Err(error.into_source()),
    };

    let os_id = sys::detect_os_id().await?;
    let sys_inventory = service
        .inventory(
            InventoryRequest::for_kind(CapabilityKindV1::Sys)
                .with_sys_os_id(os_id)
                .installed_only(),
        )
        .await
        .map_err(FrontendServiceError::into_source)?;
    let installed_sys = installed_sys_items_from_inventory(&sys_inventory);

    let missing_sources = missing_app_sources(config).await?;
    let installed_shell = sorted_names(installed_shell);
    let installed_app = sorted_names(installed_app);
    let installed_sys = sorted_names(installed_sys);

    Ok(InventoryInputs {
        installed_shell,
        installed_app,
        installed_sys,
        missing_sources,
    })
}
