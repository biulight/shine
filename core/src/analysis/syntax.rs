use super::*;
use std::borrow::Cow;
use std::ops::ControlFlow;
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
type Capture = (usize, usize, usize);

impl Binding {
    fn identity(&self) -> BindingIdentity {
        (self.name.clone(), self.start, self.end)
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Mutation {
    first: usize,
    scope: Option<(usize, usize)>,
}

// Positions are ordered within an execution scope. A possible restoration in a
// different function has unknown order with respect to this capture.
type ScopedWrites = BTreeMap<String, BTreeMap<(usize, usize), BTreeSet<usize>>>;

fn record_position(sites: &mut ScopedWrites, path: String, (at, start, end): Capture) {
    sites
        .entry(path)
        .or_default()
        .entry((start, end))
        .or_default()
        .insert(at);
}

#[derive(Clone, PartialEq)]
enum MutationIndex {
    Possible(BTreeMap<String, Mutation>, ScopedWrites),
    Guaranteed {
        replacements: ScopedWrites,
        restorations: ScopedWrites,
    },
}
impl Default for MutationIndex {
    fn default() -> Self {
        Self::Possible(BTreeMap::new(), BTreeMap::new())
    }
}
impl MutationIndex {
    fn guaranteed() -> Self {
        Self::Guaranteed {
            replacements: BTreeMap::new(),
            restorations: BTreeMap::new(),
        }
    }
    fn record(&mut self, path: String, position: Capture, distinct: bool) {
        let (at, start, end) = position;
        match self {
            Self::Possible(writes, restorations) => {
                if !distinct {
                    record_position(restorations, path.clone(), position);
                }
                writes
                    .entry(path)
                    .and_modify(|mutation| {
                        mutation.first = mutation.first.min(at);
                        if mutation.scope != Some((start, end)) {
                            mutation.scope = None;
                        }
                    })
                    .or_insert(Mutation {
                        first: at,
                        scope: Some((start, end)),
                    });
            }
            Self::Guaranteed { replacements, .. } => {
                if distinct {
                    record_position(replacements, path, position);
                }
            }
        }
    }
    fn unchanged(&self, path: &str, capture: Option<Capture>) -> bool {
        match self {
            Self::Possible(writes, _) => writes.get(path).is_none_or(|mutation| {
                capture.is_some_and(|(at, start, end)| {
                    at < mutation.first && mutation.scope == Some((start, end))
                })
            }),
            Self::Guaranteed {
                replacements,
                restorations,
            } => {
                let Some((at, start, end)) = capture else {
                    return true;
                };
                let scope = (start, end);
                let Some(&replaced) = replacements
                    .get(path)
                    .and_then(|s| s.get(&scope))
                    .and_then(|sites| sites.range(..=at).next_back())
                else {
                    return true;
                };
                let may_restore = |key: &str| {
                    restorations.get(key).is_some_and(|scopes| {
                        scopes.len() != 1
                            || scopes
                                .get(&scope)
                                .is_none_or(|sites| sites.range(replaced..=at).next().is_some())
                    })
                };
                may_restore(path)
                    || path
                        .rsplit_once('.')
                        .is_some_and(|(parent, _)| may_restore(&format!("{parent}.*")))
            }
        }
    }
    fn is_empty(&self) -> bool {
        match self {
            Self::Possible(writes, _) => writes.is_empty(),
            Self::Guaranteed { replacements, .. } => replacements.is_empty(),
        }
    }
}

// A write through the global object replaces the slot used by the bare name.
fn record_member_mutation(
    mutations: &mut MutationIndex,
    object: &str,
    key: Option<&str>,
    position: Capture,
    distinct: bool,
) {
    if key.is_some_and(|k| k.contains(['.', '*'])) {
        return;
    }
    match (object, key) {
        ("globalThis", None) => {
            for name in BUILTIN_GLOBALS {
                mutations.record(name.into(), position, distinct);
            }
        }
        ("globalThis", Some(key)) if BUILTIN_GLOBALS.contains(&key) => {
            mutations.record(key.into(), position, distinct)
        }
        ("globalThis", _) => {}
        _ => mutations.record(
            format!("{object}.{}", key.unwrap_or("*")),
            position,
            distinct,
        ),
    }
}

// Source ranges still own diagnostics and visibility. Only the mutation clock
// moves an input ahead of the pattern which consumes it. Preserve subtree sizes
// so sibling positions, including nested patterns, remain directly comparable.
#[derive(Default)]
struct EvaluationOrder {
    shifted: BTreeMap<usize, usize>,
}
impl EvaluationOrder {
    fn start(&self, node: Node<'_>) -> usize {
        self.shifted
            .get(&node.id())
            .copied()
            .unwrap_or(node.start_byte())
    }
    fn end(&self, node: Node<'_>) -> usize {
        self.start(node) + node.end_byte() - node.start_byte()
    }
}

fn input_before_pattern(node: Node<'_>) -> Option<(Node<'_>, Node<'_>)> {
    let (left, right) = match node.kind() {
        "assignment_expression" | "assignment_pattern" | "for_in_statement" => ("left", "right"),
        "variable_declarator" => ("name", "value"),
        _ => return None,
    };
    let left = node.child_by_field_name(left)?;
    if node.kind() != "for_in_statement"
        && !unwrapped(left).is_some_and(|n| matches!(n.kind(), "array_pattern" | "object_pattern"))
    {
        return None;
    }
    Some((left, node.child_by_field_name(right)?))
}

#[derive(Default)]
struct Bindings {
    order: EvaluationOrder,
    names: BTreeMap<String, Vec<Binding>>,
    mutations: MutationIndex,
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

    fn member(&self, object: &str, key: &str, capture: Option<Capture>) -> Option<String> {
        if object == "globalThis" {
            return (key == "fetch" && self.mutations.unchanged("fetch", capture))
                .then(|| key.to_owned());
        }
        // Dots separate components and '*' denotes a dynamic write in this index.
        if key.contains(['.', '*']) {
            return None;
        }
        let member = format!("{object}.{key}");
        (self.mutations.unchanged(&member, capture)
            && self.mutations.unchanged(&format!("{object}.*"), capture))
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
            match session.budget() {
                Ok(()) => ControlFlow::Continue(()),
                Err(_) => ControlFlow::Break(()),
            }
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
    let mut pattern_values = BTreeMap::new();
    let mut skipped_defaults = BTreeSet::new();
    let mut namespace_names = NamespaceNames::default();
    let mut bindings = Bindings::default();
    let mut pending = vec![(
        tree.root_node(),
        false,
        false,
        tree.root_node().start_byte(),
    )];
    while let Some((node, syntax_only, ambient, at)) = pending.pop() {
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
        // Still validate syntax and charge the budget inside erased types and
        // unevaluated defaults. Ambient declarations retain unknown bindings.
        let syntax_only = syntax_only || skipped_defaults.remove(&node.id());
        if !syntax_only
            && matches!(
                node.kind(),
                "type_alias_declaration" | "interface_declaration"
            )
            && let Some(name) = node.child_by_field_name("name")
        {
            namespace_names.add(text(name, bytes).into_owned(), scope(node), None);
        }
        let syntax_only = syntax_only || is_type_syntax(node);
        let ambient = ambient
            || node.kind() == "ambient_declaration"
            || (matches!(node.kind(), "internal_module" | "module")
                && node
                    .child_by_field_name("name")
                    .is_some_and(|n| n.kind() == "string"));
        if node.is_named() && !syntax_only {
            nodes.push((node, !ambient));
            if at != node.start_byte() {
                bindings.order.shifted.insert(node.id(), at);
            }
        }
        let reordered = input_before_pattern(node);
        if let Some((left, right)) = reordered
            && node.kind() == "assignment_expression"
            && !syntax_only
            && !ambient
            && !node.has_error()
        {
            collect_pattern_inputs(
                session,
                left,
                right,
                bytes,
                &mut pattern_values,
                &mut skipped_defaults,
            )?;
        }
        let operand = wrapped_operand(node);
        let mut cursor = node.walk();
        // Missing delimiters are anonymous nodes too; never accept a recovered tree.
        pending.extend(node.children(&mut cursor).map(|child| {
            let start = match reordered {
                Some((left, right)) if child == left => {
                    right.end_byte() - (left.end_byte() - left.start_byte())
                }
                Some((left, right)) if child == right => left.start_byte(),
                _ => child.start_byte(),
            };
            (
                child,
                syntax_only
                    || operand.is_some_and(|value| {
                        child.is_named() && !child.is_extra() && child != value
                    }),
                ambient,
                at + (start - node.start_byte()),
            )
        }));
    }
    let module = matches!(extension, "mjs" | "mts")
        || nodes
            .iter()
            .any(|(n, _)| matches!(n.kind(), "import_statement" | "export_statement"));
    let mut namespaces = Vec::new();
    let mut binding_scopes = BTreeMap::new();
    let mut declared_functions = BTreeSet::new();
    for (node, runtime) in &nodes {
        match node.kind() {
            "import_statement" => {
                let type_import = type_only_module(*node);
                let source = node
                    .child_by_field_name("source")
                    .and_then(|n| literal(n, bytes));
                if let Some(source) = &source
                    && *runtime
                    && !type_import
                {
                    let p = node.start_position();
                    facts
                        .imports
                        .push((source.clone(), p.row + 1, p.column + 1));
                } else if *runtime && !type_import {
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
                        "import_specifier" => {
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
                        if type_import
                            || (child.kind() == "import_specifier" && has_token(child, "type"))
                        {
                            namespace_names.add(name.into_owned(), location, None);
                            continue;
                        }
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
            | "abstract_class_declaration"
            | "function_signature"
            | "enum_declaration" => {
                if let Some(name) = node.child_by_field_name("name") {
                    let lexical_scope = scope(node.parent().unwrap_or(*node));
                    let mut scope = lexical_scope;
                    if node.kind() == "function_declaration" && !module && scope.kind() != "program"
                    {
                        // Sloppy-script block functions can be hoisted (Annex B).
                        // Without an explicit module boundary, do not claim the
                        // outer spelling still denotes an unshadowed global.
                        scope = function_binding_scope(session, *node, &mut binding_scopes)?;
                    }
                    if *runtime
                        && matches!(
                            node.kind(),
                            "function_declaration" | "generator_function_declaration"
                        )
                        && scope == lexical_scope
                    {
                        declared_functions.insert((
                            text(name, bytes).into_owned(),
                            scope.start_byte(),
                            scope.end_byte(),
                        ));
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
                namespaces.push((*node, *runtime));
                if let Some(name) = node.child_by_field_name("name") {
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
                        namespace_names.add(
                            text(id, bytes).into_owned(),
                            location,
                            Some(node.id()),
                        );
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
    namespace_names.bind(&namespaces, bytes, &mut bindings, session)?;
    let mut execution_scopes = ExecutionScopes::new(session, &nodes, bytes)?;
    let mut aliases = Vec::new();
    for (node, runtime) in nodes.iter().rev() {
        if *runtime
            && node.kind() == "variable_declarator"
            && node.parent().is_some_and(|p| has_token(p, "const"))
            && let Some(name) = node
                .child_by_field_name("name")
                .filter(|n| n.kind() == "identifier")
            && let Some(value) = node.child_by_field_name("value")
        {
            let location = scope(*node);
            aliases.push(Alias {
                binding: (
                    text(name, bytes).into_owned(),
                    location.start_byte(),
                    location.end_byte(),
                ),
                value: unwrapped(value).unwrap_or(value),
                capture: execution_scopes
                    .get(session, *node)?
                    .capture(bindings.order.start(*node)),
            });
        }
    }
    let mut aliases: Vec<_> = aliases.iter().collect();
    resolve_aliases(session, &aliases, bytes, &mut bindings)?;
    // With no writes applied, every recognizable assign callee is a candidate.
    // Resolve its actual identity and receiver under the same writes as ordinary stores.
    let mut mutated_bindings = BTreeSet::new();
    let mut member_writes = Vec::new();
    let mut write_only = BTreeSet::new();
    for (node, runtime) in &nodes {
        if !runtime {
            continue;
        }
        if node.kind() == "call_expression"
            && let Some(function) = node.child_by_field_name("function")
            && expression(function, bytes, &bindings).as_deref() == Some("Object.assign")
            && let Some(args) = node.child_by_field_name("arguments")
            && let Some(receiver) = first_operand(args)
        {
            let scope = execution_scopes.get(session, *node)?;
            member_writes.push(MemberWrite {
                receiver: Some(receiver),
                assign: Some(function),
                unconditional: scope.straight_line && !scope.protected,
                keys: assign_keys(args, bytes),
                capture: scope.capture(bindings.order.start(*node)),
                effect: bindings.order.end(*node),
            });
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
                // Computed keys and default RHS expressions remain reads.
                if matches!(
                    node.kind(),
                    "assignment_expression" | "for_in_statement" | "unary_expression"
                ) {
                    write_only.insert(target.id());
                }
                let scope = execution_scopes.get(session, *node)?;
                let capture = scope.capture(bindings.order.start(target));
                let rhs = match node.kind() {
                    "assignment_expression"
                        if node.child_by_field_name("left").and_then(unwrapped) != Some(target) =>
                    {
                        pattern_values.get(&target.id()).copied()
                    }
                    "assignment_expression" | "augmented_assignment_expression" => {
                        node.child_by_field_name("right")
                    }
                    _ => None,
                };
                let outer = outer_expression(target);
                let rest_target = outer.parent().is_some_and(|p| p.kind() == "rest_pattern");
                let distinct = rest_target
                    || matches!(node.kind(), "update_expression" | "unary_expression")
                    || (node.kind() == "augmented_assignment_expression"
                        && !short_circuit_assignment(*node));
                let unconditional = scope.straight_line
                    && !scope.protected
                    && node.kind() != "for_in_statement"
                    && !short_circuit_assignment(*node);
                let effect = if input_before_pattern(*node).is_some() {
                    // A simple reference is evaluated before its default, but
                    // the store completes after the default has been evaluated.
                    let store = outer
                        .parent()
                        .filter(|p| {
                            matches!(p.kind(), "assignment_pattern" | "object_assignment_pattern")
                                && p.child_by_field_name("left") == Some(outer)
                        })
                        .unwrap_or(outer);
                    bindings.order.end(store)
                } else {
                    bindings.order.end(*node)
                };
                let receiver = target.child_by_field_name("object");
                let key = if receiver.is_some() {
                    member_key(target, bytes)
                } else {
                    let name = text(target, bytes);
                    match bindings.visible(&name, target) {
                        Ok(Some(binding)) => {
                            mutated_bindings.insert(binding.identity());
                            continue;
                        }
                        Ok(None) if BUILTIN_GLOBALS.contains(&name.as_ref()) => {
                            Some(name.into_owned())
                        }
                        _ => continue,
                    }
                };
                member_writes.push(MemberWrite {
                    receiver,
                    assign: None,
                    unconditional,
                    keys: vec![WriteKey {
                        name: key,
                        value: rhs,
                        distinct,
                    }],
                    capture,
                    effect,
                });
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
    aliases.retain(|alias| !mutated_bindings.contains(&alias.binding));
    if !member_writes.is_empty() || !mutated_bindings.is_empty() {
        declared_functions.retain(|binding| !mutated_bindings.contains(binding));
        resolve_writes(
            session,
            &mut member_writes,
            &aliases,
            bytes,
            &mut bindings,
            declared_functions,
        )?;
        resolve_aliases(session, &aliases, bytes, &mut bindings)?;
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
            let capture = if bindings.mutations.is_empty() {
                None
            } else {
                let scope = execution_scopes.get(session, node)?;
                let ordered_loader = scope.ordered_pattern
                    && unwrapped(function).is_some_and(|callee| {
                        callee.kind() == "identifier" && text(callee, bytes) == "require"
                    });
                (scope.straight_line || ordered_loader)
                    .then(|| scope.capture(bindings.order.start(node)))
            };
            let name = expression_at(function, bytes, &bindings, capture);
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
                            || (bun_file_method(parent, bytes, &bindings, capture).is_some()
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
                        && let Some((access, path)) =
                            bun_file_method(function, bytes, &bindings, capture)
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

struct Alias<'tree> {
    binding: BindingIdentity,
    value: Node<'tree>,
    capture: Capture,
}

struct WriteKey<'tree> {
    value: Option<Node<'tree>>,
    name: Option<String>,
    distinct: bool,
}

// Only new objects/functions and primitives are provably different from an
// existing builtin. Immutable aliases of these values carry the same proof.
fn distinct_literal(value: Node<'_>) -> bool {
    matches!(
        value.kind(),
        "object"
            | "array"
            | "arrow_function"
            | "function_expression"
            | "generator_function"
            | "class"
            | "number"
            | "string"
            | "template_string"
            | "regex"
            | "true"
            | "false"
            | "null"
    )
}

fn distinct_const_bindings(
    session: &mut Session,
    aliases: &[&Alias<'_>],
    bytes: &[u8],
    bindings: &Bindings,
    mut fresh: BTreeSet<BindingIdentity>,
) -> Result<BTreeSet<BindingIdentity>, TerminalFailure> {
    let mut dependents = BTreeMap::<BindingIdentity, Vec<_>>::new();
    let mut pending = Vec::new();
    for alias in aliases {
        session.nodes += 1;
        session.budget()?;
        if distinct_literal(alias.value) {
            pending.push(&alias.binding);
        } else if alias.value.kind() == "identifier"
            && let Ok(Some(parent)) = bindings.visible(&text(alias.value, bytes), alias.value)
        {
            dependents
                .entry(parent.identity())
                .or_default()
                .push(&alias.binding);
        }
    }
    for declared in &fresh {
        pending.extend(dependents.remove(declared).into_iter().flatten());
    }
    while let Some(binding) = pending.pop() {
        session.nodes += 1;
        session.budget()?;
        if fresh.insert(binding.clone()) {
            pending.extend(dependents.remove(binding).into_iter().flatten());
        }
    }
    Ok(fresh)
}

fn short_circuit_assignment(node: Node<'_>) -> bool {
    node.child_by_field_name("operator")
        .is_some_and(|op| matches!(op.kind(), "&&=" | "||=" | "??="))
}

enum PatternKey {
    Known(String),
    Numeric(Option<f64>),
    Unknown,
}

fn pattern_property_key(property: Node<'_>, bytes: &[u8]) -> PatternKey {
    let key = property
        .child_by_field_name("key")
        .or_else(|| property.child_by_field_name("name"))
        .unwrap_or(property);
    let mut operand = if key.kind() == "computed_property_name" {
        first_named_child(key).and_then(unwrapped)
    } else {
        Some(key)
    };
    let mut negative = false;
    if let Some(unary) = operand.filter(|n| n.kind() == "unary_expression")
        && let Some(op) = unary.child_by_field_name("operator")
        && matches!(op.kind(), "+" | "-")
    {
        negative = op.kind() == "-";
        operand = unary.child_by_field_name("argument").and_then(unwrapped);
        if operand.is_none_or(|n| n.kind() != "number") {
            return PatternKey::Numeric(None);
        }
    }
    if let Some(n) = operand.filter(|n| n.kind() == "number") {
        return numeric_pattern_key(raw_text(n, bytes), negative);
    }
    if property.kind() == "object_assignment_pattern"
        && let Some(name) = property.child_by_field_name("left")
        && name.kind() == "shorthand_property_identifier_pattern"
    {
        return PatternKey::Known(text(name, bytes).into_owned());
    }
    assign_key(property, bytes).map_or(PatternKey::Unknown, PatternKey::Known)
}

fn numeric_pattern_key(raw: &str, negative: bool) -> PatternKey {
    let raw = raw.replace('_', "");
    let digits = raw.strip_suffix('n').unwrap_or(&raw);
    let radix = match digits.as_bytes() {
        [b'0', b'x' | b'X', ..] => Some(16),
        [b'0', b'o' | b'O', ..] => Some(8),
        [b'0', b'b' | b'B', ..] => Some(2),
        // Preserve uncertainty for legacy leading-zero spellings.
        [b'0', b'0'..=b'9', ..] => return PatternKey::Numeric(None),
        _ => None,
    };
    let number = match radix {
        Some(radix) => u64::from_str_radix(&digits[2..], radix)
            .ok()
            .map(|n| n as f64),
        None => digits.parse::<f64>().ok(),
    };
    let Some(number) = number else {
        return PatternKey::Numeric(None);
    };
    let number = if negative { -number } else { number };
    if number.fract() == 0.0 && number.abs() <= 9_007_199_254_740_991.0 {
        return PatternKey::Known((number as i64).to_string());
    }
    // Other numeric spellings may overwrite a string key that parses to the
    // same number. Matching f64 values alone cannot prove the keys are equal.
    PatternKey::Numeric(Some(number))
}

// Follow statically paired literal slots only.
fn collect_pattern_inputs<'tree>(
    session: &mut Session,
    left: Node<'tree>,
    right: Node<'tree>,
    bytes: &[u8],
    values: &mut BTreeMap<usize, Node<'tree>>,
    skipped_defaults: &mut BTreeSet<usize>,
) -> Result<(), TerminalFailure> {
    let mut pending = vec![(left, right)];
    while let Some((target, input)) = pending.pop() {
        session.nodes += 1;
        session.budget()?;
        let (Some(target), Some(input)) = (unwrapped(target), unwrapped(input)) else {
            continue;
        };
        match (target.kind(), input.kind()) {
            ("assignment_pattern" | "object_assignment_pattern", _) if distinct_literal(input) => {
                if let Some(default) = target.child_by_field_name("right") {
                    skipped_defaults.insert(default.id());
                }
                if let Some(left) = target.child_by_field_name("left") {
                    pending.push((left, input));
                }
            }
            ("array_pattern", "array") => {
                session.nodes += target.child_count() as usize + input.child_count() as usize;
                session.budget()?;
                let slots = |array: Node<'tree>| {
                    let mut index = 0;
                    let mut slots = BTreeMap::new();
                    for child in array.children(&mut array.walk()) {
                        match child.kind() {
                            "," => index += 1,
                            "spread_element" | "rest_pattern" => break,
                            _ if child.is_named() && !child.is_extra() => {
                                slots.insert(index, child);
                            }
                            _ => {}
                        }
                    }
                    slots
                };
                let inputs = slots(input);
                for (i, target) in slots(target) {
                    if let Some(&value) = inputs.get(&i) {
                        pending.push((target, value));
                    }
                }
            }
            ("object_pattern", "object") => {
                session.nodes += target.child_count() as usize + input.child_count() as usize;
                session.budget()?;
                let mut fields = BTreeMap::new();
                let mut unknown_numeric_end = 0;
                let mut numeric_ends = BTreeMap::new();
                for prop in input
                    .named_children(&mut input.walk())
                    .filter(|p| !p.is_extra())
                {
                    let key = match pattern_property_key(prop, bytes) {
                        PatternKey::Known(key) => key,
                        PatternKey::Numeric(Some(number)) => {
                            numeric_ends.insert(number.to_bits(), prop.end_byte());
                            continue;
                        }
                        PatternKey::Numeric(None) => {
                            unknown_numeric_end = prop.end_byte();
                            continue;
                        }
                        PatternKey::Unknown => {
                            fields.clear();
                            continue;
                        }
                    };
                    if key == "__proto__"
                        && prop.kind() == "pair"
                        && prop
                            .child_by_field_name("key")
                            .is_some_and(|k| k.kind() != "computed_property_name")
                    {
                        // The colon form sets a prototype, not a data property.
                        continue;
                    }
                    if let Some(value) = property_value(prop) {
                        fields.insert(key, value);
                    } else {
                        fields.remove(&key);
                    }
                }
                for prop in target
                    .named_children(&mut target.walk())
                    .filter(|p| !p.is_extra())
                {
                    if let PatternKey::Known(key) = pattern_property_key(prop, bytes)
                        && let Some(&input) = fields.get(&key)
                        && !key.parse::<f64>().is_ok_and(|number| {
                            let end = numeric_ends.get(&number.to_bits()).copied().unwrap_or(0);
                            input.start_byte() < end.max(unknown_numeric_end)
                        })
                        && let Some(dest) = prop.child_by_field_name("value").or_else(|| {
                            matches!(
                                prop.kind(),
                                "shorthand_property_identifier_pattern"
                                    | "object_assignment_pattern"
                            )
                            .then_some(prop)
                        })
                    {
                        pending.push((dest, input));
                    }
                }
            }
            (
                "identifier"
                | "shorthand_property_identifier_pattern"
                | "member_expression"
                | "subscript_expression",
                _,
            ) => {
                values.insert(target.id(), input);
            }
            _ => {}
        }
    }
    Ok(())
}

fn property_value(property: Node<'_>) -> Option<Node<'_>> {
    property
        .child_by_field_name("value")
        .or_else(|| (property.kind() == "shorthand_property_identifier").then_some(property))
}

struct MemberWrite<'tree> {
    receiver: Option<Node<'tree>>,
    assign: Option<Node<'tree>>,
    unconditional: bool,
    keys: Vec<WriteKey<'tree>>,
    capture: Capture,
    effect: usize,
}

fn assign_keys<'tree>(args: Node<'tree>, bytes: &[u8]) -> Vec<WriteKey<'tree>> {
    let mut keys = Vec::new();
    for source in args
        .named_children(&mut args.walk())
        .filter(|n| !n.is_extra())
        .skip(1)
    {
        let source = unwrapped(source).unwrap_or(source);
        if source.kind() == "object" {
            keys.extend(
                source
                    .named_children(&mut source.walk())
                    .filter(|n| !n.is_extra())
                    .map(|property| WriteKey {
                        name: assign_key(property, bytes),
                        distinct: false,
                        value: property_value(property),
                    }),
            );
        } else if !matches!(
            source.kind(),
            "null" | "true" | "false" | "number" | "string"
        ) {
            keys.push(WriteKey {
                value: None,
                name: None,
                distinct: false,
            });
        }
    }
    keys
}

impl MemberWrite<'_> {
    fn target(&self, bytes: &[u8], bindings: &Bindings) -> Option<String> {
        match self.receiver {
            Some(receiver) => expression_at(receiver, bytes, bindings, Some(self.capture)),
            None => Some("globalThis".into()),
        }
    }
    fn record(&self, bytes: &[u8], bindings: &Bindings, mutations: &mut MutationIndex) {
        let guaranteed = matches!(mutations, MutationIndex::Guaranteed { .. });
        if guaranteed && !self.unconditional {
            return;
        }
        if self.assign.is_some_and(|function| {
            expression_at(function, bytes, bindings, Some(self.capture)).as_deref()
                != Some("Object.assign")
        }) {
            return;
        }
        let position = (self.effect, self.capture.1, self.capture.2);
        let Some(target) = self.target(bytes, bindings) else {
            // If the bare globalThis still refers to the original object, writing
            // its own globalThis slot replaces it. Otherwise it was already replaced.
            if guaranteed
                && self
                    .keys
                    .iter()
                    .any(|key| key.distinct && key.name.as_deref() == Some("globalThis"))
                && self.receiver.and_then(unwrapped).is_some_and(|n| {
                    n.kind() == "identifier"
                        && text(n, bytes) == "globalThis"
                        && matches!(bindings.visible("globalThis", n), Ok(None))
                })
            {
                mutations.record("globalThis".into(), position, true);
            }
            return;
        };
        if self.assign.is_some() {
            if !(matches!(
                target.as_str(),
                "globalThis"
                    | "Object"
                    | "Bun"
                    | "process"
                    | "fs"
                    | "fs.promises"
                    | "child_process"
                    | "https"
            ) || bindings.bun_files.contains_key(&target))
            {
                return;
            }
        } else if matches!(target.as_str(), "process.env" | "Bun.env") {
            // Updating an env slot does not replace the environment object.
            return;
        }
        for key in &self.keys {
            // A dynamic key or opaque source can affect any member, but
            // establishes no guaranteed write to an individual member.
            if !guaranteed || (key.distinct && key.name.is_some()) {
                record_member_mutation(
                    mutations,
                    &target,
                    key.name.as_deref(),
                    position,
                    key.distinct,
                );
            }
        }
    }
}

// These names denote distinct objects in the supported runtime. Arbitrary
// different property paths are not proof: two properties can alias each other.
fn known_builtin_identity(path: &str) -> bool {
    BUILTIN_GLOBALS.contains(&path)
        || matches!(path, "fs" | "fs.promises" | "https" | "child_process")
        || path.strip_prefix("Object.").is_some_and(|key| {
            matches!(
                key,
                "assign"
                    | "create"
                    | "defineProperties"
                    | "defineProperty"
                    | "entries"
                    | "freeze"
                    | "fromEntries"
                    | "getOwnPropertyDescriptor"
                    | "getOwnPropertyDescriptors"
                    | "getOwnPropertyNames"
                    | "getOwnPropertySymbols"
                    | "getPrototypeOf"
                    | "groupBy"
                    | "hasOwn"
                    | "is"
                    | "isExtensible"
                    | "isFrozen"
                    | "isSealed"
                    | "keys"
                    | "preventExtensions"
                    | "seal"
                    | "setPrototypeOf"
                    | "values"
            )
        })
}

fn resolve_writes<'tree>(
    session: &mut Session,
    writes: &mut [MemberWrite<'tree>],
    aliases: &[&Alias<'tree>],
    bytes: &[u8],
    bindings: &mut Bindings,
    declared_functions: BTreeSet<BindingIdentity>,
) -> Result<(), TerminalFailure> {
    if writes.is_empty() {
        return Ok(());
    }
    let relevant = write_aliases(session, writes, aliases, bytes, bindings)?;
    let fresh = distinct_const_bindings(session, &relevant, bytes, bindings, declared_functions)?;
    resolve_aliases(session, &relevant, bytes, bindings)?;
    let mut comparisons = Vec::new();
    for (wi, write) in writes.iter_mut().enumerate() {
        let target = write.target(bytes, bindings);
        for (ki, key) in write.keys.iter_mut().enumerate() {
            session.nodes += 1;
            session.budget()?;
            let Some(value) = key.value.and_then(unwrapped) else {
                continue;
            };
            key.distinct |= distinct_literal(value)
                || matches!(value.kind(), "identifier" | "shorthand_property_identifier")
                    && bindings
                        .visible(&text(value, bytes), value)
                        .ok()
                        .flatten()
                        .is_some_and(|b| fresh.contains(&b.identity()));
            if !key.distinct
                && let Some((object, name)) = target.as_deref().zip(key.name.as_deref())
            {
                let slot = if object == "globalThis" {
                    name.to_owned()
                } else {
                    format!("{object}.{name}")
                };
                if known_builtin_identity(&slot) {
                    comparisons.push((wi, ki, value, slot));
                }
            }
        }
    }
    if !comparisons.is_empty() {
        // Confirm RHS identities with *all* candidate mutations applied before
        // using them to rule out a receiver. A modified Object.is could be assign.
        let mut upper = MutationIndex::default();
        for write in writes.iter() {
            session.nodes += 1 + write.keys.len();
            session.budget()?;
            write.record(bytes, bindings, &mut upper);
        }
        bindings.mutations = upper;
        resolve_aliases(session, &relevant, bytes, bindings)?;
        for (wi, ki, value, slot) in comparisons {
            session.nodes += 1;
            session.budget()?;
            let write = &mut writes[wi];
            let capture = (
                bindings.order.start(value),
                write.capture.1,
                write.capture.2,
            );
            write.keys[ki].distinct = expression_at(value, bytes, bindings, Some(capture))
                .is_some_and(|rhs| rhs != slot && known_builtin_identity(&rhs));
        }
    }
    let mut collect = |bindings: &mut Bindings, mut index: MutationIndex| {
        resolve_aliases(session, &relevant, bytes, bindings)?;
        for write in writes.iter() {
            session.nodes += 1 + write.keys.len();
            session.budget()?;
            write.record(bytes, bindings, &mut index);
        }
        Ok::<_, TerminalFailure>(index)
    };
    let mut guaranteed = MutationIndex::guaranteed();
    loop {
        bindings.mutations = guaranteed;
        let possible = collect(bindings, MutationIndex::default())?;
        let previous = std::mem::replace(&mut bindings.mutations, possible);
        guaranteed = collect(bindings, MutationIndex::guaranteed())?;
        if let (
            MutationIndex::Possible(_, possible_restorations),
            MutationIndex::Guaranteed { restorations, .. },
        ) = (&mut bindings.mutations, &mut guaranteed)
        {
            *restorations = std::mem::take(possible_restorations);
        }
        if guaranteed == previous {
            return Ok(());
        }
    }
}

// Only the root binding can supply the identity of a supported receiver, callee
// or initializer. Computed keys and loader arguments contribute no alias values.
fn write_aliases<'a, 'tree>(
    session: &mut Session,
    writes: &[MemberWrite<'tree>],
    aliases: &[&'a Alias<'tree>],
    bytes: &[u8],
    bindings: &Bindings,
) -> Result<Vec<&'a Alias<'tree>>, TerminalFailure> {
    if writes.is_empty() || aliases.is_empty() {
        return Ok(Vec::new());
    }
    let by_binding: BTreeMap<_, _> = aliases
        .iter()
        .map(|alias| (&alias.binding, *alias))
        .collect();
    let mut pending = Vec::new();
    for write in writes {
        pending.extend(write.receiver);
        pending.extend(write.keys.iter().filter_map(|key| key.value));
        pending.extend(write.assign);
    }
    let mut needed = BTreeSet::new();
    while let Some(node) = pending.pop() {
        session.nodes += 1;
        session.budget()?;
        match node.kind() {
            "identifier" | "shorthand_property_identifier" => {
                if let Ok(Some(binding)) = bindings.visible(&text(node, bytes), node)
                    && let Some(alias) = by_binding.get(&binding.identity())
                    && needed.insert(&alias.binding)
                {
                    pending.push(alias.value);
                }
            }
            "member_expression" | "subscript_expression" => {
                pending.extend(node.child_by_field_name("object"))
            }
            "call_expression" => pending.extend(node.child_by_field_name("function")),
            _ => pending.extend(wrapped_operand(node)),
        }
    }
    Ok(aliases
        .iter()
        .copied()
        .filter(|alias| needed.contains(&alias.binding))
        .collect())
}

fn resolve_aliases(
    session: &mut Session,
    aliases: &[&Alias<'_>],
    bytes: &[u8],
    bindings: &mut Bindings,
) -> Result<(), TerminalFailure> {
    for alias in aliases {
        if let Some(slot) = alias.slot(bindings) {
            slot.value = None;
        }
    }
    // Forward references in closures can need another pass. Cycles stay unknown.
    for _ in 0..64 {
        let mut changed = false;
        for alias in aliases {
            session.nodes += 1;
            session.budget()?;
            let node = alias.value;
            let capture = Some(alias.capture);
            let value = if is_bun_file_call(node, bytes, bindings, capture) {
                let handle = format!("#{}", node.start_byte());
                bindings
                    .bun_files
                    .entry(handle.clone())
                    .or_insert_with(|| bun_file_path(node, bytes));
                Some(handle)
            } else {
                expression_at(node, bytes, bindings, capture)
            };
            if let Some(binding) = alias.slot(bindings)
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

impl Alias<'_> {
    fn slot<'a>(&self, bindings: &'a mut Bindings) -> Option<&'a mut Binding> {
        let (name, start, end) = &self.binding;
        bindings
            .named_mut(name)
            .iter_mut()
            .take(128)
            .find(|b| b.start == *start && b.end == *end)
    }
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
#[derive(Clone, Copy)]
struct ExecutionScope<'tree> {
    function: Node<'tree>,
    straight_line: bool,
    protected: bool,
    // A top-level pattern can evaluate a loader default before a later store.
    ordered_pattern: bool,
}

impl ExecutionScope<'_> {
    fn capture(self, at: usize) -> Capture {
        (at, self.function.start_byte(), self.function.end_byte())
    }
}

// Compute closed, non-throwing syntax bottom-up. Unknown bindings, spreads,
// getter reads and calls are deliberately absent. Keep primitive results
// separate because coercion of an object could run user code.
#[derive(Clone, Copy, PartialEq)]
enum Inert {
    Primitive,
    Other,
}
fn erased_or_extra(node: Node<'_>) -> bool {
    node.is_extra()
        || is_type_syntax(node)
        || matches!(node.kind(), "ambient_declaration" | "function_signature")
}
fn inert_node(node: Node<'_>, bytes: &[u8], known: &BTreeMap<usize, Inert>) -> Option<Inert> {
    let value = |n: Node<'_>| known.get(&n.id()).copied();
    let all = || {
        node.named_children(&mut node.walk())
            .filter(|n| !erased_or_extra(*n))
            .all(|n| value(n).is_some())
    };
    if let Some(operand) = wrapped_operand(node) {
        return value(operand);
    }
    match node.kind() {
        "template_string"
            if node.named_children(&mut node.walk()).all(|n| {
                matches!(n.kind(), "string_fragment" | "escape_sequence")
                    || value(n) == Some(Inert::Primitive)
            }) =>
        {
            Some(Inert::Primitive)
        }
        "template_substitution"
            if first_named_child(node).and_then(value) == Some(Inert::Primitive) =>
        {
            Some(Inert::Primitive)
        }
        "number" if !raw_text(node, bytes).ends_with('n') => Some(Inert::Primitive),
        "true" | "false" | "null" | "string" => Some(Inert::Primitive),
        "number"
        | "regex"
        | "arrow_function"
        | "function_expression"
        | "generator_function"
        | "empty_statement"
        | "function_declaration"
        | "generator_function_declaration"
        | "interface_declaration"
        | "type_alias_declaration" => Some(Inert::Other),
        "variable_declarator"
            if node
                .child_by_field_name("name")
                .is_some_and(|n| n.kind() == "identifier")
                && node
                    .child_by_field_name("value")
                    .is_none_or(|n| value(n).is_some()) =>
        {
            Some(Inert::Other)
        }
        "expression_statement"
        | "array"
        | "object"
        | "statement_block"
        | "lexical_declaration"
        | "variable_declaration"
            if all() =>
        {
            Some(Inert::Other)
        }
        "sequence_expression" if all() => node
            .named_children(&mut node.walk())
            .filter(|n| !erased_or_extra(*n))
            .last()
            .and_then(value),
        "computed_property_name"
            if first_named_child(node).and_then(value) == Some(Inert::Primitive) =>
        {
            Some(Inert::Other)
        }
        "pair" | "method_definition" => {
            let key = node
                .child_by_field_name("key")
                .or_else(|| node.child_by_field_name("name"))?;
            let safe_key = matches!(key.kind(), "property_identifier" | "string" | "number")
                || value(key).is_some();
            (safe_key
                && node
                    .child_by_field_name("value")
                    .is_none_or(|n| value(n).is_some())
                && !has_token(node, "decorator"))
            .then_some(Inert::Other)
        }
        "unary_expression" => {
            let operand = value(node.child_by_field_name("argument")?)?;
            match node.child_by_field_name("operator")?.kind() {
                "!" | "typeof" | "void" => Some(Inert::Primitive),
                "+" | "-" | "~" if operand == Inert::Primitive => Some(Inert::Primitive),
                _ => None,
            }
        }
        "binary_expression" => {
            let left = value(node.child_by_field_name("left")?)?;
            let right = value(node.child_by_field_name("right")?)?;
            let primitives = left == Inert::Primitive && right == Inert::Primitive;
            match node.child_by_field_name("operator")?.kind() {
                "===" | "!==" => Some(Inert::Primitive),
                "&&" | "||" | "??" => Some(if primitives {
                    Inert::Primitive
                } else {
                    Inert::Other
                }),
                "in" | "instanceof" => None,
                _ if primitives => Some(Inert::Primitive),
                _ => None,
            }
        }
        _ => None,
    }
}

fn first_effectful<'tree>(
    node: Node<'tree>,
    known: &BTreeMap<usize, Inert>,
) -> Option<Node<'tree>> {
    node.named_children(&mut node.walk())
        .find(|n| !erased_or_extra(*n) && !known.contains_key(&n.id()))
}

#[derive(Default)]
struct ExecutionScopes<'tree> {
    scopes: BTreeMap<usize, ExecutionScope<'tree>>,
    optional_chains: BTreeSet<usize>,
    body_cutoffs: BTreeMap<usize, usize>,
    protected_tries: BTreeSet<usize>,
    try_operands: BTreeMap<usize, Option<usize>>,
    inert: BTreeMap<usize, Inert>,
}

impl<'tree> ExecutionScopes<'tree> {
    fn new(
        session: &mut Session,
        nodes: &[(Node<'tree>, bool)],
        bytes: &[u8],
    ) -> Result<Self, TerminalFailure> {
        let mut scopes = Self::default();
        let has_try = nodes
            .iter()
            .any(|(n, runtime)| *runtime && n.kind() == "try_statement");
        let track_jumps = has_try
            || nodes.iter().any(|(n, runtime)| {
                *runtime && matches!(n.kind(), "do_statement" | "labeled_statement")
            });
        let mut may_jump = BTreeSet::new();
        // The validation walk already charged each node. Check cancellation on
        // this reverse walk and charge repeated inspections of block prefixes.
        for (node, runtime) in nodes.iter().rev() {
            session.check()?;
            if !runtime {
                continue;
            }
            if has_try && let Some(fact) = inert_node(*node, bytes, &scopes.inert) {
                scopes.inert.insert(node.id(), fact);
            }
            if track_jumps
                && (matches!(
                    node.kind(),
                    "throw_statement"
                        | "break_statement"
                        | "continue_statement"
                        | "return_statement"
                ) || !function_boundary(*node, node.end_byte())
                    && node
                        .named_children(&mut node.walk())
                        .any(|c| may_jump.contains(&c.id())))
            {
                may_jump.insert(node.id());
            }
            if matches!(
                node.kind(),
                "try_statement" | "do_statement" | "labeled_statement"
            ) {
                let finalizer = node
                    .child_by_field_name("finalizer")
                    .and_then(|f| f.child_by_field_name("body"));
                let protected = node.kind() == "try_statement"
                    && (node.child_by_field_name("handler").is_some()
                        || finalizer
                            .and_then(|f| first_effectful(f, &scopes.inert))
                            .is_some());
                if protected {
                    scopes.protected_tries.insert(node.id());
                }
                for (body, prefix_only) in node
                    .child_by_field_name("body")
                    .map(|b| (b, protected))
                    .into_iter()
                    .chain(finalizer.map(|b| (b, false)))
                {
                    let mut entry = Some(body);
                    while let Some(block) = entry.filter(|n| n.kind() == "statement_block") {
                        session.nodes += block.named_child_count();
                        session.budget()?;
                        let boundary = if prefix_only {
                            first_effectful(block, &scopes.inert)
                        } else {
                            block
                                .named_children(&mut block.walk())
                                .find(|stmt| may_jump.contains(&stmt.id()))
                        };
                        let Some(stop) = boundary else { break };
                        scopes.body_cutoffs.insert(block.id(), stop.start_byte());
                        entry = Some(stop);
                    }
                }
            }
            let previous = match node.kind() {
                "member_expression" | "subscript_expression" => node.child_by_field_name("object"),
                "call_expression" => node.child_by_field_name("function"),
                // Erased TS postfix syntax continues the chain; parentheses end it.
                "non_null_expression" | "instantiation_expression" => wrapped_operand(*node),
                _ => continue,
            };
            if node.child_by_field_name("optional_chain").is_some()
                || has_token(*node, "?.")
                || has_token(*node, "optional_chain")
                || previous.is_some_and(|n| scopes.optional_chains.contains(&n.id()))
            {
                scopes.optional_chains.insert(node.id());
            }
        }
        Ok(scopes)
    }

    fn get(
        &mut self,
        session: &mut Session,
        mut node: Node<'tree>,
    ) -> Result<ExecutionScope<'tree>, TerminalFailure> {
        let mut path = Vec::new();
        let mut scope = loop {
            session.nodes += 1;
            session.budget()?;
            if let Some(scope) = self.scopes.get(&node.id()) {
                break *scope;
            }
            let Some(parent) = node.parent() else {
                break ExecutionScope {
                    function: node,
                    straight_line: true,
                    protected: false,
                    ordered_pattern: true,
                };
            };
            if function_boundary(parent, node.start_byte()) {
                path.push((parent, node, true, true));
                break ExecutionScope {
                    function: parent,
                    straight_line: true,
                    protected: false,
                    ordered_pattern: parent.kind() == "program",
                };
            }
            let eager = match parent.kind() {
                "member_expression" | "subscript_expression" | "call_expression" => {
                    parent.child_by_field_name("object") == Some(node)
                        || parent.child_by_field_name("function") == Some(node)
                        || !self.optional_chains.contains(&parent.id())
                }
                "binary_expression" => {
                    parent.child_by_field_name("left") == Some(node)
                        || !parent
                            .child_by_field_name("operator")
                            .is_some_and(|operator| matches!(operator.kind(), "&&" | "||" | "??"))
                }
                "ternary_expression" | "if_statement" | "while_statement" => {
                    parent.child_by_field_name("condition") == Some(node)
                }
                "switch_statement" => parent.child_by_field_name("value") == Some(node),
                "for_statement" => {
                    parent.child_by_field_name("initializer") == Some(node)
                        || parent.child_by_field_name("condition") == Some(node)
                }
                "augmented_assignment_expression" => {
                    parent.child_by_field_name("left") == Some(node)
                        || !short_circuit_assignment(parent)
                }
                "try_statement" => {
                    parent.child_by_field_name("body") == Some(node)
                        || parent.child_by_field_name("finalizer") == Some(node)
                }
                "finally_clause" | "do_statement" | "labeled_statement" => {
                    parent.child_by_field_name("body") == Some(node)
                }
                "statement_block" => self
                    .body_cutoffs
                    .get(&parent.id())
                    .is_none_or(|end| node.start_byte() <= *end),
                "for_in_statement" => parent.child_by_field_name("right") == Some(node),
                "expression_statement"
                | "new_expression"
                | "await_expression"
                | "return_statement"
                | "throw_statement"
                | "arguments"
                | "variable_declarator"
                | "lexical_declaration"
                | "variable_declaration"
                | "program"
                | "export_statement"
                | "internal_module"
                | "module"
                | "array"
                | "object"
                | "pair"
                | "computed_property_name"
                | "unary_expression"
                | "sequence_expression"
                | "assignment_expression"
                | "template_string"
                | "template_substitution"
                | "spread_element" => true,
                _ => wrapped_operand(parent) == Some(node),
            };
            let pattern = matches!(
                parent.kind(),
                "array_pattern"
                    | "object_pattern"
                    | "pair_pattern"
                    | "assignment_pattern"
                    | "object_assignment_pattern"
                    | "rest_pattern"
            );
            path.push((parent, node, eager, eager || pattern));
            node = parent;
        };
        for (parent, child, eager_edge, ordered_edge) in path.into_iter().rev() {
            let finalizer = parent.kind() == "try_statement"
                && parent.child_by_field_name("finalizer") == Some(child);
            scope.protected |= !finalizer && self.protected_tries.contains(&parent.id());
            // In a try, an earlier operand can throw and the caller may still
            // reach the later replacement. Only follow the first evaluation path.
            let reached = !scope.protected
                || finalizer
                || *self.try_operands.entry(parent.id()).or_insert_with(|| {
                    let first = match parent.kind() {
                        "try_statement" | "finally_clause" | "do_statement"
                        | "labeled_statement" => parent.child_by_field_name("body"),
                        "variable_declarator" => parent.child_by_field_name("value"),
                        "for_in_statement" => parent.child_by_field_name("right"),
                        "assignment_expression"
                            if parent
                                .child_by_field_name("left")
                                .and_then(unwrapped)
                                .is_some_and(|n| {
                                    matches!(
                                        n.kind(),
                                        "identifier" | "object_pattern" | "array_pattern"
                                    )
                                }) =>
                        {
                            parent.child_by_field_name("right")
                        }
                        "pair"
                            if parent.child_by_field_name("key").is_some_and(|n| {
                                matches!(n.kind(), "property_identifier" | "string" | "number")
                            }) =>
                        {
                            parent.child_by_field_name("value")
                        }
                        _ => {
                            wrapped_operand(parent).or_else(|| first_effectful(parent, &self.inert))
                        }
                    };
                    first.map(|n| n.id())
                }) == Some(child.id());
            scope.straight_line &= eager_edge && reached;
            scope.ordered_pattern &= ordered_edge && reached;
            self.scopes.insert(child.id(), scope);
        }
        Ok(scope)
    }
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
// Type names participate in ambient export lookup only; they must never
// shadow a value in the expression resolver.
#[derive(Default)]
struct NamespaceNames {
    names: BTreeMap<String, Vec<NamespaceName>>,
}
struct NamespaceName {
    range: std::ops::Range<usize>,
    namespace: Option<usize>,
}
impl NamespaceNames {
    fn add(&mut self, name: String, location: Node<'_>, namespace: Option<usize>) {
        self.names.entry(name).or_default().push(NamespaceName {
            range: location.byte_range(),
            namespace,
        });
    }

    fn exported_value(
        &self,
        name: &str,
        at: Node<'_>,
        bindings: &Bindings,
        dependencies: &mut Vec<usize>,
    ) -> bool {
        let types = self.names.get(name).map(Vec::as_slice).unwrap_or(&[]);
        let values = bindings.named(name);
        if types.len() + values.len() > 128 {
            return true;
        }
        let visible = |start: usize, end: usize| start <= at.start_byte() && at.end_byte() <= end;
        let types = types.iter().filter(|t| visible(t.range.start, t.range.end));
        let Some(width) = types.clone().map(|t| t.range.len()).min() else {
            // A value declaration or an unresolved name may supply an ambient value.
            return true;
        };
        if values
            .iter()
            .any(|b| visible(b.start, b.end) && b.end - b.start <= width)
        {
            return true;
        }
        dependencies.extend(
            types
                .filter(|t| t.range.len() == width)
                .filter_map(|t| t.namespace),
        );
        false
    }

    fn bind(
        self,
        namespaces: &[(Node<'_>, bool)],
        bytes: &[u8],
        bindings: &mut Bindings,
        session: &mut Session,
    ) -> Result<(), TerminalFailure> {
        let mut dependents: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        let mut pending = Vec::new();
        for &(node, runtime) in namespaces {
            let mut dependencies = Vec::new();
            if namespace_has_value(node, bytes, runtime, &self, bindings, &mut dependencies) {
                pending.push(node.id());
            } else {
                for dependency in dependencies {
                    dependents.entry(dependency).or_default().push(node.id());
                }
            }
        }
        // A nested or forward-exported namespace supplies a value only when a
        // declaration in its dependency chain does. Type-only cycles stay erased.
        let mut values = BTreeSet::new();
        while let Some(id) = pending.pop() {
            session.nodes += 1;
            session.budget()?;
            if values.insert(id) {
                pending.extend(dependents.remove(&id).into_iter().flatten());
            }
        }
        for (name, decls) in self.names {
            for decl in decls {
                if decl.namespace.is_some_and(|id| values.contains(&id)) {
                    bindings.push(Binding {
                        name: name.clone(),
                        start: decl.range.start,
                        end: decl.range.end,
                        value: None,
                    });
                }
            }
        }
        Ok(())
    }
}

fn namespace_has_value(
    node: Node<'_>,
    bytes: &[u8],
    runtime: bool,
    names: &NamespaceNames,
    bindings: &Bindings,
    dependencies: &mut Vec<usize>,
) -> bool {
    // Bun emits the outer object in A.B even when B has no runtime statements.
    if runtime
        && node
            .child_by_field_name("name")
            .is_some_and(|name| name.kind() == "nested_identifier")
    {
        return true;
    }
    let Some(body) = node.child_by_field_name("body") else {
        return false;
    };
    let mut in_directives = true;
    body.named_children(&mut body.walk()).any(|statement| {
        if statement.is_extra() {
            return false;
        }
        let exported = statement.kind() == "export_statement";
        let declaration = if exported {
            match statement.child_by_field_name("declaration") {
                Some(declaration) => declaration,
                None if !runtime && statement.child_by_field_name("source").is_none() => {
                    if has_token(statement, "type") {
                        return false;
                    }
                    if let Some(clause) =
                        first_named_child(statement).filter(|c| c.kind() == "export_clause")
                    {
                        return clause.named_children(&mut clause.walk()).any(|spec| {
                            spec.kind() == "export_specifier"
                                && !has_token(spec, "type")
                                && spec.child_by_field_name("name").is_none_or(|name| {
                                    names.exported_value(
                                        &text(name, bytes),
                                        spec,
                                        bindings,
                                        dependencies,
                                    )
                                })
                        });
                    }
                    return true;
                }
                None => return !type_only_module(statement),
            }
        } else {
            statement
        };
        let value = if declaration.kind() == "expression_statement" {
            first_named_child(declaration).unwrap_or(declaration)
        } else {
            declaration
        };
        match value.kind() {
            "internal_module" | "module" => {
                dependencies.push(value.id());
                false
            }
            "ambient_declaration" => {
                // An exported ambient value can materialize its enclosing namespace.
                exported
                    && first_named_child(value).is_some_and(|child| {
                        !matches!(
                            child.kind(),
                            "internal_module"
                                | "module"
                                | "interface_declaration"
                                | "type_alias_declaration"
                        )
                    })
            }
            "import_alias" | "import_statement" => {
                let runtime_import = !type_only_module(value);
                in_directives &= !runtime_import;
                exported && runtime_import
            }
            "empty_statement" => {
                in_directives = false;
                false
            }
            "function_signature" => !runtime,
            "interface_declaration" | "type_alias_declaration" => false,
            _ => {
                !(in_directives
                    && decoded_string(value, bytes).is_some_and(|literal| {
                        matches!(literal.as_str(), "use strict" | "use asm")
                    }))
            }
        }
    })
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
            | "type_identifier"
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
        "property_identifier"
        | "shorthand_property_identifier"
        | "shorthand_property_identifier_pattern"
        | "number" => {
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
    capture: Option<Capture>,
) -> Option<String> {
    // Bound recursion independently of hostile expression depth.
    let mut node = node;
    let mut keys = Vec::new();
    for _ in 0..64 {
        let mut value = match node.kind() {
            "member_expression" | "subscript_expression" => {
                keys.push((member_key(node, bytes)?, bindings.order.end(node)));
                node = node.child_by_field_name("object")?;
                continue;
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
                builtin(&spec)?.to_owned()
            }
            "identifier" | "shorthand_property_identifier" | "import" => {
                let name = text(node, bytes);
                if let Some(binding) = bindings.visible(&name, node).ok()? {
                    binding.value.clone()?
                } else if name == "import" || BUILTIN_GLOBALS.contains(&name.as_ref()) {
                    if !bindings.mutations.unchanged(&name, capture) {
                        return None;
                    }
                    name.to_string()
                } else {
                    return None;
                }
            }
            _ => {
                node = wrapped_operand(node)?;
                continue;
            }
        };
        // A member is looked up only after evaluating its receiver and key. In
        // particular, require() arguments can replace the exported method.
        for (key, at) in keys.into_iter().rev() {
            let lookup = capture.map(|(earliest, start, end)| (at.max(earliest), start, end));
            value = bindings.member(&value, &key, lookup)?;
        }
        return Some(value);
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
    capture: Option<Capture>,
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
    capture: Option<Capture>,
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
    let path = if is_bun_file_call(receiver, bytes, bindings, capture) {
        bun_file_path(receiver, bytes)
    } else {
        let handle = expression_at(receiver, bytes, bindings, capture)?;
        let path = bindings.bun_files.get(&handle)?;
        bindings.member(&handle, &key, capture)?;
        path.clone()
    };
    Some((access, path))
}
