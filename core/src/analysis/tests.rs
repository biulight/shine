use super::*;

mod semantic_matrix;

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
fn transparent_wrappers_preserve_network_identity() {
    let declared = declaration("network=[{scope='host',host='example.org'}]");
    for code in [
        "(/* callee */ fetch)('https://example.org');",
        "(fetch as typeof fetch)('https://example.org');",
        "(fetch satisfies typeof fetch)('https://example.org');",
        "(<typeof fetch>fetch)('https://example.org');",
        "fetch!('https://example.org');",
        "const request=(fetch as typeof fetch); request('https://example.org');",
        "(globalThis as typeof globalThis).fetch('https://example.org');",
        "((require)('node:https')!).get('https://example.org');",
        "import https from 'node:https'; (https.get)('https://example.org');",
    ] {
        let r = report(code, None);
        assert_eq!(differences(&r), 1, "{code}: {r:?}");
        assert!(
            report(code, Some(&declared)).diagnostics.is_empty(),
            "{code}"
        );
    }
}

#[test]
fn transparent_wrappers_preserve_mutations_and_captured_functions() {
    for mutation in [
        "(alias).readFileSync=other;",
        "(alias as typeof fs).readFileSync=other;",
        "(<typeof fs>alias).readFileSync=other;",
        "alias!.readFileSync=other;",
        "(Object.assign)((alias), {readFileSync:other});",
        "const assign=(Object.assign); assign((alias), {readFileSync:other});",
    ] {
        let code = format!(
            "import fs from 'node:fs'; const alias=(fs); const before=(alias.readFileSync);\n\
             {mutation}\n\
             (fs.readFileSync)('in');\n\
             (fs.writeFileSync)('out','data');\n\
             (before)('in');"
        );
        let r = report(&code, None);
        assert_eq!(differences(&r), 2, "{code}: {r:?}");
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.line == 3 && d.code == "code_analysis_call_unknown"),
            "{r:?}"
        );
        assert!(
            r.diagnostics
                .iter()
                .filter(|d| d.blocking())
                .all(|d| matches!(d.line, 4 | 5)),
            "{r:?}"
        );
    }
    for code in [
        "function f(fetch){(fetch as typeof fetch)('https://example.org')}",
        "const fetch=other; (fetch)('https://example.org');",
        "const request=(other as typeof fetch); request('https://example.org');",
        "const request=(<typeof fetch>other); request('https://example.org');",
        "(fetch, other)('https://example.org');",
        "(globalThis).fetch=other; (fetch)('https://example.org');",
        "const request=(fetch); request=other; (request)('https://example.org');",
        "(Object.assign)((globalThis), {[key]:other}); (fetch)('https://example.org');",
        "import fs from 'node:fs'; (fs).readFileSync=other; fs.readFileSync('in');",
    ] {
        let r = report(code, None);
        assert!(
            r.allowed && has(&r, "code_analysis_call_unknown"),
            "{code}: {r:?}"
        );
    }
}

#[test]
fn transparent_wrappers_do_not_hide_member_replacements() {
    for code in [
        "import fs from 'node:fs'; (fs).readFileSync=other; fs.readFileSync('in');",
        "import fs from 'node:fs'; (Object.assign)(fs, {readFileSync:other}); fs.readFileSync('in');",
    ] {
        let r = report(code, None);
        assert!(
            r.allowed && has(&r, "code_analysis_call_unknown"),
            "{code}: {r:?}"
        );
    }
}

#[test]
fn transparent_wrappers_preserve_bun_file_receivers() {
    for code in [
        "(Bun.file('in')).text();",
        "(Bun.file('in').text)();",
        "const f=(Bun.file('in') as ReturnType<typeof Bun.file>); (f.text)();",
        "const f=Bun.file('in'); (f as typeof f).text();",
    ] {
        let r = report(code, None);
        assert_eq!(differences(&r), 1, "{code}: {r:?}");
    }
    let r = report(
        "const f=(Bun.file('in')); (f).text=other; (f.text)();",
        None,
    );
    assert!(r.allowed && has(&r, "code_analysis_call_unknown"), "{r:?}");
    let r = report("const read=(Bun.file('in').text); read();", None);
    assert!(r.allowed && has(&r, "code_analysis_call_unknown"), "{r:?}");
}

#[test]
fn transparent_wrappers_preserve_environment_read_write_roles() {
    let env = EnvironmentContract {
        fixed: BTreeSet::new(),
        mappings: BTreeMap::from([(
            "NAME".into(),
            ("NAME".into(), Some(EnvironmentSensitivityV1::Plain)),
        )]),
    };
    let report = |code: &str, declaration: Option<&PermissionDeclarationV1>| {
        Session::default()
            .analyze(&sources(code), "main.ts", declaration, &env)
            .unwrap()
    };
    for code in [
        "((process as typeof process).env.NAME)=value;",
        "delete ((Bun as typeof Bun).env.NAME);",
        "for (((process).env.NAME) of values) {}",
    ] {
        let r = report(code, None);
        assert!(r.allowed, "{code}: {r:?}");
    }
    let declared = declaration("environment=[{name='NAME',sensitivity='plain'}]");
    for code in [
        "(process.env).NAME;",
        "(Bun.env as Record<string,string>).NAME;",
        "((process).env.NAME)+=value;",
    ] {
        let r = report(code, None);
        assert_eq!(differences(&r), 1, "{code}: {r:?}");
        let matching = report(code, Some(&declared));
        assert!(matching.diagnostics.is_empty(), "{code}: {matching:?}");
    }
}

#[test]
fn transparent_wrappers_have_matching_javascript_and_typescript_behavior() {
    for (code, expected) in [
        ("/* 中文 */ (fetch)('https://example.org/private');", 1),
        (
            "import fs from 'node:fs'; (fs).readFileSync=other; fs.readFileSync('in');",
            0,
        ),
        (
            "const fs=(require)('node:fs'); require=other; (fs.readFileSync)('in');",
            1,
        ),
        ("(Bun.file('in')).text();", 1),
    ] {
        for file in ["main.js", "main.ts"] {
            let r = Session::default()
                .analyze(
                    &BTreeMap::from([(file.into(), code.as_bytes())]),
                    file,
                    None,
                    &EnvironmentContract::default(),
                )
                .unwrap();
            assert_eq!(differences(&r), expected, "{file}: {code}: {r:?}");
            assert!(!has(&r, "code_analysis_parse_failure"), "{r:?}");
            if code.starts_with("/*") {
                assert_eq!((r.diagnostics[0].line, r.diagnostics[0].column), (1, 14));
                assert!(!format!("{r:?}").contains("example.org"));
            }
        }
    }
    let deep = format!(
        "{}fetch{}('https://example.org');",
        "(".repeat(128),
        ")".repeat(128)
    );
    let r = report(&deep, None);
    assert!(r.allowed && has(&r, "code_analysis_call_unknown"), "{r:?}");
}

#[test]
fn wrapped_literal_arguments_preserve_effects_and_permission_matching() {
    let permissions = declaration(
        "network=[{scope='host',host='example.org'}]\n\
         filesystem=[{base='preset',path='in',access=['read']}, {base='preset',path='out',access=['write']}]\n\
         commands=['echo']",
    );
    for (code, expected) in [
        ("fetch((/* URL */ 'https://example.org'));", 1),
        ("fetch('https://example.org' as const);", 1),
        ("fetch((<string>'https://example.org'));", 1),
        ("fetch(('https://example.org' satisfies string)!);", 1),
        ("import fs from 'node:fs'; fs.readFileSync(('in'));", 1),
        ("require(('node:fs')).copyFileSync(('in'), ('out'));", 2),
        ("Bun.file(('in' as const)).text();", 1),
        ("const f=Bun.file(('in')); f.text();", 1),
        ("Bun.spawn((['echo']));", 1),
        (
            "import {spawn} from 'node:child_process'; spawn(('echo'), ([]));",
            1,
        ),
    ] {
        let r = report(code, None);
        assert_eq!(differences(&r), expected, "{code}: {r:?}");
        let matching = report(code, Some(&permissions));
        assert!(matching.allowed, "{code}: {matching:?}");
        assert!(
            !has(&matching, "code_analysis_dynamic_target"),
            "{matching:?}"
        );
    }
}

#[test]
fn wrapped_assign_sources_preserve_known_keys_and_captures() {
    for (source, expected_read) in [
        ("({other: replacement})", true),
        ("({other: replacement} as object)", true),
        ("(null)", true),
        ("('readFileSync')", true),
        ("({[('other')]: replacement})", true),
        ("({[('readFileSync')]: replacement})", false),
        ("({[key]: replacement})", false),
        ("(patch as object)", false),
        ("({...patch})", false),
    ] {
        let code = format!(
            "import fs from 'node:fs'; const alias=fs; const before=alias.readFileSync;\n\
             Object.assign(alias, {source});\n\
             fs.readFileSync('in');\n\
             before('in');"
        );
        let r = report(&code, None);
        assert_eq!(
            differences(&r),
            1 + usize::from(expected_read),
            "{code}: {r:?}"
        );
        assert_eq!(
            r.diagnostics.iter().any(|d| d.line == 3 && d.blocking()),
            expected_read,
            "{r:?}"
        );
        assert!(
            r.diagnostics.iter().any(|d| d.line == 4 && d.blocking()),
            "{r:?}"
        );
    }
}

#[test]
fn wrapped_https_callbacks_and_member_keys_remain_precise() {
    for code in [
        "import https from 'node:https'; https.get('https://example.org', (()=>{}));",
        "import https from 'node:https'; https.get(('https://example.org'), ((function(){}) as Function));",
        "globalThis[('fetch')]('https://example.org');",
        "import fs from 'node:fs'; fs[('readFileSync' as const)]('in');",
    ] {
        let r = report(code, None);
        assert_eq!(differences(&r), 1, "{code}: {r:?}");
    }
    for code in [
        "fetch((url as string));",
        "fetch((condition ? 'https://example.org' : url));",
        "fetch((url, 'https://example.org'));",
        "import https from 'node:https'; https.get('https://example.org', (options));",
        "import https from 'node:https'; https.get('https://example.org', (callback));",
        "import https from 'node:https'; https.get('https://example.org', ({hostname:'other.invalid'} as object));",
        "import {spawn} from 'node:child_process'; spawn(('echo'), ({shell:true}));",
        r"Bun.file(('i\u006e')).text();",
        "import fs from 'node:fs'; fs[(key)]('in');",
    ] {
        let r = report(code, None);
        assert!(r.allowed, "{code}: {r:?}");
        assert!(
            has(&r, "code_analysis_dynamic_target") || has(&r, "code_analysis_call_unknown"),
            "{r:?}"
        );
    }
}

#[test]
fn wrapped_literals_preserve_dependencies_locations_and_depth_limits() {
    let code = "const fs=require(('node:fs'));\nfs.copyFileSync(\n  ('in'),\n  ('out')\n);\nrequire(('./dep.js'));";
    for file in ["main.js", "main.ts"] {
        let input = BTreeMap::from([
            (file.into(), code.as_bytes()),
            (
                "dep.js".into(),
                b"fetch(('https://example.org/private'));".as_slice(),
            ),
        ]);
        let r = Session::default()
            .analyze(&input, file, None, &EnvironmentContract::default())
            .unwrap();
        assert_eq!(differences(&r), 3, "{file}: {r:?}");
        for (source, line, column) in [(file, 3, 3), (file, 4, 3), ("dep.js", 1, 1)] {
            assert!(
                r.diagnostics.iter().any(|d| d.blocking()
                    && d.file == source
                    && d.line == line
                    && d.column == column),
                "{r:?}"
            );
        }
        assert!(!format!("{r:?}").contains("example.org"));
    }
    let deep = format!(
        "fetch({}'https://example.org'{});",
        "(".repeat(128),
        ")".repeat(128)
    );
    let r = report(&deep, None);
    assert!(
        r.allowed && has(&r, "code_analysis_dynamic_target"),
        "{r:?}"
    );
}

