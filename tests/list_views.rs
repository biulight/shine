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
        let root = std::env::temp_dir().join(format!("shine-list-{}", uuid::Uuid::new_v4()));
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
        let mut command = Command::new(env!("CARGO_BIN_EXE_shine"));
        command.args(args);
        self.run_command(command)
    }
    fn run_command(&self, mut command: Command) -> Output {
        command
            .current_dir(&self.0)
            .env("HOME", self.0.join("home"))
            .env("USERPROFILE", self.0.join("home"))
            .env("APPDATA", self.0.join("home/AppData/Roaming"))
            .env("LOCALAPPDATA", self.0.join("home/AppData/Local"))
            .env("SHINE_CONFIG_DIR", self.0.join("state"))
            .env_remove("SHINE_PRESETS")
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
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

fn hint<'a>(text: &'a str, prefix: &str, suffix: &str) -> &'a str {
    text.lines()
        .find_map(|line| line.split_once(prefix)?.1.strip_suffix(suffix))
        .unwrap_or_else(|| panic!("missing hint {prefix:?} in {text}"))
}

fn assert_hint_argv(command: &str, expected: &[&str]) {
    use clap::Parser;
    // Parse the suggested command without invoking Shine or any Preset code.
    #[cfg(unix)]
    let parsed = Command::new("/bin/sh")
        .args(["-c", &format!("set -- {command}; printf '%s\\0' \"$@\"")])
        .output()
        .unwrap();
    #[cfg(windows)]
    let parsed = {
        // A PowerShell function consumes `--`; use a native executable like the real CLI.
        static PROBE: std::sync::LazyLock<Fixture> = std::sync::LazyLock::new(|| {
            let fixture = Fixture::new();
            let source = fixture.0.join("argv.rs");
            fs::write(
                &source,
                r#"fn main() {
                    for arg in std::iter::once("shine".to_owned()).chain(std::env::args().skip(1)) {
                        for byte in arg.bytes() { print!("{byte:02x}"); }
                        println!();
                    }
                }"#,
            )
            .unwrap();
            let build = Command::new("rustc")
                .arg(&source)
                .arg("-o")
                .arg(fixture.0.join("shine.exe"))
                .output()
                .unwrap();
            success(&build);
            fixture
        });
        let mut paths = vec![PROBE.0.clone()];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", command])
            .current_dir(&PROBE.0)
            .env("PATH", std::env::join_paths(paths).unwrap())
            .output()
            .unwrap()
    };
    success(&parsed);
    assert!(parsed.stderr.is_empty(), "{parsed:?}");
    #[cfg(unix)]
    assert_eq!(
        parsed.stdout,
        format!("{}\0", expected.join("\0")).as_bytes()
    );
    #[cfg(windows)]
    assert_eq!(
        String::from_utf8(parsed.stdout)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        expected
            .iter()
            .map(|arg| arg
                .bytes()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>())
            .collect::<Vec<_>>()
    );
    cli::commands::Cli::try_parse_from(expected).unwrap();
}

#[test]
fn app_info_diff_escapes_terminal_controls_without_mutating_content_or_state() {
    let fixture = Fixture::new();
    fixture.app(false);
    let source = fixture.0.join("state/presets/app/demo/config.txt");
    let destination = fixture.0.join("destination/config.txt");
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    let current = b"context\x07\nold\x1b[2J\t\nend\n";
    let expected = b"context\x07\nnew\x1b[H\rREPLACED\t\nend\n";
    fs::write(&source, expected).unwrap();
    fs::write(&destination, current).unwrap();
    let config = fs::read(fixture.0.join("state/config.toml")).unwrap();

    let output = fixture.run(&["app", "info", "demo", "--diff"]);
    success(&output);
    assert!(output.stderr.is_empty(), "{output:?}");
    let text = stdout(&output);
    // Uninstalled App info previews the expected content against an empty file.
    for escaped in ["+context\\u{7}", "+new\\u{1b}[H", "\\r", "REPLACED\\t"] {
        assert!(text.contains(escaped), "missing {escaped:?} in {text:?}");
    }
    assert!(
        text.chars().all(|ch| !ch.is_control() || ch == '\n'),
        "raw control character in {text:?}"
    );
    assert_eq!(fs::read(&source).unwrap(), expected);
    assert_eq!(fs::read(&destination).unwrap(), current);
    assert_eq!(
        fs::read(fixture.0.join("state/config.toml")).unwrap(),
        config
    );
    for state in ["app-manifest.toml", "trust.toml"] {
        assert!(!fixture.0.join("state").join(state).exists());
    }
}

#[test]
fn shell_info_hints_preserve_category_arguments_before_and_after_install() {
    for category in ["team tools", "it's", "-demo"] {
        let fixture = Fixture::new();
        let preset = fixture.0.join("state/presets/shell").join(category);
        fs::create_dir_all(&preset).unwrap();
        let script = if cfg!(windows) { "tool.ps1" } else { "tool.sh" };
        fs::write(
            preset.join("shine.toml"),
            format!("[[files]]\nsource = '{script}'\ntarget = 'tool'\n"),
        )
        .unwrap();
        fs::write(preset.join(script), "# never executed\n").unwrap();
        let selector = format!("{category}/tool");
        let target = format!("shell/{selector}");
        let info = fixture.run(&["shell", "info", "--", category]);
        success(&info);
        let mut expected = vec!["shine", "shell", "install"];
        if category.starts_with('-') {
            expected.push("--");
        }
        expected.push(&selector);
        assert_hint_argv(
            hint(&stdout(&info), "Run `", "` to install this target."),
            &expected,
        );

        success(&fixture.run(&["trust", "grant", &target, "--yes"]));
        success(&fixture.run(&["shell", "install", "--yes", "--", &selector]));
        let receipt = fs::read(fixture.0.join("state/shell-manifest.toml")).unwrap();
        let info = fixture.run(&["shell", "info", "--", category]);
        success(&info);
        assert_hint_argv(
            hint(&stdout(&info), "Run `", "` to repair this target."),
            &["shine", "install", &target, "--replace-managed"],
        );
        assert_eq!(
            fs::read(fixture.0.join("state/shell-manifest.toml")).unwrap(),
            receipt
        );
    }
}

