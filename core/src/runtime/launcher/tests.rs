use super::*;
use crate::runtime::RealHost;

#[cfg(windows)]
#[tokio::test]
async fn windows_verbatim_roots_retain_launcher_ownership_and_foreign_pair_protection() {
    use crate::runtime::{HostOperation, InMemoryHost};
    use std::os::windows::ffi::OsStringExt;

    for root in [
        Path::new(r"\\?\C:\shine\installed\shell\demo"),
        Path::new(r"\\?\UNC\server\share\shine\installed\shell\demo"),
    ] {
        let bin = root.parent().unwrap().join("bin");
        let spec = bun_spec(&root.join("tool.ts"), "tool");
        let resources = prepare_launcher_resources(&bin, &spec);
        let host = InMemoryHost::new();
        for resource in &resources {
            apply_prepared_launcher_resource(&host, resource)
                .await
                .unwrap();
        }
        let before = host.operations().len();
        let probe = probe_managed_command_with_host(&host, &bin, &spec.link_name, &[root.into()])
            .await
            .unwrap();
        assert!(
            probe.conflicts.is_empty(),
            "{root:?}: {:?}",
            probe.conflicts
        );
        assert!(
            probe
                .resources
                .iter()
                .map(|r| r.destination())
                .eq(resources.iter().map(|r| r.destination()))
        );
        assert!(
            host.operations()[before..]
                .iter()
                .all(|op| matches!(op, HostOperation::Read(_)))
        );

        let foreign = bun_spec(&root.with_file_name("demo-foreign").join("tool.ts"), "tool");
        let foreign_resources = prepare_launcher_resources(&bin, &foreign);
        // One foreign member must protect both Windows shim files, including
        // when its marker's root shares the entire managed directory prefix.
        for (resource, foreign_resource) in resources.iter().zip(&foreign_resources) {
            apply_prepared_launcher_resource(&host, foreign_resource)
                .await
                .unwrap();
            let before = host.operations().len();
            let probe =
                probe_managed_command_with_host(&host, &bin, &spec.link_name, &[root.into()])
                    .await
                    .unwrap();
            assert!(probe.resources.is_empty());
            assert_eq!(probe.conflicts, [resource.destination()]);
            assert!(
                host.operations()[before..]
                    .iter()
                    .all(|op| matches!(op, HostOperation::Read(_)))
            );
            apply_prepared_launcher_resource(&host, resource)
                .await
                .unwrap();
        }
    }
    let root = PathBuf::from(std::ffi::OsString::from_wide(&[
        b'C' as u16,
        b':' as u16,
        b'\\' as u16,
        0xd800,
    ]));
    assert!(!target_is_managed(
        Path::new("C:\\\u{fffd}\\tool.ts"),
        &root,
        Path::new(r"C:\bin")
    ));
}

#[test]
fn native_cmd_paths_preserve_apostrophes_and_legacy_reconstruction() {
    for source in [
        r"C:\Users\O'Connor\.shine\presets\shell\demo\run.ps1",
        r"\\?\C:\Users\O'Connor\.shine\presets\shell\demo\run.ps1",
        r"\\?\UNC\server\O'Connor\run.ps1",
    ] {
        let source = Path::new(source);
        let legacy = cmd_shim_content(
            source,
            LinkRuntime::Native,
            "run",
            BunDependencyMode::Disabled,
            &[],
            None,
        );
        let target = windows_native_path(source);
        assert!(legacy.contains(&format!("-File \"{}\"", target.replace('\'', "''"))));
        let corrected = correct_native_cmd_path(legacy.clone(), source);
        assert!(corrected.contains(&format!("-File \"{target}\"")));
        assert_eq!(
            shim_target_from_content(&legacy),
            shim_target_from_content(&corrected)
        );
        assert_ne!(legacy, corrected);
    }
}

#[cfg(windows)]
#[test]
fn native_cmd_shim_executes_powershell_file_in_apostrophe_directory() {
    let root = std::env::temp_dir()
        .join(format!("shine-cmd-{}", uuid::Uuid::new_v4()))
        .join("O'Connor");
    std::fs::create_dir_all(root.join("payload")).unwrap();
    let source = root.join("payload/probe.ps1");
    std::fs::write(&source, "[Console]::Write($args[0]); exit 17\n").unwrap();
    let spec = LinkSpec {
        native_cmd_literal_path: true,
        live_launch_config: None,
        source,
        link_name: "probe".into(),
        runtime: LinkRuntime::Native,
        bun_dependencies: BunDependencyMode::Disabled,
        env: Vec::new(),
        render_target: None,
    };
    for resource in prepare_launcher_resources(&root, &spec) {
        if let PreparedLauncherResource::File {
            destination, bytes, ..
        } = resource
        {
            std::fs::write(destination, bytes).unwrap();
        }
    }
    let output = std::process::Command::new("cmd.exe")
        .args(["/D", "/C"])
        .arg(root.join("probe.cmd"))
        .arg("sentinel")
        .output()
        .unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    assert_eq!(output.status.code(), Some(17), "{output:?}");
    assert_eq!(output.stdout, b"sentinel", "{output:?}");
}

