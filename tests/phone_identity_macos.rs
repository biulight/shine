//! Exercise the real macOS CLI without touching hardware or the user's config.
#![cfg(target_os = "macos")]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

const TAG: &str = "age1tag1qgg72x2qfk9wg3wh0qg9u0v7l5dkq4jx69fv80p6wdus3ftg6flwgc25f05";

struct Fixture {
    root: PathBuf,
    original_config: String,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("shine-phone-cli-{}", uuid::Uuid::new_v4()));
        for dir in ["bin", "state/age", "project"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        // Isolate project discovery from any configuration above the temp tree.
        fs::write(root.join("project/shine.config.toml"), "").unwrap();
        // Seed the normal defaults so first-run initialization cannot obscure
        // whether the pairing command itself changed configuration.
        let default_env: std::collections::BTreeMap<_, _> =
            cli::config::DEFAULT_ENV_VARS.iter().copied().collect();
        let original_config = format!(
            "secret_backend = \"gpg\"\nage_recipients = [\"{TAG}\"]\n\n[env]\n{}",
            toml::to_string(&default_env).unwrap()
        );
        fs::write(root.join("state/config.toml"), &original_config).unwrap();
        fs::write(
            root.join("state/age/identity.txt"),
            "AGE-SECRET-KEY-1EXAMPLE\n",
        )
        .unwrap();
        fs::write(
            root.join("phone identity.txt"),
            format!("# recipient: {TAG}\nAGE-PLUGIN-PHONE-1EXAMPLE\n"),
        )
        .unwrap();
        let fixture = Self {
            root,
            original_config,
        };
        fixture.script(
            "age",
            "[ \"$1\" = --version ] || exit 11\nprintf 'v1.3.1\\n'\n",
        );
        fixture.script(
            "age-plugin-phone",
            r#"printf '%s\n' "$@" >> "$PHONE_TEST_ARGS"
printf x >> "$PHONE_TEST_CALLS"
[ "$PHONE_TEST_FAIL" != 1 ] || exit 2
printf '%s\n' "$PHONE_TEST_JSON"
"#,
        );
        fixture
    }

    fn script(&self, name: &str, body: &str) {
        let path = self.root.join("bin").join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    fn run(&self, extra: &[&str], fail: bool, recipient: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_shine"))
            .args(["env", "secret", "identity", "init", "--phone"])
            .args(extra)
            .current_dir(self.root.join("project"))
            .env_clear()
            .env("PATH", self.root.join("bin"))
            .env("SHINE_CONFIG_DIR", self.root.join("state"))
            .env("PHONE_TEST_ARGS", self.root.join("args"))
            .env("PHONE_TEST_CALLS", self.root.join("calls"))
            .env("PHONE_TEST_FAIL", if fail { "1" } else { "0" })
            .env(
                "PHONE_TEST_JSON",
                serde_json::json!({
                    "schema_version": 1,
                    "identity_path": self.root.join("phone identity.txt"),
                    "recipient": recipient,
                })
                .to_string(),
            )
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn assert_config_unchanged(&self) {
        assert_eq!(
            fs::read_to_string(self.root.join("state/config.toml")).unwrap(),
            self.original_config
        );
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn macos_phone_cli_appends_stub_and_preserves_existing_configuration() {
    let fixture = Fixture::new();
    let output = fixture.run(&["--label", "Work Mac"], false, TAG);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("args")).unwrap(),
        "setup\n--label\nWork Mac\n--transport\nauto\n--json\n--recipient-type\ntag\n"
    );
    assert_eq!(fs::read_to_string(fixture.root.join("calls")).unwrap(), "x");
    let config: toml::Value =
        toml::from_str(&fs::read_to_string(fixture.root.join("state/config.toml")).unwrap())
            .unwrap();
    assert_eq!(config["secret_backend"].as_str(), Some("gpg"));
    assert_eq!(config["age_recipients"][0].as_str(), Some(TAG));
    let paths = config["age_identities"].as_array().unwrap();
    assert_eq!(paths.len(), 2);
    assert_eq!(
        paths[0].as_str().unwrap(),
        fixture
            .root
            .join("state/age/identity.txt")
            .to_str()
            .unwrap()
    );
    assert_eq!(
        paths[1].as_str().unwrap(),
        fixture.root.join("phone identity.txt").to_str().unwrap()
    );
}

#[test]
fn macos_phone_cli_resolves_default_label_and_forwards_explicit_transport() {
    let fixture = Fixture::new();
    let output = fixture.run(
        &["--transport", "adb", "--adb-serial", "device serial"],
        false,
        TAG,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let args = fs::read_to_string(fixture.root.join("args")).unwrap();
    let args: Vec<_> = args.lines().collect();
    assert_eq!(&args[..2], &["setup", "--label"]);
    assert!(!args[2].trim().is_empty() && args[2].len() <= 64);
    assert_eq!(
        &args[3..],
        &[
            "--transport",
            "adb",
            "--json",
            "--recipient-type",
            "tag",
            "--adb-serial",
            "device serial"
        ]
    );
}

#[test]
fn macos_phone_cli_failure_never_retries_or_updates_config() {
    for (fail, recipient) in [(true, TAG), (false, "age1tag1invalid")] {
        let fixture = Fixture::new();
        let output = fixture.run(&["--label", "Work Mac"], fail, recipient);
        assert!(!output.status.success());
        assert_eq!(fs::read_to_string(fixture.root.join("calls")).unwrap(), "x");
        fixture.assert_config_unchanged();
    }
}

#[test]
fn macos_phone_cli_preflight_blocks_pairing() {
    for project_override in [false, true] {
        let fixture = Fixture::new();
        if project_override {
            fs::write(
                fixture.root.join("project/shine.config.toml"),
                "age_identities = []\n",
            )
            .unwrap();
        } else {
            fixture.script("age", "printf 'v1.2.1\\n'\n");
        }
        let output = fixture.run(&["--label", "Work Mac"], false, TAG);
        assert!(!output.status.success());
        assert!(!fixture.root.join("calls").exists());
        fixture.assert_config_unchanged();
    }
}