#[tokio::test]
async fn upgrade_summaries_keep_sys_preservation_failures_and_partial_success() {
    let fixture = Fixture::new();
    let os_id = cli::sys::detect_os_id().await.unwrap();
    let preset = fixture.0.join("state/presets/sys").join(os_id);
    fs::create_dir_all(&preset).unwrap();
    let mut metadata = "version = 2\n".to_string();
    for id in ["demo", "other"] {
        let destination = fixture.0.join("home").join(format!("{id}.txt"));
        fs::write(preset.join(format!("{id}.txt")), "v1\n").unwrap();
        metadata.push_str(&format!("[[items]]\nid = {id:?}\nlabel = {id:?}\nmode = 'managed'\ndriver = 'managed-file'\n[items.config]\nsource = '{id}.txt'\ntarget = {:?}\n", destination.to_str().unwrap()));
    }
    fs::write(preset.join("shine.toml"), metadata).unwrap();
    for id in ["demo", "other"] {
        success(&fixture.run(&["sys", "apply", id, "--yes"]));
    }
    let receipt_path = fixture.0.join("state/sys-manifest.toml");
    let receipt = fs::read(&receipt_path).unwrap();
    let unchanged = fixture.run(&["upgrade", "--yes"]);
    success(&unchanged);
    assert!(
        stdout(&unchanged).contains("Nothing to upgrade."),
        "{unchanged:?}"
    );

    fs::write(fixture.0.join("home/demo.txt"), "user edit\n").unwrap();
    fs::write(preset.join("demo.txt"), "v2\n").unwrap();
    for args in [
        vec!["upgrade", "sys/demo", "--yes"],
        vec!["upgrade", "--yes"],
        vec!["upgrade", "--verbose", "--yes"],
    ] {
        let output = fixture.run(&args);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let text = stdout(&output);
        let summary = text
            .lines()
            .find(|line| line.starts_with("Upgrade incomplete"))
            .unwrap_or_else(|| panic!("{text}"));
        assert!(
            summary.contains("1 user-modified (kept)") && summary.contains("1 failed"),
            "{summary}"
        );
        assert!(
            !text.contains("nothing changed") && !text.contains("Nothing to upgrade."),
            "{text}"
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("configuration item(s) failed"),
            "{output:?}"
        );
        assert_eq!(fs::read(&receipt_path).unwrap(), receipt);
        assert_eq!(
            fs::read(fixture.0.join("home/demo.txt")).unwrap(),
            b"user edit\n"
        );
    }
    fixture.app(false);
    success(&fixture.run(&["app", "install", "demo", "--yes"]));
    fs::write(
        fixture.0.join("state/presets/app/demo/config.txt"),
        "static-v2\n",
    )
    .unwrap();
    let partial = fixture.run(&["upgrade", "--yes"]);
    assert_eq!(partial.status.code(), Some(1), "{partial:?}");
    let text = stdout(&partial);
    let summary = text
        .lines()
        .find(|line| line.starts_with("Upgrade incomplete"))
        .unwrap_or_else(|| panic!("{partial:?}"));
    for expected in ["1 updated", "1 user-modified (kept)", "1 failed"] {
        assert!(summary.contains(expected), "{summary}");
    }
    assert_eq!(
        fs::read(fixture.0.join("home/demo.txt")).unwrap(),
        b"user edit\n"
    );
    assert_eq!(fs::read(&receipt_path).unwrap(), receipt);
    assert_eq!(
        fs::read(fixture.0.join("destination/config.txt")).unwrap(),
        b"static-v2\n"
    );
}

#[test]
fn app_and_trust_hints_preserve_literal_category_arguments() {
    for category in ["demo", "demo space", "it's", "literal$HOME", "literal`tick"] {
        let fixture = Fixture::new();
        fixture.app(false);
        let preset = fixture.0.join("state/presets/app").join(category);
        if category != "demo" {
            fs::rename(fixture.0.join("state/presets/app/demo"), &preset).unwrap();
        }
        let metadata = fs::read_to_string(preset.join("shine.toml")).unwrap();
        fs::write(
            preset.join("shine.toml"),
            format!("{metadata}\n[artifact]\nscript = 'build.ts'\nteardown = 'build.ts'\n"),
        )
        .unwrap();
        fs::write(
            preset.join("build.ts"),
            format!(
                "await Bun.write({:?}, 'ran');\n",
                fixture.0.join("artifact-ran").to_str().unwrap()
            ),
        )
        .unwrap();
        let target = format!("app/{category}");
        let blocked = fixture.run(&["app", "artifact", "apply", category, "--yes"]);
        assert_eq!(blocked.status.code(), Some(1), "{blocked:?}");
        let error = String::from_utf8_lossy(&blocked.stderr);
        let inspect = error
            .split_once("run `")
            .unwrap()
            .1
            .split_once("` to review")
            .unwrap()
            .0;
        let grant = error
            .split_once("then `")
            .unwrap()
            .1
            .split_once("` if you accept")
            .unwrap()
            .0;
        assert_hint_argv(inspect, &["shine", "trust", "inspect", &target]);
        assert_hint_argv(grant, &["shine", "trust", "grant", &target]);
        let info = fixture.run(&["app", "info", category]);
        success(&info);
        assert_hint_argv(
            hint(&stdout(&info), "Not installed. Run `", "` to install."),
            &["shine", "install", &target],
        );
        let inspect = fixture.run(&["trust", "inspect", &target]);
        success(&inspect);
        let text = stdout(&inspect);
        let grant = text
            .lines()
            .find(|line| line.trim_start().starts_with("shine trust grant "))
            .unwrap()
            .trim();
        assert_hint_argv(grant, &["shine", "trust", "grant", &target]);
        let installed = fixture.run(&["app", "install", category, "--yes"]);
        success(&installed);
        assert_hint_argv(
            hint(&stdout(&installed), "managed files changed; run `", "`"),
            &["shine", "app", "artifact", "apply", category],
        );
        let info = fixture.run(&["app", "info", category]);
        success(&info);
        assert_hint_argv(
            hint(
                &stdout(&info),
                "Installed. Run `",
                "` to repair managed files.",
            ),
            &["shine", "install", &target, "--replace-managed"],
        );
        fs::write(preset.join("config.txt"), "updated source\n").unwrap();
        let update = fixture.run(&["update"]);
        success(&update);
        assert_hint_argv(
            hint(&stdout(&update), "Run `", "` to apply updates."),
            &["shine", "upgrade", &target],
        );
        fs::write(fixture.0.join("destination/config.txt"), "user edit\n").unwrap();
        let removed = fixture.run(&["app", "uninstall", category, "--yes"]);
        assert_eq!(removed.status.code(), Some(1), "{removed:?}");
        let text = stdout(&removed);
        assert_hint_argv(
            hint(&text, "Preview forced uninstall: ", ""),
            &[
                "shine",
                "app",
                "uninstall",
                "--force",
                "--dry-run",
                category,
            ],
        );
        assert_hint_argv(
            hint(&text, "manual: ", ")"),
            &["shine", "app", "artifact", "remove", category],
        );
        assert_eq!(
            fs::read(fixture.0.join("destination/config.txt")).unwrap(),
            b"user edit\n"
        );
        assert!(!fixture.0.join("artifact-ran").exists());
    }
}

#[test]
fn stale_trust_list_hints_preserve_targets_without_changing_grants() {
    for category in ["demo", "demo space", "it's", "literal$HOME", "literal`tick"] {
        let fixture = Fixture::new();
        fixture.app(true);
        let preset = fixture.0.join("state/presets/app").join(category);
        if category != "demo" {
            fs::rename(fixture.0.join("state/presets/app/demo"), &preset).unwrap();
        }
        let target = format!("app/{category}");
        success(&fixture.run(&["trust", "grant", &target, "--yes"]));
        let store = fixture.0.join("state/trust.toml");
        let grants = fs::read(&store).unwrap();
        let current = fixture.run(&["trust", "list", "--verbose"]);
        success(&current);
        assert!(!stdout(&current).contains("Review"), "{current:?}");

        fs::write(preset.join("generate.ts"), "// changed; never executed\n").unwrap();
        for status in ["code changed", "capability missing"] {
            if status == "capability missing" {
                fs::remove_dir_all(&preset).unwrap();
            }
            let output = fixture.run(&["trust", "list", "--verbose"]);
            success(&output);
            assert!(output.stderr.is_empty(), "{output:?}");
            let text = stdout(&output);
            assert!(text.contains(status), "{text}");
            let review = text
                .lines()
                .find_map(|line| line.trim_start().strip_prefix("Review").map(str::trim))
                .unwrap();
            assert_hint_argv(review, &["shine", "trust", "inspect", &target]);
            assert_eq!(fs::read(&store).unwrap(), grants);
        }
        assert!(!fixture.0.join("generator-ran").exists());
    }
}

