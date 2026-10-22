use super::*;

fn sources(code: &str) -> BTreeMap<String, Vec<u8>> {
    BTreeMap::from([("main.ts".into(), code.as_bytes().to_vec())])
}
fn declaration(body: &str) -> PermissionDeclarationV1 {
    toml::from_str(&format!("schema_version = 1\n{body}")).unwrap()
}
fn report(code: &str, declaration: Option<&PermissionDeclarationV1>) -> Report {
    Session::default()
        .analyze(
            &sources(code),
            "main.ts",
            declaration,
            &EnvironmentContract::default(),
        )
        .unwrap()
}
fn has(report: &Report, code: &str) -> bool {
    report.diagnostics.iter().any(|d| d.code == code)
}
fn differences(report: &Report) -> usize {
    report
        .diagnostics
        .iter()
        .filter(|d| d.kind == DiagnosticKind::Difference)
        .count()
}

#[test]
fn literal_network_matching_is_silent_and_difference_is_located() {
    let code = "// fetch('https://comment.invalid')\nawait fetch('https://example.org/path');";
    let matching = report(
        code,
        Some(&declaration(
            "network = [{ scope = 'host', host = 'example.org' }]",
        )),
    );
    assert!(matching.allowed);
    assert!(matching.diagnostics.is_empty(), "{matching:?}");
    let missing = report(code, None);
    assert!(!missing.allowed);
    assert_eq!(differences(&missing), 1);
    assert_eq!(
        (missing.diagnostics[0].line, missing.diagnostics[0].column),
        (2, 7)
    );
    assert!(report("const unused = `fetch('https://fake.invalid')`;", None).allowed);
}

#[test]
fn builtin_alias_and_parameter_shadowing_are_distinct() {
    let r = report(
        "import { readFileSync as read } from 'node:fs';\nread('config');\nfunction local(read: any) { read('private'); }",
        None,
    );
    assert_eq!(differences(&r), 1, "{r:?}");
    assert!(has(&r, "code_analysis_call_unknown"));
    let shadow = report(
        "function run(fetch: any) { fetch('https://fake.invalid'); }",
        None,
    );
    assert!(shadow.allowed, "{shadow:?}");
    assert!(has(&shadow, "code_analysis_call_unknown"));
    let alias = report(
        "const request = fetch; request('https://example.org');",
        None,
    );
    assert_eq!(differences(&alias), 1, "{alias:?}");
    let mutated = report(
        "let request = fetch; request = other; request('https://example.org');",
        None,
    );
    assert!(mutated.allowed);
    assert!(has(&mutated, "code_analysis_call_unknown"));
}

#[test]
fn environment_mapping_is_per_entry_and_source_cache_is_shared() {
    let mut session = Session::default();
    let code = sources("const value = process.env.TARGET; const fixed = Bun.env.SHINE_APP_ID;");
    let env = EnvironmentContract {
        fixed: BTreeSet::from(["SHINE_APP_ID".into()]),
        mappings: BTreeMap::from([(
            "TARGET".into(),
            ("SOURCE".into(), Some(EnvironmentSensitivityV1::Plain)),
        )]),
    };
    let allowed = session
        .analyze(
            &code,
            "main.ts",
            Some(&declaration(
                "environment = [{ name = 'SOURCE', sensitivity = 'plain' }]",
            )),
            &env,
        )
        .unwrap();
    assert!(allowed.diagnostics.is_empty(), "{allowed:?}");
    let blocked = session
        .analyze(
            &code,
            "main.ts",
            Some(&declaration(
                "environment = [{ name = 'TARGET', sensitivity = 'plain' }]",
            )),
            &env,
        )
        .unwrap();
    assert!(!blocked.allowed);
    assert_eq!(differences(&blocked), 1);
    assert_eq!(session.parsed_files(), 1);
    let unknown = session
        .analyze(&code, "main.ts", None, &EnvironmentContract::default())
        .unwrap();
    assert!(unknown.allowed);
    assert!(has(&unknown, "code_analysis_identity_unknown"));
}

