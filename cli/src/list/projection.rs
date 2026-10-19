//! CLI-only grouping, target selection and status priority. No Host or approval authority.
use super::*;

pub(super) fn pending_shell_rows<'a>(
    rows: &'a [ShellRow],
    pending: &BTreeSet<&str>,
) -> Vec<&'a ShellRow> {
    rows.iter()
        .filter(|row| row.is_installed && pending.contains(row.target().as_str()))
        .collect()
}

pub(super) fn shell_attention_lines(rows: &[&ShellRow]) -> Vec<String> {
    let mut lines = Vec::new();
    for row in rows
        .iter()
        .filter(|row| row.is_installed && (row.link_conflict || row.preset_missing))
    {
        lines.push(format!("  ! shell/{}  {}", row.label, row.status_text));
        if row.link_conflict {
            lines.push("    Launcher preserved; resolve its ownership conflict before upgrading this target.".to_string());
        }
        if row.preset_missing {
            lines.push(format!("    Restore the Preset or review `shine shell uninstall {}` to remove the installed command.", crate::shell_quote::command_arg(&row.selector())));
        }
    }
    lines
}

pub(super) fn update_hint_text(targets: &[String]) -> String {
    let command = match targets {
        [target] => format!("shine upgrade {}", crate::shell_quote::command_arg(target)),
        _ => "shine upgrade".to_string(),
    };
    format!("Run `{command}` to apply updates.")
}

pub(super) fn update_targets(shell: &[String], app: &[String], sys: &[String]) -> Vec<String> {
    shell
        .iter()
        .map(|category| format!("shell/{category}"))
        .chain(app.iter().map(|category| format!("app/{category}")))
        .chain(sys.iter().map(|item| format!("sys/{item}")))
        .collect()
}

pub(super) fn app_update_categories<'a>(rows: &[&'a AppRow]) -> BTreeMap<&'a str, Vec<&'a AppRow>> {
    let mut categories = BTreeMap::new();
    for row in rows {
        categories
            .entry(row.category.as_str())
            .or_insert_with(Vec::new)
            .push(*row);
    }
    categories
}