#[test]
fn missing_app_source_hints_preserve_arguments_and_installed_state() {
    let fixture = Fixture::new();
    fixture.app(false);
    let category = "missing space's";
    let preset = fixture.0.join("state/presets/app").join(category);
    fs::rename(fixture.0.join("state/presets/app/demo"), &preset).unwrap();
    success(&fixture.run(&["app", "install", category, "--yes"]));
    let receipt = fs::read(fixture.0.join("state/app-manifest.toml")).unwrap();
    let metadata = fs::read_to_string(preset.join("shine.toml")).unwrap();
    fs::write(
        preset.join("shine.toml"),
        metadata.replace("[[files]]\nsource = 'config.txt'\n", ""),
    )
    .unwrap();
    let target = format!("app/{category}");
    for args in [
        vec!["update"],
        vec!["list"],
        vec!["update", "--verbose"],
        vec!["update", "--verbose", "--diff"],
        vec!["upgrade", &target, "--yes"],
    ] {
        let output = fixture.run(&args);
        success(&output);
        let text = stdout(&output);
        assert!(text.contains("Preset source missing"), "{output:?}");
        assert_hint_argv(
            hint(&text, "Preview uninstall: ", ""),
            &["shine", "app", "uninstall", "--dry-run", category],
        );
        assert_eq!(
            fs::read(fixture.0.join("state/app-manifest.toml")).unwrap(),
            receipt
        );
        assert_eq!(
            fs::read(fixture.0.join("destination/config.txt")).unwrap(),
            b"static-v1\n"
        );
    }
}

