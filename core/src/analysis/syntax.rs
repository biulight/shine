use super::*;
use std::borrow::Cow;
use tree_sitter::{Node, ParseOptions, Parser};

pub(super) fn builtin(spec: &str) -> Option<&str> {
    match spec.strip_prefix("node:").unwrap_or(spec) {
        "fs" => Some("fs"),
        "https" => Some("https"),
        "fs/promises" => Some("fs.promises"),
        "child_process" => Some("child_process"),
        "process" => Some("process"),
        "bun" => Some("Bun"),
        _ => None,
    }
}

const BUILTIN_GLOBALS: [&str; 7] = [
    "globalThis",
    "Bun",
    "process",
    "fetch",
    "eval",
    "require",
    "Object",
];

struct Binding {
    name: String,
    start: usize,
    end: usize,
    value: Option<String>,
}

type BindingIdentity = (String, usize, usize);

impl Binding {
    fn identity(&self) -> BindingIdentity {
        (self.name.clone(), self.start, self.end)
    }
}

struct Mutation {
    first: usize,
    scope: Option<(usize, usize)>,
}

fn record_mutation(
    mutations: &mut BTreeMap<String, Mutation>,
    path: String,
    position: (usize, usize, usize),
) {
    let (offset, start, end) = position;
    mutations
        .entry(path)
        .and_modify(|mutation| {
            mutation.first = mutation.first.min(offset);
            if mutation.scope != Some((start, end)) {
                mutation.scope = None;
            }
        })
        .or_insert(Mutation {
            first: offset,
            scope: Some((start, end)),
        });
}

// A write through the global object replaces the slot used by the bare name.
fn record_member_mutation(
    mutations: &mut BTreeMap<String, Mutation>,
    object: &str,
    key: Option<&str>,
    position: (usize, usize, usize),
) -> bool {
    if key.is_some_and(|k| k.contains(['.', '*'])) {
        return false;
    }
    match (object, key) {
        ("globalThis", None) => {
            for name in BUILTIN_GLOBALS {
                record_mutation(mutations, name.into(), position);
            }
        }
        ("globalThis", Some(key)) if BUILTIN_GLOBALS.contains(&key) => {
            record_mutation(mutations, key.into(), position)
        }
        ("globalThis", _) => return false,
        _ => record_mutation(
            mutations,
            format!("{object}.{}", key.unwrap_or("*")),
            position,
        ),
    }
    true
}

#[derive(Default)]
struct Bindings {
    names: BTreeMap<String, Vec<Binding>>,
    mutations: BTreeMap<String, Mutation>,
    // Per-allocation-site identities share paths without embedding literals in member keys.
    bun_files: BTreeMap<String, Option<Arc<str>>>,
}
impl Bindings {
    fn push(&mut self, binding: Binding) {
        self.names
            .entry(binding.name.clone())
            .or_default()
            .push(binding);
    }
    fn named(&self, name: &str) -> &[Binding] {
        self.names.get(name).map(Vec::as_slice).unwrap_or(&[])
    }
    fn named_mut(&mut self, name: &str) -> &mut [Binding] {
        self.names
            .get_mut(name)
            .map(Vec::as_mut_slice)
            .unwrap_or(&mut [])
    }

    // Distinguish an unbound global from an ambiguous local binding. Reads and
    // writes must resolve the same lexical identity, not every matching name.
    fn visible(&self, name: &str, node: Node<'_>) -> Result<Option<&Binding>, ()> {
        let bindings = self.named(name);
        if bindings.len() > 128 {
            return Err(());
        }
        let mut closest: Option<&Binding> = None;
        let mut ambiguous = false;
        for binding in bindings
            .iter()
            .filter(|b| b.start <= node.start_byte() && node.end_byte() <= b.end)
        {
            match closest {
                Some(previous) if previous.end - previous.start < binding.end - binding.start => {}
                Some(previous) if previous.end - previous.start == binding.end - binding.start => {
                    ambiguous = true;
                }
                _ => {
                    closest = Some(binding);
                    ambiguous = false;
                }
            }
        }
        if ambiguous { Err(()) } else { Ok(closest) }
    }

    fn unchanged(&self, path: &str, capture: Option<(usize, usize, usize)>) -> bool {
        self.mutations.get(path).is_none_or(|mutation| {
            // Only a preceding capture in the same function can retain the
            // original value. Cross-function execution order is unknown.
            capture.is_some_and(|(at, start, end)| {
                at < mutation.first && mutation.scope == Some((start, end))
            })
        })
    }

    fn member(
        &self,
        object: &str,
        key: &str,
        capture: Option<(usize, usize, usize)>,
    ) -> Option<String> {
        if object == "globalThis" {
            return (key == "fetch" && self.unchanged("fetch", capture)).then(|| key.to_owned());
        }
        // Dots separate components and '*' denotes a dynamic write in this index.
        if key.contains(['.', '*']) {
            return None;
        }
        let member = format!("{object}.{key}");
        (self.unchanged(&member, capture) && self.unchanged(&format!("{object}.*"), capture))
            .then_some(member)
    }
}

