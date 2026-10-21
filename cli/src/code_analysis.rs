//! Safe, display-only projection shared by runtime review and authoring tools.
use shine_core::analysis::{DiagnosticKind, Scope};

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
    for diagnostic in diagnostics {
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
}
