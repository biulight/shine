//! Coordinate existing lifecycle owners; never treat the configuration directory as disposable.
use super::receipt;
use crate::{
    apps, config::Config, core_runtime, env, legacy_serve, output::escape_controls, shells, sys,
};
use anyhow::{Context, Result, bail, ensure};
use shine_core::{
    lifecycle::LifecycleOperation,
    plan::{PlanActionV1, PlanV1},
    runtime::{
        AppPlanRequest, CoreRuntime, FileSystemHost, RealHost, ShellPlanRequest, SysRunManifest,
    },
};
use std::{
    io::{IsTerminal, Read},
    path::{Path, PathBuf},
};

pub(crate) fn confirm_removal(prompt: &str, yes: bool) -> Result<()> {
    if yes {
        return Ok(());
    }
    ensure!(
        std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
        "approval requires a terminal; review --dry-run and pass --yes to apply"
    );
    ensure!(
        dialoguer::Confirm::new()
            .with_prompt(prompt)
            .default(false)
            .interact()?,
        "uninstall cancelled"
    );
    Ok(())
}

fn show(path: &Path) -> String {
    escape_controls(&path.display().to_string())
}

fn exists(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).with_context(|| format!("inspecting {}", show(path))),
    }
}

fn check_recovery(config: &Config) -> Result<()> {
    for (journal, domain) in [
        ("app-operation-journal.toml", "app"),
        ("app-secret-operation.json", "app"),
        ("shell-operation-journal.toml", "shell"),
        ("sys-operation-journal.toml", "sys"),
    ] {
        ensure!(
            !exists(&config.shine_dir().join(journal))?,
            "recovery required: review `shine {domain} recover`, then retry"
        );
    }
    Ok(())
}

#[derive(PartialEq, Eq)]
struct Work {
    app: bool,
    shell: bool,
    sys: Vec<String>,
    proxies: Vec<String>,
}
impl Work {
    fn any(&self) -> bool {
        self.app || self.shell || !self.sys.is_empty() || !self.proxies.is_empty()
    }
}

fn has_work(plan: PlanV1, domain: &str) -> Result<bool> {
    ensure!(
        plan.is_ready(),
        "{domain} uninstall is blocked; inspect `shine {domain} uninstall --dry-run`"
    );
    Ok(plan.steps.iter().any(|s| {
        s.action != PlanActionV1::None
            && !(s.action == PlanActionV1::Preserve
                && s.target == domain
                && s.resource.as_deref() == Some("preset-cache"))
    }))
}

async fn inventory(config: &Config, runtime: &CoreRuntime<RealHost>, os: &str) -> Result<Work> {
    check_recovery(config)?;
    let app = has_work(
        runtime
            .plan_apps(AppPlanRequest {
                operation: LifecycleOperation::Uninstall,
                target: None,
                purge: true,
                force: false,
                prune_stale: false,
                input_versions: Default::default(),
            })
            .await?,
        "app",
    )?;
    let shell = has_work(
        runtime
            .plan_shells(ShellPlanRequest {
                operation: LifecycleOperation::Uninstall,
                target: None,
                purge: true,
                force: false,
                input_versions: Default::default(),
            })
            .await?,
        "shell",
    )?;
    let mut sys = Vec::new();
    for entry in SysRunManifest::load(&RealHost, config.shine_dir())
        .await?
        .entries
    {
        ensure!(
            entry.os_id == os,
            "Sys records for {} remain; finish cleanup on their original platform",
            escape_controls(&entry.os_id)
        );
        if entry.managed || entry.receipt.is_some() {
            sys.push(entry.item_id);
        }
    }
    Ok(Work {
        app,
        shell,
        sys,
        proxies: env::proxy::installed_commands(config).await?,
    })
}

