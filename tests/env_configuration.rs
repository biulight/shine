//! Env layering and directed writes through the real CLI, with isolated roots.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("shine-env-layers-{}", uuid::Uuid::new_v4()));
        for path in [
            "outside",
            "project/child",
            "overlay",
            "state",
            "home",
            "home/AppData/Roaming",
            "home/AppData/Local",
            "tools",
        ] {
            fs::create_dir_all(root.join(path)).unwrap();
        }
        Self(root)
    }
    fn write(&self, path: &str, text: &str) {
        fs::write(self.0.join(path), text).unwrap();
    }
    fn read(&self, path: &str) -> String {
        fs::read_to_string(self.0.join(path)).unwrap()
    }
    fn command(&self, project: bool) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_shine"));
        cmd.current_dir(
            self.0
                .join(if project { "project/child" } else { "outside" }),
        )
        .env("HOME", self.0.join("home"))
        .env("USERPROFILE", self.0.join("home"))
        .env("APPDATA", self.0.join("home/AppData/Roaming"))
        .env("LOCALAPPDATA", self.0.join("home/AppData/Local"))
        .env_remove("SUDO_USER")
        .env_remove("SHINE_PRESETS")
        .env_remove("SHINE_CONFIG_DIR")
        .arg("--config-dir")
        .arg(self.0.join("state"));
        cmd
    }
    fn run(&self, project: bool, args: &[&str]) -> Output {
        self.command(project)
            .arg("env")
            .args(args)
            .output()
            .unwrap()
    }
    fn ok(&self, project: bool, args: &[&str]) -> String {
        let out = self.run(project, args);
        assert!(out.status.success(), "{args:?}: {out:?}");
        String::from_utf8(out.stdout).unwrap()
    }
    fn fail(&self, project: bool, args: &[&str], message: &str) {
        let out = self.run(project, args);
        assert!(!out.status.success(), "{args:?}: {out:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(message),
            "{out:?}"
        );
    }
    fn layers(&self) {
        self.write("state/config.toml", "# global comment\npresets_overlay_dir = '../overlay'\n[env]\nHTTP_PROXY_PORT = { value = 'global-config', description = 'global description' }\n");
        self.write(
            "overlay/shine.env.toml",
            "HTTP_PROXY_PORT = { value = 'overlay', description = 'overlay description' }\n",
        );
        self.write("state/shine.env.toml", "HTTP_PROXY_PORT = 'global-env'\n");
        self.write(
            "project/shine.config.toml",
            "# project comment\n[env]\nHTTP_PROXY_PORT = { value = 'project-config' }\n",
        );
        self.write(
            "project/shine.env.toml",
            "HTTP_PROXY_PORT = 'project-env'\n",
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn proxy_exec_preserves_normal_status_streams_and_injection_selection() {
    for mode in ["enabled", "disabled", "missing"] {
        let f = Fixture::new();
        let rule = if mode == "missing" { "other" } else { "probe" };
        let source = if mode == "enabled" {
            "CHILD_SOURCE"
        } else {
            "UNAVAILABLE_SOURCE"
        };
        f.write("state/config.toml", &format!("schema_version = 2\n[[env_proxy]]\ncommand = '{rule}'\nenabled = {}\nwith = ['{source}=CHILD_VALUE']\n[env]\nCHILD_SOURCE = 'injected'\n", mode != "disabled"));
        let before = fs::read(f.0.join("state/config.toml")).unwrap();
        #[cfg(unix)]
        let target = PathBuf::from("/bin/sh");
        #[cfg(windows)]
        let target = PathBuf::from(std::env::var_os("COMSPEC").unwrap());
        for code in [
            0,
            7,
            #[cfg(windows)]
            -1073741510, // STATUS_CONTROL_C_EXIT remains a native Windows exit code.
        ] {
            #[cfg(unix)]
            let args = vec![
                "-c".to_string(),
                "printf 'out:%s:%s\\n' \"$CHILD_VALUE\" \"$1\"; printf 'err\\n' >&2; exit \"$2\""
                    .into(),
                "probe".into(),
                "literal value".into(),
                code.to_string(),
            ];
            #[cfg(windows)]
            let args = vec![
                "/d".to_string(),
                "/c".into(),
                format!("echo out:%CHILD_VALUE%:literal value&echo err>&2&exit /b {code}"),
            ];
            let result = f
                .command(false)
                .env("CHILD_VALUE", "parent")
                .args(["env", "proxy", "exec", "--target"])
                .arg(&target)
                .arg("probe")
                .args(args)
                .output()
                .unwrap();
            assert_eq!(result.status.code(), Some(code), "{mode}: {result:?}");
            let value = if mode == "enabled" {
                "injected"
            } else {
                "parent"
            };
            let newline = if cfg!(windows) { "\r\n" } else { "\n" };
            assert_eq!(
                result.stdout,
                format!("out:{value}:literal value{newline}").as_bytes()
            );
            assert_eq!(result.stderr, format!("err{newline}").as_bytes());
            assert_eq!(fs::read(f.0.join("state/config.toml")).unwrap(), before);
        }
        // A spawn failure still reports an error rather than becoming a child status.
        f.write("tools/non-executable", "not a program");
        let result = f
            .command(false)
            .args(["env", "proxy", "exec", "--target"])
            .arg(f.0.join("tools/non-executable"))
            .arg("probe")
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(1), "{result:?}");
        assert!(result.stdout.is_empty(), "{result:?}");
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("running proxy target"),
            "{result:?}"
        );
        assert_eq!(fs::read(f.0.join("state/config.toml")).unwrap(), before);
    }
}

