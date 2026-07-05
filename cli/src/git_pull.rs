use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result, bail};
use tokio::process::Command;

use crate::colors;
use crate::config::Config;

#[derive(Debug, Eq, PartialEq)]
struct PullTarget {
    label: &'static str,
    path: PathBuf,
}

/// Pull each Git repository that contributes to the effective preset namespace.
pub async fn handle_pull(config: &Config) -> Result<()> {
    let targets = configured_targets(config);
    let mut repositories = Vec::new();
    let mut seen = HashSet::new();

    for target in targets {
        match repository_root(&target.path).await? {
            Some(root) if seen.insert(root.clone()) => repositories.push((target.label, root)),
            Some(root) => println!(
                "  {} {} ({})",
                colors::dim("skipped"),
                target.label,
                colors::dim(&format!("same repository: {}", root.display()))
            ),
            None => println!(
                "  {} {} ({})",
                colors::dim("skipped"),
                target.label,
                colors::dim(&format!("not a Git repository: {}", target.path.display()))
            ),
        }
    }

    if repositories.is_empty() {
        println!("{}", colors::dim("Nothing to pull."));
        return Ok(());
    }

    // Validate every repository before changing any of them.
    for (label, root) in &repositories {
        ensure_clean(label, root).await?;
        ensure_tracking_branch(label, root).await?;
    }

    for (label, root) in repositories {
        println!("Pulling {label} from {} ...", root.display());
        pull_ff_only(&root).await?;
        println!("{}", colors::green(&format!("  {label} updated")));
    }

    Ok(())
}

fn configured_targets(config: &Config) -> Vec<PullTarget> {
    let mut targets = vec![PullTarget {
        label: "preset source",
        path: config.presets_dir().to_path_buf(),
    }];
    if let Some(path) = config.active_presets_overlay_dir() {
        targets.push(PullTarget {
            label: "overlay source",
            path: path.to_path_buf(),
        });
    }
    targets
}

async fn repository_root(path: &Path) -> Result<Option<PathBuf>> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(path)
        .output()
        .await
        .with_context(|| "failed to run git; install Git and ensure it is available in PATH")?;

    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        if detail.contains("not a git repository") {
            return Ok(None);
        }
        bail!(
            "failed to inspect Git repository at {}: {}",
            path.display(),
            detail.trim()
        );
    }

    let root =
        String::from_utf8(output.stdout).context("git returned a non-UTF-8 repository path")?;
    let root = root.trim();
    if root.is_empty() {
        bail!(
            "git returned an empty repository root for {}",
            path.display()
        );
    }
    Ok(Some(PathBuf::from(root)))
}

async fn ensure_clean(label: &str, root: &Path) -> Result<()> {
    let output = git_output(root, &["status", "--porcelain=v1", "--untracked-files=all"]).await?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        bail!(
            "failed to inspect {label} repository {}: {}",
            root.display(),
            detail.trim()
        );
    }
    if !output.stdout.is_empty() {
        bail!(
            "refusing to pull {label}: Git worktree has uncommitted changes: {}\nCommit, stash, or discard the changes, then run 'shine pull' again.",
            root.display()
        );
    }
    Ok(())
}

async fn ensure_tracking_branch(label: &str, root: &Path) -> Result<()> {
    let branch = git_output(root, &["symbolic-ref", "--quiet", "--short", "HEAD"]).await?;
    if !branch.status.success() {
        bail!(
            "refusing to pull {label}: repository is in detached HEAD state: {}",
            root.display()
        );
    }

    let upstream = git_output(
        root,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    )
    .await?;
    if !upstream.status.success() {
        let branch = String::from_utf8_lossy(&branch.stdout);
        bail!(
            "refusing to pull {label}: branch '{}' has no upstream: {}",
            branch.trim(),
            root.display()
        );
    }
    Ok(())
}

async fn pull_ff_only(root: &Path) -> Result<()> {
    let status = Command::new("git")
        .args(["pull", "--ff-only"])
        .current_dir(root)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .await
        .with_context(|| "failed to run git; install Git and ensure it is available in PATH")?;
    if !status.success() {
        bail!(
            "Git pull failed in {} (status {}). Resolve the Git error, then run 'shine pull' again.",
            root.display(),
            status
        );
    }
    Ok(())
}

