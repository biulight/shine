use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result, bail};
use tokio::process::Command;

use crate::colors;
use crate::config::Config;
use crate::output::escape_controls;

// Keep authentication/transport settings, but never let inherited repository
// selectors redirect commands away from the explicitly selected checkout.
fn git_command() -> Command {
    let mut command = Command::new("git");
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_PREFIX",
        "GIT_NAMESPACE",
        "GIT_CEILING_DIRECTORIES",
        "GIT_DISCOVERY_ACROSS_FILESYSTEM",
        "GIT_CONFIG",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
    ] {
        command.env_remove(name);
    }
    command
}

#[derive(Debug, Eq, PartialEq)]
struct PullTarget {
    label: &'static str,
    path: PathBuf,
}

/// Pull each Git repository that contributes to the effective preset namespace.
pub async fn handle_pull(config: &Config, verbose: bool) -> Result<()> {
    let overlay_git = config.overlay_git_source();

    let targets = configured_targets(config);
    let mut repositories = Vec::new();
    let mut seen = HashSet::new();

    for target in targets {
        match repository_root(&target.path).await? {
            Some(root) if seen.insert(root.clone()) => repositories.push((target.label, root)),
            Some(root) if verbose => println!(
                "  {} {} ({})",
                colors::dim("skipped"),
                target.label,
                colors::dim(&format!(
                    "same repository: {}",
                    escape_controls(&root.display().to_string())
                ))
            ),
            Some(_) => {}
            None if verbose => println!(
                "  {} {} ({})",
                colors::dim("skipped"),
                target.label,
                colors::dim(&format!(
                    "not a Git repository: {}",
                    escape_controls(&target.path.display().to_string())
                ))
            ),
            None => {}
        }
    }

    if overlay_git.is_none() && repositories.is_empty() {
        println!("{}", colors::dim("Nothing to pull."));
        return Ok(());
    }

    // Validate every fast-forward repository before changing any of them.
    for (label, root) in &repositories {
        ensure_clean(label, root).await?;
        ensure_tracking_branch(label, root).await?;
    }

    println!("{}", colors::bold("Preset Sources"));

    // The shine-managed Git overlay syncs first: cloned on first use, then
    // force-mirrored to the remote tip on every subsequent pull.
    if let Some((url, branch, dir)) = overlay_git {
        sync_managed_overlay(url, branch, dir, verbose).await?;
    }

    for (label, root) in repositories {
        if verbose {
            println!(
                "Pulling {label} from {} ...",
                escape_controls(&root.display().to_string())
            );
        }
        let summary = pull_ff_only(&root, verbose).await?;
        print_pull_summary(label, &summary);
    }

    Ok(())
}

fn configured_targets(config: &Config) -> Vec<PullTarget> {
    let mut targets = vec![PullTarget {
        label: "preset source",
        path: config.presets_dir().to_path_buf(),
    }];
    // Only a *manually linked* overlay is fast-forward-pulled here. A
    // shine-managed Git overlay (`presets_overlay_git`) is force-mirrored
    // separately by `sync_managed_overlay` and must never go through the
    // clean-worktree / fast-forward path.
    if let Some(path) = config.presets_overlay_dir_override.as_deref() {
        targets.push(PullTarget {
            label: "overlay source",
            path: path.to_path_buf(),
        });
    }
    targets
}

fn git_path_output(stdout: Vec<u8>) -> Result<PathBuf> {
    let path = String::from_utf8(stdout)?;
    // Remove only Git's record terminator; whitespace can belong to the path.
    let Some(path) = path.strip_suffix('\n') else {
        return Ok(PathBuf::from(path));
    };
    #[cfg(windows)]
    let path = path.strip_suffix('\r').unwrap_or(path);
    Ok(PathBuf::from(path))
}

async fn repository_root(path: &Path) -> Result<Option<PathBuf>> {
    let output = git_command()
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
            escape_controls(&path.display().to_string()),
            detail.trim()
        );
    }

    let root =
        git_path_output(output.stdout).context("git returned a non-UTF-8 repository path")?;
    if root.as_os_str().is_empty() {
        bail!(
            "git returned an empty repository root for {}",
            escape_controls(&path.display().to_string())
        );
    }
    Ok(Some(root))
}

