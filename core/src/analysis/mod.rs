//! Read-only source evidence. This module has no host, process or environment access.
//!
//! Reports are review projections, never execution capabilities. Core owns entry
//! discovery and must bind the report to its captured input before admitting code.

mod syntax;
#[cfg(test)]
mod tests;

use crate::digest::sha256_hex;
use crate::permission::PermissionDeclarationV1;
use crate::plan::{EnvironmentSensitivityV1, FilesystemAccessV1, PermissionV1};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub const RULE_VERSION: u32 = 4;
pub const ANALYZER_VERSION: &str = "tree-sitter-0.25.10/js-0.23.1/ts-0.23.2/language-0.1.9";
pub const SCHEMA_VERSION: u32 = 1;

#[derive(
    Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, Eq, PartialEq, Ord, PartialOrd,
)]
#[serde(rename_all = "kebab-case")]
pub enum DiagnosticKind {
    Difference,
    Failure,
    CoverageGap,
    Uncorroborated,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, Eq, PartialEq, Ord, PartialOrd)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    pub code: String,
    /// Category-relative logical path; never a physical checkout path.
    pub file: String,
    pub line: usize,
    /// One-based UTF-8 byte column.
    pub column: usize,
    pub behavior: String,
    /// A digest, never a raw source literal, argv, URL or environment value.
    pub identity: Option<String>,
}

impl Diagnostic {
    pub(crate) fn new(
        kind: DiagnosticKind,
        code: &str,
        file: &str,
        line: usize,
        column: usize,
        behavior: &str,
    ) -> Self {
        Self {
            kind,
            code: code.into(),
            file: file.into(),
            line,
            column,
            behavior: behavior.into(),
            identity: None,
        }
    }
    pub fn blocking(&self) -> bool {
        matches!(
            self.kind,
            DiagnosticKind::Difference | DiagnosticKind::Failure
        )
    }

    pub fn explanation(&self) -> &'static str {
        match self.kind {
            DiagnosticKind::Difference => {
                "Detected behavior is not covered by the declarations; add an accurate declaration or change the script to fit its declarations."
            }
            DiagnosticKind::Failure => {
                "Analysis evidence is unavailable or failed; fix the reported source or limit before continuing."
            }
            DiagnosticKind::CoverageGap => {
                "This behavior is outside verified coverage; existing code trust is still required."
            }
            DiagnosticKind::Uncorroborated => {
                "This declaration was not corroborated within the analyzed scope; it does not block admission."
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, Eq, PartialEq)]
pub struct Report {
    pub analyzer_version: String,
    pub schema_version: u32,
    pub rule_version: u32,
    pub files: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
    pub allowed: bool,
}

/// Safe author/frontend projection. It is not execution evidence.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, Eq, PartialEq)]
pub struct Scope {
    pub target: String,
    pub entry: String,
    pub script: Option<String>,
    pub report: Report,
}

/// The digest binds a Plan; only Core's private prepared report can satisfy it.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, Eq, PartialEq)]
pub struct Binding {
    pub identity: String,
    pub scope: Scope,
}

/// Captured identity only. Fixed variables take precedence over injection mappings.
#[derive(Clone, Debug, Default, Serialize, Eq, PartialEq)]
pub struct EnvironmentContract {
    pub fixed: BTreeSet<String>,
    /// target -> (source, known sensitivity). Unknown sensitivity cannot be guessed.
    pub mappings: BTreeMap<String, (String, Option<EnvironmentSensitivityV1>)>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Limits {
    pub file_bytes: usize,
    pub graph_files: usize,
    pub graph_depth: usize,
    pub command_bytes: usize,
    pub command_files: usize,
    pub findings: usize,
    pub nodes: usize,
    pub parser_steps: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            file_bytes: 1 << 20,
            graph_files: 256,
            graph_depth: 32,
            command_bytes: 16 << 20,
            command_files: 2048,
            findings: 50_000,
            nodes: 2_000_000,
            parser_steps: 100_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalFailure {
    Cancelled,
    Budget,
    Analyzer,
}
impl std::fmt::Display for TerminalFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Cancelled => "code_analysis_cancelled",
            Self::Budget => "code_analysis_command_budget",
            Self::Analyzer => "code_analysis_analyzer_failure",
        })
    }
}
impl std::error::Error for TerminalFailure {}