async fn git_output(root: &Path, args: &[&str]) -> Result<std::process::Output> {
    Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .await
        .with_context(|| "failed to run git; install Git and ensure it is available in PATH")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as StdCommand;

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("shine-git-pull-{name}-{}", uuid::Uuid::new_v4()))
    }

    fn git(dir: &Path, args: &[&str]) {
        let output = StdCommand::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_repo(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        git(dir, &["init"]);
        git(dir, &["config", "user.name", "Shine Tests"]);
        git(dir, &["config", "user.email", "shine@example.invalid"]);
        std::fs::write(dir.join("preset.txt"), "one\n").unwrap();
        git(dir, &["add", "preset.txt"]);
        git(dir, &["commit", "-m", "initial"]);
    }

    #[test]
    fn configured_targets_are_ordered_preset_then_overlay() {
        let dir = std::env::temp_dir().join("shine-pull-targets");
        let config =
            Config::new_for_test(&dir).with_presets_overlay_dir_override(Some(dir.join("overlay")));
        assert_eq!(
            configured_targets(&config),
            vec![
                PullTarget {
                    label: "preset source",
                    path: dir.join("presets"),
                },
                PullTarget {
                    label: "overlay source",
                    path: dir.join("overlay"),
                },
            ]
        );
    }

    #[tokio::test]
    async fn non_git_directory_has_nothing_to_pull() {
        let root = temp_dir("non-git");
        std::fs::create_dir_all(root.join("presets")).unwrap();
        let config = Config::new_for_test(&root);

        handle_pull(&config).await.unwrap();

        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn dirty_worktree_is_rejected_before_pull() {
        let root = temp_dir("dirty");
        let presets = root.join("presets");
        init_repo(&presets);
        std::fs::write(presets.join("local.txt"), "dirty\n").unwrap();
        let config = Config::new_for_test(&root);

        let error = handle_pull(&config).await.unwrap_err();

        assert!(error.to_string().contains("uncommitted changes"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn branch_without_upstream_is_rejected() {
        let root = temp_dir("no-upstream");
        let presets = root.join("presets");
        init_repo(&presets);
        let config = Config::new_for_test(&root);

        let error = handle_pull(&config).await.unwrap_err();

        assert!(error.to_string().contains("has no upstream"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn pulls_fast_forward_from_local_remote() {
        let root = temp_dir("fast-forward");
        let remote = root.join("remote.git");
        let seed = root.join("seed");
        let presets = root.join("presets");
        std::fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "--bare", remote.to_str().unwrap()]);
        git(
            &root,
            &["clone", remote.to_str().unwrap(), seed.to_str().unwrap()],
        );
        git(&seed, &["config", "user.name", "Shine Tests"]);
        git(&seed, &["config", "user.email", "shine@example.invalid"]);
        std::fs::write(seed.join("preset.txt"), "one\n").unwrap();
        git(&seed, &["add", "preset.txt"]);
        git(&seed, &["commit", "-m", "initial"]);
        git(&seed, &["push", "-u", "origin", "HEAD"]);
        git(
            &root,
            &["clone", remote.to_str().unwrap(), presets.to_str().unwrap()],
        );
        std::fs::write(seed.join("preset.txt"), "two\n").unwrap();
        git(&seed, &["add", "preset.txt"]);
        git(&seed, &["commit", "-m", "update"]);
        git(&seed, &["push"]);
        let config = Config::new_for_test(&root);

        handle_pull(&config).await.unwrap();

        assert_eq!(
            std::fs::read_to_string(presets.join("preset.txt")).unwrap(),
            "two\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn detached_head_is_rejected() {
        let root = temp_dir("detached");
        let presets = root.join("presets");
        init_repo(&presets);
        git(&presets, &["checkout", "--detach"]);
        let config = Config::new_for_test(&root);

        let error = handle_pull(&config).await.unwrap_err();

        assert!(error.to_string().contains("detached HEAD"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