#[test]
fn leading_hyphen_uninstall_hints_reach_the_cli_parser_without_changing_state() {
    use clap::Parser;
    let fixture = Fixture::new();
    fixture.app(false);
    let category = "-demo";
    let preset = fixture.0.join("state/presets/app").join(category);
    fs::rename(fixture.0.join("state/presets/app/demo"), &preset).unwrap();
    success(&fixture.run(&["app", "install", "--yes", "--", category]));
    let metadata = fs::read_to_string(preset.join("shine.toml")).unwrap();
    fs::write(
        preset.join("shine.toml"),
        metadata.replace("[[files]]\nsource = 'config.txt'\n", ""),
    )
    .unwrap();
    let receipt = fs::read(fixture.0.join("state/app-manifest.toml")).unwrap();
    for args in [
        vec!["list"],
        vec!["update"],
        vec!["upgrade", "app/-demo", "--yes"],
    ] {
        let output = fixture.run(&args);
        success(&output);
        let text = stdout(&output);
        let command = hint(&text, "Preview uninstall: ", "");
        let argv: Vec<_> = command.split_whitespace().collect();
        cli::commands::Cli::try_parse_from(&argv).unwrap();
        success(&fixture.run(&argv[1..]));
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
}

#[test]
fn missing_shell_hints_preserve_special_names_and_dry_run_state() {
    for (category, posix, powershell) in [
        (
            "missing space",
            "'missing space/tool'",
            "'missing space/tool'",
        ),
        ("it's", "'it'\\''s/tool'", "'it''s/tool'"),
        ("-demo", "-- -demo/tool", "-- -demo/tool"),
    ] {
        let fixture = Fixture::new();
        fs::write(
            fixture.0.join("state/config.toml"),
            "schema_version = 2\nexternal_shell_mode = 'snapshot'\n",
        )
        .unwrap();
        let preset = fixture.0.join("state/presets/shell").join(category);
        fs::create_dir_all(&preset).unwrap();
        let script = if cfg!(windows) { "tool.ps1" } else { "tool.sh" };
        fs::write(
            preset.join("shine.toml"),
            format!("[[files]]\nsource = '{script}'\ntarget = 'tool'\n"),
        )
        .unwrap();
        fs::write(preset.join(script), "# never executed\n").unwrap();
        let selector = format!("{category}/tool");
        let target = format!("shell/{selector}");
        success(&fixture.run(&["trust", "grant", &target, "--yes"]));
        success(&fixture.run(&["shell", "install", "--yes", "--", &selector]));
        fs::remove_dir_all(&preset).unwrap();

        let mut paths = vec![
            fixture.0.join("state/shell-manifest.toml"),
            fixture.0.join("state/config.toml"),
            fixture.0.join("state/trust.toml"),
            fixture
                .0
                .join("state/installed/shell")
                .join(category)
                .join(script),
        ];
        let launchers = if cfg!(windows) {
            &["tool.ps1", "tool.cmd"][..]
        } else {
            &["tool"][..]
        };
        for launcher in launchers {
            paths.push(fixture.0.join("state/bin").join(launcher));
        }
        let read_state = || {
            paths
                .iter()
                .map(|path| (fs::read(path).unwrap(), fs::read_link(path).ok()))
                .collect::<Vec<_>>()
        };
        let before = read_state();
        let category_target = format!("shell/{category}");
        for args in [
            vec!["update"],
            vec!["update", "--verbose"],
            vec!["update", &category_target],
            vec!["update", &target],
        ] {
            let output = fixture.run(&args);
            success(&output);
            assert_eq!(read_state(), before, "{category}: {args:?}");
            let text = stdout(&output);
            let operands = text
                .split_once("`shine shell uninstall ")
                .unwrap_or_else(|| panic!("missing uninstall hint in {text}"))
                .1
                .split('`')
                .next()
                .unwrap();
            assert_eq!(operands, if cfg!(windows) { powershell } else { posix });
            let preview = format!("shine shell uninstall --dry-run {operands}");
            #[cfg(unix)]
            let shell = {
                let mut command = Command::new("/bin/sh");
                command.args([
                    "-c",
                    &format!(
                        "shine() {{ {} \"$@\"; }}; {preview}",
                        shine_core::shell_quote::posix_single_quote(env!("CARGO_BIN_EXE_shine"))
                    ),
                ]);
                command
            };
            #[cfg(windows)]
            let shell = {
                let mut command = Command::new("powershell.exe");
                // An alias invokes the native CLI; a PowerShell function would consume `--`.
                command.args(["-NoProfile", "-NonInteractive", "-Command"]);
                command.arg(format!(
                    "Set-Alias shine {}; {preview}; exit $LASTEXITCODE",
                    shine_core::shell_quote::powershell_single_quote(env!("CARGO_BIN_EXE_shine"))
                ));
                command
            };
            let preview = fixture.run_command(shell);
            success(&preview);
            let text = stdout(&preview);
            assert!(
                text.contains("[dry-run] No files will be modified."),
                "{text}"
            );
            assert!(
                text.lines().any(|line| line.starts_with("Bin Links")
                    && line
                        .trim_end()
                        .ends_with(&format!("{} would remove", launchers.len()))),
                "{text}"
            );
            assert_eq!(read_state(), before, "{category}: {args:?}");
            assert!(!preset.exists());
        }
    }
}

#[test]
fn shell_trust_remediation_preserves_target_and_required_mode_without_enrolling() {
    for live in [false, true] {
        let fixture = Fixture::new();
        let config = format!(
            "schema_version = 2\nexternal_shell_mode = '{}'\n",
            if live { "live" } else { "snapshot" }
        );
        fs::write(fixture.0.join("state/config.toml"), &config).unwrap();
        let category = "demo space's";
        let preset = fixture.0.join("state/presets/shell").join(category);
        fs::create_dir_all(&preset).unwrap();
        let script = if cfg!(windows) { "tool.ps1" } else { "tool.sh" };
        fs::write(
            preset.join("shine.toml"),
            format!("[[files]]\nsource = '{script}'\ntarget = 'tool'\n"),
        )
        .unwrap();
        fs::write(preset.join(script), "# not executed\n").unwrap();
        let selector = format!("{category}/tool");
        let target = format!("shell/{selector}");
        let output = fixture.run(&["shell", "install", &selector, "--yes"]);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let text = stdout(&output);
        assert!(
            text.contains(if live {
                "shell_live_requires_development_trust"
            } else {
                "shell_external_code_not_allowed"
            }),
            "{output:?}"
        );
        let error = String::from_utf8_lossy(&output.stderr);
        let inspect = error
            .split_once("run `")
            .unwrap()
            .1
            .split_once("` to review")
            .unwrap()
            .0;
        assert_hint_argv(inspect, &["shine", "trust", "inspect", &target]);
        let grant = error
            .split_once(if live { "source, run `" } else { "then `" })
            .unwrap()
            .1
            .split('`')
            .next()
            .unwrap();
        let mut expected = vec!["shine", "trust", "grant", &target];
        if live {
            expected.push("--development");
            assert!(error.contains("long-term authorization of future code"));
            assert!(error.contains("external_shell_mode = \"snapshot\""));
        } else {
            assert!(!error.contains("--development"));
        }
        assert_hint_argv(grant, &expected);
        assert_eq!(
            fs::read_to_string(fixture.0.join("state/config.toml")).unwrap(),
            config
        );
        assert!(!fixture.0.join("state/trust.toml").exists());
        assert!(!fixture.0.join("state/shell-manifest.toml").exists());
        assert!(!fixture.0.join("state/bin/tool").exists());
    }
}

#[test]
fn status_keeps_receipt_only_apps_visible_and_preserves_the_true_empty_state() {
    let fixture = Fixture::new();
    fixture.app(false);
    for args in [
        vec!["update", "--verbose"],
        vec!["update", "--verbose", "--diff"],
    ] {
        let output = fixture.run(&args);
        success(&output);
        assert!(
            stdout(&output).contains("Nothing installed yet."),
            "{output:?}"
        );
        assert!(!stdout(&output).contains("Preset source missing"));
        assert!(!fixture.0.join("state/app-manifest.toml").exists());
    }

    let category = "missing space's";
    let preset = fixture.0.join("state/presets/app").join(category);
    fs::rename(fixture.0.join("state/presets/app/demo"), &preset).unwrap();
    success(&fixture.run(&["app", "install", category, "--yes"]));
    let receipt = fs::read(fixture.0.join("state/app-manifest.toml")).unwrap();
    let config = fs::read(fixture.0.join("state/config.toml")).unwrap();
    fs::write(fixture.0.join("destination/config.txt"), "user edit\n").unwrap();
    fs::remove_dir_all(&preset).unwrap();
    for args in [
        vec!["list"],
        vec!["update"],
        vec!["update", "--diff"],
        vec!["update", "--verbose"],
        vec!["update", "--verbose", "--diff"],
        vec!["update", "--verbose", "--run-generators"],
    ] {
        let output = fixture.run(&args);
        success(&output);
        let text = stdout(&output);
        assert!(!text.contains("Nothing installed yet"), "{output:?}");
        assert_eq!(
            text.matches("Preset source missing").count(),
            2,
            "{output:?}"
        );
        for resource in ["config.txt", "manual.txt"] {
            assert!(text.contains(&format!("app/{category} · {resource}: Preset source missing; installed content and receipt retained.")), "{output:?}");
        }
        assert_hint_argv(
            hint(&text, "Preview uninstall: ", ""),
            &["shine", "app", "uninstall", "--dry-run", category],
        );
        assert!(!text.contains("Run `shine upgrade"), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert_eq!(
            fs::read(fixture.0.join("state/app-manifest.toml")).unwrap(),
            receipt
        );
        assert_eq!(
            fs::read(fixture.0.join("state/config.toml")).unwrap(),
            config
        );
        assert_eq!(
            fs::read(fixture.0.join("destination/config.txt")).unwrap(),
            b"user edit\n"
        );
        assert_eq!(
            fs::read(fixture.0.join("destination/manual.txt")).unwrap(),
            b"manual-v1\n"
        );
        assert!(!preset.exists());
        assert!(!fixture.0.join("generator-ran").exists());
    }
}

#[tokio::test]
async fn sys_all_catalog_limits_guidance_to_current_os_without_changing_lookup() {
    let fixture = Fixture::new();
    let os = cli::sys::detect_os_id().await.unwrap();
    let foreign = if os == "ubuntu" { "macos" } else { "ubuntu" };
    for (os_id, unique, label) in [
        (os.as_str(), "local-only", "Local"),
        (foreign, "foreign-only", "Foreign"),
    ] {
        let preset = fixture.0.join("state/presets/sys").join(os_id);
        fs::create_dir_all(&preset).unwrap();
        let mut metadata = "version = 2\n".to_string();
        for id in ["shared", unique] {
            fs::write(preset.join(format!("{id}.txt")), "source\n").unwrap();
            metadata.push_str(&format!("[[items]]\nid = {id:?}\nlabel = '{label} {id}'\nmode = 'managed'\ndriver = 'managed-file'\n[items.config]\nsource = '{id}.txt'\ntarget = {:?}\n", fixture.0.join(format!("{os_id}-{id}.txt")).to_str().unwrap()));
        }
        fs::write(preset.join("shine.toml"), metadata).unwrap();
    }
    let config = fs::read(fixture.0.join("state/config.toml")).unwrap();
    let all = fixture.run(&["sys", "list", "--all"]);
    success(&all);
    let text = stdout(&all);
    assert!(
        text.contains(&format!("{foreign} (catalog only)")),
        "{text}"
    );
    assert!(text.contains("Commands use the current OS"), "{text}");
    assert_eq!(
        text.matches("Run: shine sys apply shared").count(),
        1,
        "{text}"
    );
    assert!(text.contains("Run: shine sys apply local-only"), "{text}");
    assert!(
        !text.contains("Run: shine sys apply foreign-only"),
        "{text}"
    );
    let local = fixture.run(&["sys", "list"]);
    success(&local);
    assert!(!stdout(&local).contains("Foreign"));
    assert!(!stdout(&local).contains("catalog only"));
    let info = fixture.run(&["sys", "info", "shared"]);
    success(&info);
    assert!(stdout(&info).contains("Local shared"));
    let info = fixture.run(&["sys", "info", "foreign-only"]);
    assert_eq!(info.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&info.stderr).contains("unknown sys item"));
    let apply = fixture.run(&["sys", "apply", "shared", "--dry-run"]);
    success(&apply);
    assert!(stdout(&apply).contains("Local shared"), "{apply:?}");
    let apply = fixture.run(&["sys", "apply", "foreign-only", "--dry-run"]);
    assert_eq!(apply.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&apply.stderr).contains("unknown sys item"));
    for os_id in [os.as_str(), foreign] {
        assert!(!fixture.0.join(format!("{os_id}-shared.txt")).exists());
    }
    assert_eq!(
        fs::read(fixture.0.join("state/config.toml")).unwrap(),
        config
    );
    assert!(!fixture.0.join("state/sys-manifest.toml").exists());
}

