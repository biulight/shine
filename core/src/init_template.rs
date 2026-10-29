use anyhow::{Context, Result, bail};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Write a `shine.toml` preset-metadata template into `dir`, refusing to
/// overwrite an existing file unless `force` is set.
///
/// Returns the written path and whether an existing file was overwritten.
pub fn write_shine_toml_template(
    dir: &Path,
    force: bool,
    template: &str,
) -> Result<(PathBuf, bool)> {
    let path = dir.join("shine.toml");
    if !force {
        // Enforce no-overwrite at creation, including dangling links and late arrivals.
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                bail!("shine.toml already exists; use --force to overwrite");
            }
            Err(error) => return Err(error).with_context(|| format!("writing {}", path.display())),
        };
        file.write_all(template.as_bytes())
            .with_context(|| format!("writing {}", path.display()))?;
        return Ok((path, false));
    }
    let exists = path.exists();
    std::fs::write(&path, template).with_context(|| format!("writing {}", path.display()))?;
    Ok((path, exists))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Directory(PathBuf);

    impl Directory {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("shine-template-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn template_creation_preserves_dangling_symlink_and_absent_referent() {
        let directory = Directory::new();
        let path = directory.0.join("shine.toml");
        let referent = directory.0.join("unrelated.toml");
        std::os::unix::fs::symlink(&referent, &path).unwrap();

        let error = write_shine_toml_template(&directory.0, false, "template").unwrap_err();

        assert_eq!(
            error.to_string(),
            "shine.toml already exists; use --force to overwrite"
        );
        assert_eq!(std::fs::read_link(&path).unwrap(), referent);
        assert!(!referent.exists());
    }

    #[test]
    fn concurrent_template_creation_preserves_the_only_winner() {
        let directory = Directory::new();
        let barrier = std::sync::Barrier::new(8);
        let results = std::thread::scope(|scope| {
            let handles = (0..8)
                .map(|index| {
                    let root = &directory.0;
                    let barrier = &barrier;
                    scope.spawn(move || {
                        let template = format!("writer = {index}\n").repeat(1024);
                        barrier.wait();
                        (
                            template.clone(),
                            write_shine_toml_template(root, false, &template),
                        )
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>()
        });
        let winners = results
            .iter()
            .filter(|(_, result)| result.is_ok())
            .collect::<Vec<_>>();
        assert_eq!(winners.len(), 1);
        assert_eq!(
            winners[0].1.as_ref().unwrap(),
            &(directory.0.join("shine.toml"), false)
        );
        assert_eq!(
            std::fs::read_to_string(directory.0.join("shine.toml")).unwrap(),
            winners[0].0
        );
        for (_, result) in results.iter().filter(|(_, result)| result.is_err()) {
            assert_eq!(
                result.as_ref().unwrap_err().to_string(),
                "shine.toml already exists; use --force to overwrite"
            );
        }
    }
}
