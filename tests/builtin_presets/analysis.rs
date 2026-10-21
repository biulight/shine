//! Real source samples complement the small manually labelled rule fixtures.
use shine_core::analysis::{DiagnosticKind, EnvironmentContract, Session};
use shine_core::runtime::RuntimePlatform;
use std::collections::BTreeMap;

#[test]
fn builtin_bun_samples_parse_deterministically_with_explicit_coverage_gaps() {
    let runtime = super::support::runtime(RuntimePlatform::Macos);
    let mut session = Session::default();
    let mut sample_count = 0;
    for category in ["surge", "clash-verge"] {
        let prefix = format!("app/{category}/");
        let sources = runtime
            .presets()
            .files()
            .iter()
            .filter_map(|(path, bytes)| {
                path.strip_prefix(&prefix)
                    .map(|relative| (relative.to_string(), bytes.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        for path in sources
            .keys()
            .filter(|p| p.ends_with(".ts") && !p.ends_with(".test.ts"))
        {
            let first = session
                .analyze(&sources, path, None, &EnvironmentContract::default())
                .unwrap();
            let parsed = session.parsed_files();
            let second = session
                .analyze(&sources, path, None, &EnvironmentContract::default())
                .unwrap();
            assert_eq!(first, second);
            assert_eq!(
                parsed,
                session.parsed_files(),
                "source cache was not reused"
            );
            assert!(
                !first
                    .diagnostics
                    .iter()
                    .any(|d| d.kind == DiagnosticKind::Failure),
                "{category}/{path}: {first:?}"
            );
            // These real scripts include dynamic paths/URLs and function calls;
            // parsing their syntax must not turn that into complete coverage.
            assert!(
                first
                    .diagnostics
                    .iter()
                    .any(|d| d.kind == DiagnosticKind::CoverageGap),
                "{category}/{path}"
            );
            println!(
                "{category}/{path}: bytes={}, files={}, differences={}, gaps={}",
                sources[path].len(),
                first.files.len(),
                first
                    .diagnostics
                    .iter()
                    .filter(|d| d.kind == DiagnosticKind::Difference)
                    .count(),
                first
                    .diagnostics
                    .iter()
                    .filter(|d| d.kind == DiagnosticKind::CoverageGap)
                    .count()
            );
            sample_count += 1;
        }
    }
    assert_eq!(sample_count, 6);
}