#[test]
fn live_bun_windows_templates_bind_config_and_do_not_render_twice() {
    let source = Path::new(r"C:\state\rendered\shell\demo\run.ts");
    let config = Path::new(r"C:\state with 'quotes' $x %PATH% !x! &\.shine");
    let ps = powershell_live_bun_launch_content(source, config, "shell/demo/run");
    let cmd = cmd_live_bun_launch_content(source, config, "shell/demo/run");
    assert!(ps.contains("'--config-dir', 'C:\\state with ''quotes'' $x %PATH% !x! &\\.shine'"));
    assert!(ps.contains("'--') + @($args)"));
    assert!(ps.contains("$start.UseShellExecute = $false"));
    assert!(cmd.contains("%%PATH%%"));
    assert!(cmd.contains("DisableDelayedExpansion"));
    assert!(cmd.contains("-- %*"));
    assert!(cmd.starts_with("@echo off\r\ngoto :shine_launch\r\n"));
    assert!(cmd.contains("\r\n:shine_launch\r\nsetlocal"));
    assert_eq!(shim_target_from_content(&cmd).as_deref(), Some(source));
    for text in [ps, cmd] {
        assert!(text.contains("__shell-launch"));
        assert!(!text.contains("__shell-render"));
        assert!(!text.contains("env run"));
    }
}

#[cfg(unix)]
use tokio::fs;

#[test]
fn recognizes_powershell_unix_and_cmd_managed_markers() {
    assert!(has_shim_managed_marker("# shine-managed\n"));
    assert!(has_shim_managed_marker("REM shine-managed\r\n"));
    assert!(!has_shim_managed_marker("shine-managed\n"));
}

#[cfg(unix)]
fn native_specs(sources: &[PathBuf]) -> Vec<LinkSpec> {
    sources
        .iter()
        .map(|source| LinkSpec {
            native_cmd_literal_path: false,
            live_launch_config: None,
            source: source.clone(),
            link_name: link_stem(source),
            runtime: LinkRuntime::Native,
            bun_dependencies: BunDependencyMode::Disabled,
            env: Vec::new(),
            render_target: None,
        })
        .collect()
}

#[cfg(unix)]
async fn make_dirs() -> (PathBuf, PathBuf) {
    let id = uuid::Uuid::new_v4();
    let src_dir = std::env::temp_dir().join(format!("shine-bl-src-{id}"));
    let bin_dir = std::env::temp_dir().join(format!("shine-bl-bin-{id}"));
    fs::create_dir_all(&src_dir).await.unwrap();
    fs::create_dir_all(&bin_dir).await.unwrap();
    (src_dir, bin_dir)
}

/// Write a file and set the executable bit for the Host executable-bit check.
#[cfg(unix)]
async fn make_executable(dir: &Path, name: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    fs::write(&path, b"#!/bin/sh\n").await.unwrap();
    let mut perms = fs::metadata(&path).await.unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).await.unwrap();
    path
}

#[cfg(unix)]
async fn make_plain(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, b"data").await.unwrap();
    path
}

