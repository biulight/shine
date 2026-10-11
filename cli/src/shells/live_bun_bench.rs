//! Opt-in local benchmark: no developer presets, credentials or business code.
use crate::{config::Config, test_support::env_lock};
use shine_core::runtime::{
    BunDependencyMode, ExternalShellMode, LinkRuntime, LinkSpec, RealHost, ShellManifest,
    ShellManifestEntry, link_executables_with_host,
};
use std::{
    ffi::OsString,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Stdio},
    time::Instant,
};

struct Restore {
    cwd: PathBuf,
    vars: Vec<(&'static str, Option<OsString>)>,
    root: PathBuf,
}
impl Drop for Restore {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.cwd).unwrap();
        for (key, value) in &self.vars {
            unsafe {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn distribution(mut samples: Vec<f64>) -> (f64, f64, f64) {
    samples.sort_by(f64::total_cmp);
    let median = samples[samples.len() / 2];
    let p95 = samples[(samples.len() * 95).div_ceil(100) - 1];
    let mut deviations = samples
        .iter()
        .map(|value| (value - median).abs())
        .collect::<Vec<_>>();
    deviations.sort_by(f64::total_cmp);
    (median, p95, deviations[deviations.len() / 2])
}

#[cfg(target_os = "macos")]
fn resident_bytes(pid: u32) -> Option<u64> {
    // PROC_PIDTASKINFO: six u64 counters followed by twelve i32 counters.
    #[repr(C)]
    struct TaskInfo {
        counters: [u64; 6],
        counts: [i32; 12],
    }
    unsafe extern "C" {
        fn proc_pidinfo(
            pid: i32,
            flavor: i32,
            arg: u64,
            buffer: *mut std::ffi::c_void,
            size: i32,
        ) -> i32;
    }
    let mut info = TaskInfo {
        counters: [0; 6],
        counts: [0; 12],
    };
    let size = std::mem::size_of::<TaskInfo>() as i32;
    let read = unsafe { proc_pidinfo(pid as i32, 4, 0, (&mut info as *mut TaskInfo).cast(), size) };
    (read == size).then_some(info.counters[1])
}
#[cfg(not(target_os = "macos"))]
fn resident_bytes(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    status.lines().find_map(|line| {
        line.strip_prefix("VmRSS:")
            .and_then(|value| value.split_whitespace().next()?.parse::<u64>().ok())
            .map(|kb| kb * 1024)
    })
}

#[test]
#[ignore = "opt-in latency/memory benchmark; use the same locally built binary for both launch formats"]
fn live_bun_launch_performance_matrix() {
    let _lock = env_lock();
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
    let cwd = std::env::current_dir().unwrap();
    let binary = cwd.join("target/debug/shine");
    assert!(binary.is_file(), "build the debug Shine binary first");
    let root = std::env::temp_dir().join(format!("shine-live-bench-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let _restore = Restore {
        cwd,
        root: root.clone(),
        vars: ["SHINE_CONFIG_DIR", "SHINE_PRESETS", "PATH", "HOME"]
            .into_iter()
            .map(|key| (key, std::env::var_os(key)))
            .collect(),
    };
    std::env::set_current_dir(&root).unwrap();
    let tools = root.join("tools");
    std::fs::create_dir(&tools).unwrap();
    std::os::unix::fs::symlink(&binary, tools.join("shine")).unwrap();
    std::fs::write(tools.join("bun"), "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(tools.join("bun"), std::fs::Permissions::from_mode(0o755)).unwrap();
    unsafe {
        std::env::set_var(
            "PATH",
            std::env::join_paths([
                tools.clone(),
                PathBuf::from("/usr/bin"),
                PathBuf::from("/bin"),
            ])
            .unwrap(),
        );
        std::env::set_var("HOME", root.join("home"));
        std::env::remove_var("SHINE_PRESETS");
    }
    // Fix methodology before sampling: 40 alternating pairs after 5 warmups;
    // median, nearest-rank p95 and MAD. Repeatable regression threshold is
    // max(5% of the legacy median, 3 * the larger MAD). No OS-cache purge.
    println!(
        "files,env,changed,old_median_ms,old_p95_ms,old_mad_ms,new_median_ms,new_p95_ms,new_mad_ms,config_ms,capture_ms,prepare_ms"
    );
    for files in [0, 500] {
        for env in [false, true] {
            for changed in [false, true] {
                let state = root.join(format!("state-{files}-{env}-{changed}"));
                std::fs::create_dir_all(state.join("presets/shell/demo")).unwrap();
                std::fs::write(state.join("config.toml"), "schema_version = 2\nexternal_shell_mode = 'live'\n[env]\nVALUE = 'rendered'\nTOKEN = 'plain-test-token'\n").unwrap();
                let source = state.join("presets/shell/demo/run.ts");
                std::fs::write(&source, "@@VALUE@@\n").unwrap();
                for i in 0..files {
                    std::fs::write(
                        state.join(format!("presets/shell/demo/helper-{i}.txt")),
                        vec![b'x'; 4096],
                    )
                    .unwrap();
                }
                unsafe {
                    std::env::set_var("SHINE_CONFIG_DIR", &state);
                }
                let entry = ShellManifestEntry {
                    launcher_format: Some("live-bun-v2".into()),
                    launcher_config_dir: Some(state.clone()),
                    category: "demo".into(),
                    command: "run".into(),
                    mode: ExternalShellMode::Live,
                    source_path: source.clone(),
                    rendered_path: state.join("rendered/shell/demo/run.ts"),
                    runtime: "bun".into(),
                    bun_dependencies: None,
                    dependency_hash: None,
                    transforms: vec!["template".into()],
                    env: if env { vec!["TOKEN".into()] } else { vec![] },
                    needs_source: false,
                    content_hash: 0,
                };
                ShellManifest {
                    entries: vec![entry.clone()],
                    ..ShellManifest::default()
                }
                .save(&RealHost, &state)
                .await
                .unwrap();
                let mut spec = LinkSpec {
                    source: entry.rendered_path.clone(),
                    link_name: "old".into(),
                    runtime: LinkRuntime::Bun,
                    bun_dependencies: BunDependencyMode::Disabled,
                    env: entry.env.clone(),
                    render_target: Some("shell/demo/run".into()),
                    native_cmd_literal_path: false,
                    live_launch_config: None,
                };
                link_executables_with_host(&RealHost, &state.join("bin"), &[spec.clone()], false)
                    .await
                    .unwrap();
                spec.link_name = "new".into();
                spec.live_launch_config = Some(state.clone());
                link_executables_with_host(&RealHost, &state.join("bin"), &[spec], false)
                    .await
                    .unwrap();
                let mut totals = [vec![], vec![]];
                let mut stages = [vec![], vec![], vec![]];
                for sample in 0..45 {
                    for which in if sample % 2 == 0 { [0, 1] } else { [1, 0] } {
                        if changed {
                            std::fs::write(&source, format!("@@VALUE@@ {sample:02} {which}\n"))
                                .unwrap();
                        }
                        let start = Instant::now();
                        let output = Command::new(state.join("bin").join(if which == 0 {
                            "old"
                        } else {
                            "new"
                        }))
                        .stdout(Stdio::null())
                        .stderr(Stdio::piped())
                        .output()
                        .unwrap();
                        assert!(output.status.success(), "{output:?}");
                        if sample >= 5 {
                            totals[which].push(start.elapsed().as_secs_f64() * 1000.0);
                        }
                    }
                    let start = Instant::now();
                    let config = Config::load_or_init().await.unwrap();
                    let loaded = start.elapsed().as_secs_f64() * 1000.0;
                    let start = Instant::now();
                    let runtime = crate::core_runtime::from_config(&config).await.unwrap();
                    let captured = start.elapsed().as_secs_f64() * 1000.0;
                    if changed {
                        std::fs::write(&source, format!("@@VALUE@@ {sample:02} stage\n")).unwrap();
                    }
                    let start = Instant::now();
                    runtime
                        .prepare_live_bun_launch("shell/demo/run")
                        .await
                        .unwrap();
                    if sample >= 5 {
                        stages[0].push(loaded);
                        stages[1].push(captured);
                        stages[2].push(start.elapsed().as_secs_f64() * 1000.0);
                    }
                }
                let old = distribution(totals[0].clone());
                let new = distribution(totals[1].clone());
                println!(
                    "{files},{env},{changed},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3}",
                    old.0,
                    old.1,
                    old.2,
                    new.0,
                    new.1,
                    new.2,
                    distribution(stages[0].clone()).0,
                    distribution(stages[1].clone()).0,
                    distribution(stages[2].clone()).0
                );
                // Separately observe the launcher PID while Bun is waiting.
                // This is resident memory at the wait point, not peak RSS.
                if !changed {
                    std::fs::write(
                        tools.join("bun"),
                        "#!/bin/sh\nprintf ready > \"$BENCH_WAIT_FILE\"\n/bin/sleep 0.4\n",
                    )
                    .unwrap();
                    for which in [0, 1] {
                        let mut memory = vec![];
                        for sample in 0..5 {
                            let ready = root.join(format!("ready-{files}-{env}-{which}-{sample}"));
                            let mut child = Command::new(state.join("bin").join(if which == 0 {
                                "old"
                            } else {
                                "new"
                            }))
                            .env("BENCH_WAIT_FILE", &ready)
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .spawn()
                            .unwrap();
                            let deadline = Instant::now() + std::time::Duration::from_secs(5);
                            while !ready.exists() && Instant::now() < deadline {
                                std::thread::sleep(std::time::Duration::from_millis(5));
                            }
                            assert!(ready.exists());
                            if let Some(bytes) = resident_bytes(child.id()) {
                                memory.push(bytes as f64 / 1024.0);
                            }
                            assert!(child.wait().unwrap().success());
                        }
                        if memory.is_empty() {
                            println!("WAIT_RSS_KIB {files},{env},{which},unavailable");
                        } else {
                            println!(
                                "WAIT_RSS_KIB {files},{env},{which},{:.0}",
                                distribution(memory).0
                            );
                        }
                    }
                    std::fs::write(tools.join("bun"), "#!/bin/sh\nexit 0\n").unwrap();
                }
                if new.0 - old.0 > (old.0 * 0.05).max(3.0 * old.2.max(new.2)) {
                    println!("REGRESSION: {files},{env},{changed}");
                }
            }
        }
    }
    });
}