#[test]
fn refresh_summary_distinguishes_preservation_failure_and_partial_progress() {
    for mixed in [false, true] {
        let fixture = Fixture::new();
        let preset = fixture.0.join("state/presets/app/demo");
        fs::create_dir_all(&preset).unwrap();
        let mut metadata = format!(
            "metadata_schema_version = 2\ndest = {:?}\n[permissions]\nschema_version = 2\nopaque_code = 'unrestricted'\nenvironment = [{{ name = 'ENABLE', sensitivity = 'plain' }}]\n",
            fixture.0.join("destination").to_str().unwrap()
        );
        for id in 0..if mixed { 3 } else { 1 } {
            metadata.push_str(&format!("[[files]]\nsource = '{id}.txt'\ngenerator = {{ script = '{id}.ts', runtime = 'bun', when_env = 'ENABLE', env = ['ENABLE'], auto = false }}\n"));
            fs::write(preset.join(format!("{id}.txt")), "fallback\n").unwrap();
            fs::write(preset.join(format!("{id}.ts")), "console.log('v1');\n").unwrap();
        }
        fs::write(preset.join("shine.toml"), metadata).unwrap();
        success(&fixture.run(&["trust", "grant", "app/demo", "--yes"]));
        success(&fixture.run(&["app", "install", "demo", "--yes"]));
        let receipt = fs::read(fixture.0.join("state/app-manifest.toml")).unwrap();
        fs::write(fixture.0.join("destination/0.txt"), "user edit\n").unwrap();
        if mixed {
            fs::write(preset.join("1.ts"), "throw new Error('fixture failure');\n").unwrap();
            fs::write(preset.join("2.ts"), "console.log('v2');\n").unwrap();
            success(&fixture.run(&["trust", "grant", "app/demo", "--yes"]));
        }
        let denied = fixture.run(&["app", "refresh", "demo"]);
        assert_eq!(denied.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&denied.stderr).contains("approval requires"));
        assert_eq!(
            fs::read(fixture.0.join("state/app-manifest.toml")).unwrap(),
            receipt
        );
        let output = fixture.run(&["app", "refresh", "demo", "--yes"]);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let text = stdout(&output);
        let summary = text
            .lines()
            .find(|line| line.contains("Refresh incomplete:"))
            .unwrap();
        assert!(summary.contains("user-modified (kept)"), "{output:?}");
        assert_eq!(
            fs::read(fixture.0.join("destination/0.txt")).unwrap(),
            b"user edit\n"
        );
        if mixed {
            assert!(
                summary.contains("1 updated, 1 user-modified (kept), 1 failed"),
                "{output:?}"
            );
            assert_eq!(
                fs::read(fixture.0.join("destination/1.txt")).unwrap(),
                b"v1\n"
            );
            assert_eq!(
                fs::read(fixture.0.join("destination/2.txt")).unwrap(),
                b"v2\n"
            );
            let before: shine_core::install::manifest::AppManifest =
                toml::from_slice(&receipt).unwrap();
            let after: shine_core::install::manifest::AppManifest =
                toml::from_slice(&fs::read(fixture.0.join("state/app-manifest.toml")).unwrap())
                    .unwrap();
            for file in ["0.txt", "1.txt"] {
                let destination = fixture.0.join("destination").join(file);
                assert_eq!(
                    before
                        .entries
                        .iter()
                        .find(|entry| entry.destination == destination),
                    after
                        .entries
                        .iter()
                        .find(|entry| entry.destination == destination)
                );
            }
        } else {
            assert!(!summary.contains("failed"), "{output:?}");
            assert!(!String::from_utf8_lossy(&output.stderr).contains("failed to refresh"));
            assert_eq!(
                fs::read(fixture.0.join("state/app-manifest.toml")).unwrap(),
                receipt
            );
            success(&fixture.run(&["app", "refresh", "demo", "--force", "--yes"]));
            assert_eq!(
                fs::read(fixture.0.join("destination/0.txt")).unwrap(),
                b"v1\n"
            );
        }
    }
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
fn shell_catalog_uses_active_overlay_in_both_base_modes() {
    let extension = if cfg!(windows) { "ps1" } else { "sh" };
    for external in [false, true] {
        let fixture = Fixture::new();
        let config = fixture.0.join(if external {
            "state/config.toml"
        } else {
            "shine.config.toml"
        });
        if external {
            let proxy = fixture.0.join("state/presets/shell/proxy");
            fs::create_dir_all(&proxy).unwrap();
            fs::write(
                proxy.join("shine.toml"),
                format!("[[files]]\nsource = 'set_proxy.{extension}'\ntarget = 'setproxy'\n"),
            )
            .unwrap();
            fs::write(
                proxy.join(format!("set_proxy.{extension}")),
                "# Base description\n",
            )
            .unwrap();
        }
        let overlay = fixture.0.join("overlay");
        let proxy = overlay.join("shell/proxy");
        let extra = overlay.join("shell/review-extra");
        fs::create_dir_all(&proxy).unwrap();
        fs::create_dir_all(&extra).unwrap();
        fs::write(
            proxy.join(format!("set_proxy.{extension}")),
            "# Overlay description sentinel\n",
        )
        .unwrap();
        fs::write(
            extra.join("shine.toml"),
            format!("[[files]]\nsource = 'extra.{extension}'\ntarget = 'review-extra'\n"),
        )
        .unwrap();
        fs::write(
            extra.join(format!("extra.{extension}")),
            "# Overlay addition\n",
        )
        .unwrap();
        for with_overlay in [false, true] {
            let config_text = format!(
                "schema_version = 2\n{}",
                if with_overlay {
                    format!("presets_overlay_dir = {:?}\n", overlay.to_str().unwrap())
                } else {
                    String::new()
                }
            );
            fs::write(&config, &config_text).unwrap();
            for args in [vec!["shell", "list"], vec!["list", "--available", "shell"]] {
                let output = fixture.run(&args);
                success(&output);
                let text = stdout(&output);
                assert_eq!(text.contains("review-extra"), with_overlay, "{output:?}");
                assert_eq!(
                    text.contains("Overlay description sentinel"),
                    with_overlay,
                    "{output:?}"
                );
                assert!(text.contains("setproxy"), "{output:?}");
            }
            if with_overlay {
                for args in [
                    vec!["shell", "info", "proxy"],
                    vec!["shell", "info", "review-extra"],
                    vec!["shell", "install", "review-extra", "--dry-run"],
                ] {
                    let output = fixture.run(&args);
                    success(&output);
                    assert!(
                        stdout(&output).contains(if args[2] == "proxy" {
                            "Overlay description sentinel"
                        } else {
                            "review-extra"
                        }),
                        "{output:?}"
                    );
                }
            }
            assert_eq!(fs::read_to_string(&config).unwrap(), config_text);
            assert!(!fixture.0.join("state/shell-manifest.toml").exists());
        }
    }
}