#[derive(Clone, Debug)]
enum Behavior {
    Environment(String),
    Command(String),
    Network(String),
    File(FilesystemAccessV1, String),
}
#[derive(Clone, Debug)]
struct Finding {
    behavior: Behavior,
    line: usize,
    column: usize,
}
#[derive(Clone, Debug, Default)]
struct Facts {
    findings: Vec<Finding>,
    diagnostics: Vec<Diagnostic>,
    imports: Vec<(String, usize, usize)>,
}

/// One command's cache and cumulative budget. It never reads a file itself.
pub struct Session {
    limits: Limits,
    cancelled: Arc<AtomicBool>,
    terminal: Option<TerminalFailure>,
    cache: BTreeMap<(String, String), Facts>,
    bytes: usize,
    files: usize,
    findings: usize,
    nodes: usize,
    parser_steps: usize,
}

impl Default for Session {
    fn default() -> Self {
        Self::new(Limits::default(), Arc::new(AtomicBool::new(false)))
    }
}
impl Session {
    pub fn new(limits: Limits, cancelled: Arc<AtomicBool>) -> Self {
        Self {
            limits,
            cancelled,
            terminal: None,
            cache: BTreeMap::new(),
            bytes: 0,
            files: 0,
            findings: 0,
            nodes: 0,
            parser_steps: 0,
        }
    }
    pub fn parsed_files(&self) -> usize {
        self.files
    }
    pub(crate) fn charge_diagnostics(&mut self, count: usize) -> Result<(), TerminalFailure> {
        self.findings = self.findings.saturating_add(count);
        self.budget()
    }
    pub fn check(&mut self) -> Result<(), TerminalFailure> {
        if self.cancelled.load(Ordering::Relaxed) {
            self.terminal = Some(TerminalFailure::Cancelled);
        }
        if let Some(error) = self.terminal {
            Err(error)
        } else {
            Ok(())
        }
    }
    fn budget(&mut self) -> Result<(), TerminalFailure> {
        if self.bytes > self.limits.command_bytes
            || self.files > self.limits.command_files
            || self.findings > self.limits.findings
            || self.nodes > self.limits.nodes
            || self.parser_steps > self.limits.parser_steps
        {
            self.terminal = Some(TerminalFailure::Budget);
        }
        self.check()
    }
    /// `sources` contains category-relative names and captured bytes only.
    pub fn analyze<S: AsRef<[u8]>>(
        &mut self,
        sources: &BTreeMap<String, S>,
        entry: &str,
        declaration: Option<&PermissionDeclarationV1>,
        env: &EnvironmentContract,
    ) -> Result<Report, TerminalFailure> {
        let result = self.analyze_inner(sources, entry, declaration, env);
        if let Err(error) = result {
            self.terminal = Some(error);
        }
        result
    }