#[cfg(unix)]
#[test]
fn installed_proxy_preserves_signal_status_binary_output_and_runtime_state() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    for mode in ["enabled", "disabled", "missing"] {
        let f = Fixture::new();
        f.write(
            "state/config.toml",
            "schema_version = 2\n[env]\nCHILD_SOURCE = 'injected'\n",
        );
        f.write("project/shine.config.toml", "# project-only rule\n");
        f.write("tools/probe", "#!/bin/sh\nprintf '\\000OUT:%s:%s\\n' \"$CHILD_VALUE\" \"$2\"\nprintf '\\000ERR\\n' >&2\ncase \"$1\" in\nterm) kill -TERM \"$$\";;\nkill) kill -KILL \"$$\";;\nexit7) exit 7;;\nesac\n");
        fs::set_permissions(f.0.join("tools/probe"), fs::Permissions::from_mode(0o700)).unwrap();
        symlink(env!("CARGO_BIN_EXE_shine"), f.0.join("tools/shine")).unwrap();
        let path = std::env::join_paths(
            std::iter::once(f.0.join("tools"))
                .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        let mut install = f.command(mode == "missing");
        install.env("PATH", &path).args([
            "env",
            "proxy",
            "install",
            "probe",
            "--with",
            "CHILD_SOURCE=CHILD_VALUE",
        ]);
        if mode == "missing" {
            install.arg("--project");
        }
        let result = install.output().unwrap();
        assert!(result.status.success(), "{result:?}");
        if mode == "disabled" {
            f.ok(false, &["proxy", "disable", "probe"]);
        }
        let launcher = f.0.join("state/bin/probe");
        let paths = [
            f.0.join("state/config.toml"),
            f.0.join("project/shine.config.toml"),
            f.0.join("state/proxy-manifest.toml"),
            launcher.clone(),
        ];
        let before = paths
            .iter()
            .map(|path| fs::read(path).unwrap())
            .collect::<Vec<_>>();
        for (operation, code) in [
            ("term", 128 + libc::SIGTERM),
            ("kill", 128 + libc::SIGKILL),
            ("exit7", 7),
            ("ok", 0),
        ] {
            let argument = "literal ' \\\nargument";
            let result = Command::new(&launcher)
                .current_dir(f.0.join("outside"))
                .env("HOME", f.0.join("home"))
                .env_remove("SUDO_USER")
                .env_remove("SHINE_PRESETS")
                .env("PATH", &path)
                .env("CHILD_VALUE", "parent")
                .args([operation, argument])
                .output()
                .unwrap();
            assert_eq!(
                result.status.code(),
                Some(code),
                "{mode}/{operation}: {result:?}"
            );
            let value = if mode == "enabled" {
                "injected"
            } else {
                "parent"
            };
            assert_eq!(
                result.stdout,
                format!("\0OUT:{value}:{argument}\n").as_bytes()
            );
            assert_eq!(result.stderr, b"\0ERR\n");
            for (path, bytes) in paths.iter().zip(&before) {
                assert_eq!(fs::read(path).unwrap(), *bytes);
            }
        }
    }
}

#[test]
fn shell_export_rejects_nul_without_altering_raw_values_or_configuration() {
    for shell in ["bash", "zsh", "fish", "powershell"] {
        for value in [
            "\0private-prefix",
            "private-prefix\0private-suffix",
            "private-suffix\0",
        ] {
            let f = Fixture::new();
            f.write(
                "state/config.toml",
                &format!(
                    "schema_version = 2\nshell_type = '{shell}'\n[env]\nTOKEN = {}\n",
                    toml::Value::String(value.into())
                ),
            );
            let before = fs::read(f.0.join("state/config.toml")).unwrap();
            for alias in ["TOKEN", "API_TOKEN"] {
                let result = f.run(false, &["secret", "export", "TOKEN", "--as", alias]);
                assert_eq!(result.status.code(), Some(1), "{shell}: {result:?}");
                assert!(result.stdout.is_empty(), "{result:?}");
                let error = String::from_utf8(result.stderr).unwrap();
                assert!(error.contains("NUL"), "{error}");
                assert!(!error.contains("private-"), "{error}");
                assert!(!error.contains('\0'), "{error}");
            }
            let raw = f.run(false, &["get", "TOKEN"]);
            assert_eq!(raw.status.code(), Some(0), "{raw:?}");
            assert_eq!(raw.stdout, format!("{value}\n").as_bytes());
            assert!(raw.stderr.is_empty(), "{raw:?}");
            assert_eq!(fs::read(f.0.join("state/config.toml")).unwrap(), before);
            assert_eq!(fs::read_dir(f.0.join("state")).unwrap().count(), 1);
        }
    }
}