#[test]
fn environment_writes_and_deletes_are_not_reads() {
    let r = report(
        "process.env.TOKEN = 'new'; delete Bun.env.TOKEN; delete/*gap*/process.env.TOKEN; delete\nBun.env.TOKEN;",
        None,
    );
    assert!(!has(&r, "code_analysis_identity_unknown"), "{r:?}");
    let r = report("process.env.TOKEN += 'suffix'; ++Bun.env.TOKEN;", None);
    assert!(has(&r, "code_analysis_identity_unknown"), "{r:?}");
}

#[test]
fn subprocess_effects_remain_unknown_when_command_matches() {
    let code = "Bun.spawn(['curl', 'https://credential@private/path']);";
    let matching = report(code, Some(&declaration("commands = ['curl']")));
    assert!(matching.allowed);
    assert!(has(&matching, "code_analysis_subprocess_effects"));
    let missing = report(code, None);
    assert!(!missing.allowed);
    assert!(has(&missing, "code_analysis_subprocess_effects"));
    assert_eq!(differences(&missing), 1);
    let dynamic = report("Bun.spawn(command);", None);
    assert!(has(&dynamic, "code_analysis_dynamic_target"));
    assert!(has(&dynamic, "code_analysis_subprocess_effects"));
}

#[test]
fn filesystem_operations_and_bun_file_construction_are_separate() {
    let code = "import * as fs from 'node:fs'; fs.readFileSync('a'); fs.writeFileSync('b', 'x'); fs.rmSync('c'); Bun.file('unused'); await Bun.file('d').text();";
    let r = report(code, None);
    assert_eq!(differences(&r), 4, "{r:?}");
    let matched = report(
        code,
        Some(&declaration(
            "filesystem = [{ base = 'preset', path = 'a', access = ['read'] }, { base = 'preset', path = 'b', access = ['write'] }, { base = 'preset', path = 'c', access = ['remove'] }, { base = 'preset', path = 'd', access = ['read'] }]",
        )),
    );
    assert!(matched.diagnostics.is_empty(), "{matched:?}");
}

#[test]
fn relative_graph_handles_cycles_missing_and_ambiguous_dependencies() {
    let mut input =
        sources("import './a'; import './missing'; import '../escape'; import 'package';");
    input.insert(
        "a.ts".into(),
        b"import './main'; fetch('https://example.org');".to_vec(),
    );
    let r = Session::default()
        .analyze(&input, "main.ts", None, &EnvironmentContract::default())
        .unwrap();
    assert_eq!(r.files, ["a.ts", "main.ts"]);
    assert_eq!(differences(&r), 1);
    for code in [
        "code_analysis_dependency_missing",
        "code_analysis_dependency_escape",
        "code_analysis_external_dependency",
    ] {
        assert!(has(&r, code), "{r:?}");
    }
    input.insert("a.js".into(), b"export {};".to_vec());
    let r = Session::default()
        .analyze(&input, "main.ts", None, &EnvironmentContract::default())
        .unwrap();
    assert!(has(&r, "code_analysis_dependency_ambiguous"));
}

#[test]
fn failed_sources_are_cached_without_poisoning_independent_entries() {
    let mut input = sources("import './broken';");
    input.insert("broken.ts".into(), b"const value = ;".to_vec());
    input.insert("other.ts".into(), b"export {};".to_vec());
    let mut session = Session::default();
    for _ in 0..2 {
        let r = session
            .analyze(&input, "main.ts", None, &EnvironmentContract::default())
            .unwrap();
        assert!(!r.allowed);
        assert!(has(&r, "code_analysis_parse_failure"));
    }
    assert_eq!(session.parsed_files(), 2);
    assert!(
        session
            .analyze(&input, "other.ts", None, &EnvironmentContract::default())
            .unwrap()
            .allowed
    );
    let native = report("#!/bin/sh\necho hello", None);
    assert!(!native.allowed);
}

