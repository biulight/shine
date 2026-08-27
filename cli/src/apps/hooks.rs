//! Shared runner for app-preset lifecycle command hooks.
//!
//! `post_install` (fired by `shine app install`, including `--replace-managed`) and `post_upgrade`
//! (fired by `shine upgrade`) share identical execution semantics — run once per
//! changed category, gated behind `allow_app_hooks` for external presets, with
//! non-fatal failures. This module is the single implementation; the two phases
//! differ only in which hook list they read and the wording of their log lines.

use std::collections::BTreeSet;
use tokio::process::Command;

use crate::colors;
use crate::config::Config;

use super::metadata::{AppCategory, AppHook};

/// Which lifecycle moment a hook run belongs to.
pub(crate) enum HookPhase {
    PostInstall,
    PostUpgrade,
}

impl HookPhase {
    fn select<'a>(&self, cat: &'a AppCategory) -> &'a [AppHook] {
        match self {
            HookPhase::PostInstall => &cat.post_install,
            HookPhase::PostUpgrade => &cat.post_upgrade,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            HookPhase::PostInstall => "post-install",
            HookPhase::PostUpgrade => "post-upgrade",
        }
    }
}

/// Runs the phase's hooks for every changed category, in `changed` (sorted)
/// order. External presets are gated behind `allow_app_hooks` (a skipped
/// category prints a copy-pasteable manual command). Hook failures are
/// non-fatal: the failure is printed and that category's remaining hooks are
/// skipped, but the caller's command still succeeds. `show_success` controls
/// successful completion lines and `show_output` notes only; blocked and failed
/// hooks are always reported. Returns any notes that were shown.
pub(crate) async fn run_app_hooks<'a>(
    config: &Config,
    get_category: impl Fn(&str) -> Option<&'a AppCategory>,
    changed: &BTreeSet<String>,
    phase: HookPhase,
    show_success: bool,
) -> Vec<String> {
    let label = phase.label();
    let mut all_notes: Vec<String> = Vec::new();
    for category in changed {
        let Some(cat) = get_category(category) else {
            continue;
        };
        let hooks = phase.select(cat);
        if hooks.is_empty() {
            continue;
        }
        if config.is_external_presets && !config.allow_app_hooks {
            println!(
                "  {} {category}: {label} hook skipped (set allow_app_hooks = true to allow external app hooks; manual: {})",
                colors::symbol("!"),
                hook_sequence_display(hooks)
            );
            continue;
        }
        let mut completed = true;
        let mut notes: Vec<String> = Vec::new();
        for hook in hooks {
            match Command::new(&hook.command).args(&hook.args).output().await {
                Ok(output) if output.status.success() => {
                    if show_success && hook.show_output {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        let trimmed = stdout.trim();
                        if !trimmed.is_empty() {
                            notes.push(trimmed.to_string());
                        }
                    }
                }
                Ok(output) => {
                    eprintln!(
                        "  {} {category}: {label} hook failed: {} exited with {}{}",
                        colors::symbol("!"),
                        hook.command,
                        output.status,
                        command_output_detail(&output)
                    );
                    completed = false;
                    break;
                }
                Err(e) => {
                    eprintln!(
                        "  {} {category}: {label} hook failed: {}: {e}",
                        colors::symbol("!"),
                        hook.command
                    );
                    completed = false;
                    break;
                }
            }
        }
        if completed && show_success {
            println!(
                "  {} {category}: {label} hook completed",
                colors::symbol("✓")
            );
        }
        for note in &notes {
            for line in note.lines() {
                println!("     {}", colors::dim(line));
            }
        }
        all_notes.extend(notes);
    }
    all_notes
}

fn command_output_detail(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = stderr.trim();
    let detail = if detail.is_empty() {
        stdout.trim()
    } else {
        detail
    };
    if detail.is_empty() {
        String::new()
    } else {
        format!(": {detail}")
    }
}

fn hook_sequence_display(hooks: &[AppHook]) -> String {
    hooks
        .iter()
        .map(hook_command_display)
        .collect::<Vec<_>>()
        .join(" && ")
}

