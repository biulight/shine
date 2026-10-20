//! Process-level routing and consent contracts, independent of module layout.
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("shine-dispatch-{}", uuid::Uuid::new_v4()));
        for path in ["home", "home/AppData/Roaming", "home/AppData/Local"] {
            fs::create_dir_all(root.join(path)).unwrap();
        }
        Self(root)
    }
    fn command(&self) -> Command {
        self.configure(Command::new(env!("CARGO_BIN_EXE_shine")))
    }
    fn configure(&self, mut command: Command) -> Command {
        command
            .current_dir(&self.0)
            .env("HOME", self.0.join("home"))
            .env("USERPROFILE", self.0.join("home"))
            .env("APPDATA", self.0.join("home/AppData/Roaming"))
            .env("LOCALAPPDATA", self.0.join("home/AppData/Local"))
            .env("SHINE_CONFIG_DIR", self.0.join("state"))
            .env_remove("SHINE_PRESETS")
            .stdin(Stdio::null());
        command
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
#[cfg(unix)]
fn app_generator_notes_keep_stderr_and_hook_notes_keep_stdout() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let preset = state.join("presets/app/demo");
    let destination = fixture.0.join("destination");
    fs::create_dir_all(&preset).unwrap();
    let dest = toml::Value::String(destination.to_string_lossy().into_owned());
    fs::write(preset.join("shine.toml"), format!(
        "metadata_schema_version = 2\ndest = {dest}\npost_install = {{ command = '/bin/sh', args = ['-c', 'printf HOOK_STDOUT_NOTE'], show_output = true }}\npost_upgrade = {{ command = '/bin/sh', args = ['-c', 'printf HOOK_STDOUT_NOTE'], show_output = true }}\n[permissions]\nschema_version = 1\ncommands = ['/bin/sh']\nfilesystem = [{{ access = ['execute'], base = 'preset', path = 'generate.sh' }}]\nenvironment = [{{ name = 'ENABLE', sensitivity = 'plain' }}]\n[[files]]\nsource = 'config.txt'\ngenerator = {{ script = 'generate.sh', env = ['ENABLE'], when_env = 'ENABLE' }}\n"
    )).unwrap();
    fs::write(preset.join("config.txt"), "fallback").unwrap();
    let script = preset.join("generate.sh");
    fs::write(
        &script,
        "#!/bin/sh\nprintf '%s' \"$ENABLE\"\nprintf 'GENERATOR_STDERR_NOTE\\nsecond line\\n' >&2\n",
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let run = |args: &[&str]| {
        fixture
            .command()
            .env("NO_COLOR", "1")
            .args(args)
            .output()
            .unwrap()
    };
    let config = state.join("config.toml");
    fs::write(&config, "schema_version = 2\n[env]\nENABLE = '1'\n").unwrap();
    let trusted = run(&["trust", "grant", "app/demo", "--yes"]);
    assert!(trusted.status.success(), "{trusted:?}");
    let trust = fs::read(state.join("trust.toml")).unwrap();
    for (value, args) in [
        ("1", vec!["app", "install", "demo", "--yes"]),
        ("2", vec!["upgrade", "app/demo", "--yes", "--verbose"]),
        ("3", vec!["app", "refresh", "demo", "config.txt", "--yes"]),
    ] {
        let contents = format!("schema_version = 2\n[env]\nENABLE = '{value}'\n");
        fs::write(&config, &contents).unwrap();
        let output = run(&args);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "     GENERATOR_STDERR_NOTE\n     second line\n"
        );
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(text.contains("HOOK_STDOUT_NOTE"), "{output:?}");
        assert!(!text.contains("GENERATOR_STDERR_NOTE"), "{output:?}");
        assert_eq!(
            fs::read(destination.join("config.txt")).unwrap(),
            value.as_bytes()
        );
        assert_eq!(fs::read_to_string(&config).unwrap(), contents);
        assert_eq!(fs::read(state.join("trust.toml")).unwrap(), trust);
        assert!(!state.join("app-operation-journal.toml").exists());
    }
    // A no-op refresh still renders the note without changing content or receipts.
    let manifest = fs::read(state.join("app-manifest.toml")).unwrap();
    let noop = run(&["app", "refresh", "demo", "--yes"]);
    assert_eq!(noop.status.code(), Some(0), "{noop:?}");
    assert!(String::from_utf8_lossy(&noop.stdout).contains("Already up to date"));
    assert_eq!(
        String::from_utf8_lossy(&noop.stderr),
        "     GENERATOR_STDERR_NOTE\n     second line\n"
    );
    assert_eq!(fs::read(state.join("app-manifest.toml")).unwrap(), manifest);
    assert_eq!(fs::read(destination.join("config.txt")).unwrap(), b"3");
}

#[test]
fn shell_install_preview_labels_candidates_without_changing_conflict_policy() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let preset = state.join("presets/shell/demo");
    fs::create_dir_all(&preset).unwrap();
    let source = if cfg!(windows) { "run.ps1" } else { "run.sh" };
    fs::write(
        preset.join("shine.toml"),
        format!("[[files]]\nsource = '{source}'\ntarget = 'demo-run'\n"),
    )
    .unwrap();
    fs::write(preset.join(source), "# fixture; never executed\n").unwrap();
    let config = "schema_version = 2\n";
    fs::write(state.join("config.toml"), config).unwrap();
    fs::create_dir_all(state.join("bin")).unwrap();
    let launcher = state.join("bin").join(if cfg!(windows) {
        "demo-run.ps1"
    } else {
        "demo-run"
    });
    fs::write(&launcher, "foreign launcher\n").unwrap();
    let run = |args: &[&str]| {
        fixture
            .command()
            .env("NO_COLOR", "1")
            .args(args)
            .output()
            .unwrap()
    };
    let trusted = run(&["trust", "grant", "shell/demo/demo-run", "--yes"]);
    assert_eq!(trusted.status.code(), Some(0), "{trusted:?}");
    let trust = fs::read(state.join("trust.toml")).unwrap();
    for flags in [vec![], vec!["--replace-managed"], vec!["--verbose"]] {
        let mut args = vec!["shell", "install", "demo", "--dry-run"];
        args.extend(flags);
        let output = run(&args);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(
            text.contains("link candidates only (not a security Plan)"),
            "{text}"
        );
        assert!(text.contains("Link candidate demo-run:"), "{text}");
        assert!(text.contains("--replace-managed effects"), "{text}");
        assert!(
            text.contains("--verbose does not expand this preview"),
            "{text}"
        );
        assert!(!text.contains("Would link"), "{text}");
    }
    let blocked = run(&["shell", "install", "demo", "--yes"]);
    assert_eq!(blocked.status.code(), Some(1), "{blocked:?}");
    assert!(
        String::from_utf8_lossy(&blocked.stderr).contains("ownership conflict"),
        "{blocked:?}"
    );
    assert_eq!(fs::read_to_string(&launcher).unwrap(), "foreign launcher\n");
    assert_eq!(
        fs::read_to_string(state.join("config.toml")).unwrap(),
        config
    );
    assert_eq!(fs::read(state.join("trust.toml")).unwrap(), trust);
    for path in [
        "installed/shell",
        "shell",
        "shell-manifest.toml",
        "shell-operation-journal.toml",
    ] {
        assert!(!state.join(path).exists(), "{path}");
    }
    assert!(!fixture.0.join("home/.zshrc").exists());
    assert!(!fixture.0.join("home/Documents/PowerShell").exists());
}

#[test]
fn preset_lint_text_counts_preserve_json_and_warning_exits() {
    let fixture = Fixture::new();
    let category = fixture.0.join("app/demo");
    fs::create_dir_all(&category).unwrap();
    fs::write(category.join("shine.toml"), "metadata_schema_version = 2\ndescription = 'Demo'\ndest = '~/.config/demo'\n[[files]]\nsource = 'config.txt'\n").unwrap();
    fs::write(category.join("config.txt"), "fixture\n").unwrap();
    for (missing, expected, exit) in [
        (false, "Summary: 1 category, 0 errors, 1 warning", 0),
        (true, "Summary: 0 categories, 1 error, 0 warnings", 1),
    ] {
        let path = if missing {
            fixture.0.join("missing")
        } else {
            category.clone()
        };
        for deny in [false, true] {
            let run = |json| {
                let mut command = fixture.command();
                command
                    .env("NO_COLOR", "1")
                    .args(["preset", "lint"])
                    .arg(&path);
                if deny {
                    command.arg("--deny-warnings");
                }
                if json {
                    command.args(["--format", "json"]);
                }
                command.output().unwrap()
            };
            let text = run(false);
            let json = run(true);
            let expected_exit = if deny { 1 } else { exit };
            assert_eq!(text.status.code(), Some(expected_exit), "{text:?}");
            assert_eq!(json.status.code(), text.status.code(), "{json:?}");
            assert!(text.stderr.is_empty() && json.stderr.is_empty());
            assert!(
                String::from_utf8_lossy(&text.stdout).contains(expected),
                "{text:?}"
            );
            let report: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
            assert_eq!(
                report["summary"],
                if missing {
                    serde_json::json!({"categories": 0, "errors": 1, "warnings": 0})
                } else {
                    serde_json::json!({"categories": 1, "errors": 0, "warnings": 1})
                }
            );
        }
    }
    assert!(!fixture.0.join("state").exists());
}

fn sys_output_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.0 = fs::canonicalize(&fixture.0).unwrap();
    let state = fixture.0.join("state");
    for os in ["macos", "ubuntu", "windows"] {
        let preset = state.join("presets/sys").join(os);
        fs::create_dir_all(&preset).unwrap();
        let provider = if os == "windows" {
            "winget"
        } else {
            "homebrew"
        };
        fs::write(preset.join("shine.toml"), format!(r#"version = 2
[[items]]
id = "--help"
label = "Bootstrap"
permissions = {{ schema_version = 1 }}
detect = {{ kind = "command", command = "if", version_args = ["two words", "it's", "$HOME", "a;b", "`literal`", "中文"] }}
install = {{ kind = "package", provider = "{provider}", package = "tool" }}
[[items]]
id = "--version"
label = "Managed\n\r\t\u001b[2J中文é"
mode = "managed"
driver = "managed-file"
permissions = {{ schema_version = 1 }}
[items.config]
source = "managed.txt"
target = "~/.config/managed.txt"
[[items]]
id = "plain"
label = "Plain"
mode = "managed"
driver = "managed-file"
permissions = {{ schema_version = 1 }}
[items.config]
source = "managed.txt"
target = "~/.config/plain.txt"
"#)).unwrap();
        fs::write(preset.join("managed.txt"), "managed fixture\n").unwrap();
    }
    fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
    fs::write(state.join("update-check.json"), serde_json::to_vec(&serde_json::json!({
        "latest_version": env!("CARGO_PKG_VERSION"),
        "checked_at_unix_secs": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
    })).unwrap()).unwrap();
    fixture
}

#[test]
fn sys_output_hints_preserve_leading_hyphen_ids_through_shell_and_clap() {
    let fixture = sys_output_fixture();
    for (id, action) in [
        ("--help", "bootstrap"),
        ("--version", "apply"),
        ("plain", "apply"),
    ] {
        let info = fixture
            .command()
            .env("NO_COLOR", "1")
            .args(["sys", "info", "--", id])
            .output()
            .unwrap();
        assert_eq!(info.status.code(), Some(0), "{info:?}");
        let text = String::from_utf8(info.stdout).unwrap();
        let hint = text
            .split_once("Next: run `")
            .unwrap()
            .1
            .split('`')
            .next()
            .unwrap();
        let operand = if id.starts_with('-') {
            format!("-- {id}")
        } else {
            id.to_string()
        };
        assert_eq!(hint, format!("shine sys {action} {operand}"));
        let preview = hint.replace(
            &format!("sys {action} "),
            &format!("sys {action} --dry-run "),
        );
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
            command.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!(
                    "Set-Alias shine {}; {preview}; exit $LASTEXITCODE",
                    shine_core::shell_quote::powershell_single_quote(env!("CARGO_BIN_EXE_shine"))
                ),
            ]);
            command
        };
        let copied = fixture
            .configure(shell)
            .env("NO_COLOR", "1")
            .output()
            .unwrap();
        let direct = fixture
            .command()
            .env("NO_COLOR", "1")
            .args(["sys", action, "--dry-run", "--", id])
            .output()
            .unwrap();
        assert_eq!(direct.status.code(), Some(0), "{direct:?}");
        assert_eq!(copied.status.code(), direct.status.code(), "{copied:?}");
        assert_eq!(
            String::from_utf8_lossy(&copied.stdout)
                .lines()
                .collect::<Vec<_>>(),
            String::from_utf8_lossy(&direct.stdout)
                .lines()
                .collect::<Vec<_>>()
        );
        assert_eq!(copied.stderr, direct.stderr);
    }
    let listed = fixture.command().args(["sys", "list"]).output().unwrap();
    let text = String::from_utf8(listed.stdout).unwrap();
    assert!(text.contains("Run: shine sys apply -- --version"), "{text}");
    assert!(text.contains("Run: shine sys apply plain"), "{text}");
    assert!(!fixture.0.join("state/sys-manifest.toml").exists());
    assert!(!fixture.0.join("home/.config/managed.txt").exists());
}

#[test]
fn sys_output_labels_escape_controls_without_changing_receipts_or_approval() {
    let fixture = sys_output_fixture();
    let run = |args: &[&str]| {
        fixture
            .command()
            .env("NO_COLOR", "1")
            .args(args)
            .output()
            .unwrap()
    };
    let raw = "Managed\n\r\t\u{1b}[2J中文é";
    let displayed = r"Managed\n\r\t\u{1b}[2J中文é";
    for args in [
        vec!["sys", "list"],
        vec!["sys", "info", "--", "--version"],
        vec!["sys", "apply", "--dry-run", "--", "--version"],
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains(displayed), "{text}");
        assert!(
            !text.contains(raw) && !text.contains(['\r', '\t', '\u{1b}']),
            "{text:?}"
        );
    }
    let denied = run(&["sys", "apply", "--", "--version"]);
    assert_eq!(denied.status.code(), Some(1), "{denied:?}");
    assert!(String::from_utf8_lossy(&denied.stderr).contains("approval requires"));
    let receipt = fixture.0.join("state/sys-manifest.toml");
    assert!(!receipt.exists());
    let destination = fixture.0.join("home/.config/managed.txt");
    assert!(!destination.exists());
    let applied = run(&["sys", "apply", "--yes", "--", "--version"]);
    assert_eq!(applied.status.code(), Some(0), "{applied:?}");
    let text = String::from_utf8(applied.stdout).unwrap();
    assert!(text.contains(displayed) && !text.contains(raw), "{text}");
    assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
    assert_eq!(fs::read(&destination).unwrap(), b"managed fixture\n");
    let before = fs::read(&receipt).unwrap();
    let manifest: shine_core::runtime::SysRunManifest =
        toml::from_str(std::str::from_utf8(&before).unwrap()).unwrap();
    assert_eq!(manifest.entries[0].label, raw);
    for args in [
        vec!["sys", "status"],
        vec!["update", "--verbose"],
        vec!["sys", "info", "--", "--version"],
        vec!["sys", "uninstall", "--dry-run", "--", "--version"],
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(0), "{args:?}: {output:?}");
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains(displayed) && !text.contains(raw), "{text}");
        assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
        if args.contains(&"info") {
            assert!(text.contains("`shine sys apply -- --version`"), "{text}");
            assert!(
                text.contains("`shine sys uninstall -- --version`"),
                "{text}"
            );
        }
        assert_eq!(fs::read(&receipt).unwrap(), before);
        assert_eq!(fs::read(&destination).unwrap(), b"managed fixture\n");
    }
}