#[test]
fn type_queries_do_not_read_environment_or_load_runtime_dependencies() {
    let env = EnvironmentContract {
        fixed: BTreeSet::new(),
        mappings: BTreeMap::from([
            (
                "NAME".into(),
                ("NAME".into(), Some(EnvironmentSensitivityV1::Plain)),
            ),
            (
                "env".into(),
                ("SOURCE".into(), Some(EnvironmentSensitivityV1::Plain)),
            ),
        ]),
    };
    for code in [
        "type Value = typeof process.env.env;",
        "type Value = typeof process.env['NAME'];",
        "type Value = typeof process.env.NAME;",
        "let value: typeof Bun.env.NAME;",
        "interface Options { value: typeof process.env.NAME }",
        "class Options { value!: typeof Bun.env.NAME }",
        "type Generic<T extends typeof process.env.NAME> = T;",
        "type Module = typeof import('./missing.js');",
        "type Module = typeof import('unknown-package');",
    ] {
        for file in ["main.ts", "main.tsx"] {
            let input = BTreeMap::from([(file.into(), code.as_bytes())]);
            let r = Session::default()
                .analyze(&input, file, None, &env)
                .unwrap();
            assert!(
                r.allowed && r.diagnostics.is_empty(),
                "{file}: {code}: {r:?}"
            );
            assert_eq!(r.files, vec![file]);
        }
    }
    let runtime = Session::default()
        .analyze(&sources("typeof process.env.NAME;"), "main.ts", None, &env)
        .unwrap();
    assert_eq!(differences(&runtime), 1, "{runtime:?}");
}

#[test]
fn erased_type_queries_preserve_adjacent_runtime_effects_and_shadowing() {
    for code in [
        "const value = fetch('https://example.org') as typeof Bun.env.NAME;",
        "const value = fetch('https://example.org') satisfies typeof Bun.env.NAME;",
        "class C { value: typeof Bun.env.NAME = fetch('https://example.org') }",
        "function f(value: typeof Bun.env.NAME = fetch('https://example.org')) {}",
        "const request: typeof fetch = fetch; request('https://example.org');",
        "typeof fetch('https://example.org');",
    ] {
        let r = report(code, None);
        assert_eq!(differences(&r), 1, "{code}: {r:?}");
        assert_eq!(r.diagnostics.len(), 1, "{code}: {r:?}");
    }
    let r = report(
        "function f(fetch: typeof globalThis.fetch){fetch('https://example.org')}",
        None,
    );
    assert!(r.allowed && has(&r, "code_analysis_call_unknown"), "{r:?}");
    let r = report("import('./missing.js');", None);
    assert!(has(&r, "code_analysis_dynamic_code"), "{r:?}");
}

