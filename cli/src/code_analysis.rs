//! Safe, display-only projection shared by runtime review and authoring tools.
use shine_core::analysis::{Diagnostic, DiagnosticKind, Scope};
use std::collections::BTreeMap;

pub(crate) fn same_advisory_report(left: &Scope, right: &Scope) -> bool {
    left.script.is_some()
        && left.script == right.script
        && left.target == right.target
        && left.report == right.report
        && !left.report.diagnostics.iter().any(Diagnostic::blocking)
}

pub(crate) fn plan_lines<'a>(
    scopes: impl Iterator<Item = &'a Scope>,
    detailed: bool,
) -> Vec<String> {
    if detailed {
        return scopes.flat_map(|scope| lines(scope, true)).collect();
    }
    let mut groups: Vec<(&Scope, Vec<&str>)> = Vec::new();
    for scope in scopes {
        if let Some((_, entries)) = groups
            .iter_mut()
            .find(|(first, _)| same_advisory_report(first, scope))
        {
            entries.push(&scope.entry);
        } else {
            groups.push((scope, vec![&scope.entry]));
        }
    }
    groups
        .into_iter()
        .flat_map(|(scope, entries)| entry_lines(scope, false, &entries, "--verbose"))
        .collect()
}

pub(crate) fn lines(scope: &Scope, detailed: bool) -> Vec<String> {
    entry_lines(scope, detailed, &[&scope.entry], "--verbose")
}

fn entry_lines(
    scope: &Scope,
    detailed: bool,
    entries: &[&str],
    full_location_flags: &str,
) -> Vec<String> {
    let diagnostics = scope
        .report
        .diagnostics
        .iter()
        .filter(|d| detailed || d.kind != DiagnosticKind::Uncorroborated)
        .collect::<Vec<_>>();
    if diagnostics.is_empty() && !detailed {
        return Vec::new();
    }
    let encode = crate::output::escape_controls;
    let mut lines = vec![format!(
        "  Code analysis: {} · {} · {}",
        encode(&scope.target),
        entries
            .iter()
            .map(|entry| encode(entry))
            .collect::<Vec<_>>()
            .join(", "),
        encode(scope.script.as_deref().unwrap_or("native command"))
    )];
    if detailed {
        lines.push(format!(
            "    Scope: {} source files; this is not a runtime sandbox.",
            scope.report.files.len()
        ));
    }
    if diagnostics
        .iter()
        .any(|d| d.kind == DiagnosticKind::CoverageGap)
    {
        lines.push("    Coverage notes show behavior static analysis could not verify.".into());
    }
    let mut gaps = BTreeMap::new();
    if !detailed {
        for diagnostic in &diagnostics {
            if diagnostic.kind == DiagnosticKind::CoverageGap {
                *gaps
                    .entry((diagnostic.code.as_str(), diagnostic.behavior.as_str()))
                    .or_insert(0usize) += 1;
            }
        }
    }
    let mut summarized = false;
    for diagnostic in diagnostics {
        if !detailed && diagnostic.kind == DiagnosticKind::CoverageGap {
            let Some(count) =
                gaps.remove(&(diagnostic.code.as_str(), diagnostic.behavior.as_str()))
            else {
                continue;
            };
            if count > 1 {
                lines.push(format!(
                    "    note[{}] {count} diagnostics ({}); example: {}:{}:{}",
                    encode(&diagnostic.code),
                    encode(&diagnostic.behavior),
                    encode(&diagnostic.file),
                    diagnostic.line,
                    diagnostic.column,
                ));
                summarized = true;
                continue;
            }
        }
        let severity = if diagnostic.blocking() {
            "error"
        } else {
            "note"
        };
        lines.push(format!(
            "    {severity}[{}] {}:{}:{} ({})",
            encode(&diagnostic.code),
            encode(&diagnostic.file),
            diagnostic.line,
            diagnostic.column,
            encode(&diagnostic.behavior)
        ));
        if diagnostic.kind != DiagnosticKind::CoverageGap {
            lines.push(format!("      {}", diagnostic.explanation()));
        }
        if detailed && let Some(identity) = &diagnostic.identity {
            lines.push(format!("      Target identity: {}", encode(identity)));
        }
    }
    if summarized {
        lines.push(format!("    Full locations: re-run with {full_location_flags}. A separate `shine preset lint <PATH> --format json` checks all source entries."));
    }
    lines
}

