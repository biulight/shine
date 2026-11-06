//! CLI presentation of already captured App generator inspections.
use crate::{colors, config::Config, output, status::FileStatus};
use shine_core::runtime::AppFileInspection;
use std::collections::BTreeMap;

/// Counts from the captured file inspections, independent of category/file row presentation.
#[derive(Debug, Default)]
pub(crate) struct GeneratorCheckFailure {
    trust: usize,
    failed: usize,
    pub(crate) files_by_category: BTreeMap<String, usize>,
}

impl std::fmt::Display for GeneratorCheckFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Configuration check incomplete")?;
        match self.trust {
            0 => (),
            1 => f.write_str(" · 1 file requires generator trust")?,
            n => write!(f, " · {n} files require generator trust")?,
        }
        match self.failed {
            0 => (),
            1 => f.write_str(" · generator evaluation failed for 1 file")?,
            n => write!(f, " · generator evaluation failed for {n} files")?,
        }
        if self.files_by_category.len() == 1
            && let Some((category, _)) = self.files_by_category.first_key_value()
        {
            write!(f, " (app/{})", output::escape_controls(category))?;
        }
        Ok(())
    }
}

impl std::error::Error for GeneratorCheckFailure {}

pub(crate) fn file_unit(count: usize) -> &'static str {
    if count == 1 { "file" } else { "files" }
}

pub(crate) fn generator_failed(file: &AppFileInspection) -> bool {
    matches!(
        file.status,
        FileStatus::GeneratorTrustRequired | FileStatus::GeneratorEvaluationFailed
    )
}

fn same_generator_report(left: &AppFileInspection, right: &AppFileInspection) -> bool {
    match (&left.code_analysis, &right.code_analysis) {
        (Some(a), Some(b)) => crate::code_analysis::same_advisory_report(a, b),
        _ => false,
    }
}

fn generator_groups<'a>(
    files: impl IntoIterator<Item = &'a AppFileInspection>,
) -> Vec<Vec<&'a AppFileInspection>> {
    let mut groups: Vec<Vec<&AppFileInspection>> = Vec::new();
    for file in files {
        if let Some(group) = groups
            .iter_mut()
            .find(|group| same_generator_report(group[0], file))
        {
            group.push(file);
        } else {
            groups.push(vec![file]);
        }
    }
    groups
}