#[cfg(unix)]
#[tokio::test]
async fn creates_symlink_for_executable_source() {
    let (src, bin) = make_dirs().await;
    let exe = make_executable(&src, "run.sh").await;

    let report = link_executables_with_host(
        &RealHost,
        &bin,
        &native_specs(std::slice::from_ref(&exe)),
        false,
    )
    .await
    .unwrap();

    assert_eq!(report.created.len(), 1);
    let link = &report.created[0];
    assert!(link.is_symlink());
    assert_eq!(fs::read_link(link).await.unwrap(), exe);
    // symlink name is the stem, not the full filename
    assert_eq!(link.file_name().unwrap(), "run");

    fs::remove_dir_all(&src).await.unwrap();
    fs::remove_dir_all(&bin).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn skips_non_executable_source() {
    let (src, bin) = make_dirs().await;
    let plain = make_plain(&src, "readme.txt").await;

    let report = link_executables_with_host(&RealHost, &bin, &native_specs(&[plain]), false)
        .await
        .unwrap();

    assert!(report.created.is_empty());
    assert!(report.skipped.is_empty());

    fs::remove_dir_all(&src).await.unwrap();
    fs::remove_dir_all(&bin).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn skips_when_correct_symlink_already_exists() {
    let (src, bin) = make_dirs().await;
    let exe = make_executable(&src, "run.sh").await;
    tokio::fs::symlink(&exe, bin.join("run")).await.unwrap();

    let report = link_executables_with_host(
        &RealHost,
        &bin,
        &native_specs(std::slice::from_ref(&exe)),
        false,
    )
    .await
    .unwrap();

    assert!(report.created.is_empty());
    assert_eq!(report.skipped.len(), 1);

    fs::remove_dir_all(&src).await.unwrap();
    fs::remove_dir_all(&bin).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn reports_conflict_when_regular_file_exists() {
    let (src, bin) = make_dirs().await;
    let exe = make_executable(&src, "run.sh").await;
    make_plain(&bin, "run").await;

    let report = link_executables_with_host(
        &RealHost,
        &bin,
        &native_specs(std::slice::from_ref(&exe)),
        false,
    )
    .await
    .unwrap();

    assert!(report.created.is_empty());
    assert_eq!(report.conflicts.len(), 1);
    assert_eq!(report.conflicts[0].link_path, bin.join("run"));
    assert_eq!(report.conflicts[0].source, exe);
    assert_eq!(report.conflicts[0].kind, LinkConflictKind::ExistingEntry);

    fs::remove_dir_all(&src).await.unwrap();
    fs::remove_dir_all(&bin).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn overwrites_stale_symlink_when_overwrite_true() {
    let (src, bin) = make_dirs().await;
    let exe = make_executable(&src, "run.sh").await;
    let other = make_executable(&src, "other.sh").await;
    tokio::fs::symlink(&other, bin.join("run")).await.unwrap();

    let report = link_executables_with_host(
        &RealHost,
        &bin,
        &native_specs(std::slice::from_ref(&exe)),
        true,
    )
    .await
    .unwrap();

    assert_eq!(report.overwritten.len(), 1);
    assert_eq!(fs::read_link(bin.join("run")).await.unwrap(), exe);

    fs::remove_dir_all(&src).await.unwrap();
    fs::remove_dir_all(&bin).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn flattens_nested_preset_path_into_bin_dir() {
    let (src, bin) = make_dirs().await;
    let sub = src.join("shell").join("proxy");
    fs::create_dir_all(&sub).await.unwrap();
    let exe = {
        use std::os::unix::fs::PermissionsExt;
        let path = sub.join("set_proxy.sh");
        fs::write(&path, b"#!/bin/sh\n").await.unwrap();
        let mut perms = fs::metadata(&path).await.unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).await.unwrap();
        path
    };

    let report = link_executables_with_host(&RealHost, &bin, &native_specs(&[exe]), false)
        .await
        .unwrap();

    assert_eq!(report.created.len(), 1);
    assert!(bin.join("set_proxy").exists());

    fs::remove_dir_all(&src).await.unwrap();
    fs::remove_dir_all(&bin).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn reports_collision_when_two_sources_share_basename() {
    let (src, bin) = make_dirs().await;
    let sub1 = src.join("a");
    let sub2 = src.join("b");
    fs::create_dir_all(&sub1).await.unwrap();
    fs::create_dir_all(&sub2).await.unwrap();
    let exe1 = make_executable(&sub1, "run.sh").await;
    let exe2 = make_executable(&sub2, "run.sh").await;

    let report =
        link_executables_with_host(&RealHost, &bin, &native_specs(&[exe1, exe2.clone()]), false)
            .await
            .unwrap();

    assert_eq!(report.created.len(), 1);
    assert_eq!(report.conflicts.len(), 1);
    assert_eq!(report.conflicts[0].link_path, bin.join("run"));
    assert_eq!(report.conflicts[0].source, exe2);
    assert_eq!(report.conflicts[0].kind, LinkConflictKind::DuplicateName);

    fs::remove_dir_all(&src).await.unwrap();
    fs::remove_dir_all(&bin).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn creates_symlink_with_explicit_link_name() {
    let (src, bin) = make_dirs().await;
    let exe = make_executable(&src, "set_proxy.sh").await;
    let specs = [LinkSpec {
        native_cmd_literal_path: false,
        live_launch_config: None,

        source: exe.clone(),
        link_name: OsString::from("setproxy"),
        runtime: LinkRuntime::Native,
        bun_dependencies: BunDependencyMode::Disabled,
        env: Vec::new(),
        render_target: None,
    }];

    let report = link_executables_with_host(&RealHost, &bin, &specs, false)
        .await
        .unwrap();

    assert_eq!(report.created.len(), 1);
    assert!(bin.join("setproxy").exists());
    assert!(!bin.join("set_proxy").exists());
    assert_eq!(fs::read_link(bin.join("setproxy")).await.unwrap(), exe);

    fs::remove_dir_all(&src).await.unwrap();
    fs::remove_dir_all(&bin).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn links_non_executable_shell_script_source() {
    let (src, bin) = make_dirs().await;
    let script = src.join("set_proxy.sh");
    fs::write(&script, b"#!/bin/sh\n").await.unwrap();
    let specs = [LinkSpec {
        native_cmd_literal_path: false,
        live_launch_config: None,

        source: script.clone(),
        link_name: OsString::from("setproxy"),
        runtime: LinkRuntime::Native,
        bun_dependencies: BunDependencyMode::Disabled,
        env: Vec::new(),
        render_target: None,
    }];

    let report = link_executables_with_host(&RealHost, &bin, &specs, false)
        .await
        .unwrap();

    assert_eq!(report.created.len(), 1);
    assert!(bin.join("setproxy").exists());
    assert_eq!(fs::read_link(bin.join("setproxy")).await.unwrap(), script);

    fs::remove_dir_all(&src).await.unwrap();
    fs::remove_dir_all(&bin).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn skips_non_executable_non_script_source_with_custom_name() {
    let (src, bin) = make_dirs().await;
    let plain = make_plain(&src, "proxy.txt").await;
    let specs = [LinkSpec {
        native_cmd_literal_path: false,
        live_launch_config: None,

        source: plain,
        link_name: OsString::from("setproxy"),
        runtime: LinkRuntime::Native,
        bun_dependencies: BunDependencyMode::Disabled,
        env: Vec::new(),
        render_target: None,
    }];

    let report = link_executables_with_host(&RealHost, &bin, &specs, false)
        .await
        .unwrap();

    assert!(report.created.is_empty());
    assert!(!bin.join("setproxy").exists());

    fs::remove_dir_all(&src).await.unwrap();
    fs::remove_dir_all(&bin).await.unwrap();
}

// --- unlink_managed tests ---

#[cfg(unix)]
#[tokio::test]
async fn unlink_removes_symlink_pointing_into_managed_root() {
    let (src, bin) = make_dirs().await;
    let exe = make_executable(&src, "run.sh").await;
    tokio::fs::symlink(&exe, bin.join("run.sh")).await.unwrap();

    let report = unlink_managed_command_with_host(
        &RealHost,
        &bin,
        OsStr::new("run.sh"),
        std::slice::from_ref(&src),
        false,
    )
    .await
    .unwrap();

    assert_eq!(report.removed.len(), 1);
    assert!(!bin.join("run.sh").exists());

    fs::remove_dir_all(&src).await.unwrap();
    fs::remove_dir_all(&bin).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn unlink_skips_symlink_outside_managed_root() {
    let (src, bin) = make_dirs().await;
    let outside = std::env::temp_dir().join(format!("shine-bl-out-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&outside).await.unwrap();
    let exe = make_executable(&outside, "run.sh").await;
    tokio::fs::symlink(&exe, bin.join("run.sh")).await.unwrap();

    let report = unlink_managed_command_with_host(
        &RealHost,
        &bin,
        OsStr::new("run.sh"),
        std::slice::from_ref(&src),
        false,
    )
    .await
    .unwrap();

    assert_eq!(report.skipped.len(), 1);
    assert!(bin.join("run.sh").is_symlink());

    fs::remove_dir_all(&src).await.unwrap();
    fs::remove_dir_all(&bin).await.unwrap();
    fs::remove_dir_all(&outside).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn unlink_skips_regular_files_in_bin_dir() {
    let (src, bin) = make_dirs().await;
    make_plain(&bin, "user_script.sh").await;

    let report = unlink_managed_command_with_host(
        &RealHost,
        &bin,
        OsStr::new("user_script.sh"),
        std::slice::from_ref(&src),
        false,
    )
    .await
    .unwrap();

    assert!(report.removed.is_empty());
    assert_eq!(report.skipped.len(), 1);
    assert!(bin.join("user_script.sh").exists());

    fs::remove_dir_all(&src).await.unwrap();
    fs::remove_dir_all(&bin).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn unlink_dry_run_reports_but_does_not_remove() {
    let (src, bin) = make_dirs().await;
    let exe = make_executable(&src, "run.sh").await;
    tokio::fs::symlink(&exe, bin.join("run.sh")).await.unwrap();

    let report = unlink_managed_command_with_host(
        &RealHost,
        &bin,
        OsStr::new("run.sh"),
        std::slice::from_ref(&src),
        true,
    )
    .await
    .unwrap();

    assert_eq!(report.removed.len(), 1);
    assert!(bin.join("run.sh").is_symlink(), "dry-run must not remove");

    fs::remove_dir_all(&src).await.unwrap();
    fs::remove_dir_all(&bin).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn unlink_is_idempotent_on_empty_bin_dir() {
    let (src, bin) = make_dirs().await;

    let r1 = unlink_managed_command_with_host(
        &RealHost,
        &bin,
        OsStr::new("run"),
        std::slice::from_ref(&src),
        false,
    )
    .await
    .unwrap();
    let r2 = unlink_managed_command_with_host(
        &RealHost,
        &bin,
        OsStr::new("run"),
        std::slice::from_ref(&src),
        false,
    )
    .await
    .unwrap();

    assert!(r1.removed.is_empty());
    assert!(r2.removed.is_empty());

    fs::remove_dir_all(&src).await.unwrap();
    fs::remove_dir_all(&bin).await.unwrap();
}

#[tokio::test]
async fn unlink_returns_empty_report_when_bin_dir_missing() {
    let missing = std::env::temp_dir().join(format!("shine-bl-miss-{}", uuid::Uuid::new_v4()));
    let managed = std::env::temp_dir().join(format!("shine-bl-mgd-{}", uuid::Uuid::new_v4()));

    let report =
        unlink_managed_command_with_host(&RealHost, &missing, OsStr::new("run"), &[managed], false)
            .await
            .unwrap();

    assert!(report.removed.is_empty());
    assert!(report.skipped.is_empty());
}

#[test]
fn link_stem_strips_bun_extensions() {
    assert_eq!(link_stem(Path::new("tool.ts")), OsString::from("tool"));
    assert_eq!(link_stem(Path::new("tool.js")), OsString::from("tool"));
    assert_eq!(link_stem(Path::new("tool.mts")), OsString::from("tool"));
    assert_eq!(link_stem(Path::new("tool.mjs")), OsString::from("tool"));
}

fn bun_spec(source: &Path, name: &str) -> LinkSpec {
    bun_spec_with_env(source, name, Vec::new())
}

#[cfg(unix)]
fn locked_bun_spec(source: &Path, name: &str) -> LinkSpec {
    LinkSpec {
        native_cmd_literal_path: false,
        live_launch_config: None,

        source: source.to_path_buf(),
        link_name: OsString::from(name),
        runtime: LinkRuntime::Bun,
        bun_dependencies: BunDependencyMode::Locked,
        env: Vec::new(),
        render_target: None,
    }
}

fn bun_spec_with_env(source: &Path, name: &str, env: Vec<String>) -> LinkSpec {
    LinkSpec {
        native_cmd_literal_path: false,
        live_launch_config: None,

        source: source.to_path_buf(),
        link_name: OsString::from(name),
        runtime: LinkRuntime::Bun,
        bun_dependencies: BunDependencyMode::Disabled,
        env,
        render_target: None,
    }
}

#[cfg(unix)]
#[tokio::test]
async fn creates_bun_launcher_as_marked_executable_regular_file() {
    use std::os::unix::fs::PermissionsExt;
    let (src, bin) = make_dirs().await;
    let script = src.join("tool.ts");
    fs::write(&script, b"console.log('hi')\n").await.unwrap();

    let report = link_executables_with_host(&RealHost, &bin, &[bun_spec(&script, "tool")], false)
        .await
        .unwrap();

    assert_eq!(report.created.len(), 1);
    let launcher = bin.join("tool");
    assert!(launcher.exists());
    assert!(
        !launcher.is_symlink(),
        "bun launcher must be a regular file, not a symlink"
    );
    let content = fs::read_to_string(&launcher).await.unwrap();
    assert!(content.contains("# shine-managed"));
    assert!(content.contains(&format!("# shine-target: {}", script.display())));
    assert!(content.contains("command -v bun"));
    assert!(content.contains("exit 127"));
    assert!(content.contains(&format!(
        "exec bun --no-install '{}' \"$@\"",
        script.display()
    )));
    let mode = fs::metadata(&launcher).await.unwrap().permissions().mode();
    assert!(mode & 0o111 != 0, "launcher must be executable");

    fs::remove_dir_all(&src).await.unwrap();
    fs::remove_dir_all(&bin).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn bun_launcher_is_idempotent_and_refreshes_when_stale() {
    let (src, bin) = make_dirs().await;
    let script = src.join("tool.ts");
    fs::write(&script, b"console.log('hi')\n").await.unwrap();

    link_executables_with_host(&RealHost, &bin, &[bun_spec(&script, "tool")], false)
        .await
        .unwrap();
    let again = link_executables_with_host(&RealHost, &bin, &[bun_spec(&script, "tool")], false)
        .await
        .unwrap();
    assert_eq!(
        again.skipped.len(),
        1,
        "identical launcher should be skipped"
    );
    assert!(again.created.is_empty());
    assert!(again.overwritten.is_empty());

    // Same marker + target but different body → stale, refreshed without --force.
    let launcher = bin.join("tool");
    fs::write(
        &launcher,
        format!(
            "#!/usr/bin/env bash\n# shine-managed\n# shine-target: {}\necho stale\n",
            script.display()
        ),
    )
    .await
    .unwrap();
    let refreshed =
        link_executables_with_host(&RealHost, &bin, &[bun_spec(&script, "tool")], false)
            .await
            .unwrap();
    assert_eq!(
        refreshed.overwritten.len(),
        1,
        "stale launcher should refresh"
    );
    assert!(
        fs::read_to_string(&launcher)
            .await
            .unwrap()
            .contains("exec bun")
    );

    fs::remove_dir_all(&src).await.unwrap();
    fs::remove_dir_all(&bin).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn bun_launcher_conflicts_with_user_file_unless_forced() {
    let (src, bin) = make_dirs().await;
    let script = src.join("tool.ts");
    fs::write(&script, b"console.log('hi')\n").await.unwrap();
    // A user's own file at the same command name, no managed marker.
    make_plain(&bin, "tool").await;

    let report = link_executables_with_host(&RealHost, &bin, &[bun_spec(&script, "tool")], false)
        .await
        .unwrap();
    assert_eq!(report.conflicts.len(), 1);
    assert_eq!(report.conflicts[0].kind, LinkConflictKind::ExistingEntry);
    assert_eq!(fs::read_to_string(bin.join("tool")).await.unwrap(), "data");

    let forced = link_executables_with_host(&RealHost, &bin, &[bun_spec(&script, "tool")], true)
        .await
        .unwrap();
    assert_eq!(forced.overwritten.len(), 1);
    assert!(
        fs::read_to_string(bin.join("tool"))
            .await
            .unwrap()
            .contains("exec bun")
    );

    fs::remove_dir_all(&src).await.unwrap();
    fs::remove_dir_all(&bin).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn unlink_removes_managed_bun_launcher_but_skips_user_file() {
    let (src, bin) = make_dirs().await;
    let script = src.join("tool.ts");
    fs::write(&script, b"console.log('hi')\n").await.unwrap();
    link_executables_with_host(&RealHost, &bin, &[bun_spec(&script, "tool")], false)
        .await
        .unwrap();
    // A user's own regular file that must survive uninstall.
    make_plain(&bin, "user_tool").await;

    let report = unlink_managed_command_with_host(
        &RealHost,
        &bin,
        OsStr::new("tool"),
        std::slice::from_ref(&src),
        false,
    )
    .await
    .unwrap();
    let user = unlink_managed_command_with_host(
        &RealHost,
        &bin,
        OsStr::new("user_tool"),
        std::slice::from_ref(&src),
        false,
    )
    .await
    .unwrap();
    assert!(user.removed.is_empty());

    assert!(report.removed.iter().any(|p| p.ends_with("tool")));
    assert!(
        !bin.join("tool").exists(),
        "managed launcher should be removed"
    );
    assert!(
        bin.join("user_tool").exists(),
        "user file must be preserved"
    );
    assert!(user.skipped.iter().any(|p| p.ends_with("user_tool")));

    fs::remove_dir_all(&src).await.unwrap();
    fs::remove_dir_all(&bin).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn bun_launcher_without_env_has_no_shine_dependency() {
    let (src, bin) = make_dirs().await;
    let script = src.join("tool.ts");
    fs::write(&script, b"console.log('hi')\n").await.unwrap();

    link_executables_with_host(&RealHost, &bin, &[bun_spec(&script, "tool")], false)
        .await
        .unwrap();

    let content = fs::read_to_string(bin.join("tool")).await.unwrap();
    assert!(
        content.contains(&format!(
            "exec bun --no-install '{}' \"$@\"",
            script.display()
        )),
        "no-env launcher must run bun directly: {content}"
    );
    assert!(
        !content.contains("shine env run"),
        "no-env launcher must not depend on shine: {content}"
    );

    fs::remove_dir_all(&src).await.unwrap();
    fs::remove_dir_all(&bin).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn bun_launcher_with_env_wraps_shine_env_run() {
    let (src, bin) = make_dirs().await;
    let script = src.join("tool.ts");
    fs::write(&script, b"console.log('hi')\n").await.unwrap();

    let env = vec!["API_URL".to_string(), "SERVICE_TOKEN=API_TOKEN".to_string()];
    link_executables_with_host(
        &RealHost,
        &bin,
        &[bun_spec_with_env(&script, "tool", env)],
        false,
    )
    .await
    .unwrap();

    let content = fs::read_to_string(bin.join("tool")).await.unwrap();
    // Both prerequisites are checked with a 127 exit.
    assert!(content.contains("command -v bun"));
    assert!(content.contains("command -v shine"));
    assert_eq!(content.matches("exit 127").count(), 2);
    // The child runs through shine env run with the declared, ordered specs.
    assert!(content.contains(&format!(
            "exec shine env run --no-workspace --with 'API_URL' --with 'SERVICE_TOKEN=API_TOKEN' -- bun --no-install '{}' \"$@\"",
            script.display()
        )));

    fs::remove_dir_all(&src).await.unwrap();
    fs::remove_dir_all(&bin).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn locked_bun_launcher_uses_fallback_and_refreshes_disabled_launcher() {
    let (src, bin) = make_dirs().await;
    let script = src.join("tool.ts");
    fs::write(&script, b"import 'zod'\n").await.unwrap();

    link_executables_with_host(&RealHost, &bin, &[bun_spec(&script, "tool")], false)
        .await
        .unwrap();
    let refreshed =
        link_executables_with_host(&RealHost, &bin, &[locked_bun_spec(&script, "tool")], false)
            .await
            .unwrap();
    assert_eq!(refreshed.overwritten.len(), 1);
    let content = fs::read_to_string(bin.join("tool")).await.unwrap();
    assert!(content.contains("exec bun --install=fallback"));

    fs::remove_dir_all(&src).await.unwrap();
    fs::remove_dir_all(&bin).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn bun_launcher_refreshes_when_env_declaration_changes() {
    let (src, bin) = make_dirs().await;
    let script = src.join("tool.ts");
    fs::write(&script, b"console.log('hi')\n").await.unwrap();

    // Install with no env, then replace the same source with a declaration.
    link_executables_with_host(&RealHost, &bin, &[bun_spec(&script, "tool")], false)
        .await
        .unwrap();
    let changed = link_executables_with_host(
        &RealHost,
        &bin,
        &[bun_spec_with_env(
            &script,
            "tool",
            vec!["API_URL".to_string()],
        )],
        false,
    )
    .await
    .unwrap();
    assert_eq!(
        changed.overwritten.len(),
        1,
        "adding an env declaration must refresh the launcher without --force"
    );

    // Re-running with the same declaration is a no-op (byte-identical).
    let again = link_executables_with_host(
        &RealHost,
        &bin,
        &[bun_spec_with_env(
            &script,
            "tool",
            vec!["API_URL".to_string()],
        )],
        false,
    )
    .await
    .unwrap();
    assert_eq!(again.skipped.len(), 1);
    assert!(again.overwritten.is_empty());

    fs::remove_dir_all(&src).await.unwrap();
    fs::remove_dir_all(&bin).await.unwrap();
}

#[cfg(not(unix))]
#[test]
fn shell_shims_pass_bash_compatible_paths_on_windows() {
    let source = PathBuf::from(r"C:\Users\me\.shine\rendered\shell\utils\copyfile.sh");

    let ps1 = powershell_shim_content(
        &source,
        LinkRuntime::Native,
        "copyfile",
        BunDependencyMode::Disabled,
        &[],
        None,
    );
    let cmd = cmd_shim_content(
        &source,
        LinkRuntime::Native,
        "copyfile",
        BunDependencyMode::Disabled,
        &[],
        None,
    );

    assert!(ps1.contains("C:/Users/me/.shine/rendered/shell/utils/copyfile.sh"));
    assert!(cmd.contains("C:/Users/me/.shine/rendered/shell/utils/copyfile.sh"));
    assert!(!ps1.contains(r"& bash 'C:\Users\me"));
}

#[tokio::test]
async fn host_launchers_match_transaction_resources_and_capture_live_config() {
    use crate::runtime::InMemoryHost;
    let root = std::env::temp_dir().join("shine-launcher-memory");
    let bin = root.join("bin");
    let host = InMemoryHost::new();
    let mut spec = LinkSpec {
        native_cmd_literal_path: false,
        live_launch_config: Some(root.join("state-a")),
        source: root.join("presets/demo/tool.ts"),
        link_name: "tool".into(),
        runtime: LinkRuntime::Bun,
        bun_dependencies: BunDependencyMode::Locked,
        env: vec!["SOURCE=TARGET".into()],
        render_target: Some("shell/demo/tool".into()),
    };
    let installed = link_executables_with_host(&host, &bin, &[spec.clone()], false)
        .await
        .unwrap();
    assert_eq!(
        installed.created,
        [command_path_for_name(&bin, &spec.link_name)]
    );
    for resource in prepare_launcher_resources(&bin, &spec) {
        assert!(
            prepared_launcher_resource_is_exact(&host, &resource)
                .await
                .unwrap()
        );
    }
    let again = link_executables_with_host(&host, &bin, &[spec.clone()], false)
        .await
        .unwrap();
    assert_eq!(again.skipped, installed.created);
    spec.live_launch_config = Some(root.join("state-b"));
    let refreshed = link_executables_with_host(&host, &bin, &[spec.clone()], false)
        .await
        .unwrap();
    assert_eq!(refreshed.overwritten, installed.created);
    for resource in prepare_launcher_resources(&bin, &spec) {
        assert!(
            prepared_launcher_resource_is_exact(&host, &resource)
                .await
                .unwrap()
        );
    }
}

#[tokio::test]
async fn host_unlink_preserves_all_resources_when_any_launcher_is_foreign() {
    use crate::runtime::{HostOperation, InMemoryHost};
    let root = std::env::temp_dir().join("shine-launcher-pair-memory");
    let bin = root.join("bin");
    let managed = root.join("presets");
    let spec = bun_spec(&managed.join("demo/tool.ts"), "tool");
    let resources = prepare_launcher_resources(&bin, &spec);
    for foreign in &resources {
        let host = InMemoryHost::new();
        for resource in &resources {
            apply_prepared_launcher_resource(&host, resource)
                .await
                .unwrap();
        }
        host.put_file(foreign.destination(), b"user replacement".to_vec());
        let before = host.operations().len();
        let report = unlink_managed_command_with_host(
            &host,
            &bin,
            &spec.link_name,
            std::slice::from_ref(&managed),
            false,
        )
        .await
        .unwrap();
        assert!(report.removed.is_empty());
        assert_eq!(report.skipped, [foreign.destination()]);
        assert!(
            host.operations()[before..]
                .iter()
                .all(|op| matches!(op, HostOperation::Read(_)))
        );
        assert_eq!(
            host.read(foreign.destination()).await.unwrap(),
            b"user replacement"
        );
        for resource in resources
            .iter()
            .filter(|r| r.destination() != foreign.destination())
        {
            assert!(
                prepared_launcher_resource_is_exact(&host, resource)
                    .await
                    .unwrap()
            );
        }
    }
}

// Digests are captured from the pre-refactor production templates, including quoting and EOLs.
#[test]
#[cfg(unix)]
fn launcher_template_bytes_remain_compatible() {
    use sha2::{Digest, Sha256};
    let source = Path::new("/captured root/O'Connor/$tools/run.ts");
    let env = vec!["SOURCE=TARGET".to_string(), "PLAIN".to_string()];
    let cases = [
        unix_bun_launcher_content(source, "run", BunDependencyMode::Disabled, &[]),
        unix_bun_launcher_content(source, "run", BunDependencyMode::Locked, &env),
        unix_live_launcher_content(
            source,
            "run",
            LinkRuntime::Bun,
            BunDependencyMode::Locked,
            &env,
            "demo/run",
        ),
        unix_live_launcher_content(
            source,
            "run",
            LinkRuntime::Native,
            BunDependencyMode::Disabled,
            &[],
            "demo/run",
        ),
        unix_live_bun_launch_content(source, Path::new("/captured config"), "demo/run"),
        powershell_live_bun_launch_content(source, Path::new("/captured config"), "demo/run"),
        cmd_live_bun_launch_content(source, Path::new("/captured config"), "demo/run"),
        cmd_shim_content(
            Path::new(r"C:\Users\O'Connor\run.ps1"),
            LinkRuntime::Native,
            "run",
            BunDependencyMode::Disabled,
            &[],
            None,
        ),
        correct_native_cmd_path(
            cmd_shim_content(
                Path::new(r"C:\Users\O'Connor\run.ps1"),
                LinkRuntime::Native,
                "run",
                BunDependencyMode::Disabled,
                &[],
                None,
            ),
            Path::new(r"C:\Users\O'Connor\run.ps1"),
        ),
    ];
    let hashes = cases
        .iter()
        .map(|content| format!("{:x}", Sha256::digest(content.as_bytes())))
        .collect::<Vec<_>>();
    assert_eq!(
        hashes,
        [
            "d3435639a50a3b45af95d7248b65ad839b25681fef3aebfa0a3f105396dae229",
            "2e2562a7c7618d9dd85a1208664a9087a36c301c2b4d0668ea8c008f0a6f3cd3",
            "6684c697a19379494c3836f1cbffe8c61611ad3d3b992fbed476b90890b223a6",
            "811f87e50f688a79ce4f4959d4fe048fba35e068dce8e68fa738b3724f1ffd87",
            "d507e930ce521bdbf0a2f6a5cffbb9b19f5c0ca4187d7affd32135e3262bbc7e",
            "bf1197e1801687e77f0a53ed308f43d17bdde3bc29fdb6845253de17cde00b98",
            "8a1803b944a0397212f89fd71a9a6047a371879659cfcd2a62223b7165bfc7c9",
            "5be59a029c6e43b298b30770602e335da183ce02c4d8028372a55b5d31ee9737",
            "8bdc6e0f573f3efe2d7211257766e4d90ed55179d87844d4a8d918e6e6c2f466"
        ]
    );
}
