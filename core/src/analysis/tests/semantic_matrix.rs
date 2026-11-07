use super::*;

// Equivalent source layouts move locations and may repeat the same advisory.
// Compare admission and diagnostic identities; location/count tests remain separate.
type DiagnosticIdentity = (DiagnosticKind, String, String, Option<String>);

fn signature(r: &Report) -> (bool, BTreeSet<DiagnosticIdentity>) {
    (
        r.allowed,
        r.diagnostics
            .iter()
            .map(|d| {
                (
                    d.kind,
                    d.code.clone(),
                    d.behavior.clone(),
                    d.identity.clone(),
                )
            })
            .collect(),
    )
}

#[test]
fn recognized_calls_keep_identity_across_spelling_and_wrapper_matrix() {
    for (setup, object, key, escaped_key, args, permission) in [
        (
            "",
            "globalThis",
            "fetch",
            r"f\u0065tch",
            "'https://example.org'",
            "network=[{scope='host',host='example.org'}]",
        ),
        (
            "import fs from 'node:fs';",
            "fs",
            "readFileSync",
            r"read\u0046ileSync",
            "'in'",
            "filesystem=[{base='preset',path='in',access=['read']}]",
        ),
        (
            "",
            "Bun",
            "write",
            r"wr\u0069te",
            "'out','data'",
            "filesystem=[{base='preset',path='out',access=['write']}]",
        ),
        (
            "import cp from 'node:child_process';",
            "cp",
            "spawn",
            r"sp\u0061wn",
            "'echo'",
            "commands=['echo']",
        ),
    ] {
        let plain = format!("{setup} {object}.{key}({args});");
        let declared = declaration(permission);
        let expected_missing = report(&plain, None);
        let expected_matching = report(&plain, Some(&declared));
        assert_eq!(differences(&expected_missing), 1, "{plain}");
        assert!(expected_matching.allowed, "{plain}");
        for member in [
            format!("{object}.{key}"),
            format!("({object})['{key}']"),
            format!("{object}['{escaped_key}']"),
        ] {
            for callee in [
                member.clone(),
                format!("({member})"),
                format!("({member} as unknown)"),
                format!("({member} satisfies unknown)"),
                format!("(<unknown>{member})"),
                format!("{member}!"),
                format!("({member}<unknown>)"),
            ] {
                for body in [
                    format!("{callee}({args});"),
                    format!("const invoke={callee}; invoke({args});"),
                ] {
                    let source = format!("{setup} {body}");
                    assert_eq!(
                        signature(&report(&source, None)),
                        signature(&expected_missing),
                        "{source}"
                    );
                    assert_eq!(
                        signature(&report(&source, Some(&declared))),
                        signature(&expected_matching),
                        "{source}"
                    );
                }
            }
        }
    }
}

#[test]
fn equivalent_builtin_import_forms_keep_the_same_object_identity() {
    for (module, call) in [
        ("node:fs", "api.readFileSync('in')"),
        ("node:fs/promises", "api.readFile('in')"),
        ("node:https", "api.get('https://example.org')"),
    ] {
        let expected = report(&format!("import api from '{module}'; {call};"), None);
        assert_eq!(differences(&expected), 1);
        for import in [
            format!("import * as api from '{module}';"),
            format!("import {{default as api}} from '{module}';"),
            format!(r"import {{'def\u0061ult' as api}} from '{module}';"),
            format!("const api=require('{module}');"),
        ] {
            let code = format!("{import} {call};");
            for file in ["main.js", "main.ts"] {
                let r = Session::default()
                    .analyze(
                        &BTreeMap::from([(file.into(), code.as_bytes())]),
                        file,
                        None,
                        &EnvironmentContract::default(),
                    )
                    .unwrap();
                assert_eq!(signature(&r), signature(&expected), "{file}: {code}");
            }
        }
    }
}

