//! Fixed provider argv, preview and administrator requirement; no Host effects.
use super::*;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ProviderInvocation {
    pub(super) program: String,
    pub(super) args: Vec<String>,
    pub(super) requires_admin: bool,
}

pub(in crate::runtime) fn sys_install_requires_admin(
    os_id: &str,
    install: &SysInstall,
    item: &SysItem,
) -> Result<bool> {
    match install {
        SysInstall::Package {
            provider, package, ..
        } => Ok(package_command(os_id, *provider, package, &BTreeMap::new())?.requires_admin),
        SysInstall::Script { .. } => Ok(item.requires_admin && os_id != "windows"),
    }
}
pub(super) fn install_requires_admin(
    os_id: &str,
    install: &SysInstall,
    item: &SysItem,
) -> Result<bool> {
    sys_install_requires_admin(os_id, install, item)
}
pub(super) fn package_preview(
    os_id: &str,
    provider: SysPackageProvider,
    package: &str,
) -> Result<String> {
    let ProviderInvocation {
        program,
        args,
        requires_admin: admin,
    } = package_command(os_id, provider, package, &BTreeMap::new())?;
    Ok(format!(
        "{}{} {}",
        if admin { "sudo " } else { "" },
        program,
        args.join(" ")
    ))
}
pub(super) fn package_command(
    os_id: &str,
    provider: SysPackageProvider,
    package: &str,
    proxy: &BTreeMap<String, String>,
) -> Result<ProviderInvocation> {
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
            Ok(ProviderInvocation {
                program: "brew".into(),
                args,
                requires_admin: false,
            })
        }
        SysPackageProvider::Apt => {
            if os_id != "ubuntu" {
                bail!("APT sys provider is supported only on Ubuntu presets");
            }
            Ok(ProviderInvocation {
                program: "apt-get".into(),
                args: vec!["install".into(), "-y".into(), package.into()],
                requires_admin: true,
            })
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
            Ok(ProviderInvocation {
                program: "winget".into(),
                args,
                requires_admin: false,
            })
        }
    }
}
