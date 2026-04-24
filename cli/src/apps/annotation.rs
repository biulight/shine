use crate::config::Config;
use anyhow::{Context, Result, bail};
use std::path::PathBuf;

pub(crate) fn resolve_destination(
    annotation: Option<&str>,
    category: &str,
    filename: &str,
    config: &Config,
) -> Result<PathBuf> {
    let raw = match annotation {
        Some(ann) => ann.to_string(),
        None => {
            let root = config.app_default_dest_root();
            format!("{}/{}/{}", root.display(), category, filename)
        }
    };

    let expanded = shellexpand::full(&raw)
        .with_context(|| format!("failed to expand destination path: {raw}"))?
        .to_string();

    let path = PathBuf::from(&expanded);

    if !path.is_absolute() {
        bail!("destination path must be absolute after expansion, got: {expanded}");
    }

    if path
        .components()
        .any(|c| c == std::path::Component::ParentDir)
    {
        bail!("destination path must not contain '..': {expanded}");
    }

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::path::Path;

    fn test_config(home: &Path) -> Config {
        Config::new_for_test(home)
    }

    #[test]
    fn resolves_annotation_with_tilde() {
        let home = std::env::temp_dir().join("shine-ann-test");
        let config = test_config(&home);
        let result =
            resolve_destination(Some("~/.config/foo/bar.toml"), "foo", "bar.toml", &config)
                .unwrap();
        assert!(result.is_absolute());
        assert!(result.ends_with("bar.toml"));
    }

    #[test]
    fn resolves_absolute_annotation() {
        let home = std::env::temp_dir().join("shine-ann-test");
        let config = test_config(&home);
        let dest = format!("{}/myapp/config.toml", home.display());
        let result = resolve_destination(Some(&dest), "myapp", "config.toml", &config).unwrap();
        assert_eq!(result, PathBuf::from(&dest));
    }

    #[test]
    fn falls_back_to_default_dest_root_when_no_annotation() {
        let home = std::env::temp_dir().join("shine-ann-fallback");
        let config = test_config(&home);
        let result = resolve_destination(None, "starship", "starship.toml", &config).unwrap();
        // fallback is home/.config/starship/starship.toml
        assert!(result.ends_with("starship/starship.toml"));
    }

    #[test]
    fn rejects_path_with_parent_dir_component() {
        let home = std::env::temp_dir().join("shine-ann-sec");
        let config = test_config(&home);
        let dest = format!("{}/../etc/passwd", home.display());
        assert!(resolve_destination(Some(&dest), "x", "y", &config).is_err());
    }

    #[test]
    fn rejects_non_absolute_path_after_expansion() {
        let home = std::env::temp_dir().join("shine-ann-rel");
        let config = test_config(&home);
        assert!(resolve_destination(Some("relative/path"), "x", "y", &config).is_err());
    }
}
