use super::*;
use tree_sitter::{Node, ParseOptions, Parser};

pub(super) fn builtin(spec: &str) -> Option<&str> {
    match spec.strip_prefix("node:").unwrap_or(spec) {
        "fs" => Some("fs"),
        "fs/promises" => Some("fs"),
        "child_process" => Some("child_process"),
        "process" => Some("process"),
        "bun" => Some("Bun"),
        _ => None,
    }
}

struct Binding {
    name: String,
    start: usize,
    end: usize,
    value: Option<String>,
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

#[derive(Default)]
struct Bindings {
    names: BTreeMap<String, Vec<Binding>>,
    mutations: BTreeMap<String, Mutation>,
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
    let mut pending = vec![tree.root_node()];
    while let Some(node) = pending.pop() {
        session.nodes += 1;
        session.budget()?;
        if node.is_error() || node.is_missing() {
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
        if node.is_named() {
            nodes.push(node);
        }
        let mut cursor = node.walk();
        // Missing delimiters are anonymous nodes too; never accept a recovered tree.
        pending.extend(node.children(&mut cursor));
    }
    let module = matches!(extension, "mjs" | "mts")
        || nodes
            .iter()
            .any(|n| matches!(n.kind(), "import_statement" | "export_statement"));
    let mut bindings = Bindings::default();
    for node in &nodes {
        match node.kind() {
            "import_statement" => {
                if type_only_module(*node) {
                    continue;
                }
                let Some(source) = node
                    .child_by_field_name("source")
                    .and_then(|n| literal(n, bytes))
                else {
                    gap(
                        &mut facts,
                        file,
                        Some(*node),
                        "code_analysis_dependency_unsupported",
                        "dependency",
                    );
                    continue;
                };
                let p = node.start_position();
                facts
                    .imports
                    .push((source.clone(), p.row + 1, p.column + 1));
                let module = builtin(&source);
                for child in descendants(*node) {
                    let binding = match child.kind() {
                        "import_specifier" if !has_token(child, "type") => {
                            let name = child.child_by_field_name("name");
                            let alias = child.child_by_field_name("alias").or(name);
                            alias.map(|alias| {
                                (
                                    text(alias, bytes),
                                    module
                                        .zip(name)
                                        .map(|(m, n)| format!("{m}.{}", text(n, bytes))),
                                )
                            })
                        }
                        "namespace_import" => child
                            .named_child(0)
                            .map(|n| (text(n, bytes), module.map(str::to_string))),
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
                            start: 0,
                            end: bytes.len(),
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
                        function_scope(*node)
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
            "function_declaration" | "generator_function_declaration" | "class_declaration" => {
                if let Some(name) = node.child_by_field_name("name") {
                    let mut scope = scope(node.parent().unwrap_or(*node));
                    if node.kind() == "function_declaration" && !module && scope.kind() != "program"
                    {
                        // Sloppy-script block functions can be hoisted (Annex B).
                        // Without an explicit module boundary, do not claim the
                        // outer spelling still denotes an unshadowed global.
                        scope = function_scope(*node);
                    }
                    bindings.push(Binding {
                        name: text(name, bytes).into(),
                        start: scope.start_byte(),
                        end: scope.end_byte(),
                        value: None,
                    });
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
                        start: parent.start_byte(),
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
                if let Some(left) = node.child_by_field_name("left") {
                    let scope = if node
                        .child_by_field_name("kind")
                        .is_some_and(|n| n.kind() == "var")
                    {
                        function_scope(*node)
                    } else if node.child_by_field_name("kind").is_none() {
                        tree.root_node()
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
            "with_statement" => {
                for name in ["Bun", "process", "fetch", "eval", "require"] {
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
        .filter_map(|node| {
            if node.kind() != "variable_declarator"
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
    let mut mutated_names = BTreeSet::new();
    for node in &nodes {
        if matches!(
            node.kind(),
            "assignment_expression" | "augmented_assignment_expression" | "update_expression"
        ) || (node.kind() == "unary_expression" && has_token(*node, "delete"))
        {
            let target = node
                .child_by_field_name("left")
                .or_else(|| node.child_by_field_name("argument"));
            if let Some(target) = target {
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
                    node.start_byte(),
                    location.start_byte(),
                    location.end_byte(),
                );
                if let Some(object_node) = target.child_by_field_name("object") {
                    if text(object_node, bytes) == "globalThis" {
                        if let Some(key) = member_key(target, bytes) {
                            record_mutation(&mut mutations, key, position);
                        } else {
                            for name in ["Bun", "process", "fetch", "eval", "require"] {
                                record_mutation(&mut mutations, name.into(), position);
                            }
                        }
                    } else if let Some(object) = expression(object_node, bytes, &bindings) {
                        let key = member_key(target, bytes).unwrap_or_else(|| "*".into());
                        record_mutation(&mut mutations, format!("{object}.{key}"), position);
                    }
                } else {
                    let name = text(target, bytes);
                    if bindings.named(name).is_empty()
                        && matches!(name, "Bun" | "process" | "fetch" | "eval" | "require")
                    {
                        record_mutation(&mut mutations, name.into(), position);
                    } else {
                        mutated_names.insert(name.to_string());
                    }
                }
            }
        }
    }
    for name in &mutated_names {
        for binding in bindings.named_mut(name) {
            binding.value = None;
        }
    }
    bindings.mutations = mutations;
    // Recompute captured values with member mutations applied at each initializer.
    // Copied function values do not re-read the object's subsequently changed slot.
    for (node, name, _) in &aliases {
        let scope = scope(*node);
        for binding in bindings.named_mut(text(*name, bytes)) {
            if binding.start == scope.start_byte() && binding.end == scope.end_byte() {
                binding.value = None;
            }
        }
    }
    resolve_aliases(session, &aliases, bytes, &mut bindings, &mutated_names)?;
    for node in nodes {
        session.check()?;
        if node.kind() == "call_expression" || node.kind() == "new_expression" {
            let Some(function) = node
                .child_by_field_name("function")
                .or_else(|| node.child_by_field_name("constructor"))
            else {
                continue;
            };
            let name = expression(function, bytes, &bindings);
            let args = node.child_by_field_name("arguments");
            let first = args.and_then(|n| n.named_child(0));
            match name.as_deref() {
                Some("fetch") => {
                    if let Some(host) = first.and_then(|n| literal(n, bytes)).and_then(|s| host(&s))
                    {
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
                    let command = first.and_then(|n| {
                        if n.kind() == "array" {
                            n.named_child(0).and_then(|n| literal(n, bytes))
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
                                args.named_children(&mut cursor).skip(1).any(|n| {
                                    !matches!(
                                        n.kind(),
                                        "array" | "arrow_function" | "function_expression"
                                    )
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
                    if let Some(path) = first.and_then(|n| literal(n, bytes)) {
                        finding(
                            &mut facts,
                            node,
                            Behavior::File(file_access(name).unwrap(), path),
                        );
                    } else {
                        gap(
                            &mut facts,
                            file,
                            Some(node),
                            "code_analysis_dynamic_target",
                            "filesystem",
                        );
                    }
                }
                Some("Bun.file") => {
                    // Constructing a BunFile does not read it. Its supported methods
                    // are handled by the parent call below.
                    if !node.parent().is_some_and(|parent| {
                        parent.kind() == "expression_statement"
                            || (bun_file_method(parent, bytes, &bindings).is_some()
                                && parent
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
                    if let Some((access, path)) = bun_file_method(function, bytes, &bindings) {
                        if let Some(path) = path {
                            finding(&mut facts, node, Behavior::File(access, path));
                        } else {
                            gap(
                                &mut facts,
                                file,
                                Some(node),
                                "code_analysis_dynamic_target",
                                "filesystem",
                            );
                        }
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
            ) && is_read(node)
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
        if matches!(
            expression(node, bytes, &bindings).as_deref(),
            Some("process.env" | "Bun.env")
        ) && !node.parent().is_some_and(|p| {
            matches!(p.kind(), "member_expression" | "subscript_expression")
                && p.child_by_field_name("object") == Some(node)
        }) {
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

fn resolve_aliases(
    session: &mut Session,
    aliases: &[(Node<'_>, Node<'_>, Node<'_>)],
    bytes: &[u8],
    bindings: &mut Bindings,
    mutated_names: &BTreeSet<String>,
) -> Result<(), TerminalFailure> {
    // Bounded fixed point supports closures referring to later initialized consts;
    // cycles and chains beyond the bound retain unknown-call diagnostics.
    for _ in 0..64 {
        let mut changed = false;
        for &(node, name, value) in aliases {
            session.nodes += 1;
            session.budget()?;
            if mutated_names.contains(text(name, bytes)) {
                continue;
            }
            let function = function_scope(node);
            let capture = Some((
                node.start_byte(),
                function.start_byte(),
                function.end_byte(),
            ));
            let value = expression_at(value, bytes, bindings, capture);
            let scope = scope(node);
            if let Some(binding) = bindings
                .named_mut(text(name, bytes))
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
    descendants(node)
        .into_iter()
        .filter(|n| {
            matches!(
                n.kind(),
                "identifier" | "shorthand_property_identifier_pattern"
            ) && n.parent().is_none_or(|p| p.kind() != "type_annotation")
        })
        .collect()
}
fn function_scope(mut node: Node<'_>) -> Node<'_> {
    while let Some(parent) = node.parent() {
        node = parent;
        if matches!(
            node.kind(),
            "function_declaration"
                | "generator_function_declaration"
                | "function_expression"
                | "arrow_function"
                | "generator_function"
                | "method_definition"
                | "program"
        ) {
            break;
        }
    }
    node
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

fn text<'a>(node: Node<'_>, bytes: &'a [u8]) -> &'a str {
    std::str::from_utf8(&bytes[node.byte_range()]).unwrap_or("")
}
fn literal(node: Node<'_>, bytes: &[u8]) -> Option<String> {
    if node.kind() != "string" {
        return None;
    }
    let raw = text(node, bytes);
    let content = raw.get(1..raw.len().checked_sub(1)?)?;
    // Escaped literals are deliberately unknown until a complete JS decoder is used.
    if content.contains('\\') || content.chars().any(char::is_control) {
        None
    } else {
        Some(content.to_string())
    }
}
fn member_key(node: Node<'_>, bytes: &[u8]) -> Option<String> {
    if let Some(property) = node.child_by_field_name("property") {
        Some(text(property, bytes).to_string())
    } else {
        node.child_by_field_name("index")
            .and_then(|n| literal(n, bytes))
    }
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
                let callee = node.child_by_field_name("function")?;
                if text(callee, bytes) != "require" || !bindings.named("require").is_empty() {
                    return None;
                }
                let spec = literal(
                    node.child_by_field_name("arguments")?.named_child(0)?,
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
                if bindings.named(name).len() > 128 {
                    return None;
                }
                let candidates = bindings
                    .named(name)
                    .iter()
                    .filter(|b| {
                        b.name == name && b.start <= node.start_byte() && node.end_byte() <= b.end
                    })
                    .collect::<Vec<_>>();
                let mut value = if let Some(min) = candidates.iter().map(|b| b.end - b.start).min()
                {
                    let closest = candidates
                        .into_iter()
                        .filter(|b| b.end - b.start == min)
                        .collect::<Vec<_>>();
                    if closest.len() != 1 {
                        return None;
                    }
                    closest[0].value.clone()?
                } else if matches!(
                    name,
                    "process" | "Bun" | "fetch" | "eval" | "require" | "import"
                ) {
                    if !bindings.unchanged(name, capture) {
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
            _ => return None,
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
fn is_read(node: Node<'_>) -> bool {
    !node.parent().is_some_and(|p| {
        (p.kind() == "assignment_expression" && p.child_by_field_name("left") == Some(node))
            || (p.kind() == "unary_expression" && has_token(p, "delete"))
    })
}
fn host(value: &str) -> Option<String> {
    let url = url::Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.host_str().map(str::to_ascii_lowercase)
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
        | "fs.promises.unlink" | "fs.promises.rm" => Some(FilesystemAccessV1::Remove),
        _ => None,
    }
}
fn bun_file_method(
    node: Node<'_>,
    bytes: &[u8],
    bindings: &Bindings,
) -> Option<(FilesystemAccessV1, Option<String>)> {
    let key = member_key(node, bytes)?;
    let access = match key.as_str() {
        "text" | "json" | "arrayBuffer" | "bytes" | "stream" | "exists" => FilesystemAccessV1::Read,
        "delete" => FilesystemAccessV1::Remove,
        "writer" => FilesystemAccessV1::Write,
        _ => return None,
    };
    let call = node.child_by_field_name("object")?;
    if call.kind() != "call_expression"
        || expression(call.child_by_field_name("function")?, bytes, bindings).as_deref()
            != Some("Bun.file")
    {
        return None;
    }
    let path = call
        .child_by_field_name("arguments")
        .and_then(|n| n.named_child(0))
        .and_then(|n| literal(n, bytes));
    Some((access, path))
}