    fn analyze_inner<S: AsRef<[u8]>>(
        &mut self,
        sources: &BTreeMap<String, S>,
        entry: &str,
        declaration: Option<&PermissionDeclarationV1>,
        env: &EnvironmentContract,
    ) -> Result<Report, TerminalFailure> {
        self.check()?;
        let permissions = declaration.and_then(|d| d.permission_set().ok());
        let declared = permissions
            .as_ref()
            .map(|p| p.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let mut corroborated = BTreeSet::new();
        let mut diagnostics = Vec::new();
        let mut visited = BTreeSet::new();
        let mut pending = vec![(entry.to_string(), 0)];
        while let Some((file, depth)) = pending.pop() {
            self.check()?;
            if visited.contains(&file) {
                continue;
            }
            if depth > self.limits.graph_depth || visited.len() >= self.limits.graph_files {
                diagnostics.push(Diagnostic::new(
                    DiagnosticKind::Failure,
                    "code_analysis_graph_limit",
                    &file,
                    1,
                    1,
                    "dependency",
                ));
                continue;
            }
            visited.insert(file.clone());
            let Some(bytes) = sources.get(&file) else {
                diagnostics.push(Diagnostic::new(
                    DiagnosticKind::Failure,
                    "code_analysis_source_missing",
                    &file,
                    1,
                    1,
                    "dependency",
                ));
                continue;
            };
            let bytes = bytes.as_ref();
            if bytes.len() > self.limits.file_bytes {
                diagnostics.push(Diagnostic::new(
                    DiagnosticKind::Failure,
                    "code_analysis_file_limit",
                    &file,
                    1,
                    1,
                    "source",
                ));
                continue;
            }
            let key = (file.clone(), sha256_hex(bytes));
            let facts = if let Some(facts) = self.cache.get(&key) {
                facts.clone()
            } else {
                self.files += 1;
                self.bytes += bytes.len();
                self.budget()?;
                let facts = syntax::parse(self, &file, bytes)?;
                self.findings +=
                    facts.findings.len() + facts.diagnostics.len() + facts.imports.len();
                self.budget()?;
                self.cache.insert(key, facts.clone());
                facts
            };
            diagnostics.extend(facts.diagnostics);
            for (specifier, line, column) in facts.imports {
                match resolve(sources, &file, &specifier) {
                    Ok(Some(path)) => pending.push((path, depth + 1)),
                    Ok(None) => {}
                    Err(code) => diagnostics.push(Diagnostic::new(
                        DiagnosticKind::CoverageGap,
                        code,
                        &file,
                        line,
                        column,
                        "dependency",
                    )),
                }
            }
            for finding in facts.findings {
                let (behavior, required) = match finding.behavior {
                    Behavior::Environment(name) => {
                        if env.fixed.contains(&name) {
                            continue;
                        }
                        if let Some((source, None)) = env.mappings.get(&name)
                            && !declared.iter().any(|p| matches!(p, PermissionV1::Environment { name, .. } if name == source))
                        {
                            // Source identity is known even when sensitivity is not.
                            // A missing source declaration is definite; never guess
                            // sensitivity merely to construct a permission value.
                            let mut diagnostic = Diagnostic::new(
                                DiagnosticKind::Difference,
                                "code_analysis_declaration_difference",
                                &file, finding.line, finding.column, "environment",
                            );
                            diagnostic.identity = Some(sha256_hex(source.as_bytes()));
                            diagnostics.push(diagnostic);
                        }
                        match env.mappings.get(&name) {
                            Some((source, Some(sensitivity))) => (
                                "environment",
                                Some(PermissionV1::Environment {
                                    name: source.clone(),
                                    sensitivity: *sensitivity,
                                }),
                            ),
                            _ => ("environment", None),
                        }
                    }
                    Behavior::Command(program) => {
                        ("command", Some(PermissionV1::Command { program }))
                    }
                    Behavior::Network(host) => (
                        "network",
                        Some(PermissionV1::Network {
                            scope: crate::plan::NetworkScopeV1::Host(host),
                        }),
                    ),
                    Behavior::File(access, path) => ("filesystem", file_permission(access, &path)),
                };
                if let Some(required) = required {
                    let matched = declared
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| covers(p, &required))
                        .map(|(i, _)| i)
                        .collect::<Vec<_>>();
                    if matched.is_empty() {
                        let mut d = Diagnostic::new(
                            DiagnosticKind::Difference,
                            "code_analysis_declaration_difference",
                            &file,
                            finding.line,
                            finding.column,
                            behavior,
                        );
                        d.identity = Some(sha256_hex(
                            &serde_json::to_vec(&required)
                                .map_err(|_| TerminalFailure::Analyzer)?,
                        ));
                        diagnostics.push(d);
                    } else {
                        corroborated.extend(matched);
                    }
                } else {
                    diagnostics.push(Diagnostic::new(
                        DiagnosticKind::CoverageGap,
                        "code_analysis_identity_unknown",
                        &file,
                        finding.line,
                        finding.column,
                        behavior,
                    ));
                }
                self.findings += 1;
                self.budget()?;
            }
        }
        for (i, p) in declared.iter().enumerate() {
            if !corroborated.contains(&i)
                && matches!(
                    p,
                    PermissionV1::Command { .. }
                        | PermissionV1::Filesystem { .. }
                        | PermissionV1::Network { .. }
                        | PermissionV1::Environment { .. }
                )
            {
                diagnostics.push(Diagnostic::new(
                    DiagnosticKind::Uncorroborated,
                    "code_analysis_declaration_uncorroborated",
                    entry,
                    1,
                    1,
                    "declaration",
                ));
            }
        }
        self.findings += diagnostics.len();
        self.budget()?;
        diagnostics.sort();
        diagnostics.dedup();
        Ok(Report {
            analyzer_version: ANALYZER_VERSION.into(),
            schema_version: SCHEMA_VERSION,
            rule_version: RULE_VERSION,
            files: visited.into_iter().collect(),
            allowed: !diagnostics.iter().any(Diagnostic::blocking),
            diagnostics,
        })
    }
}

