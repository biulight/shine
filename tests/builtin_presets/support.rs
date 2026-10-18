use shine_core::runtime::{
    CoreRuntime, RealHost, RuntimeContext, RuntimePlatform, ShellType,
    capture_embedded_preset_snapshot,
};

pub(super) fn runtime(platform: RuntimePlatform) -> CoreRuntime<RealHost> {
    let files = cli::presets::embedded_asset_paths("")
        .into_iter()
        .map(|path| {
            let bytes = cli::presets::read_embedded_asset_bytes(&path)
                .expect("enumerated embedded asset must be readable");
            (path, bytes)
        })
        .collect();
    let root =
        std::env::temp_dir().join(format!("shine-builtin-contracts-{}", uuid::Uuid::new_v4()));
    let mut context = RuntimeContext::isolated(
        root.join("home"),
        root.join("state"),
        root.join("presets"),
        root.join("bin"),
        platform,
    );
    context.shell = if platform == RuntimePlatform::Windows {
        ShellType::PowerShell
    } else {
        ShellType::Zsh
    };
    // Use the production profile selector; no config initialization or host mutations.
    context.shell_config_paths = vec![
        cli::shells::get_shell_config_path(&context.shell, &context.home_dir)
            .expect("profile path"),
    ];
    // The CLI also captures the legacy Windows PowerShell profile on Windows hosts.
    #[cfg(windows)]
    if platform == RuntimePlatform::Windows {
        context.shell_config_paths.push(
            context
                .home_dir
                .join("Documents/WindowsPowerShell/Microsoft.PowerShell_profile.ps1"),
        );
    }
    CoreRuntime::new(RealHost, context, capture_embedded_preset_snapshot(files))
}