#[test]
fn cumulative_budget_and_cancellation_are_terminal() {
    let mut session = Session::new(
        Limits {
            command_files: 1,
            ..Limits::default()
        },
        Arc::new(AtomicBool::new(false)),
    );
    let mut input = sources("export {};");
    input.insert("other.ts".into(), b"export {};".to_vec());
    assert!(
        session
            .analyze(&input, "main.ts", None, &EnvironmentContract::default())
            .unwrap()
            .allowed
    );
    assert_eq!(
        session.analyze(&input, "other.ts", None, &EnvironmentContract::default()),
        Err(TerminalFailure::Budget)
    );
    assert_eq!(
        session.analyze(&input, "main.ts", None, &EnvironmentContract::default()),
        Err(TerminalFailure::Budget)
    );
    let cancel = Arc::new(AtomicBool::new(true));
    let mut session = Session::new(Limits::default(), cancel);
    assert_eq!(
        session.analyze(&input, "main.ts", None, &EnvironmentContract::default()),
        Err(TerminalFailure::Cancelled)
    );
    assert_eq!(session.parsed_files(), 0);
}

#[test]
fn report_does_not_export_source_literals_or_parser_errors() {
    for code in [
        "fetch('https://user:secret@example.org/private?token=secret'); Bun.spawn(['private-program', 'secret']);",
        "const secret = ;",
    ] {
        let r = report(code, None);
        let json = serde_json::to_string(&r).unwrap();
        for sensitive in [
            "secret",
            "private-program",
            "example.org",
            "user:",
            "const ",
        ] {
            assert!(!json.contains(sensitive), "{json}");
        }
    }
}

#[test]
fn local_limits_do_not_become_command_failures() {
    let mut session = Session::new(
        Limits {
            file_bytes: 10,
            ..Limits::default()
        },
        Arc::new(AtomicBool::new(false)),
    );
    let mut input = sources("const too_long = 123456789;");
    input.insert("small.ts".into(), b"export {};".to_vec());
    let r = session
        .analyze(&input, "main.ts", None, &EnvironmentContract::default())
        .unwrap();
    assert!(!r.allowed);
    assert!(has(&r, "code_analysis_file_limit"));
    assert!(
        session
            .analyze(&input, "small.ts", None, &EnvironmentContract::default())
            .unwrap()
            .allowed
    );
}

#[test]
fn hoisted_var_and_named_function_expressions_do_not_become_globals() {
    for source in [
        "function f() { if (true) { var fetch = other; } fetch('https://fake.invalid'); }",
        "const local = function fetch() { fetch('https://fake.invalid'); };",
        "const local = (fetch: any) => fetch('https://fake.invalid');",
        "try { throw 1; } catch (fetch) { fetch('https://fake.invalid'); }",
        "for (const fetch of callbacks) { fetch('https://fake.invalid'); }",
        "globalThis.fetch = replacement; fetch('https://fake.invalid');",
    ] {
        let r = report(source, None);
        assert_eq!(differences(&r), 0, "{source}: {r:?}");
        assert!(has(&r, "code_analysis_call_unknown"));
    }
}

#[test]
fn commonjs_relative_dependencies_and_builtin_aliases_are_analyzed() {
    let mut input =
        sources("const fs = require('node:fs'); fs.readFileSync('config'); require('./helper');");
    input.insert(
        "helper.js".into(),
        b"fetch('https://example.org');".to_vec(),
    );
    let r = Session::default()
        .analyze(&input, "main.ts", None, &EnvironmentContract::default())
        .unwrap();
    assert_eq!(differences(&r), 2, "{r:?}");
    assert_eq!(r.files, ["helper.js", "main.ts"]);
}

#[test]
fn destructured_environment_and_dynamic_names_remain_gaps() {
    let r = report(
        "const { TOKEN } = process.env; const value = Bun.env[name];",
        None,
    );
    assert!(r.allowed);
    assert!(has(&r, "code_analysis_environment_object"));
    assert!(has(&r, "code_analysis_dynamic_target"));
}