fn covers(declared: &PermissionV1, required: &PermissionV1) -> bool {
    declared == required
        || matches!(
            (declared, required),
            (
                PermissionV1::Network {
                    scope: crate::plan::NetworkScopeV1::Any
                },
                PermissionV1::Network { .. }
            )
        )
}

fn file_permission(access: FilesystemAccessV1, path: &str) -> Option<PermissionV1> {
    if path.is_empty() || path.contains(['\\', '\0']) || path.split('/').any(|p| p == "..") {
        return None;
    }
    let (base, path) = if path.starts_with('/') {
        ("absolute", path.to_string())
    } else if path.contains(':') {
        return None;
    } else {
        (
            "preset",
            path.strip_prefix("./").unwrap_or(path).to_string(),
        )
    };
    Some(PermissionV1::Filesystem {
        access,
        path: format!(
            "{base}:{}",
            crate::permission::normalize_declared_path(&path)
        ),
    })
}

fn resolve<S>(
    sources: &BTreeMap<String, S>,
    file: &str,
    spec: &str,
) -> Result<Option<String>, &'static str> {
    if syntax::builtin(spec).is_some() {
        return Ok(None);
    }
    if spec.starts_with("node:") {
        return Err("code_analysis_builtin_unsupported");
    }
    if !spec.starts_with("./") && !spec.starts_with("../") {
        return Err("code_analysis_external_dependency");
    }
    if spec.contains(['\\', '\0', '?', '#']) {
        return Err("code_analysis_dependency_unsupported");
    }
    let mut parts = file.split('/').collect::<Vec<_>>();
    parts.pop();
    for part in spec.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err("code_analysis_dependency_escape");
                }
            }
            _ => parts.push(part),
        }
    }
    let base = parts.join("/");
    if sources.contains_key(&base) {
        return Ok(Some(base));
    }
    let candidates = [
        ".ts",
        ".tsx",
        ".js",
        ".jsx",
        ".mjs",
        ".cjs",
        "/index.ts",
        "/index.js",
    ]
    .iter()
    .map(|suffix| format!("{base}{suffix}"))
    .filter(|p| sources.contains_key(p))
    .collect::<Vec<_>>();
    match candidates.as_slice() {
        [path] => Ok(Some(path.clone())),
        [] => Err("code_analysis_dependency_missing"),
        _ => Err("code_analysis_dependency_ambiguous"),
    }
}