pub(super) fn parse(
    session: &mut Session,
    file: &str,
    bytes: &[u8],
) -> Result<Facts, TerminalFailure> {
    let mut facts = Facts::default();
    let extension = file.rsplit('.').next().unwrap_or("");
    let language = match extension {
        "ts" | "mts" | "cts" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX,
        "js" | "jsx" | "mjs" | "cjs" => tree_sitter_javascript::LANGUAGE,
        _ => {
            gap(
                &mut facts,
                file,
                None,
                "code_analysis_language_unsupported",
                "source",
            );
            return Ok(facts);
        }
    };
    if std::str::from_utf8(bytes).is_err() {
        facts.diagnostics.push(Diagnostic::new(
            DiagnosticKind::Failure,
            "code_analysis_encoding",
            file,
            1,
            1,
            "source",
        ));
        return Ok(facts);
    }
    let mut parser = Parser::new();
    parser
        .set_language(&language.into())
        .map_err(|_| TerminalFailure::Analyzer)?;
    let tree = {
        let mut progress = |_: &tree_sitter::ParseState| {
            session.parser_steps += 1;
            session.budget().is_err()
        };
        parser.parse_with_options(
            &mut |offset, _| &bytes[offset..],
            None,
            Some(ParseOptions::new().progress_callback(&mut progress)),
        )
    };
    session.check()?;
    let tree = tree.ok_or(TerminalFailure::Analyzer)?;
    let mut nodes = Vec::new();
    let mut pending = vec![(tree.root_node(), false, false)];
    while let Some((node, type_only, ambient)) = pending.pop() {
        session.nodes += 1;
        session.budget()?;
        if node.is_error()
            || node.is_missing()
            || (is_identifier(node) && decode_js(raw_text(node, bytes), true).is_none())
        {
            let p = node.start_position();
            facts.diagnostics.push(Diagnostic::new(
                DiagnosticKind::Failure,
                "code_analysis_parse_failure",
                file,
                p.row + 1,
                p.column + 1,
                "source",
            ));
            // No definite behavior is claimed from a recovered/partial tree.
            return Ok(facts);
        }
        // Syntax validation/budgets cover all nodes. Pure types contribute neither
        // bindings nor effects; ambient declarations retain unknown value bindings.
        let type_only = type_only || is_type_syntax(node);
        let ambient = ambient
            || node.kind() == "ambient_declaration"
            || (matches!(node.kind(), "internal_module" | "module")
                && node
                    .child_by_field_name("name")
                    .is_some_and(|n| n.kind() == "string"));
        if node.is_named() && !type_only {
            nodes.push((node, !ambient));
        }
        let operand = wrapped_operand(node);
        let mut cursor = node.walk();
        // Missing delimiters are anonymous nodes too; never accept a recovered tree.
        pending.extend(node.children(&mut cursor).map(|child| {
            (
                child,
                type_only
                    || operand.is_some_and(|value| {
                        child.is_named() && !child.is_extra() && child != value
                    }),
                ambient,
            )
        }));
    }
    let module = matches!(extension, "mjs" | "mts")
        || nodes
            .iter()
            .any(|(n, _)| matches!(n.kind(), "import_statement" | "export_statement"));
    let mut bindings = Bindings::default();
    let mut binding_scopes = BTreeMap::new();
    for (node, runtime) in &nodes {
        match node.kind() {
            "import_statement" => {
                if type_only_module(*node) {
                    continue;
                }
                let source = node
                    .child_by_field_name("source")
                    .and_then(|n| literal(n, bytes));
                if let Some(source) = &source
                    && *runtime
                {
                    let p = node.start_position();
                    facts
                        .imports
                        .push((source.clone(), p.row + 1, p.column + 1));
                } else if *runtime {
                    gap(
                        &mut facts,
                        file,
                        Some(*node),
                        "code_analysis_dependency_unsupported",
                        "dependency",
                    );
                }
                // An unresolved module still declares local runtime bindings.
                // Retain their shadowing without claiming any builtin identity.
                let module = source.as_deref().filter(|_| *runtime).and_then(builtin);
                let location = scope(*node);
                for child in descendants(*node) {
                    let binding = match child.kind() {
                        "import_specifier" if !has_token(child, "type") => {
                            let name = child.child_by_field_name("name");
                            let alias = child.child_by_field_name("alias").or(name);
                            alias.map(|alias| {
                                (
                                    text(alias, bytes),
                                    module.zip(name).and_then(|(m, n)| {
                                        let key = if n.kind() == "string" {
                                            decoded_string(n, bytes)?
                                        } else {
                                            text(n, bytes).into_owned()
                                        };
                                        if key == "default" {
                                            Some(m.to_owned())
                                        } else {
                                            bindings.member(m, &key, None)
                                        }
                                    }),
                                )
                            })
                        }
                        "namespace_import" => first_named_child(child)
                            .map(|n| (text(n, bytes), module.map(str::to_string))),
                        // Keep the declared name even though this loader syntax
                        // is outside the supported module-resolution forms.
                        "import_require_clause" => {
                            first_named_child(child).map(|n| (text(n, bytes), None))
                        }
                        "identifier"
                            if child.parent().is_some_and(|p| p.kind() == "import_clause") =>
                        {
                            Some((text(child, bytes), module.map(str::to_string)))
                        }
                        _ => None,
                    };
                    if let Some((name, value)) = binding {
                        bindings.push(Binding {
                            name: name.into(),
                            start: location.start_byte(),
                            end: location.end_byte(),
                            value,
                        });
                    }
                }
            }
            "variable_declarator" => {
                if let Some(name) = node.child_by_field_name("name") {
                    let scope = if node
                        .parent()
                        .is_some_and(|p| p.kind() == "variable_declaration")
                    {
                        function_binding_scope(session, *node, &mut binding_scopes)?
                    } else {
                        scope(*node)
                    };
                    for id in pattern_names(name) {
                        bindings.push(Binding {
                            name: text(id, bytes).into(),
                            start: scope.start_byte(),
                            end: scope.end_byte(),
                            value: None,
                        });
                    }
                }
            }
            "function_expression" | "generator_function" | "class" => {
                if let Some(name) = node.child_by_field_name("name") {
                    bindings.push(Binding {
                        name: text(name, bytes).into(),
                        start: node.start_byte(),
                        end: node.end_byte(),
                        value: None,
                    });
                }
            }
            "function_declaration"
            | "generator_function_declaration"
            | "class_declaration"
            | "function_signature"
            | "enum_declaration" => {
                if let Some(name) = node.child_by_field_name("name") {
                    let mut scope = scope(node.parent().unwrap_or(*node));
                    if node.kind() == "function_declaration" && !module && scope.kind() != "program"
                    {
                        // Sloppy-script block functions can be hoisted (Annex B).
                        // Without an explicit module boundary, do not claim the
                        // outer spelling still denotes an unshadowed global.
                        scope = function_binding_scope(session, *node, &mut binding_scopes)?;
                    }
                    bindings.push(Binding {
                        name: text(name, bytes).into(),
                        start: scope.start_byte(),
                        end: scope.end_byte(),
                        value: None,
                    });
                }
            }
            "import_alias" if !has_token(*node, "type") => {
                if let Some(name) = first_named_child(*node) {
                    let location = scope(*node);
                    bindings.push(Binding {
                        name: text(name, bytes).into(),
                        start: location.start_byte(),
                        end: location.end_byte(),
                        value: None,
                    });
                    if *runtime {
                        gap(
                            &mut facts,
                            file,
                            Some(*node),
                            "code_analysis_binding_unknown",
                            "code",
                        );
                    }
                }
            }
            "internal_module" | "module" => {
                if let Some(name) = node.child_by_field_name("name") {
                    // A.B creates A in the enclosing scope and B inside the
                    // namespace body, not a global B. No namespace value inference.
                    for id in descendants(name)
                        .into_iter()
                        .filter(|n| matches!(n.kind(), "identifier" | "property_identifier"))
                    {
                        let location = if id.start_byte() == name.start_byte() {
                            scope(node.parent().unwrap_or(*node))
                        } else if let Some(body) = node.child_by_field_name("body") {
                            body
                        } else {
                            continue;
                        };
                        bindings.push(Binding {
                            name: text(id, bytes).into(),
                            start: location.start_byte(),
                            end: location.end_byte(),
                            value: None,
                        });
                    }
                }
            }
            "formal_parameters" | "required_parameter" | "optional_parameter" => {
                if node.parent().is_some_and(|p| {
                    matches!(
                        p.kind(),
                        "formal_parameters" | "required_parameter" | "optional_parameter"
                    )
                }) {
                    continue;
                }
                let parent = node.parent().unwrap_or(*node);
                for id in pattern_names(*node) {
                    bindings.push(Binding {
                        name: text(id, bytes).into(),
                        // Computed method keys/decorators run in the outer scope,
                        // before the parameter environment exists.
                        start: node.start_byte(),
                        end: parent.end_byte(),
                        value: None,
                    });
                }
            }
            "arrow_function" => {
                if let Some(id) = node.child_by_field_name("parameter") {
                    bindings.push(Binding {
                        name: text(id, bytes).into(),
                        start: node.start_byte(),
                        end: node.end_byte(),
                        value: None,
                    });
                }
            }
            "for_in_statement" => {
                // Without a declaration keyword this is a write to an existing
                // binding/member, handled with the other assignment targets.
                if node.child_by_field_name("kind").is_none() {
                    continue;
                }
                if let Some(left) = node.child_by_field_name("left") {
                    let scope = if node
                        .child_by_field_name("kind")
                        .is_some_and(|n| n.kind() == "var")
                    {
                        function_binding_scope(session, *node, &mut binding_scopes)?
                    } else {
                        *node
                    };
                    for id in pattern_names(left) {
                        bindings.push(Binding {
                            name: text(id, bytes).into(),
                            start: scope.start_byte(),
                            end: scope.end_byte(),
                            value: None,
                        });
                    }
                }
            }
            "with_statement" if *runtime => {
                for name in BUILTIN_GLOBALS {
                    bindings.push(Binding {
                        name: name.into(),
                        start: 0,
                        end: bytes.len(),
                        value: None,
                    });
                }
                gap(
                    &mut facts,
                    file,
                    Some(*node),
                    "code_analysis_binding_unknown",
                    "code",
                );
            }
            "catch_clause" => {
                if let Some(parameter) = node.child_by_field_name("parameter") {
                    for id in pattern_names(parameter) {
                        bindings.push(Binding {
                            name: text(id, bytes).into(),
                            start: node.start_byte(),
                            end: node.end_byte(),
                            value: None,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    let aliases = nodes
        .iter()
        .rev()
        .filter_map(|(node, runtime)| {
            if !runtime
                || node.kind() != "variable_declarator"
                || !node.parent().is_some_and(|p| has_token(p, "const"))
            {
                return None;
            }
            let name = node.child_by_field_name("name")?;
            let value = node.child_by_field_name("value")?;
            (name.kind() == "identifier").then_some((*node, name, value))
        })
        .collect::<Vec<_>>();
    resolve_aliases(session, &aliases, bytes, &mut bindings, &BTreeSet::new())?;
    // Capture object identities before invalidating any spelling. Otherwise the
    // traversal order could hide a later mutation through another alias.
    let mut mutations: BTreeMap<String, Mutation> = BTreeMap::new();
    let mut mutated_bindings = BTreeSet::new();
    let mut assign_calls = Vec::new();
    let mut write_only = BTreeSet::new();
    for (node, runtime) in &nodes {
        if !runtime {
            continue;
        }
        if node.kind() == "call_expression"
            && node
                .child_by_field_name("function")
                .and_then(|f| expression(f, bytes, &bindings))
                .as_deref()
                == Some("Object.assign")
        {
            assign_calls.push(*node);
        }
        if matches!(
            node.kind(),
            "assignment_expression" | "augmented_assignment_expression" | "update_expression"
        ) || (node.kind() == "unary_expression" && has_token(*node, "delete"))
            || (node.kind() == "for_in_statement" && node.child_by_field_name("kind").is_none())
        {
            let target = node
                .child_by_field_name("left")
                .or_else(|| node.child_by_field_name("argument"));
            for target in target.into_iter().flat_map(pattern_targets) {
                // The same pattern walk owns both mutation targets and read/write
                // roles. Computed keys and default RHS expressions remain reads.
                if matches!(
                    node.kind(),
                    "assignment_expression" | "for_in_statement" | "unary_expression"
                ) {
                    write_only.insert(target.id());
                }
                // Updating a named env slot does not rebind process/Bun or env.
                if target
                    .child_by_field_name("object")
                    .and_then(|n| expression(n, bytes, &bindings))
                    .is_some_and(|n| matches!(n.as_str(), "process.env" | "Bun.env"))
                {
                    continue;
                }
                let location = function_scope(*node);
                let position = (
                    if node.kind() == "for_in_statement" {
                        // Evaluate the iterable/object before the first target
                        // write; a loader call in that input is still captured.
                        node.child_by_field_name("right")
                            .map_or(node.start_byte(), |right| right.end_byte())
                    } else {
                        node.start_byte()
                    },
                    location.start_byte(),
                    location.end_byte(),
                );
                if let Some(object_node) = target.child_by_field_name("object") {
                    if let Some(object) = expression(object_node, bytes, &bindings) {
                        record_member_mutation(
                            &mut mutations,
                            &object,
                            member_key(target, bytes).as_deref(),
                            position,
                        );
                    }
                } else {
                    let name = text(target, bytes);
                    match bindings.visible(&name, target) {
                        Ok(Some(binding)) => {
                            mutated_bindings.insert(binding.identity());
                        }
                        Ok(None) if BUILTIN_GLOBALS.contains(&name.as_ref()) => {
                            record_mutation(&mut mutations, name.into(), position);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    for (name, start, end) in &mutated_bindings {
        for binding in bindings.named_mut(name) {
            if binding.start == *start && binding.end == *end {
                binding.value = None;
            }
        }
    }
    bindings.mutations = mutations;
    // Recompute captured values with member mutations applied at each initializer.
    // Copied function values do not re-read the object's subsequently changed slot.
    let recompute_aliases = |session: &mut Session, bindings: &mut Bindings| {
        for (node, name, _) in &aliases {
            let scope = scope(*node);
            for binding in bindings.named_mut(&text(*name, bytes)) {
                if binding.start == scope.start_byte() && binding.end == scope.end_byte() {
                    binding.value = None;
                }
            }
        }
        resolve_aliases(session, &aliases, bytes, bindings, &mutated_bindings)
    };
    recompute_aliases(session, &mut bindings)?;
    if record_object_assigns(assign_calls, bytes, &mut bindings) {
        recompute_aliases(session, &mut bindings)?;
    }
    for (node, runtime) in nodes {
        if !runtime {
            continue;
        }
        session.check()?;
        if node.kind() == "call_expression" || node.kind() == "new_expression" {
            let Some(function) = node
                .child_by_field_name("function")
                .or_else(|| node.child_by_field_name("constructor"))
            else {
                continue;
            };
            // A direct loader call before replacement still loads its dependency.
            // Resolve the callee when it is captured, before evaluating arguments;
            // cross-function mutation order remains unknown.
            let name = if unwrapped(function).is_some_and(|callee| {
                callee.kind() == "identifier" && text(callee, bytes) == "require"
            }) {
                let scope = function_scope(node);
                expression_at(
                    function,
                    bytes,
                    &bindings,
                    Some((node.start_byte(), scope.start_byte(), scope.end_byte())),
                )
            } else {
                expression(function, bytes, &bindings)
            };
            let args = node.child_by_field_name("arguments");
            let first = args.and_then(first_operand);
            match name.as_deref() {
                Some(api @ ("fetch" | "https.get")) if node.kind() == "call_expression" => {
                    if let Some(host) = args.and_then(|args| network_host(api, args, bytes)) {
                        finding(&mut facts, node, Behavior::Network(host));
                    } else {
                        gap(
                            &mut facts,
                            file,
                            Some(node),
                            "code_analysis_dynamic_target",
                            "network",
                        );
                    }
                }
                Some(
                    "Bun.spawn"
                    | "Bun.spawnSync"
                    | "child_process.spawn"
                    | "child_process.spawnSync"
                    | "child_process.execFile"
                    | "child_process.execFileSync"
                    | "child_process.fork",
                ) => {
                    let command = first.and_then(unwrapped).and_then(|n| {
                        if n.kind() == "array" {
                            first_operand(n).and_then(|n| literal(n, bytes))
                        } else {
                            literal(n, bytes)
                        }
                    });
                    let command = if name.as_deref() == Some("child_process.fork")
                        || (name
                            .as_deref()
                            .is_some_and(|n| n.starts_with("child_process."))
                            && args.is_some_and(|args| {
                                let mut cursor = args.walk();
                                args.named_children(&mut cursor)
                                    .filter(|n| !n.is_extra())
                                    .skip(1)
                                    .any(|n| {
                                        unwrapped(n).is_none_or(|n| {
                                            !matches!(
                                                n.kind(),
                                                "array" | "arrow_function" | "function_expression"
                                            )
                                        })
                                    })
                            })) {
                        // fork's first argument is a module, not an executable.
                        // Node options may select a shell or another execPath.
                        None
                    } else {
                        command
                    };
                    if let Some(command) = command {
                        finding(&mut facts, node, Behavior::Command(command));
                    } else {
                        gap(
                            &mut facts,
                            file,
                            Some(node),
                            "code_analysis_dynamic_target",
                            "command",
                        );
                    }
                    gap(
                        &mut facts,
                        file,
                        Some(node),
                        "code_analysis_subprocess_effects",
                        "command",
                    );
                }
                Some("require") => {
                    if let Some(spec) = first.and_then(|n| literal(n, bytes)) {
                        let p = node.start_position();
                        facts.imports.push((spec, p.row + 1, p.column + 1));
                    } else {
                        gap(
                            &mut facts,
                            file,
                            Some(node),
                            "code_analysis_dynamic_code",
                            "dependency",
                        );
                    }
                }
                Some(
                    "eval" | "import" | "child_process.exec" | "child_process.execSync" | "Bun.$",
                ) => {
                    gap(
                        &mut facts,
                        file,
                        Some(node),
                        "code_analysis_dynamic_code",
                        "code",
                    );
                    if name
                        .as_deref()
                        .is_some_and(|n| n.starts_with("child_process.") || n == "Bun.$")
                    {
                        gap(
                            &mut facts,
                            file,
                            Some(node),
                            "code_analysis_subprocess_effects",
                            "command",
                        );
                    }
                }
                Some(name) if file_access(name).is_some() => {
                    file_target(
                        &mut facts,
                        file,
                        node,
                        first.and_then(|n| literal(n, bytes)),
                        file_access(name).unwrap(),
                    );
                }
                Some("Bun.file") => {
                    // Charge file access at the method call. A stored or passed
                    // handle may have further uses outside the modeled methods.
                    if !outer_expression(node).parent().is_some_and(|parent| {
                        parent.kind() == "expression_statement"
                            || (bun_file_method(parent, bytes, &bindings).is_some()
                                && outer_expression(parent)
                                    .parent()
                                    .is_some_and(|call| call.kind() == "call_expression"))
                    }) {
                        gap(
                            &mut facts,
                            file,
                            Some(node),
                            "code_analysis_file_object",
                            "filesystem",
                        );
                    }
                }
                _ => {
                    if node.kind() == "call_expression"
                        && let Some(paths) = name
                            .as_deref()
                            .zip(args)
                            .and_then(|(name, args)| transfer_paths(name, args))
                    {
                        for (operand, access) in paths {
                            file_target(&mut facts, file, operand, literal(operand, bytes), access);
                        }
                    } else if node.kind() == "call_expression"
                        && let Some((access, path)) = bun_file_method(function, bytes, &bindings)
                    {
                        file_target(&mut facts, file, node, path, access);
                    } else {
                        gap(
                            &mut facts,
                            file,
                            Some(node),
                            "code_analysis_call_unknown",
                            "code",
                        );
                    }
                }
            }
        }
        if matches!(node.kind(), "member_expression" | "subscript_expression") {
            let Some(object) = node.child_by_field_name("object") else {
                continue;
            };
            if matches!(
                expression(object, bytes, &bindings).as_deref(),
                Some("process.env" | "Bun.env")
            ) && !write_only.contains(&node.id())
            {
                if let Some(name) = member_key(node, bytes) {
                    finding(&mut facts, node, Behavior::Environment(name));
                } else {
                    gap(
                        &mut facts,
                        file,
                        Some(node),
                        "code_analysis_dynamic_target",
                        "environment",
                    );
                }
            }
        }
        if wrapped_operand(node).is_none()
            && matches!(
                expression(node, bytes, &bindings).as_deref(),
                Some("process.env" | "Bun.env")
            )
            && !outer_expression(node).parent().is_some_and(|p| {
                matches!(p.kind(), "member_expression" | "subscript_expression")
                    && p.child_by_field_name("object") == Some(outer_expression(node))
            })
        {
            gap(
                &mut facts,
                file,
                Some(node),
                "code_analysis_environment_object",
                "environment",
            );
        }
        if node.kind() == "export_statement"
            && !type_only_module(node)
            && let Some(source) = node.child_by_field_name("source")
        {
            if let Some(source) = literal(source, bytes) {
                let p = node.start_position();
                facts.imports.push((source, p.row + 1, p.column + 1));
            } else {
                gap(
                    &mut facts,
                    file,
                    Some(node),
                    "code_analysis_dependency_unsupported",
                    "dependency",
                );
            }
        }
    }
    Ok(facts)
}

// Ordinary writes and loader replacements have already been resolved. Identify
// calls that can replace the global object or its assign helper first.
fn record_object_assigns(calls: Vec<Node<'_>>, bytes: &[u8], bindings: &mut Bindings) -> bool {
    let mut helper_writes = Vec::new();
    let mut object_writes = Vec::new();
    for node in calls {
        let scope = function_scope(node);
        let capture = (node.start_byte(), scope.start_byte(), scope.end_byte());
        let Some(args) = node.child_by_field_name("arguments") else {
            continue;
        };
        let Some(receiver) = first_operand(args) else {
            continue;
        };
        let Some(target) = expression_at(receiver, bytes, bindings, Some(capture)) else {
            continue;
        };
        let write = (node, args, receiver, capture);
        match target.as_str() {
            "Object" | "globalThis" => helper_writes.push(write),
            "Bun" | "process" | "fs" | "fs.promises" | "child_process" | "https" => {
                object_writes.push(write)
            }
            _ if bindings.bun_files.contains_key(&target) => object_writes.push(write),
            _ => {}
        }
    }
    // Visit earlier straight-line replacements before checking dependent calls.
    // Capture positions keep writes in other function scopes ambiguous.
    helper_writes.sort_unstable_by_key(|(node, ..)| node.end_byte());
    let mut changed = false;
    for (node, args, receiver, capture) in helper_writes.into_iter().chain(object_writes) {
        if node
            .child_by_field_name("function")
            .and_then(|f| expression_at(f, bytes, bindings, Some(capture)))
            .as_deref()
            != Some("Object.assign")
        {
            continue;
        }
        let Some(target) = expression_at(receiver, bytes, bindings, Some(capture)) else {
            continue;
        };
        let position = (node.end_byte(), capture.1, capture.2);
        let mut cursor = args.walk();
        for source in args
            .named_children(&mut cursor)
            .filter(|n| !n.is_extra())
            .skip(1)
        {
            let source = unwrapped(source).unwrap_or(source);
            let mut record = |key: Option<String>| {
                changed |= record_member_mutation(
                    &mut bindings.mutations,
                    &target,
                    key.as_deref(),
                    position,
                );
            };
            if source.kind() == "object" {
                let mut properties = source.walk();
                for property in source
                    .named_children(&mut properties)
                    .filter(|n| !n.is_extra())
                {
                    record(assign_key(property, bytes));
                }
            } else if !matches!(
                source.kind(),
                "null" | "true" | "false" | "number" | "string"
            ) {
                record(None);
            }
        }
    }
    changed
}

fn resolve_aliases(
    session: &mut Session,
    aliases: &[(Node<'_>, Node<'_>, Node<'_>)],
    bytes: &[u8],
    bindings: &mut Bindings,
    mutated_bindings: &BTreeSet<BindingIdentity>,
) -> Result<(), TerminalFailure> {
    // Bounded fixed point supports closures referring to later initialized consts;
    // cycles and chains beyond the bound retain unknown-call diagnostics.
    for _ in 0..64 {
        let mut changed = false;
        for &(node, name, value) in aliases {
            session.nodes += 1;
            session.budget()?;
            let scope = scope(node);
            if mutated_bindings.contains(&(
                text(name, bytes).into(),
                scope.start_byte(),
                scope.end_byte(),
            )) {
                continue;
            }
            let function = function_scope(node);
            let capture = Some((
                node.start_byte(),
                function.start_byte(),
                function.end_byte(),
            ));
            let value = unwrapped(value).unwrap_or(value);
            let value = if is_bun_file_call(value, bytes, bindings, capture) {
                // The prefix cannot be produced by a builtin module or global name.
                let handle = format!("#{}", value.start_byte());
                bindings
                    .bun_files
                    .entry(handle.clone())
                    .or_insert_with(|| bun_file_path(value, bytes));
                Some(handle)
            } else {
                expression_at(value, bytes, bindings, capture)
            };
            if let Some(binding) = bindings
                .named_mut(&text(name, bytes))
                .iter_mut()
                .take(128)
                .find(|b| b.start == scope.start_byte() && b.end == scope.end_byte())
                && binding.value != value
            {
                binding.value = value;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    Ok(())
}

fn descendants(node: Node<'_>) -> Vec<Node<'_>> {
    let mut result = Vec::new();
    let mut pending = vec![node];
    while let Some(n) = pending.pop() {
        result.push(n);
        let mut c = n.walk();
        pending.extend(n.named_children(&mut c));
    }
    result
}
fn pattern_names(node: Node<'_>) -> Vec<Node<'_>> {
    pattern_targets(node)
        .into_iter()
        .filter(|n| {
            matches!(
                n.kind(),
                "identifier" | "shorthand_property_identifier_pattern"
            )
        })
        .collect()
}

/// Visit only binding/write positions, never property keys, default-value
/// expressions or type annotations. Member targets remain whole for mutation analysis.
fn pattern_targets(node: Node<'_>) -> Vec<Node<'_>> {
    let mut targets = Vec::new();
    let mut pending = vec![node];
    while let Some(node) = pending.pop() {
        match node.kind() {
            "identifier"
            | "shorthand_property_identifier_pattern"
            | "member_expression"
            | "subscript_expression" => targets.push(node),
            "assignment_pattern" | "object_assignment_pattern" => {
                pending.extend(node.child_by_field_name("left"));
            }
            "pair_pattern" => pending.extend(node.child_by_field_name("value")),
            "required_parameter" | "optional_parameter" => {
                pending.extend(
                    node.child_by_field_name("pattern")
                        .or_else(|| node.child_by_field_name("name")),
                );
            }
            "formal_parameters" | "object_pattern" | "array_pattern" | "rest_pattern" => {
                let mut cursor = node.walk();
                pending.extend(node.named_children(&mut cursor));
            }
            _ => pending.extend(wrapped_operand(node)),
        }
    }
    targets
}
fn function_scope(mut node: Node<'_>) -> Node<'_> {
    while let Some(parent) = node.parent() {
        let position = node.start_byte();
        node = parent;
        if function_boundary(node, position) {
            break;
        }
    }
    node
}
fn function_boundary(node: Node<'_>, position: usize) -> bool {
    if node.kind() == "method_definition"
        && node
            .child_by_field_name("parameters")
            .is_some_and(|parameters| position < parameters.start_byte())
    {
        return false;
    }
    matches!(
        node.kind(),
        "function_declaration"
            | "generator_function_declaration"
            | "function_expression"
            | "arrow_function"
            | "generator_function"
            | "method_definition"
            | "class_static_block"
            | "program"
    )
}
fn function_binding_scope<'tree>(
    session: &mut Session,
    mut node: Node<'tree>,
    cache: &mut BTreeMap<usize, Node<'tree>>,
) -> Result<Node<'tree>, TerminalFailure> {
    // Namespace bodies contain their own var declarations, but execute inline.
    // Do not turn that binding boundary into a cross-function capture barrier.
    let mut visited = Vec::new();
    let scope = loop {
        session.nodes += 1;
        session.budget()?;
        if let Some(scope) = cache.get(&node.id()) {
            break *scope;
        }
        visited.push(node.id());
        let Some(parent) = node.parent() else {
            break node;
        };
        if function_boundary(parent, node.start_byte())
            || matches!(parent.kind(), "internal_module" | "module")
        {
            // Body declarations must not shadow default parameter initializers.
            break parent.child_by_field_name("body").unwrap_or(parent);
        }
        node = parent;
    };
    // Sibling declarations share ancestor results within this one parsed tree.
    for id in visited {
        cache.insert(id, scope);
    }
    Ok(scope)
}
fn scope(mut node: Node<'_>) -> Node<'_> {
    loop {
        if matches!(
            node.kind(),
            "statement_block"
                | "program"
                | "for_statement"
                | "for_in_statement"
                | "switch_body"
                | "catch_clause"
        ) {
            return node;
        }
        match node.parent() {
            Some(parent) => node = parent,
            None => return node,
        }
    }
}
fn type_only_module(node: Node<'_>) -> bool {
    if has_token(node, "type") {
        return true;
    }
    let nodes = descendants(node);
    if nodes.iter().any(|n| {
        n.kind() == "namespace_import"
            || (n.kind() == "identifier" && n.parent().is_some_and(|p| p.kind() == "import_clause"))
    }) {
        return false;
    }
    let specifiers = nodes
        .iter()
        .filter(|n| matches!(n.kind(), "import_specifier" | "export_specifier"))
        .collect::<Vec<_>>();
    !specifiers.is_empty() && specifiers.iter().all(|n| has_token(**n, "type"))
}

fn has_token(node: Node<'_>, kind: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor).any(|child| child.kind() == kind)
}

fn first_named_child(node: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| !child.is_extra())
}

/// Comments are named extras, not operands. Do not skip a leading array hole
/// or spread to find a later literal that is not the executable/first argument.
fn first_operand(node: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = node.walk();
    let first = node
        .children(&mut cursor)
        .find(|child| !child.is_extra() && !matches!(child.kind(), "(" | "["))?;
    first.is_named().then_some(first)
}

fn raw_text<'a>(node: Node<'_>, bytes: &'a [u8]) -> &'a str {
    std::str::from_utf8(&bytes[node.byte_range()]).unwrap_or("")
}

fn is_identifier(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "identifier"
            | "property_identifier"
            | "shorthand_property_identifier"
            | "shorthand_property_identifier_pattern"
    )
}

fn text<'a>(node: Node<'_>, bytes: &'a [u8]) -> Cow<'a, str> {
    let raw = raw_text(node, bytes);
    if is_identifier(node) {
        // Undecodable identifier escapes were rejected before binding collection.
        decode_js(raw, true).unwrap_or(Cow::Borrowed(raw))
    } else {
        Cow::Borrowed(raw)
    }
}

// Decode token contents, never source code. Legacy numeric escapes and unpaired
// UTF-16 surrogates remain unsupported. Identifier escapes must be Unicode scalars.
fn decode_js(raw: &str, identifier: bool) -> Option<Cow<'_, str>> {
    if !raw.contains('\\') {
        return Some(Cow::Borrowed(raw));
    }
    let mut units = Vec::new();
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        let value = if c != '\\' {
            u32::from(c)
        } else {
            let escape = chars.next()?;
            if identifier && escape != 'u' {
                return None;
            }
            match escape {
                'u' if chars.next_if_eq(&'{').is_some() => {
                    let mut value = chars.next()?.to_digit(16)?;
                    loop {
                        let digit = chars.next()?;
                        if digit == '}' {
                            break;
                        }
                        value = value.checked_mul(16)?.checked_add(digit.to_digit(16)?)?;
                    }
                    value
                }
                'u' | 'x' => {
                    let mut value = 0;
                    for _ in 0..if escape == 'u' { 4 } else { 2 } {
                        value = value * 16 + chars.next()?.to_digit(16)?;
                    }
                    value
                }
                '\n' | '\u{2028}' | '\u{2029}' => continue,
                '\r' => {
                    chars.next_if_eq(&'\n');
                    continue;
                }
                '0' if !chars.peek().is_some_and(char::is_ascii_digit) => 0,
                '0'..='9' => return None,
                'b' => 8,
                't' => 9,
                'n' => 10,
                'v' => 11,
                'f' => 12,
                'r' => 13,
                other => u32::from(other),
            }
        };
        if !identifier && (0xD800..=0xDFFF).contains(&value) {
            units.push(value as u16);
        } else {
            units.extend_from_slice(char::from_u32(value)?.encode_utf16(&mut [0; 2]));
        }
    }
    String::from_utf16(&units).ok().map(Cow::Owned)
}

fn decoded_string(node: Node<'_>, bytes: &[u8]) -> Option<String> {
    let node = unwrapped(node)?;
    if node.kind() != "string" {
        return None;
    }
    let raw = raw_text(node, bytes);
    let decoded = decode_js(raw.get(1..raw.len().checked_sub(1)?)?, false)?;
    (!decoded.chars().any(char::is_control)).then(|| decoded.into_owned())
}
fn literal(node: Node<'_>, bytes: &[u8]) -> Option<String> {
    let node = unwrapped(node)?;
    if node.kind() != "string" {
        return None;
    }
    let raw = text(node, bytes);
    let content = raw.get(1..raw.len().checked_sub(1)?)?;
    // Path, command and dependency literals retain their existing escape boundary.
    if content.contains('\\') || content.chars().any(char::is_control) {
        None
    } else {
        Some(content.to_string())
    }
}
fn assign_key(property: Node<'_>, bytes: &[u8]) -> Option<String> {
    let key = property
        .child_by_field_name("key")
        .or_else(|| property.child_by_field_name("name"))
        .unwrap_or(property);
    match key.kind() {
        "property_identifier" | "shorthand_property_identifier" | "number" => {
            let name = text(key, bytes);
            Some(name.into_owned())
        }
        "computed_property_name" => decoded_string(first_named_child(key)?, bytes),
        _ => decoded_string(key, bytes),
    }
}
fn member_key(node: Node<'_>, bytes: &[u8]) -> Option<String> {
    if let Some(property) = node.child_by_field_name("property") {
        Some(text(property, bytes).to_string())
    } else {
        node.child_by_field_name("index")
            .and_then(|n| decoded_string(n, bytes))
    }
}
// Pure type containers are excluded from both binding and effect collection.
// Assertion type operands are selected by wrapped_operand during the same walk.
fn is_type_syntax(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "type_query"
            | "type_annotation"
            | "type_predicate_annotation"
            | "asserts_annotation"
            | "type_alias_declaration"
            | "interface_declaration"
            | "type_parameters"
            | "type_arguments"
            | "implements_clause"
    )
}
// These wrappers preserve the runtime value/reference. Never visit their type
// names, or unwrap value-changing expressions such as await, comma or conditionals.
fn wrapped_operand(node: Node<'_>) -> Option<Node<'_>> {
    match node.kind() {
        "parenthesized_expression"
        | "non_null_expression"
        | "as_expression"
        | "satisfies_expression" => first_named_child(node),
        "instantiation_expression" => node
            .child_by_field_name("function")
            .or_else(|| first_named_child(node)),
        "type_assertion" => {
            let mut cursor = node.walk();
            node.named_children(&mut cursor)
                .find(|child| !child.is_extra() && child.kind() != "type_arguments")
        }
        _ => None,
    }
}
fn unwrapped(mut node: Node<'_>) -> Option<Node<'_>> {
    for _ in 0..64 {
        match wrapped_operand(node) {
            Some(operand) => node = operand,
            None => return Some(node),
        }
    }
    None
}
// Used only for effect-bearing operands, not for every enclosing wrapper node.
// The wrapper chain was already charged to the parser's node budget.
fn outer_expression(mut node: Node<'_>) -> Node<'_> {
    while let Some(parent) = node.parent() {
        if wrapped_operand(parent) != Some(node) {
            break;
        }
        node = parent;
    }
    node
}
fn expression(node: Node<'_>, bytes: &[u8], bindings: &Bindings) -> Option<String> {
    expression_at(node, bytes, bindings, None)
}
fn expression_at(
    node: Node<'_>,
    bytes: &[u8],
    bindings: &Bindings,
    capture: Option<(usize, usize, usize)>,
) -> Option<String> {
    // Bound recursion independently of hostile expression depth.
    let mut node = node;
    let mut keys = Vec::new();
    for _ in 0..64 {
        match node.kind() {
            "member_expression" | "subscript_expression" => {
                keys.push(member_key(node, bytes)?);
                node = node.child_by_field_name("object")?;
            }
            "call_expression" => {
                let callee = unwrapped(node.child_by_field_name("function")?)?;
                // The callee is a single identifier, so this adds no unbounded
                // expression recursion. Apply the same lexical/mutation checks
                // as the require call itself, including initializer capture time.
                if callee.kind() != "identifier"
                    || expression_at(callee, bytes, bindings, capture).as_deref() != Some("require")
                {
                    return None;
                }
                let spec = literal(
                    first_operand(node.child_by_field_name("arguments")?)?,
                    bytes,
                )?;
                let mut value = builtin(&spec)?.to_string();
                for key in keys.into_iter().rev() {
                    value = bindings.member(&value, &key, capture)?;
                }
                return Some(value);
            }
            "identifier" | "import" => {
                let name = text(node, bytes);
                let mut value = if let Some(binding) = bindings.visible(&name, node).ok()? {
                    binding.value.clone()?
                } else if name == "import" || BUILTIN_GLOBALS.contains(&name.as_ref()) {
                    if !bindings.unchanged(&name, capture) {
                        return None;
                    }
                    name.to_string()
                } else {
                    return None;
                };
                for key in keys.into_iter().rev() {
                    value = bindings.member(&value, &key, capture)?;
                }
                return Some(value);
            }
            _ => node = wrapped_operand(node)?,
        }
    }
    None
}
fn finding(facts: &mut Facts, node: Node<'_>, behavior: Behavior) {
    let p = node.start_position();
    facts.findings.push(Finding {
        behavior,
        line: p.row + 1,
        column: p.column + 1,
    });
}
fn gap(facts: &mut Facts, file: &str, node: Option<Node<'_>>, code: &str, behavior: &str) {
    let p = node.map(|n| n.start_position()).unwrap_or_default();
    facts.diagnostics.push(Diagnostic::new(
        DiagnosticKind::CoverageGap,
        code,
        file,
        p.row + 1,
        p.column + 1,
        behavior,
    ));
}
fn network_host(api: &str, args: Node<'_>, bytes: &[u8]) -> Option<String> {
    if args.kind() != "arguments" {
        return None;
    }
    let mut cursor = args.walk();
    let mut operands = args.named_children(&mut cursor).filter(|n| !n.is_extra());
    let url = url::Url::parse(&decoded_string(operands.next()?, bytes)?).ok()?;
    if api == "https.get" {
        // An options argument can override the URL's hostname or socket.
        // Only an unambiguous inline callback can follow the literal URL.
        if url.scheme() != "https"
            || operands.next().is_some_and(|n| {
                unwrapped(n).is_none_or(|n| {
                    !matches!(
                        n.kind(),
                        "arrow_function" | "function_expression" | "generator_function"
                    )
                })
            })
            || operands.next().is_some()
        {
            return None;
        }
    } else if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.host_str().map(str::to_ascii_lowercase)
}

fn transfer_paths<'tree>(
    name: &str,
    args: Node<'tree>,
) -> Option<[(Node<'tree>, FilesystemAccessV1); 2]> {
    use FilesystemAccessV1::{Read, Remove, Write};
    let (source_access, needs_callback) = match name {
        "fs.copyFile" => (Read, true),
        "fs.copyFileSync" | "fs.promises.copyFile" => (Read, false),
        "fs.rename" => (Remove, true),
        "fs.renameSync" | "fs.promises.rename" => (Remove, false),
        _ => return None,
    };
    if args.kind() != "arguments" {
        return None;
    }
    let mut cursor = args.walk();
    let mut operands = args.named_children(&mut cursor).filter(|n| !n.is_extra());
    let source = operands.next()?;
    // A leading spread can supply either path. Pin both unknowns to that
    // operand rather than treating a later argument as the destination.
    let destination = if source.kind() == "spread_element" {
        source
    } else {
        operands.next()?
    };
    if needs_callback && destination.kind() != "spread_element" && operands.next().is_none() {
        return None;
    }
    Some([(source, source_access), (destination, Write)])
}

fn file_target(
    facts: &mut Facts,
    file: &str,
    location: Node<'_>,
    path: Option<impl Into<Arc<str>>>,
    access: FilesystemAccessV1,
) {
    if let Some(path) = path {
        finding(facts, location, Behavior::File(access, path.into()));
    } else {
        gap(
            facts,
            file,
            Some(location),
            "code_analysis_dynamic_target",
            "filesystem",
        );
    }
}
fn file_access(name: &str) -> Option<FilesystemAccessV1> {
    match name {
        "fs.readFile" | "fs.readFileSync" | "fs.promises.readFile" | "fs.createReadStream" => {
            Some(FilesystemAccessV1::Read)
        }
        "fs.writeFile"
        | "fs.writeFileSync"
        | "fs.appendFile"
        | "fs.appendFileSync"
        | "fs.promises.writeFile"
        | "fs.promises.appendFile"
        | "fs.createWriteStream"
        | "Bun.write" => Some(FilesystemAccessV1::Write),
        "fs.unlink" | "fs.unlinkSync" | "fs.rm" | "fs.rmSync" | "fs.rmdir" | "fs.rmdirSync"
        | "fs.promises.unlink" | "fs.promises.rm" | "fs.promises.rmdir" => {
            Some(FilesystemAccessV1::Remove)
        }
        _ => None,
    }
}
fn is_bun_file_call(
    node: Node<'_>,
    bytes: &[u8],
    bindings: &Bindings,
    capture: Option<(usize, usize, usize)>,
) -> bool {
    node.kind() == "call_expression"
        && node
            .child_by_field_name("arguments")
            .is_some_and(|args| args.kind() == "arguments")
        && node
            .child_by_field_name("function")
            .and_then(|f| expression_at(f, bytes, bindings, capture))
            .as_deref()
            == Some("Bun.file")
}

fn bun_file_path(call: Node<'_>, bytes: &[u8]) -> Option<Arc<str>> {
    literal(
        first_operand(call.child_by_field_name("arguments")?)?,
        bytes,
    )
    .map(Arc::from)
}

fn bun_file_method(
    node: Node<'_>,
    bytes: &[u8],
    bindings: &Bindings,
) -> Option<(FilesystemAccessV1, Option<Arc<str>>)> {
    let node = unwrapped(node)?;
    let key = member_key(node, bytes)?;
    let access = match key.as_str() {
        "text" | "json" | "arrayBuffer" | "bytes" | "stream" | "exists" => FilesystemAccessV1::Read,
        "delete" => FilesystemAccessV1::Remove,
        "writer" => FilesystemAccessV1::Write,
        _ => return None,
    };
    let receiver = unwrapped(node.child_by_field_name("object")?)?;
    let path = if is_bun_file_call(receiver, bytes, bindings, None) {
        bun_file_path(receiver, bytes)
    } else {
        let handle = expression(receiver, bytes, bindings)?;
        let path = bindings.bun_files.get(&handle)?;
        bindings.member(&handle, &key, None)?;
        path.clone()
    };
    Some((access, path))
}
