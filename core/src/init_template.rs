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

    const ALREADY_EXISTS: &str = "shine.toml already exists; use --force to overwrite";

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

        assert_eq!(error.to_string(), ALREADY_EXISTS);
        assert_eq!(std::fs::read_link(&path).unwrap(), referent);
        assert!(!referent.exists());
    }

    #[test]
    fn concurrent_template_creation_preserves_the_only_winner() {
        const WRITERS: usize = 8;
        let directory = Directory::new();
        let barrier = std::sync::Barrier::new(WRITERS);
        let path = directory.0.join("shine.toml");
        let mut winners = Vec::new();
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for index in 0..WRITERS {
                let root = &directory.0;
                let barrier = &barrier;
                handles.push(scope.spawn(move || {
                    let template = format!("writer = {index}\n").repeat(1024);
                    barrier.wait();
                    let result = write_shine_toml_template(root, false, &template);
                    (template, result)
                }));
            }
            for handle in handles {
                let (template, result) = handle.join().unwrap();
                match result {
                    Ok((written, overwritten)) => {
                        assert_eq!(written, path);
                        assert!(!overwritten);
                        winners.push(template);
                    }
                    Err(error) => assert_eq!(error.to_string(), ALREADY_EXISTS),
                }
            }
        });
        assert_eq!(winners.len(), 1);
        assert_eq!(std::fs::read_to_string(path).unwrap(), winners[0]);
    }
}
