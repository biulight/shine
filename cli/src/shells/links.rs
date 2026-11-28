use crate::colors;
use crate::config::Config;
use crate::output;
use crate::path_display;
use std::ffi::OsStr;
use std::path::Path;

pub(super) fn link_conflict_render_lines(
    config: &Config,
    conflicts: &[crate::bin_links::LinkConflict],
    category_hint: Option<&str>,
) -> Vec<String> {
    let mut rendered = Vec::new();
    for conflict in conflicts {
        let lines = link_conflict_detail_lines(config, conflict, category_hint);
        if let Some((first, rest)) = lines.split_first() {
            let command = conflict
                .link_path
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or("unknown");
            rendered.push(output::detail_line_text(
                "Link Conflict",
                &colors::yellow(&output::escape_controls(command)),
                Some(first.clone()),
            ));
            for line in rest {
                rendered.push(format!(
                    "{}{}",
                    " ".repeat(16),
                    colors::dim(&output::escape_controls(line))
                ));
            }
        }
    }
    rendered
}

fn link_conflict_detail_lines(
    config: &Config,
    conflict: &crate::bin_links::LinkConflict,
    category_hint: Option<&str>,
) -> Vec<String> {
    let link = path_display::format_home(&conflict.link_path, &config.home_dir);
    let wanted = path_display::format_home(&conflict.source, &config.home_dir);
    let mut lines = Vec::new();

    match conflict.kind {
        crate::bin_links::LinkConflictKind::DuplicateName => {
            lines.push("duplicate requested command".to_string());
        }
        crate::bin_links::LinkConflictKind::ExistingEntry => {
            lines.push(format!(
                "existing: {}",
                describe_existing_link(&conflict.link_path, &link)
            ));
        }
    }

    lines.push(format!("wanted:   {wanted}"));
    lines.push(format!(
        "fix:      remove {link} or {}",
        reinstall_hint(config, category_hint, &conflict.source)
    ));
    lines
}

fn describe_existing_link(path: &Path, display_path: &str) -> String {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => match std::fs::read_link(path) {
            Ok(target) => format!("{display_path} -> {}", path_display::format(&target)),
            Err(_) => format!("{display_path} -> unreadable target"),
        },
        Ok(meta) if meta.is_dir() => format!("directory {display_path}"),
        Ok(_) => format!("file {display_path}"),
        Err(_) => format!("{display_path} no longer exists"),
    }
}

fn reinstall_hint(config: &Config, category_hint: Option<&str>, source: &Path) -> String {
    if let Some(category) = category_hint
        .map(str::to_owned)
        .or_else(|| infer_shell_category(config, source))
    {
        if category.chars().any(char::is_control) {
            return "rerun installation with the original category argument".into();
        }
        let target_arg = crate::shell_quote::command_arg(&format!("shell/{category}"));
        return format!("run `shine install {target_arg} --replace-managed`");
    }
    "run `shine shell install --replace-managed`".into()
}

fn infer_shell_category(config: &Config, source: &Path) -> Option<String> {
    source
        .strip_prefix(config.presets_dir())
        .ok()
        .and_then(shell_category_after_root)
        .or_else(|| shell_category_after_root(source.strip_prefix(config.rendered_dir()).ok()?))
}

fn shell_category_after_root(path: &Path) -> Option<String> {
    let mut components = path.components();
    if components.next()?.as_os_str() != OsStr::new("shell") {
        return None;
    }
    components.next()?.as_os_str().to_str().map(str::to_string)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::bin_links::{LinkConflict, LinkConflictKind};

    #[tokio::test]
    async fn link_conflict_details_describe_files_and_symlinks_with_reinstall_hint() {
        for is_symlink in [false, true] {
            let dir = crate::test_support::make_temp_dir("shine-shell").await;
            let config = Config::new_for_test(&dir);
            tokio::fs::create_dir_all(config.bin_dir()).await.unwrap();
            let conflict = LinkConflict {
                link_path: config.bin_dir().join("setproxy"),
                source: config.presets_dir().join("shell/proxy/set_proxy.sh"),
                kind: LinkConflictKind::ExistingEntry,
            };
            let (category_hint, existing) = if is_symlink {
                tokio::fs::create_dir_all(conflict.source.parent().unwrap())
                    .await
                    .unwrap();
                let target = dir.join("other-setproxy");
                tokio::fs::write(&target, b"#!/bin/sh\n").await.unwrap();
                tokio::fs::symlink(&target, &conflict.link_path)
                    .await
                    .unwrap();
                (
                    None,
                    format!("existing: ~/bin/setproxy -> {}", target.display()),
                )
            } else {
                tokio::fs::write(&conflict.link_path, b"custom command")
                    .await
                    .unwrap();
                (Some("proxy"), "existing: file ~/bin/setproxy".to_owned())
            };

            let lines = link_conflict_detail_lines(&config, &conflict, category_hint);
            assert_eq!(lines[0], existing);
            assert_eq!(lines[1], "wanted:   ~/presets/shell/proxy/set_proxy.sh");
            assert_eq!(
                lines[2],
                "fix:      remove ~/bin/setproxy or run `shine install shell/proxy --replace-managed`"
            );
            tokio::fs::remove_dir_all(dir).await.unwrap();
        }
    }
}