#[test]
fn shell_export_valid_literals_round_trip_through_native_shell() {
    #[cfg(unix)]
    let shell = "bash";
    #[cfg(windows)]
    let shell = "powershell";
    for value in ["", "plain", "a ' quote \" $HOME `tick` \\path\nnext\t中文"] {
        let f = Fixture::new();
        f.write(
            "state/config.toml",
            &format!(
                "schema_version = 2\nshell_type = '{shell}'\n[env]\nTOKEN = {}\n",
                toml::Value::String(value.into())
            ),
        );
        let before = fs::read(f.0.join("state/config.toml")).unwrap();
        let result = f.run(false, &["secret", "export", "TOKEN", "--as", "API_TOKEN"]);
        assert_eq!(result.status.code(), Some(0), "{result:?}");
        assert!(result.stderr.is_empty(), "{result:?}");
        let code = String::from_utf8(result.stdout).unwrap();
        #[cfg(unix)]
        let mut consumer = {
            let mut command = Command::new("/bin/sh");
            command.args(["-c", &format!("{code}\nprintf '%s' \"$API_TOKEN\"")]);
            command
        };
        #[cfg(windows)]
        let mut consumer = {
            let mut command = Command::new("powershell.exe");
            command.args(["-NoProfile", "-NonInteractive", "-Command", &format!(
                "{code}\n$bytes = [Text.Encoding]::UTF8.GetBytes([string]$env:API_TOKEN); [Console]::OpenStandardOutput().Write($bytes, 0, $bytes.Length)"
            )]);
            command
        };
        let consumed = consumer
            .current_dir(f.0.join("outside"))
            .env_remove("API_TOKEN")
            .output()
            .unwrap();
        assert_eq!(consumed.status.code(), Some(0), "{consumed:?}");
        assert!(consumed.stderr.is_empty(), "{consumed:?}");
        assert_eq!(consumed.stdout, value.as_bytes());
        assert_eq!(fs::read(f.0.join("state/config.toml")).unwrap(), before);
    }
}

#[cfg(unix)]
#[test]
fn shell_export_preset_preserves_failures_and_only_evaluates_success() {
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("presets/shell/utils/shine-env-export.sh");
    for shell in [
        "/bin/bash",
        #[cfg(target_os = "macos")]
        "/bin/zsh",
    ] {
        for (mode, expected_code, expected_value) in [
            ("valid", 0, "next\nvalue"),
            ("collision", 0, "next\nvalue"),
            ("nul", 1, "parent"),
            ("missing", 1, "parent"),
            ("parse-error", 2, "parent"),
            ("partial-failure", 7, "parent"),
        ] {
            let f = Fixture::new();
            let value = if mode == "nul" {
                "private\0value"
            } else {
                "next\nvalue"
            };
            f.write(
                "state/config.toml",
                &format!(
                    "schema_version = 2\nshell_type = 'bash'\n[env]\nTOKEN = {}\n",
                    toml::Value::String(value.into())
                ),
            );
            let before = fs::read(f.0.join("state/config.toml")).unwrap();
            // Source the actual needs_source preset inside a function, as the shell
            // launcher does. A failing producer's partial stdout must never be eval'd.
            let result = Command::new(shell)
                .current_dir(f.0.join("outside"))
                .env("HOME", f.0.join("home"))
                .env("SHINE_CONFIG_DIR", f.0.join("state"))
                .env_remove("SHINE_PRESETS")
                .env_remove("SUDO_USER")
                .env("SHINE_TEST_BINARY", env!("CARGO_BIN_EXE_shine"))
                .env("SHINE_TEST_PRESET", &source)
                .env("SHINE_TEST_MODE", mode)
                .env("API_TOKEN", "parent")
                .args([
                    "-c",
                    r#"
shine() {
    if [ "$SHINE_TEST_MODE" = partial-failure ]; then
        printf "export API_TOKEN='must-not-run'\n"
        return 7
    fi
    "$SHINE_TEST_BINARY" "$@"
}
wrapped() { . "$SHINE_TEST_PRESET"; }
case "$SHINE_TEST_MODE" in
    missing) wrapped MISSING_TOKEN --as API_TOKEN;;
    parse-error) wrapped TOKEN --invalid-option;;
    collision) wrapped TOKEN --as _shine_export_code;;
    *) wrapped TOKEN --as API_TOKEN;;
esac
export_status=$?
if [ "$SHINE_TEST_MODE" = collision ]; then
    printf '%s' "$_shine_export_code"
else
    printf '%s' "$API_TOKEN"
fi
exit "$export_status"
"#,
                ])
                .output()
                .unwrap();
            assert_eq!(
                result.status.code(),
                Some(expected_code),
                "{mode}: {result:?}"
            );
            assert_eq!(
                result.stdout,
                expected_value.as_bytes(),
                "{mode}: {result:?}"
            );
            if matches!(mode, "valid" | "collision" | "partial-failure") {
                assert!(result.stderr.is_empty(), "{result:?}");
            } else {
                assert!(!result.stderr.is_empty(), "{result:?}");
                assert!(
                    !String::from_utf8_lossy(&result.stderr).contains("private"),
                    "{result:?}"
                );
            }
            assert_eq!(fs::read(f.0.join("state/config.toml")).unwrap(), before);
        }
    }
}