#[test]
fn sys_output_detection_preserves_literal_argv_in_the_native_shell() {
    let fixture = sys_output_fixture();
    let output = fixture
        .command()
        .env("NO_COLOR", "1")
        .args(["sys", "info", "--", "--help"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    let detection = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("Detection"))
        .unwrap()
        .trim();
    #[cfg(unix)]
    let shell = {
        use std::os::unix::fs::PermissionsExt;
        let bin = fixture.0.join("bin");
        fs::create_dir(&bin).unwrap();
        let program = bin.join("if");
        fs::write(&program, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        let mut command = Command::new("/bin/sh");
        command.env("PATH", bin).args(["-c", detection]);
        command
    };
    #[cfg(windows)]
    let shell = {
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", &format!("[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); Set-Item -Path Function:if -Value {{ $args | ForEach-Object {{ [Console]::WriteLine($_) }} }}; {detection}")]);
        command
    };
    let copied = fixture.configure(shell).output().unwrap();
    assert_eq!(copied.status.code(), Some(0), "{copied:?}");
    assert!(copied.stderr.is_empty(), "{copied:?}");
    assert_eq!(
        String::from_utf8(copied.stdout)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        ["two words", "it's", "$HOME", "a;b", "`literal`", "中文"]
    );
    assert!(!fixture.0.join("state/sys-manifest.toml").exists());
}

#[test]
fn lint_validation_hints_recheck_the_original_input_without_changing_json() {
    let fixture = Fixture::new();
    let good = fixture.0.join("app/good");
    fs::create_dir_all(&good).unwrap();
    // PowerShell expands Windows 8.3 cwd aliases; give both launches the same spelling,
    // including when the missing operand cannot itself be canonicalized.
    let good = fs::canonicalize(good).unwrap();
    let metadata =
        "metadata_schema_version = 2\ndest = '~/.config/good'\n[[files]]\nsource = 'config.txt'\n";
    fs::write(good.join("shine.toml"), metadata).unwrap();
    fs::write(good.join("config.txt"), "fixture\n").unwrap();
    for name in ["bad space", "it's"] {
        let bad = fixture.0.join("app").join(name);
        fs::create_dir_all(&bad).unwrap();
        fs::write(bad.join("shine.toml"), "[[files]\n").unwrap();
    }
    let default = fixture
        .command()
        .current_dir(&good)
        .args(["preset", "validate"])
        .output()
        .unwrap();
    assert_eq!(default.status.code(), Some(0), "{default:?}");
    for (path, missing) in [
        ("../bad space", false),
        ("../it's", false),
        ("-missing", true),
        ("../missing space", true),
    ] {
        let run = |format| {
            fixture
                .command()
                .current_dir(&good)
                .args(["preset", "lint", "--format", format, "--", path])
                .output()
                .unwrap()
        };
        let text = run("text");
        let json = run("json");
        for output in [&text, &json] {
            assert_eq!(output.status.code(), Some(1), "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
        }
        let text = String::from_utf8(text.stdout).unwrap();
        let hint = text
            .split_once("run `")
            .unwrap()
            .1
            .split('`')
            .next()
            .unwrap();
        #[cfg(unix)]
        let shell = {
            let mut command = Command::new("/bin/sh");
            command.args([
                "-c",
                &format!(
                    "shine() {{ {} \"$@\"; }}; {hint}",
                    shine_core::shell_quote::posix_single_quote(env!("CARGO_BIN_EXE_shine"))
                ),
            ]);
            command
        };
        #[cfg(windows)]
        let shell = {
            let mut command = Command::new("powershell.exe");
            command.args(["-NoProfile", "-NonInteractive", "-Command"]);
            command.arg(format!(
                "Set-Alias shine {}; {hint}; exit $LASTEXITCODE",
                shine_core::shell_quote::powershell_single_quote(env!("CARGO_BIN_EXE_shine"))
            ));
            command
        };
        let hinted = fixture
            .configure(shell)
            .current_dir(&good)
            .output()
            .unwrap();
        let explicit = fixture
            .command()
            .current_dir(&good)
            .args(["preset", "validate", "--", path])
            .output()
            .unwrap();
        assert_eq!(hinted.status.code(), Some(1), "{hinted:?}");
        // PowerShell may re-emit native stdout with CRLF line endings.
        assert_eq!(
            std::str::from_utf8(&hinted.stdout)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            std::str::from_utf8(&explicit.stdout)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            "{hint}"
        );
        assert_eq!(hinted.stderr, explicit.stderr, "{hint}");
        let report: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
        assert_eq!(report["valid"], false);
        assert_eq!(
            report["diagnostics"][0]["message"],
            if missing {
                "preset input could not be loaded; run `shine preset validate` for path details"
            } else {
                "static validation failed; run `shine preset validate` for details"
            }
        );
    }
    assert_eq!(
        fs::read_to_string(good.join("shine.toml")).unwrap(),
        metadata
    );
    assert!(!fixture.0.join("state").exists());
}

#[cfg(unix)]
#[test]
fn lint_non_utf8_path_does_not_suggest_a_different_valid_input() {
    use std::os::unix::ffi::OsStringExt;
    let fixture = Fixture::new();
    let original = PathBuf::from(std::ffi::OsString::from_vec(
        b"checkout-\xff/app/demo".to_vec(),
    ));
    let replacement = fixture.0.join("checkout-\u{fffd}/app/demo");
    fs::create_dir_all(&replacement).unwrap();
    let metadata =
        "metadata_schema_version = 2\ndest = '~/.config/demo'\n[[files]]\nsource = 'config.txt'\n";
    fs::write(replacement.join("shine.toml"), metadata).unwrap();
    fs::write(replacement.join("config.txt"), "fixture\n").unwrap();
    // The lossy spelling is a distinct, valid input. Never suggest validating it.
    let valid = fixture
        .command()
        .args(["preset", "validate"])
        .arg(&replacement)
        .output()
        .unwrap();
    assert_eq!(valid.status.code(), Some(0), "{valid:?}");
    let run = |format| {
        fixture
            .command()
            .args(["preset", "lint", "--format", format, "--"])
            .arg(&original)
            .output()
            .unwrap()
    };
    let text = run("text");
    let json = run("json");
    for output in [&text, &json] {
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
    }
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(
        text.contains(
            "preset validate with the original PATH (non-UTF-8; copyable command omitted)"
        ),
        "{text}"
    );
    assert!(!text.contains("`shine preset validate"), "{text}");
    assert!(!text.contains('\u{fffd}'), "{text}");
    let report: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(report["valid"], false);
    assert_eq!(report["diagnostics"][0]["code"], "invalid_input");
    assert_eq!(
        report["diagnostics"][0]["message"],
        "preset input could not be loaded; run `shine preset validate` for path details"
    );
    assert_eq!(
        fs::read_to_string(replacement.join("shine.toml")).unwrap(),
        metadata
    );
    assert_eq!(
        fs::read(replacement.join("config.txt")).unwrap(),
        b"fixture\n"
    );
    assert!(!fixture.0.join("state").exists());
}

#[test]
fn broker_policy_list_escapes_controls_without_changing_stored_values() {
    for (project, ssh_target, row) in [
        (
            "API's \"build\" \\ path",
            "example.invalid",
            "review: example.invalid (API's \"build\" \\ path)\n",
        ),
        (
            "api)\nforged: host (injected\r\t\u{1b}[31m",
            "example.invalid\talias",
            r"review: example.invalid\talias (api)\nforged: host (injected\r\t\u{1b}[31m)",
        ),
    ] {
        let fixture = Fixture::new();
        let workspace = fixture.0.join("shine.workspace.toml");
        fs::write(&workspace, "version = 2\n[env]\nfiles = ['env.toml']\n").unwrap();
        let source = "version = 1\n[secret]\nTOKEN = true\n[payload]\ndata = 'not-decrypted-by-policy-commands'\n";
        fs::write(fixture.0.join("env.toml"), source).unwrap();
        let added = fixture
            .command()
            .args([
                "env",
                "broker",
                "policy",
                "add",
                "--name",
                "review",
                "--project",
                project,
                "--ssh-target",
                ssh_target,
                "--workspace",
            ])
            .arg(&workspace)
            .args([
                "--mode",
                "dev",
                "--release",
                "TOKEN",
                "--",
                "echo",
                "never executed",
            ])
            .output()
            .unwrap();
        assert_eq!(added.status.code(), Some(0), "{added:?}");
        assert!(added.stderr.is_empty(), "{added:?}");
        let policy = fixture.0.join("state/ssh-secret-broker.toml");
        let before = fs::read(&policy).unwrap();
        let listed = fixture
            .command()
            .args(["env", "broker", "policy", "list"])
            .output()
            .unwrap();
        assert_eq!(listed.status.code(), Some(0), "{listed:?}");
        assert!(listed.stderr.is_empty(), "{listed:?}");
        let text = String::from_utf8(listed.stdout).unwrap();
        assert_eq!(text.trim_end_matches('\n'), row.trim_end_matches('\n'));
        assert_eq!(text.lines().count(), 1);
        assert!(!text.contains(['\r', '\t', '\u{1b}']));
        let info = fixture
            .command()
            .args(["env", "broker", "policy", "info", "review"])
            .output()
            .unwrap();
        assert_eq!(info.status.code(), Some(0), "{info:?}");
        assert!(info.stderr.is_empty(), "{info:?}");
        let decoded: toml::Value =
            toml::from_str(std::str::from_utf8(&info.stdout).unwrap()).unwrap();
        assert_eq!(decoded["project"].as_str(), Some(project));
        assert_eq!(decoded["ssh_target"].as_str(), Some(ssh_target));
        assert_eq!(fs::read(&policy).unwrap(), before);
        assert_eq!(
            fs::read_to_string(fixture.0.join("env.toml")).unwrap(),
            source
        );
    }
}

#[test]
fn preset_test_text_keeps_control_names_on_one_row_and_preserves_json_and_exit_status() {
    let fixture = Fixture::new();
    let category = fixture.0.join("app/demo");
    fs::create_dir_all(&category).unwrap();
    let metadata =
        "metadata_schema_version = 2\ndest = '~/.config/demo'\n[[files]]\nsource = 'config.txt'\n";
    fs::write(category.join("shine.toml"), metadata).unwrap();
    fs::write(category.join("config.txt"), "fixture\n").unwrap();
    for (name, passed) in [
        ("ordinary name", true),
        ("case\n  PASS forged\r\t\u{1b}[31m", false),
    ] {
        let source = format!(
            "schema_version = 1\n[[cases]]\nname = {}\nplatform = 'linux'\n[cases.expect]\nvalid = {passed}\n",
            toml::Value::String(name.into())
        );
        fs::write(category.join("shine.test.toml"), &source).unwrap();
        let run = |json| {
            let mut command = fixture.command();
            command
                .env("NO_COLOR", "1")
                .args(["preset", "test"])
                .arg(&category);
            if json {
                command.args(["--format", "json"]);
            }
            command.output().unwrap()
        };
        let text = run(false);
        let json = run(true);
        for output in [&text, &json] {
            assert_eq!(output.status.code(), Some(i32::from(!passed)), "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
        }
        let text = String::from_utf8(text.stdout).unwrap();
        assert_eq!(
            text.lines()
                .filter(|line| line.starts_with("  PASS ") || line.starts_with("  FAIL "))
                .count(),
            1,
            "{text}"
        );
        assert!(!text.contains(['\r', '\t', '\u{1b}']), "{text:?}");
        if passed {
            assert!(text.contains("PASS ordinary name (linux)"), "{text}");
            assert!(text.contains("Summary: 1 case, 1 passed, 0 failed"));
        } else {
            assert!(
                text.contains(r"FAIL case\n  PASS forged\r\t\u{1b}[31m"),
                "{text}"
            );
            assert!(text.contains("Summary: 1 case, 0 passed, 1 failed"));
        }
        let report: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
        assert_eq!(report["cases"].as_array().unwrap().len(), 1);
        assert_eq!(report["cases"][0]["name"], name);
        assert_eq!(report["cases"][0]["passed"], passed);
        assert_eq!(
            fs::read_to_string(category.join("shine.test.toml")).unwrap(),
            source
        );
        assert_eq!(
            fs::read_to_string(category.join("shine.toml")).unwrap(),
            metadata
        );
    }
    assert!(!fixture.0.join("state").exists());
}

#[test]
fn dotenv_import_force_reports_writes_and_preserves_preview_and_conflict_boundaries() {
    let fixture = Fixture::new();
    let input = fixture.0.join(".env");
    fs::write(&input, "PUBLIC=original\n").unwrap();
    let run = |flags: &[&str]| {
        fixture
            .command()
            .args(["env", "workspace", "init", "--from-dotenv"])
            .args(flags)
            .output()
            .unwrap()
    };
    let preview = run(&["--dry-run"]);
    assert!(preview.status.success(), "{preview:?}");
    assert_eq!(
        String::from_utf8(preview.stdout).unwrap(),
        "Would create .env.shine.toml\nWould create shine.workspace.toml\n"
    );
    let source = fixture.0.join(".env.shine.toml");
    let workspace = fixture.0.join("shine.workspace.toml");
    assert!(!source.exists() && !workspace.exists());
    let created = run(&[]);
    assert!(created.status.success(), "{created:?}");
    assert_eq!(
        String::from_utf8(created.stdout).unwrap(),
        "Created .env.shine.toml\nCreated shine.workspace.toml\n"
    );
    let original = [fs::read(&source).unwrap(), fs::read(&workspace).unwrap()];
    fs::write(&input, "PUBLIC=updated\n").unwrap();
    for flags in [&["--force", "--dry-run"][..], &[][..]] {
        let output = run(flags);
        if flags.is_empty() {
            assert_eq!(output.status.code(), Some(1), "{output:?}");
            assert!(String::from_utf8_lossy(&output.stderr).contains("already exists"));
        } else {
            assert!(output.status.success(), "{output:?}");
            assert_eq!(
                String::from_utf8(output.stdout).unwrap(),
                "Would write (replace if present) .env.shine.toml\nWould write (replace if present) shine.workspace.toml\n"
            );
        }
        assert_eq!(
            [fs::read(&source).unwrap(), fs::read(&workspace).unwrap()],
            original
        );
    }
    let written = run(&["--force"]);
    assert!(written.status.success(), "{written:?}");
    assert_eq!(
        String::from_utf8(written.stdout).unwrap(),
        "Wrote (replacing if present) .env.shine.toml\nWrote (replacing if present) shine.workspace.toml\n"
    );
    let imported: toml::Value = toml::from_str(&fs::read_to_string(&source).unwrap()).unwrap();
    assert_eq!(imported["plain"]["PUBLIC"].as_str(), Some("updated"));
    assert_eq!(fs::read(&workspace).unwrap(), original[1]);
    assert_eq!(fs::read_to_string(&input).unwrap(), "PUBLIC=updated\n");
    assert!(!fixture.0.join("state").exists());
}

#[tokio::test]
async fn shell_recovery_reports_rollback_and_committed_cleanup_without_reversing_committed_content()
{
    use shine_core::action::{
        ActionIrV1, DeclarativeActionV1, ShellFileIdentityV1, ShellLauncherReceiptV1,
        ShellReceiptTransitionV1, ShellRenderedFileReplacementSpecV1, managed_file_rollback_path,
    };
    use shine_core::install::hash_content;
    use shine_core::runtime::{FileSystemObservationHost, RealHost};

    for committed in [false, true] {
        let fixture = Fixture::new();
        let state = fixture.0.join("state");
        let presets = state.join("presets");
        let destination = state.join("rendered/demo.ts");
        let rollback = managed_file_rollback_path(&destination);
        fs::create_dir_all(&presets).unwrap();
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
        fs::write(&destination, b"new content\n").unwrap();
        fs::write(&rollback, b"old content\n").unwrap();
        let previous_mode = RealHost.metadata(&rollback).await.unwrap().unix_mode;
        let desired_mode = RealHost.metadata(&destination).await.unwrap().unix_mode;
        let previous = ShellLauncherReceiptV1 {
            launcher_format: None,
            launcher_config_dir: None,
            category: "demo".into(),
            command: "run".into(),
            mode: "snapshot".into(),
            source_path: presets.join("shell/demo/run.ts"),
            rendered_path: destination.clone(),
            runtime: "bun".into(),
            bun_dependencies: None,
            dependency_hash: None,
            transforms: vec!["template".into()],
            env: Vec::new(),
            needs_source: false,
            content_hash: hash_content(b"old source\n"),
        };
        let desired = ShellLauncherReceiptV1 {
            content_hash: hash_content(b"new source\n"),
            ..previous.clone()
        };
        let manifest = toml::to_string(&serde_json::json!({
            "schema_version": 1, "entries": [if committed { &desired } else { &previous }],
        }))
        .unwrap();
        fs::write(state.join("shell-manifest.toml"), &manifest).unwrap();
        let action_ir = ActionIrV1::new(
            "recovery-output-fixture",
            vec![DeclarativeActionV1::replace_shell_rendered_file(
                "rendered",
                "shell/demo/run",
                "rendered",
                ShellRenderedFileReplacementSpecV1 {
                    destination: destination.clone(),
                    previous: Some(ShellFileIdentityV1 {
                        content_hash: hash_content(b"old content\n"),
                        unix_mode: previous_mode,
                    }),
                    desired: ShellFileIdentityV1 {
                        content_hash: hash_content(b"new content\n"),
                        unix_mode: desired_mode,
                    },
                    receipts: vec![ShellReceiptTransitionV1 {
                        target: "shell/demo/run".into(),
                        previous: Some(Box::new(previous)),
                        desired: Box::new(desired),
                    }],
                },
            )],
        );
        action_ir.validate().unwrap();
        // The persisted approval is historical data; the CLI must review a fresh recovery Plan.
        let journal = toml::to_string(&serde_json::json!({
            "schema_version": 1, "action_ir": action_ir, "applied": ["rendered"],
            "receipt_committed": if committed { vec!["rendered"] } else { vec![] },
            "approval": { "schema_version": shine_core::plan::PLAN_APPROVAL_SCHEMA_VERSION,
                "plan_fingerprint": "0".repeat(64), "approved_permissions": [] },
        }))
        .unwrap();
        let journal_path = state.join("shell-operation-journal.toml");
        fs::write(&journal_path, &journal).unwrap();
        let run = |args: &[&str]| {
            fixture
                .command()
                .env("SHINE_PRESETS", &presets)
                .env("NO_COLOR", "1")
                .args(args)
                .output()
                .unwrap()
        };
        let denied = run(&["shell", "recover"]);
        assert!(!denied.status.success(), "{denied:?}");
        assert!(
            String::from_utf8_lossy(&denied.stderr).contains("approval requires"),
            "{denied:?}"
        );
        assert_eq!(fs::read_to_string(&journal_path).unwrap(), journal);
        assert_eq!(fs::read(&destination).unwrap(), b"new content\n");
        assert_eq!(fs::read(&rollback).unwrap(), b"old content\n");
        let output = run(&["shell", "recover", "--yes"]);
        assert!(output.status.success(), "{output:?}");
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(
            text.contains("recovered 1 Shell resource action(s), including rollback or cleanup"),
            "{output:?}"
        );
        assert!(!text.contains("rolled back"), "{output:?}");
        assert_eq!(
            fs::read(&destination).unwrap(),
            if committed {
                b"new content\n"
            } else {
                b"old content\n"
            }
        );
        assert!(!rollback.exists() && !journal_path.exists());
        assert_eq!(
            fs::read_to_string(state.join("shell-manifest.toml")).unwrap(),
            manifest
        );
        if committed {
            fs::write(&journal_path, &journal).unwrap();
            let output = run(&["shell", "recover", "--yes"]);
            assert!(output.status.success(), "{output:?}");
            assert!(
                String::from_utf8_lossy(&output.stdout)
                    .contains("no Shell resource action needed recovery"),
                "{output:?}"
            );
            assert_eq!(fs::read(&destination).unwrap(), b"new content\n");
            assert!(!journal_path.exists());
        }
    }
}

#[test]
fn schema_and_invalid_validation_do_not_initialize_state() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .args(["preset", "schema", "--format", "json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap();
    assert!(!fixture.0.join("state").exists());
    fs::create_dir_all(fixture.0.join("app/broken")).unwrap();
    fs::write(fixture.0.join("app/broken/shine.toml"), "invalid = [").unwrap();
    let output = fixture
        .command()
        .args(["preset", "validate", "app/broken", "--format", "json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap();
    assert!(!fixture.0.join("state").exists());
}

#[test]
fn schema_output_errors_use_the_normal_failure_exit_without_initializing_state() {
    for format in ["text", "json"] {
        let fixture = Fixture::new();
        let (reader, writer) = std::io::pipe().unwrap();
        drop(reader);
        let output = fixture
            .command()
            .args(["preset", "schema", "--format", format])
            .stdout(writer)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.starts_with("Error:"), "{stderr}");
        assert!(
            !stderr.contains("panicked") && !stderr.contains("RUST_BACKTRACE"),
            "{stderr}"
        );
        assert!(!fixture.0.join("state").exists());
    }
}

#[test]
fn app_mutation_requires_consent_and_preserves_modified_files() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let preset = state.join("presets/app/demo");
    let destination = fixture.0.join("destination");
    fs::create_dir_all(&preset).unwrap();
    fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
    let destination_toml =
        toml::Value::String(destination.to_string_lossy().into_owned()).to_string();
    fs::write(preset.join("shine.toml"), format!("metadata_schema_version = 2\ndest = {destination_toml}\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.txt'\n")).unwrap();
    fs::write(preset.join("config.txt"), "managed").unwrap();
    // An explicit external tree makes this fixture independent of embedded presets.
    let run = |args: &[&str]| {
        fixture
            .command()
            .env("SHINE_PRESETS", state.join("presets"))
            .args(args)
            .output()
            .unwrap()
    };
    let rejected = run(&["app", "install", "demo"]);
    assert!(!rejected.status.success(), "{rejected:?}");
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("approval requires"),
        "{rejected:?}"
    );
    assert!(!destination.exists());
    assert!(!state.join("app-manifest.toml").exists());
    let installed = run(&["app", "install", "demo", "--yes"]);
    assert!(installed.status.success(), "{installed:?}");
    assert_eq!(
        fs::read(destination.join("config.txt")).unwrap(),
        b"managed"
    );
    let receipt = fs::read(state.join("app-manifest.toml")).unwrap();
    fs::write(destination.join("config.txt"), "user edit").unwrap();
    let removed = run(&["app", "uninstall", "demo", "--yes"]);
    assert_eq!(removed.status.code(), Some(1), "{removed:?}");
    assert!(String::from_utf8_lossy(&removed.stdout).contains("Uninstall incomplete"));
    assert_eq!(fs::read(state.join("app-manifest.toml")).unwrap(), receipt);
    assert_eq!(
        fs::read(destination.join("config.txt")).unwrap(),
        b"user edit"
    );
}

#[test]
fn targeted_upgrade_keeps_category_scope_consent_and_summary_contracts() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let presets = state.join("presets");
    let destination = fixture.0.join("destination");
    fs::create_dir_all(&state).unwrap();
    fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
    for category in ["demo", "other"] {
        let preset = presets.join("app").join(category);
        fs::create_dir_all(&preset).unwrap();
        let dest = toml::Value::String(destination.join(category).to_string_lossy().into_owned())
            .to_string();
        fs::write(preset.join("shine.toml"), format!("metadata_schema_version = 2\ndest = {dest}\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'one.txt'\n[[files]]\nsource = 'two.txt'\n")).unwrap();
        for file in ["one.txt", "two.txt"] {
            fs::write(preset.join(file), "v1").unwrap();
        }
    }
    let run = |args: &[&str]| {
        fixture
            .command()
            .env("SHINE_PRESETS", &presets)
            .args(args)
            .output()
            .unwrap()
    };
    for category in ["demo", "other"] {
        let installed = run(&["app", "install", category, "--yes"]);
        assert!(installed.status.success(), "{installed:?}");
        for file in ["one.txt", "two.txt"] {
            fs::write(presets.join("app").join(category).join(file), "v2").unwrap();
        }
    }
    let receipt = fs::read(state.join("app-manifest.toml")).unwrap();
    let rejected = run(&["upgrade", "app/demo/one.txt"]);
    assert!(!rejected.status.success(), "{rejected:?}");
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("approval requires"),
        "{rejected:?}"
    );
    assert_eq!(fs::read(state.join("app-manifest.toml")).unwrap(), receipt);
    assert_eq!(fs::read(destination.join("demo/one.txt")).unwrap(), b"v1");

    // A file suffix intentionally still selects the entire category, not one file.
    let upgraded = run(&["upgrade", "app/demo/one.txt", "--yes"]);
    assert!(upgraded.status.success(), "{upgraded:?}");
    assert!(
        String::from_utf8_lossy(&upgraded.stdout).contains("1 updated"),
        "{upgraded:?}"
    );
    for file in ["one.txt", "two.txt"] {
        assert_eq!(
            fs::read(destination.join("demo").join(file)).unwrap(),
            b"v2"
        );
        assert_eq!(
            fs::read(destination.join("other").join(file)).unwrap(),
            b"v1"
        );
    }
    let receipt = fs::read(state.join("app-manifest.toml")).unwrap();
    let noop = run(&["upgrade", "app/demo", "--yes"]);
    assert!(noop.status.success(), "{noop:?}");
    assert!(
        String::from_utf8_lossy(&noop.stdout).contains("Nothing to upgrade."),
        "{noop:?}"
    );
    assert_eq!(fs::read(state.join("app-manifest.toml")).unwrap(), receipt);

    fs::write(destination.join("demo/one.txt"), "user edit").unwrap();
    fs::write(presets.join("app/demo/one.txt"), "v3").unwrap();
    let preserved = run(&["upgrade", "app/demo", "--yes"]);
    assert!(preserved.status.success(), "{preserved:?}");
    assert!(
        String::from_utf8_lossy(&preserved.stdout).contains("1 user-modified (kept)"),
        "{preserved:?}"
    );
    assert_eq!(
        fs::read(destination.join("demo/one.txt")).unwrap(),
        b"user edit"
    );
    assert_eq!(fs::read(state.join("app-manifest.toml")).unwrap(), receipt);

    for (args, error) in [
        (vec!["upgrade", "sys/", "--yes"], "invalid system target"),
        (
            vec!["upgrade", "sys/demo/extra", "--yes"],
            "invalid system target",
        ),
        (
            vec!["upgrade", "sys/demo", "--prune-stale", "--yes"],
            "applies only to app targets",
        ),
    ] {
        let invalid = run(&args);
        assert!(!invalid.status.success(), "{invalid:?}");
        assert!(
            String::from_utf8_lossy(&invalid.stderr).contains(error),
            "{invalid:?}"
        );
        assert_eq!(fs::read(state.join("app-manifest.toml")).unwrap(), receipt);
    }
}

#[test]
#[cfg(unix)]
fn targeted_upgrade_distinguishes_nonfatal_hook_and_generator_failures() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let presets = state.join("presets");
    let preset = presets.join("app/demo");
    let destination = fixture.0.join("destination");
    fs::create_dir_all(&preset).unwrap();
    fs::write(
        state.join("config.toml"),
        "schema_version = 2\n[env]\nENABLE = '1'\n",
    )
    .unwrap();
    let dest = toml::Value::String(destination.to_string_lossy().into_owned()).to_string();
    let metadata = format!(
        "metadata_schema_version = 2\ndest = {dest}\npost_install = {{ command = '/bin/sh', args = ['-c', 'exit 6'] }}\npost_upgrade = {{ command = '/bin/sh', args = ['-c', 'exit 7'] }}\n[permissions]\nschema_version = 1\ncommands = ['/bin/sh']\nfilesystem = [{{ access = ['execute'], base = 'preset', path = 'generate.sh' }}]\nenvironment = [{{ name = 'ENABLE', sensitivity = 'plain' }}]\n[[files]]\nsource = 'config.txt'\nrestart_hint = 'Restart demo'\ngenerator = {{ script = 'generate.sh', env = ['ENABLE'], when_env = 'ENABLE' }}\n"
    );
    fs::write(preset.join("shine.toml"), metadata).unwrap();
    fs::write(preset.join("config.txt"), "fallback").unwrap();
    let script = preset.join("generate.sh");
    let write_script = |body: &str| {
        use std::os::unix::fs::PermissionsExt;
        fs::write(&script, body).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    };
    let run = |args: &[&str]| {
        fixture
            .command()
            .env("SHINE_PRESETS", &presets)
            .args(args)
            .output()
            .unwrap()
    };
    write_script("#!/bin/sh\nprintf v1\n");
    let trusted = run(&["trust", "grant", "app/demo", "--yes"]);
    assert!(trusted.status.success(), "{trusted:?}");
    let installed = run(&["install", "app/demo", "--yes"]);
    assert!(installed.status.success(), "{installed:?}");
    assert!(
        String::from_utf8_lossy(&installed.stderr).contains("/bin/sh exited with 6"),
        "{installed:?}"
    );
    assert!(
        String::from_utf8_lossy(&installed.stdout).contains("1 installed"),
        "{installed:?}"
    );
    assert_eq!(fs::read(destination.join("config.txt")).unwrap(), b"v1");
    let initial_receipt = fs::read(state.join("app-manifest.toml")).unwrap();

    write_script("#!/bin/sh\nprintf v2\n");
    // Snapshot trust must be renewed for changed executable input.
    let untrusted = run(&["upgrade", "app/demo", "--yes"]);
    assert!(!untrusted.status.success(), "{untrusted:?}");
    assert!(
        String::from_utf8_lossy(&untrusted.stderr).contains("Plan is blocked"),
        "{untrusted:?}"
    );
    assert_eq!(fs::read(destination.join("config.txt")).unwrap(), b"v1");
    assert_eq!(
        fs::read(state.join("app-manifest.toml")).unwrap(),
        initial_receipt
    );
    let trusted = run(&["trust", "grant", "app/demo", "--yes"]);
    assert!(trusted.status.success(), "{trusted:?}");
    let upgraded = run(&["upgrade", "app/demo", "--yes"]);
    assert!(upgraded.status.success(), "{upgraded:?}");
    let stdout = String::from_utf8_lossy(&upgraded.stdout);
    assert!(stdout.contains("1 updated"), "{upgraded:?}");
    assert!(stdout.contains("Restart demo"), "{upgraded:?}");
    assert!(
        String::from_utf8_lossy(&upgraded.stderr).contains("/bin/sh exited with 7"),
        "{upgraded:?}"
    );
    assert_eq!(fs::read(destination.join("config.txt")).unwrap(), b"v2");
    let receipt = fs::read(state.join("app-manifest.toml")).unwrap();

    write_script("#!/bin/sh\nexit 9\n");
    let trusted = run(&["trust", "grant", "app/demo", "--yes"]);
    assert!(trusted.status.success(), "{trusted:?}");
    let failed = run(&["upgrade", "app/demo", "--yes"]);
    assert_eq!(failed.status.code(), Some(1), "{failed:?}");
    let text = String::from_utf8_lossy(&failed.stdout);
    assert!(
        text.contains("Upgrade incomplete") && text.contains("1 failed"),
        "{text}"
    );
    assert!(!text.contains("nothing changed"), "{text}");
    assert!(
        String::from_utf8_lossy(&failed.stderr).contains("1 managed configuration item(s) failed"),
        "{failed:?}"
    );
    assert_eq!(fs::read(destination.join("config.txt")).unwrap(), b"v2");
    assert_eq!(fs::read(state.join("app-manifest.toml")).unwrap(), receipt);
    // Install preserves the last good file on generator failure; unlike upgrade,
    // that existing policy is a warning, not a failed file result.
    let preserved = run(&["app", "install", "demo", "--yes"]);
    assert!(preserved.status.success(), "{preserved:?}");
    let stdout = String::from_utf8_lossy(&preserved.stdout);
    assert!(
        stdout.contains("1 skipped") && !stdout.contains("Install incomplete"),
        "{stdout}"
    );
    assert!(
        String::from_utf8_lossy(&preserved.stderr)
            .contains("generator unavailable; installed copy kept"),
        "{preserved:?}"
    );
    assert_eq!(fs::read(destination.join("config.txt")).unwrap(), b"v2");
    assert_eq!(fs::read(state.join("app-manifest.toml")).unwrap(), receipt);
}

#[test]
fn top_level_uninstall_uses_receipts_after_app_source_removal() {
    for target in ["app/retired", "retired"] {
        let fixture = Fixture::new();
        let state = fixture.0.join("state");
        let preset = state.join("presets/app/retired");
        let destination = fixture.0.join("destination");
        fs::create_dir_all(&preset).unwrap();
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join("config.txt"), "original").unwrap();
        fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
        let dest = toml::Value::String(destination.to_string_lossy().into_owned()).to_string();
        fs::write(preset.join("shine.toml"), format!("metadata_schema_version = 2\ndest = {dest}\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.txt'\n")).unwrap();
        fs::write(preset.join("config.txt"), "managed").unwrap();
        let run = |args: &[&str]| {
            fixture
                .command()
                .env("SHINE_PRESETS", state.join("presets"))
                .args(args)
                .output()
                .unwrap()
        };
        let installed = run(&["install", "app/retired", "--yes"]);
        assert!(installed.status.success(), "{installed:?}");
        fs::remove_dir_all(&preset).unwrap();
        let receipt = fs::read(state.join("app-manifest.toml")).unwrap();
        let preview = run(&["uninstall", target, "--dry-run"]);
        assert!(preview.status.success(), "{preview:?}");
        assert_eq!(
            fs::read(destination.join("config.txt")).unwrap(),
            b"managed"
        );
        assert_eq!(fs::read(state.join("app-manifest.toml")).unwrap(), receipt);
        let unapproved = run(&["uninstall", target]);
        assert!(!unapproved.status.success(), "{unapproved:?}");
        assert!(String::from_utf8_lossy(&unapproved.stderr).contains("approval requires"));
        fs::write(destination.join("config.txt"), "user edit").unwrap();
        let protected = run(&["uninstall", target, "--yes"]);
        assert!(!protected.status.success(), "{protected:?}");
        assert_eq!(
            fs::read(destination.join("config.txt")).unwrap(),
            b"user edit"
        );
        assert_eq!(fs::read(state.join("app-manifest.toml")).unwrap(), receipt);
        // A same-name Shell source makes the shorthand ambiguous even though
        // the App source is gone. Explicit App selection must still work.
        let shell = state.join("presets/shell/retired");
        fs::create_dir_all(&shell).unwrap();
        let script = if cfg!(windows) { "tool.ps1" } else { "tool.sh" };
        fs::write(shell.join("shine.toml"), format!("[[files]]\nsource = '{script}'\ntarget = 'tool'\n[files.permissions]\nschema_version = 1\n")).unwrap();
        fs::write(shell.join(script), "# fixture; never executed\n").unwrap();
        let ambiguous = run(&["uninstall", "retired", "--yes", "--force"]);
        assert!(!ambiguous.status.success(), "{ambiguous:?}");
        assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("ambiguous"));
        assert_eq!(fs::read(state.join("app-manifest.toml")).unwrap(), receipt);
        if target == "retired" {
            fs::remove_dir_all(&shell).unwrap();
        }
        let removed = run(&["uninstall", target, "--yes", "--force"]);
        assert!(removed.status.success(), "{removed:?}");
        assert_eq!(
            fs::read(destination.join("config.txt")).unwrap(),
            b"original"
        );
        let manifest: shine_core::install::AppManifest =
            toml::from_str(&fs::read_to_string(state.join("app-manifest.toml")).unwrap()).unwrap();
        assert!(manifest.entries.is_empty());
    }
}

