use anyhow::{Context, Result};
use std::path::Path;

use crate::config::Config;
use crate::install_core::eol_eq;
use crate::shells::ShellType;

use super::profile::SysShellProfileUpdate;
use super::{SYS_PROFILE_PHASES, ShellProfileBlockPosition, SysProfilePhase};

pub(super) async fn update_sys_shell_profiles(
    config: &Config,
    os_id: &str,
    sys_shell: &str,
) -> Result<SysShellProfileUpdate> {
    match os_id {
        "macos" => update_macos_shell_profile(config, os_id).await,
        "ubuntu" => update_ubuntu_shell_profile(config, os_id, sys_shell).await,
        "windows" => update_windows_shell_profile(config, os_id).await,
        _ => Ok(SysShellProfileUpdate {
            updated: false,
            unsupported_shell: true,
            detail: format!("unsupported OS for sys profile: {os_id}"),
        }),
    }
}

async fn update_macos_shell_profile(config: &Config, os_id: &str) -> Result<SysShellProfileUpdate> {
    let path = config.home_dir.join(".zshrc");
    let updated = update_sys_shell_profile_blocks(&path, os_id, None).await?;
    Ok(SysShellProfileUpdate {
        updated,
        unsupported_shell: false,
        detail: format!("~/.zshrc -> {}", sys_loader_display(os_id)),
    })
}

async fn update_ubuntu_shell_profile(
    config: &Config,
    os_id: &str,
    sys_shell: &str,
) -> Result<SysShellProfileUpdate> {
    match config.shell_type {
        ShellType::Bash => {
            let updated = update_sys_shell_profile_blocks(
                &config.home_dir.join(".bashrc"),
                os_id,
                Some("bash"),
            )
            .await?;
            remove_sys_shell_profile_blocks(&config.home_dir.join(".zshrc"), os_id).await?;
            Ok(SysShellProfileUpdate {
                updated,
                unsupported_shell: false,
                detail: format!("~/.bashrc -> {}", sys_loader_display(os_id)),
            })
        }
        ShellType::Zsh => {
            let updated = update_sys_shell_profile_blocks(
                &config.home_dir.join(".zshrc"),
                os_id,
                Some("zsh"),
            )
            .await?;
            remove_sys_shell_profile_blocks(&config.home_dir.join(".bashrc"), os_id).await?;
            Ok(SysShellProfileUpdate {
                updated,
                unsupported_shell: false,
                detail: format!("~/.zshrc -> {}", sys_loader_display(os_id)),
            })
        }
        _ => Ok(SysShellProfileUpdate {
            updated: false,
            unsupported_shell: true,
            detail: format!("unsupported shell for sys profile: {sys_shell}"),
        }),
    }
}

async fn update_windows_shell_profile(
    config: &Config,
    os_id: &str,
) -> Result<SysShellProfileUpdate> {
    let mut updated = false;
    for path in [
        config
            .home_dir
            .join("Documents/PowerShell/Microsoft.PowerShell_profile.ps1"),
        config
            .home_dir
            .join("Documents/WindowsPowerShell/Microsoft.PowerShell_profile.ps1"),
    ] {
        updated |= update_sys_shell_profile_blocks(&path, os_id, None).await?;
    }
    Ok(SysShellProfileUpdate {
        updated,
        unsupported_shell: false,
        detail: "PowerShell profiles".to_string(),
    })
}