pub(super) fn app_refresh_commands(rows: &BTreeMap<&str, Vec<&AppRow>>) -> Vec<String> {
    rows.iter()
        .flat_map(|(category, rows)| {
            rows.iter().flat_map(move |row| {
                row.refresh_sources.iter().map(move |source| {
                    format!(
                        "shine app refresh {} {}",
                        crate::shell_quote::command_arg(category),
                        crate::shell_quote::command_arg(source)
                    )
                })
            })
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn app_category_action_status(rows: &[&AppRow]) -> (&'static str, &'static str) {
    let upgrade_available = rows.iter().any(|row| row.upgrade_available);
    let refresh_available = rows.iter().any(|row| !row.refresh_sources.is_empty());
    let text = match (upgrade_available, refresh_available) {
        (true, true) => "update and refresh available",
        (false, true) => "refresh available",
        _ => "update available",
    };
    (text, "↑")
}

pub(super) fn shell_inventory_category(target: &str) -> Option<String> {
    let remainder = target.strip_prefix("shell/")?;
    let (category, command) = remainder.split_once('/')?;
    (!category.is_empty() && !command.is_empty()).then(|| category.to_string())
}

pub(super) fn installed_shell_categories_from_inventory(report: &InventoryReportV1) -> Vec<String> {
    report
        .items
        .iter()
        .filter(|item| item.available && item.installed)
        .filter_map(|item| shell_inventory_category(&item.target))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn installed_app_categories_from_inventory(report: &InventoryReportV1) -> Vec<String> {
    report
        .items
        .iter()
        .filter(|item| item.installed)
        .filter_map(|item| item.target.strip_prefix("app/").map(str::to_string))
        .collect()
}

pub(super) fn installed_sys_items_from_inventory(report: &InventoryReportV1) -> Vec<String> {
    report
        .items
        .iter()
        .filter(|item| item.installed)
        .filter_map(|item| item.target.strip_prefix("sys/").map(str::to_string))
        .collect()
}

pub(super) fn shell_categories(rows: &[&ShellRow]) -> Vec<String> {
    rows.iter()
        .map(|row| row.category.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) struct ShellLifecycleStatus {
    pub category: String,
    pub detail_label: String,
    pub selector: String,
    pub status: ShellRowStatus,
    pub status_text: &'static str,
}

impl From<&ShellRow> for ShellLifecycleStatus {
    fn from(row: &ShellRow) -> Self {
        Self {
            category: row.category.clone(),
            detail_label: row.label.clone(),
            selector: row.selector(),
            status: row.status(),
            status_text: row.status_text,
        }
    }
}

pub(super) fn shell_category_statuses(rows: &[&ShellRow]) -> Vec<ShellLifecycleStatus> {
    let mut grouped: BTreeMap<&str, Vec<&ShellRow>> = BTreeMap::new();
    for row in rows {
        grouped.entry(&row.category).or_default().push(row);
    }
    grouped
        .into_iter()
        .filter_map(|(category, rows)| {
            if !rows.iter().any(|row| row.is_installed) {
                return None;
            }
            if rows.len() == 1 {
                let row = rows[0];
                return Some(ShellLifecycleStatus::from(row));
            }
            let selected = rows
                .iter()
                .filter(|row| row.is_installed)
                .max_by_key(|row| row.status().priority())
                .expect("grouped shell category is non-empty");
            let partially_installed = rows.iter().any(|row| !row.is_installed);
            let (status, status_text) =
                if partially_installed && selected.status() == ShellRowStatus::UpToDate {
                    (ShellRowStatus::Partial, "partial install")
                } else {
                    (selected.status(), selected.status_text)
                };
            Some(ShellLifecycleStatus {
                category: category.to_string(),
                detail_label: category.to_string(),
                selector: category.to_string(),
                status,
                status_text,
            })
        })
        .collect()
}

pub(super) struct AppLifecycleStatus {
    pub category: String,
    pub detail_label: String,
    pub sym: &'static str,
    pub status_text: &'static str,
    pub file_status: FileStatus,
    pub dest: Option<String>,
    pub upgrade_available: bool,
    pub refresh_sources: Vec<String>,
}

pub(super) fn app_category_statuses(rows: &[&AppRow]) -> Vec<AppLifecycleStatus> {
    let mut grouped: BTreeMap<&str, Vec<&AppRow>> = BTreeMap::new();
    for row in rows {
        grouped.entry(&row.category).or_default().push(row);
    }
    grouped
        .into_iter()
        .filter_map(|(category, rows)| {
            let has_installed = rows
                .iter()
                .any(|row| row.file_status != FileStatus::NotInstalled);
            if !has_installed {
                return None;
            }
            if rows.len() == 1 {
                let row = rows[0];
                return Some(AppLifecycleStatus {
                    category: category.to_string(),
                    detail_label: row.label.clone(),
                    sym: row.sym,
                    status_text: row.status_text,
                    file_status: row.file_status,
                    dest: row.dest.clone(),
                    upgrade_available: row.upgrade_available,
                    refresh_sources: row.refresh_sources.clone(),
                });
            }
            let has_not_installed = rows
                .iter()
                .any(|row| row.file_status == FileStatus::NotInstalled);
            let installed_max = rows
                .iter()
                .map(|row| row.file_status)
                .filter(|status| *status != FileStatus::NotInstalled)
                .max()
                .expect("installed app category has an installed row");
            let status = if has_not_installed && installed_max == FileStatus::UpToDate {
                FileStatus::Partial
            } else {
                installed_max
            };
            let (sym, mut status_text) = match status {
                FileStatus::Missing => ("!", "destination missing"),
                FileStatus::UserModified => ("~", "user modified"),
                FileStatus::Partial => ("~", "partial install"),
                FileStatus::UpdateAvail => ("↑", "update available"),
                FileStatus::GeneratorNotEvaluated => ("!", "generator not evaluated"),
                FileStatus::GeneratorEvaluationFailed => ("!", "generator evaluation failed"),
                FileStatus::GeneratorTrustRequired => ("!", "generator trust required"),
                FileStatus::UpToDate => ("✓", "up-to-date"),
                FileStatus::NotInstalled => unreachable!(),
            };
            let upgrade_available = rows.iter().any(|row| row.upgrade_available);
            let refresh_sources = rows
                .iter()
                .flat_map(|row| row.refresh_sources.iter().cloned())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            if status == FileStatus::UpdateAvail {
                status_text = match (upgrade_available, refresh_sources.is_empty()) {
                    (true, false) => "update and refresh available",
                    (false, false) => "refresh available",
                    _ => status_text,
                };
            }
            Some(AppLifecycleStatus {
                category: category.to_string(),
                detail_label: category.to_string(),
                sym,
                status_text,
                file_status: status,
                dest: None,
                upgrade_available,
                refresh_sources,
            })
        })
        .collect()
}

pub(super) fn sorted_names(mut names: Vec<String>) -> Vec<String> {
    names.sort_by(|left, right| {
        left.to_lowercase()
            .cmp(&right.to_lowercase())
            .then_with(|| left.cmp(right))
    });
    names
}

pub(super) struct GeneratorAttention<'a> {
    pub not_evaluated: bool,
    pub failures: Vec<&'a AppRow>,
}
impl<'a> GeneratorAttention<'a> {
    pub fn from_rows(rows: &'a [AppRow]) -> Self {
        Self {
            not_evaluated: rows
                .iter()
                .any(|row| row.file_status == FileStatus::GeneratorNotEvaluated),
            failures: rows
                .iter()
                .filter(|row| {
                    matches!(
                        row.file_status,
                        FileStatus::GeneratorEvaluationFailed | FileStatus::GeneratorTrustRequired
                    )
                })
                .collect(),
        }
    }
    pub fn has_attention(&self) -> bool {
        self.not_evaluated || self.has_failures()
    }
    pub fn has_failures(&self) -> bool {
        !self.failures.is_empty()
    }
}

pub(super) struct UpdateView<'a> {
    pub update_shell: Vec<&'a ShellRow>,
    pub update_app: BTreeMap<&'a str, Vec<&'a AppRow>>,
    pub actionable_app: BTreeMap<&'a str, Vec<&'a AppRow>>,
    pub refresh_commands: Vec<String>,
    pub shell_attention: Vec<&'a ShellRow>,
    pub any_update: bool,
}
impl<'a> UpdateView<'a> {
    pub fn new(
        shell_rows: &'a [ShellRow],
        shell_lifecycle: &shine_core::lifecycle::LifecycleResultV1,
        app_rows: &'a [AppRow],
        app_lifecycle: &shine_core::lifecycle::LifecycleResultV1,
        has_sys_updates: bool,
    ) -> Self {
        let pending_shell = shell_lifecycle
            .outcomes
            .iter()
            .filter(|outcome| outcome.status == shine_core::lifecycle::LifecycleStatus::Pending)
            .map(|outcome| outcome.target.as_str())
            .collect::<BTreeSet<_>>();
        let update_shell = pending_shell_rows(shell_rows, &pending_shell);

        let pending_app = app_lifecycle
            .outcomes
            .iter()
            .filter(|outcome| outcome.status == shine_core::lifecycle::LifecycleStatus::Pending)
            .map(|outcome| outcome.target.as_str())
            .collect::<BTreeSet<_>>();
        let update_app_rows: Vec<&AppRow> = app_rows
            .iter()
            .filter(|r| {
                r.upgrade_available && pending_app.contains(format!("app/{}", r.category).as_str())
            })
            .collect();
        let update_app = app_update_categories(&update_app_rows);
        let actionable_app_rows = app_rows
            .iter()
            .filter(|row| {
                (row.upgrade_available || !row.refresh_sources.is_empty())
                    && pending_app.contains(format!("app/{}", row.category).as_str())
            })
            .collect::<Vec<_>>();
        let actionable_app = app_update_categories(&actionable_app_rows);
        let refresh_commands = app_refresh_commands(&actionable_app);
        let shell_attention = shell_rows
            .iter()
            .filter(|row| row.is_installed && (row.link_conflict || row.preset_missing))
            .collect::<Vec<_>>();
        let any_update = !update_shell.is_empty() || !actionable_app.is_empty() || has_sys_updates;
        Self {
            update_shell,
            update_app,
            actionable_app,
            refresh_commands,
            shell_attention,
            any_update,
        }
    }
}