#[cfg(windows)]
#[test]
fn proxy_executable_suffixes_reject_before_target_lookup_or_state_writes() {
    let fixture = Fixture::new();
    for name in [
        "missing.exe",
        "missing.EXE",
        "missing.com",
        "missing.bat",
        "missing.cmd",
        "missing.ps1",
    ] {
        let output = fixture
            .command()
            .args(["env", "proxy", "install", name, "--with", "TOKEN"])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("must omit executable extensions"),
            "{output:?}"
        );
        assert!(stderr.contains("use `missing`"), "{output:?}");
        assert!(!fixture.0.join("state").exists());
    }
}

#[cfg(unix)]
#[test]
fn app_install_reports_file_failures_without_hiding_partial_success() {
    use std::os::unix::fs::PermissionsExt;

    for partial in [false, true] {
        let fixture = Fixture::new();
        let state = fixture.0.join("state");
        let preset = state.join("presets/app/demo");
        let destination = fixture.0.join("destination");
        fs::create_dir_all(&preset).unwrap();
        fs::write(
            state.join("config.toml"),
            "schema_version = 2\n[env]\nENABLE = '1'\n",
        )
        .unwrap();
        let dest = toml::Value::String(destination.to_string_lossy().into_owned()).to_string();
        let extra = if partial {
            "[[files]]\nsource = 'good.txt'\n"
        } else {
            ""
        };
        fs::write(preset.join("shine.toml"), format!(
            "metadata_schema_version = 2\ndest = {dest}\n[permissions]\nschema_version = 1\nfilesystem = [{{ access = ['execute'], base = 'preset', path = 'generate.sh' }}]\nenvironment = [{{ name = 'ENABLE', sensitivity = 'plain' }}]\n[[files]]\nsource = 'config.json'\ninstall_mode = 'json-merge'\nmanaged_keys = ['demo']\ngenerator = {{ script = 'generate.sh', env = ['ENABLE'], when_env = 'ENABLE' }}\n{extra}"
        )).unwrap();
        fs::write(preset.join("config.json"), r#"{"demo":1}"#).unwrap();
        fs::write(preset.join("good.txt"), "good").unwrap();
        let script = preset.join("generate.sh");
        fs::write(&script, "#!/bin/sh\nprintf invalid-json\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let trusted = fixture
            .command()
            .args(["trust", "grant", "app/demo", "--yes"])
            .output()
            .unwrap();
        assert!(trusted.status.success(), "{trusted:?}");
        // Exercise both public spellings through normal approval and execution.
        let args = if partial {
            vec!["install", "app/demo", "--yes"]
        } else {
            vec!["app", "install", "demo", "--yes"]
        };
        let output = fixture
            .command()
            .env("NO_COLOR", "1")
            .args(args)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(
            stdout.contains("Install incomplete") && stdout.contains("1 failed"),
            "{stdout}"
        );
        assert!(
            !stdout.contains("Done") && !stdout.contains("nothing changed"),
            "{stdout}"
        );
        assert!(stderr.contains("demo/config.json: json-merge"), "{stderr}");
        assert!(
            !stderr.contains("Error:") && !stderr.contains("command incomplete;"),
            "{stderr}"
        );
        assert!(!destination.join("config.json").exists());
        let receipt = fs::read_to_string(state.join("app-manifest.toml")).unwrap();
        assert!(!receipt.contains("config.json"), "{receipt}");
        if partial {
            assert!(stdout.contains("1 installed"), "{stdout}");
            assert_eq!(fs::read(destination.join("good.txt")).unwrap(), b"good");
            assert!(receipt.contains("good.txt"), "{receipt}");
        }
    }
}

#[test]
fn app_install_preview_and_unchanged_install_keep_distinct_counts() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let preset = state.join("presets/app/demo");
    let destination = fixture.0.join("destination");
    fs::create_dir_all(&preset).unwrap();
    fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
    let dest = toml::Value::String(destination.to_string_lossy().into_owned()).to_string();
    let metadata = format!(
        "metadata_schema_version = 2\ndest = {dest}\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'one.txt'\n"
    );
    fs::write(preset.join("shine.toml"), &metadata).unwrap();
    fs::write(preset.join("one.txt"), "one").unwrap();
    let preview = fixture
        .command()
        .args(["app", "install", "demo", "--dry-run"])
        .output()
        .unwrap();
    assert!(preview.status.success(), "{preview:?}");
    let stdout = String::from_utf8_lossy(&preview.stdout);
    assert!(
        stdout.contains("1 file available") && stdout.contains("1 would install"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("skipped") && !stdout.contains("Done"),
        "{stdout}"
    );
    assert!(!destination.exists());
    assert!(!state.join("app-manifest.toml").exists());
    let installed = fixture
        .command()
        .args(["app", "install", "demo", "--yes"])
        .output()
        .unwrap();
    assert!(installed.status.success(), "{installed:?}");
    let receipt = fs::read(state.join("app-manifest.toml")).unwrap();
    let unchanged = fixture
        .command()
        .args(["app", "install", "demo", "--yes"])
        .output()
        .unwrap();
    assert!(unchanged.status.success(), "{unchanged:?}");
    let unchanged_stdout = String::from_utf8_lossy(&unchanged.stdout);
    assert!(
        unchanged_stdout.contains("1 skipped") && !unchanged_stdout.contains("would install"),
        "{unchanged_stdout}"
    );
    fs::write(
        preset.join("shine.toml"),
        format!("{metadata}[[files]]\nsource = 'two.txt'\n"),
    )
    .unwrap();
    fs::write(preset.join("two.txt"), "two").unwrap();
    let preview = fixture
        .command()
        .args(["app", "install", "demo", "--dry-run"])
        .output()
        .unwrap();
    assert!(preview.status.success(), "{preview:?}");
    let stdout = String::from_utf8_lossy(&preview.stdout);
    assert!(
        stdout.contains("2 files available")
            && !stdout.contains("skipped")
            && stdout.contains("2 would install"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Preview") && !stdout.contains("Done"),
        "{stdout}"
    );
    assert_eq!(fs::read(state.join("app-manifest.toml")).unwrap(), receipt);
    assert_eq!(fs::read(destination.join("one.txt")).unwrap(), b"one");
    assert!(!destination.join("two.txt").exists());
}

#[cfg(unix)]
#[test]
fn stderr_warning_color_follows_its_own_terminal_and_no_color() {
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd};

    for stderr_terminal in [false, true] {
        for no_color in [false, true] {
            let fixture = Fixture::new();
            let mut master = -1;
            let mut slave = -1;
            // SAFETY: openpty initializes both descriptors; defaults are sufficient.
            assert_eq!(
                unsafe {
                    libc::openpty(
                        &mut master,
                        &mut slave,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    )
                },
                0
            );
            for fd in [master, slave] {
                assert_eq!(
                    unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) },
                    0
                );
            }
            // SAFETY: each successful openpty descriptor is owned exactly once.
            let mut master = unsafe { fs::File::from_raw_fd(master) };
            let slave = unsafe { fs::File::from_raw_fd(slave) };
            // Keep unread macOS PTY data alive until after the child exits.
            let keep_alive = slave.try_clone().unwrap();
            let mut command = fixture.command();
            command
                .args(["env", "get", "SHINE_OUTPUT_TEST_MISSING"])
                .env("TERM", "xterm-256color")
                .env_remove("NO_COLOR")
                .env_remove("FORCE_COLOR")
                .env_remove("CLICOLOR_FORCE");
            if no_color {
                command.env("NO_COLOR", "1");
            }
            if stderr_terminal {
                command.stderr(Stdio::from(slave));
            } else {
                command.stdout(Stdio::from(slave));
            }
            let output = command.output().unwrap();
            drop(command);
            assert_eq!(output.status.code(), Some(1), "{output:?}");
            assert_eq!(
                unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) },
                0
            );
            let mut terminal = Vec::new();
            match master.read_to_end(&mut terminal) {
                Ok(_) => {}
                Err(error)
                    if error.raw_os_error() == Some(libc::EIO)
                        || error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => panic!("reading warning PTY: {error}"),
            }
            drop(keep_alive);
            let warning = if stderr_terminal {
                &terminal
            } else {
                &output.stderr
            };
            let warning = String::from_utf8_lossy(warning);
            assert!(
                warning.contains("SHINE_OUTPUT_TEST_MISSING is not set"),
                "{warning}"
            );
            assert_eq!(
                warning.contains("\x1b["),
                stderr_terminal && !no_color,
                "{warning:?}"
            );
        }
    }
}

