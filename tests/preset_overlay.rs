//! Overlay scope and persistence through separate CLI invocations.
use std::{fs, path::PathBuf, process::Command};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("shine-overlay-scope-{}", uuid::Uuid::new_v4()));
        for dir in ["state", "outside", "project/child", "manual", "remote"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        fs::write(root.join("state/config.toml"), "schema_version = 2\n").unwrap();
        fs::write(root.join("gitconfig"), "").unwrap();
        fs::write(
            root.join("project/shine.config.toml"),
            "# project settings\n",
        )
        .unwrap();
        let fixture = Self(root);
        fixture.git(&["init", "-q"]);
        fixture.git(&["config", "user.name", "Fixture"]);
        fixture.git(&["config", "user.email", "fixture@example.invalid"]);
        fs::write(fixture.0.join("remote/marker"), "original\n").unwrap();
        fixture.git(&["add", "marker"]);
        fixture.git(&["commit", "-qm", "fixture"]);
        fixture
    }

    fn git(&self, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(self.0.join("remote"))
            .env("GIT_CONFIG_GLOBAL", self.0.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }

    fn command(&self, project: bool) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_shine"));
        command
            .current_dir(
                self.0
                    .join(if project { "project/child" } else { "outside" }),
            )
            .env("SHINE_CONFIG_DIR", self.0.join("state"))
            .env_remove("SHINE_PRESETS")
            .env("GIT_CONFIG_GLOBAL", self.0.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1");
        command
    }

    fn link_git(&self, project: bool) -> std::process::Output {
        self.command(project)
            .args(["preset", "overlay", "link", "--git"])
            .arg(self.0.join("remote"))
            .output()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn project_git_overlay_rejection_does_not_initialize_global_state() {
    let fixture = Fixture::new();
    fs::remove_dir_all(fixture.0.join("state")).unwrap();
    let project = fs::read(fixture.0.join("project/shine.config.toml")).unwrap();
    let output = fixture.link_git(true);
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("global-only"),
        "{output:?}"
    );
    assert!(!fixture.0.join("state").exists());
    assert_eq!(
        fs::read(fixture.0.join("project/shine.config.toml")).unwrap(),
        project
    );
}

#[test]
fn project_git_overlay_link_preserves_configs_and_absent_or_existing_mirror() {
    let fixture = Fixture::new();
    for existing in [false, true] {
        if existing {
            let output = fixture.link_git(false);
            assert!(output.status.success(), "{output:?}");
            // This untracked file would be removed if a rejected project link synced the mirror.
            fs::write(fixture.0.join("state/overlay/local-edit"), "retain\n").unwrap();
        }
        let global = fs::read(fixture.0.join("state/config.toml")).unwrap();
        let project = fs::read(fixture.0.join("project/shine.config.toml")).unwrap();
        let output = fixture.link_git(true);
        assert!(!output.status.success(), "{output:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("global-only"), "{output:?}");
        assert!(error.contains("outside"), "{output:?}");
        assert_eq!(
            fs::read(fixture.0.join("state/config.toml")).unwrap(),
            global
        );
        assert_eq!(
            fs::read(fixture.0.join("project/shine.config.toml")).unwrap(),
            project
        );
        if existing {
            assert_eq!(
                fs::read(fixture.0.join("state/overlay/local-edit")).unwrap(),
                b"retain\n"
            );
        } else {
            assert!(!fixture.0.join("state/overlay").exists());
        }
    }
}

#[test]
fn global_git_overlay_survives_reload_and_pull_and_project_manual_link_still_works() {
    let fixture = Fixture::new();
    let output = fixture.link_git(false);
    assert!(output.status.success(), "{output:?}");
    for project in [false, true] {
        let output = fixture
            .command(project)
            .args(["preset", "overlay", "info"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("Overlay Git source:"),
            "{output:?}"
        );
    }
    fs::write(fixture.0.join("remote/marker"), "updated\n").unwrap();
    fixture.git(&["commit", "-qam", "update"]);
    let output = fixture
        .command(false)
        .args(["preset", "pull"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        fs::read(fixture.0.join("state/overlay/marker")).unwrap(),
        b"updated\n"
    );

    let global = fs::read(fixture.0.join("state/config.toml")).unwrap();
    let output = fixture
        .command(true)
        .args(["preset", "overlay", "link"])
        .arg(fixture.0.join("manual"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let saved: toml::Table =
        toml::from_str(&fs::read_to_string(fixture.0.join("project/shine.config.toml")).unwrap())
            .unwrap();
    assert!(saved.contains_key("presets_overlay_dir"));
    assert!(!saved.contains_key("presets_overlay_git"));
    assert_eq!(
        fs::read(fixture.0.join("state/config.toml")).unwrap(),
        global
    );

    fs::write(
        fixture.0.join("manual/shine.env.toml"),
        "OVERLAY_SOURCE = 'project-local'\n",
    )
    .unwrap();
    let project_config = fs::read(fixture.0.join("project/shine.config.toml")).unwrap();
    let info = fixture
        .command(true)
        .args(["preset", "overlay", "info"])
        .output()
        .unwrap();
    assert_eq!(info.status.code(), Some(0), "{info:?}");
    assert!(info.stderr.is_empty(), "{info:?}");
    let stdout = String::from_utf8_lossy(&info.stdout);
    assert!(stdout.contains("manual"), "{info:?}");
    assert!(stdout.contains("Active"), "{info:?}");
    assert!(!stdout.contains("Overlay Git source:"), "{info:?}");
    let value = fixture
        .command(true)
        .args(["env", "get", "OVERLAY_SOURCE"])
        .output()
        .unwrap();
    assert_eq!(value.status.code(), Some(0), "{value:?}");
    assert_eq!(value.stdout, b"project-local\n");
    assert!(value.stderr.is_empty(), "{value:?}");
    assert_eq!(
        fs::read(fixture.0.join("project/shine.config.toml")).unwrap(),
        project_config
    );
    assert_eq!(
        fs::read(fixture.0.join("state/config.toml")).unwrap(),
        global
    );
    assert_eq!(
        fs::read(fixture.0.join("state/overlay/marker")).unwrap(),
        b"updated\n"
    );
}