#[test]
fn member_write_matrix_preserves_unrelated_and_captured_functions() {
    for dynamic in [false, true] {
        let reference = if dynamic {
            "alias[key]=other"
        } else {
            "alias.readFileSync=other"
        };
        let build = |write: &str| {
            format!(
                "import fs from 'node:fs'; const alias=(fs); const before=alias.readFileSync; {write}; alias.readFileSync('in'); fs.writeFileSync('out','data'); before('in');"
            )
        };
        let expected = report(&build(reference), None);
        assert_eq!(differences(&expected), if dynamic { 1 } else { 2 });
        for object in ["fs", "alias", "(alias as typeof fs)"] {
            let keys = if dynamic {
                vec!["key"]
            } else {
                vec![
                    "'readFileSync'",
                    r"'read\u0046ileSync'",
                    "('readFileSync' as const)",
                ]
            };
            for key in keys {
                for write in [
                    format!("{object}[{key}]=other"),
                    format!("namespace Changes {{ {object}[{key}]=other; }}"),
                    format!("Object.assign({object}, {{[{key}]:other}})"),
                    format!("(Object.assign)({object}, ({{[{key}]:other}} as object))"),
                ] {
                    let source = build(&write);
                    let actual = report(&source, None);
                    assert_eq!(
                        differences(&actual),
                        differences(&expected),
                        "{source}: {actual:?}"
                    );
                    assert_eq!(signature(&actual), signature(&expected), "{source}");
                }
            }
        }
    }
}

#[test]
fn environment_pattern_writes_have_the_same_read_set_as_direct_writes() {
    let env = EnvironmentContract {
        fixed: BTreeSet::new(),
        mappings: ["READ", "WRITE"]
            .into_iter()
            .map(|name| {
                (
                    name.into(),
                    (name.into(), Some(EnvironmentSensitivityV1::Plain)),
                )
            })
            .collect(),
    };
    let analyze = |source: &str| {
        Session::default()
            .analyze(&sources(source), "main.ts", None, &env)
            .unwrap()
    };
    let expected = analyze("process.env.READ;");
    assert_eq!(differences(&expected), 1);
    for source in [
        "process.env.WRITE=other; process.env.READ;",
        "({value:process.env.WRITE}=input); process.env.READ;",
        "[process.env.WRITE]=input; process.env.READ;",
        "({value:[process.env.WRITE]}=input); process.env.READ;",
        "[...process.env.WRITE]=input; process.env.READ;",
        "for ({value:process.env.WRITE} of items) {} process.env.READ;",
        "delete (process.env.WRITE); process.env.READ;",
        "({[process.env.READ]:process.env.WRITE}=input);",
        "({value:process.env.WRITE=process.env.READ}=input);",
        "[process.env.WRITE]=[process.env.READ];",
    ] {
        let r = analyze(source);
        assert_eq!(differences(&r), 1, "{source}: {r:?}");
        assert_eq!(signature(&r), signature(&expected), "{source}");
    }
    for source in [
        "process.env.WRITE+=1; process.env.READ;",
        "process.env.WRITE++; process.env.READ;",
        "process.env.WRITE.member=other; process.env.READ;",
    ] {
        assert_eq!(differences(&analyze(source)), 2, "{source}");
    }
}

#[test]
fn erased_type_positions_do_not_add_effects_or_mutations() {
    let expected = signature(&report("fetch('https://example.org');", None));
    for annotation in [
        "type T = import('./missing').T;",
        "let typed: import('./missing').T;",
        "interface T { [process.env.KEY]: unknown }",
        "type T = { [process.env.KEY]: unknown };",
        "const typed = 0 as { [process.env.KEY]: unknown };",
        "const typed = 0 satisfies { [process.env.KEY]: unknown };",
        "function typed<T extends import('./missing').T>() {}",
        "declare class Typed { [Object.assign(globalThis,{fetch:other})]: unknown }",
    ] {
        let source = format!("{annotation} fetch('https://example.org');");
        let r = report(&source, None);
        assert_eq!(signature(&r), expected, "{source}: {r:?}");
    }
    for source in [
        "const value: unknown = fetch('https://example.org');",
        "class C { [fetch('https://example.org')] = 1 }",
        "enum E { Value = fetch('https://example.org') }",
        "namespace N { fetch('https://example.org'); }",
        "function f(value:unknown=fetch('https://example.org')) {}",
    ] {
        assert_eq!(differences(&report(source, None)), 1, "{source}");
    }
}