#[test]
fn app_install_early_error_is_not_suppressed_as_already_reported() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let preset = state.join("presets/app/demo");
    fs::create_dir_all(&preset).unwrap();
    fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
    fs::write(preset.join("shine.toml"), "metadata_schema_version = 2\ndest = '~'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.txt'\n").unwrap();
    fs::write(preset.join("config.txt"), "managed").unwrap();
    let future_receipt = b"schema_version = 999\n";
    fs::write(state.join("app-manifest.toml"), future_receipt).unwrap();
    let output = fixture
        .command()
        .args(["app", "install", "demo", "--yes"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Error:") && stderr.contains("newer than this Shine supports"),
        "{stderr}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("Done") && !stdout.contains("Install incomplete"),
        "{stdout}"
    );
    assert_eq!(
        fs::read(state.join("app-manifest.toml")).unwrap(),
        future_receipt
    );
    assert!(!fixture.0.join("home/config.txt").exists());
}

#[test]
fn task_info_aligns_values_with_and_without_working_directory() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    fs::create_dir_all(&state).unwrap();
    fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
    let cwd = toml::Value::String(fixture.0.to_string_lossy().into_owned()).to_string();
    for fixed in [false, true] {
        let cwd_line = if fixed {
            format!("cwd = {cwd}\n")
        } else {
            String::new()
        };
        fs::write(
            state.join("tasks.toml"),
            format!("[tasks.build]\ncommand = ['cargo', 'build']\n{cwd_line}"),
        )
        .unwrap();
        let output = fixture
            .command()
            .env("NO_COLOR", "1")
            .args(["task", "info", "build"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let text = String::from_utf8(output.stdout).unwrap();
        let lines = text.lines().collect::<Vec<_>>();
        assert_eq!(lines[0], "Task        build");
        assert_eq!(lines[1], "Command     cargo build");
        assert_eq!(lines.len(), if fixed { 3 } else { 2 });
        if fixed {
            assert!(lines[2].starts_with("Working dir "), "{text}");
        }
    }
}

#[cfg(unix)]
#[test]
fn task_displays_execute_reserved_program_names_with_original_argv_and_exit_status() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let bin = fixture.0.join("bin");
    fs::create_dir_all(&state).unwrap();
    fs::create_dir_all(&bin).unwrap();
    fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
    // Save/list/info run version checks; keep this regression independent of GitHub releases.
    let update_cache = state.join("update-check.json");
    let cached_version = serde_json::to_vec(&serde_json::json!({
        "latest_version": env!("CARGO_PKG_VERSION"),
        "checked_at_unix_secs": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
    }))
    .unwrap();
    fs::write(&update_cache, &cached_version).unwrap();
    let mut paths = vec![bin.clone()];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let path = std::env::join_paths(paths).unwrap();
    let args = [
        "",
        "two words",
        "it's",
        "$HOME",
        "`literal`",
        "MODE=test",
        "if",
    ];
    let expected = format!("{}\0", args.join("\0")).into_bytes();
    for program in ["if", "then", "time", "MODE=test"] {
        let executable = bin.join(program);
        fs::write(&executable, "#!/bin/sh\nprintf '%s\\0' \"$@\"\nexit 7\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let saved = fixture
            .command()
            .env("NO_COLOR", "1")
            .args(["task", "save", "literal", "--force", "--", program])
            .args(args)
            .output()
            .unwrap();
        assert!(saved.status.success(), "{saved:?}");
        let saved_text = String::from_utf8(saved.stdout).unwrap();
        let display = saved_text.lines().nth(1).unwrap();
        let manifest = fs::read(state.join("tasks.toml")).unwrap();
        for (command, prefix) in [("info", "Command"), ("list", "literal")] {
            let mut query = fixture.command();
            query.env("NO_COLOR", "1").args(["task", command]);
            if command == "info" {
                query.arg("literal");
            }
            let output = query.output().unwrap();
            assert!(output.status.success(), "{output:?}");
            let text = String::from_utf8(output.stdout).unwrap();
            assert_eq!(
                text.lines()
                    .find_map(|line| line.strip_prefix(prefix))
                    .unwrap()
                    .trim(),
                display
            );
        }
        let direct = fixture
            .command()
            .env("NO_COLOR", "1")
            .env("PATH", &path)
            .args(["task", "run", "literal"])
            .output()
            .unwrap();
        assert_eq!(direct.status.code(), Some(7), "{direct:?}");
        assert_eq!(direct.stdout, expected);
        assert_eq!(
            String::from_utf8(direct.stderr).unwrap(),
            format!("Running literal: {display}\n")
        );
        let copied = Command::new("/bin/sh")
            .env("PATH", &path)
            .args(["-c", display])
            .output()
            .unwrap();
        assert_eq!(copied.status.code(), Some(7), "{copied:?}");
        assert_eq!(copied.stdout, expected);
        assert!(copied.stderr.is_empty(), "{copied:?}");
        assert_eq!(fs::read(state.join("tasks.toml")).unwrap(), manifest);
        assert_eq!(fs::read(&update_cache).unwrap(), cached_version);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn sys_profile_reports_attention_and_noop_without_changing_exit_policy() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let preset = state.join("presets/sys/macos");
    fs::create_dir_all(&preset).unwrap();
    fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
    fs::write(fixture.0.join("home/.tool-present"), "present").unwrap();
    fs::write(preset.join("shine.toml"), "version = 2\n[[items]]\nid = 'tool'\nlabel = 'Tool'\npermissions = { schema_version = 1 }\ndetect = { kind = 'path', path = '$HOME/.tool-present' }\ninstall = { kind = 'package', provider = 'homebrew', package = 'tool' }\n[[items.shell]]\nshells = ['zsh']\nphase = 'post'\npath = '$HOME/.tool/bin'\n").unwrap();
    let run = |args: &[&str]| {
        fixture
            .command()
            .env("SHELL", "/bin/zsh")
            .env("NO_COLOR", "1")
            .args(args)
            .output()
            .unwrap()
    };
    let denied = run(&["sys", "profile", "enable", "tool"]);
    assert!(!denied.status.success(), "{denied:?}");
    assert!(
        String::from_utf8_lossy(&denied.stderr).contains("approval requires"),
        "{denied:?}"
    );
    let profile = fixture.0.join("home/.shine/profile");
    assert!(!profile.exists());
    assert!(!state.join("sys-manifest.toml").exists());
    assert!(!fixture.0.join("home/.zshrc").exists());
    let preview = run(&["sys", "profile", "enable", "tool", "--dry-run"]);
    assert!(preview.status.success(), "{preview:?}");
    assert!(
        String::from_utf8_lossy(&preview.stdout).contains("[dry-run] sys/tool: would enable"),
        "{preview:?}"
    );
    assert!(!profile.exists());
    assert!(!state.join("sys-manifest.toml").exists());
    assert!(!fixture.0.join("home/.zshrc").exists());
    let enabled = run(&["sys", "profile", "enable", "tool", "--yes"]);
    assert!(enabled.status.success(), "{enabled:?}");
    assert!(
        String::from_utf8_lossy(&enabled.stdout).contains("shell integration enabled"),
        "{enabled:?}"
    );
    let again = run(&["sys", "profile", "enable", "tool", "--yes"]);
    assert!(again.status.success(), "{again:?}");
    assert!(
        String::from_utf8_lossy(&again.stdout).contains("shell integration already enabled"),
        "{again:?}"
    );
    // Make the active profile diverge from its merge base at the integration itself.
    let active = profile.join("macos-sys.post.sh");
    let original = fs::read_to_string(&active).unwrap();
    assert!(original.contains(".tool/bin"), "{original}");
    fs::write(&active, original.replace(".tool/bin", ".custom/bin")).unwrap();
    let user_edit = fs::read(&active).unwrap();
    let conflict = run(&["sys", "profile", "disable", "tool", "--yes"]);
    assert!(conflict.status.success(), "{conflict:?}");
    let text = String::from_utf8_lossy(&conflict.stdout);
    assert!(
        text.lines()
            .last()
            .unwrap()
            .starts_with("~ sys/tool: shell integration needs action")
            && !text.contains("shell integration disabled"),
        "{text}"
    );
    assert_eq!(fs::read(&active).unwrap(), user_edit);
    assert!(profile.join("macos-sys.post.new.sh").exists());
    // Missing merge ancestry also requires attention and must not replace user content.
    fs::remove_file(profile.join("macos-sys.post.base.sh")).unwrap();
    let missing_base = run(&["sys", "profile", "enable", "tool", "--yes"]);
    assert!(missing_base.status.success(), "{missing_base:?}");
    let text = String::from_utf8_lossy(&missing_base.stdout);
    assert!(
        text.lines()
            .last()
            .unwrap()
            .starts_with("~ sys/tool: shell integration needs action")
            && text.contains("no merge base"),
        "{text}"
    );
    assert_eq!(fs::read(&active).unwrap(), user_edit);
}

#[test]
fn theme_quiet_preserves_config_errors_and_keeps_stdout_clean() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    fs::create_dir_all(&state).unwrap();
    let malformed = "invalid = [";
    fs::write(state.join("config.toml"), malformed).unwrap();
    let output = fixture
        .command()
        .args(["theme", "sync", "--quiet"])
        .env("SHINE_TERMINAL_THEME", "dark")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("TOML parse error"),
        "{output:?}"
    );
    assert_eq!(
        fs::read_to_string(state.join("config.toml")).unwrap(),
        malformed
    );
    assert_eq!(fs::read_dir(&state).unwrap().count(), 1);
}