fn lint_hint(config: &Config, file: &AppFileInspection) -> String {
    // Lint reads one directory. Only the external base without a configured overlay
    // identifies a single on-disk tree for the entire preset (including imports).
    if config.is_external_presets
        && config.presets_overlay_dir_override.is_none()
        && config.presets_overlay_git.is_none()
    {
        let path = config.presets_dir().join("app").join(&file.category.name);
        if let Some(path) = path
            .to_str()
            .filter(|p| !p.starts_with('-') && !p.chars().any(char::is_control))
        {
            return format!(
                "\n  All locations for a separate preset source check:\n    shine preset lint {} --format json",
                crate::shell_quote::command_arg(path)
            );
        }
    }
    "\n  For a separate source check, replace <PATH> with a complete preset source tree including any effective overlay files:\n    shine preset lint <PATH> --format json".into()
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum GeneratorView {
    CompactList,
    VerboseList,
    CompactTarget,
    VerboseTarget,
}

impl GeneratorView {
    fn verbose(self) -> bool {
        matches!(self, Self::VerboseList | Self::VerboseTarget)
    }

    fn targeted(self) -> bool {
        matches!(self, Self::CompactTarget | Self::VerboseTarget)
    }

    fn failure(self, file: &AppFileInspection) -> Option<FileStatus> {
        if self.targeted() {
            // The installed file's status can be user-modified or missing even if its
            // generator failed. Targeted update has always reported that assessment.
            match file.assessment_diagnostic {
                Some("app_generator_trust_required") => Some(FileStatus::GeneratorTrustRequired),
                Some("app_generator_evaluation_failed") => {
                    Some(FileStatus::GeneratorEvaluationFailed)
                }
                _ => None,
            }
        } else {
            generator_failed(file).then_some(file.status)
        }
    }
}

fn print_generator_errors(
    files: &[AppFileInspection],
    config: &Config,
    view: GeneratorView,
) -> GeneratorCheckFailure {
    let mut summary = GeneratorCheckFailure::default();
    let mut categories = BTreeMap::new();
    for file in files {
        let Some(status) = view.failure(file) else {
            continue;
        };
        categories
            .entry((&file.category.name, status))
            .or_insert_with(Vec::new)
            .push(file);
    }
    for ((category, status), failures) in categories {
        let target = format!("app/{category}");
        let trust = status == FileStatus::GeneratorTrustRequired;
        let reason = if trust {
            use shine_core::runtime::PresetSourceKind::{External, Overlay};
            let contributes = |kind| {
                failures
                    .iter()
                    .any(|file| file.generator_trust_sources.contains(&kind))
            };
            match (contributes(External), contributes(Overlay)) {
                (true, true) => {
                    "uses external preset and overlay files and requires generator trust"
                }
                (true, false) => "uses an external preset source and requires generator trust",
                (false, true) => "includes overlay files and requires generator trust",
                (false, false) => "requires generator trust",
            }
        } else {
            "generator evaluation failed"
        };
        let count = failures.len();
        if view != GeneratorView::CompactList {
            *summary
                .files_by_category
                .entry(category.clone())
                .or_default() += count;
            if trust {
                summary.trust += count;
            } else {
                summary.failed += count;
            }
            eprintln!(
                "{} {} · {count} generated {} could not be checked",
                colors::red_stderr("!"),
                colors::bold_stderr(&output::escape_controls(&target)),
                file_unit(count)
            );
            if trust {
                eprintln!("  This category {reason} for the current snapshot.");
            } else {
                eprintln!("  Generator evaluation failed.");
            }
        } else {
            eprintln!(
                "{} {} {reason}. Could not check {count} {} for updates.",
                colors::red_stderr("Error:"),
                colors::bold_stderr(&output::escape_controls(&target)),
                file_unit(count)
            );
        }
        let review = (trust && !target.chars().any(char::is_control)).then(|| {
            format!(
                "  Review the category before granting trust:\n    shine trust inspect {}",
                crate::shell_quote::command_arg(&target)
            )
        });
        if view == GeneratorView::VerboseList
            && let Some(review) = &review
        {
            eprintln!("{review}");
        }
        let category_file = failures[0];
        for (index, group) in generator_groups(failures).into_iter().enumerate() {
            let first = group[0];
            if view != GeneratorView::CompactTarget
                && let Some(generator) = &first.file.generator
            {
                eprintln!(
                    "\n  {}  {}",
                    colors::dim_for_stream("Generator", owo_colors::Stream::Stderr),
                    output::escape_controls(&generator.script.to_string_lossy())
                );
            }
            for (index, file) in group.iter().enumerate() {
                let label = if index == 0 {
                    colors::dim_for_stream("Files", owo_colors::Stream::Stderr)
                } else {
                    "     ".to_owned()
                };
                eprintln!(
                    "  {label}      {}",
                    output::escape_controls(&file.file.source_rel.to_string_lossy())
                );
                if view.targeted()
                    && !trust
                    && let Some(error) = &file.assessment_error
                {
                    eprintln!(
                        "             {}",
                        colors::dim_for_stream(
                            &output::escape_controls(error),
                            owo_colors::Stream::Stderr
                        )
                    );
                }
            }
            if index == 0
                && view.targeted()
                && let Some(review) = &review
            {
                eprintln!("\n{review}");
            }
            if let Some(scope) = &first.code_analysis {
                let summary = match view {
                    GeneratorView::CompactTarget
                        if scope.report.diagnostics.iter().any(|d| d.blocking()) =>
                    {
                        crate::code_analysis::lines(scope, false)
                    }
                    GeneratorView::CompactTarget => Vec::new(),
                    GeneratorView::CompactList => crate::code_analysis::update_summary(scope),
                    _ => crate::code_analysis::verbose_generator_summary(scope, group.len()),
                };
                if !summary.is_empty() {
                    eprintln!();
                }
                for line in summary {
                    eprintln!("{line}");
                }
            }
        }
        if view.verbose() {
            eprintln!("{}\n", lint_hint(config, category_file));
        } else if view == GeneratorView::CompactList
            && trust
            && !target.chars().any(char::is_control)
        {
            let argument = crate::shell_quote::command_arg(&target);
            eprintln!(
                "\nReview all capabilities for this category:\n  shine trust inspect {argument}\nIf you trust every listed generator, hook and artifact, grant trust for this snapshot and retry:\n  shine trust grant {argument}\n  shine update --run-generators"
            );
        }
    }
    summary
}

pub(crate) fn print_target_generator_review(
    config: &Config,
    files: &[AppFileInspection],
    verbose: bool,
) -> Option<GeneratorCheckFailure> {
    let view = if verbose {
        GeneratorView::VerboseTarget
    } else {
        GeneratorView::CompactTarget
    };
    let failure = print_generator_review(config, files, view);
    (!failure.files_by_category.is_empty()).then_some(failure)
}

pub(crate) fn print_generator_review(
    config: &Config,
    files: &[AppFileInspection],
    view: GeneratorView,
) -> GeneratorCheckFailure {
    let failure = print_generator_errors(files, config, view);
    if view.verbose() {
        for group in generator_groups(
            files
                .iter()
                .filter(|file| view.failure(file).is_none() && file.code_analysis.is_some()),
        ) {
            let first = group[0];
            let scope = first.code_analysis.as_ref().expect("filtered above");
            println!(
                "  Generator  {} · {}",
                output::escape_controls(&scope.target),
                output::escape_controls(scope.script.as_deref().unwrap_or("native command"))
            );
            for file in &group {
                println!(
                    "    {}",
                    output::escape_controls(&file.file.source_rel.to_string_lossy())
                );
            }
            for line in crate::code_analysis::verbose_generator_summary(scope, group.len()) {
                println!("{line}");
            }
            println!("{}\n", lint_hint(config, first));
        }
    }
    failure
}
