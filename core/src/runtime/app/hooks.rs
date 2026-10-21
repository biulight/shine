//! Post-lifecycle hooks with non-fatal failures and ordered notes.

use super::*;

impl<H: FileSystemHost + ProcessHost> CoreRuntime<H> {
    pub(in crate::runtime) async fn run_app_hooks(
        &self,
        request: AppHookRequest,
        observer: &mut impl RuntimeObserver,
    ) -> AppHookReport {
        let categories = request
            .categories
            .into_iter()
            .map(|category| (category.name.clone(), category))
            .collect::<BTreeMap<_, _>>();
        let mut report = AppHookReport::default();
        for category_name in request.changed {
            if self.check_analysis_command().is_err() {
                return report;
            }
            let Some(category) = categories.get(&category_name) else {
                continue;
            };
            let hooks = match request.phase {
                AppHookPhase::PostInstall => &category.post_install,
                AppHookPhase::PostUpgrade => &category.post_upgrade,
            };
            if hooks.is_empty() {
                continue;
            }
            let resource = match request.phase {
                AppHookPhase::PostInstall => "hook:post-install",
                AppHookPhase::PostUpgrade => "hook:post-upgrade",
            };
            if let Err(error) = self.ensure_app_code_allowed(&category_name, resource) {
                observer.emit(RuntimeEvent::Warning {
                    code: "app_hook_permission_required",
                    target: Some(format!("app/{category_name}")),
                    detail: error.to_string(),
                });
                report.outcomes.push(
                    LifecycleOutcomeV1::new(
                        format!("app/{category_name}"),
                        Some(resource),
                        LifecycleStatus::Skipped,
                        [],
                    )
                    .with_diagnostic_code("app_hook_permission_required"),
                );
                continue;
            }
            let mut completed = true;
            let mut notes = Vec::new();
            for (index, hook) in hooks.iter().enumerate() {
                let mut env: BTreeMap<String, String> = hook
                    .env
                    .iter()
                    .filter_map(|spec| {
                        self.context
                            .env
                            .get(&spec.source)
                            .map(|value| (spec.target.clone(), value.clone()))
                    })
                    .collect();
                let label = match &hook.action {
                    AppHookAction::Command(command) => command.clone(),
                    AppHookAction::Script { script, .. } => script.display().to_string(),
                };
                let process = ProcessRequest {
                    args: hook.args.clone(),
                    env: std::mem::take(&mut env),
                    ..ProcessRequest::default()
                };
                let output = match &hook.action {
                    AppHookAction::Command(command) => {
                        let admission = self
                            .app_analysis_binding(
                                category,
                                &format!("hook:{index}"),
                                None,
                                ArtifactRuntime::Native,
                                &hook.env,
                            )
                            .and_then(|binding| self.require_code_analysis(&binding));
                        if let Err(error) = admission {
                            report.outcomes.push(
                                LifecycleOutcomeV1::new(
                                    format!("app/{category_name}"),
                                    Some(resource),
                                    LifecycleStatus::Failed,
                                    [],
                                )
                                .with_diagnostic_code("code_analysis_blocked"),
                            );
                            observer.emit(RuntimeEvent::Warning {
                                code: "code_analysis_blocked",
                                target: Some(format!("app/{category_name}")),
                                detail: error.to_string(),
                            });
                            return report;
                        }
                        self.run_app_process(ProcessRequest {
                            program: command.clone(),
                            ..process
                        })
                        .await
                    }
                    AppHookAction::Script { script, runtime } => {
                        self.run_app_script(
                            &category_name,
                            &format!("app/{}/{}", category_name, script.display()),
                            *runtime,
                            &format!("hook:{index}"),
                            &hook.env,
                            process,
                        )
                        .await
                    }
                };
                match output {
                    Ok(output) if output.exit_code == Some(0) => {
                        if request.show_success && hook.show_output {
                            let note = String::from_utf8_lossy(&output.stdout).trim().to_string();
                            if !note.is_empty() {
                                notes.push(note);
                            }
                        }
                    }
                    Ok(output) => {
                        observer.emit(RuntimeEvent::Warning {
                            code: "app_hook_failed",
                            target: Some(format!("app/{category_name}")),
                            detail: format!(
                                "{} exited with {}{}",
                                label,
                                display_exit_code(output.exit_code),
                                process_detail(&output)
                            ),
                        });
                        completed = false;
                        break;
                    }
                    Err(error) => {
                        observer.emit(RuntimeEvent::Warning {
                            code: "app_hook_failed",
                            target: Some(format!("app/{category_name}")),
                            detail: format!("{label}: {error}"),
                        });
                        completed = false;
                        break;
                    }
                }
            }
            if request.show_success && completed {
                observer.emit(RuntimeEvent::Progress {
                    code: "app_hook_completed",
                    target: format!("app/{category_name}"),
                });
            }
            for note in &notes {
                observer.emit(RuntimeEvent::ProcessOutput {
                    code: "app_hook_note",
                    target: format!("app/{category_name}"),
                    stream: "stdout",
                    text: note.clone(),
                });
            }
            report.notes.extend(notes);
            report.outcomes.push(if completed {
                LifecycleOutcomeV1::new(
                    format!("app/{category_name}"),
                    Some(resource),
                    LifecycleStatus::Changed,
                    [LifecycleEffect::CodeExecuted],
                )
            } else {
                LifecycleOutcomeV1::new(
                    format!("app/{category_name}"),
                    Some(resource),
                    LifecycleStatus::Failed,
                    [],
                )
                .with_diagnostic_code("app_hook_failed")
            });
        }
        report
    }
}

fn process_detail(output: &crate::runtime::ProcessOutput) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    if detail.is_empty() {
        String::new()
    } else {
        format!(": {detail}")
    }
}