async fn run_domains(config: &Config, work: &Work, dry_run: bool, yes: bool) -> Result<()> {
    if work.app {
        apps::handle_uninstall_approved(config, None, false, dry_run, yes).await?;
    }
    if work.shell {
        shells::handle_uninstall_approved(config, None, dry_run, yes).await?;
    }
    for item in &work.sys {
        sys::handle_uninstall_approved(config, item, dry_run, yes).await?;
    }
    for proxy in &work.proxies {
        env::proxy::review_self_removal(config, proxy, dry_run, yes).await?;
    }
    Ok(())
}

// A receipt, grant or fingerprint key is collectible only after its owner is idle.
// Persistent lock files remain in place so waiters keep sharing the same inode.
const STATE: &[&str] = &[
    "app-secret-manifest.json",
    "app-secret-fingerprint.key",
    "shell-manifest.toml",
    "sys-manifest.toml",
    "proxy-manifest.toml",
    "trust.toml",
    "ssh-secret-broker.toml",
];
const LOCKS: &[&str] = &[
    "app-lifecycle.lock",
    "shell-lifecycle.lock",
    "sys-lifecycle.lock",
    "proxy-lifecycle.lock",
    "self-install.lock",
    "trust.lock",
    "ssh-secret-broker.lock",
];

#[derive(PartialEq, Eq)]
struct Cleanup {
    files: Vec<(PathBuf, String)>,
    loaders: Vec<(PathBuf, String, String)>,
    copies: Vec<receipt::CopyReceipt>,
    config_edits: Vec<(PathBuf, String, String)>,
}

fn regular_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    if !exists(path)? {
        return Ok(None);
    }
    const LIMIT: u64 = 16 * 1024 * 1024;
    let file = crate::env::workspace::file_groups::platform::open_read(path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file() && meta.len() <= LIMIT,
        "cannot safely inspect {}",
        show(path)
    );
    #[cfg(windows)]
    ensure!(
        !crate::env::workspace::file_groups::platform::is_link(&meta),
        "linked path: {}",
        show(path)
    );
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= LIMIT,
        "oversized file: {}",
        show(path)
    );
    Ok(Some(bytes))
}

impl Cleanup {
    async fn capture(config: &Config, runtime: &CoreRuntime<RealHost>, os: &str) -> Result<Self> {
        let mut files = Vec::new();
        let mut loaders = Vec::new();
        for name in STATE {
            let path = config.shine_dir().join(name);
            if let Some(bytes) = regular_bytes(&path)? {
                if *name == "app-secret-fingerprint.key" {
                    ensure!(
                        bytes.len() == 32,
                        "invalid App fingerprint key; preserve state and inspect before retrying"
                    );
                }
                files.push((path, shine_core::digest::sha256_hex(&bytes)));
            }
        }
        if files
            .iter()
            .any(|(p, _)| p.ends_with("ssh-secret-broker.toml"))
        {
            env::broker::load_store(config).await?;
        }
        // Sys startup loaders always address the default account directory.
        for path in shine_core::runtime::sys_profile_block_paths(runtime.context(), os)
            .into_iter()
            .filter(|_| config.shine_dir() == config.home_dir.join(".shine"))
        {
            if let Some(bytes) = regular_bytes(&path)? {
                let shell = if path.ends_with(".zshrc") {
                    "zsh"
                } else {
                    "bash"
                };
                let text = std::str::from_utf8(&bytes)
                    .with_context(|| format!("cannot inspect profile {}", show(&path)))?;
                if let Some(desired) = shine_core::runtime::detach_sys_loader(text, os, Some(shell))
                    .with_context(|| format!("preserving profile {}", show(&path)))?
                {
                    loaders.push((path, shine_core::digest::sha256_hex(&bytes), desired));
                }
            }
        }
        let copies = receipt::load(config)?;
        receipt::verify(&copies)?;
        let mut config_edits = Vec::new();
        let mut paths = vec![config.config_path().to_owned(), config.global_config_path()];
        paths.sort();
        paths.dedup();
        for path in paths.into_iter().filter(|_| !copies.is_empty()) {
            if let Some(bytes) = regular_bytes(&path)? {
                let mut doc: toml_edit::DocumentMut = std::str::from_utf8(&bytes)?
                    .parse()
                    .map_err(crate::config::config_parse_error)?;
                if doc
                    .get("self_install_dest")
                    .and_then(toml_edit::Item::as_str)
                    .is_some_and(|p| copies.iter().any(|c| c.path == Path::new(p)))
                {
                    doc.remove("self_install_dest");
                    config_edits.push((
                        path,
                        shine_core::digest::sha256_hex(&bytes),
                        doc.to_string(),
                    ));
                }
            }
        }
        Ok(Self {
            files,
            loaders,
            copies,
            config_edits,
        })
    }
    fn display(&self) {
        for (p, _, _) in &self.loaders {
            println!("Detach Sys loader: {}", show(p));
        }
        for (p, _) in &self.files {
            println!(
                "Remove installation state after managed cleanup: {}",
                show(p)
            );
        }
        for (p, _, _) in &self.config_edits {
            println!("Clear the recorded self_install_dest in {}", show(p));
        }
        for c in &self.copies {
            println!("Remove self-installed copy last: {}", show(&c.path));
        }
    }
    fn any(&self) -> bool {
        !self.files.is_empty() || !self.loaders.is_empty() || !self.copies.is_empty()
    }
}

