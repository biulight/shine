//! Safe, display-only projection shared by runtime review and authoring tools.
use shine_core::analysis::{DiagnosticKind, Scope};
use std::collections::BTreeMap;

pub(crate) fn lines(scope: &Scope, detailed: bool) -> Vec<String> {
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
        encode(&scope.entry),
        encode(scope.script.as_deref().unwrap_or("native command"))
    )];
    if detailed {
        lines.push(format!(
            "    Scope: {} source files; this is not a runtime sandbox.",
            scope.report.files.len()
        ));
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
                lines.push(format!("      {}", diagnostic.explanation()));
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
        lines.push(format!("      {}", diagnostic.explanation()));
        if detailed && let Some(identity) = &diagnostic.identity {
            lines.push(format!("      Target identity: {}", encode(identity)));
        }
    }
    if summarized {
        lines.push("    Full locations: re-run with --verbose. A separate `shine preset lint <PATH> --format json` checks all source entries.".into());
    }
    lines
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
        assert!(text.contains("existing code trust is still required"));
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
