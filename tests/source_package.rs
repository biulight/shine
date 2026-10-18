//! Exercise Cargo's include matching, including ignored dependency-tree sentinels.

use std::{fs, path::PathBuf, process::Command};

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn source_package_keeps_required_files_without_dependency_trees() {
    let manifest: toml::Value = toml::from_str(include_str!("../Cargo.toml")).unwrap();
    let include = manifest["package"]["include"].clone();
    let fixture = Fixture(
        std::env::temp_dir().join(format!("shine-package-contract-{}", uuid::Uuid::new_v4(),)),
    );
    fs::create_dir_all(&fixture.0).unwrap();
    // Reuse the real include policy, but avoid network, workspace and build dependencies.
    let package = toml::toml! {
        [package]
        name = "shine-package-contract"
        version = "0.0.0"
        edition = "2024"
        license = "MIT"
        readme = "README.md"
        [lib]
        path = "cli/src/lib.rs"
        [workspace]
    };
    let mut package = toml::Value::Table(package);
    package["package"]
        .as_table_mut()
        .unwrap()
        .insert("include".into(), include);
    fs::write(
        fixture.0.join("Cargo.toml"),
        toml::to_string(&package).unwrap(),
    )
    .unwrap();
    let required = [
        "README.md",
        "LICENSE",
        "cli/build.rs",
        "cli/src/lib.rs",
        "presets/app/demo/shine.toml",
        "presets/shell/demo/run.ts",
        "skills/shine-preset-author/SKILL.md",
        "tests/builtin_presets.rs",
        "tests/builtin_presets/support.rs",
        "tests/source_package.rs",
    ];
    let unwanted = [
        "node_modules/demo/README.md",
        "node_modules/demo/LICENSE",
        "website/node_modules/demo/README.md",
        "website/node_modules/demo/LICENSE",
        "presets/app/demo/node_modules/dependency/index.ts",
        "cli/src/node_modules/dependency/README.md",
        "tests/builtin_presets/node_modules/dependency/LICENSE",
        "skills/demo/node_modules/dependency/README.md",
        "unrelated/README.md",
        "unrelated/LICENSE",
    ];
    for path in required.iter().chain(&unwanted) {
        let path = fixture.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "// fixture\n").unwrap();
    }
    let output = Command::new(env!("CARGO"))
        .args([
            "package",
            "--list",
            "--allow-dirty",
            "--offline",
            "--manifest-path",
        ])
        .arg(fixture.0.join("Cargo.toml"))
        .current_dir(&fixture.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap().replace('\\', "/");
    let paths: std::collections::BTreeSet<_> = stdout.lines().collect();
    for path in required {
        assert!(
            paths.contains(path),
            "missing required package member: {path}\n{stdout}"
        );
    }
    for path in unwanted {
        assert!(
            !paths.contains(path),
            "unexpected package member: {path}\n{stdout}"
        );
    }
    assert!(
        !paths
            .iter()
            .any(|path| path.split('/').any(|part| part == "node_modules"))
    );
}