async fn legacy_present(config: &Config) -> Result<bool> {
    let os = std::env::consts::OS;
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    if legacy_serve::has_registration(&RealHost, os, &config.home_dir, xdg.as_deref()).await? {
        return Ok(true);
    }
    legacy_serve::has_native_service(&RealHost, os).await.context(
        "could not verify the legacy HTTP service; retry from a session with access to the user service manager",
    )
}

fn program_handoff(current: &Path, copies: &[receipt::CopyReceipt]) {
    // Cargo owns its installation database. A PATH location alone is not provenance.
    let cargo_root = current
        .parent()
        .filter(|dir| dir.ends_with("bin"))
        .and_then(Path::parent);
    if let Some(root) = cargo_root
        && let Ok(Some(bytes)) = regular_bytes(&root.join(".crates.toml"))
        && let Ok(data) = toml::from_slice::<toml::Table>(&bytes)
        && data
            .get("v1")
            .and_then(toml::Value::as_table)
            .is_some_and(|v| {
                v.iter().any(|(key, bins)| {
                    key.starts_with("shine-cli ")
                        && bins.as_array().is_some_and(|a| {
                            a.iter()
                                .any(|b| b.as_str() == current.file_name().and_then(|p| p.to_str()))
                        })
                })
            })
    {
        println!(
            "Cargo owns {}. Finish with cargo uninstall --root <ROOT> shine-cli (ROOT: {}).",
            show(current),
            show(root)
        );
    } else {
        println!(
            "Program handoff: {} has no matching self-install receipt. Remove this copy through its original installation channel.",
            show(current)
        );
    }
    if let Some(path) = std::env::var_os("PATH") {
        let mut other = std::collections::BTreeSet::new();
        for dir in std::env::split_paths(&path).filter(|p| p.is_absolute()) {
            let entry = dir.join(crate::platform::current_executable_name());
            if let Ok(actual) = entry.canonicalize()
                && actual != current
                && actual.is_file()
                && !copies
                    .iter()
                    .any(|c| c.path.canonicalize().is_ok_and(|p| p == actual))
            {
                other.insert(entry);
            }
        }
        for entry in other {
            println!("Additional PATH copy (retained): {}", show(&entry));
        }
    }
}