#[test]
fn url_identity_uses_url_semantics_without_retaining_credentials() {
    let r = report(
        "fetch('https://secret:password@127.1/private');",
        Some(&declaration(
            "network = [{ scope = 'host', host = '127.0.0.1' }]",
        )),
    );
    assert!(r.diagnostics.is_empty(), "{r:?}");
    let r = report("fetch('https://example.org:invalid/');", None);
    assert!(r.allowed);
    assert!(has(&r, "code_analysis_dynamic_target"));
}

#[test]
fn parser_and_node_budgets_reject_hostile_inputs() {
    let mut session = Session::new(
        Limits {
            nodes: 20,
            ..Limits::default()
        },
        Arc::new(AtomicBool::new(false)),
    );
    let source = "const value = [".to_owned() + &"1,".repeat(10000) + "];";
    assert_eq!(
        session.analyze(
            &sources(&source),
            "main.ts",
            None,
            &EnvironmentContract::default()
        ),
        Err(TerminalFailure::Budget)
    );
    let mut session = Session::new(
        Limits {
            parser_steps: 0,
            ..Limits::default()
        },
        Arc::new(AtomicBool::new(false)),
    );
    assert_eq!(
        session.analyze(
            &sources(&source),
            "main.ts",
            None,
            &EnvironmentContract::default()
        ),
        Err(TerminalFailure::Budget)
    );
}

#[test]
fn missing_source_declaration_does_not_require_guessing_sensitivity() {
    let env = EnvironmentContract {
        fixed: BTreeSet::new(),
        mappings: BTreeMap::from([("TARGET".into(), ("SOURCE".into(), None))]),
    };
    let r = Session::default()
        .analyze(
            &sources("const value = Bun.env.TARGET;"),
            "main.ts",
            None,
            &env,
        )
        .unwrap();
    assert_eq!(differences(&r), 1);
    assert!(has(&r, "code_analysis_identity_unknown"));
    let r = Session::default()
        .analyze(
            &sources("const value = Bun.env.TARGET;"),
            "main.ts",
            Some(&declaration(
                "environment = [{name='SOURCE', sensitivity='secret'}]",
            )),
            &env,
        )
        .unwrap();
    assert!(r.allowed);
    assert!(has(&r, "code_analysis_identity_unknown"));
}

#[test]
fn fork_modules_and_shell_options_are_not_executable_identities() {
    for code in [
        "import { fork } from 'node:child_process'; fork('./worker.js');",
        "import { spawn } from 'node:child_process'; spawn('echo private', { shell: true });",
        "import { exec } from 'node:child_process'; exec('echo private');",
    ] {
        let r = report(code, None);
        assert!(r.allowed, "{r:?}");
        assert_eq!(differences(&r), 0);
        assert!(has(&r, "code_analysis_subprocess_effects"));
    }
}

#[test]
fn file_comparison_reuses_declaration_normalization_and_retains_object_gaps() {
    let r = report(
        "import { readFileSync } from 'node:fs'; readFileSync('./dir//./file');",
        Some(&declaration(
            "filesystem = [{base='preset', path='dir/file', access=['read']}]",
        )),
    );
    assert!(r.diagnostics.is_empty(), "{r:?}");
    let r = report("Bun.write('dest', Bun.file('source'));", None);
    assert_eq!(differences(&r), 1);
    assert!(has(&r, "code_analysis_file_object"));
}

