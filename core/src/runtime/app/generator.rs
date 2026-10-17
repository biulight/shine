//! Generator gating, bounded snapshot execution and redacted failure diagnostics.

use super::*;

use std::time::Duration;

const GENERATOR_TIMEOUT: Duration = Duration::from_secs(30);
const GENERATOR_STDOUT_LIMIT: usize = 8 * 1024 * 1024;
const GENERATOR_STDERR_LIMIT: usize = 64 * 1024;

impl<H: FileSystemHost + ProcessHost> CoreRuntime<H> {
    pub(in crate::runtime) async fn run_app_generator(
        &self,
        request: AppGeneratorRequest,
        observer: &mut impl RuntimeObserver,
    ) -> Result<Option<Vec<u8>>> {
        if !self.context.env.contains_key(&request.generator.when_env) {
            return Ok(None);
        }
        if !request.explicit && !request.generator.auto {
            return Ok(None);
        }
        self.ensure_app_code_allowed(&request.category, "generator")?;
        let logical = format!(
            "app/{}/{}",
            request.category,
            request.generator.script.display()
        );
        let mut env = BTreeMap::new();
        for spec in &request.generator.env {
            let value = self.context.env.get(&spec.source).ok_or_else(|| {
                anyhow::anyhow!(
                    "app '{}' generator requires config env '{}'",
                    request.category,
                    spec.source
                )
            })?;
            env.insert(spec.target.clone(), value.clone());
        }
        let output = self
            .run_app_script(
                &request.category,
                &logical,
                request.generator.runtime,
                ProcessRequest {
                    env,
                    timeout: Some(GENERATOR_TIMEOUT),
                    stdout_limit: Some(GENERATOR_STDOUT_LIMIT),
                    stderr_limit: Some(GENERATOR_STDERR_LIMIT),
                    ..ProcessRequest::default()
                },
            )
            .await
            .with_context(|| format!("running app '{}' generator", request.category))?;
        if output.exit_code != Some(0) {
            bail!(generator_failure_message(
                &request.category,
                output.exit_code,
                &output.stderr
            ));
        }
        let content = String::from_utf8(output.stdout)
            .with_context(|| format!("app '{}' generator output is not UTF-8", request.category))?;
        let note = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !note.is_empty() {
            observer.emit(RuntimeEvent::ProcessOutput {
                code: "app_generator_note",
                target: format!("app/{}", request.category),
                stream: "stderr",
                text: note,
            });
        }
        Ok(Some(content.into_bytes()))
    }
}

pub(super) fn generator_failure_message(
    category: &str,
    exit_code: Option<i32>,
    stderr: &[u8],
) -> String {
    let detail = generator_failure_detail(stderr).unwrap_or_else(|| "details redacted".to_string());
    format!(
        "app '{category}' generator exited with {} ({detail})",
        display_exit_code(exit_code)
    )
}

pub(super) fn generator_failure_detail(stderr: &[u8]) -> Option<String> {
    const PREFIX: &str = "shine-generator-diagnostic-v1:";
    let line = std::str::from_utf8(stderr).ok()?;
    let line = line.strip_suffix('\n').unwrap_or(line);
    if line.contains(['\n', '\r']) {
        return None;
    }
    let diagnostic = line.strip_prefix(PREFIX)?;
    let detail = match diagnostic {
        "missing-input" => "required generator input is missing",
        "invalid-url" => "configured URL is invalid",
        "https-required" => "configured URL must use HTTPS",
        "request-timeout" => "remote request timed out",
        "request-failed" => "remote request failed before receiving a response",
        "https-redirect-required" => "remote redirect left HTTPS",
        "response-too-large" => "remote response exceeded the size limit",
        "surge-configuration-response" => {
            "remote server returned a complete Surge configuration; use a URI/Base64 subscription URL"
        }
        "no-compatible-nodes" => "response contained no compatible proxy nodes",
        "invalid-subscription" => "subscription response could not be decoded",
        "generation-failed" => "generation failed; details redacted",
        _ => {
            let status = diagnostic
                .strip_prefix("http-status:")?
                .parse::<u16>()
                .ok()?;
            if !(100..=599).contains(&status) {
                return None;
            }
            return Some(format!("remote server returned HTTP {status}"));
        }
    };
    Some(detail.to_string())
}