fn hook_command_display(hook: &AppHook) -> String {
    std::iter::once(hook.command.as_str())
        .chain(hook.args.iter().map(String::as_str))
        .map(shell_quote_for_display)
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote_for_display(value: &str) -> String {
    crate::shell_quote::quote_if_needed(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::metadata;
    #[cfg(unix)]
    use crate::apps::metadata::AppListMode;
    #[cfg(unix)]
    use crate::config::Config;
    #[cfg(unix)]
    use std::collections::{BTreeMap, BTreeSet};

    #[cfg(unix)]
    #[tokio::test]
    async fn external_post_upgrade_hook_requires_opt_in() {
        let dir = std::env::temp_dir().join(format!("shine-hook-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let marker = dir.join("marker");
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;

        let mut categories = BTreeMap::new();
        categories.insert(
            "sample".to_string(),
            sample_hook_category(
                &format!("printf ran > {}", marker.display()),
                false,
                HookPhase::PostUpgrade,
            ),
        );
        let updated = BTreeSet::from(["sample".to_string()]);

        run_app_hooks(
            &config,
            |name| categories.get(name),
            &updated,
            HookPhase::PostUpgrade,
            true,
        )
        .await;
        assert!(!marker.exists(), "external hook must be skipped by default");

        config.allow_app_hooks = true;
        run_app_hooks(
            &config,
            |name| categories.get(name),
            &updated,
            HookPhase::PostUpgrade,
            true,
        )
        .await;
        assert_eq!(tokio::fs::read_to_string(&marker).await.unwrap(), "ran");

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn post_install_hook_runs_its_own_phase() {
        let dir = std::env::temp_dir().join(format!("shine-hook-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let marker = dir.join("marker");
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        config.allow_app_hooks = true;

        let mut categories = BTreeMap::new();
        categories.insert(
            "sample".to_string(),
            sample_hook_category(
                &format!("printf ran > {}", marker.display()),
                false,
                HookPhase::PostInstall,
            ),
        );
        let changed = BTreeSet::from(["sample".to_string()]);

        // The post-upgrade phase must not fire an install-only hook.
        run_app_hooks(
            &config,
            |name| categories.get(name),
            &changed,
            HookPhase::PostUpgrade,
            true,
        )
        .await;
        assert!(
            !marker.exists(),
            "post_install hook must not run on upgrade"
        );

        run_app_hooks(
            &config,
            |name| categories.get(name),
            &changed,
            HookPhase::PostInstall,
            true,
        )
        .await;
        assert_eq!(tokio::fs::read_to_string(&marker).await.unwrap(), "ran");

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn post_upgrade_hook_prints_stdout_when_show_output_is_true() {
        let dir = std::env::temp_dir().join(format!("shine-hook-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        config.allow_app_hooks = true;

        let mut categories = BTreeMap::new();
        categories.insert(
            "sample".to_string(),
            sample_hook_category("echo hello from hook", true, HookPhase::PostUpgrade),
        );
        let updated = BTreeSet::from(["sample".to_string()]);

        let notes = run_app_hooks(
            &config,
            |name| categories.get(name),
            &updated,
            HookPhase::PostUpgrade,
            true,
        )
        .await;
        assert_eq!(notes, vec!["hello from hook".to_string()]);

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn post_upgrade_hook_stays_silent_without_show_output() {
        let dir = std::env::temp_dir().join(format!("shine-hook-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        config.allow_app_hooks = true;

        let mut categories = BTreeMap::new();
        categories.insert(
            "sample".to_string(),
            sample_hook_category("echo hello from hook", false, HookPhase::PostUpgrade),
        );
        let updated = BTreeSet::from(["sample".to_string()]);

        let notes = run_app_hooks(
            &config,
            |name| categories.get(name),
            &updated,
            HookPhase::PostUpgrade,
            true,
        )
        .await;
        assert!(
            notes.is_empty(),
            "hook stdout must stay silent by default: {notes:?}"
        );

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn quiet_upgrade_suppresses_success_output_notes() {
        let dir = std::env::temp_dir().join(format!("shine-hook-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        config.allow_app_hooks = true;

        let categories = BTreeMap::from([(
            "sample".to_string(),
            sample_hook_category("echo hidden detail", true, HookPhase::PostUpgrade),
        )]);
        let updated = BTreeSet::from(["sample".to_string()]);
        let notes = run_app_hooks(
            &config,
            |name| categories.get(name),
            &updated,
            HookPhase::PostUpgrade,
            false,
        )
        .await;

        assert!(notes.is_empty());
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn hook_command_display_is_copy_pasteable() {
        let hook = metadata::AppHook {
            command: "/Applications/Surge.app/Contents/Applications/surge-cli".to_string(),
            args: vec![
                "external-resource".to_string(),
                "update".to_string(),
                "all".to_string(),
            ],
            show_output: false,
        };
        assert_eq!(
            hook_command_display(&hook),
            "/Applications/Surge.app/Contents/Applications/surge-cli external-resource update all"
        );

        let hook = metadata::AppHook {
            command: "/tmp/my hook".to_string(),
            args: vec!["it's".to_string()],
            show_output: false,
        };
        assert_eq!(hook_command_display(&hook), "'/tmp/my hook' 'it'\\''s'");
    }

    #[test]
    fn hook_sequence_display_joins_commands_in_order() {
        let hooks = vec![
            metadata::AppHook {
                command: "surge-cli".to_string(),
                args: vec![
                    "external-resource".to_string(),
                    "update".to_string(),
                    "all".to_string(),
                ],
                show_output: false,
            },
            metadata::AppHook {
                command: "surge-cli".to_string(),
                args: vec!["reload".to_string()],
                show_output: false,
            },
        ];
        assert_eq!(
            hook_sequence_display(&hooks),
            "surge-cli external-resource update all && surge-cli reload"
        );
    }

    #[cfg(unix)]
    fn sample_hook_category(
        script: &str,
        show_output: bool,
        phase: HookPhase,
    ) -> metadata::AppCategory {
        let hook = vec![metadata::AppHook {
            command: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), script.to_string()],
            show_output,
        }];
        let (post_install, post_upgrade) = match phase {
            HookPhase::PostInstall => (hook, Vec::new()),
            HookPhase::PostUpgrade => (Vec::new(), hook),
        };
        metadata::AppCategory {
            name: "sample".to_string(),
            description: None,
            destination_root: Some("~/.config/sample".to_string()),
            files: vec![],
            list_mode: AppListMode::Files,
            post_upgrade,
            post_install,
            uses_metadata: true,
            has_explicit_files: true,
            artifact: None,
        }
    }
}