#[test]
fn embedded_shell_catalog_does_not_read_invalid_trust_but_runtime_paths_still_do() {
    for (contents, error) in [
        (
            Some("unknown='TRUST_DOCUMENT_CANARY' invalid\n"),
            "parsing trust store",
        ),
        (
            Some("schema_version='TRUST_DOCUMENT_CANARY'\ngrants=[]\n"),
            "parsing trust store",
        ),
        (
            Some("schema_version = 999\ngrants = []\n"),
            "unsupported trust store schema version 999",
        ),
        (None, "trust store must be a regular file"),
    ] {
        let fixture = Fixture::new();
        let project = fixture.0.join("shine.config.toml");
        let project_text = "schema_version = 2\n";
        fs::write(&project, project_text).unwrap();
        let commands = [vec!["shell", "list"], vec!["list", "--available", "shell"]];
        let baseline: Vec<_> = commands.iter().map(|args| fixture.run(args)).collect();
        for output in &baseline {
            success(output);
            assert!(
                stdout(output).contains("Shell Preset Categories"),
                "{output:?}"
            );
        }
        let trust = fixture.0.join("state/trust.toml");
        if let Some(contents) = contents {
            fs::write(&trust, contents).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&trust, fs::Permissions::from_mode(0o600)).unwrap();
            }
        } else {
            fs::create_dir(&trust).unwrap();
        }
        for (args, baseline) in commands.iter().zip(baseline) {
            let output = fixture.run(args);
            success(&output);
            assert_eq!(output.stdout, baseline.stdout, "{output:?}");
            assert_eq!(output.stderr, baseline.stderr, "{output:?}");
        }
        assert_eq!(fs::read_to_string(&project).unwrap(), project_text);

        // The exception is only for the pristine embedded catalog, never runtime-backed sources.
        let overlay = fixture.0.join("overlay");
        fs::create_dir(&overlay).unwrap();
        for source in [
            "presets_overlay_dir = 'overlay'",
            "presets_dir = 'state/presets'",
        ] {
            fs::write(&project, format!("{project_text}{source}\n")).unwrap();
            for args in &commands {
                let output = fixture.run(args);
                assert_eq!(output.status.code(), Some(1), "{output:?}");
                assert!(
                    String::from_utf8_lossy(&output.stderr).contains(error),
                    "{output:?}"
                );
                assert!(
                    !String::from_utf8_lossy(&output.stderr).contains("TRUST_DOCUMENT_CANARY"),
                    "{output:?}"
                );
                assert!(
                    !stdout(&output).contains("Shell Preset Categories"),
                    "{output:?}"
                );
            }
        }
        fs::write(&project, project_text).unwrap();
        let install = fixture.run(&["shell", "install", "proxy", "--yes"]);
        assert_eq!(install.status.code(), Some(1), "{install:?}");
        assert!(
            String::from_utf8_lossy(&install.stderr).contains(error),
            "{install:?}"
        );
        assert!(
            !String::from_utf8_lossy(&install.stderr).contains("TRUST_DOCUMENT_CANARY"),
            "{install:?}"
        );
        assert!(!fixture.0.join("state/shell-manifest.toml").exists());
        if let Some(contents) = contents {
            assert_eq!(fs::read_to_string(&trust).unwrap(), contents);
        } else {
            assert!(trust.is_dir());
            assert_eq!(fs::read_dir(&trust).unwrap().count(), 0);
        }
    }
}

#[test]
fn global_generator_notices_escape_labels_and_keep_failure_exit_and_column_alignment() {
    let fixture = Fixture::new();
    fixture.app(false);
    success(&fixture.run(&["app", "install", "demo", "--yes"]));
    fixture.app(true);
    let preset = fixture.0.join("state/presets/app/demo");
    let label = "manual\nFORGED\r\t\u{1b}中文";
    let metadata = fs::read_to_string(preset.join("shine.toml"))
        .unwrap()
        .replace(
            "source = 'manual.txt'",
            &format!(
                "source = 'manual.txt'\ndisplay_name = {}",
                toml::Value::String(label.into())
            ),
        );
    fs::write(preset.join("shine.toml"), &metadata).unwrap();
    fs::write(preset.join("generate.ts"), format!(
        "await Bun.write({:?}, 'ran'); process.stderr.write('PRIVATE_DETAIL'); process.exit(9);\n",
        fixture.0.join("generator-ran").to_str().unwrap())).unwrap();
    let manifest = fs::read(fixture.0.join("state/app-manifest.toml")).unwrap();
    for trusted in [false, true] {
        if trusted {
            success(&fixture.run(&["trust", "grant", "app/demo", "--yes"]));
        }
        for args in [
            vec!["update", "--run-generators"],
            vec!["update", "--verbose", "--diff", "--run-generators"],
        ] {
            let output = fixture.run(&args);
            assert_eq!(output.status.code(), Some(1), "{output:?}");
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains("one or more App generators could not be evaluated")
            );
            let text = stdout(&output);
            assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
            assert!(
                !text.contains("\nFORGED") && !text.contains("PRIVATE_DETAIL"),
                "{text:?}"
            );
            assert!(
                text.contains(r"! manual\nFORGED\r\t\u{1b}中文: generator"),
                "{text:?}"
            );
            assert!(!text.contains("Nothing to update."));
            if args.contains(&"--diff") {
                let columns: Vec<_> = text
                    .lines()
                    .filter_map(|line| line.split_once("  →  "))
                    .map(|(prefix, _)| console::measure_text_width(prefix))
                    .collect();
                assert_eq!(columns.len(), 2, "{text}");
                assert_eq!(columns[0], columns[1], "{text}");
            }
        }
        assert_eq!(fixture.0.join("generator-ran").exists(), trusted);
    }
    assert_eq!(
        fs::read(fixture.0.join("state/app-manifest.toml")).unwrap(),
        manifest
    );
    assert_eq!(
        fs::read_to_string(fixture.0.join("destination/manual.txt")).unwrap(),
        "manual-v1\n"
    );
    assert_eq!(
        fs::read_to_string(preset.join("shine.toml")).unwrap(),
        metadata
    );
}