#[cfg(unix)]
#[test]
fn shell_export_checks_the_decrypted_winner_without_changing_decrypt_output() {
    use std::os::unix::fs::PermissionsExt;
    for value in [
        b"secret-prefix\0secret-suffix".as_slice(),
        b"safe-decrypted",
    ] {
        let f = Fixture::new();
        f.write("state/config.toml", "schema_version = 2\nshell_type = 'bash'\nsecret_backend = 'age'\n[env]\nTOKEN = \"plaintext-\\u0000-fallback\"\nTOKEN_SECRET = 'eA=='\n");
        let before = fs::read(f.0.join("state/config.toml")).unwrap();
        let payload = f.0.join("payload");
        fs::write(&payload, value).unwrap();
        let log = f.0.join("gpg.log");
        let gpg = f.0.join("tools/gpg");
        // An isolated substitute returns public test bytes; no real keys or tools.
        fs::write(&gpg, "#!/bin/sh\n[ \"$1\" = --decrypt ] && [ \"$2\" = --output ] && [ \"$3\" = - ] && [ \"$#\" = 4 ] || exit 91\nprintf 'called\\n' >> \"$SHINE_TEST_GPG_LOG\"\n/bin/cat \"$SHINE_TEST_PLAINTEXT\"\n").unwrap();
        fs::set_permissions(gpg, fs::Permissions::from_mode(0o700)).unwrap();
        let run = |args: &[&str]| {
            f.command(false)
                .env("PATH", f.0.join("tools"))
                .env("SHINE_TEST_PLAINTEXT", &payload)
                .env("SHINE_TEST_GPG_LOG", &log)
                .arg("env")
                .args(args)
                .output()
                .unwrap()
        };
        let result = run(&["secret", "export", "TOKEN", "--as", "API_TOKEN"]);
        if value.contains(&0) {
            assert_eq!(result.status.code(), Some(1), "{result:?}");
            assert!(result.stdout.is_empty(), "{result:?}");
            let error = String::from_utf8(result.stderr).unwrap();
            assert!(error.contains("NUL"), "{error}");
            assert!(!error.contains("secret-prefix"), "{error}");
            assert!(!error.contains("secret-suffix"), "{error}");
        } else {
            assert_eq!(result.status.code(), Some(0), "{result:?}");
            assert_eq!(result.stdout, b"export API_TOKEN='safe-decrypted'\n");
            assert!(result.stderr.is_empty(), "{result:?}");
        }
        assert_eq!(fs::read(&log).unwrap(), b"called\n");
        let decrypted = run(&["secret", "decrypt", "TOKEN_SECRET"]);
        assert_eq!(decrypted.status.code(), Some(0), "{decrypted:?}");
        assert_eq!(decrypted.stdout, value);
        assert!(decrypted.stderr.is_empty(), "{decrypted:?}");
        assert_eq!(fs::read(log).unwrap(), b"called\ncalled\n");
        assert_eq!(fs::read(f.0.join("state/config.toml")).unwrap(), before);
        assert_eq!(fs::read_dir(f.0.join("state")).unwrap().count(), 1);
    }
}

#[test]
fn queries_are_read_only_and_defaults_are_not_persisted() {
    let f = Fixture::new();
    fs::remove_dir(f.0.join("state")).unwrap();
    assert_eq!(f.ok(false, &["get", "HTTP_PROXY_PORT"]), "6152\n");
    assert!(f.ok(false, &["list"]).contains("builtin"));
    let explanation = f.ok(false, &["explain", "HTTP_PROXY_PORT"]);
    assert!(explanation.contains("Value source: builtin"));
    assert!(!explanation.contains("6152"));
    assert!(!f.0.join("state").exists());
    f.ok(false, &["set", "HTTP_PROXY_PORT", "6152", "--global"]);
    let saved = f.read("state/config.toml");
    assert!(saved.contains("HTTP_PROXY_PORT"));
    let table: toml::Table = toml::from_str(&saved).unwrap();
    assert_eq!(table["schema_version"].as_integer(), Some(2));
    assert!(!saved.contains("SOCKS5_PROXY_PORT"));
    assert!(
        f.ok(false, &["explain", "HTTP_PROXY_PORT"])
            .contains("Value source: global config")
    );
    f.ok(false, &["delete", "HTTP_PROXY_PORT", "--global"]);
    assert_eq!(f.ok(false, &["get", "HTTP_PROXY_PORT"]), "6152\n");
    assert!(!f.read("state/config.toml").contains("HTTP_PROXY_PORT"));
}