#[test]
fn theme_quiet_exports_to_stdout_without_initializing_state() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .args(["theme", "sync", "--quiet"])
        .env("SHINE_TERMINAL_THEME", "dark")
        .env("BAT_THEME", "user-choice")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("SHINE_TERMINAL_THEME"), "{output:?}");
    assert!(stdout.contains("dark"), "{output:?}");
    assert!(!stdout.contains("BAT_THEME"), "{output:?}");
    assert!(!fixture.0.join("state").exists());
}

#[test]
fn self_upgrade_preserves_error_chain_without_printing_credentials() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.0.join("state")).unwrap();
    fs::write(fixture.0.join("state/config.toml"), "schema_version = 2\n").unwrap();
    // Invalid header bytes fail before any HTTP request or binary installation.
    let output = fixture
        .command()
        .args(["self", "upgrade"])
        .env("GITHUB_TOKEN", "fixture-secret\ninvalid")
        .env_remove("GH_TOKEN")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    for message in [
        "Upgrade failed",
        "invalid GitHub token header",
        "failed to parse header value",
    ] {
        assert!(stderr.contains(message), "{output:?}");
    }
    assert!(!stderr.contains("fixture-secret"), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Checking for stable upgrades"),
        "{output:?}"
    );
    assert!(!stdout.contains("Upgraded shine"), "{output:?}");
    assert!(!stdout.contains("fixture-secret"), "{output:?}");
}

#[test]
fn preset_link_warning_matches_project_and_environment_source_selection() {
    for (project, env_presets, warns, selected) in [
        (false, None, true, "state-source"),
        (true, None, false, "linked-source"),
        (true, Some(""), false, "linked-source"),
        (true, Some("env"), true, "env-source"),
    ] {
        let fixture = Fixture::new();
        let state = fixture.0.join("state");
        let project_dir = fixture.0.join("project");
        let linked = fixture.0.join("linked");
        let env_source = fixture.0.join("env-source");
        fs::create_dir_all(&project_dir).unwrap();
        let script = if cfg!(windows) { "run.ps1" } else { "run.sh" };
        for (source, category) in [
            (state.join("presets"), "state-source"),
            (linked.clone(), "linked-source"),
            (env_source.clone(), "env-source"),
        ] {
            let category = source.join("shell").join(category);
            fs::create_dir_all(&category).unwrap();
            fs::write(
                category.join("shine.toml"),
                format!("[[files]]\nsource = '{script}'\n"),
            )
            .unwrap();
            fs::write(category.join(script), "# inspection only\n").unwrap();
        }
        let global_config = "schema_version = 2\n# retain global settings\ncustom = 'keep'\n";
        fs::write(state.join("config.toml"), global_config).unwrap();
        if project {
            fs::write(
                project_dir.join("shine.config.toml"),
                "# retain project settings\ncustom = 'keep'\npresets_dir = 'old-source'\n",
            )
            .unwrap();
        }
        let command = || {
            let mut command = fixture.command();
            if project {
                command.current_dir(&project_dir);
            }
            if let Some(value) = env_presets {
                command.env(
                    "SHINE_PRESETS",
                    if value.is_empty() {
                        PathBuf::new()
                    } else {
                        env_source.clone()
                    },
                );
            }
            command
        };
        let output = command()
            .args(["preset", "link", "--live"])
            .arg(&linked)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(stdout.contains("Warning:"), warns, "{output:?}");
        assert!(stdout.contains("Shell mode: live"), "{output:?}");
        let config_path = if project {
            project_dir.join("shine.config.toml")
        } else {
            state.join("config.toml")
        };
        let saved = fs::read(&config_path).unwrap();
        let config: toml::Table = toml::from_str(std::str::from_utf8(&saved).unwrap()).unwrap();
        assert_eq!(
            PathBuf::from(config["presets_dir"].as_str().unwrap()),
            fs::canonicalize(&linked).unwrap()
        );
        assert_eq!(config["external_shell_mode"].as_str(), Some("live"));
        assert_eq!(config["custom"].as_str(), Some("keep"));
        let list = command().args(["shell", "list"]).output().unwrap();
        assert_eq!(list.status.code(), Some(0), "{list:?}");
        assert!(list.stderr.is_empty(), "{list:?}");
        assert!(
            String::from_utf8_lossy(&list.stdout).contains(selected),
            "{list:?}"
        );
        assert_eq!(fs::read(config_path).unwrap(), saved);
        if project {
            assert_eq!(
                fs::read(state.join("config.toml")).unwrap(),
                global_config.as_bytes()
            );
        }
        for (source, category) in [
            (state.join("presets"), "state-source"),
            (linked, "linked-source"),
            (env_source, "env-source"),
        ] {
            assert_eq!(
                fs::read(source.join("shell").join(category).join(script)).unwrap(),
                b"# inspection only\n"
            );
        }
        assert!(!state.join("shell-manifest.toml").exists());
        assert!(!state.join("installed").exists());
        assert!(!state.join("trust.toml").exists());
    }
}