#[test]
fn unknown_value_declarations_shadow_globals_without_leaking_scope() {
    for source in [
        "const Bun=other; Bun.write('out','data');",
        "class Bun { static write(...args:unknown[]){} } Bun.write('out','data');",
        "namespace Bun { export function write(...args:unknown[]){} } Bun.write('out','data');",
        "enum Bun { write } Bun.write('out','data');",
        "import Bun = require('unknown-package'); Bun.write('out','data');",
        "import Bun = Local.Bun; Bun.write('out','data');",
        "declare const Bun: {write:Function}; Bun.write('out','data');",
        "declare function fetch(url:string):void; fetch('https://example.org');",
    ] {
        let r = report(source, None);
        assert!(
            r.allowed && has(&r, "code_analysis_call_unknown"),
            "{source}: {r:?}"
        );
    }
    for source in [
        "namespace N { namespace Bun { export function write(...args:unknown[]){} } Bun.write('out','data'); } Bun.write('out','data');",
        "namespace N.Bun { export function write(...args:unknown[]){} } Bun.write('out','data');",
        "namespace N.Bun { Bun.write('out','data'); } Bun.write('out','data');",
        "namespace N { var fetch=other; } fetch('https://example.org');",
        "namespace N { export var fetch=other; } fetch('https://example.org');",
        "module N { var fetch=other; } fetch('https://example.org');",
        "declare module 'external' { const Bun:unknown; } Bun.write('out','data');",
        "declare module 'external' { var Bun:unknown; } Bun.write('out','data');",
        "declare module 'external' { import fetch from 'unknown-package'; export {fetch}; } fetch('https://example.org');",
        "interface Bun { value:unknown } Bun.write('out','data');",
        "type Bun = unknown; Bun.write('out','data');",
        "import type Bun = require('unknown-package'); Bun.write('out','data');",
    ] {
        assert_eq!(differences(&report(source, None)), 1, "{source}");
    }
}

#[test]
fn unsupported_value_evaluation_retains_advisory_gaps() {
    for source in [
        "const target=choose(fetch,other); target('https://example.org');",
        "const target=flag ? fetch : other; target('https://example.org');",
        "const {fetch:target}=globalThis; target('https://example.org');",
        "fetch(url);",
        "import {request} from 'unknown-package'; request('https://example.org');",
        "import {get} from 'node:https'; get('https://example.org', options);",
    ] {
        let r = report(source, None);
        assert!(
            r.allowed
                && r.diagnostics
                    .iter()
                    .any(|d| d.kind == DiagnosticKind::CoverageGap),
            "{source}: {r:?}"
        );
    }
}

#[test]
fn erased_syntax_keeps_validation_budgets_and_runtime_locations() {
    for source in [
        "type T = import('./missing';",
        "interface T { value: ; }",
        "declare class T { [process.env.KEY",
        r"type T = typeof \uD800;",
    ] {
        let r = report(source, None);
        assert!(
            !r.allowed && has(&r, "code_analysis_parse_failure"),
            "{source}: {r:?}"
        );
    }
    let source = format!("interface T {{ {} }}", "value: string;".repeat(100));
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
    let r = report(
        "// 中文\ntype T = import('./missing').T;\nfetch('https://example.org/private');",
        None,
    );
    assert_eq!(r.diagnostics.len(), 1, "{r:?}");
    assert_eq!((r.diagnostics[0].line, r.diagnostics[0].column), (3, 1));
    assert!(!format!("{r:?}").contains("example.org"));
}