/// Short explanation for an already identified generator. Keep blocking diagnostics individual.
pub(crate) fn update_summary(scope: &Scope) -> Vec<String> {
    if scope.report.diagnostics.iter().any(|d| d.blocking()) {
        return entry_lines(scope, false, &[&scope.entry], "--verbose --diff");
    }
    let mut counts = [0usize; 3];
    for diagnostic in &scope.report.diagnostics {
        if diagnostic.kind != DiagnosticKind::CoverageGap {
            continue;
        }
        let index = match (diagnostic.code.as_str(), diagnostic.behavior.as_str()) {
            ("code_analysis_builtin_unsupported", "dependency") => 0,
            ("code_analysis_call_unknown", "code") => 1,
            ("code_analysis_dynamic_target", "network") => 2,
            _ => return entry_lines(scope, false, &[&scope.entry], "--verbose --diff"),
        };
        counts[index] += 1;
    }
    let mut items = Vec::new();
    for (count, (singular, plural)) in counts.into_iter().zip([
        ("a dependency", "dependencies"),
        ("a call", "calls"),
        ("a network target", "network targets"),
    ]) {
        match count {
            0 => (),
            1 => items.push(singular.to_owned()),
            _ => items.push(format!("{count} {plural}")),
        }
    }
    let description = match items.as_slice() {
        [] => return Vec::new(),
        [item] => item.clone(),
        [a, b] => format!("{a} and {b}"),
        _ => format!("{}, {}, and {}", items[0], items[1], items[2]),
    };
    vec![
        format!("  Static analysis could not verify {description}."),
        "  Run `shine update --run-generators --verbose` for a summary with example locations."
            .into(),
    ]
}