#[test]
fn erased_type_queries_still_validate_syntax_and_charge_node_budget() {
    for code in [
        "type Value = typeof process.env.;",
        r"type Value = typeof \uD800;",
    ] {
        let r = report(code, None);
        assert!(
            !r.allowed && has(&r, "code_analysis_parse_failure"),
            "{code}: {r:?}"
        );
    }
    let source = format!("type Value = typeof root{};", ".member".repeat(100));
    let mut session = Session::new(
        Limits {
            nodes: 20,
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
fn unsupported_import_spellings_still_shadow_builtin_globals() {
    for code in [
        r"import fetch from './lo\u0063al.js'; fetch('https://example.org');",
        r"import {request as fetch} from './lo\u0063al.js'; fetch('https://example.org');",
        r"import {fetch} from './lo\u0063al.js'; fetch('https://example.org');",
        r"import * as Bun from './lo\u0063al.js'; Bun.write('out','data');",
        r"import globalThis from './lo\u0063al.js'; globalThis.fetch('https://example.org');",
        r"import require from './lo\u0063al.js'; require('node:fs').readFileSync('in');",
        r"import f\u0065tch from './lo\u0063al.js'; fetch('https://example.org');",
        "import fetch from './lo\\\ncal.js'; fetch('https://example.org');",
    ] {
        for file in ["main.js", "main.ts"] {
            let input = BTreeMap::from([
                (file.into(), code.as_bytes()),
                (
                    "local.js".into(),
                    b"fetch('https://dependency.invalid');".as_slice(),
                ),
            ]);
            let r = Session::default()
                .analyze(&input, file, None, &EnvironmentContract::default())
                .unwrap();
            assert!(
                r.allowed && has(&r, "code_analysis_call_unknown"),
                "{file}: {code}: {r:?}"
            );
            assert!(has(&r, "code_analysis_dependency_unsupported"), "{r:?}");
            assert_eq!(r.files, vec![file]);
        }
    }
}

#[test]
fn unsupported_imports_do_not_hide_independent_builtin_effects() {
    for code in [
        r"import './lo\u0063al.js'; fetch('https://example.org');",
        r"import fetch from './lo\u0063al.js'; globalThis.fetch('https://example.org');",
        r"import Object from './lo\u0063al.js'; Object.assign(globalThis,{fetch:other}); fetch('https://example.org');",
        r"import * as Object from './lo\u0063al.js'; const assign=Object.assign; assign(globalThis,{fetch:other}); fetch('https://example.org');",
        r"import globalThis from './lo\u0063al.js'; globalThis.fetch=other; fetch('https://example.org');",
        r"import Object from './lo\u0063al.js'; import fs from 'node:fs'; Object.assign(fs,{readFileSync:other}); fs.readFileSync('in');",
    ] {
        let r = report(code, None);
        assert_eq!(differences(&r), 1, "{code}: {r:?}");
        assert!(has(&r, "code_analysis_dependency_unsupported"), "{r:?}");
    }
}

#[test]
fn unsupported_imports_keep_type_only_and_runtime_names_separate() {
    let r = report(
        r"import {type Other, request as fetch} from './lo\u0063al.js'; fetch('https://example.org');",
        None,
    );
    assert!(r.allowed && has(&r, "code_analysis_call_unknown"), "{r:?}");
    assert!(has(&r, "code_analysis_dependency_unsupported"), "{r:?}");
    for code in [
        r"import type {fetch} from './lo\u0063al.js'; fetch('https://example.org');",
        r"import {type fetch} from './lo\u0063al.js'; fetch('https://example.org');",
    ] {
        let r = report(code, None);
        assert_eq!(differences(&r), 1, "{code}: {r:?}");
        assert!(!has(&r, "code_analysis_dependency_unsupported"), "{r:?}");
    }
}

#[test]
fn quoted_builtin_import_names_match_identifier_spelling() {
    let permissions = declaration(
        "network=[{scope='host',host='example.org'}]\n\
         filesystem=[{base='preset',path='in',access=['read','remove']}, {base='preset',path='out',access=['write']}]\n\
         commands=['echo']",
    );
    for (code, expected) in [
        (
            r#"import {"get" as request} from 'node:https'; request('https://example.org');"#,
            1,
        ),
        (
            r#"import {"g\u0065t" as request} from 'node:https'; request('https://example.org');"#,
            1,
        ),
        (
            r#"import {'readFileSync' as read} from 'node:fs'; read('in');"#,
            1,
        ),
        (
            r#"import {'read\x46ile' as read} from 'node:fs/promises'; read('in');"#,
            1,
        ),
        (
            r#"import {'renameSync' as move} from 'node:fs'; move('in','out');"#,
            2,
        ),
        (
            r#"import {'spawn' as run} from 'node:child_process'; run('echo');"#,
            1,
        ),
    ] {
        for file in ["main.js", "main.ts"] {
            let input = BTreeMap::from([(file.into(), code.as_bytes())]);
            let r = Session::default()
                .analyze(&input, file, None, &EnvironmentContract::default())
                .unwrap();
            assert_eq!(differences(&r), expected, "{file}: {code}: {r:?}");
            let matching = Session::default()
                .analyze(
                    &input,
                    file,
                    Some(&permissions),
                    &EnvironmentContract::default(),
                )
                .unwrap();
            assert!(matching.allowed, "{file}: {code}: {matching:?}");
            assert!(
                !has(&matching, "code_analysis_call_unknown"),
                "{matching:?}"
            );
        }
    }
}

#[test]
fn quoted_import_names_preserve_lexical_and_member_identity_boundaries() {
    for code in [
        r#"import {'get' as request} from 'node:https'; function f(request){request('https://local.invalid')} request('https://example.org');"#,
        r#"import {'get' as request} from 'node:https'; const alias=request; {const alias=local; alias('https://local.invalid')} alias('https://example.org');"#,
        r#"import fs, {'readFileSync' as read} from 'node:fs'; Object.assign(fs,{readFileSync:other}); read('in');"#,
    ] {
        let r = report(code, None);
        assert_eq!(differences(&r), 1, "{code}: {r:?}");
    }
    for code in [
        r#"import {'get' as fetch} from 'unknown-package'; fetch('https://example.org');"#,
        r#"import {'get' as fetch} from './lo\u0063al.js'; fetch('https://example.org');"#,
        r#"import {'promises.readFile' as read} from 'node:fs'; read('in');"#,
        r#"import {'promises\u002ereadFile' as read} from 'node:fs'; read('in');"#,
        r#"import {'*' as read} from 'node:fs'; read('in');"#,
        r#"import {'get\0' as request} from 'node:https'; request('https://example.org');"#,
    ] {
        let r = report(code, None);
        assert!(
            r.allowed && has(&r, "code_analysis_call_unknown"),
            "{code}: {r:?}"
        );
    }
    let r = report(
        r#"import {type Other, 'get' as request} from 'node:https'; request('https://example.org');"#,
        None,
    );
    assert_eq!(differences(&r), 1, "{r:?}");
    let r = report(
        r#"import type {'get' as fetch} from 'node:https'; fetch('https://example.org');"#,
        None,
    );
    assert_eq!(differences(&r), 1, "{r:?}");
}

#[test]
fn escaped_network_names_and_urls_match_plain_spelling() {
    let declared = declaration("network=[{scope='host',host='example.org'}]");
    let wrong = declaration("network=[{scope='host',host='other.invalid'}]");
    for code in [
        r"f\u0065tch('https://example.org');",
        r"\u{66}etch('https://ex\u0061mple.org/path');",
        r"gl\u006fbalThis.fetch('https://ex\u{61}mple.org');",
        r"globalThis.f\u0065tch('https:\/\/example.org');",
        r"globalThis['f\u0065tch']('https://example.org/\uD83D\uDE00');",
        r"fetch('https://example.org/\u{1F600}');",
        r#"fetch('https://userinfo\@example.org/\'quoted\"');"#,
        r"const r\u0065quest=fetch; request('https://\x65xample.org');",
        r"import { g\u0065t as r\u0065quest } from 'node:https'; request('https://example.org');",
        r"import h\u0074tps from 'node:https'; https.get('https://\x65xample.org');",
        r"re\u0071uire('node:https').get('https://ex\u0061mple.org');",
        r"fetch('https://exam\ple.org');",
        "fetch('https://exam\\\nple.org');",
        "fetch('https://exam\\\r\nple.org');",
    ] {
        for file in ["main.js", "main.ts"] {
            let input = BTreeMap::from([(file.into(), code.as_bytes())]);
            let r = Session::default()
                .analyze(&input, file, None, &EnvironmentContract::default())
                .unwrap();
            assert_eq!(differences(&r), 1, "{code}: {r:?}");
            assert!(
                r.diagnostics.iter().all(|d| d.behavior == "network"),
                "{r:?}"
            );
            let matching = Session::default()
                .analyze(
                    &input,
                    file,
                    Some(&declared),
                    &EnvironmentContract::default(),
                )
                .unwrap();
            assert!(matching.diagnostics.is_empty(), "{code}: {matching:?}");
            assert_eq!(differences(&report(code, Some(&wrong))), 1);
        }
    }
}

#[test]
fn escaped_binding_names_and_writes_preserve_shadowing() {
    for code in [
        r"function f(f\u0065tch){fetch('https://example.org')}",
        r"function f(fetch){f\u0065tch('https://example.org')}",
        r"const f\u0065tch=other; fetch('https://example.org')",
        r"const {f\u0065tch}=other; fetch('https://example.org')",
        r"f\u0065tch=other; fetch('https://example.org')",
        r"globalThis.f\u0065tch=other; fetch('https://example.org')",
        r"Object.assign(globalThis,{f\u0065tch:other}); fetch('https://example.org')",
        r"Object.assign(globalThis,{'f\u0065tch':other}); fetch('https://example.org')",
        r"globalThis['f\u0065tch']=other; fetch('https://example.org')",
        r"globalThis['f\\u0065tch']('https://example.org')",
        r"const request=fetch; r\u0065quest=other; request('https://example.org')",
    ] {
        let r = report(code, None);
        assert!(
            r.allowed && has(&r, "code_analysis_call_unknown"),
            "{code}: {r:?}"
        );
    }
    let r = report(
        r"const r\u0065quest=fetch; globalThis.f\u0065tch=other; request('https://example.org')",
        None,
    );
    assert_eq!(differences(&r), 1, "{r:?}");
}

#[test]
fn escaped_urls_keep_ambiguous_values_unknown_and_locations_private() {
    for code in [
        r"f\uD800tch('https://example.org')",
        r"f\u{110000}tch('https://example.org')",
    ] {
        let r = report(code, None);
        assert!(
            !r.allowed && has(&r, "code_analysis_parse_failure"),
            "{r:?}"
        );
    }
    for url in [
        r"https://ex\141mple.org",
        r"https://example.org/\uD800",
        r"https://example.org/\0",
        r"https://exa\nmple.org",
    ] {
        let r = report(&format!("fetch('{url}');"), None);
        assert!(
            r.allowed && has(&r, "code_analysis_dynamic_target"),
            "{url}: {r:?}"
        );
    }
    let code =
        r"/* 汉 */ f\u0065tch('https://user:secret@ex\u0061mple.org/private?q=hidden#part');";
    let r = report(code, None);
    assert_eq!(differences(&r), 1);
    assert_eq!((r.diagnostics[0].line, r.diagnostics[0].column), (1, 11));
    let public = serde_json::to_string(&r).unwrap();
    for private in ["example.org", "secret", "/private", "hidden", "part"] {
        assert!(!public.contains(private));
    }
    let declaration = declaration("network=[{scope='host',host='trusted.invalid'}]");
    let r = report(
        r"fetch('https://trusted.invalid\\@other.invalid');",
        Some(&declaration),
    );
    assert!(r.diagnostics.is_empty(), "{r:?}");
}

#[test]
fn https_get_compares_the_literal_https_host() {
    let expected = declaration("network = [{scope='host',host='example.org'}]");
    let wrong = declaration("network = [{scope='host',host='other.invalid'}]");
    for (setup, call, callback) in [
        ("import https from 'node:https';", "https.get", ""),
        (
            "import * as https from 'https';",
            "https['get']",
            ", /* callback */ () => {}",
        ),
        (
            "import {get as request} from 'node:https';",
            "request",
            ", function () {}",
        ),
        (
            "const h=require('node:https'); const get=h.get; h.get=other;",
            "get",
            "",
        ),
    ] {
        let src = format!(
            "{setup}\n{call}(/* URL */ 'HTTPS://user:secret@EXAMPLE.org:8443/private?q=hidden#fragment'{callback});"
        );
        let missing = report(&src, None);
        assert_eq!(differences(&missing), 1, "{src}: {missing:?}");
        let diag = &missing.diagnostics[0];
        assert_eq!(
            (diag.line, diag.column, diag.behavior.as_str()),
            (2, 1, "network")
        );
        assert_eq!(differences(&report(&src, Some(&wrong))), 1);
        assert!(
            report(&src, Some(&expected)).diagnostics.is_empty(),
            "{src}"
        );
        let public = serde_json::to_string(&missing).unwrap();
        for private in [
            "user:secret",
            "example.org",
            "/private",
            "q=hidden",
            "#fragment",
        ] {
            assert!(!public.contains(private));
        }
    }
}

#[test]
fn https_get_keeps_uncertain_targets_and_callees_distinct() {
    for args in [
        "url",
        "{ hostname:'override.invalid' }",
        "'https://literal.invalid', {hostname:'override.invalid'}, () => {}",
        "'https://literal.invalid', callbackOrOptions",
        "'https://literal.invalid', ...extra",
        "...urls, 'https://literal.invalid'",
        "'http://literal.invalid'",
        "'not a URL'",
    ] {
        let r = report(
            &format!("import https from 'node:https'; https.get({args});"),
            None,
        );
        assert!(
            r.allowed
                && has(&r, "code_analysis_dynamic_target")
                && !has(&r, "code_analysis_call_unknown"),
            "{args}: {r:?}"
        );
    }
    for src in [
        "function f(https) { https.get('https://example.org'); }",
        "import h from 'node:https'; const a=h; a.get=other; h.get('https://example.org');",
        "import h from 'node:https'; Object.assign(h,{get:other}); h.get('https://example.org');",
        "import h from 'node:https'; h.request('https://example.org');",
        "function f(require) { require('node:https').get('https://example.org'); }",
    ] {
        let r = report(src, None);
        assert!(
            r.allowed
                && has(&r, "code_analysis_call_unknown")
                && !has(&r, "code_analysis_dynamic_target"),
            "{src}: {r:?}"
        );
    }
}

#[test]
fn qualified_fetch_uses_the_same_host_and_privacy_rules() {
    let declared = declaration("network=[{scope='host',host='example.org'}]");
    for callee in ["globalThis.fetch", "globalThis['fetch']", "request"] {
        let src = format!(
            "const root=globalThis; const request=root.fetch;\n{callee}('https://user:secret@example.org/private?q=hidden');"
        );
        let missing = report(&src, None);
        assert_eq!(differences(&missing), 1, "{src}: {missing:?}");
        let d = &missing.diagnostics[0];
        assert_eq!((d.line, d.column, d.behavior.as_str()), (2, 1, "network"));
        assert!(!serde_json::to_string(&missing).unwrap().contains("secret"));
        assert!(report(&src, Some(&declared)).diagnostics.is_empty());
    }
    let dynamic = report("globalThis.fetch(url)", None);
    assert!(dynamic.allowed && has(&dynamic, "code_analysis_dynamic_target"));
}

#[test]
fn qualified_fetch_respects_shadowing_and_shared_global_writes() {
    for code in [
        "new globalThis.fetch('https://example.org');",
        "function f(globalThis) { globalThis.fetch('https://example.org'); }",
        "globalThis=other; globalThis.fetch('https://example.org');",
        "fetch=other; globalThis.fetch('https://example.org');",
        "const g=globalThis; g.fetch=other; fetch('https://example.org');",
        "const g=globalThis; Object.assign(g,{fetch:other}); globalThis.fetch('https://example.org');",
        "const g=globalThis; const set=Object['assign']; set(g,{fetch:other}); fetch('https://example.org');",
        "globalThis.fetch=other; const saved=globalThis.fetch; saved('https://example.org');",
        "const g=globalThis; delete g[key]; globalThis.fetch('https://example.org');",
    ] {
        let r = report(code, None);
        assert!(
            r.allowed && has(&r, "code_analysis_call_unknown"),
            "{code}: {r:?}"
        );
    }
    for code in [
        "function f(fetch) { globalThis.fetch('https://example.org'); }",
        "const g=globalThis; const saved=g.fetch; g.fetch=other; saved('https://example.org');",
        "const g=globalThis; g.unrelated=other; g.fetch('https://example.org');",
        "Object.assign(globalThis,{Object:{assign(){}}}); Object.assign(globalThis,{fetch:other}); fetch('https://example.org');",
        "Object.assign(globalThis,{globalThis:{}}); Object.assign(globalThis,{fetch:other}); fetch('https://example.org');",
        "const g=globalThis; globalThis=other; g.fetch('https://example.org');",
    ] {
        assert_eq!(differences(&report(code, None)), 1, "{code}");
    }
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
fn comments_preserve_literal_arguments_and_dependency_identity() {
    for (code, permission, subprocess) in [
        (
            "fetch(/* target */ 'https://example.org');",
            "network=[{scope='host',host='example.org'}]",
            false,
        ),
        (
            "import { readFileSync as read } from 'node:fs'; read(/* path */ 'config');",
            "filesystem=[{access=['read'],base='preset',path='config'}]",
            false,
        ),
        (
            "import fs from 'node:fs'; fs.writeFileSync(/* path */ 'config', 'ok');",
            "filesystem=[{access=['write'],base='preset',path='config'}]",
            false,
        ),
        (
            "import fs from 'node:fs'; fs.unlinkSync(/* path */ 'config');",
            "filesystem=[{access=['remove'],base='preset',path='config'}]",
            false,
        ),
        (
            "await Bun.file(/* path */ 'config').text();",
            "filesystem=[{access=['read'],base='preset',path='config'}]",
            false,
        ),
        (
            "Bun.spawn(/* argv */ [/* program */ 'echo', 'ok']);",
            "commands=['echo']",
            true,
        ),
        (
            "import { spawn } from 'node:child_process'; spawn(/* program */ 'echo', /* argv */ []);",
            "commands=['echo']",
            true,
        ),
        (
            "const fs = require(/* module */ 'node:fs'); fs.readFileSync(/* path */ 'config');",
            "filesystem=[{access=['read'],base='preset',path='config'}]",
            false,
        ),
    ] {
        let missing = report(code, None);
        assert_eq!(differences(&missing), 1, "{code}: {missing:?}");
        let matching = report(code, Some(&declaration(permission)));
        assert!(matching.allowed, "{code}: {matching:?}");
        assert_eq!(
            has(&matching, "code_analysis_subprocess_effects"),
            subprocess
        );
        assert!(
            !has(&matching, "code_analysis_dynamic_target"),
            "{matching:?}"
        );
    }
    for code in [
        "fetch(/* target */ url);",
        "Bun.spawn([/* missing executable */ , 'echo']);",
        "Bun.spawn([/* dynamic executable */ ...argv, 'echo']);",
        "import { spawn } from 'node:child_process'; spawn('echo', /* options */ {shell:true});",
    ] {
        let r = report(code, None);
        assert!(r.allowed, "{code}: {r:?}");
        assert!(has(&r, "code_analysis_dynamic_target"), "{code}: {r:?}");
    }
    let input = BTreeMap::from([
        (
            "main.ts".into(),
            b"require(/* local */ './dep');".as_slice(),
        ),
        ("dep.ts".into(), b"fetch('https://example.org');".as_slice()),
    ]);
    let r = Session::default()
        .analyze(&input, "main.ts", None, &EnvironmentContract::default())
        .unwrap();
    assert_eq!(differences(&r), 1, "{r:?}");
    assert_eq!(r.files.len(), 2);
}

#[test]
fn binding_mutations_and_builtin_lookups_respect_lexical_scope() {
    for code in [
        "import {readFileSync} from 'node:fs'; function local(readFileSync) { readFileSync = other; } readFileSync('config');",
        "import fs from 'node:fs'; const read = fs.readFileSync; function local(read) { read = other; } read('config');",
        "function local(require) {} const fs = require('node:fs'); fs.readFileSync('config');",
        "function local(globalThis) { globalThis.fetch = other; } fetch('https://example.org');",
        "function local(fetch) { fetch = other; } fetch('https://example.org');",
    ] {
        let r = report(code, None);
        assert_eq!(differences(&r), 1, "{code}: {r:?}");
    }
    for code in [
        "function local(require) { const fs = require('node:fs'); fs.readFileSync('config'); }",
        "import fs from 'node:fs'; const read = fs.readFileSync; function local() { read = other; } read('config');",
        "function local(fetch) {} fetch = other; fetch('https://example.org');",
        "function local(globalThis) {} globalThis.fetch = other; fetch('https://example.org');",
    ] {
        let r = report(code, None);
        assert!(r.allowed, "{code}: {r:?}");
        assert!(has(&r, "code_analysis_call_unknown"), "{r:?}");
    }
}

#[test]
fn default_parameters_use_outer_bindings_before_function_body_scope() {
    for extension in ["js", "ts"] {
        for code in [
            "function run(x = fetch('https://example.org')) { var fetch = other; }",
            "function run(x = fetch('https://example.org')) { function fetch() {} }",
            "export {}; function run(x = fetch('https://example.org')) { function fetch() {} }",
            "const run = (x = fetch('https://example.org')) => { var fetch = other; };",
            "function* run(x = fetch('https://example.org')) { var fetch = other; }",
            "const object = { run(x = fetch('https://example.org')) { for (var fetch of []) {} } };",
            "function run(x = () => fetch('https://example.org')) { var fetch = other; }",
        ] {
            let file = format!("main.{extension}");
            let input = BTreeMap::from([(file.clone(), code.as_bytes())]);
            let r = Session::default()
                .analyze(&input, &file, None, &EnvironmentContract::default())
                .unwrap();
            assert_eq!(differences(&r), 1, "{code}: {r:?}");
        }
    }
    for code in [
        "function run(fetch, x = fetch('https://local.invalid')) {}",
        "function run(x = other) { var fetch = other; fetch('https://local.invalid'); }",
        "function run(x = other) { function fetch() {} fetch('https://local.invalid'); }",
        "const fetch = other; function run(x = fetch('https://local.invalid')) { var fetch; }",
    ] {
        let r = report(code, None);
        assert!(r.allowed, "{code}: {r:?}");
        assert!(has(&r, "code_analysis_call_unknown"), "{r:?}");
    }
}

#[test]
fn replaced_require_does_not_produce_builtin_evidence() {
    for code in [
        "require = () => ({readFileSync: () => 'ok'}); const fs = require('node:fs'); fs.readFileSync('config');",
        "globalThis.require = other; require('node:fs').readFileSync('config');",
        "({require} = input); const fs = require('node:fs'); fs.readFileSync('config');",
        "function local() { require = other; } const fs = require('node:fs'); fs.readFileSync('config');",
    ] {
        let r = report(code, None);
        assert!(r.allowed, "{code}: {r:?}");
        assert!(has(&r, "code_analysis_call_unknown"), "{r:?}");
    }
    for code in [
        "const fs = require('node:fs'); fs.readFileSync('config');",
        "const fs = require('node:fs'); require = other; fs.readFileSync('config');",
        "const load = require; require = other; load('./dep');",
    ] {
        let input = BTreeMap::from([
            ("main.ts".into(), code.as_bytes()),
            ("dep.ts".into(), b"fetch('https://example.org');".as_slice()),
        ]);
        let r = Session::default()
            .analyze(&input, "main.ts", None, &EnvironmentContract::default())
            .unwrap();
        assert_eq!(differences(&r), 1, "{code}: {r:?}");
    }
}

#[test]
fn generator_declarations_shadow_globals_without_leaking_local_bindings() {
    for extension in ["js", "ts"] {
        for source in [
            "export {}; function* fetch(url) { yield url; } fetch('https://fake.invalid');",
            "export {}; async function* fetch(url) { yield url; } fetch('https://fake.invalid');",
            "export {}; function* run() { var fetch = other; fetch('https://fake.invalid'); }",
        ] {
            let input = BTreeMap::from([(format!("main.{extension}"), source.as_bytes())]);
            let r = Session::default()
                .analyze(
                    &input,
                    &format!("main.{extension}"),
                    None,
                    &EnvironmentContract::default(),
                )
                .unwrap();
            assert!(r.allowed, "{source}: {r:?}");
            assert!(has(&r, "code_analysis_call_unknown"), "{r:?}");
            // A generator's local var must not shadow the real global outside it.
            let source = format!("{source}\nfetch('https://real.invalid');");
            if source.contains("function* run") {
                let r = report(&source, None);
                assert_eq!(differences(&r), 1, "{r:?}");
            }
        }
    }
    let r = report(
        "export {}; { function* fetch() {} } fetch('https://real.invalid');",
        None,
    );
    assert_eq!(differences(&r), 1, "{r:?}");
}

#[test]
fn review_comments_do_not_hide_namespace_bindings_or_assignment_targets() {
    for extension in ["js", "ts"] {
        for (code, expected) in [
            (
                "import * /* ns */ as fs from 'node:fs'; fs.readFileSync('config');",
                1,
            ),
            (
                "import * as /* ns */ fs from 'node:fs'; fs.readFileSync('config');",
                1,
            ),
            (
                "import { /* member */ readFileSync as read } from 'node:fs'; read('config');",
                1,
            ),
            (
                "import fs from 'node:fs'; (/* target */ fs.readFileSync) = other; fs.readFileSync('config');",
                0,
            ),
            (
                "(/* target */ fetch) = other; fetch('https://example.org');",
                0,
            ),
            (
                "import fs from 'node:fs'; (/* target */ fs.other) = other; fs.readFileSync('config');",
                1,
            ),
        ] {
            let file = format!("main.{extension}");
            let input = BTreeMap::from([(file.clone(), code.as_bytes())]);
            let r = Session::default()
                .analyze(&input, &file, None, &EnvironmentContract::default())
                .unwrap();
            assert_eq!(differences(&r), expected, "{code}: {r:?}");
            if expected == 0 {
                assert!(has(&r, "code_analysis_call_unknown"), "{r:?}");
            }
        }
    }
}

#[test]
fn review_loop_writes_resolve_targets_instead_of_declaring_global_names() {
    for operator in ["in", "of"] {
        assert!(
            report(
                &format!("for (process.env.KEY {operator} values) {{}}"),
                None
            )
            .diagnostics
            .is_empty()
        );
        assert!(has(
            &report(
                &format!("for (const value {operator} process.env.KEY) {{}}"),
                None
            ),
            "code_analysis_identity_unknown"
        ));
        for (code, expected) in [
            (
                format!(
                    "import {{readFileSync as read}} from 'node:fs'; function local(read) {{ for (read {operator} []) {{}} }} read('config');"
                ),
                1,
            ),
            (
                format!(
                    "import fs from 'node:fs'; for (fs.readFileSync {operator} []) {{}} fs.readFileSync('config');"
                ),
                0,
            ),
            (
                format!("for (globalThis.fetch {operator} []) {{}} fetch('https://example.org');"),
                0,
            ),
            (
                format!("for (fetch {operator} []) {{}} fetch('https://example.org');"),
                0,
            ),
            (
                format!(
                    "import fs from 'node:fs'; const read = fs.readFileSync; for (fs.readFileSync {operator} []) {{}} read('config');"
                ),
                1,
            ),
            (
                format!(
                    "function local(globalThis) {{ for (globalThis.fetch {operator} []) {{}} }} fetch('https://example.org');"
                ),
                1,
            ),
            (
                format!(
                    "for (let fetch {operator} []) {{ fetch('https://local.invalid'); }} fetch('https://example.org');"
                ),
                1,
            ),
            (
                format!(
                    "import fs from 'node:fs'; for (fs.other {operator} []) {{}} fs.readFileSync('config');"
                ),
                1,
            ),
        ] {
            let r = report(&code, None);
            assert_eq!(differences(&r), expected, "{code}: {r:?}");
            if expected == 0 {
                assert!(has(&r, "code_analysis_call_unknown"), "{r:?}");
            }
        }
    }
    for code in [
        "import fs from 'node:fs'; for ({read: fs.readFileSync} of values) {} fs.readFileSync('config');",
        "import fs from 'node:fs'; for ([fs.readFileSync] of values) {} fs.readFileSync('config');",
        "import fs from 'node:fs'; for await (fs.readFileSync of values) {} fs.readFileSync('config');",
    ] {
        let r = report(code, None);
        assert!(r.allowed, "{code}: {r:?}");
        assert!(has(&r, "code_analysis_call_unknown"), "{r:?}");
    }
}

#[test]
fn review_typescript_write_wrappers_do_not_hide_targets_or_visit_type_names() {
    for target in [
        "(/* target */ fs.readFileSync)",
        "(fs.readFileSync as unknown)",
        "(fs.readFileSync satisfies unknown)",
        "(<unknown>fs.readFileSync)",
        "(fs.readFileSync!)",
    ] {
        let code =
            format!("import fs from 'node:fs'; {target} = other; fs.readFileSync('config');");
        let r = report(&code, None);
        assert!(r.allowed, "{code}: {r:?}");
        assert!(has(&r, "code_analysis_call_unknown"), "{r:?}");
    }
    for target in ["(fs.other as typeof fetch)", "(<typeof fetch>fs.other)"] {
        let code =
            format!("import fs from 'node:fs'; {target} = other; fetch('https://example.org');");
        assert_eq!(differences(&report(&code, None)), 1, "{code}");
    }
}

#[test]
fn review_method_keys_and_static_blocks_keep_their_own_binding_scopes() {
    for extension in ["js", "ts"] {
        for (code, expected) in [
            (
                "const object = { [fetch('https://example.org')](fetch) {} };",
                1,
            ),
            ("class C { [fetch('https://example.org')](fetch) {} }", 1),
            (
                "class C { static [fetch('https://example.org')](fetch) {} }",
                1,
            ),
            (
                "class C { static { var fetch = other; } } fetch('https://example.org');",
                1,
            ),
            (
                "class C { static { var fetch = other; fetch('https://local.invalid'); } }",
                0,
            ),
            (
                "const object = { run(fetch, x = fetch('https://local.invalid')) {} };",
                0,
            ),
            (
                "const fetch = other; const object = { [fetch('https://local.invalid')](x) {} };",
                0,
            ),
        ] {
            let file = format!("main.{extension}");
            let input = BTreeMap::from([(file.clone(), code.as_bytes())]);
            let r = Session::default()
                .analyze(&input, &file, None, &EnvironmentContract::default())
                .unwrap();
            assert_eq!(differences(&r), expected, "{code}: {r:?}");
        }
    }
}

#[test]
fn review_captured_loaders_keep_dependency_effects_and_builtin_return_identity() {
    for (code, expected) in [
        ("for (require of [require('./dep')]) {}", 1),
        ("for (globalThis.require of [require('./dep')]) {}", 1),
        ("const dep = require('./dep'); require = other;", 1),
        ("require('./dep'); require = other;", 1),
        ("require = other; const dep = require('./dep');", 0),
        (
            "function local() { require = other; } const dep = require('./dep');",
            0,
        ),
        (
            "const load = require; require = other; const fs = load('node:fs'); fs.readFileSync('config');",
            1,
        ),
        ("const load = require; require = other; load('./dep');", 1),
        (
            "const load = other; const fs = load('node:fs'); fs.readFileSync('config');",
            0,
        ),
        (
            "const load = require; function local(load) { const fs = load('node:fs'); fs.readFileSync('config'); }",
            0,
        ),
    ] {
        let input = BTreeMap::from([
            ("main.ts".into(), code.as_bytes()),
            ("dep.ts".into(), b"fetch('https://example.org');".as_slice()),
        ]);
        let r = Session::default()
            .analyze(&input, "main.ts", None, &EnvironmentContract::default())
            .unwrap();
        assert_eq!(differences(&r), expected, "{code}: {r:?}");
        if expected == 0 {
            assert!(has(&r, "code_analysis_call_unknown"), "{r:?}");
        }
    }
}

#[test]
fn object_alias_mutations_invalidate_shared_builtin_identity() {
    for source in [
        "import fs from 'node:fs'; const alias = fs; alias.readFileSync = () => 'ok'; fs.readFileSync('config');",
        "import fs from 'node:fs'; const alias = fs; fs.readFileSync = () => 'ok'; alias.readFileSync('config');",
        "import fs from 'node:fs'; const first = fs; const second = first; second.readFileSync = () => 'ok'; fs.readFileSync('config');",
        "import fs from 'node:fs'; const alias = fs; alias.readFileSync = other; fs.writeFileSync = other; alias.readFileSync('config');",
        "const alias = require('node:fs'); alias.readFileSync = other; require('node:fs').readFileSync('config');",
        "const alias = Bun; alias.write = other; Bun.write('config', 'ok');",
        "import fs from 'node:fs'; const alias = fs; delete alias.readFileSync; fs.readFileSync('config');",
    ] {
        let r = report(source, None);
        assert!(r.allowed, "{source}: {r:?}");
        assert!(has(&r, "code_analysis_call_unknown"), "{source}: {r:?}");
        // Invalidating one object must not hide an unrelated real network call.
        let r = report(&format!("{source}\nfetch('https://real.invalid');"), None);
        assert_eq!(differences(&r), 1, "{source}: {r:?}");
    }
    for source in [
        "import fs from 'node:fs'; const alias = fs; alias.readFileSync('config');",
        "import fs from 'node:fs'; function local(alias) { alias.readFileSync = other; } fs.readFileSync('config');",
        "import fs from 'node:fs'; let alias = fs; alias = other; fs.readFileSync('config');",
    ] {
        assert_eq!(differences(&report(source, None)), 1, "{source}");
    }
}

#[test]
fn review_erased_namespaces_leave_global_calls_visible() {
    for body in [
        "export type Options = {};",
        "interface Options {} ; /* only a type */",
        "export namespace Inner { export interface Options {} }",
        "namespace Inner { type T = number }",
        "declare const option: string;",
        "function overload(): void;",
        "export declare interface Options {}",
        "'use strict'; export type Options = {};",
    ] {
        let source = format!("namespace fetch {{ {body} }} fetch('https://example.org')");
        assert_eq!(differences(&report(&source, None)), 1, "{source}");
    }
    for body in [
        "export const value = 1",
        "export namespace Inner { export const x=1 }",
    ] {
        let source = format!("namespace fetch {{ {body} }} fetch('https://example.org')");
        let result = report(&source, None);
        assert!(
            result.allowed && has(&result, "code_analysis_call_unknown"),
            "{source}"
        );
    }
}

#[test]
fn review_namespaces_that_bun_materializes_keep_their_bindings() {
    for declaration in [
        "namespace Bun.Inner { export type T = number; }",
        "namespace Bun { namespace Inner.Types {} }",
        "namespace Bun { ; 'use strict'; }",
        "namespace Bun { export declare const version: number; }",
    ] {
        let source = format!("{declaration} Bun.write('out','data');");
        let result = report(&source, None);
        assert!(
            result.allowed && has(&result, "code_analysis_call_unknown"),
            "{source}"
        );
    }
}

#[test]
fn review_escaped_class_names_shadow_builtin_calls() {
    for source in [
        r"class B\u0075n { static write() {} } B\u0075n.write('out','data');",
        r"class B\u0075n { static write() {} } Bun.write('out','data');",
        r"abstract class B\u0075n { static write() {} } Bun.write('out','data');",
    ] {
        let result = report(source, None);
        assert!(
            result.allowed && has(&result, "code_analysis_call_unknown"),
            "{result:?}"
        );
    }
}

#[test]
fn review_calls_use_the_original_builtin_before_a_later_write() {
    for code in [
        "fetch('https://example.org'); Object.assign(globalThis,{fetch:other});",
        "function run(){ globalThis.fetch('https://example.org'); globalThis.fetch=other; }",
        "import fs from 'node:fs'; fs.readFileSync('in'); Object.assign(fs,{readFileSync:other});",
        "Bun.file('in').text(); Object.assign(Bun,{file:other});",
        "const f=Bun.file('in'); f.text(); Object.assign(f,{text:other});",
    ] {
        assert_eq!(differences(&report(code, None)), 1, "{code}");
    }
    for code in [
        "Object.assign(globalThis,{fetch:other}); fetch('https://example.org');",
        "function run(){fetch('https://example.org')} Object.assign(globalThis,{fetch:other}); run();",
        "for (let i=0;i<2;i++){ if(i) fetch('https://example.org'); Object.assign(globalThis,{fetch:other}); }",
    ] {
        assert!(report(code, None).allowed, "{code}");
    }
}

#[test]
fn review_assign_aliases_observe_replacement_at_the_initializer() {
    for (code, expected) in [
        (
            "Object.assign(Object,{assign:()=>{}}); const set=Object.assign; set(globalThis,{fetch:other});",
            1,
        ),
        (
            "Object.assign(globalThis,{Object:{assign(){}}}); const set=Object.assign; const alias=set; alias(globalThis,{fetch:other});",
            1,
        ),
        (
            "const set=Object.assign; Object.assign(Object,{assign:()=>{}}); set(globalThis,{fetch:other});",
            0,
        ),
    ] {
        let result = report(&format!("{code} fetch('https://example.org');"), None);
        assert_eq!(differences(&result), expected, "{code}: {result:?}");
    }
}

#[test]
fn review_assign_scope_work_is_budgeted_and_shared_between_siblings() {
    let analyze = |count, nodes| {
        let source =
            "{".repeat(128) + &"Object.assign(globalThis, {x:0});".repeat(count) + &"}".repeat(128);
        Session::new(
            Limits {
                nodes,
                ..Limits::default()
            },
            Arc::new(AtomicBool::new(false)),
        )
        .analyze(
            &sources(&source),
            "main.ts",
            None,
            &EnvironmentContract::default(),
        )
    };
    assert_eq!(analyze(1, 470), Err(TerminalFailure::Budget));
    assert!(analyze(100, 3500).unwrap().allowed);
}

#[test]
fn followup_ambient_namespaces_only_shadow_values_they_declare() {
    for (body, expected) in [
        ("export type Options = {};", 1),
        ("export interface Options {}", 1),
        ("namespace Nested { interface Options {} }", 1),
        ("export const version: string;", 0),
        ("export function request(): void;", 0),
        ("namespace Nested { const version: string; }", 0),
    ] {
        let code = format!(
            "export {{}}; declare namespace fetch {{ {body} }} fetch('https://example.org')"
        );
        let r = report(&code, None);
        assert_eq!(differences(&r), expected, "{code}: {r:?}");
        assert_eq!(has(&r, "code_analysis_call_unknown"), expected == 0);
    }
}

#[test]
fn followup_result_property_reads_preserve_eager_receiver_order() {
    for result in [
        "(await fetch('https://example.org')).ok",
        "fetch('https://example.org')['ok']",
        "fetch('https://example.org')?.ok",
    ] {
        let code = format!("const ok = {result}; Object.assign(globalThis,{{fetch:other}});");
        assert_eq!(differences(&report(&code, None)), 1, "{code}");
    }
    let conditional =
        "const o=null; o?.[fetch('https://example.org')]; Object.assign(globalThis,{fetch:other});";
    assert!(report(conditional, None).allowed);
}

#[test]
fn review_eager_comparisons_and_subscripts_keep_network_identity() {
    for expression in [
        "(await fetch('https://example.org')).status === 200",
        "200 !== (await fetch('https://example.org')).status",
        "table[(await fetch('https://example.org')).status]",
        "(table?.values)[(await fetch('https://example.org')).status]",
        "fetch('https://example.org') && other",
    ] {
        let code = format!("const ok={expression}; Object.assign(globalThis,{{fetch:other}});");
        assert_eq!(differences(&report(&code, None)), 1, "{code}");
    }
    for expression in [
        "flag && fetch('https://example.org')",
        "flag ?? fetch('https://example.org')",
        "table?.values[fetch('https://example.org')]",
    ] {
        let code = format!("const ok={expression}; Object.assign(globalThis,{{fetch:other}});");
        assert!(report(&code, None).allowed, "{code}");
    }
}

#[test]
fn review_optional_call_arguments_remain_conditional() {
    for (call, expected) in [
        ("cb?.(fetch('https://example.org'))", 0),
        ("cb?.method(fetch('https://example.org'))", 0),
        ("cb?.method.next(fetch('https://example.org'))", 0),
        ("(cb?.method)(fetch('https://example.org'))", 1),
        ("cb(fetch('https://example.org'))", 1),
    ] {
        let code = format!(
            "function run(cb){{{call}; Object.assign(globalThis,{{fetch:other}});}} run(null); run(other);"
        );
        for file in ["main.js", "main.ts"] {
            let input = BTreeMap::from([(file.into(), code.as_bytes())]);
            let r = Session::default()
                .analyze(&input, file, None, &EnvironmentContract::default())
                .unwrap();
            assert_eq!(r.allowed, expected == 0, "{file}: {code}: {r:?}");
            assert_eq!(differences(&r), expected, "{file}: {code}");
        }
    }
}

#[test]
fn review_ambient_export_lists_follow_local_name_kinds() {
    for (outer, body, expected) in [
        ("", "export {};", 1),
        ("", "type Options={}; export {Options} from 'external';", 0),
        ("", "const Options: number; export {};", 0),
        (
            "declare namespace Types { type T=number; }",
            "export {Types};",
            1,
        ),
        (
            "declare namespace Values { const n: number; }",
            "export {Values};",
            0,
        ),
        ("", "export {Options}; type Options={};", 1),
        ("", "type Options={}; export {Options as Config};", 1),
        ("interface Options {}", "export {Options};", 1),
        ("const Options=1;", "type Options={}; export {Options};", 1),
        (
            "type Options=string;",
            "export const Options=1; export {Options as Config};",
            0,
        ),
        ("const Options=1;", "export {Options as Config};", 0),
        ("", "export {Unknown};", 0),
        ("", "namespace Types { type T=number; } export {Types};", 1),
        ("", "type T=number; const x=1; export {T,x};", 0),
        (
            "import type {Options} from 'some-module';",
            "export {Options};",
            1,
        ),
    ] {
        let code = format!(
            "export {{}}; {outer} declare namespace fetch {{ {body} }} fetch('https://example.org');"
        );
        let r = report(&code, None);
        assert_eq!(r.allowed, expected == 0, "{code}: {r:?}");
        assert_eq!(differences(&r), expected, "{code}");
    }
}

#[test]
fn review_destructuring_defaults_see_prior_stores_and_the_original_input() {
    for (code, expected) in [
        ("let x; [require,x=require('./dep')]=[()=>42];", 0),
        ("let x; [x=require('./dep'),require]=[undefined,()=>42];", 1),
        ("let x; ({a:require,b:x=require('./dep')}={a:()=>42});", 0),
        ("let x; [require,[x=require('./dep')]=[]]=[()=>42];", 0),
        ("let x; [require,x]=[()=>42,require('./dep')];", 1),
        ("let x; [require=require('./dep')]=[];", 1),
        ("[[require]=[require('./dep')]]=[];", 1),
        (
            "function run(cb){cb?.(require('./dep')); require=()=>{};} run(null); run(()=>{});",
            0,
        ),
        (
            "function run(src){let x; [x=require('./dep'),require]=src;} run([1,()=>{}]); run([]);",
            0,
        ),
    ] {
        let input = BTreeMap::from([
            ("main.ts".into(), code.as_bytes()),
            ("dep.ts".into(), b"fetch('https://example.org')".as_slice()),
        ]);
        let r = Session::default()
            .analyze(&input, "main.ts", None, &EnvironmentContract::default())
            .unwrap();
        assert_eq!(r.allowed, expected == 0, "{code}: {r:?}");
        assert_eq!(differences(&r), expected, "{code}");
    }
    for (targets, expected) in [
        ("globalThis.fetch,globalThis", 0),
        ("globalThis,globalThis.fetch", 1),
    ] {
        let input = if expected == 0 {
            "[()=>42,{}]"
        } else {
            "[{},()=>42]"
        };
        let code = format!("[{targets}]={input}; fetch('https://example.org');");
        assert_eq!(differences(&report(&code, None)), expected, "{code}");
    }
}

#[test]
fn condition_loads_analyze_dependencies_before_loader_replacement() {
    for (statement, expected) in [
        ("if (require('./dep')) {}", 1),
        ("switch (require('./dep')) {}", 1),
        ("while (require('./dep')) { break; }", 1),
        ("for (let d=require('./dep'); false;) {}", 1),
        ("for (;require('./dep');) {break;}", 1),
        ("let n=0; n+=require('./dep');", 1),
        ("try { /* first */ require('./dep'); } catch {}", 1),
        ("do {require('./dep');} while(false);", 1),
        ("load: require('./dep');", 1),
        ("for (;false;require('./dep')) {}", 0),
        ("for (;false;) {require('./dep');}", 0),
        ("let n=1; n ||= require('./dep');", 0),
        ("try {} catch { require('./dep'); }", 0),
        ("try { throw 1; require('./dep'); } catch {}", 0),
        ("try { missing(),require('./dep'); } catch {}", 0),
        ("try { const dep=require('./dep'); } catch {}", 1),
        ("try { { throw 1; require('./dep'); } } catch {}", 0),
        ("try { { require('./dep'); } } finally {}", 1),
        ("try {} finally { require('./dep'); }", 1),
        ("try {} finally { setup(); require('./dep'); }", 1),
        ("try { setup(); require('./dep'); } finally {}", 1),
        ("try { const n=1,d=require('./dep'); } catch {}", 1),
        ("try { const n=1; require('./dep'); } finally {}", 1),
        ("try { ; require('./dep'); } catch {}", 1),
        ("try { const opts={a:[1]}; require('./dep'); } catch {}", 1),
        (
            "try { const opts={a:missing()}; require('./dep'); } catch {}",
            0,
        ),
        ("try { 0,require('./dep'); } catch {}", 1),
        ("try { const v=`ok${1+2}`; require('./dep'); } catch {}", 1),
        ("try { const v=1n/0n; require('./dep'); } catch {}", 0),
        (
            "try { const opts={get x(){throw 1;}}; require('./dep'); } catch {}",
            1,
        ),
        (
            "try { const opts={[missing()]:0}; require('./dep'); } catch {}",
            0,
        ),
        ("try { throw require('./dep'); } catch {}", 1),
        ("try { throw 1; } finally { require('./dep'); }", 1),
        ("try { throw 1; require('./dep'); } finally {}", 0),
        ("if (false) { require('./dep'); }", 0),
        ("switch (1) { case 2: require('./dep'); }", 0),
        ("while (false) { require('./dep'); }", 0),
    ] {
        for file in ["main.ts", "main.js"] {
            let statement = if file == "main.ts" {
                statement.replace("try {", "try { type Erased = string;")
            } else {
                statement.to_owned()
            };
            let code = format!("{statement} require = () => {{}};");
            let input = BTreeMap::from([
                (file.into(), code.as_bytes()),
                ("dep.js".into(), b"fetch('https://example.org');".as_slice()),
            ]);
            let r = Session::default()
                .analyze(&input, file, None, &EnvironmentContract::default())
                .unwrap();
            assert_eq!(r.allowed, expected == 0, "{code}: {r:?}");
            assert_eq!(differences(&r), expected, "{code}: {r:?}");
            assert_eq!(
                r.diagnostics
                    .iter()
                    .any(|d| d.file == "dep.js" && d.blocking()),
                expected == 1
            );
        }
    }
}

#[test]
fn possible_receiver_and_helper_replacements_keep_dependent_writes_advisory() {
    for (writes, expected) in [
        (
            "function unused(){globalThis.globalThis={};} globalThis.fetch=()=>{};",
            0,
        ),
        (
            "if (flag) globalThis.globalThis={}; globalThis.fetch=()=>{};",
            0,
        ),
        (
            "if (flag) globalThis={}; const g=globalThis; g.fetch=()=>{};",
            0,
        ),
        (
            "function unused(){Object.assign=()=>{};} Object.assign(globalThis,{fetch:()=>{}});",
            0,
        ),
        (
            "if (flag) Object.assign=()=>{}; const set=Object.assign; set(globalThis,{fetch:()=>{}});",
            0,
        ),
        ("globalThis.globalThis ||= {}; globalThis.fetch=()=>{};", 0),
        ("globalThis ??= {}; globalThis.fetch=()=>{};", 0),
        ("for(globalThis of []) {} globalThis.fetch=()=>{};", 0),
        (
            "function unused(){globalThis.globalThis={};} globalThis.globalThis={}; globalThis.fetch=()=>{};",
            1,
        ),
        (
            "function unused(){globalThis.globalThis={};} Object.assign(globalThis,{globalThis:{}}); globalThis.fetch=()=>{};",
            1,
        ),
        (
            "function unused(){globalThis.globalThis={};} globalThis={}; globalThis.fetch=()=>{};",
            1,
        ),
        ("globalThis.globalThis={}; globalThis.fetch=()=>{};", 1),
        (
            "Object.assign=()=>{}; Object.assign(globalThis,{fetch:()=>{}});",
            1,
        ),
        (
            "function unused(){globalThis.globalThis={};} globalThis.other=()=>{};",
            1,
        ),
    ] {
        let code = format!("{writes} fetch('https://example.org');");
        let r = report(&code, None);
        assert_eq!(r.allowed, expected == 0, "{code}: {r:?}");
        assert_eq!(differences(&r), expected, "{code}: {r:?}");
    }
}

#[test]
fn restored_receivers_and_helpers_keep_later_writes_visible() {
    for (writes, expected) in [
        (
            "globalThis.globalThis=globalThis; globalThis.fetch=()=>{};",
            0,
        ),
        (
            "const root=globalThis; globalThis={}; globalThis=root; globalThis.fetch=()=>{};",
            0,
        ),
        (
            "const root=globalThis; root.globalThis={}; root.globalThis=root; globalThis.fetch=()=>{};",
            0,
        ),
        (
            "Object.assign(globalThis,{globalThis}); globalThis.fetch=()=>{};",
            0,
        ),
        (
            "const root=globalThis; root.globalThis={}; Object.assign(root,{globalThis:root}); globalThis.fetch=()=>{};",
            0,
        ),
        (
            "const set=Object.assign; Object.assign=()=>{}; Object.assign=set; Object.assign(globalThis,{fetch:()=>{}});",
            0,
        ),
        (
            "const root=globalThis; globalThis=pick(root); globalThis.fetch=()=>{};",
            0,
        ),
        (
            "const root=globalThis; globalThis={}; globalThis.fetch=()=>{}; globalThis=root;",
            1,
        ),
        (
            "const root=globalThis; globalThis=root; globalThis={}; globalThis.fetch=()=>{};",
            1,
        ),
        (
            "const different={}; const alias=different; globalThis=alias; globalThis.fetch=()=>{};",
            1,
        ),
        (
            "const set=Object.assign; Object.assign=()=>{}; Object.assign(Object,{assign:set}); Object.assign(globalThis,{fetch:()=>{}});",
            1,
        ),
        (
            "const set=Object.assign; Object.assign=()=>{}; Object[key]=set; Object.assign(globalThis,{fetch:()=>{}});",
            0,
        ),
        (
            "try { missing(globalThis={}); } catch {} globalThis.fetch=()=>{};",
            0,
        ),
    ] {
        let code = format!("{writes} fetch('https://example.org');");
        let r = report(&code, None);
        assert_eq!(differences(&r), expected, "{code}: {r:?}");
        assert_eq!(r.allowed, expected == 0, "{code}: {r:?}");
    }
}

#[test]
fn literal_patterns_and_known_functions_preserve_original_calls() {
    for (setup, expected) in [
        ("function noop() {} Object.assign=noop;", 1),
        (
            "function noop() {} const alias=noop; Object.assign=alias;",
            1,
        ),
        (
            "function noop() {} noop=Object.assign; Object.assign=noop;",
            0,
        ),
        ("Object.assign=Object.is;", 1),
        (
            "const same=Object.assign; Object.is=same; Object.assign=Object.is;",
            0,
        ),
        ("Object.assign(globalThis,{globalThis:Object});", 1),
        ("({root:globalThis}={root:{}});", 1),
        ("({root:globalThis}={root:globalThis});", 0),
        ("({root:globalThis}={root:{},...{root:globalThis}});", 0),
        (
            "({root:globalThis}={root:{},get root(){return globalThis;}});",
            0,
        ),
        ("({root:globalThis}={...{},root:{}});", 1),
        ("({nested:[globalThis]}={nested:[{}]});", 1),
        ("try{globalThis={};}finally{}", 1),
        ("try{}finally{globalThis={};}", 1),
        ("({1:globalThis}={1:{},1.0:globalThis});", 0),
        ("({1:globalThis}={1e0:{}});", 1),
        ("Object.assign=Object.keys;", 1),
        ("({['__proto__']:globalThis}={['__proto__']:{}});", 1),
        ("({root:globalThis}={get root(){return globalThis;}});", 0),
    ] {
        let code = format!(
            "{setup} Object.assign(globalThis,{{fetch:()=>{{}}}}); fetch('https://example.org');"
        );
        let r = report(&code, None);
        assert_eq!(r.allowed, expected == 0, "{code}: {r:?}");
        assert_eq!(differences(&r), expected, "{code}: {r:?}");
    }
}

fn assert_pattern_fetch_admission(setup: &str, blocked: bool) {
    let code = format!(
        "{setup} Object.assign(globalThis,{{fetch:()=>{{}}}}); fetch('https://example.org');"
    );
    let r = report(&code, None);
    assert_eq!(r.allowed, !blocked, "{code}: {r:?}");
    assert_eq!(differences(&r), usize::from(blocked), "{code}: {r:?}");
}

#[test]
fn pattern_admission_numeric_keys_only_invalidate_possible_collisions() {
    for key in ["0x10", "1.5", "[0x10]", "[-1.5]", "16n", "0b10000"] {
        assert_pattern_fetch_admission(
            &format!("({{root:globalThis}}={{root:{{}},{key}:0}});"),
            true,
        );
    }
    for key in ["0x10", "1.5", "16n", "[-1.5]"] {
        assert_pattern_fetch_admission(
            &format!("({{1:globalThis}}={{1:{{}},{key}:globalThis}});"),
            true,
        );
    }
    for (field, key) in [
        ("16", "0x10"),
        ("1.5", "1.5"),
        ("-1.5", "[-1.5]"),
        ("16", "16n"),
        ("Infinity", "1e999"),
    ] {
        assert_pattern_fetch_admission(
            &format!("({{'{field}':globalThis}}={{'{field}':{{}},{key}:globalThis}});"),
            false,
        );
    }
    assert_pattern_fetch_admission(
        "const key='root'; ({root:globalThis}={root:{},[key]:globalThis});",
        false,
    );
}

#[test]
fn pattern_admission_rest_targets_receive_a_fresh_object() {
    for setup in [
        "({...globalThis}={});",
        "const source=globalThis; ({...globalThis}=source);",
        "({root:{...globalThis}}={root:globalThis});",
        "({other:globalThis,...globalThis}={other:globalThis});",
        "([...globalThis]=[globalThis]);",
    ] {
        assert_pattern_fetch_admission(setup, true);
    }
    assert_pattern_fetch_admission(
        "const saved=globalThis; ({...globalThis}=saved); globalThis=saved;",
        false,
    );
}

#[test]
fn pattern_admission_defaults_use_definitely_defined_literal_inputs() {
    for setup in [
        "({root:globalThis=globalThis}={root:{}});",
        "({root:globalThis=Object.assign(globalThis,{fetch:()=>{}})}={root:{}});",
        "([globalThis=Object.assign(globalThis,{fetch:()=>{}})]=[{}]);",
        "({globalThis=globalThis}={globalThis:{}});",
        "([globalThis=globalThis]=[{}]);",
        "({root:[globalThis]=[globalThis]}={root:[{}]});",
    ] {
        assert_pattern_fetch_admission(setup, true);
    }
    for setup in [
        "({root:globalThis=globalThis}={});",
        "({root:globalThis=Object.assign(globalThis,{fetch:()=>{}})}={});",
        "([globalThis=globalThis]=[undefined]);",
        "({globalThis={}}={globalThis:globalThis});",
    ] {
        assert_pattern_fetch_admission(setup, false);
    }
}

#[test]
fn pattern_admission_default_effects_follow_the_literal_input() {
    for (input, expected) in [("{}", 1), ("{root: 0}", 0), ("{root: undefined}", 1)] {
        let src = format!("let value; ({{root:value=fetch('https://example.org')}}={input});");
        assert_eq!(differences(&report(&src, None)), expected, "{src}");
    }
}

#[test]
fn followup_ordinary_and_assign_writes_share_receiver_capture() {
    for write in [
        "shim.fetch=other",
        "Object.assign(shim,{fetch:other})",
        "delete shim.fetch",
        "({value:shim.fetch}=input)",
    ] {
        for (setup, expected) in [
            (
                "Object.assign(globalThis,{globalThis:{}}); const shim=globalThis;",
                1,
            ),
            (
                "const shim=globalThis; Object.assign(globalThis,{globalThis:{}});",
                0,
            ),
        ] {
            let code = format!("{setup} {write}; fetch('https://example.org');");
            assert_eq!(differences(&report(&code, None)), expected, "{code}");
        }
    }
}

#[test]
fn followup_write_receivers_in_closures_use_their_initializer_snapshot() {
    for write in ["shim.fetch=other", "Object.assign(shim,{fetch:other})"] {
        let code = format!(
            "function patch(){{{write}}} Object.assign(globalThis,{{globalThis:{{}}}}); const shim=globalThis; patch(); fetch('https://example.org');"
        );
        assert_eq!(differences(&report(&code, None)), 1, "{code}");
    }
    // Cross-function order is uncertain. Retain both possible writes but still
    // diagnose independent definite effects.
    let code = "const saved=globalThis; function patch(){ saved.globalThis={}; saved.fetch=other } patch(); fetch('https://example.org'); Bun.write('out','data');";
    let r = report(code, None);
    assert_eq!(differences(&r), 1);
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.blocking() && d.behavior == "filesystem")
    );
}

#[test]
fn followup_loader_result_members_are_read_after_loader_arguments() {
    for access in [
        "require('node:fs', patch).readFileSync('in')",
        "const read=require('node:fs', patch).readFileSync; read('in')",
    ] {
        for (key, expected) in [("readFileSync", 0), ("other", 1)] {
            let code = access.replace(
                "patch",
                &format!("Object.assign(require('node:fs'),{{{key}:()=>0}})"),
            );
            assert_eq!(differences(&report(&code, None)), expected, "{code}");
        }
    }
}

#[test]
fn followup_unrelated_member_writes_do_not_exhaust_the_alias_budget() {
    let mut src = "import fs from 'node:fs'; const alias=fs;".to_owned();
    for i in 0..100 {
        src.push_str(&format!("const value{i}={i};"));
    }
    for i in 0..100 {
        src.push_str(&format!("Object.assign(alias,{{key{i}:0}});"));
    }
    src.push_str("alias.readFileSync('in');");
    let mut session = Session::new(
        Limits {
            nodes: 10_000,
            ..Limits::default()
        },
        Arc::new(AtomicBool::new(false)),
    );
    let r = session
        .analyze(
            &sources(&src),
            "main.ts",
            None,
            &EnvironmentContract::default(),
        )
        .unwrap();
    assert_eq!(differences(&r), 1);
}

#[test]
fn object_assign_tracks_replaced_members_without_erasing_other_evidence() {
    for (setup, expected_read, expected_write) in [
        ("Object.assign(fs, {readFileSync: () => 0});", false, true),
        (
            "Object.assign(alias, {}, {['readFileSync']: other});",
            false,
            true,
        ),
        ("Object.assign(fs, {[key]: other});", false, false),
        ("Object.assign(alias, patch);", false, false),
        (
            "Object.assign(alias, {'*': other, 'nested.key': other});",
            true,
            true,
        ),
        (
            "const noop=()=>{}; Object.assign(Object, {assign: noop}); Object.assign(fs, patch);",
            true,
            true,
        ),
        ("Object.assign(alias, {...patch});", false, false),
        ("Object.assign({}, {readFileSync: other});", true, true),
        (
            "{ const Object = local; Object.assign(fs, patch); }",
            true,
            true,
        ),
        (
            "const noop=()=>{}; Object.assign = noop; Object.assign(fs, patch);",
            true,
            true,
        ),
    ] {
        let code = format!(
            "import fs from 'node:fs'; const alias = fs; const before = alias.readFileSync;\n\
             {setup}\n\
             alias.readFileSync('in');\n\
             fs.writeFileSync('out', 'data');\n\
             before('in');\n\
             const after = fs.readFileSync; after('in');"
        );
        for file in ["main.js", "main.ts"] {
            let r = Session::default()
                .analyze(
                    &BTreeMap::from([(file.into(), code.as_bytes())]),
                    file,
                    None,
                    &EnvironmentContract::default(),
                )
                .unwrap();
            for (line, definite) in [
                (3, expected_read),
                (4, expected_write),
                (5, true),
                (6, expected_read),
            ] {
                let expected = if definite {
                    "code_analysis_declaration_difference"
                } else {
                    "code_analysis_call_unknown"
                };
                assert!(
                    r.diagnostics
                        .iter()
                        .any(|d| d.line == line && d.code == expected),
                    "{file}: {setup}: {r:?}"
                );
                assert_eq!(
                    r.diagnostics.iter().any(|d| d.line == line && d.blocking()),
                    definite
                );
            }
        }
    }
}

#[test]
fn member_mutations_preserve_captured_functions_and_unrelated_members() {
    for (source, expected) in [
        (
            "import fs from 'node:fs'; const read = fs.readFileSync; const alias = fs; alias.readFileSync = () => 'ok'; read('private');",
            1,
        ),
        (
            "import fs from 'node:fs'; const alias = fs; alias.custom = 1; fs.readFileSync('private');",
            1,
        ),
        (
            "import fs from 'node:fs'; const alias = fs; alias.readFileSync = other; fs.writeFileSync('private', 'data');",
            1,
        ),
        (
            "import fs from 'node:fs'; const alias = fs; alias.readFileSync = other; const read = fs.readFileSync; read('private');",
            0,
        ),
        (
            "import fs from 'node:fs'; const read = fs.readFileSync; delete fs.readFileSync; read('private');",
            1,
        ),
        (
            "import fs from 'node:fs'; const alias = fs; alias[key] = other; fs.readFileSync('private');",
            0,
        ),
        (
            "import fs from 'node:fs'; function run() { const read = fs.readFileSync; read('private'); } fs.readFileSync = other; run();",
            0,
        ),
        (
            "const request = fetch; globalThis.fetch = other; request('https://real.invalid');",
            1,
        ),
        (
            "globalThis.fetch = other; const request = fetch; request('https://fake.invalid');",
            0,
        ),
        (
            "import fs from 'node:fs'; function local() { const fs = other; } fs.readFileSync('private');",
            1,
        ),
    ] {
        for extension in ["js", "ts"] {
            let file = format!("main.{extension}");
            let input = BTreeMap::from([(file.clone(), source.as_bytes())]);
            let r = Session::default()
                .analyze(&input, &file, None, &EnvironmentContract::default())
                .unwrap();
            assert_eq!(differences(&r), expected, "{extension}: {source}: {r:?}");
            assert!(!has(&r, "code_analysis_parse_failure"), "{r:?}");
            if expected == 0 {
                assert!(has(&r, "code_analysis_call_unknown"), "{r:?}");
            }
        }
    }
}

#[test]
fn immutable_alias_resolution_handles_forward_closures_chains_and_cycles() {
    for (source, expected) in [
        (
            "import fs from 'node:fs'; function run() { const read = original; read('private'); } const original = fs.readFileSync; run();",
            1,
        ),
        (
            "import fs from 'node:fs'; const first = fs; const second = first; second.readFileSync('private');",
            1,
        ),
        (
            "import fs from 'node:fs'; function run() { const read = original; read('private'); } const original = fs.readFileSync; fs.readFileSync = other; run();",
            1,
        ),
        (
            "import fs from 'node:fs'; function run() { const read = original; read('private'); } fs.readFileSync = other; const original = fs.readFileSync; run();",
            0,
        ),
        (
            "import fs from 'node:fs'; function run(original) { const read = original; read('private'); } const original = fs.readFileSync;",
            0,
        ),
        (
            "const first = second; const second = first; first('private');",
            0,
        ),
    ] {
        let r = report(source, None);
        assert_eq!(differences(&r), expected, "{source}: {r:?}");
        assert!(!has(&r, "code_analysis_parse_failure"), "{r:?}");
    }
    let mut session = Session::new(
        Limits {
            nodes: 200,
            ..Limits::default()
        },
        Arc::new(AtomicBool::new(false)),
    );
    let chain = (0..20)
        .map(|i| format!("const a{i} = a{};", i + 1))
        .collect::<String>()
        + "const a20 = fetch; a0('https://real.invalid');";
    assert_eq!(
        session.analyze(
            &sources(&chain),
            "main.ts",
            None,
            &EnvironmentContract::default()
        ),
        Err(TerminalFailure::Budget)
    );
}

#[test]
fn binding_patterns_exclude_defaults_keys_and_type_queries() {
    for (source, expected) in [
        (
            "function run(client = fetch) { fetch('https://real.invalid'); } run();",
            1,
        ),
        (
            "const { request = fetch } = {}; fetch('https://real.invalid');",
            1,
        ),
        (
            "function run({ request = fetch } = {}) { fetch('https://real.invalid'); }",
            1,
        ),
        (
            "const [request = fetch] = []; fetch('https://real.invalid');",
            1,
        ),
        (
            "const { [fetch.name]: request } = {}; fetch('https://real.invalid');",
            1,
        ),
        ("function run(client = fetch('https://real.invalid')) {}", 1),
        ("const { request = fetch('https://real.invalid') } = {};", 1),
        (
            "function run(fetch = other) { fetch('https://fake.invalid'); }",
            0,
        ),
        (
            "function run({ fetch = other }) { fetch('https://fake.invalid'); }",
            0,
        ),
        (
            "function run({ nested: [{ value: fetch = other }] }) { fetch('https://fake.invalid'); }",
            0,
        ),
        (
            "function run(...fetch) { fetch('https://fake.invalid'); }",
            0,
        ),
        (
            "const { network: fetch = other } = {}; fetch('https://fake.invalid');",
            0,
        ),
    ] {
        for extension in ["js", "ts"] {
            let file = format!("main.{extension}");
            let input = BTreeMap::from([(file.clone(), source.as_bytes())]);
            let r = Session::default()
                .analyze(&input, &file, None, &EnvironmentContract::default())
                .unwrap();
            assert!(!has(&r, "code_analysis_parse_failure"), "{source}: {r:?}");
            assert_eq!(differences(&r), expected, "{extension}: {source}: {r:?}");
        }
    }
    for source in [
        "function run(options: { token: typeof fetch }) { fetch('https://real.invalid'); }",
        "function run(options?: { token: typeof fetch }) { fetch('https://real.invalid'); }",
        "const run = (options: { token: typeof fetch }) => fetch('https://real.invalid');",
    ] {
        let r = report(source, None);
        assert!(!has(&r, "code_analysis_parse_failure"), "{r:?}");
        assert_eq!(differences(&r), 1, "{source}: {r:?}");
    }
}

#[test]
fn destructuring_assignments_invalidate_only_written_targets() {
    for (source, expected) in [
        (
            "({ fetch } = { fetch: other }); fetch('https://fake.invalid');",
            0,
        ),
        ("[fetch] = [other]; fetch('https://fake.invalid');", 0),
        (
            "({ network: fetch = other } = {}); fetch('https://fake.invalid');",
            0,
        ),
        (
            "({ nested: [fetch] } = input); fetch('https://fake.invalid');",
            0,
        ),
        ("[...fetch] = input; fetch('https://fake.invalid');", 0),
        (
            "import fs from 'node:fs'; ({ read: fs.readFileSync } = input); fs.readFileSync('private');",
            0,
        ),
        (
            "import fs from 'node:fs'; const alias = fs; [alias.readFileSync] = input; fs.readFileSync('private');",
            0,
        ),
        (
            "import fs from 'node:fs'; ({ read: fs.readFileSync } = input); fs.writeFileSync('private', 'data');",
            1,
        ),
        (
            "import fs from 'node:fs'; const read = fs.readFileSync; ({ read: fs.readFileSync } = input); read('private');",
            1,
        ),
        (
            "let request; ({ request = fetch } = {}); fetch('https://real.invalid');",
            1,
        ),
        (
            "let request; ({ [fetch.name]: request } = input); fetch('https://real.invalid');",
            1,
        ),
        (
            "let request; [request = fetch('https://real.invalid')] = [];",
            1,
        ),
    ] {
        for extension in ["js", "ts"] {
            let file = format!("main.{extension}");
            let input = BTreeMap::from([(file.clone(), source.as_bytes())]);
            let r = Session::default()
                .analyze(&input, &file, None, &EnvironmentContract::default())
                .unwrap();
            assert!(!has(&r, "code_analysis_parse_failure"), "{source}: {r:?}");
            assert_eq!(differences(&r), expected, "{extension}: {source}: {r:?}");
            if expected == 0 {
                assert!(has(&r, "code_analysis_call_unknown"), "{r:?}");
            }
        }
    }
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
fn saved_bun_file_compares_the_path_at_the_method_call() {
    for (method, access) in [
        ("text", "read"),
        ("json", "read"),
        ("arrayBuffer", "read"),
        ("bytes", "read"),
        ("stream", "read"),
        ("exists", "read"),
        ("writer", "write"),
        ("delete", "remove"),
    ] {
        let code = format!("const f = Bun.file('in');\nawait f.{method}();");
        let missing = report(&code, None);
        assert_eq!(differences(&missing), 1, "{missing:?}");
        assert!(
            missing
                .diagnostics
                .iter()
                .any(|d| d.blocking() && d.line == 2 && d.column == 7)
        );
        let decl = declaration(&format!(
            "filesystem=[{{base='preset',path='in',access=['{access}']}}]"
        ));
        let matched = report(&code, Some(&decl));
        assert!(
            matched.allowed
                && !has(&matched, "code_analysis_call_unknown")
                && !has(&matched, "code_analysis_declaration_uncorroborated"),
            "{matched:?}"
        );
    }
    let dynamic = report("const f=Bun.file(input); f.text();", None);
    assert!(
        dynamic.allowed && has(&dynamic, "code_analysis_dynamic_target"),
        "{dynamic:?}"
    );
    assert!(report("const f=Bun.file('in');", None).allowed);
}

#[test]
fn saved_bun_file_requires_the_original_receiver_and_method() {
    for source in [
        "function local(Bun){ const f=Bun.file('in'); f.text(); }",
        "const f=Bun.file('in'); function local(f){ f.text(); }",
        "let f=Bun.file('in'); f=other; f.text();",
        "Bun.file=other; const f=Bun.file('in'); f.text();",
        "const f=Bun.file('in'); const alias=f; alias.text=other; f.text();",
        "const f=Bun.file('in'); Object.assign(f,{[key]:other}); f.text();",
        "const f=Bun.file('in'); const detached=f.text; detached();",
        "Object.assign(Bun,{file:fake}); const f=Bun.file('in'); f.text();",
        "const f=other.file('in'); f.text();",
    ] {
        let r = report(source, None);
        assert!(
            r.allowed && has(&r, "code_analysis_call_unknown"),
            "{source}: {r:?}"
        );
    }
    let r = report(
        "const f=Bun.file('in'); const alias=f; const g=Bun.file('in'); Object.assign(f,{text:other});\nalias.text();\nalias.json();\ng.text();",
        None,
    );
    let blocked: Vec<_> = r
        .diagnostics
        .iter()
        .filter(|d| d.blocking())
        .map(|d| d.line)
        .collect();
    assert_eq!(blocked, [3, 4], "{r:?}");
    let captured = report("const f=Bun.file('in'); Bun.file=other; f.text();", None);
    assert_eq!(differences(&captured), 1, "{captured:?}");
}

#[test]
fn copy_and_rename_compare_both_paths_independently() {
    for (callee, access, callback) in [
        ("fs.copyFileSync", "read", ""),
        ("fs.copyFile", "read", ", done"),
        ("fs.promises.copyFile", "read", ""),
        ("moveFile", "remove", ""),
        ("fs.renameSync", "remove", ""),
        ("fs.rename", "remove", ", done"),
    ] {
        let prelude =
            "import fs from 'node:fs'; import {rename as moveFile} from 'node:fs/promises';";
        let decl = declaration(&format!(
            "filesystem=[{{base='preset',path='src',access=['{access}']}},{{base='preset',path='dst',access=['write']}}]"
        ));
        for (args, missing, dynamic) in [
            ("'src', /* destination */ 'dst'", 0, false),
            ("'dst', 'src'", 2, false),
            ("input, 'other'", 1, true),
            ("'other', output", 1, true),
            ("...paths, 'other'", 0, true),
            ("'src', ...paths", 0, true),
        ] {
            let code = format!("{prelude} {callee}({args}{callback});");
            let r = report(&code, Some(&decl));
            assert_eq!(differences(&r), missing, "{code}: {r:?}");
            assert_eq!(
                has(&r, "code_analysis_dynamic_target"),
                dynamic,
                "{code}: {r:?}"
            );
            assert!(!has(&r, "code_analysis_call_unknown"), "{code}: {r:?}");
            if !dynamic && missing == 0 {
                assert!(r.diagnostics.is_empty(), "{code}: {r:?}");
            }
        }
    }
    let distinct = report(
        "import fs from 'node:fs'; fs['promises.rename'] = other; fs.promises.rename('a', 'b');",
        None,
    );
    assert_eq!(differences(&distinct), 2, "{distinct:?}");
    for code in [
        "import fs from 'node:fs'; fs['promises.rename']('a', 'b');",
        "import p from 'node:fs/promises'; new p.rename('a','b');",
        "import fs from 'node:fs'; fs.rename('a','b');",
        "import fs from 'node:fs'; fs.copyFileSync('a');",
        "import fs from 'node:fs'; import p from 'node:fs/promises'; Object.assign(p,{rename: fn}); fs.promises.rename('a','b');",
        "function f(fs) { fs.copyFileSync('a','b'); }",
        "import fs from 'node:fs'; Object.assign(fs, {renameSync: fn}); fs.renameSync('a','b');",
        "import fs from 'node:fs/promises'; fs.copyFileSync('a','b');",
    ] {
        let r = report(code, None);
        assert!(
            r.allowed && has(&r, "code_analysis_call_unknown"),
            "{code}: {r:?}"
        );
    }
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
fn binding_scope_walks_consume_the_command_budget() {
    // Few syntax nodes can still require a long ancestor walk for a var binding.
    let source = "{".repeat(128) + "var local = 0;" + &"}".repeat(128);
    let input = sources(&source);
    assert!(
        Session::default()
            .analyze(&input, "main.ts", None, &EnvironmentContract::default())
            .unwrap()
            .allowed
    );
    let mut session = Session::new(
        Limits {
            nodes: 450,
            ..Limits::default()
        },
        Arc::new(AtomicBool::new(false)),
    );
    assert_eq!(
        session.analyze(&input, "main.ts", None, &EnvironmentContract::default()),
        Err(TerminalFailure::Budget)
    );
    assert_eq!(
        session.analyze(
            &sources("export {};"),
            "main.ts",
            None,
            &EnvironmentContract::default()
        ),
        Err(TerminalFailure::Budget)
    );
}

#[test]
fn binding_scope_cache_bounds_siblings_without_leaking_namespace_vars() {
    let locals = (0..128)
        .map(|i| format!("var local{i} = 0;"))
        .collect::<String>();
    let source = "namespace Local {".to_owned()
        + &"{".repeat(128)
        + "var fetch = custom;"
        + &locals
        + &"}".repeat(128)
        + "fetch('https://local.invalid'); } fetch('https://example.org');";
    let mut session = Session::new(
        Limits {
            nodes: 5000,
            ..Limits::default()
        },
        Arc::new(AtomicBool::new(false)),
    );
    let r = session
        .analyze(
            &sources(&source),
            "main.ts",
            None,
            &EnvironmentContract::default(),
        )
        .unwrap();
    assert_eq!(differences(&r), 1);
    assert!(has(&r, "code_analysis_call_unknown"));
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