#[test]
fn preset_unlink_reports_saved_scope_without_promising_the_next_source() {
    for (project, config_env, presets_env, selected) in [
        (false, true, false, Some("state-source")),
        (true, true, true, Some("env-source")),
        (true, false, false, Some("global-source")),
        (false, false, false, None),
    ] {
        let fixture = Fixture::new();
        let state = fixture
            .0
            .join(if config_env { "state" } else { "home/.shine" });
        let project_dir = fixture.0.join("project");
        let global_source = fixture.0.join("global-source");
        let env_source = fixture.0.join("env-source");
        fs::create_dir_all(&project_dir).unwrap();
        let script = if cfg!(windows) { "run.ps1" } else { "run.sh" };
        for (source, category) in [
            (state.join("presets"), "state-source"),
            (global_source.clone(), "global-source"),
            (env_source.clone(), "env-source"),
        ] {
            let category = source.join("shell").join(category);
            fs::create_dir_all(&category).unwrap();
            fs::write(
                category.join("shine.toml"),
                format!("[[files]]\nsource = '{script}'\n"),
            )
            .unwrap();
            fs::write(category.join(script), "# inspection only\n").unwrap();
        }
        let global_config = format!(
            "schema_version = 2\n# retained\ncustom = 'keep'\nexternal_shell_mode = 'live'\npresets_dir = {}\n",
            toml::Value::String(global_source.to_string_lossy().into_owned())
        );
        fs::write(state.join("config.toml"), &global_config).unwrap();
        if project {
            fs::write(project_dir.join("shine.config.toml"), "# retained\ncustom = 'keep'\npresets_dir = 'local-source'\nexternal_shell_mode = 'live'\n").unwrap();
        }
        let command = || {
            let mut command = fixture.command();
            if project {
                command.current_dir(&project_dir);
            }
            if !config_env {
                command.env_remove("SHINE_CONFIG_DIR");
            }
            if presets_env {
                command.env("SHINE_PRESETS", &env_source);
            }
            command
        };
        let output = command().args(["preset", "unlink"]).output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("removed from the active config"),
            "{output:?}"
        );
        assert!(
            stdout.contains(
                "Environment overrides and inherited settings still apply on the next run."
            ),
            "{output:?}"
        );
        assert!(
            !stdout.contains("Built-in embedded presets will be used"),
            "{output:?}"
        );
        let config_path = if project {
            project_dir.join("shine.config.toml")
        } else {
            state.join("config.toml")
        };
        let saved = fs::read_to_string(&config_path).unwrap();
        let config: toml::Table = toml::from_str(&saved).unwrap();
        assert!(!config.contains_key("presets_dir"));
        assert_eq!(config["custom"].as_str(), Some("keep"));
        assert!(saved.contains("# retained"));
        assert_ne!(
            config
                .get("external_shell_mode")
                .and_then(toml::Value::as_str),
            Some("live")
        );
        let list = command().args(["shell", "list"]).output().unwrap();
        assert_eq!(list.status.code(), Some(0), "{list:?}");
        assert!(list.stderr.is_empty(), "{list:?}");
        let stdout = String::from_utf8_lossy(&list.stdout);
        if let Some(category) = selected {
            assert!(stdout.contains(category), "{list:?}");
        } else {
            assert!(stdout.contains("built-in"), "{list:?}");
            assert!(!stdout.contains("global-source"), "{list:?}");
        }
        assert_eq!(fs::read_to_string(config_path).unwrap(), saved);
        if project {
            assert_eq!(
                fs::read_to_string(state.join("config.toml")).unwrap(),
                global_config
            );
        }
        for (source, category) in [
            (state.join("presets"), "state-source"),
            (global_source, "global-source"),
            (env_source, "env-source"),
        ] {
            assert_eq!(
                fs::read(source.join("shell").join(category).join(script)).unwrap(),
                b"# inspection only\n"
            );
        }
        assert!(!state.join("shell-manifest.toml").exists());
        assert!(!state.join("installed").exists());
        assert!(!state.join("trust.toml").exists());
    }
}

#[test]
fn overlay_unlink_reports_inheritance_without_removing_global_or_source_files() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let global = fixture.0.join("global-overlay");
    let project = fixture.0.join("project");
    let local = project.join("local-overlay");
    for dir in [&state, &global, &local] {
        fs::create_dir_all(dir).unwrap();
    }
    fs::write(global.join("marker"), "global").unwrap();
    fs::write(local.join("marker"), "project").unwrap();
    let global_config = format!(
        "schema_version = 2\npresets_overlay_dir = {}\n",
        toml::Value::String(global.to_string_lossy().into_owned())
    );
    fs::write(state.join("config.toml"), &global_config).unwrap();
    fs::write(
        project.join("shine.config.toml"),
        "# retained\ncustom = 'keep'\npresets_overlay_dir = 'local-overlay'\n",
    )
    .unwrap();
    let output = fixture
        .command()
        .current_dir(&project)
        .args(["preset", "overlay", "unlink"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("removed from the active config"),
        "{output:?}"
    );
    assert!(
        stdout.contains("Global overlay settings may still be inherited"),
        "{output:?}"
    );
    assert!(
        !stdout.contains("without overlay on the next run"),
        "{output:?}"
    );
    assert_eq!(
        fs::read_to_string(state.join("config.toml")).unwrap(),
        global_config
    );
    let project_config = fs::read_to_string(project.join("shine.config.toml")).unwrap();
    assert!(project_config.contains("# retained"));
    assert!(project_config.contains("custom = 'keep'"));
    assert!(!project_config.contains("presets_overlay_dir"));
    let info = fixture
        .command()
        .current_dir(&project)
        .args(["preset", "overlay", "info"])
        .output()
        .unwrap();
    assert!(info.status.success(), "{info:?}");
    assert!(
        String::from_utf8_lossy(&info.stdout).contains("global-overlay"),
        "{info:?}"
    );
    assert_eq!(fs::read(global.join("marker")).unwrap(), b"global");
    assert_eq!(fs::read(local.join("marker")).unwrap(), b"project");
    let output = fixture
        .command()
        .args(["preset", "overlay", "unlink"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("base presets source"),
        "{output:?}"
    );
    assert_eq!(fs::read(global.join("marker")).unwrap(), b"global");
}

#[test]
fn preset_export_reports_overlay_sources_and_preserves_export_semantics() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    let overlay = fixture.0.join("overlay");
    fs::create_dir_all(state.join("presets")).unwrap();
    fs::create_dir_all(&overlay).unwrap();
    fs::write(overlay.join("overlay-only.txt"), "overlay").unwrap();
    fs::write(state.join("presets/external-only.txt"), "external").unwrap();
    for with_overlay in [false, true] {
        let mut config = "schema_version = 2\n".to_string();
        if with_overlay {
            config.push_str(&format!(
                "presets_overlay_dir = {}\n",
                toml::Value::String(overlay.to_string_lossy().into_owned())
            ));
        }
        fs::write(state.join("config.toml"), config).unwrap();
        let out = fixture
            .0
            .join(if with_overlay { "merged" } else { "builtin" });
        let output = fixture
            .command()
            .args(["preset", "export"])
            .arg(&out)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("Exporting built-in presets"), "{output:?}");
        assert_eq!(
            stdout.contains("with overlay overrides and additions"),
            with_overlay,
            "{output:?}"
        );
        assert_eq!(out.join("overlay-only.txt").exists(), with_overlay);
        assert!(out.join("shell/proxy/shine.toml").is_file());
        assert!(!out.join("external-only.txt").exists());
    }
}

#[test]
fn preset_activation_hints_use_templates_and_separate_escaped_paths() {
    let fixture = Fixture::new();
    // Apostrophes and shell metacharacters remain legal on Windows too.
    let destination = fixture.0.join("custom presets 'quoted' $NAME");
    fs::create_dir_all(&destination).unwrap();
    let output = fixture
        .command()
        .current_dir(&destination)
        .args(["preset", "copy", "shell/proxy"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("`shine preset overlay link <PATH>`"),
        "{output:?}"
    );
    let shown = stdout
        .lines()
        .find_map(|line| line.strip_prefix("  PATH (display-escaped): "))
        .unwrap();
    let shown: String = serde_json::from_str(shown).unwrap();
    assert_eq!(
        fs::canonicalize(shown).unwrap(),
        fs::canonicalize(&destination).unwrap()
    );
    assert!(!stdout.contains("Tip: run `"), "{output:?}");
    // Without the explicit config-dir override, export emits the activation hint.
    let output = fixture
        .command()
        .env_remove("SHINE_CONFIG_DIR")
        .args(["preset", "export"])
        .arg(&destination)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("`shine preset link <PATH>`"), "{output:?}");
    assert!(
        stdout.contains(&format!(
            "PATH (display-escaped): {:?}",
            destination.to_string_lossy()
        )),
        "{output:?}"
    );
    assert!(!stdout.contains("Tip: run `"), "{output:?}");
}