#[test]
fn binding_grammar_is_independent_of_comments_and_loop_spacing() {
    for source in [
        "for(var fetch of callbacks) {} fetch('https://example.org');",
        "{ function fetch() {} } fetch('https://example.org');",
        "for (fetch of callbacks) {} fetch('https://example.org');",
        "import/*comment*/type {readFileSync} from 'node:fs'; readFileSync('file');",
        "import {type readFileSync} from 'node:fs'; readFileSync('file');",
    ] {
        let r = report(source, None);
        assert_eq!(differences(&r), 0, "{source}: {r:?}");
        assert!(has(&r, "code_analysis_call_unknown"));
    }
    for source in [
        "const/**/request=fetch; request('https://example.org');",
        "for (let fetch of callbacks) { fetch('https://inside.invalid'); } fetch('https://outside.example');",
        "export {}; { function fetch() {} fetch('https://inside.invalid'); } fetch('https://outside.example');",
    ] {
        let r = report(source, None);
        assert_eq!(differences(&r), 1, "{source}: {r:?}");
    }
}

#[test]
fn advisory_diagnostics_coexist_without_granting_or_blocking_admission() {
    let extra = declaration("commands=['unused']");
    for code in ["export {};", "unknownCall();"] {
        let r = report(code, Some(&extra));
        assert!(r.allowed);
        assert!(has(&r, "code_analysis_declaration_uncorroborated"));
        assert_eq!(
            has(&r, "code_analysis_call_unknown"),
            code.contains("unknownCall")
        );
    }
    let gaps = report("unknownCall();", None);
    assert!(gaps.allowed);
    assert!(has(&gaps, "code_analysis_call_unknown"));
    let mut input = sources("import './broken'; fetch('https://missing.example'); unknownCall();");
    input.insert("broken.ts".into(), b"const broken = ;".to_vec());
    let r = Session::default()
        .analyze(
            &input,
            "main.ts",
            Some(&extra),
            &EnvironmentContract::default(),
        )
        .unwrap();
    assert!(!r.allowed);
    for kind in [
        DiagnosticKind::Difference,
        DiagnosticKind::Failure,
        DiagnosticKind::CoverageGap,
        DiagnosticKind::Uncorroborated,
    ] {
        assert!(
            r.diagnostics.iter().any(|d| d.kind == kind),
            "{kind:?}: {r:?}"
        );
    }
}

#[test]
fn fixed_environment_exemption_is_exact_and_overrides_aliases() {
    let env = EnvironmentContract {
        fixed: BTreeSet::from(["SHINE_APP_ID".into()]),
        mappings: BTreeMap::from([(
            "SHINE_APP_ID".into(),
            ("UNDECLARED".into(), Some(EnvironmentSensitivityV1::Secret)),
        )]),
    };
    let r = Session::default()
        .analyze(
            &sources("process.env.SHINE_APP_ID; process.env.SHINE_FORGED; process.env[dynamic];"),
            "main.ts",
            None,
            &env,
        )
        .unwrap();
    assert!(r.allowed);
    assert_eq!(differences(&r), 0);
    assert!(has(&r, "code_analysis_identity_unknown"));
    assert!(has(&r, "code_analysis_dynamic_target"));
}

#[test]
fn review_missing_punctuation_and_escaped_static_modules_are_not_silent() {
    for code in [
        "const x = { a: 1",
        "fetch('https://example.org'",
        "function f() { return 1;",
    ] {
        let r = report(
            code,
            Some(&declaration("network=[{scope='host',host='example.org'}]")),
        );
        assert!(has(&r, "code_analysis_parse_failure"), "{code}: {r:?}");
        assert!(!r.allowed);
    }
    for code in [r"import './\u0062ad';", r"export * from './\u0062ad';"] {
        let r = report(code, None);
        assert!(
            has(&r, "code_analysis_dependency_unsupported"),
            "{code}: {r:?}"
        );
    }
    let mut input = sources(
        "export type { X } from './broken'; export { type X } from './broken'; import { type X } from './broken';",
    );
    input.insert("broken.ts".into(), b"const broken = ;".to_vec());
    let r = Session::default()
        .analyze(&input, "main.ts", None, &EnvironmentContract::default())
        .unwrap();
    assert!(r.allowed, "{r:?}");
    assert_eq!(r.files, ["main.ts"]);
}
