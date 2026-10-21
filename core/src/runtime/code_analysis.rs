//! Core-owned evidence preparation. Planning and locked execution only consume it.

use super::{AppCategory, ArtifactRuntime, CoreRuntime};
use crate::analysis::{
    self, Binding, Diagnostic, DiagnosticKind, EnvironmentContract, Report, Scope, Session,
    TerminalFailure,
};
use crate::env::EnvVarSpec;
use crate::permission::PermissionDeclarationV1;
use anyhow::{Context, Result, ensure};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

#[cfg(test)]
mod tests;

#[derive(Clone)]
pub(crate) struct CodeAnalysisSession(
    Arc<Mutex<State>>,
    Arc<std::sync::atomic::AtomicBool>,
    Arc<tokio::sync::Notify>,
);

impl Default for CodeAnalysisSession {
    fn default() -> Self {
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        Self(
            Arc::new(Mutex::new(State {
                sources: Session::new(analysis::Limits::default(), flag.clone()),
                ..State::default()
            })),
            flag,
            Arc::new(tokio::sync::Notify::new()),
        )
    }
}

struct CancelPreparation {
    session: CodeAnalysisSession,
    armed: bool,
}
impl Drop for CancelPreparation {
    fn drop(&mut self) {
        if self.armed {
            self.session.cancel();
        }
    }
}

impl std::fmt::Debug for CodeAnalysisSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CodeAnalysisSession { .. }")
    }
}

impl CodeAnalysisSession {
    pub(super) fn cancel(&self) {
        self.1.store(true, std::sync::atomic::Ordering::Release);
        self.2.notify_waiters();
    }
    pub(super) async fn cancelled(&self) {
        let notified = self.2.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.1.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        notified.await;
    }