#[test]
fn task_run_help_describes_saved_and_caller_working_directories() {
    let fixture = Fixture::new();
    for args in [vec!["task", "--help"], vec!["task", "run", "--help"]] {
        let output = fixture.command().args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{args:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{args:?}: {output:?}");
        let help = String::from_utf8(output.stdout).unwrap();
        let help = help.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(help.contains("Run a saved task in its saved working directory, or the caller's directory if none was saved"), "{args:?}: {help}");
        assert!(
            !help.contains("Run a saved task in the current directory"),
            "{args:?}: {help}"
        );
    }
    assert!(!fixture.0.join("state").exists());
}

#[tokio::test]
async fn sys_status_reports_per_item_records_without_mutating_them() {
    let fixture = Fixture::new();
    let state = fixture.0.join("state");
    fs::create_dir_all(&state).unwrap();
    let os = cli::sys::detect_os_id().await.unwrap();
    let mut manifest = shine_core::runtime::SysRunManifest::default();
    for (index, (label, managed, status)) in [
        (
            "ASCII",
            false,
            shine_core::runtime::SysItemStatus::Installed,
        ),
        (
            "中文标签中文标签",
            true,
            shine_core::runtime::SysItemStatus::Updated,
        ),
        (
            "e\u{301}",
            false,
            shine_core::runtime::SysItemStatus::Failed,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        manifest.entries.push(shine_core::runtime::SysRunEntry {
            os_id: os.clone(),
            item_id: format!("item-{index}"),
            label: label.into(),
            status,
            detail: "recorded detail".into(),
            updated_at: index.to_string(),
            managed,
            profile_enabled: false,
            receipt: None,
        });
    }
    let mut foreign = manifest.entries[0].clone();
    foreign.os_id = "foreign-os".into();
    foreign.label = "Foreign item".into();
    manifest.entries.push(foreign.clone());
    // Foreign-only and genuinely empty records share the current-OS empty state.
    for entries in [manifest.entries.clone(), vec![foreign], vec![]] {
        manifest.entries = entries;
        let bytes = toml::to_string(&manifest).unwrap();
        let path = state.join("sys-manifest.toml");
        fs::write(&path, &bytes).unwrap();
        let output = fixture
            .command()
            .env("NO_COLOR", "1")
            .args(["sys", "status"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(!stdout.contains("Foreign item"), "{stdout}");
        assert!(!stdout.contains("bootstrap"), "{stdout}");
        if manifest.entries.len() > 1 {
            assert!(
                stdout.contains("Recorded System Operation Results"),
                "{stdout}"
            );
            assert!(
                stdout.contains(
                    "latest recorded results per item across system operations, not live checks"
                ),
                "{stdout}"
            );
            let rows: Vec<_> = stdout
                .lines()
                .filter(|line| line.contains("recorded detail"))
                .collect();
            assert_eq!(rows.len(), 3, "{stdout}");
            for (row, (label, status)) in rows.iter().zip([
                ("ASCII", "installed"),
                ("中文标签中文标签", "updated"),
                ("e\u{301}", "failed"),
            ]) {
                assert!(row.contains(label), "{row}");
                let (prefix, _) = row.split_once(status).unwrap();
                assert_eq!(console::measure_text_width(prefix), 21, "{row}");
            }
        } else {
            assert_eq!(
                stdout.trim(),
                format!(
                    "No system operation results recorded for {os}. Use `shine sys list` to view available items."
                )
            );
        }
        assert_eq!(fs::read_to_string(path).unwrap(), bytes);
    }
}

#[cfg(unix)]
#[test]
fn self_install_path_hint_preserves_literal_directory_and_existing_path() {
    let fixture = Fixture::new();
    let dir = fixture
        .0
        .join("space ' \" $HOME $(touch sentinel) `touch sentinel` \\ 中文");
    let dest = dir.join("shine");
    let output = fixture
        .command()
        .env("NO_COLOR", "1")
        .args(["self", "install", "--dest"])
        .arg(&dest)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert!(dest.is_file());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let command = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Add it to PATH, for example: "))
        .unwrap();
    let output = Command::new("/bin/sh")
        .current_dir(&fixture.0)
        .env("PATH", "/usr/bin:/bin")
        .arg("-c")
        .arg(format!("{command}; printf '%s' \"$PATH\""))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("{}:/usr/bin:/bin", dir.display())
    );
    assert!(!fixture.0.join("sentinel").exists());
    let config: toml::Value =
        toml::from_str(&fs::read_to_string(fixture.0.join("state/config.toml")).unwrap()).unwrap();
    assert_eq!(
        config["self_install_dest"].as_str(),
        Some(dest.to_str().unwrap())
    );
}

#[test]
fn preset_migration_artifact_hint_preserves_category_argument() {
    for (category, posix, powershell) in [
        ("demo", "demo", "demo"),
        ("demo space", "'demo space'", "'demo space'"),
        ("it's", "'it'\\''s'", "'it''s'"),
        ("literal$HOME", "'literal$HOME'", "'literal$HOME'"),
        #[cfg(unix)]
        ("double\"quote", "'double\"quote'", ""),
    ] {
        let fixture = Fixture::new();
        let root = fixture.0.join("presets");
        let app = root.join("app").join(category);
        fs::create_dir_all(&app).unwrap();
        let quoted = toml::Value::String(category.into()).to_string();
        let original = format!(
            "dest = '~/.demo'\npost_install = {{ command = 'shine', args = ['app', 'artifact', 'apply', {quoted}] }}\n[artifact]\nscript = 'build.ts'\n"
        );
        fs::write(app.join("shine.toml"), &original).unwrap();
        let script = "throw new Error('artifact must not run during migration');\n";
        fs::write(app.join("build.ts"), script).unwrap();
        let output = fixture
            .command()
            .env("NO_COLOR", "1")
            .args(["preset", "migrate"])
            .arg(&root)
            .arg("--dry-run")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let stdout = String::from_utf8(output.stdout).unwrap();
        let command = stdout
            .lines()
            .find_map(|line| line.strip_prefix("    Next: run `"))
            .and_then(|line| line.strip_suffix("` after relevant changes."))
            .unwrap();
        let expected = if cfg!(windows) { powershell } else { posix };
        assert_eq!(command, format!("shine app artifact apply {expected}"));
        #[cfg(unix)]
        {
            // Parse only: set positional arguments instead of executing the suggested operation.
            let parsed = Command::new("/bin/sh")
                .arg("-c")
                .arg(format!("set -- {command}; printf '%s\\0' \"$@\""))
                .env("HOME", fixture.0.join("home"))
                .output()
                .unwrap();
            assert!(parsed.status.success(), "{parsed:?}");
            assert!(parsed.stderr.is_empty(), "{parsed:?}");
            assert_eq!(
                parsed.stdout,
                format!("shine\0app\0artifact\0apply\0{category}\0").as_bytes()
            );
        }
        #[cfg(windows)]
        {
            // A local function captures arguments; no artifact command is invoked.
            let parsed = Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command"])
                .arg(format!(
                    "function shine {{ ConvertTo-Json -Compress -InputObject @($args) }}; {command}"
                ))
                .output()
                .unwrap();
            assert!(parsed.status.success(), "{parsed:?}");
            assert!(parsed.stderr.is_empty(), "{parsed:?}");
            let args: Vec<String> = serde_json::from_slice(&parsed.stdout).unwrap();
            assert_eq!(args, ["app", "artifact", "apply", category]);
        }
        assert_eq!(
            fs::read_to_string(app.join("shine.toml")).unwrap(),
            original
        );
        assert_eq!(fs::read_to_string(app.join("build.ts")).unwrap(), script);
        assert!(!fixture.0.join("state").exists());
    }
}

#[test]
fn dotenv_export_preview_counts_only_effective_non_secret_values() {
    let fixture = Fixture::new();
    let workspace = fixture.0.join("shine.workspace.toml");
    let base = fixture.0.join("base.toml");
    let overlay = fixture.0.join("production.toml");
    let workspace_text = "version = 2\n[env]\nfiles = ['base.toml', 'production.toml']\n";
    let base_text = "version = 1\n[plain]\nPUBLIC = 'public-value-sentinel'\nSHADOWED = 'shadowed-value-sentinel'\n[secret]\nTOKEN = 'secret-value-sentinel'\n";
    let overlay_text = "version = 1\n[secret]\nSHADOWED = true\n";
    fs::write(&workspace, workspace_text).unwrap();
    fs::write(&base, base_text).unwrap();
    fs::write(&overlay, overlay_text).unwrap();
    // Pending/unsealed secrets would fail if preview tried the secret export path.
    for include_secrets in [false, true] {
        for existing in [false, true] {
            let destination = fixture.0.join("export.env");
            if existing {
                fs::write(&destination, "existing-output-sentinel").unwrap();
            }
            let mut command = fixture.command();
            command
                .env("NO_COLOR", "1")
                .args([
                    "env",
                    "workspace",
                    "export",
                    "--format",
                    "dotenv",
                    "--mode",
                    "production",
                    "--dry-run",
                    "--output",
                ])
                .arg(&destination);
            if include_secrets {
                command.arg("--include-secrets");
            }
            if existing {
                command.arg("--force");
            }
            let output = command.output().unwrap();
            assert_eq!(output.status.code(), Some(0), "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
            let stdout = String::from_utf8(output.stdout).unwrap();
            let suffix = if include_secrets {
                "; --include-secrets will be applied during actual export."
            } else {
                "."
            };
            assert_eq!(
                stdout.trim(),
                format!(
                    "Would export 1 non-secret variable(s) for mode production to {:?}. Secrets were not decrypted or counted{suffix}",
                    destination.to_string_lossy()
                )
            );
            for sentinel in [
                "public-value-sentinel",
                "shadowed-value-sentinel",
                "secret-value-sentinel",
                "existing-output-sentinel",
            ] {
                assert!(!stdout.contains(sentinel), "{stdout}");
            }
            if existing {
                assert_eq!(
                    fs::read_to_string(&destination).unwrap(),
                    "existing-output-sentinel"
                );
                fs::remove_file(&destination).unwrap();
            } else {
                assert!(!destination.exists());
            }
            for (path, original) in [
                (&workspace, workspace_text),
                (&base, base_text),
                (&overlay, overlay_text),
            ] {
                assert_eq!(fs::read_to_string(path).unwrap(), original);
            }
        }
    }
}

#[test]
fn broker_duplicate_policy_hint_preserves_policy_and_update_approval() {
    let fixture = Fixture::new();
    let workspace = fixture.0.join("shine.workspace.toml");
    fs::write(&workspace, "version = 2\n[env]\nfiles = ['env.toml']\n").unwrap();
    fs::write(fixture.0.join("env.toml"), "version = 1\n[secret]\nTOKEN = true\n[payload]\ndata = 'not-decrypted-by-policy-commands'\n").unwrap();
    let run = |operation: &str, argument: &str| {
        fixture
            .command()
            .env("NO_COLOR", "1")
            .args([
                "env",
                "broker",
                "policy",
                operation,
                "--name",
                "review",
                "--ssh-target",
                "example.invalid",
                "--workspace",
            ])
            .arg(&workspace)
            .args([
                "--mode",
                "dev",
                "--release",
                "TOKEN",
                "--",
                "echo",
                argument,
            ])
            .output()
            .unwrap()
    };
    let added = run("add", "original");
    assert_eq!(added.status.code(), Some(0), "{added:?}");
    assert!(added.stderr.is_empty(), "{added:?}");
    assert!(
        String::from_utf8_lossy(&added.stdout).contains("added SSH secret broker policy review")
    );
    let policy = fixture.0.join("state/ssh-secret-broker.toml");
    let original = fs::read(&policy).unwrap();
    let duplicate = run("add", "changed");
    assert_eq!(duplicate.status.code(), Some(1), "{duplicate:?}");
    assert!(duplicate.stdout.is_empty(), "{duplicate:?}");
    let error = String::from_utf8_lossy(&duplicate.stderr);
    for fragment in [
        "policy review already exists",
        "shine env broker policy update --name review",
        "policy parameters supplied again",
        "--ssh-target",
        "--workspace",
        "--mode",
        "--release or --release-all-declared",
        "command after `--`",
        "--project",
        "--remote-workspace",
        "Review and confirmation are still required for changes",
        "shine env broker policy update --help",
    ] {
        assert!(error.contains(fragment), "{error}");
    }
    assert!(!error.contains("policy update review"), "{error}");
    assert_eq!(fs::read(&policy).unwrap(), original);
    let help = fixture
        .command()
        .args(["env", "broker", "policy", "update", "--help"])
        .output()
        .unwrap();
    assert_eq!(help.status.code(), Some(0), "{help:?}");
    assert!(help.stderr.is_empty(), "{help:?}");
    let help = String::from_utf8_lossy(&help.stdout);
    for option in [
        "--name",
        "--ssh-target",
        "--workspace",
        "--mode",
        "--release",
        "--release-all-declared",
        "--project",
        "--remote-workspace",
    ] {
        assert!(help.contains(option), "{help}");
    }
    // Supplying the corrected arguments does not approve a changed policy.
    let update = run("update", "changed");
    assert_eq!(update.status.code(), Some(1), "{update:?}");
    assert!(
        String::from_utf8_lossy(&update.stdout).contains("changed"),
        "{update:?}"
    );
    assert!(
        String::from_utf8_lossy(&update.stderr).contains("reading policy update confirmation"),
        "{update:?}"
    );
    assert_eq!(fs::read(&policy).unwrap(), original);
}

#[cfg(unix)]
#[test]
fn identity_hint_round_trips_toml_and_survives_changed_directory() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let bin = fixture.0.join("test-bin");
    fs::create_dir_all(&bin).unwrap();
    let keygen = bin.join("age-keygen");
    // Public placeholder only: no real keys, device interaction or external keygen.
    fs::write(&keygen, "#!/bin/sh\n[ \"$1\" = '-o' ] && [ \"$#\" = 2 ] || exit 91\nprintf '# recipient: age1reviewplaceholder\\n' > \"$2\"\nprintf 'called\\n' >> \"$SHINE_TEST_KEYGEN_LOG\"\n").unwrap();
    fs::set_permissions(&keygen, fs::Permissions::from_mode(0o755)).unwrap();
    let log = fixture.0.join("keygen.log");
    let relative = PathBuf::from("keys/space ' quote\" back\\slash $HOME/identity.txt");
    let expected = fs::canonicalize(&fixture.0).unwrap().join(&relative);
    let path = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    )))
    .unwrap();
    let output = fixture
        .command()
        .env("PATH", path)
        .env("SHINE_TEST_KEYGEN_LOG", &log)
        .env("NO_COLOR", "1")
        .args(["env", "secret", "identity", "init", "--output"])
        .arg(&relative)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let original = fs::read(&expected).unwrap();
    assert_eq!(original, b"# recipient: age1reviewplaceholder\n");
    assert_eq!(fs::read_to_string(&log).unwrap(), "called\n");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let assignment = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Set age_identity = "))
        .and_then(|line| line.strip_suffix(" in config.toml so shine can find this identity."))
        .unwrap();
    let snippet = format!("age_identity = {assignment}\n");
    let parsed: toml::Value = toml::from_str(&snippet).unwrap();
    let value = parsed["age_identity"].as_str().unwrap();
    assert!(std::path::Path::new(value).is_absolute());
    assert_eq!(std::path::Path::new(value), expected);
    let state = fixture.0.join("state");
    assert!(
        !state.join("config.toml").exists(),
        "init must not apply its config hint"
    );
    fs::create_dir_all(&state).unwrap();
    fs::write(
        state.join("config.toml"),
        format!("schema_version = 2\n{snippet}"),
    )
    .unwrap();
    let elsewhere = fixture.0.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    for cwd in [&fixture.0, &elsewhere] {
        let listed = fixture
            .command()
            .current_dir(cwd)
            .env("NO_COLOR", "1")
            .args(["env", "secret", "identity", "list"])
            .output()
            .unwrap();
        assert_eq!(listed.status.code(), Some(0), "{listed:?}");
        assert!(listed.stderr.is_empty(), "{listed:?}");
        assert_eq!(
            String::from_utf8(listed.stdout).unwrap(),
            format!("{}  age1reviewplaceholder\n", expected.to_string_lossy())
        );
    }
    assert_eq!(fs::read(expected).unwrap(), original);
    assert_eq!(fs::read_to_string(log).unwrap(), "called\n");
}

#[cfg(unix)]
#[test]
fn identity_hint_survives_working_directory_removed_by_keygen() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let work = fixture.0.join("work");
    let bin = fixture.0.join("bin");
    fs::create_dir_all(&work).unwrap();
    fs::create_dir_all(&bin).unwrap();
    let keygen = bin.join("age-keygen");
    // Keep the output outside the directory removed by this isolated substitute.
    fs::write(&keygen, "#!/bin/sh\n[ \"$1\" = '-o' ] && [ \"$2\" = '../identity.txt' ] && [ \"$#\" = 2 ] || exit 91\nprintf '# recipient: age1reviewplaceholder\\n' > \"$2\"\nprintf 'called\\n' >> \"$SHINE_TEST_KEYGEN_LOG\"\n/bin/rmdir \"$SHINE_TEST_CWD\"\n").unwrap();
    fs::set_permissions(&keygen, fs::Permissions::from_mode(0o755)).unwrap();
    let log = fixture.0.join("keygen.log");
    let output = fixture
        .command()
        .current_dir(&work)
        .env("PATH", &bin)
        .env("SHINE_TEST_CWD", &work)
        .env("SHINE_TEST_KEYGEN_LOG", &log)
        .env("NO_COLOR", "1")
        .args([
            "env",
            "secret",
            "identity",
            "init",
            "--output",
            "../identity.txt",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert!(!work.exists());
    assert_eq!(fs::read_to_string(&log).unwrap(), "called\n");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("generated age identity at ../identity.txt"),
        "{stdout}"
    );
    let value = stdout
        .lines()
        .find_map(|line| line.strip_prefix("Set age_identity = "))
        .and_then(|line| line.strip_suffix(" in config.toml so shine can find this identity."))
        .unwrap();
    let snippet = format!("age_identity = {value}\n");
    let parsed: toml::Value = toml::from_str(&snippet).unwrap();
    let hinted = std::path::Path::new(parsed["age_identity"].as_str().unwrap());
    assert!(hinted.is_absolute());
    assert_eq!(
        hinted,
        fs::canonicalize(fixture.0.join("identity.txt")).unwrap()
    );
    assert_eq!(
        fs::read(hinted).unwrap(),
        b"# recipient: age1reviewplaceholder\n"
    );
    let state = fixture.0.join("state");
    assert!(!state.join("config.toml").exists());
    fs::create_dir_all(&state).unwrap();
    fs::write(
        state.join("config.toml"),
        format!("schema_version = 2\n{snippet}"),
    )
    .unwrap();
    let listed = fixture
        .command()
        .args(["env", "secret", "identity", "list"])
        .output()
        .unwrap();
    assert_eq!(listed.status.code(), Some(0), "{listed:?}");
    assert!(listed.stderr.is_empty(), "{listed:?}");
    assert!(String::from_utf8_lossy(&listed.stdout).contains("age1reviewplaceholder"));
    assert_eq!(fs::read_to_string(log).unwrap(), "called\n");
}

#[test]
fn sys_status_help_describes_recorded_current_os_results() {
    let fixture = Fixture::new();
    for args in [vec!["sys", "--help"], vec!["sys", "status", "--help"]] {
        let output = fixture.command().args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{args:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{args:?}: {output:?}");
        let help = String::from_utf8(output.stdout).unwrap();
        let help = help.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            help.contains("Show recorded results per item for the current OS, not live checks"),
            "{args:?}: {help}"
        );
        assert!(
            !help.contains("Show system bootstrap items previously initialized by shine"),
            "{args:?}: {help}"
        );
        assert!(
            !fixture.0.join("state").exists(),
            "{args:?} initialized state"
        );
    }
}