pub(super) async fn update_sys_shell_profile_blocks(
    path: &Path,
    os_id: &str,
    shell_name: Option<&str>,
) -> Result<bool> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating {}", parent.display()))?;
    }

    let content = match tokio::fs::read_to_string(path).await {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let had_utf8_bom = content.contains('\u{feff}');
    let bom_was_at_start = content.starts_with('\u{feff}');
    let content = content.replace('\u{feff}', "");
    let pre_block = sys_shell_profile_block(os_id, SysProfilePhase::Pre, shell_name);
    let post_block = sys_shell_profile_block(os_id, SysProfilePhase::Post, shell_name);
    let pre_sentinel = sys_sentinel(os_id, SysProfilePhase::Pre);
    let post_sentinel = sys_sentinel(os_id, SysProfilePhase::Post);

    // Compare the installed block against the expected one ignoring line-ending
    // style, so a CRLF profile (e.g. one a Windows editor re-saved) whose block
    // is otherwise up to date short-circuits here — leaving the user's file
    // untouched instead of reporting a spurious update and rewriting it to LF.
    // `extract_sentinel_block` only reattaches a trailing `\n` (never `\r\n`), so
    // a CRLF block extracts without its terminator while `expected` ends in `\n`;
    // trim the trailing line break on both sides before the ending-agnostic compare.
    let block_matches = |extracted: Option<&str>, expected: &str| {
        extracted.is_some_and(|block| {
            eol_eq(
                block.trim_end_matches(['\r', '\n']).as_bytes(),
                expected.trim_end_matches(['\r', '\n']).as_bytes(),
            )
        })
    };
    if extract_sentinel_block(&content, legacy_sys_sentinel(os_id)).is_none()
        && block_matches(extract_sentinel_block(&content, pre_sentinel), &pre_block)
        && block_matches(extract_sentinel_block(&content, post_sentinel), &post_block)
        && sentinel_order_is_valid(&content, pre_sentinel, post_sentinel)
        && (!had_utf8_bom || bom_was_at_start)
    {
        return Ok(false);
    }

    let mut updated = remove_sentinel_block(&content, legacy_sys_sentinel(os_id));
    updated = remove_sentinel_block(&updated, pre_sentinel);
    updated = remove_sentinel_block(&updated, post_sentinel);
    updated = trim_outer_blank_lines(&updated);
    updated = insert_shell_profile_block(&updated, &pre_block, ShellProfileBlockPosition::Start);
    updated = insert_shell_profile_block(&updated, &post_block, ShellProfileBlockPosition::End);
    if had_utf8_bom {
        updated.insert(0, '\u{feff}');
    }

    tokio::fs::write(path, updated)
        .await
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

async fn remove_sys_shell_profile_blocks(path: &Path, os_id: &str) -> Result<bool> {
    let mut updated = remove_shell_profile_block(path, legacy_sys_sentinel(os_id)).await?;
    for phase in SYS_PROFILE_PHASES {
        updated |= remove_shell_profile_block(path, sys_sentinel(os_id, phase)).await?;
    }
    Ok(updated)
}

fn legacy_sys_sentinel(os_id: &str) -> (&'static str, &'static str) {
    match os_id {
        "macos" => ("# >>> shine macos sys >>>", "# <<< shine macos sys <<<"),
        "windows" => ("# >>> shine windows sys >>>", "# <<< shine windows sys <<<"),
        _ => ("# >>> shine ubuntu sys >>>", "# <<< shine ubuntu sys <<<"),
    }
}

fn sys_sentinel(os_id: &str, phase: SysProfilePhase) -> (&'static str, &'static str) {
    match (os_id, phase) {
        ("macos", SysProfilePhase::Pre) => (
            "# >>> shine macos sys pre >>>",
            "# <<< shine macos sys pre <<<",
        ),
        ("macos", SysProfilePhase::Post) => (
            "# >>> shine macos sys post >>>",
            "# <<< shine macos sys post <<<",
        ),
        ("windows", SysProfilePhase::Pre) => (
            "# >>> shine windows sys pre >>>",
            "# <<< shine windows sys pre <<<",
        ),
        ("windows", SysProfilePhase::Post) => (
            "# >>> shine windows sys post >>>",
            "# <<< shine windows sys post <<<",
        ),
        (_, SysProfilePhase::Pre) => (
            "# >>> shine ubuntu sys pre >>>",
            "# <<< shine ubuntu sys pre <<<",
        ),
        (_, SysProfilePhase::Post) => (
            "# >>> shine ubuntu sys post >>>",
            "# <<< shine ubuntu sys post <<<",
        ),
    }
}

fn sys_shell_profile_block(
    os_id: &str,
    phase: SysProfilePhase,
    shell_name: Option<&str>,
) -> String {
    let (start, end) = sys_sentinel(os_id, phase);
    match os_id {
        "windows" => format!(
            r#"{start}
$shineWindowsSysProfile = Join-Path $HOME ".shine\profile\windows-sys.{phase}.ps1"
if (Test-Path -LiteralPath $shineWindowsSysProfile) {{
    . $shineWindowsSysProfile
}}
{end}
"#,
            phase = phase.as_str()
        ),
        "macos" => format!(
            r#"{start}
shine_macos_sys_profile="$HOME/.shine/profile/macos-sys.{phase}.sh"
if [[ -f "$shine_macos_sys_profile" ]]; then
  source "$shine_macos_sys_profile"
fi
{end}
"#,
            phase = phase.as_str()
        ),
        _ => {
            let shell_name = shell_name.unwrap_or("bash");
            format!(
                r#"{start}
shine_ubuntu_sys_profile="$HOME/.shine/profile/ubuntu-sys.{phase}.sh"
if [[ -f "$shine_ubuntu_sys_profile" ]]; then
  SHINE_UBUNTU_SYS_SHELL="{shell_name}"
  source "$shine_ubuntu_sys_profile"
fi
{end}
"#,
                phase = phase.as_str()
            )
        }
    }
}

fn to_shared_sentinel<'a>(sentinel: (&'a str, &'a str)) -> crate::sentinel::Sentinel<'a> {
    crate::sentinel::Sentinel {
        start: sentinel.0,
        end: sentinel.1,
    }
}

fn insert_shell_profile_block(
    content: &str,
    desired_block: &str,
    position: ShellProfileBlockPosition,
) -> String {
    let at = match position {
        ShellProfileBlockPosition::Start => crate::sentinel::InsertAt::Start,
        ShellProfileBlockPosition::End => crate::sentinel::InsertAt::End,
    };
    crate::sentinel::insert_block(content, desired_block, at)
}

fn sentinel_order_is_valid(content: &str, first: (&str, &str), second: (&str, &str)) -> bool {
    match (content.find(first.0), content.find(second.0)) {
        (Some(first), Some(second)) => first < second,
        _ => false,
    }
}

fn trim_outer_blank_lines(content: &str) -> String {
    crate::sentinel::trim_outer_blank_lines(content)
}

async fn remove_shell_profile_block(path: &Path, sentinel: (&str, &str)) -> Result<bool> {
    let Ok(content) = tokio::fs::read_to_string(path).await else {
        return Ok(false);
    };
    let updated = remove_sentinel_block(&content, sentinel);
    if updated == content {
        return Ok(false);
    }
    tokio::fs::write(path, updated)
        .await
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

fn extract_sentinel_block<'a>(content: &'a str, sentinel: (&str, &str)) -> Option<&'a str> {
    crate::sentinel::extract_block_with_newline(content, &to_shared_sentinel(sentinel))
}