#[test]
fn six_layers_descriptions_and_manual_inheritance_are_deterministic() {
    let f = Fixture::new();
    f.layers();
    let explanation = f.ok(true, &["explain", "HTTP_PROXY_PORT"]);
    assert!(explanation.contains("Value source: project env file"));
    assert!(explanation.contains("Description source: global config"));
    for value in [
        "6152",
        "global-config",
        "global-env",
        "project-config",
        "project-env",
    ] {
        assert!(!explanation.contains(&format!("value: {value}")));
    }
    assert!(
        f.ok(true, &["explain", "HTTP_PROXY_PORT", "--reveal"])
            .contains("value: \"6152\"")
    );
    for (project, flags, expected) in [
        (true, vec!["--project", "--env-file"], "project-config\n"),
        (true, vec!["--project"], "global-env\n"),
        (false, vec!["--global", "--env-file"], "global-config\n"),
        (false, vec!["--global"], "overlay\n"),
        (false, vec!["--overlay"], "6152\n"),
    ] {
        let mut args = vec!["delete", "HTTP_PROXY_PORT"];
        args.extend(flags);
        f.ok(project, &args);
        assert_eq!(f.ok(project, &["get", "HTTP_PROXY_PORT"]), expected);
    }
    assert!(f.read("state/config.toml").contains("# global comment"));
    assert!(
        f.read("project/shine.config.toml")
            .contains("# project comment")
    );
}

#[test]
fn implicit_shadow_protection_and_explicit_maintenance_preserve_other_layers() {
    let f = Fixture::new();
    f.layers();
    let global = f.read("state/config.toml");
    let project = f.read("project/shine.config.toml");
    f.fail(
        true,
        &["set", "HTTP_PROXY_PORT", "new"],
        "--project --env-file",
    );
    f.fail(true, &["delete", "HTTP_PROXY_PORT"], "would be shadowed");
    assert_eq!(f.read("state/config.toml"), global);
    assert_eq!(f.read("project/shine.config.toml"), project);
    let output = f.ok(true, &["set", "HTTP_PROXY_PORT", "new", "--global"]);
    assert!(output.contains("Current effective source: project env file"));
    assert!(output.contains("Current value unchanged"));
    assert!(
        f.read("state/config.toml")
            .contains("description = 'global description'")
    );
    assert_eq!(f.read("project/shine.config.toml"), project);
    f.ok(
        true,
        &[
            "set",
            "HTTP_PROXY_PORT",
            "baseline",
            "--overlay",
            "--env-file",
        ],
    );
    assert_eq!(f.ok(true, &["get", "HTTP_PROXY_PORT"]), "project-env\n");
    f.ok(true, &["set", "NEW_KEY", "project", "--env-file"]);
    assert!(f.read("project/shine.env.toml").contains("NEW_KEY"));
    assert!(!f.read("state/shine.env.toml").contains("NEW_KEY"));
    f.ok(true, &["delete", "HTTP_PROXY_PORT", "--project"]);
    f.fail(
        true,
        &["delete", "HTTP_PROXY_PORT", "--project"],
        "has no local entry",
    );
    assert_eq!(f.ok(true, &["get", "HTTP_PROXY_PORT"]), "project-env\n");
}

#[test]
fn invalid_targets_and_force_reject_without_creating_files() {
    let f = Fixture::new();
    f.fail(
        false,
        &["set", "K", "v", "--project"],
        "requires an existing",
    );
    f.fail(
        false,
        &["set", "K", "v", "--global", "--project"],
        "cannot be used with",
    );
    f.fail(false, &["set", "K", "v", "--overlay"], "manual overlay");
    f.fail(false, &["set", "K", "v", "--force"], "--force is retired");
    assert!(!f.0.join("state/config.toml").exists());
    f.write(
        "state/config.toml",
        "presets_overlay_git = 'https://example.invalid/overlay'\n",
    );
    fs::create_dir(f.0.join("state/overlay")).unwrap();
    f.write("state/overlay/shine.env.toml", "K = 'mirror'\n");
    f.fail(
        false,
        &["set", "K", "v", "--overlay"],
        "managed mirrors are read-only",
    );
    f.fail(false, &["delete", "K", "--force"], "--overlay");
    assert_eq!(f.read("state/overlay/shine.env.toml"), "K = 'mirror'\n");
}