    pub(super) fn analyzed_entries(&self) -> Result<Vec<String>> {
        let state = self.0.lock().map_err(|_| TerminalFailure::Analyzer)?;
        Ok(state
            .evidence
            .values()
            .map(|b| {
                format!(
                    "{}:{}:{}",
                    b.scope.target,
                    b.scope.entry,
                    b.scope.script.as_deref().unwrap_or("native-command")
                )
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect())
    }
}

#[derive(Default)]
struct State {
    sources: Session,
    pending: BTreeMap<String, Entry>,
    evidence: BTreeMap<String, Binding>,
}

#[derive(Clone)]
struct Entry {
    category: String,
    scope: Scope,
    supported: bool,
    declaration: Option<PermissionDeclarationV1>,
    environment: EnvironmentContract,
}

impl<H> CoreRuntime<H> {
    pub(super) fn generator_analysis(
        &self,
        category: &AppCategory,
        source: &str,
        generator: &super::AppGenerator,
    ) -> Result<Binding> {
        let script = generator
            .script
            .to_str()
            .context("App script path is not UTF-8")?
            .replace('\\', "/");
        self.app_analysis_binding(
            category,
            &format!("generator:{source}"),
            Some(&script),
            generator.runtime,
            &generator.env,
        )
    }

    pub(super) fn require_code_analysis(&self, binding: &Binding) -> Result<()> {
        let mut state = self
            .analysis
            .0
            .lock()
            .map_err(|_| TerminalFailure::Analyzer)?;
        state.sources.check()?;
        ensure!(
            state.evidence.get(&binding.identity) == Some(binding),
            "code_analysis_evidence_missing"
        );
        ensure!(
            binding.scope.report.allowed,
            "code_analysis_blocked: correct the declarations or script at the reported locations"
        );
        Ok(())
    }

    pub(super) fn require_app_script_analysis(
        &self,
        category: &str,
        entry: &str,
        script: &str,
        runtime: ArtifactRuntime,
        specs: &[EnvVarSpec],
    ) -> Result<()> {
        let category = self
            .app_categories(Some(category))?
            .into_iter()
            .next()
            .context("App analysis category is missing")?;
        let binding = self.app_analysis_binding(&category, entry, Some(script), runtime, specs)?;
        self.require_code_analysis(&binding)
    }

    pub(super) fn check_analysis_command(&self) -> Result<()> {
        self.analysis
            .0
            .lock()
            .map_err(|_| TerminalFailure::Analyzer)?
            .sources
            .check()?;
        Ok(())
    }
    /// Lint deliberately examines every typed entry, independent of run inputs.
    pub(super) async fn lint_app_code(&self, category: &AppCategory) -> Result<Vec<Scope>> {
        let mut bindings = Vec::new();
        for file in &category.files {
            if let Some(generator) = &file.generator {
                let script = generator
                    .script
                    .to_str()
                    .ok_or(TerminalFailure::Analyzer)?
                    .replace('\\', "/");
                bindings.push(self.app_analysis_binding(
                    category,
                    &format!(
                        "generator:{}",
                        file.source_rel.to_str().ok_or(TerminalFailure::Analyzer)?
                    ),
                    Some(&script),
                    generator.runtime,
                    &generator.env,
                )?);
            }
        }
        for (index, hook) in category
            .post_install
            .iter()
            .enumerate()
            .chain(category.post_upgrade.iter().enumerate())
        {
            let (script, runtime) = match &hook.action {
                super::AppHookAction::Script { script, runtime } => (
                    Some(
                        script
                            .to_str()
                            .ok_or(TerminalFailure::Analyzer)?
                            .replace('\\', "/"),
                    ),
                    *runtime,
                ),
                super::AppHookAction::Command(_) => (None, ArtifactRuntime::Native),
            };
            bindings.push(self.app_analysis_binding(
                category,
                &format!("hook:{index}"),
                script.as_deref(),
                runtime,
                &hook.env,
            )?);
        }
        if let Some(artifact) = &category.artifact {
            for script in
                std::iter::once(artifact.script.as_str()).chain(artifact.teardown.as_deref())
            {
                bindings.push(self.app_analysis_binding(
                    category,
                    "artifact",
                    Some(script),
                    artifact.runtime,
                    &artifact.env,
                )?);
            }
        }
        self.prepare_code_analysis(&bindings).await?;
        let state = self
            .analysis
            .0
            .lock()
            .map_err(|_| TerminalFailure::Analyzer)?;
        bindings
            .into_iter()
            .map(|b| {
                state
                    .evidence
                    .get(&b.identity)
                    .map(|b| b.scope.clone())
                    .ok_or_else(|| TerminalFailure::Analyzer.into())
            })
            .collect()
    }
    /// Capture names and identities only. Fixed keys follow the actual launcher,
    /// including the absent-overlay empty override, without exposing values.
    fn analysis_environment(
        &self,
        category: &AppCategory,
        specs: &[EnvVarSpec],
    ) -> EnvironmentContract {
        let mut fixed = self
            .fixed_app_contract_env(&category.name, Path::new("source"))
            .into_keys()
            .collect::<std::collections::BTreeSet<_>>();
        fixed.insert("SHINE_APP_OVERLAY_DIR".into());
        let mappings = specs
            .iter()
            .map(|spec| {
                let sensitivity = category
                    .permissions
                    .as_ref()
                    .and_then(|p| p.environment.iter().find(|e| e.name == spec.source))
                    .map(|e| e.sensitivity);
                (spec.target.clone(), (spec.source.clone(), sensitivity))
            })
            .collect();
        EnvironmentContract { fixed, mappings }
    }

    /// Called only after the domain has selected a potentially triggered entry.
    /// Registers missing work but does not run the parser or observe live state.
    pub(super) fn app_analysis_binding(
        &self,
        category: &AppCategory,
        entry: &str,
        script: Option<&str>,
        runtime: ArtifactRuntime,
        specs: &[EnvVarSpec],
    ) -> Result<Binding> {
        let source = if let Some(source) = self.analysis_source.get() {
            *source
        } else {
            let source = self.presets().digest_v1()?;
            let _ = self.analysis_source.set(source);
            source
        };
        let environment = self.analysis_environment(category, specs);
        let declaration = category
            .permissions
            .as_ref()
            .map(|d| d.permission_set())
            .transpose()?;
        let identity = crate::digest::sha256_hex(&serde_json::to_vec(&(
            "shine.code-analysis.v1",
            analysis::ANALYZER_VERSION,
            analysis::RULE_VERSION,
            analysis::SCHEMA_VERSION,
            analysis::Limits::default(),
            source.as_hex(),
            &category.name,
            entry,
            script,
            runtime == ArtifactRuntime::Bun,
            declaration,
            &environment,
        ))?);
        let mut state = self
            .analysis
            .0
            .lock()
            .map_err(|_| TerminalFailure::Analyzer)?;
        state.sources.check()?;
        if let Some(binding) = state.evidence.get(&identity) {
            return Ok(binding.clone());
        }
        if !state.pending.contains_key(&identity) {
            state.sources.charge_diagnostics(1)?;
        }
        let scope = Scope {
            target: format!("app/{}", category.name),
            entry: entry.into(),
            script: script.map(str::to_string),
            report: Report {
                analyzer_version: analysis::ANALYZER_VERSION.into(),
                schema_version: analysis::SCHEMA_VERSION,
                rule_version: analysis::RULE_VERSION,
                files: Vec::new(),
                allowed: false,
                diagnostics: vec![Diagnostic::new(
                    DiagnosticKind::Failure,
                    "code_analysis_evidence_missing",
                    script.unwrap_or(""),
                    1,
                    1,
                    "evidence",
                )],
            },
        };
        state
            .pending
            .entry(identity.clone())
            .or_insert_with(|| Entry {
                category: category.name.clone(),
                scope: scope.clone(),
                supported: runtime == ArtifactRuntime::Bun && script.is_some(),
                declaration: category.permissions.clone(),
                environment,
            });
        Ok(Binding { identity, scope })
    }

    /// Run only in orchestration outside lifecycle locks. Scope comes from a
    /// just-produced Core plan, not from a frontend-supplied report.
    pub(super) async fn prepare_code_analysis(&self, requested: &[Binding]) -> Result<bool> {
        {
            let mut state = self
                .analysis
                .0
                .lock()
                .map_err(|_| TerminalFailure::Analyzer)?;
            state.sources.check()?;
            if requested
                .iter()
                .all(|b| state.evidence.contains_key(&b.identity))
            {
                return Ok(false);
            }
        }
        let mut cancel = CancelPreparation {
            session: self.analysis.clone(),
            armed: true,
        };
        let session = self.analysis.clone();
        let snapshot = self.presets.clone();
        let requested = requested.to_vec();
        let result =
            tokio::task::spawn_blocking(move || prepare(&session, &snapshot, &requested)).await;
        cancel.armed = false;
        match result {
            Ok(result) => result,
            Err(_) => {
                self.analysis.cancel();
                Err(TerminalFailure::Analyzer.into())
            }
        }
    }
}

fn prepare(
    session: &CodeAnalysisSession,
    snapshot: &super::PresetSnapshot,
    requested: &[Binding],
) -> Result<bool> {
    let mut state = session.0.lock().map_err(|_| TerminalFailure::Analyzer)?;
    state.sources.check()?;
    let mut prepared = false;
    for binding in requested {
        state.sources.check()?;
        if state.evidence.contains_key(&binding.identity) {
            continue;
        }
        let entry = state
            .pending
            .get(&binding.identity)
            .cloned()
            .ok_or(TerminalFailure::Analyzer)?;
        let report = if entry.supported {
            let prefix = format!("app/{}/", entry.category);
            let sources = snapshot
                .files()
                .iter()
                .filter_map(|(path, bytes)| {
                    path.strip_prefix(&prefix)
                        .map(|relative| (relative.to_string(), bytes.as_slice()))
                })
                .collect();
            state.sources.analyze(
                &sources,
                entry.scope.script.as_deref().expect("supported entry"),
                entry.declaration.as_ref(),
                &entry.environment,
            )?
        } else {
            state.sources.charge_diagnostics(1)?;
            Report {
                analyzer_version: analysis::ANALYZER_VERSION.into(),
                schema_version: analysis::SCHEMA_VERSION,
                rule_version: analysis::RULE_VERSION,
                files: Vec::new(),
                allowed: true,
                diagnostics: vec![Diagnostic::new(
                    DiagnosticKind::CoverageGap,
                    "code_analysis_language_unsupported",
                    entry.scope.script.as_deref().unwrap_or(""),
                    1,
                    1,
                    "code",
                )],
            }
        };
        state.evidence.insert(
            binding.identity.clone(),
            Binding {
                identity: binding.identity.clone(),
                scope: Scope {
                    report,
                    ..entry.scope
                },
            },
        );
        state.pending.remove(&binding.identity);
        prepared = true;
    }
    Ok(prepared)
}
