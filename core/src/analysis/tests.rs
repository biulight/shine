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