#[test]
fn force_diagnoses_the_retired_target_even_when_new_precedence_changes_the_winner() {
    let f = Fixture::new();
    f.layers();
    fs::remove_file(f.0.join("project/shine.env.toml")).unwrap();
    assert_eq!(f.ok(true, &["get", "HTTP_PROXY_PORT"]), "project-config\n");
    f.fail(
        true,
        &["set", "HTTP_PROXY_PORT", "v", "--force"],
        "Replace --force with --overlay",
    );
    assert!(
        f.read("overlay/shine.env.toml")
            .contains("value = 'overlay'")
    );
    fs::remove_file(f.0.join("overlay/shine.env.toml")).unwrap();
    f.fail(
        true,
        &["secret", "encrypt", "--set", "HTTP_PROXY_PORT", "--force"],
        "Replace --force with --global --env-file",
    );
    assert_eq!(
        f.read("state/shine.env.toml"),
        "HTTP_PROXY_PORT = 'global-env'\n"
    );
    fs::remove_file(f.0.join("state/shine.env.toml")).unwrap();
    f.ok(true, &["delete", "HTTP_PROXY_PORT", "--project"]);
    f.fail(
        true,
        &["set", "HTTP_PROXY_PORT", "v", "--force"],
        "Replace --force with --project",
    );
}

#[test]
fn project_discovery_ignores_lone_env_files_and_uses_each_projects_overlay() {
    let f = Fixture::new();
    f.write("project/shine.env.toml", "HTTP_PROXY_PORT = 'ignored'\n");
    assert_eq!(f.ok(true, &["get", "HTTP_PROXY_PORT"]), "6152\n");
    f.write(
        "project/shine.config.toml",
        "presets_overlay_dir = '../overlay'\n",
    );
    f.write(
        "overlay/shine.env.toml",
        "HTTP_PROXY_PORT = 'project-baseline'\n",
    );
    assert_eq!(f.ok(true, &["get", "HTTP_PROXY_PORT"]), "ignored\n");
    fs::remove_file(f.0.join("project/shine.env.toml")).unwrap();
    assert_eq!(
        f.ok(true, &["get", "HTTP_PROXY_PORT"]),
        "project-baseline\n"
    );
    assert_eq!(f.ok(false, &["get", "HTTP_PROXY_PORT"]), "6152\n");
    f.write("project/shine.config.toml", "presets_overlay_dir = '../overlay'\n[env]\nHTTP_PROXY_PORT = { value = 'project', description = '' }\nHTTP_PROXY_PORT_SECRET = 'ciphertext-hidden'\n");
    let out = f.ok(true, &["explain", "HTTP_PROXY_PORT"]);
    assert!(out.contains("Description source: project config"));
    assert!(out.contains("Explicit injection prefers HTTP_PROXY_PORT_SECRET"));
    assert!(!out.contains("ciphertext-hidden"));
    f.ok(true, &["set", "HTTP_PROXY_PORT", "other", "--project"]);
    assert!(
        f.read("project/shine.config.toml")
            .contains("description = ''")
    );
}

#[test]
fn invalid_shadowed_layers_are_still_validated() {
    let f = Fixture::new();
    f.layers();
    f.write("overlay/shine.env.toml", "HTTP_PROXY_PORT = 12\n");
    f.fail(true, &["get", "HTTP_PROXY_PORT"], "invalid env variable");
    f.fail(
        true,
        &["set", "HTTP_PROXY_PORT", "v", "--project"],
        "invalid env variable",
    );
}

#[cfg(unix)]
fn fake_gpg(f: &Fixture) {
    use std::os::unix::fs::PermissionsExt;
    f.write("tools/gpg", "#!/bin/sh\ncat >/dev/null\n: > \"$SIGNAL\"\nwhile [ ! -f \"$RELEASE\" ]; do sleep 0.01; done\nprintf 'fake-ciphertext'\n");
    fs::set_permissions(f.0.join("tools/gpg"), fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(windows)]
fn fake_gpg(f: &Fixture) {
    // A native probe lets Windows exercise the same preflight/commit handshake
    // without a shell shim or a real encryption tool/keyring.
    static PROBE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    let probe = PROBE.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("shine-env-gpg-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("gpg.rs");
        let binary = root.join("gpg.exe");
        fs::write(
            &source,
            r#"
use std::{env, fs, io::{self, Read}, path::Path, thread, time::{Duration, Instant}};
fn main() {
    let mut input = Vec::new();
    io::stdin().read_to_end(&mut input).unwrap();
    fs::write(env::var_os("SIGNAL").unwrap(), b"").unwrap();
    let release = env::var_os("RELEASE").unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !Path::new(&release).exists() {
        assert!(Instant::now() < deadline, "release timed out");
        thread::sleep(Duration::from_millis(10));
    }
    print!("fake-ciphertext");
}
"#,
        )
        .unwrap();
        let out = Command::new("rustc")
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        binary
    });
    fs::copy(probe, f.0.join("tools/gpg.exe")).unwrap();
}