#[test]
fn global_manual_refresh_hints_preserve_source_identity_without_copying_controls() {
    let fixture = Fixture::new();
    fixture.app(false);
    success(&fixture.run(&["app", "install", "demo", "--yes"]));
    fixture.app(true);
    let preset = fixture.0.join("state/presets/app/demo");
    let source = if cfg!(unix) {
        "manual\nFORGED\r\t\u{1b}.txt"
    } else {
        "manual 中文.txt"
    };
    let metadata = fs::read_to_string(preset.join("shine.toml"))
        .unwrap()
        .replace(
            "source = 'manual.txt'",
            &format!(
                "source = {}\ntarget = 'manual.txt'",
                toml::Value::String(source.into())
            ),
        );
    fs::write(preset.join("shine.toml"), &metadata).unwrap();
    fs::rename(preset.join("manual.txt"), preset.join(source)).unwrap();
    success(&fixture.run(&["trust", "grant", "app/demo", "--yes"]));
    let manifest = fs::read(fixture.0.join("state/app-manifest.toml")).unwrap();
    let deferred = fixture.run(&["update"]);
    success(&deferred);
    assert!(!fixture.0.join("generator-ran").exists());
    for args in [
        vec!["update", "--run-generators"],
        vec!["update", "--verbose", "--run-generators"],
    ] {
        let output = fixture.run(&args);
        success(&output);
        assert!(output.stderr.is_empty(), "{output:?}");
        let text = stdout(&output);
        assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
        assert!(!text.contains("\nFORGED"), "{text:?}");
        if cfg!(unix) {
            assert!(text.contains("original category and source arguments (control characters; copyable command omitted)"), "{text}");
            assert!(!text.contains("`shine app refresh demo"));
        } else {
            assert!(
                text.contains("`shine app refresh demo 'manual 中文.txt'`"),
                "{text}"
            );
        }
        assert!(!text.contains("shine upgrade app/demo"));
    }
    assert!(fixture.0.join("generator-ran").exists());
    assert_eq!(
        fs::read(fixture.0.join("state/app-manifest.toml")).unwrap(),
        manifest
    );
    assert_eq!(
        fs::read_to_string(fixture.0.join("destination/manual.txt")).unwrap(),
        "manual-v1\n"
    );
    assert_eq!(
        fs::read_to_string(preset.join("shine.toml")).unwrap(),
        metadata
    );
    assert!(preset.join(source).exists());
}

#[tokio::test]
async fn global_sys_change_details_escape_recorded_values_without_touching_resources() {
    let fixture = Fixture::new();
    let os_id = cli::sys::detect_os_id().await.unwrap();
    let preset = fixture.0.join("state/presets/sys").join(&os_id);
    fs::create_dir_all(&preset).unwrap();
    fs::write(fixture.0.join("state/config.toml"), "schema_version=2\n[env]\nPRIVATE_DNS_DOMAIN='example.test'\nPRIVATE_DNS_SERVERS='1.1.1.1'\n").unwrap();
    fs::write(preset.join("shine.toml"), "version=2\n[[items]]\nid='demo'\nlabel='DNS demo'\nmode='managed'\ndriver='split-dns'\nrequired_env=['PRIVATE_DNS_DOMAIN','PRIVATE_DNS_SERVERS']\n[items.config]\ndomain_env='PRIVATE_DNS_DOMAIN'\nservers_env='PRIVATE_DNS_SERVERS'\n").unwrap();
    let old = toml::Value::String("old\nFORGED\r\t\u{1b}".into()).to_string();
    let receipt = format!(
        "schema_version=1\n[[entries]]\nos_id={os_id:?}\nitem_id='demo'\nlabel='DNS demo'\nstatus='updated'\nupdated_at='1'\nmanaged=true\n[entries.receipt]\ndriver='split-dns'\nversion=1\nos_id={os_id:?}\nitem_id='demo'\ndomain={old}\nservers=['8.8.8.8']\nresource={old}\n"
    );
    fs::write(fixture.0.join("state/sys-manifest.toml"), &receipt).unwrap();
    for args in [vec!["update", "--diff"], vec!["update", "--verbose"]] {
        let output = fixture.run(&args);
        success(&output);
        assert!(output.stderr.is_empty(), "{output:?}");
        let text = stdout(&output);
        assert!(
            text.contains(r"Domain: old\nFORGED\r\t\u{1b} -> example.test"),
            "{text:?}"
        );
        assert!(
            text.contains(r"Resource: old\nFORGED\r\t\u{1b} -> "),
            "{text:?}"
        );
        assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
        assert!(!text.contains("\nFORGED"));
    }
    assert_eq!(
        fs::read_to_string(fixture.0.join("state/sys-manifest.toml")).unwrap(),
        receipt
    );
    assert!(!fixture.0.join("state/operations").exists());
}

#[test]
fn targeted_update_encodes_relocation_rows_and_preserves_installed_files() {
    let fixture = Fixture::new();
    fixture.app(false);
    success(&fixture.run(&["app", "install", "demo", "--yes"]));
    let manifest = fs::read(fixture.0.join("state/app-manifest.toml")).unwrap();
    let metadata = fixture.0.join("state/presets/app/demo/shine.toml");
    let original = fs::read_to_string(&metadata).unwrap();
    let name = if cfg!(unix) {
        "new\nFORGED\r\t\u{1b}中文"
    } else {
        "new中文"
    };
    let destination = fixture.0.join(name);
    let changed = original.replace(
        &format!("{:?}", fixture.0.join("destination").to_str().unwrap()),
        &toml::Value::String(destination.to_str().unwrap().into()).to_string(),
    );
    assert_ne!(original, changed);
    fs::write(&metadata, &changed).unwrap();
    for args in [
        vec!["update", "app/demo"],
        vec!["update", "--diff"],
        vec!["update", "--verbose", "--diff"],
    ] {
        let output = fixture.run(&args);
        success(&output);
        assert!(output.stderr.is_empty(), "{output:?}");
        let text = stdout(&output);
        assert!(
            text.contains("destination:") && text.contains("content: unchanged"),
            "{text:?}"
        );
        assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
        assert!(!text.contains("\nFORGED"), "{text:?}");
        if cfg!(unix) {
            assert!(text.contains(r"new\nFORGED\r\t\u{1b}中文"));
        }
    }
    assert_eq!(
        fs::read(fixture.0.join("state/app-manifest.toml")).unwrap(),
        manifest
    );
    assert_eq!(
        fs::read_to_string(fixture.0.join("destination/config.txt")).unwrap(),
        "static-v1\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.0.join("destination/manual.txt")).unwrap(),
        "manual-v1\n"
    );
    assert!(!destination.exists());
    assert_eq!(fs::read_to_string(&metadata).unwrap(), changed);
}