fn remove_sentinel_block(content: &str, sentinel: (&str, &str)) -> String {
    crate::sentinel::remove_block_linewise(content, &to_shared_sentinel(sentinel))
}

fn sys_loader_display(os_id: &str) -> String {
    format!(
        "~/.shine/profile/{os_id}-sys.{{pre,post}}.{}",
        if os_id == "windows" { "ps1" } else { "sh" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SENTINEL: (&str, &str) = ("START", "END");

    #[test]
    fn remove_sentinel_block_returns_unchanged_when_sentinel_absent() {
        let content = "no sentinel here\n";
        assert_eq!(remove_sentinel_block(content, SENTINEL), content);
    }

    #[test]
    fn remove_sentinel_block_removes_bounded_lines_but_keeps_preceding_blank_line() {
        // Unlike shells/profile.rs's byte-offset version, this line-based
        // implementation never consumes a preceding blank line separator —
        // it only drops the sentinel lines themselves.
        let content = "before\n\nSTART\nbody\nEND\nafter\n";
        assert_eq!(
            remove_sentinel_block(content, SENTINEL),
            "before\n\nafter\n"
        );
    }

    #[test]
    fn remove_sentinel_block_normalizes_crlf_to_lf_even_without_sentinel() {
        // content.lines() strips '\r' unconditionally, so any CRLF input is
        // normalized to LF by this function, whether or not it contains the
        // sentinel — a side effect shells/profile.rs's byte-offset version
        // does not have.
        let content = "before\r\nafter\r\n";
        assert_eq!(remove_sentinel_block(content, SENTINEL), "before\nafter\n");
    }

    #[test]
    fn remove_sentinel_block_preserves_trailing_newline_presence() {
        let with_newline = "before\nSTART\nbody\nEND\nafter\n";
        let without_newline = "before\nSTART\nbody\nEND\nafter";
        assert!(remove_sentinel_block(with_newline, SENTINEL).ends_with('\n'));
        assert!(!remove_sentinel_block(without_newline, SENTINEL).ends_with('\n'));
    }

    #[test]
    fn extract_sentinel_block_returns_none_when_start_missing() {
        assert_eq!(extract_sentinel_block("no markers here", SENTINEL), None);
    }

    #[test]
    fn extract_sentinel_block_returns_none_when_end_missing_after_start() {
        assert_eq!(extract_sentinel_block("STARTonly, no end", SENTINEL), None);
    }

    #[test]
    fn extract_sentinel_block_includes_one_trailing_newline_when_present() {
        let content = "pre\nSTART\nbody\nEND\npost";
        assert_eq!(
            extract_sentinel_block(content, SENTINEL),
            Some("START\nbody\nEND\n")
        );
    }

    #[test]
    fn extract_sentinel_block_omits_trailing_newline_when_absent() {
        let content = "pre\nSTART\nbody\nEND";
        assert_eq!(
            extract_sentinel_block(content, SENTINEL),
            Some("START\nbody\nEND")
        );
    }

    #[test]
    fn insert_start_adds_exactly_one_blank_line_before_nonempty_content() {
        let updated =
            insert_shell_profile_block("rest", "BLOCK\n", ShellProfileBlockPosition::Start);
        assert_eq!(updated, "BLOCK\n\nrest");
    }

    #[test]
    fn insert_start_into_empty_content_has_no_trailing_blank_line() {
        let updated = insert_shell_profile_block("", "BLOCK\n", ShellProfileBlockPosition::Start);
        assert_eq!(updated, "BLOCK\n");
    }

    #[test]
    fn insert_end_adds_exactly_one_blank_line_regardless_of_existing_trailing_newline() {
        let with_newline =
            insert_shell_profile_block("rest\n", "BLOCK\n", ShellProfileBlockPosition::End);
        let without_newline =
            insert_shell_profile_block("rest", "BLOCK\n", ShellProfileBlockPosition::End);
        assert_eq!(with_newline, "rest\n\nBLOCK\n");
        assert_eq!(without_newline, "rest\n\nBLOCK\n");
    }

    #[test]
    fn insert_end_into_empty_content_has_no_leading_blank_line() {
        let updated = insert_shell_profile_block("", "BLOCK\n", ShellProfileBlockPosition::End);
        assert_eq!(updated, "BLOCK\n");
    }

    #[test]
    fn trim_outer_blank_lines_removes_all_leading_and_trailing_newlines() {
        assert_eq!(trim_outer_blank_lines("\n\n\nfoo\nbar\n\n"), "foo\nbar");
    }

    #[test]
    fn sentinel_order_is_valid_requires_first_before_second() {
        let first = ("FIRST_START", "FIRST_END");
        let second = ("SECOND_START", "SECOND_END");
        assert!(sentinel_order_is_valid(
            "x FIRST_START y SECOND_START z",
            first,
            second
        ));
        assert!(!sentinel_order_is_valid(
            "x SECOND_START y FIRST_START z",
            first,
            second
        ));
        assert!(!sentinel_order_is_valid(
            "neither marker present",
            first,
            second
        ));
    }

    #[tokio::test]
    async fn update_sys_shell_profile_blocks_inserts_pre_and_post_on_empty_file() {
        let dir = crate::test_support::make_temp_dir("shine-sys-profile").await;
        let path = dir.join("profile.sh");

        let updated = update_sys_shell_profile_blocks(&path, "ubuntu", Some("bash"))
            .await
            .unwrap();

        assert!(updated);
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(content.contains("shine ubuntu sys pre"));
        assert!(content.contains("shine ubuntu sys post"));
        // Pre block must come before the post block.
        assert!(content.find("sys pre").unwrap() < content.find("sys post").unwrap());

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn update_sys_shell_profile_blocks_is_idempotent_when_already_up_to_date() {
        let dir = crate::test_support::make_temp_dir("shine-sys-profile").await;
        let path = dir.join("profile.sh");

        assert!(
            update_sys_shell_profile_blocks(&path, "ubuntu", Some("bash"))
                .await
                .unwrap()
        );
        // Second call against the now-converged file must report no change.
        assert!(
            !update_sys_shell_profile_blocks(&path, "ubuntu", Some("bash"))
                .await
                .unwrap()
        );

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn update_sys_shell_profile_blocks_ignores_crlf_and_preserves_endings() {
        let dir = crate::test_support::make_temp_dir("shine-sys-profile").await;
        let path = dir.join("profile.sh");

        // Install (writes the block LF).
        update_sys_shell_profile_blocks(&path, "ubuntu", Some("bash"))
            .await
            .unwrap();

        // Simulate a Windows editor re-saving the whole file with CRLF endings.
        let lf = tokio::fs::read_to_string(&path).await.unwrap();
        let crlf = lf.replace('\n', "\r\n");
        tokio::fs::write(&path, &crlf).await.unwrap();

        // Only the endings differ, so this must report no change...
        assert!(
            !update_sys_shell_profile_blocks(&path, "ubuntu", Some("bash"))
                .await
                .unwrap()
        );
        // ...and leave the user's CRLF file untouched (no silent LF rewrite).
        let after = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(after, crlf);

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn update_sys_shell_profile_blocks_preserves_leading_bom() {
        let dir = crate::test_support::make_temp_dir("shine-sys-profile").await;
        let path = dir.join("profile.ps1");
        tokio::fs::write(&path, "\u{feff}# existing content\n")
            .await
            .unwrap();

        update_sys_shell_profile_blocks(&path, "windows", None)
            .await
            .unwrap();

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(
            content.starts_with('\u{feff}'),
            "BOM must be preserved at the start of the file"
        );
        assert_eq!(
            content.matches('\u{feff}').count(),
            1,
            "exactly one BOM must remain, not re-duplicated"
        );

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn update_sys_shell_profile_blocks_migrates_legacy_sentinel() {
        let dir = crate::test_support::make_temp_dir("shine-sys-profile").await;
        let path = dir.join("profile.sh");
        let (legacy_start, legacy_end) = legacy_sys_sentinel("ubuntu");
        tokio::fs::write(&path, format!("{legacy_start}\nold body\n{legacy_end}\n"))
            .await
            .unwrap();

        let updated = update_sys_shell_profile_blocks(&path, "ubuntu", Some("bash"))
            .await
            .unwrap();

        assert!(updated);
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(
            !content.contains(legacy_start),
            "legacy sentinel must be removed"
        );
        assert!(content.contains("shine ubuntu sys pre"));
        assert!(content.contains("shine ubuntu sys post"));

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