pub async fn handle_self_uninstall(dry_run: bool, yes: bool) -> Result<()> {
    #[cfg(unix)]
    ensure!(
        std::env::var("SUDO_USER").map_or(true, |u| u == "root" || u.is_empty()),
        "run self uninstall as the installation user; domain operations request elevation separately"
    );
    let config = Config::load_read_only().await?;
    let runtime = core_runtime::from_config(&config).await?;
    let os = sys::detect_os_id().await?;
    let work = inventory(&config, &runtime, &os).await?;
    let preview = Cleanup::capture(&config, &runtime, &os).await?;
    println!(
        "Shine uninstall · configuration {}",
        show(config.shine_dir())
    );
    println!(
        "Keep personal configuration, tasks, external presets, encryption identities and software installed by bootstrap."
    );
    println!(
        "Keep custom Sys profiles and unrecognized files. Only the listed installation state is collected."
    );
    let legacy = legacy_present(&config).await?;
    if legacy {
        println!("Stop and remove the retired Shine HTTP user service; keep its published files.");
    }
    preview.display();
    let current = std::env::current_exe()?.canonicalize()?;
    let current_tracked = preview
        .copies
        .iter()
        .any(|c| c.path.canonicalize().is_ok_and(|p| p == current));
    if !current_tracked {
        program_handoff(&current, &preview.copies);
    }
    run_domains(&config, &work, true, false).await?;
    #[cfg(windows)]
    if current_tracked {
        println!(
            "Windows needs the running executable to exit. Run self uninstall from another Shine copy with the same configuration."
        );
        if !dry_run {
            bail!("current program is a removal target; retry from another copy");
        }
    }
    if dry_run {
        return Ok(());
    }
    if legacy {
        ensure!(
            config.shine_dir() == config.home_dir.join(".shine"),
            "legacy service is account-wide; verify and run shine clear serve separately"
        );
    }
    run_domains(&config, &work, false, yes).await?;
    if legacy {
        confirm_removal("Remove the displayed legacy service?", yes)?;
        legacy_serve::handle_uninstall().await?;
    }
    let runtime = core_runtime::from_config(&config).await?;
    ensure!(
        !inventory(&config, &runtime, &os).await?.any(),
        "managed work remains; retry before removing the program"
    );
    ensure!(
        !legacy_present(&config).await?,
        "legacy HTTP service still present; run shine clear serve and retry"
    );
    let cleanup = Cleanup::capture(&config, &runtime, &os).await?;
    ensure!(
        preview.copies == cleanup.copies,
        "self-installed copies changed; retry"
    );
    if cleanup.any() {
        cleanup.display();
        confirm_removal("Apply the displayed final cleanup?", yes)?;
        let mut guards = Vec::new();
        for name in LOCKS {
            guards.push(
                RealHost
                    .acquire_operation_lock(&config.shine_dir().join(name))
                    .await?,
            );
        }
        let mut config_guards = Vec::new();
        for (path, _, _) in &cleanup.config_edits {
            config_guards.push(crate::config::env_write::lock_target(path).await?);
        }
        let current_config = Config::load_read_only().await?;
        ensure!(
            current_config.shine_dir() == config.shine_dir(),
            "configuration root changed; retry"
        );
        let runtime = core_runtime::from_config(&current_config).await?;
        ensure!(
            !inventory(&current_config, &runtime, &os).await?.any()
                && Cleanup::capture(&current_config, &runtime, &os).await? == cleanup,
            "installation changed during review; retry"
        );
        ensure!(
            !legacy_present(&current_config).await?,
            "legacy HTTP service changed; retry"
        );
        ensure!(
            runtime.secret_store()?.entries.is_empty(),
            "secret receipts remain; keep the fingerprint key"
        );
        for (path, hash, text) in &cleanup.loaders {
            verify_file(path, hash)?;
            rewrite_existing(path, text.as_bytes(), false).await?;
        }
        for (path, hash) in &cleanup.files {
            verify_file(path, hash)?;
            std::fs::remove_file(path).with_context(|| format!("removing {}", show(path)))?;
        }
        for (path, hash, text) in &cleanup.config_edits {
            verify_file(path, hash)?;
            rewrite_existing(path, text.as_bytes(), true).await?;
        }
        receipt::remove(&config, &cleanup.copies)?;
    }
    if !current_tracked && cleanup.copies.is_empty() {
        bail!(
            "managed cleanup complete; program removal requires the installation channel shown above"
        );
    }
    if current_tracked {
        println!("Shine uninstall complete. Personal data retained.");
    } else {
        println!(
            "Recorded installation removed. The invoking copy remains at {}.",
            show(&current)
        );
    }
    Ok(())
}