async fn ensure_clean(label: &str, root: &Path) -> Result<()> {
    let output = git_output(root, &["status", "--porcelain=v1", "--untracked-files=all"]).await?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        bail!(
            "failed to inspect {label} repository {}: {}",
            escape_controls(&root.display().to_string()),
            detail.trim()
        );
    }
    if !output.stdout.is_empty() {
        bail!(
            "refusing to pull {label}: Git worktree has uncommitted changes: {}\nCommit, stash, or discard the changes, then run 'shine preset pull' again.",
            escape_controls(&root.display().to_string())
        );
    }
    Ok(())
}

async fn ensure_tracking_branch(label: &str, root: &Path) -> Result<()> {
    let branch = git_output(root, &["symbolic-ref", "--quiet", "--short", "HEAD"]).await?;
    if !branch.status.success() {
        bail!(
            "refusing to pull {label}: repository is in detached HEAD state: {}",
            escape_controls(&root.display().to_string())
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
            escape_controls(&root.display().to_string())
        );
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct PullSummary {
    before: String,
    after: String,
    shortstat: Option<String>,
}

impl PullSummary {
    fn updated(&self) -> bool {
        self.before != self.after
    }
}

async fn pull_ff_only(root: &Path, verbose: bool) -> Result<PullSummary> {
    let before = head_short(root).await?;
    let mut command = git_command();
    command
        .args(["pull", "--ff-only"])
        .current_dir(root)
        .stdin(Stdio::inherit());
    let failure_detail = if verbose {
        let status = command
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .await
            .with_context(|| "failed to run git; install Git and ensure it is available in PATH")?;
        if status.success() {
            None
        } else {
            Some(format!("status {status}"))
        }
    } else {
        let output = command
            .output()
            .await
            .with_context(|| "failed to run git; install Git and ensure it is available in PATH")?;
        if output.status.success() {
            None
        } else {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            Some(
                [stdout.trim(), stderr.trim()]
                    .into_iter()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        }
    };
    if let Some(detail) = failure_detail {
        bail!(
            "Git pull failed in {}: {}\nResolve the Git error, then run 'shine preset pull' again.",
            root.display(),
            detail
        );
    }

    let after = head_short(root).await?;
    let shortstat = if before == after {
        None
    } else {
        let output = git_output(root, &["diff", "--shortstat", &before, &after]).await?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
    .filter(|stat| !stat.is_empty());
    Ok(PullSummary {
        before,
        after,
        shortstat,
    })
}

async fn head_short(root: &Path) -> Result<String> {
    let output = git_output(root, &["rev-parse", "--short=7", "HEAD"]).await?;
    if !output.status.success() {
        bail!("failed to resolve Git HEAD in {}", root.display());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn print_pull_summary(label: &str, summary: &PullSummary) {
    if summary.updated() {
        let stat = summary
            .shortstat
            .as_deref()
            .map(|value| format!("  {}", colors::dim(value)))
            .unwrap_or_default();
        println!(
            "  {}  {} updated  {} → {}{}",
            colors::symbol("✓"),
            label,
            summary.before,
            summary.after,
            stat
        );
    } else {
        println!(
            "  {}  {} {}",
            colors::symbol("✓"),
            label,
            colors::dim("up-to-date")
        );
    }
}

async fn git_output(root: &Path, args: &[&str]) -> Result<std::process::Output> {
    git_command()
        .args(args)
        .current_dir(root)
        .output()
        .await
        .with_context(|| "failed to run git; install Git and ensure it is available in PATH")
}

/// Sync the shine-managed Git overlay. Cloned `--depth 1` on first use, then
/// force-mirrored to the remote tip on every subsequent call.
///
/// The managed overlay is a read-only mirror, so this deliberately discards any
/// local edits. Two invariants keep the *previous* checkout usable when a sync
/// fails: the fetch runs before any local reset (a network failure never touches
/// the working tree), and a first-time clone lands via a temp dir + atomic
/// rename (a failed clone never leaves a half-populated overlay dir).
pub(crate) async fn sync_managed_overlay(
    url: &str,
    branch: Option<&str>,
    dir: &Path,
    verbose: bool,
) -> Result<()> {
    match tokio::fs::symlink_metadata(dir).await {
        Ok(_) => mirror_managed_overlay(url, branch, dir, verbose).await,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            clone_managed_overlay(url, branch, dir, verbose).await
        }
        Err(error) => Err(error).context("inspecting managed overlay directory"),
    }
}

async fn ensure_managed_checkout(dir: &Path) -> Result<()> {
    // A worktree's .git file can reference another repository. Only a standalone,
    // non-linked checkout is disposable; an ordinary directory can discover its
    // parent's repository, and a symlink can point at unrelated user files.
    for path in [dir.to_path_buf(), dir.join(".git")] {
        let metadata = tokio::fs::symlink_metadata(&path)
            .await
            .context("managed overlay must be a standalone Git checkout")?;
        if !metadata.is_dir() {
            bail!("managed overlay must be a standalone Git checkout without linked directories");
        }
    }
    let expected = tokio::fs::canonicalize(dir).await?;
    let root = repository_root(dir)
        .await?
        .context("managed overlay is not a Git checkout")?;
    let git_dir = git_output(dir, &["rev-parse", "--absolute-git-dir"]).await?;
    if !git_dir.status.success()
        || tokio::fs::canonicalize(root).await? != expected
        || tokio::fs::canonicalize(
            git_path_output(git_dir.stdout).context("Git returned a non-UTF-8 metadata path")?,
        )
        .await?
            != expected.join(".git")
    {
        bail!("managed overlay Git repository escapes its directory");
    }
    Ok(())
}

async fn clone_managed_overlay(
    url: &str,
    branch: Option<&str>,
    dir: &Path,
    verbose: bool,
) -> Result<()> {
    let parent = dir
        .parent()
        .context("managed overlay path has no parent directory")?;
    tokio::fs::create_dir_all(parent)
        .await
        .with_context(|| format!("failed to create {}", parent.display()))?;

    let temp = temp_clone_path(dir)?;
    let temp_arg = temp
        .to_str()
        .context("managed overlay clone path is not valid UTF-8")?;
    if temp.exists() {
        tokio::fs::remove_dir_all(&temp)
            .await
            .with_context(|| format!("failed to remove stale clone dir {}", temp.display()))?;
    }

    let mut args: Vec<&str> = vec!["clone", "--depth", "1"];
    if let Some(branch) = branch {
        args.push("--branch");
        args.push(branch);
    }
    args.push(url);
    args.push(temp_arg);

    if let Err(err) = run_git(parent, &args, verbose, "clone").await {
        let _ = tokio::fs::remove_dir_all(&temp).await;
        return Err(err);
    }

    tokio::fs::rename(&temp, dir).await.with_context(|| {
        format!(
            "failed to move cloned overlay into place at {}",
            dir.display()
        )
    })?;

    let after = head_short(dir).await?;
    println!("  {}  overlay source cloned  {after}", colors::symbol("✓"));
    Ok(())
}

async fn mirror_managed_overlay(
    url: &str,
    branch: Option<&str>,
    dir: &Path,
    verbose: bool,
) -> Result<()> {
    ensure_managed_checkout(dir).await?;
    // The configured source is authoritative, even after relinking an existing
    // checkout. HEAD follows the remote default rather than an old local branch.
    let branch = branch.unwrap_or("HEAD");
    let before = head_short(dir).await?;

    // Fetch first, reset only on success: an unreachable remote leaves the
    // existing checkout intact and usable.
    run_git(
        dir,
        &["fetch", "--depth", "1", "--", url, branch],
        verbose,
        "fetch",
    )
    .await
    .with_context(|| format!("failed to fetch managed overlay from {url}"))?;
    ensure_managed_checkout(dir).await?;
    run_git(
        dir,
        &["remote", "set-url", "origin", url],
        verbose,
        "set-url",
    )
    .await?;
    run_git(dir, &["reset", "--hard", "FETCH_HEAD"], verbose, "reset").await?;
    // This verified standalone checkout is a disposable mirror. Include ignored
    // files and nested repositories so stale local overrides cannot remain active.
    run_git(dir, &["clean", "-ffdx"], verbose, "clean").await?;

    let after = head_short(dir).await?;
    let shortstat = if before == after {
        None
    } else {
        let output = git_output(dir, &["diff", "--shortstat", &before, &after]).await?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
    .filter(|stat| !stat.is_empty());
    print_pull_summary(
        "overlay source",
        &PullSummary {
            before,
            after,
            shortstat,
        },
    );
    Ok(())
}

/// Sibling temp path used to stage a fresh clone before atomically renaming it
/// into place, so a failed clone never clobbers an existing good checkout.
fn temp_clone_path(dir: &Path) -> Result<PathBuf> {
    let name = dir
        .file_name()
        .context("managed overlay path has no final component")?;
    let mut tmp = name.to_os_string();
    tmp.push(".shine-clone-tmp");
    Ok(dir.with_file_name(tmp))
}

/// Run a git command, inheriting stdin and (when verbose) stdout/stderr, and
/// bail with captured output on a non-zero exit.
async fn run_git(cwd: &Path, args: &[&str], verbose: bool, action: &str) -> Result<()> {
    let mut command = git_command();
    command.args(args).current_dir(cwd).stdin(Stdio::inherit());
    let failure_detail = if verbose {
        let status = command
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .await
            .with_context(|| "failed to run git; install Git and ensure it is available in PATH")?;
        if status.success() {
            None
        } else {
            Some(format!("status {status}"))
        }
    } else {
        let output = command
            .output()
            .await
            .with_context(|| "failed to run git; install Git and ensure it is available in PATH")?;
        if output.status.success() {
            None
        } else {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            Some(
                [stdout.trim(), stderr.trim()]
                    .into_iter()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        }
    };
    if let Some(detail) = failure_detail {
        bail!("git {action} failed: {detail}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as StdCommand;

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("shine-git-pull-{name}-{}", uuid::Uuid::new_v4()))
    }

    fn read_text(path: impl AsRef<Path>) -> String {
        std::fs::read_to_string(path).unwrap().replace("\r\n", "\n")
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
    fn git_path_output_removes_only_the_record_terminator() {
        for (output, expected) in [
            ("repo\n", "repo"),
            ("repo", "repo"),
            ("repo \t\u{2003}\n", "repo \t\u{2003}"),
            ("\n", ""),
        ] {
            assert_eq!(
                git_path_output(output.as_bytes().to_vec()).unwrap(),
                PathBuf::from(expected)
            );
        }
        #[cfg(unix)]
        for path in ["repo\n", "repo\r", "repo\r\n"] {
            assert_eq!(
                git_path_output(format!("{path}\n").into_bytes()).unwrap(),
                PathBuf::from(path)
            );
        }
        #[cfg(windows)]
        assert_eq!(
            git_path_output(b"repo\r\n".to_vec()).unwrap(),
            PathBuf::from("repo")
        );
        assert!(git_path_output(vec![0xff, b'\n']).is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn repository_root_preserves_directory_whitespace() {
        let root = temp_dir("root-whitespace");
        for suffix in [" ", "\t", "\n", "\r", "\u{2003}"] {
            let dir = root.join(format!("selected{suffix}"));
            init_repo(&dir);

            assert_eq!(
                repository_root(&dir).await.unwrap(),
                Some(std::fs::canonicalize(&dir).unwrap()),
                "directory suffix: {suffix:?}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pull_updates_whitespace_path_without_touching_neighbor() {
        for verbose in [false, true] {
            let root = temp_dir("pull-whitespace");
            let source = root.join("source");
            let selected = root.join("selected ");
            let neighbor = root.join("selected");
            init_repo(&source);
            for dir in [&selected, &neighbor] {
                git(
                    &root,
                    &["clone", source.to_str().unwrap(), dir.to_str().unwrap()],
                );
            }
            let neighbor_head = head_short(&neighbor).await.unwrap();
            std::fs::write(source.join("preset.txt"), "two\n").unwrap();
            git(&source, &["commit", "-am", "update"]);
            let state = root.join("state");
            std::fs::create_dir_all(state.join("presets")).unwrap();
            let config = Config::new_for_test(&state)
                .with_presets_overlay_dir_override(Some(selected.clone()));

            handle_pull(&config, verbose).await.unwrap();

            assert_eq!(read_text(selected.join("preset.txt")), "two\n");
            assert_eq!(
                head_short(&selected).await.unwrap(),
                head_short(&source).await.unwrap()
            );
            assert_eq!(read_text(neighbor.join("preset.txt")), "one\n");
            assert_eq!(head_short(&neighbor).await.unwrap(), neighbor_head);
            assert!(!neighbor.join(".git/FETCH_HEAD").exists());
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pull_checks_selected_whitespace_path_before_fetch() {
        let root = temp_dir("dirty-whitespace");
        let source = root.join("source");
        let selected = root.join("selected ");
        let neighbor = root.join("selected");
        init_repo(&source);
        for dir in [&selected, &neighbor] {
            git(
                &root,
                &["clone", source.to_str().unwrap(), dir.to_str().unwrap()],
            );
        }
        std::fs::write(selected.join("preset.txt"), "user changes\n").unwrap();
        let state = root.join("state");
        std::fs::create_dir_all(state.join("presets")).unwrap();
        let config =
            Config::new_for_test(&state).with_presets_overlay_dir_override(Some(selected.clone()));

        let error = handle_pull(&config, false).await.unwrap_err();

        assert!(error.to_string().contains("uncommitted changes"));
        assert_eq!(read_text(selected.join("preset.txt")), "user changes\n");
        assert_eq!(read_text(neighbor.join("preset.txt")), "one\n");
        for dir in [&selected, &neighbor] {
            assert!(!dir.join(".git/FETCH_HEAD").exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn managed_overlay_preserves_whitespace_checkout_identity() {
        let root = temp_dir("managed-whitespace");
        let source = root.join("source");
        let selected = root.join("selected ");
        let neighbor = root.join("selected");
        init_repo(&source);
        init_repo(&neighbor);
        sync_managed_overlay(source.to_str().unwrap(), None, &selected, false)
            .await
            .unwrap();
        std::fs::write(source.join("preset.txt"), "two\n").unwrap();
        git(&source, &["commit", "-am", "update"]);
        std::fs::write(selected.join("untracked.txt"), "disposable\n").unwrap();
        std::fs::write(neighbor.join("untracked.txt"), "preserve\n").unwrap();
        let neighbor_head = head_short(&neighbor).await.unwrap();

        sync_managed_overlay(source.to_str().unwrap(), None, &selected, false)
            .await
            .unwrap();

        assert_eq!(read_text(selected.join("preset.txt")), "two\n");
        assert!(!selected.join("untracked.txt").exists());
        assert_eq!(read_text(neighbor.join("preset.txt")), "one\n");
        assert_eq!(read_text(neighbor.join("untracked.txt")), "preserve\n");
        assert_eq!(head_short(&neighbor).await.unwrap(), neighbor_head);
        assert!(!neighbor.join(".git/FETCH_HEAD").exists());
        std::fs::remove_dir_all(root).unwrap();
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

        handle_pull(&config, false).await.unwrap();

        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn rejected_pull_encodes_checkout_paths_without_fetching_or_changing_identity() {
        let root = temp_dir(if cfg!(unix) {
            "review\nFORGED\r\t\u{1b}中文"
        } else {
            "review中文"
        });
        let presets = root.join("presets");
        init_repo(&presets);
        let config = Config::new_for_test(&root);
        for reason in ["uncommitted changes", "has no upstream", "detached HEAD"] {
            if reason == "uncommitted changes" {
                std::fs::write(presets.join("local.txt"), "keep\n").unwrap();
            } else if reason == "has no upstream" {
                std::fs::remove_file(presets.join("local.txt")).unwrap();
            } else {
                git(&presets, &["checkout", "--detach"]);
            }
            let head = std::fs::read(presets.join(".git/HEAD")).unwrap();
            let error = handle_pull(&config, false).await.unwrap_err().to_string();
            assert!(error.contains(reason), "{error}");
            assert!(!error.contains(['\r', '\t', '\u{1b}']), "{error:?}");
            assert!(!error.contains("\nFORGED"), "{error:?}");
            if cfg!(unix) {
                assert!(error.contains(r"review\nFORGED\r\t\u{1b}中文"));
            }
            assert_eq!(std::fs::read(presets.join(".git/HEAD")).unwrap(), head);
            assert_eq!(read_text(presets.join("preset.txt")), "one\n");
            assert!(!presets.join(".git/FETCH_HEAD").exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn dirty_worktree_is_rejected_before_pull() {
        let root = temp_dir("dirty");
        let presets = root.join("presets");
        init_repo(&presets);
        std::fs::write(presets.join("local.txt"), "dirty\n").unwrap();
        let config = Config::new_for_test(&root);

        let error = handle_pull(&config, false).await.unwrap_err();

        assert!(error.to_string().contains("uncommitted changes"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn branch_without_upstream_is_rejected() {
        let root = temp_dir("no-upstream");
        let presets = root.join("presets");
        init_repo(&presets);
        let config = Config::new_for_test(&root);

        let error = handle_pull(&config, false).await.unwrap_err();

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

        handle_pull(&config, false).await.unwrap();

        assert_eq!(read_text(presets.join("preset.txt")), "two\n");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn detached_head_is_rejected() {
        let root = temp_dir("detached");
        let presets = root.join("presets");
        init_repo(&presets);
        git(&presets, &["checkout", "--detach"]);
        let config = Config::new_for_test(&root);

        let error = handle_pull(&config, false).await.unwrap_err();

        assert!(error.to_string().contains("detached HEAD"));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Create a bare remote with a single `overlay.txt` commit and return
    /// `(remote_path, seed_worktree, url)` for driving further commits.
    fn seed_remote(root: &Path) -> (PathBuf, PathBuf, String) {
        let remote = root.join("remote.git");
        let seed = root.join("seed");
        std::fs::create_dir_all(root).unwrap();
        git(root, &["init", "--bare", remote.to_str().unwrap()]);
        git(
            root,
            &["clone", remote.to_str().unwrap(), seed.to_str().unwrap()],
        );
        git(&seed, &["config", "user.name", "Shine Tests"]);
        git(&seed, &["config", "user.email", "shine@example.invalid"]);
        std::fs::write(seed.join("overlay.txt"), "one\n").unwrap();
        git(&seed, &["add", "overlay.txt"]);
        git(&seed, &["commit", "-m", "initial"]);
        git(&seed, &["push", "-u", "origin", "HEAD"]);
        let url = remote.to_string_lossy().into_owned();
        (remote, seed, url)
    }

    fn commit_count(dir: &Path) -> String {
        let output = StdCommand::new("git")
            .args(["rev-list", "--count", "HEAD"])
            .current_dir(dir)
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn managed_overlay_clone_rejects_unrepresentable_path_without_alias_writes() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let root = temp_dir("clone-native-path");
        std::fs::create_dir_all(&root).unwrap();
        let parent = root.join(OsString::from_vec(b"state-\xff".to_vec()));
        if let Err(error) = std::fs::create_dir(&parent) {
            eprintln!("filesystem cannot create non-UTF-8 fixture: {error}");
            std::fs::remove_dir_all(root).unwrap();
            return;
        }
        let alias = root.join("state-\u{fffd}");
        std::fs::create_dir(&alias).unwrap();
        std::fs::write(alias.join("preserve.txt"), "untouched").unwrap();
        let remote = root.join("remote");
        init_repo(&remote);
        let destination = parent.join("overlay");

        let error = sync_managed_overlay(remote.to_str().unwrap(), None, &destination, false)
            .await
            .unwrap_err();

        let children: Vec<_> = std::fs::read_dir(&alias)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(children, vec![OsString::from("preserve.txt")], "{error:#}");
        assert!(error.to_string().contains("not valid UTF-8"), "{error:#}");
        assert!(!destination.exists());
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 0);
        assert_eq!(read_text(alias.join("preserve.txt")), "untouched");
        assert_eq!(read_text(remote.join("preset.txt")), "one\n");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn managed_overlay_rejects_parent_repository_without_mutation() {
        let root = temp_dir("parent-boundary");
        let parent = root.join("project");
        let remote = root.join("remote");
        init_repo(&parent);
        init_repo(&remote);
        git(&parent, &["remote", "add", "origin", "unchanged-origin"]);
        let dir = parent.join("state/overlay");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(parent.join("preset.txt"), "user changes\n").unwrap();
        std::fs::write(dir.join("local.txt"), "preserve\n").unwrap();
        let before = head_short(&parent).await.unwrap();
        let error = sync_managed_overlay(remote.to_str().unwrap(), None, &dir, false)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("standalone Git checkout"));
        assert_eq!(head_short(&parent).await.unwrap(), before);
        assert_eq!(read_text(parent.join("preset.txt")), "user changes\n");
        assert_eq!(read_text(dir.join("local.txt")), "preserve\n");
        assert_eq!(
            git_output(&parent, &["remote", "get-url", "origin"])
                .await
                .unwrap()
                .stdout,
            b"unchanged-origin\n"
        );
        assert!(!parent.join(".git/FETCH_HEAD").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn managed_overlay_ignores_inherited_repository_environment() {
        const CHILD: &str = "SHINE_TEST_OVERLAY_GIT_ENV";
        if let Some(root) = std::env::var_os(CHILD) {
            let root = PathBuf::from(root);
            sync_managed_overlay(
                root.join("remote").to_str().unwrap(),
                None,
                &root.join("overlay"),
                false,
            )
            .await
            .unwrap();
            return;
        }
        let root = temp_dir("inherited-git");
        let remote = root.join("remote");
        let unrelated = root.join("unrelated");
        init_repo(&remote);
        init_repo(&unrelated);
        let dir = root.join("overlay");
        sync_managed_overlay(remote.to_str().unwrap(), None, &dir, false)
            .await
            .unwrap();
        let before = head_short(&unrelated).await.unwrap();
        std::fs::write(unrelated.join("preset.txt"), "unrelated user changes\n").unwrap();
        std::fs::write(remote.join("preset.txt"), "remote update\n").unwrap();
        git(&remote, &["commit", "-am", "update"]);
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "git_pull::tests::managed_overlay_ignores_inherited_repository_environment",
            ])
            .env(CHILD, &root)
            .env("GIT_DIR", unrelated.join(".git"))
            .env("GIT_WORK_TREE", &unrelated)
            .env("GIT_INDEX_FILE", unrelated.join(".git/index"))
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "core.worktree")
            .env("GIT_CONFIG_VALUE_0", &unrelated)
            .status()
            .await
            .unwrap();
        assert!(status.success());
        assert_eq!(read_text(dir.join("preset.txt")), "remote update\n");
        assert_eq!(
            read_text(unrelated.join("preset.txt")),
            "unrelated user changes\n"
        );
        assert_eq!(head_short(&unrelated).await.unwrap(), before);
        assert!(!unrelated.join(".git/FETCH_HEAD").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn managed_overlay_rejects_linked_checkout_and_metadata() {
        use std::os::unix::fs::symlink;
        let root = temp_dir("linked-checkout");
        let remote = root.join("remote");
        let user = root.join("user");
        init_repo(&remote);
        init_repo(&user);
        let dir = root.join("overlay");
        symlink(&user, &dir).unwrap();
        assert!(
            sync_managed_overlay(remote.to_str().unwrap(), None, &dir, false)
                .await
                .is_err()
        );
        std::fs::remove_file(&dir).unwrap();
        std::fs::create_dir(&dir).unwrap();
        symlink(user.join(".git"), dir.join(".git")).unwrap();
        assert!(
            sync_managed_overlay(remote.to_str().unwrap(), None, &dir, false)
                .await
                .is_err()
        );
        std::fs::remove_file(dir.join(".git")).unwrap();
        std::fs::write(
            dir.join(".git"),
            format!("gitdir: {}\n", user.join(".git").display()),
        )
        .unwrap();
        assert!(
            sync_managed_overlay(remote.to_str().unwrap(), None, &dir, false)
                .await
                .is_err()
        );
        assert_eq!(read_text(user.join("preset.txt")), "one\n");
        assert!(!user.join(".git/FETCH_HEAD").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn managed_overlay_cleans_untracked_ignored_and_nested_overrides_only_after_fetch() {
        let root = temp_dir("stale-overrides");
        let remote = root.join("remote");
        init_repo(&remote);
        std::fs::write(remote.join(".gitignore"), "ignored/\n").unwrap();
        git(&remote, &["add", ".gitignore"]);
        git(&remote, &["commit", "-m", "ignore generated files"]);
        let dir = root.join("overlay");
        sync_managed_overlay(remote.to_str().unwrap(), None, &dir, false)
            .await
            .unwrap();
        std::fs::write(dir.join("shine.env.toml"), "STALE = 'active'\n").unwrap();
        std::fs::create_dir(dir.join("ignored")).unwrap();
        std::fs::write(dir.join("ignored/secret.txt"), "local\n").unwrap();
        init_repo(&dir.join("app/stale"));
        std::fs::write(dir.join("preset.txt"), "local edits\n").unwrap();
        let origin = git_output(&dir, &["remote", "get-url", "origin"])
            .await
            .unwrap()
            .stdout;
        assert!(
            sync_managed_overlay(root.join("missing").to_str().unwrap(), None, &dir, false)
                .await
                .is_err()
        );
        assert!(dir.join("shine.env.toml").exists());
        assert!(dir.join("ignored/secret.txt").exists());
        assert!(dir.join("app/stale/.git").exists());
        assert_eq!(read_text(dir.join("preset.txt")), "local edits\n");
        assert_eq!(
            git_output(&dir, &["remote", "get-url", "origin"])
                .await
                .unwrap()
                .stdout,
            origin
        );
        // Even an unchanged remote HEAD must remove stale effective inputs.
        sync_managed_overlay(remote.to_str().unwrap(), None, &dir, false)
            .await
            .unwrap();
        assert!(!dir.join("shine.env.toml").exists());
        assert!(!dir.join("ignored").exists());
        assert!(!dir.join("app").exists());
        assert_eq!(read_text(dir.join("preset.txt")), "one\n");
        assert_eq!(read_text(remote.join("preset.txt")), "one\n");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn managed_overlay_relink_uses_new_source_and_remote_default() {
        let root = temp_dir("relink");
        let first = root.join("first");
        let second = root.join("second");
        init_repo(&first);
        init_repo(&second);
        git(&first, &["branch", "-M", "old-default"]);
        git(&second, &["branch", "-M", "new-default"]);
        std::fs::write(second.join("preset.txt"), "second source\n").unwrap();
        git(&second, &["commit", "-am", "second source"]);
        let dir = root.join("overlay");
        sync_managed_overlay(first.to_str().unwrap(), None, &dir, false)
            .await
            .unwrap();
        sync_managed_overlay(second.to_str().unwrap(), None, &dir, false)
            .await
            .unwrap();
        assert_eq!(read_text(dir.join("preset.txt")), "second source\n");
        let origin = git_output(&dir, &["remote", "get-url", "origin"])
            .await
            .unwrap();
        assert!(origin.status.success());
        assert_eq!(
            String::from_utf8(origin.stdout).unwrap().trim(),
            second.to_str().unwrap()
        );

        // A failed relink must retain both the last working content and origin.
        assert!(
            sync_managed_overlay(root.join("missing").to_str().unwrap(), None, &dir, false)
                .await
                .is_err()
        );
        assert_eq!(read_text(dir.join("preset.txt")), "second source\n");
        assert_eq!(
            git_output(&dir, &["remote", "get-url", "origin"])
                .await
                .unwrap()
                .stdout,
            format!("{}\n", second.display()).as_bytes()
        );

        // Explicit branch selection still overrides the remote's default.
        git(&second, &["checkout", "-b", "selected"]);
        std::fs::write(second.join("preset.txt"), "selected branch\n").unwrap();
        git(&second, &["commit", "-am", "selected branch"]);
        git(&second, &["checkout", "new-default"]);
        sync_managed_overlay(second.to_str().unwrap(), Some("selected"), &dir, false)
            .await
            .unwrap();
        assert_eq!(read_text(dir.join("preset.txt")), "selected branch\n");
        sync_managed_overlay(second.to_str().unwrap(), None, &dir, false)
            .await
            .unwrap();
        assert_eq!(read_text(dir.join("preset.txt")), "second source\n");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn managed_overlay_clones_shallow_then_force_mirrors() {
        let root = temp_dir("managed-overlay");
        let (_remote, seed, url) = seed_remote(&root);
        let dir = root.join("overlay");

        // First sync clones --depth 1: content present, history is a single commit.
        sync_managed_overlay(&url, None, &dir, false).await.unwrap();
        assert_eq!(read_text(dir.join("overlay.txt")), "one\n");
        assert_eq!(commit_count(&dir), "1");

        // Fast-forward update on the remote is mirrored.
        std::fs::write(seed.join("overlay.txt"), "two\n").unwrap();
        git(&seed, &["add", "overlay.txt"]);
        git(&seed, &["commit", "-m", "update"]);
        git(&seed, &["push"]);
        sync_managed_overlay(&url, None, &dir, false).await.unwrap();
        assert_eq!(read_text(dir.join("overlay.txt")), "two\n");

        // Rewritten history (amend + force-push) still mirrors where a
        // fast-forward pull would fail.
        std::fs::write(seed.join("overlay.txt"), "three\n").unwrap();
        git(&seed, &["add", "overlay.txt"]);
        git(&seed, &["commit", "--amend", "-m", "rewritten"]);
        git(&seed, &["push", "--force"]);
        sync_managed_overlay(&url, None, &dir, false).await.unwrap();
        assert_eq!(read_text(dir.join("overlay.txt")), "three\n");

        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn managed_overlay_fetch_failure_keeps_existing_checkout() {
        let root = temp_dir("managed-overlay-offline");
        let (remote, _seed, url) = seed_remote(&root);
        let dir = root.join("overlay");
        sync_managed_overlay(&url, None, &dir, false).await.unwrap();

        // Remote disappears → the next sync's fetch fails, but the existing
        // checkout is left untouched and still usable.
        std::fs::remove_dir_all(&remote).unwrap();
        let error = sync_managed_overlay(&url, None, &dir, false)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("fetch"));
        assert_eq!(read_text(dir.join("overlay.txt")), "one\n");

        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn managed_overlay_failed_clone_leaves_no_dir() {
        let root = temp_dir("managed-overlay-badurl");
        std::fs::create_dir_all(&root).unwrap();
        let dir = root.join("overlay");
        let bogus = root.join("does-not-exist.git");

        let error = sync_managed_overlay(&bogus.to_string_lossy(), None, &dir, false)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("clone"));
        assert!(
            !dir.exists(),
            "a failed first clone must not leave a managed overlay dir"
        );
        assert!(
            !temp_clone_path(&dir).unwrap().exists(),
            "the staging temp dir must be cleaned up on clone failure"
        );

        std::fs::remove_dir_all(root).unwrap();
    }
}
