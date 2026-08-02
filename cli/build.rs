use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=../presets");
    println!("cargo:rerun-if-env-changed=SHINE_VERSION_METADATA");
    println!("cargo:rerun-if-env-changed=SHINE_GIT_SHA");
    println!("cargo:rerun-if-env-changed=SHINE_GIT_DATE");

    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    emit_git_rerun_paths(&repo);

    if let Ok(metadata) = std::env::var("SHINE_VERSION_METADATA")
        && !metadata.trim().is_empty()
    {
        println!("cargo:rustc-env=SHINE_VERSION_METADATA={metadata}");
    }

    if let Some(sha) = env_or_git("SHINE_GIT_SHA", &repo, &["rev-parse", "--short=9", "HEAD"]) {
        let short_sha: String = sha.chars().take(9).collect();
        println!("cargo:rustc-env=SHINE_GIT_SHA={short_sha}");
    }
    if let Some(date) = env_or_git(
        "SHINE_GIT_DATE",
        &repo,
        &["show", "-s", "--format=%cs", "HEAD"],
    ) {
        println!("cargo:rustc-env=SHINE_GIT_DATE={date}");
    }
}

fn env_or_git(name: &str, repo: &std::path::Path, args: &[&str]) -> Option<String> {
    std::env::var(name)
        .ok()
        .and_then(nonempty)
        .or_else(|| git(repo, args))
}

fn git(repo: &std::path::Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .and_then(nonempty)
}

fn nonempty(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn emit_git_rerun_paths(repo: &std::path::Path) {
    let Some(git_dir) = git(repo, &["rev-parse", "--absolute-git-dir"]) else {
        return;
    };
    let git_dir = PathBuf::from(git_dir);
    println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
    if let Some(reference) = git(repo, &["symbolic-ref", "-q", "HEAD"]) {
        println!(
            "cargo:rerun-if-changed={}",
            git_dir.join(reference).display()
        );
    }
}