/// Summarize advisory evidence already captured for this invocation. For a blocking
/// report, keep every diagnostic and its entry identity available for repair.
pub(crate) fn verbose_generator_summary(scope: &Scope, entries: usize) -> Vec<String> {
    if scope.report.diagnostics.iter().any(Diagnostic::blocking) {
        return lines(scope, true);
    }
    let shared = if entries > 1 {
        " · same report for these files"
    } else {
        ""
    };
    let mut result = vec![format!("  Static analysis{shared} · 0 blocking findings")];
    let mut groups = BTreeMap::new();
    for diagnostic in &scope.report.diagnostics {
        let (order, label, singular, plural, detail) =
            match (diagnostic.code.as_str(), diagnostic.behavior.as_str()) {
                ("code_analysis_dynamic_target", "network") => {
                    (0, "Network", "target", "targets", "determined at runtime")
                }
                ("code_analysis_call_unknown", "code") => {
                    (1, "Calls", "call", "calls", "outside verified coverage")
                }
                ("code_analysis_builtin_unsupported", "dependency") => (
                    2,
                    "Dependency",
                    "dependency",
                    "dependencies",
                    "outside verified coverage",
                ),
                ("code_analysis_declaration_uncorroborated", _) => (
                    3,
                    "Declaration",
                    "permission declaration",
                    "permission declarations",
                    "unconfirmed",
                ),
                (code, behavior) => (4, behavior, "location", "locations", code),
            };
        let (count, _) = groups
            .entry((order, label, singular, plural, detail))
            .or_insert((0usize, diagnostic));
        *count += 1;
    }
    let encode = crate::output::escape_controls;
    for ((_, label, singular, plural, detail), (count, first)) in groups {
        result.push(format!(
            "    {:<12} {count} {} {} · {}:{}:{}",
            encode(label),
            if count == 1 { singular } else { plural },
            encode(detail),
            encode(&first.file),
            first.line,
            first.column
        ));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matching_is_silent_and_differences_are_actionable_and_encoded() {
        let mut scope = Scope {
            target: "app/demo\nFORGED".into(),
            entry: "generator".into(),
            script: Some("gen.ts".into()),
            report: shine_core::analysis::Report {
                analyzer_version: shine_core::analysis::ANALYZER_VERSION.into(),
                schema_version: 1,
                rule_version: 1,
                files: vec!["gen.ts".into()],
                allowed: true,
                diagnostics: Vec::new(),
            },
        };
        assert!(lines(&scope, false).is_empty());
        scope.report.allowed = false;
        scope
            .report
            .diagnostics
            .push(shine_core::analysis::Diagnostic {
                kind: DiagnosticKind::Difference,
                code: "code_analysis_declaration_difference".into(),
                file: "gen.ts".into(),
                line: 2,
                column: 3,
                behavior: "network".into(),
                identity: None,
            });
        let rendered = lines(&scope, false);
        assert!(rendered.iter().all(|line| !line.contains('\n')));
        let text = rendered.join("\n");
        assert!(text.contains("gen.ts:2:3"));
        assert!(text.contains("add an accurate declaration or change the script"));
    }

    #[test]
    fn plan_groups_identical_advisories_but_keeps_entry_specific_evidence() {
        use shine_core::analysis::Report;
        let first = Scope {
            target: "app/demo".into(),
            entry: "generator:first".into(),
            script: Some("gen.ts".into()),
            report: Report {
                analyzer_version: shine_core::analysis::ANALYZER_VERSION.into(),
                schema_version: 1,
                rule_version: shine_core::analysis::RULE_VERSION,
                files: vec!["gen.ts".into()],
                allowed: true,
                diagnostics: vec![Diagnostic {
                    kind: DiagnosticKind::CoverageGap,
                    code: "code_analysis_call_unknown".into(),
                    file: "gen.ts".into(),
                    line: 2,
                    column: 3,
                    behavior: "code".into(),
                    identity: None,
                }],
            },
        };
        let mut second = first.clone();
        second.entry = "generator:second\nFORGED".into();
        let mut distinct = first.clone();
        distinct.report.diagnostics[0].line = 8;
        let scopes = [&first, &distinct, &second];
        let compact = plan_lines(scopes.into_iter(), false).join("\n");
        assert!(compact.contains(r"generator:first, generator:second\nFORGED"));
        assert_eq!(
            compact.matches("note[code_analysis_call_unknown]").count(),
            2
        );
        assert!(compact.contains("gen.ts:2:3") && compact.contains("gen.ts:8:3"));
        assert!(!compact.contains("trust"));
        let verbose = plan_lines(scopes.into_iter(), true).join("\n");
        assert_eq!(
            verbose.matches("note[code_analysis_call_unknown]").count(),
            3
        );

        distinct.report.diagnostics[0].kind = DiagnosticKind::Failure;
        assert!(!same_advisory_report(&distinct, &distinct));
        second.target = "app/other".into();
        assert!(!same_advisory_report(&first, &second));
    }

    #[test]
    fn compact_gaps_keep_counts_kinds_errors_and_full_report() {
        use shine_core::analysis::{Diagnostic, Report};
        let diagnostic = |kind, code: &str, behavior: &str, line| Diagnostic {
            kind,
            code: code.into(),
            file: "gen\nFORGED.ts".into(),
            line,
            column: 1,
            behavior: behavior.into(),
            identity: Some("opaque-identity".into()),
        };
        let mut scope = Scope {
            target: "app/demo".into(),
            entry: "generator:first".into(),
            script: Some("gen\nFORGED.ts".into()),
            report: Report {
                analyzer_version: shine_core::analysis::ANALYZER_VERSION.into(),
                schema_version: 1,
                rule_version: shine_core::analysis::RULE_VERSION,
                files: vec!["gen\nFORGED.ts".into()],
                allowed: false,
                diagnostics: (1..=4)
                    .map(|line| {
                        diagnostic(
                            DiagnosticKind::CoverageGap,
                            "code_analysis_dynamic_target",
                            "network\nFORGED",
                            line,
                        )
                    })
                    .collect(),
            },
        };
        scope.report.diagnostics.extend([
            diagnostic(
                DiagnosticKind::CoverageGap,
                "code_analysis_dynamic_target",
                "file-read",
                5,
            ),
            diagnostic(
                DiagnosticKind::CoverageGap,
                "code_analysis_subprocess_effects",
                "subprocess",
                6,
            ),
            diagnostic(
                DiagnosticKind::Difference,
                "code_analysis_declaration_difference",
                "network",
                7,
            ),
            diagnostic(
                DiagnosticKind::Difference,
                "code_analysis_declaration_difference",
                "network",
                8,
            ),
            diagnostic(
                DiagnosticKind::Failure,
                "code_analysis_parse_failure",
                "syntax",
                9,
            ),
            diagnostic(
                DiagnosticKind::Uncorroborated,
                "code_analysis_declaration_uncorroborated",
                "file-write",
                0,
            ),
        ]);
        let before = serde_json::to_value(&scope).unwrap();
        let compact = lines(&scope, false);
        assert!(compact.iter().all(|line| !line.contains('\n')));
        let text = compact.join("\n");
        assert!(text.contains("4 diagnostics (network\\nFORGED)"), "{text}");
        assert_eq!(
            text.matches("note[code_analysis_dynamic_target]").count(),
            2
        );
        assert!(text.contains("code_analysis_subprocess_effects"));
        assert_eq!(
            text.matches("error[code_analysis_declaration_difference]")
                .count(),
            2
        );
        assert!(text.contains("error[code_analysis_parse_failure]"));
        assert_eq!(
            text.matches("behavior static analysis could not verify")
                .count(),
            1
        );
        assert!(text.contains("--format json"));
        assert!(!text.contains("opaque-identity"));
        assert!(!text.contains("code_analysis_declaration_uncorroborated"));
        let detailed = lines(&scope, true).join("\n");
        assert_eq!(
            detailed
                .matches("note[code_analysis_dynamic_target]")
                .count(),
            5
        );
        assert!(detailed.contains("code_analysis_declaration_uncorroborated"));
        assert_eq!(
            detailed.matches("Target identity: opaque-identity").count(),
            10
        );
        assert!(!detailed.contains("4 diagnostics"));
        assert_eq!(serde_json::to_value(&scope).unwrap(), before);

        scope.entry = "generator:second".into();
        let second = lines(&scope, false).join("\n");
        assert!(second.contains("generator:second"));
        assert!(second.contains("4 diagnostics"));
    }
}