#[test]
fn generator_failure_notices_encode_renamed_source_and_remain_read_only() {
    let fixture = Fixture::new();
    fixture.app(false);
    success(&fixture.run(&["app", "install", "demo", "--yes"]));
    fixture.app(true);
    let preset = fixture.0.join("state/presets/app/demo");
    let name = if cfg!(unix) {
        "manual\nFORGED\r\t\u{1b}.txt"
    } else {
        "manual中文.txt"
    };
    let metadata = fs::read_to_string(preset.join("shine.toml"))
        .unwrap()
        .replace(
            "source = 'manual.txt'",
            &format!(
                "source = {}\ntarget = 'manual.txt'",
                toml::Value::String(name.into())
            ),
        );
    fs::write(preset.join("shine.toml"), &metadata).unwrap();
    fs::rename(preset.join("manual.txt"), preset.join(name)).unwrap();
    fs::write(preset.join("generate.ts"), format!(
        "await Bun.write({:?}, 'ran'); process.stderr.write('PRIVATE_GENERATOR_DETAIL'); process.exit(9);\n",
        fixture.0.join("generator-ran").to_str().unwrap())).unwrap();
    let manifest = fs::read(fixture.0.join("state/app-manifest.toml")).unwrap();
    for trusted in [false, true] {
        if trusted {
            success(&fixture.run(&["trust", "grant", "app/demo", "--yes"]));
        }
        for command in ["info", "update"] {
            // Selecting a static sibling still reports every category-scoped evaluation failure.
            let output = fixture.run(&[command, "app/demo/config.txt", "--run-generators"]);
            assert_eq!(output.status.code(), Some(1), "{output:?}");
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains("one or more App generators could not be evaluated")
            );
            let text = stdout(&output);
            assert!(text.contains(if trusted {
                "generator evaluation failed"
            } else {
                "generator trust required"
            }));
            assert!(!text.contains("PRIVATE_GENERATOR_DETAIL"));
            assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
            assert!(!text.contains("\nFORGED"), "{text:?}");
            if cfg!(unix) {
                assert!(text.contains(r"demo/manual\nFORGED\r\t\u{1b}.txt:"));
            }
            assert!(!text.contains("No update available"));
        }
        assert_eq!(fixture.0.join("generator-ran").exists(), trusted);
    }
    assert_eq!(
        fs::read(fixture.0.join("state/app-manifest.toml")).unwrap(),
        manifest
    );
    assert_eq!(
        fs::read_to_string(fixture.0.join("destination/manual.txt")).unwrap(),
        "manual-v1\n"
    );
    assert_eq!(
        fs::read_to_string(preset.join("shine.toml")).unwrap(),
        metadata
    );
}

#[test]
fn targeted_generator_notices_follow_selected_resources() {
    let fixture = Fixture::new();
    fixture.app(false);
    success(&fixture.run(&["app", "install", "demo", "--yes"]));
    fixture.app(true);
    let shell = fixture.0.join("state/presets/shell/tools");
    fs::create_dir_all(&shell).unwrap();
    let script = if cfg!(windows) { "tool.ps1" } else { "tool.sh" };
    fs::write(
        shell.join("shine.toml"),
        format!("[[files]]\nsource = '{script}'\ntarget = 'review-tool'\n"),
    )
    .unwrap();
    fs::write(shell.join(script), "# Never executed\n").unwrap();
    success(&fixture.run(&["trust", "grant", "shell/tools/review-tool", "--yes"]));
    success(&fixture.run(&["shell", "install", "tools", "--yes"]));
    let paths = [
        "state/app-manifest.toml",
        "state/shell-manifest.toml",
        "destination/config.txt",
        "destination/manual.txt",
    ];
    let before: Vec<_> = paths
        .iter()
        .map(|p| fs::read(fixture.0.join(p)).unwrap())
        .collect();
    for target in [
        "shell/tools",
        "shell/tools/review-tool",
        "review-tool",
        "app/demo/config.txt",
        "config.txt",
    ] {
        for command in ["info", "update"] {
            let output = fixture.run(&[command, target]);
            success(&output);
            let text = stdout(&output);
            assert!(!text.contains("not evaluated"), "{output:?}");
            assert!(!text.contains("--run-generators"), "{output:?}");
            if command == "update" {
                assert!(
                    text.contains(&format!("No update available for {target}.")),
                    "{output:?}"
                );
            }
        }
    }
    for target in ["app/demo", "app/demo/manual.txt", "manual.txt"] {
        for command in ["info", "update"] {
            let output = fixture.run(&[command, target]);
            success(&output);
            let text = stdout(&output);
            assert!(
                text.contains("Generated configuration was not evaluated"),
                "{output:?}"
            );
            assert!(!text.contains("No update available"), "{output:?}");
        }
    }
    for command in ["info", "update"] {
        let output = fixture.run(&[command, "shell/tools/review-tool", "--run-generators"]);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("--run-generators requires an App target"),
            "{output:?}"
        );
    }
    assert!(!fixture.0.join("generator-ran").exists());
    for (path, bytes) in paths.iter().zip(before) {
        assert_eq!(fs::read(fixture.0.join(path)).unwrap(), bytes);
    }
}

#[test]
fn targeted_update_keeps_generator_failures_out_of_no_update_footer() {
    let fixture = Fixture::new();
    fixture.app(false);
    success(&fixture.run(&["app", "install", "demo", "--yes"]));
    fixture.app(true);
    let preset = fixture.0.join("state/presets/app/demo");
    fs::write(
        preset.join("generate.ts"),
        format!(
            "await Bun.write({:?}, 'ran'); throw new Error('targeted evaluation failure');\n",
            fixture.0.join("generator-ran").to_str().unwrap()
        ),
    )
    .unwrap();
    let paths = [
        "state/app-manifest.toml",
        "destination/config.txt",
        "destination/manual.txt",
    ];
    let before: Vec<_> = paths
        .iter()
        .map(|p| fs::read(fixture.0.join(p)).unwrap())
        .collect();
    for trusted in [false, true] {
        if trusted {
            success(&fixture.run(&["trust", "grant", "app/demo", "--yes"]));
        }
        // Explicit evaluation stays category-scoped, including failures in an unselected sibling.
        for target in ["app/demo", "app/demo/manual.txt", "app/demo/config.txt"] {
            for command in ["info", "update"] {
                let output = fixture.run(&[command, target, "--run-generators"]);
                assert_eq!(output.status.code(), Some(1), "{output:?}");
                let text = stdout(&output);
                assert!(
                    text.contains(if trusted {
                        "generator evaluation failed"
                    } else {
                        "generator trust required"
                    }),
                    "{output:?}"
                );
                assert!(!text.contains("No update available"), "{output:?}");
                assert!(
                    String::from_utf8_lossy(&output.stderr)
                        .contains("one or more App generators could not be evaluated"),
                    "{output:?}"
                );
            }
        }
        assert_eq!(fixture.0.join("generator-ran").exists(), trusted);
    }
    fs::write(preset.join("config.txt"), "static-v2\n").unwrap();
    success(&fixture.run(&["trust", "grant", "app/demo", "--yes"]));
    let mixed = fixture.run(&["update", "app/demo", "--run-generators"]);
    assert_eq!(mixed.status.code(), Some(1), "{mixed:?}");
    let text = stdout(&mixed);
    assert!(
        text.contains("update available") && text.contains("generator evaluation failed"),
        "{mixed:?}"
    );
    fs::write(preset.join("config.txt"), "static-v1\n").unwrap();
    fs::write(preset.join("generate.ts"), "console.log('manual-v1');\n").unwrap();
    success(&fixture.run(&["trust", "grant", "app/demo", "--yes"]));
    let current = fixture.run(&["update", "app/demo", "--run-generators"]);
    success(&current);
    assert!(
        stdout(&current).contains("No update available for app/demo."),
        "{current:?}"
    );
    for (path, bytes) in paths.iter().zip(before) {
        assert_eq!(fs::read(fixture.0.join(path)).unwrap(), bytes);
    }
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