#[cfg(unix)]
async fn rewrite_existing(path: &Path, bytes: &[u8], private: bool) -> Result<()> {
    if private {
        crate::persist::atomic_write_private(path, bytes).await
    } else {
        crate::persist::atomic_write(path, bytes).await
    }
}

#[cfg(windows)]
async fn rewrite_existing(path: &Path, bytes: &[u8], _private: bool) -> Result<()> {
    crate::persist::write_profile_like(path, bytes, path).await
}

fn verify_file(path: &Path, hash: &str) -> Result<()> {
    ensure!(
        regular_bytes(path)?.is_some_and(|b| shine_core::digest::sha256_hex(&b) == hash),
        "{} changed during cleanup; retry",
        show(path)
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[tokio::test]
    async fn rewriting_profile_preserves_owner_group_and_dacl() {
        let dir = crate::test_support::make_temp_dir("shine-self-profile-acl").await;
        let path = dir.join("profile.ps1");
        std::fs::write(&path, b"# profile").unwrap();
        let security = |script: &str| {
            let out = std::process::Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command", script])
                .env("SHINE_FIXTURE_PROFILE", &path)
                .output()
                .unwrap();
            assert!(out.status.success(), "{out:?}");
            out.stdout
        };
        security(
            "$ErrorActionPreference='Stop'; $a=Get-Acl -LiteralPath $env:SHINE_FIXTURE_PROFILE; $a.SetAccessRuleProtection($true,$true); Set-Acl -LiteralPath $env:SHINE_FIXTURE_PROFILE -AclObject $a",
        );
        let query = "(Get-Acl -LiteralPath $env:SHINE_FIXTURE_PROFILE).Sddl";
        let before = security(query);
        let stream = dir.join("profile.ps1:personal");
        std::fs::write(&stream, b"stream").unwrap();
        super::rewrite_existing(&path, b"# retained", false)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"# retained");
        assert_eq!(security(query), before);
        assert_eq!(std::fs::read(&stream).unwrap(), b"stream");
        let rollback = dir.join("rollback");
        std::fs::rename(&path, &rollback).unwrap();
        crate::persist::write_profile_like(&path, b"# reconciled", &rollback)
            .await
            .unwrap();
        assert_eq!(security(query), before);
        assert_eq!(std::fs::read(&stream).unwrap(), b"stream");
        assert_eq!(std::fs::read(&rollback).unwrap(), b"# retained");
        assert_eq!(
            std::fs::read(dir.join("rollback:personal")).unwrap(),
            b"stream"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn detach_only_exact_sys_loader_and_preserve_surrounding_bytes() {
        let block = "# >>> shine ubuntu sys pre >>>\r\nshine_ubuntu_sys_profile=\"$HOME/.shine/profile/ubuntu-sys.pre.sh\"\r\nif [[ -f \"$shine_ubuntu_sys_profile\" ]]; then\r\n  SHINE_UBUNTU_SYS_SHELL=\"bash\"\r\n  source \"$shine_ubuntu_sys_profile\"\r\nfi\r\n# <<< shine ubuntu sys pre <<<\r\n";
        let text = format!("\u{feff}{block}\r\nuser command\r\n");
        assert_eq!(
            shine_core::runtime::detach_sys_loader(&text, "ubuntu", Some("bash"))
                .unwrap()
                .unwrap(),
            "\u{feff}\r\nuser command\r\n"
        );
        for invalid in [
            text.replace("source ", "exec "),
            text.replace("sys pre <<<", "other <<<"),
            format!("{block}{block}"),
        ] {
            assert!(
                shine_core::runtime::detach_sys_loader(&invalid, "ubuntu", Some("bash")).is_err()
            );
        }
        assert!(
            shine_core::runtime::detach_sys_loader("user content", "ubuntu", Some("bash"))
                .unwrap()
                .is_none()
        );
    }
}
