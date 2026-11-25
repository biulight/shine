//! Ordered CLI observations. All domains read from the same captured preset source.
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
    service: &mut ConfiguredFrontend,
    categories: &[crate::apps::AppCategory],
    run_generators: bool,
) -> Result<(
    Vec<AppRow>,
    shine_core::lifecycle::LifecycleResultV1,
    Vec<shine_core::runtime::AppFileInspection>,
)> {
    if !run_generators {
        return build_app_rows_with_lifecycle_options(service, categories, false).await;
    }
    let (static_rows, static_lifecycle, static_inspections) =
        build_app_rows_with_lifecycle_options(service, categories, false).await?;
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
    build_app_rows_with_lifecycle_options(service, &selected, true).await
}

async fn missing_app_sources(service: &ConfiguredFrontend) -> Result<Vec<(String, String)>> {
    service
        .missing_app_sources()
        .await
        .map_err(FrontendServiceError::into_source)
}

pub(super) async fn collect_update(
    service: &mut ConfiguredFrontend,
    run_generators: bool,
) -> Result<UpdateInputs> {
    let shell_rows = inspect_shell_rows(service).await?;
    let shell_lifecycle = crate::shells::shell_update_lifecycle_from_rows(&shell_rows);
    let cats = service
        .runtime()
        .app_categories(None)
        .context("loading active App Presets for update")?;
    let (app_rows, app_lifecycle, app_inspections) =
        build_update_app_rows(service, &cats, run_generators).await?;
    let update_sys = sys::managed_inspection(service)
        .await
        .context("checking managed Sys Presets for update")?
        .updates;
    let missing_sources = missing_app_sources(service).await?;
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
    service: &mut ConfiguredFrontend,
    shell_rows: Vec<ShellRow>,
    run_generators: bool,
) -> Result<StatusInputs> {
    let cats = service
        .runtime()
        .app_categories(None)
        .context("loading active App Presets for status")?;
    let (app_rows, _, app_inspections) =
        build_update_app_rows(service, &cats, run_generators).await?;
    let sys = sys::managed_inspection(service)
        .await
        .context("checking managed Sys Presets for status")?;
    let missing_sources = missing_app_sources(service).await?;
    Ok(StatusInputs {
        shell_rows,
        app_rows,
        app_inspections,
        sys,
        missing_sources,
    })
}

pub(super) async fn collect_inventory(service: &ConfiguredFrontend) -> Result<InventoryInputs> {
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

    let missing_sources = missing_app_sources(service).await?;
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