#[test]
fn uninstall_help_describes_default_cleanup_and_rejects_removed_purge() {
    let fixture = Fixture::new();
    for (args, expected) in [
        (
            vec!["uninstall"],
            "App targets remove the entire selected category cache, including extra files",
        ),
        (
            vec!["app", "uninstall"],
            "Without CATEGORY, remove the entire internal App cache tree; remove the App manifest only when no installation records remain",
        ),
        (
            vec!["shell", "uninstall"],
            "With TARGET, retain extra cache files and shared files still needed by installed sibling commands",
        ),
    ] {
        for flag in ["--help", "-h"] {
            let output = fixture.command().args(&args).arg(flag).output().unwrap();
            assert_eq!(output.status.code(), Some(0), "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
            let help = String::from_utf8(output.stdout).unwrap();
            let help = help.split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(help.contains("Internal cleanup is included"), "{help}");
            assert!(help.contains(expected), "{help}");
            assert!(help.contains("External preset sources"), "{help}");
            assert!(!help.contains("--purge"), "{help}");
        }
        let output = fixture
            .command()
            .args(&args)
            .args(["demo", "--purge"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("unexpected argument '--purge'"));
        assert!(output.stdout.is_empty());
        assert!(!fixture.0.join("state").exists());
    }
}

#[test]
fn app_uninstall_defaults_to_reviewed_internal_cleanup() {
    use shine_core::install::{AppEntry, AppInstallStrategy, AppManifest};
    for args in [
        vec!["uninstall", "app/git"],
        vec!["app", "uninstall", "git"],
        vec!["app", "uninstall"],
    ] {
        for external in [false, true] {
            let fixture = Fixture::new();
            // A project config without presets_dir selects embedded presets while keeping
            // SHINE_CONFIG_DIR isolated; SHINE_PRESETS below selects the external variant.
            fs::write(fixture.0.join("shine.config.toml"), "schema_version = 2\n").unwrap();
            let state = fixture.0.join("state");
            let cache = state.join("presets/app/git");
            let sibling = state.join("presets/app/sibling");
            fs::create_dir_all(&cache).unwrap();
            fs::create_dir_all(&sibling).unwrap();
            fs::write(cache.join("extra.txt"), "extra cache").unwrap();
            fs::write(sibling.join("extra.txt"), "sibling cache").unwrap();
            fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
            let manifest = AppManifest {
                entries: vec![AppEntry {
                    source: "app/git/gitconfig".to_string(),
                    destination: fixture.0.join("missing-config"),
                    backup: None,
                    content_hash: 1,
                    install_strategy: AppInstallStrategy::Copy,
                    uses_env: false,
                    requires_admin: false,
                }],
                ..AppManifest::default()
            };
            let receipt = toml::to_string(&manifest).unwrap();
            fs::write(state.join("app-manifest.toml"), &receipt).unwrap();
            let run = |flags: &[&str]| {
                let mut command = fixture.command();
                if external {
                    command.env("SHINE_PRESETS", state.join("presets"));
                }
                command.args(&args).args(flags).output().unwrap()
            };
            let preview = run(&["--dry-run"]);
            assert_eq!(
                preview.status.code(),
                Some(0),
                "{args:?} external={external}: {preview:?}"
            );
            let preview_text = String::from_utf8_lossy(&preview.stdout);
            assert!(!preview_text.contains("purged"), "{preview_text}");
            assert!(
                !preview_text.contains("preset cache removed"),
                "{preview_text}"
            );
            assert_eq!(
                preview_text.contains("would remove app"),
                !external,
                "{preview_text}"
            );
            assert_eq!(
                fs::read_to_string(state.join("app-manifest.toml")).unwrap(),
                receipt
            );
            assert_eq!(fs::read(cache.join("extra.txt")).unwrap(), b"extra cache");
            let denied = run(&[]);
            assert_eq!(denied.status.code(), Some(1), "{denied:?}");
            assert!(String::from_utf8_lossy(&denied.stderr).contains("approval requires"));
            assert_eq!(fs::read(cache.join("extra.txt")).unwrap(), b"extra cache");
            assert_eq!(
                fs::read_to_string(state.join("app-manifest.toml")).unwrap(),
                receipt
            );
            let removed = run(&["--yes"]);
            assert_eq!(
                removed.status.code(),
                Some(0),
                "{args:?} external={external}: {removed:?}"
            );
            let whole = args.len() == 2 && args[0] == "app";
            let removed_text = String::from_utf8_lossy(&removed.stdout);
            assert!(!removed_text.contains("would remove"), "{removed_text}");
            assert_eq!(
                removed_text.contains("preset cache removed"),
                !external,
                "{removed_text}"
            );
            assert_eq!(
                removed_text.contains("empty App manifest removed"),
                !external && whole,
                "{removed_text}"
            );
            if external {
                assert_eq!(fs::read(cache.join("extra.txt")).unwrap(), b"extra cache");
            } else {
                assert!(!cache.exists());
            }
            if external || !whole {
                assert_eq!(
                    fs::read(sibling.join("extra.txt")).unwrap(),
                    b"sibling cache"
                );
                let remaining: AppManifest =
                    toml::from_str(&fs::read_to_string(state.join("app-manifest.toml")).unwrap())
                        .unwrap();
                assert!(remaining.entries.is_empty());
            } else {
                assert!(!state.join("presets/app").exists());
                assert!(!state.join("app-manifest.toml").exists());
            }
            assert_eq!(
                fs::read(state.join("config.toml")).unwrap(),
                b"schema_version = 2\n"
            );
        }
    }
}

#[test]
fn whole_app_cleanup_keeps_protected_receipts_for_forced_retry() {
    use shine_core::install::{AppEntry, AppInstallStrategy, AppManifest, hash_content};
    for mixed in [false, true] {
        let fixture = Fixture::new();
        let state = fixture.0.join("state");
        let cache = state.join("presets/app/git");
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join("extra.txt"), "extra cache").unwrap();
        fs::write(fixture.0.join("shine.config.toml"), "schema_version = 2\n").unwrap();
        fs::write(state.join("config.toml"), "schema_version = 2\n").unwrap();
        let protected = fixture.0.join("protected");
        let backup = fixture.0.join("protected.shine.bak");
        let normal = fixture.0.join("normal");
        fs::write(&protected, "user edit").unwrap();
        fs::write(&backup, "original").unwrap();
        let entry = AppEntry {
            source: "app/git/gitconfig".into(),
            destination: protected.clone(),
            backup: Some(backup.clone()),
            content_hash: hash_content(b"installed"),
            install_strategy: AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        };
        let mut manifest = AppManifest {
            entries: vec![entry.clone()],
            ..AppManifest::default()
        };
        if mixed {
            fs::write(&normal, "installed").unwrap();
            manifest.entries.push(AppEntry {
                source: "app/git/normal".into(),
                destination: normal.clone(),
                backup: None,
                ..entry.clone()
            });
        }
        let manifest_path = state.join("app-manifest.toml");
        let original_manifest = toml::to_string(&manifest).unwrap();
        fs::write(&manifest_path, &original_manifest).unwrap();
        let run = |flags: &[&str]| {
            fixture
                .command()
                .args(["app", "uninstall"])
                .args(flags)
                .output()
                .unwrap()
        };
        let preview = run(&["--dry-run"]);
        assert_eq!(preview.status.code(), Some(0), "{preview:?}");
        let text = String::from_utf8_lossy(&preview.stdout);
        assert!(text.contains("would remove app preset cache"), "{text}");
        assert!(!text.contains("empty App manifest"), "{text}");
        assert!(!text.contains("purged"), "{text}");
        assert_eq!(
            fs::read_to_string(&manifest_path).unwrap(),
            original_manifest
        );
        let removed = run(&["--yes"]);
        assert_eq!(removed.status.code(), Some(1), "{removed:?}");
        assert!(removed.stderr.is_empty(), "{removed:?}");
        let text = String::from_utf8_lossy(&removed.stdout);
        assert!(text.contains("Uninstall incomplete"), "{text}");
        assert!(text.contains("installation receipts retained"), "{text}");
        assert!(text.contains("app preset cache removed"), "{text}");
        assert!(!text.contains("empty App manifest removed"), "{text}");
        let remaining: AppManifest =
            toml::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
        assert_eq!(remaining.entries, vec![entry]);
        assert_eq!(fs::read(&protected).unwrap(), b"user edit");
        assert_eq!(fs::read(&backup).unwrap(), b"original");
        assert!(!normal.exists());
        assert!(!state.join("presets/app").exists());
        let repeated = run(&["--yes"]);
        assert_eq!(repeated.status.code(), Some(1), "{repeated:?}");
        assert!(!String::from_utf8_lossy(&repeated.stdout).contains("preset cache removed"));
        let forced_preview = run(&["--force", "--dry-run"]);
        assert_eq!(forced_preview.status.code(), Some(0), "{forced_preview:?}");
        assert!(
            String::from_utf8_lossy(&forced_preview.stdout)
                .contains("would remove empty App manifest")
        );
        assert!(manifest_path.exists());
        assert_eq!(fs::read(&protected).unwrap(), b"user edit");
        let forced = run(&["--force", "--yes"]);
        assert_eq!(forced.status.code(), Some(0), "{forced:?}");
        assert!(String::from_utf8_lossy(&forced.stdout).contains("empty App manifest removed"));
        assert_eq!(fs::read(&protected).unwrap(), b"original");
        assert!(!backup.exists());
        assert!(!manifest_path.exists());
    }
}

#[test]
fn shell_category_then_whole_uninstall_cleans_remaining_profile_with_approval() {
    fn config_files(dir: &std::path::Path, files: &mut Vec<PathBuf>) {
        for item in fs::read_dir(dir).unwrap() {
            let item = item.unwrap();
            if item.file_type().unwrap().is_dir() {
                config_files(&item.path(), files);
            } else if fs::read_to_string(item.path())
                .is_ok_and(|text| text.contains("# >>> shine >>>"))
            {
                files.push(item.path());
            }
        }
    }
    for category_args in [
        vec!["shell", "uninstall", "proxy"],
        vec!["uninstall", "shell/proxy"],
    ] {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("shine.config.toml"), "schema_version = 2\n").unwrap();
        let installed = fixture
            .command()
            .args(["shell", "install", "proxy", "--yes"])
            .output()
            .unwrap();
        assert_eq!(installed.status.code(), Some(0), "{installed:?}");
        let removed = fixture
            .command()
            .args(&category_args)
            .arg("--yes")
            .output()
            .unwrap();
        assert_eq!(removed.status.code(), Some(0), "{removed:?}");
        let state = fixture.0.join("state");
        let manifest = state.join("shell-manifest.toml");
        let manifest_before = fs::read(&manifest).unwrap();
        let profile = fs::read_dir(state.join("shell"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("profile.")
            })
            .unwrap();
        let profile_before = fs::read(&profile).unwrap();
        let mut configs = Vec::new();
        config_files(&fixture.0.join("home"), &mut configs);
        assert!(!configs.is_empty());
        let before = configs
            .iter()
            .map(|path| {
                let mut content = fs::read_to_string(path).unwrap();
                content.push_str("\n# user setting survives\n");
                fs::write(path, &content).unwrap();
                content.into_bytes()
            })
            .collect::<Vec<_>>();
        for flags in [vec!["--dry-run"], vec![]] {
            let output = fixture
                .command()
                .args(["shell", "uninstall"])
                .args(&flags)
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(if flags.is_empty() { 1 } else { 0 }),
                "{output:?}"
            );
            if flags.is_empty() {
                assert!(String::from_utf8_lossy(&output.stderr).contains("approval requires"));
            }
            assert_eq!(fs::read(&profile).unwrap(), profile_before);
            assert_eq!(fs::read(&manifest).unwrap(), manifest_before);
            for (path, content) in configs.iter().zip(&before) {
                assert_eq!(fs::read(path).unwrap(), *content);
            }
        }
        let removed = fixture
            .command()
            .args(["shell", "uninstall", "--yes"])
            .output()
            .unwrap();
        assert_eq!(removed.status.code(), Some(0), "{removed:?}");
        assert!(removed.stderr.is_empty(), "{removed:?}");
        assert!(!profile.exists());
        assert!(String::from_utf8_lossy(&removed.stdout).contains("Uninstall complete"));
        for path in &configs {
            let content = fs::read_to_string(path).unwrap();
            assert!(content.contains("# user setting survives"));
            assert!(!content.contains("# >>> shine >>>"));
        }
        let repeated = fixture
            .command()
            .args(["shell", "uninstall", "--yes"])
            .output()
            .unwrap();
        assert_eq!(repeated.status.code(), Some(0), "{repeated:?}");
    }
}

#[test]
fn shell_uninstall_defaults_to_cleanup_without_widening_target_scope() {
    for args in [
        vec!["uninstall", "shell/utils/shine-env-export"],
        vec!["shell", "uninstall", "utils/shine-env-export"],
        vec!["shell", "uninstall"],
    ] {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("shine.config.toml"), "schema_version = 2\n").unwrap();
        for target in ["utils/shine-env-export", "utils/shine-theme-sync"] {
            let installed = fixture
                .command()
                .args(["shell", "install", target, "--yes"])
                .output()
                .unwrap();
            assert_eq!(installed.status.code(), Some(0), "{installed:?}");
        }
        let state = fixture.0.join("state");
        let extra = state.join("presets/shell/utils/extra.txt");
        fs::write(&extra, "extra cache").unwrap();
        let selected = cli::bin_links::command_path_for_name(
            &state.join("bin"),
            std::ffi::OsStr::new("shine-env-export"),
        );
        let sibling = cli::bin_links::command_path_for_name(
            &state.join("bin"),
            std::ffi::OsStr::new("shine-theme-sync"),
        );
        let preview = fixture
            .command()
            .args(&args)
            .arg("--dry-run")
            .output()
            .unwrap();
        assert_eq!(preview.status.code(), Some(0), "{preview:?}");
        assert!(selected.exists());
        assert!(sibling.exists());
        assert_eq!(fs::read(&extra).unwrap(), b"extra cache");
        let denied = fixture.command().args(&args).output().unwrap();
        assert_eq!(denied.status.code(), Some(1), "{denied:?}");
        assert!(String::from_utf8_lossy(&denied.stderr).contains("approval requires"));
        assert!(selected.exists());
        assert_eq!(fs::read(&extra).unwrap(), b"extra cache");
        let removed = fixture.command().args(&args).arg("--yes").output().unwrap();
        assert_eq!(removed.status.code(), Some(0), "{removed:?}");
        assert!(
            fs::symlink_metadata(&selected)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        );
        if args.len() == 2 && args[0] == "shell" {
            assert!(!state.join("presets/shell").exists());
            assert!(!state.join("bin").exists());
        } else {
            assert!(sibling.exists());
            assert_eq!(fs::read(&extra).unwrap(), b"extra cache");
        }
        assert!(state.join("config.toml").exists());
    }
}

#[test]
fn shell_uninstall_help_describes_inventory_and_cleanup_scope() {
    let fixture = Fixture::new();
    for args in [
        vec!["shell", "--help"],
        vec!["shell", "uninstall", "--help"],
    ] {
        let output = fixture.command().args(&args).output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{args:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{args:?}: {output:?}");
        let help = String::from_utf8(output.stdout).unwrap();
        let help = help.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            help.contains("Run 'shine list' to see installed resources"),
            "{args:?}: {help}"
        );
        assert!(
            !help.contains("Run 'shine shell list' to see installed categories"),
            "{args:?}: {help}"
        );
        if args.contains(&"uninstall") {
            assert_eq!(
                help.matches("Run 'shine list' to see installed resources")
                    .count(),
                2,
                "{help}"
            );
            for text in [
                "without TARGET, remove the entire internal Shell preset cache tree, including extra files",
                "With TARGET, retain extra cache files and shared files still needed by installed sibling commands",
                "External preset sources and config.toml are preserved",
            ] {
                assert!(help.contains(text), "{help}");
            }
            assert!(
                !help.contains("Also remove empty managed directories after uninstall"),
                "{help}"
            );
        }
        assert!(
            !fixture.0.join("state").exists(),
            "{args:?} initialized state"
        );
    }
}