#[cfg(test)]
mod category_tests {
    use super::*;

    #[test]
    fn conflict_rows_escape_data_without_offering_control_category_aliases() {
        let config = Config::new_for_test(&std::env::temp_dir().join("shine-conflict-render"));
        let category = "demo\nFORGED=category\r\t\u{1b}";
        let command = "run\nFORGED=command\r\t\u{1b}";
        let conflict = crate::bin_links::LinkConflict {
            link_path: config.bin_dir().join(command),
            source: config
                .presets_dir()
                .join("shell")
                .join(category)
                .join("run.sh"),
            kind: crate::bin_links::LinkConflictKind::DuplicateName,
        };
        for hint in [None, Some(category)] {
            let lines = link_conflict_render_lines(&config, std::slice::from_ref(&conflict), hint);
            assert_eq!(lines.len(), 3);
            assert!(
                lines[0].contains("run\\nFORGED=command\\r\\t\\u{1b}"),
                "{lines:?}"
            );
            assert!(
                lines[1].contains("demo\\nFORGED=category\\r\\t\\u{1b}"),
                "{lines:?}"
            );
            assert!(!lines[2].contains("run `"), "{lines:?}");
            assert!(lines[2].contains("original category argument"), "{lines:?}");
            for line in lines {
                assert!(
                    !console::strip_ansi_codes(&line).contains(['\n', '\r', '\t', '\u{1b}']),
                    "{line:?}"
                );
            }
        }
        assert_eq!(conflict.link_path.file_name().unwrap(), command);
        assert_eq!(
            conflict.source.parent().unwrap().file_name().unwrap(),
            category
        );
    }

    #[test]
    fn conflict_guidance_preserves_category_for_preset_and_rendered_sources() {
        let root = std::env::temp_dir().join("shine-link-guidance-test");
        let config = Config::new_for_test(&root);
        for (category, argument) in [
            ("proxy", "shell/proxy"),
            ("demo space", "'shell/demo space'"),
            (
                "it's",
                if cfg!(windows) {
                    "'shell/it''s'"
                } else {
                    "'shell/it'\\''s'"
                },
            ),
        ] {
            for root in [config.presets_dir(), &config.rendered_dir()] {
                let conflict = crate::bin_links::LinkConflict {
                    link_path: config.bin_dir().join("run"),
                    source: root.join("shell").join(category).join("run.ts"),
                    kind: crate::bin_links::LinkConflictKind::DuplicateName,
                };
                for hint in [None, Some(category)] {
                    let lines = link_conflict_detail_lines(&config, &conflict, hint);
                    assert!(
                        lines[2].contains(&format!("shine install {argument} --replace-managed")),
                        "{lines:?}"
                    );
                }
                let hinted = link_conflict_detail_lines(&config, &conflict, Some("explicit"));
                assert!(hinted[2].contains("shine install shell/explicit --replace-managed"));
            }
        }
    }

    #[test]
    fn unknown_or_non_shell_source_keeps_generic_guidance() {
        let root = std::env::temp_dir().join("shine-link-guidance-test");
        let config = Config::new_for_test(&root);
        for source in [
            root.join("unrelated/proxy.sh"),
            config.presets_dir().join("app/proxy/config"),
            config.rendered_dir().join("app/proxy/config"),
            config.rendered_dir().join("shell"),
        ] {
            assert_eq!(
                reinstall_hint(&config, None, &source),
                "run `shine shell install --replace-managed`"
            );
        }
    }
}
