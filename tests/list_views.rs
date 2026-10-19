//! Process contracts for list/update/status, also runnable against the pre-refactor binary.
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("shine-list-{}-{nonce}", std::process::id()));
        for path in [
            "home/AppData/Roaming",
            "home/AppData/Local",
            "state/presets/app",
            "state/presets/shell",
        ] {
            fs::create_dir_all(root.join(path)).unwrap();
        }
        fs::write(
            root.join("state/config.toml"),
            "schema_version = 2\n[env]\nENABLE = '1'\n",
        )
        .unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        fs::write(
            root.join("state/update-check.json"),
            format!(
                "{{\"latest_version\":\"{}\",\"checked_at_unix_secs\":{now}}}",
                env!("CARGO_PKG_VERSION")
            ),
        )
        .unwrap();
        Self(root)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_shine"))
            .current_dir(&self.0)
            .env("HOME", self.0.join("home"))
            .env("USERPROFILE", self.0.join("home"))
            .env("APPDATA", self.0.join("home/AppData/Roaming"))
            .env("LOCALAPPDATA", self.0.join("home/AppData/Local"))
            .env("SHINE_CONFIG_DIR", self.0.join("state"))
            .env("SHINE_PRESETS", self.0.join("state/presets"))
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .args(args)
            .output()
            .unwrap()
    }
    fn app(&self, generated: bool) {
        let preset = self.0.join("state/presets/app/demo");
        fs::create_dir_all(&preset).unwrap();
        let destination = self.0.join("destination");
        // Rust's quoted path escaping also forms a TOML basic string on supported hosts.
        fs::write(preset.join("shine.toml"), format!("metadata_schema_version = 2\ndest = {:?}\nlist_mode = 'files'\n[permissions]\nschema_version = 2\nopaque_code = 'unrestricted'\n[[files]]\nsource = 'config.txt'\n[[files]]\nsource = 'manual.txt'\n{}", destination.to_str().unwrap(), if generated { "generator = { script = 'generate.ts', runtime = 'bun', when_env = 'ENABLE', env = ['ENABLE'], auto = false }\n" } else { "" })).unwrap();
        fs::write(preset.join("config.txt"), "static-v1\n").unwrap();
        fs::write(preset.join("manual.txt"), "manual-v1\n").unwrap();
        fs::write(
            preset.join("generate.ts"),
            format!(
                "await Bun.write({:?}, 'ran'); console.log('manual-v2');\n",
                self.0.join("generator-ran").to_str().unwrap()
            ),
        )
        .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn success(output: &Output) {
    assert!(output.status.success(), "{output:?}");
}
fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn list_views_keep_deferred_generators_separate_from_upgrade_and_preserve_receipts() {
    let fixture = Fixture::new();
    fixture.app(false);
    success(&fixture.run(&["app", "install", "demo", "--yes"]));
    fixture.app(true);
    fs::write(
        fixture.0.join("state/presets/app/demo/config.txt"),
        "static-v2\n",
    )
    .unwrap();
    let receipt = fs::read(fixture.0.join("state/app-manifest.toml")).unwrap();
    for args in [
        vec!["list"],
        vec!["update"],
        vec!["update", "--diff"],
        vec!["update", "--verbose"],
        vec!["update", "--verbose", "--diff"],
    ] {
        let output = fixture.run(&args);
        success(&output);
        let text = stdout(&output);
        assert!(text.contains("App Configs"), "{output:?}");
        if args[0] == "update" {
            assert!(text.contains("not evaluated"), "{output:?}");
            assert!(!text.contains("shine app refresh"), "{output:?}");
        }
        assert!(!fixture.0.join("generator-ran").exists());
        assert_eq!(
            fs::read(fixture.0.join("state/app-manifest.toml")).unwrap(),
            receipt
        );
        assert_eq!(
            fs::read(fixture.0.join("destination/config.txt")).unwrap(),
            b"static-v1\n"
        );
        assert_eq!(
            fs::read(fixture.0.join("destination/manual.txt")).unwrap(),
            b"manual-v1\n"
        );
    }
    // Explicit evaluation still checks scoped trust and renders failures before returning nonzero.
    for args in [
        vec!["update", "--run-generators"],
        vec!["update", "--verbose", "--run-generators"],
    ] {
        let output = fixture.run(&args);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(
            stdout(&output).contains("generator trust required"),
            "{output:?}"
        );
        assert!(!fixture.0.join("generator-ran").exists());
    }
    success(&fixture.run(&["trust", "grant", "app/demo", "--yes"]));
    for args in [
        vec!["update", "--run-generators"],
        vec!["update", "--diff", "--run-generators"],
        vec!["update", "--verbose", "--run-generators"],
    ] {
        let output = fixture.run(&args);
        success(&output);
        let text = stdout(&output);
        assert!(
            text.contains("shine app refresh demo manual.txt"),
            "{output:?}"
        );
        assert!(text.contains("shine upgrade app/demo"), "{output:?}");
        assert!(fixture.0.join("generator-ran").exists());
        assert_eq!(
            fs::read(fixture.0.join("state/app-manifest.toml")).unwrap(),
            receipt
        );
        assert_eq!(
            fs::read(fixture.0.join("destination/manual.txt")).unwrap(),
            b"manual-v1\n"
        );
    }
}

#[test]
fn status_prints_shell_attention_before_later_app_error_and_update_waits_for_queries() {
    let fixture = Fixture::new();
    let preset = fixture.0.join("state/presets/shell/demo");
    fs::create_dir_all(&preset).unwrap();
    fs::write(preset.join("shine.toml"), "[[files]]\nsource = 'run.ts'\ntarget = 'run'\nruntime = 'bun'\n[files.permissions]\nschema_version = 2\nopaque_code = 'unrestricted'\n").unwrap();
    fs::write(preset.join("run.ts"), "console.log('never executed');\n").unwrap();
    success(&fixture.run(&["trust", "grant", "shell/demo/run", "--yes"]));
    success(&fixture.run(&["shell", "install", "demo/run", "--yes"]));
    let bin = fixture.0.join("state/bin");
    let launcher = if cfg!(unix) {
        bin.join("run")
    } else {
        bin.join("run.ps1")
    };
    fs::write(&launcher, b"foreign replacement").unwrap();
    let receipt = fs::read(fixture.0.join("state/shell-manifest.toml")).unwrap();
    let broken = fixture.0.join("state/presets/app/broken");
    fs::create_dir_all(&broken).unwrap();
    fs::write(
        broken.join("shine.toml"),
        "metadata_schema_version = 2\ninvalid = [",
    )
    .unwrap();
    let status = fixture.run(&["update", "--verbose"]);
    assert_eq!(status.status.code(), Some(1), "{status:?}");
    assert!(
        stdout(&status).contains("Shell attention required"),
        "{status:?}"
    );
    assert!(
        String::from_utf8_lossy(&status.stderr).contains("loading active App Presets for status"),
        "{status:?}"
    );
    let update = fixture.run(&["update"]);
    assert_eq!(update.status.code(), Some(1), "{update:?}");
    assert!(
        !stdout(&update).contains("Shell attention required"),
        "{update:?}"
    );
    assert!(
        String::from_utf8_lossy(&update.stderr).contains("loading active App Presets for update"),
        "{update:?}"
    );
    assert_eq!(fs::read(&launcher).unwrap(), b"foreign replacement");
    assert_eq!(
        fs::read(fixture.0.join("state/shell-manifest.toml")).unwrap(),
        receipt
    );
}

#[test]
fn simple_list_collapses_installed_app_files_to_their_category() {
    let fixture = Fixture::new();
    fixture.app(false);
    let uninstalled = fixture.0.join("state/presets/app/uninstalled");
    fs::create_dir_all(&uninstalled).unwrap();
    fs::write(
        uninstalled.join("shine.toml"),
        "dest = '~/.config/uninstalled'\n[[files]]\nsource = 'config.txt'\n",
    )
    .unwrap();
    fs::write(uninstalled.join("config.txt"), "not installed").unwrap();
    success(&fixture.run(&["app", "install", "demo", "--yes"]));
    fs::remove_file(fixture.0.join("destination/manual.txt")).unwrap();
    let receipt = fs::read(fixture.0.join("state/app-manifest.toml")).unwrap();
    let output = fixture.run(&["list"]);
    success(&output);
    let text = stdout(&output);
    assert!(text.contains("App Configs"), "{output:?}");
    assert_eq!(
        text.lines().filter(|line| line.trim() == "demo").count(),
        1,
        "{output:?}"
    );
    assert!(!text.contains("uninstalled"), "{output:?}");
    assert!(
        !text.contains("config.txt") && !text.contains("manual.txt"),
        "{output:?}"
    );
    assert_eq!(
        fs::read(fixture.0.join("state/app-manifest.toml")).unwrap(),
        receipt
    );
    assert_eq!(
        fs::read(fixture.0.join("destination/config.txt")).unwrap(),
        b"static-v1\n"
    );
    assert!(!fixture.0.join("destination/manual.txt").exists());
}

#[test]
fn simple_list_shows_partially_installed_app_categories() {
    let fixture = Fixture::new();
    fixture.app(false);
    let metadata = fixture.0.join("state/presets/app/demo/shine.toml");
    let initial = fs::read_to_string(&metadata).unwrap();
    fs::write(
        &metadata,
        initial.replace("[[files]]\nsource = 'manual.txt'\n", ""),
    )
    .unwrap();
    success(&fixture.run(&["app", "install", "demo", "--yes"]));
    fs::write(&metadata, initial).unwrap();
    fs::write(fixture.0.join("destination/config.txt"), "user modified").unwrap();
    let receipt = fs::read(fixture.0.join("state/app-manifest.toml")).unwrap();
    let output = fixture.run(&["list"]);
    success(&output);
    let text = stdout(&output);
    assert!(text.contains("App Configs"), "{output:?}");
    assert_eq!(
        text.lines().filter(|line| line.trim() == "demo").count(),
        1,
        "{output:?}"
    );
    assert_eq!(
        fs::read(fixture.0.join("state/app-manifest.toml")).unwrap(),
        receipt
    );
    assert_eq!(
        fs::read(fixture.0.join("destination/config.txt")).unwrap(),
        b"user modified"
    );
    assert!(!fixture.0.join("destination/manual.txt").exists());
}

#[test]
fn shell_descriptions_align_unicode_names_and_continuations() {
    let fixture = Fixture::new();
    let preset = fixture.0.join("state/presets/shell/unicode");
    fs::create_dir_all(&preset).unwrap();
    #[cfg(windows)]
    let extension = "ps1";
    #[cfg(not(windows))]
    let extension = "sh";
    for name in ["abc", "猫", "e\u{301}"] {
        fs::write(
            preset.join(format!("{name}.{extension}")),
            "# First description\n# Continued description\n",
        )
        .unwrap();
    }
    let output = fixture.run(&["shell", "list"]);
    success(&output);
    let text = stdout(&output);
    let widths: Vec<_> = text
        .lines()
        .filter_map(|line| {
            line.split_once("First description")
                .or_else(|| line.split_once("Continued description"))
                .map(|(prefix, _)| console::measure_text_width(prefix))
        })
        .collect();
    assert_eq!(widths.len(), 6, "{text}");
    assert!(widths.iter().all(|width| *width == widths[0]), "{text}");
}

#[test]
fn update_version_check_failure_never_claims_everything_is_current() {
    for pending in [false, true] {
        let fixture = Fixture::new();
        if pending {
            fixture.app(false);
            success(&fixture.run(&["app", "install", "demo", "--yes"]));
            fs::write(
                fixture.0.join("state/presets/app/demo/config.txt"),
                "static-v2\n",
            )
            .unwrap();
        }
        // A local read error prevents network access and exercises the real failure path.
        let cache = fixture.0.join("state/update-check.json");
        fs::remove_file(&cache).unwrap();
        fs::create_dir(&cache).unwrap();
        let output = fixture.run(&["update"]);
        success(&output);
        let text = stdout(&output);
        assert!(!text.contains("Nothing to update."), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("warning: skipped shine version check: failed to read update cache"),
            "{output:?}"
        );
        if pending {
            assert!(
                text.contains("App Configs") && text.contains("demo"),
                "{output:?}"
            );
            assert!(
                !text.contains("No configuration updates found"),
                "{output:?}"
            );
            assert_eq!(
                fs::read(fixture.0.join("destination/config.txt")).unwrap(),
                b"static-v1\n"
            );
        } else {
            assert!(
                text.contains("No configuration updates found; Shine version check incomplete."),
                "{output:?}"
            );
        }
    }
    let fixture = Fixture::new();
    let output = fixture.run(&["update"]);
    success(&output);
    assert!(stdout(&output).contains("Nothing to update."), "{output:?}");
}

#[test]
fn generator_summary_preserves_unknown_trust_and_failure_states() {
    let fixture = Fixture::new();
    fixture.app(false);
    success(&fixture.run(&["app", "install", "demo", "--yes"]));
    fixture.app(true);
    let receipt = fs::read(fixture.0.join("state/app-manifest.toml")).unwrap();
    for (options, expected, code) in [
        (vec![], "generator not evaluated", 0),
        (vec!["--run-generators"], "generator trust required", 1),
    ] {
        for diff in [false, true] {
            let mut args = vec!["update", "--verbose"];
            args.extend_from_slice(&options);
            if diff {
                args.push("--diff");
            }
            let output = fixture.run(&args);
            assert_eq!(output.status.code(), Some(code), "{output:?}");
            let text = stdout(&output);
            let unit = if diff { "file" } else { "category" };
            assert!(
                text.contains(&format!("1 {unit}: {expected}")),
                "{output:?}"
            );
            assert!(!text.contains("user-modified"), "{output:?}");
            assert!(!fixture.0.join("generator-ran").exists());
        }
    }
    fs::write(
        fixture.0.join("state/presets/app/demo/generate.ts"),
        "throw new Error('fixture generator failure');\n",
    )
    .unwrap();
    success(&fixture.run(&["trust", "grant", "app/demo", "--yes"]));
    for diff in [false, true] {
        let mut args = vec!["update", "--verbose", "--run-generators"];
        if diff {
            args.push("--diff");
        }
        let output = fixture.run(&args);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let unit = if diff { "file" } else { "category" };
        assert!(
            stdout(&output).contains(&format!("1 {unit}: generator evaluation failed")),
            "{output:?}"
        );
        assert!(!stdout(&output).contains("user-modified"), "{output:?}");
    }
    let metadata = fixture.0.join("state/presets/app/demo/shine.toml");
    fs::write(&metadata, fs::read_to_string(&metadata).unwrap().replace(
        "source = 'config.txt'\n",
        "source = 'config.txt'\ngenerator = { script = 'generate.ts', runtime = 'bun', when_env = 'ENABLE', env = ['ENABLE'], auto = false }\n",
    )).unwrap();
    for (args, expected) in [
        (
            vec!["update", "--verbose"],
            "1 category: generator not evaluated",
        ),
        (
            vec!["update", "--verbose", "--diff"],
            "2 files: generator not evaluated",
        ),
    ] {
        let output = fixture.run(&args);
        success(&output);
        assert!(stdout(&output).contains(expected), "{output:?}");
    }
    fs::write(fixture.0.join("destination/config.txt"), b"user edit\n").unwrap();
    let output = fixture.run(&["update", "--verbose", "--diff"]);
    success(&output);
    assert!(stdout(&output).contains("1 user-modified"), "{output:?}");
    assert!(
        stdout(&output).contains("1 file: generator not evaluated"),
        "{output:?}"
    );
    assert_eq!(
        fs::read(fixture.0.join("destination/config.txt")).unwrap(),
        b"user edit\n"
    );
    assert_eq!(
        fs::read(fixture.0.join("state/app-manifest.toml")).unwrap(),
        receipt
    );
    assert_eq!(
        fs::read(fixture.0.join("destination/manual.txt")).unwrap(),
        b"manual-v1\n"
    );
}

#[tokio::test]
async fn verbose_status_keeps_current_pending_and_missing_managed_sys_entries() {
    let fixture = Fixture::new();
    let os_id = cli::sys::detect_os_id().await.unwrap();
    let preset = fixture.0.join("state/presets/sys").join(&os_id);
    fs::create_dir_all(&preset).unwrap();
    let content = b"recorded content\n";
    let mut metadata = "version = 2\n".to_string();
    let mut remaining_metadata = metadata.clone();
    let mut receipt = "schema_version = 1\n".to_string();
    for (id, label) in [
        ("demo", "Demo system config"),
        ("other", "Other system config"),
    ] {
        let destination = fixture.0.join("home").join(format!("{id}.txt"));
        fs::write(&destination, content).unwrap();
        fs::write(preset.join(format!("{id}.txt")), content).unwrap();
        let item = format!(
            "[[items]]\nid = {id:?}\nlabel = {label:?}\nmode = 'managed'\ndriver = 'managed-file'\n[items.config]\nsource = '{id}.txt'\ntarget = {:?}\n",
            destination.to_str().unwrap()
        );
        metadata.push_str(&item);
        if id == "other" {
            remaining_metadata.push_str(&item);
        }
        receipt.push_str(&format!(
            "[[entries]]\nos_id = {os_id:?}\nitem_id = {id:?}\nlabel = {label:?}\nstatus = 'updated'\nupdated_at = '1'\nmanaged = true\n[entries.receipt]\ndriver = 'managed-file'\nversion = 1\ndestination = {:?}\ncontent_hash = {}\nprivileged = false\n", destination.to_str().unwrap(), shine_core::install::hash_content(content)
        ));
    }
    fs::write(preset.join("shine.toml"), metadata).unwrap();
    fs::write(fixture.0.join("state/sys-manifest.toml"), &receipt).unwrap();
    let listed = fixture.run(&["list"]);
    success(&listed);
    assert!(
        stdout(&listed).lines().any(|line| line.trim() == "demo"),
        "{listed:?}"
    );
    for expected in [
        "up-to-date (recorded)",
        "update available",
        "preset missing; recorded entry preserved",
    ] {
        let output = fixture.run(&["update", "--verbose"]);
        success(&output);
        let text = stdout(&output);
        assert!(
            text.contains("System Configs") && text.contains("Demo system config"),
            "{output:?}"
        );
        assert!(text.contains(expected), "{output:?}");
        assert!(!text.contains("Nothing installed yet"), "{output:?}");
        assert_eq!(
            text.lines().filter(|line| line.contains("(demo)")).count(),
            1,
            "{text}"
        );
        assert_eq!(
            text.lines().filter(|line| line.contains("(other)")).count(),
            1,
            "{text}"
        );
        assert_eq!(
            text.matches("up-to-date (recorded)").count(),
            if expected == "up-to-date (recorded)" {
                2
            } else {
                1
            },
            "{text}"
        );
        for id in ["demo", "other"] {
            assert_eq!(
                fs::read(fixture.0.join("home").join(format!("{id}.txt"))).unwrap(),
                content
            );
        }
        if expected == "up-to-date (recorded)" {
            fs::write(preset.join("demo.txt"), b"new desired content\n").unwrap();
        } else if expected == "update available" {
            fs::write(preset.join("shine.toml"), &remaining_metadata).unwrap();
        }
    }
    assert_eq!(
        fs::read_to_string(fixture.0.join("state/sys-manifest.toml")).unwrap(),
        receipt
    );
}

#[test]
fn app_catalog_info_and_update_columns_use_unicode_display_width() {
    let fixture = Fixture::new();
    for category in ["abc", "猫", "e\u{301}"] {
        let preset = fixture.0.join("state/presets/app").join(category);
        fs::create_dir_all(&preset).unwrap();
        let destination = fixture.0.join("destination").join(category);
        let mut metadata = format!(
            "metadata_schema_version = 2\ndescription = 'Catalog description'\ndest = {}\n",
            serde_json::to_string(destination.to_str().unwrap()).unwrap()
        );
        for source in ["abc.txt", "猫.txt", "e\u{301}.txt"] {
            metadata.push_str(&format!(
                "[[files]]\nsource = {}\n",
                serde_json::to_string(source).unwrap()
            ));
            fs::write(preset.join(source), "original\n").unwrap();
        }
        fs::write(preset.join("shine.toml"), metadata).unwrap();
    }
    let output = fixture.run(&["app", "list"]);
    success(&output);
    let text = stdout(&output);
    let widths: Vec<_> = text
        .lines()
        .filter_map(|line| {
            line.split_once("Catalog description")
                .map(|(prefix, _)| console::measure_text_width(prefix))
        })
        .collect();
    assert_eq!(widths.len(), 3, "{text}");
    assert!(widths.iter().all(|width| *width == widths[0]), "{text}");
    let output = fixture.run(&["app", "info", "abc"]);
    success(&output);
    let text = stdout(&output);
    let widths: Vec<_> = text
        .lines()
        .filter_map(|line| {
            line.split_once('→')
                .map(|(prefix, _)| console::measure_text_width(prefix))
        })
        .collect();
    assert_eq!(widths.len(), 3, "{text}");
    assert!(widths.iter().all(|width| *width == widths[0]), "{text}");

    success(&fixture.run(&["app", "install", "--yes"]));
    let receipt = fs::read(fixture.0.join("state/app-manifest.toml")).unwrap();
    for category in ["abc", "猫", "e\u{301}"] {
        fs::write(
            fixture
                .0
                .join("state/presets/app")
                .join(category)
                .join("abc.txt"),
            "updated\n",
        )
        .unwrap();
    }
    for args in [
        vec!["update", "--diff"],
        vec!["update", "--verbose"],
        vec!["update", "--verbose", "--diff"],
    ] {
        let output = fixture.run(&args);
        success(&output);
        let text = stdout(&output);
        let widths: Vec<_> = text
            .lines()
            .filter(|line| line.starts_with("  ↑  "))
            .map(|line| {
                let marker = if args.contains(&"--verbose") && args.contains(&"--diff") {
                    "→"
                } else {
                    "update available"
                };
                console::measure_text_width(line.split_once(marker).unwrap().0)
            })
            .collect();
        assert_eq!(widths.len(), 3, "{text}");
        assert!(widths.iter().all(|width| *width == widths[0]), "{text}");
    }
    assert_eq!(
        fs::read(fixture.0.join("state/app-manifest.toml")).unwrap(),
        receipt
    );
}

#[test]
fn shell_update_columns_use_unicode_display_width_without_changing_receipts() {
    let fixture = Fixture::new();
    for category in ["abc", "猫", "e\u{301}"] {
        let preset = fixture.0.join("state/presets/shell").join(category);
        fs::create_dir_all(&preset).unwrap();
        fs::write(preset.join("shine.toml"), format!(
            "[[files]]\nsource = 'run.ts'\ntarget = {}\nruntime = 'bun'\n[files.permissions]\nschema_version = 2\nopaque_code = 'unrestricted'\n", serde_json::to_string(category).unwrap()
        )).unwrap();
        fs::write(preset.join("run.ts"), "console.log('original');\n").unwrap();
        let target = format!("{category}/{category}");
        success(&fixture.run(&["trust", "grant", &format!("shell/{target}"), "--yes"]));
        success(&fixture.run(&["shell", "install", &target, "--yes"]));
        fs::write(preset.join("run.ts"), "console.log('changed');\n").unwrap();
    }
    let receipt = fs::read(fixture.0.join("state/shell-manifest.toml")).unwrap();
    for args in [
        vec!["update", "--diff"],
        vec!["update", "--verbose"],
        vec!["update", "--verbose", "--diff"],
    ] {
        let output = fixture.run(&args);
        success(&output);
        let text = stdout(&output);
        let widths = text
            .lines()
            .filter(|line| line.starts_with("  ↑  "))
            .map(|line| console::measure_text_width(line.split_once("update available").unwrap().0))
            .collect::<Vec<_>>();
        assert_eq!(widths.len(), 3, "{text}");
        assert!(widths.iter().all(|width| *width == widths[0]), "{text}");
    }
    assert_eq!(
        fs::read(fixture.0.join("state/shell-manifest.toml")).unwrap(),
        receipt
    );
}