fn encrypt_child(f: &Fixture, signal: &Path, release: &Path, key: &str) -> std::process::Child {
    use std::process::Stdio;
    let mut paths = vec![f.0.join("tools")];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    f.command(false)
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("SIGNAL", signal)
        .env("RELEASE", release)
        .args([
            "env", "secret", "encrypt", "--from", "SOURCE", "--set", key, "--global", "-r", "fake",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn wait_for(path: &Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !path.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn encryption_preflight_rejects_before_tool_and_changes_during_tool_reject_commit() {
    let f = Fixture::new();
    fake_gpg(&f);
    f.write(
        "state/config.toml",
        "[env]\nSOURCE = 'private-input'\nTOKEN_SECRET = 'old'\n",
    );
    f.write("state/shine.env.toml", "TOKEN_SECRET = 'higher'\n");
    let signal = f.0.join("started");
    let release = f.0.join("release");
    let out = f
        .command(false)
        .env("PATH", f.0.join("tools"))
        .env("SIGNAL", &signal)
        .env("RELEASE", &release)
        .args([
            "env",
            "secret",
            "encrypt",
            "--from",
            "SOURCE",
            "--set",
            "TOKEN_SECRET",
            "-r",
            "fake",
        ])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(!signal.exists());
    let child = encrypt_child(&f, &signal, &release, "TOKEN_SECRET");
    wait_for(&signal);
    let edited = "[env]\nSOURCE = 'edited-during-encryption'\nTOKEN_SECRET = 'retained'\n";
    f.write("state/config.toml", edited);
    f.write("release", "");
    let out = child.wait_with_output().unwrap();
    assert!(!out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("retry"));
    assert_eq!(f.read("state/config.toml"), edited);

    // A changed --from in another layer also invalidates an unchanged target.
    let target = "presets_overlay_dir = '../overlay'\n[env]\nTOKEN_SECRET = 'retained'\n";
    f.write("state/config.toml", target);
    f.write(
        "overlay/shine.env.toml",
        "SOURCE = 'private-overlay-input'\n",
    );
    fs::remove_file(&signal).unwrap();
    fs::remove_file(&release).unwrap();
    let child = encrypt_child(&f, &signal, &release, "TOKEN_SECRET");
    wait_for(&signal);
    f.write("overlay/shine.env.toml", "SOURCE = 'changed-source'\n");
    f.write("release", "");
    let out = child.wait_with_output().unwrap();
    assert!(!out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("retry"));
    assert_eq!(f.read("state/config.toml"), target);
}

#[test]
fn two_processes_cannot_commit_the_same_stale_snapshot() {
    let f = Fixture::new();
    fake_gpg(&f);
    f.write("state/config.toml", "[env]\nSOURCE = 'private-input'\n");
    let release = f.0.join("release");
    let one = encrypt_child(&f, &f.0.join("one"), &release, "ONE_SECRET");
    let two = encrypt_child(&f, &f.0.join("two"), &release, "TWO_SECRET");
    wait_for(&f.0.join("one"));
    wait_for(&f.0.join("two"));
    f.write("release", "");
    let one = one.wait_with_output().unwrap();
    let two = two.wait_with_output().unwrap();
    assert_ne!(
        one.status.success(),
        two.status.success(),
        "{one:?} {two:?}"
    );
    let failed = if one.status.success() { two } else { one };
    assert!(String::from_utf8_lossy(&failed.stderr).contains("retry"));
    let table: toml::Table = toml::from_str(&f.read("state/config.toml")).unwrap();
    assert_eq!(table["env"]["SOURCE"].as_str(), Some("private-input"));
    let entries = table["env"].as_table().unwrap();
    assert_ne!(
        entries.contains_key("ONE_SECRET"),
        entries.contains_key("TWO_SECRET")
    );
}

#[cfg(windows)]
#[test]
fn windows_sharing_violation_keeps_original_and_retry_replaces_it() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new();
    let original = "# retained comment\ncustom = 'retained'\n[env]\nKEY = 'original'\n";
    f.write("state/config.toml", original);
    // Permit reading/writing but deny FILE_SHARE_DELETE, as a Windows editor can.
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(3)
        .open(f.0.join("state/config.toml"))
        .unwrap();
    let out = f.run(false, &["set", "KEY", "replacement", "--global"]);
    assert!(!out.status.success(), "{out:?}");
    assert_eq!(f.read("state/config.toml"), original);
    assert!(fs::read_dir(f.0.join("state")).unwrap().all(|entry| {
        let name = entry.unwrap().file_name();
        name == "config.toml" || name == ".config.toml.shine.lock"
    }));
    drop(held);
    f.ok(false, &["set", "KEY", "replacement", "--global"]);
    let saved = f.read("state/config.toml");
    assert!(saved.contains("# retained comment"));
    assert!(saved.contains("custom = 'retained'"));
    assert_eq!(f.ok(false, &["get", "KEY"]), "replacement\n");
}

#[cfg(unix)]
#[test]
fn linked_and_special_targets_are_rejected_and_saved_files_are_private() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let f = Fixture::new();
    f.write("outside/original", "KEEP = 'untouched'\n");
    symlink(
        f.0.join("outside/original"),
        f.0.join("state/shine.env.toml"),
    )
    .unwrap();
    f.fail(
        false,
        &["set", "K", "v", "--global", "--env-file"],
        "regular file",
    );
    assert_eq!(f.read("outside/original"), "KEEP = 'untouched'\n");
    fs::remove_file(f.0.join("state/shine.env.toml")).unwrap();
    let fifo_path =
        std::ffi::CString::new(f.0.join("state/shine.env.toml").to_str().unwrap()).unwrap();
    // SAFETY: NUL-terminated path is valid for this call; fixture owns the new FIFO.
    assert_eq!(unsafe { libc::mkfifo(fifo_path.as_ptr(), 0o600) }, 0);
    f.fail(
        false,
        &["set", "K", "v", "--global", "--env-file"],
        "regular file",
    );
    fs::remove_file(f.0.join("state/shine.env.toml")).unwrap();
    f.ok(false, &["set", "K", "v", "--global", "--env-file"]);
    assert_eq!(
        fs::metadata(f.0.join("state/shine.env.toml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn proxy_toggle_reports_write_scope_and_effective_project_override() {
    for (action, global_enabled, project_enabled) in
        [("disable", false, true), ("enable", true, false)]
    {
        let f = Fixture::new();
        f.write("state/config.toml", &format!("schema_version = 2\n[[env_proxy]]\ncommand = 'demo'\nwith = ['DEMO']\nenabled = {}\n", !global_enabled));
        let project = format!(
            "schema_version = 2\n[[env_proxy]]\ncommand = 'demo'\nwith = ['DEMO']\nenabled = {project_enabled}\n"
        );
        f.write("project/shine.config.toml", &project);
        let output = f.run(true, &["proxy", action, "demo"]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("in global config"), "{text}");
        let effective = if project_enabled {
            "enabled"
        } else {
            "disabled"
        };
        assert!(
            text.contains(&format!("Current effective state: {effective}")),
            "{text}"
        );
        assert_eq!(f.read("project/shine.config.toml"), project);
        let global: toml::Value = toml::from_str(&f.read("state/config.toml")).unwrap();
        // enabled=true can be represented by the default, so decode through the value's default.
        assert_eq!(
            global["env_proxy"][0]
                .get("enabled")
                .and_then(toml::Value::as_bool)
                .unwrap_or(true),
            global_enabled
        );
        let global_bytes = f.read("state/config.toml");
        let text = f.ok(true, &["proxy", action, "demo", "--project"]);
        assert!(text.contains("in project config"), "{text}");
        let effective = if global_enabled {
            "enabled"
        } else {
            "disabled"
        };
        assert!(
            text.contains(&format!("Current effective state: {effective}")),
            "{text}"
        );
        assert_eq!(f.read("state/config.toml"), global_bytes);
        let list = f.ok(true, &["proxy", "list"]);
        assert!(
            list.contains(&format!("demo: DEMO ({effective})")),
            "{list}"
        );
    }
}

#[test]
fn env_list_escapes_cells_aligns_display_columns_and_preserves_raw_values() {
    let f = Fixture::new();
    f.write(
        "state/config.toml",
        r#"schema_version = 2
[env]
ASCII = { value = "abc", description = "marker-ascii" }
WIDE = { value = "中文中文", description = "marker-wide" }
COMBINING = { value = "éé", description = "marker-combining" }
MULTILINE = { value = "first\nsecond\tend", description = "marker-multi\nnext" }
API_TOKEN = { value = "hidden\nvalue", description = "marker-secret" }
"#,
    );
    let before = f.read("state/config.toml");
    let output = f.ok(false, &["list"]);
    let rows: Vec<_> = output
        .lines()
        .filter(|line| line.contains("marker-"))
        .collect();
    assert_eq!(rows.len(), 5, "{output}");
    let columns: Vec<_> = rows
        .iter()
        .map(|line| console::measure_text_width(line.split_once("marker-").unwrap().0))
        .collect();
    assert!(
        columns.iter().all(|column| *column == columns[0]),
        "{columns:?}\n{output}"
    );
    assert!(output.contains(r"first\nsecond\tend"), "{output}");
    assert!(output.contains(r"marker-multi\nnext"), "{output}");
    assert!(output.contains("<redacted>"), "{output}");
    assert!(!output.contains("hidden"), "{output}");
    let revealed = f.ok(false, &["list", "--reveal"]);
    assert!(revealed.contains(r"hidden\nvalue"), "{revealed}");
    assert_eq!(f.ok(false, &["get", "MULTILINE"]), "first\nsecond\tend\n");
    assert_eq!(f.read("state/config.toml"), before);
}
