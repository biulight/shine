//! Local, session-scoped SSH secret broker authorization and decryption.

use crate::config::Config;
use crate::env::RESERVED_REMOTE_ENV;
use crate::env::broker::{self, PolicyStore, WorkspaceSnapshot};
use crate::env::{EnvConfig, parse_env_specs, secret_key};
use crate::secret;
use anyhow::{Context, Result, bail};
use shine_core::digest::sha256_hex;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
mod interaction;
#[cfg(test)]
use interaction::confirmation_approved;
use interaction::{BrokerInteraction, LocalConfirmation, TtyInteraction, raw_tty_safe_log};
#[cfg(test)]
mod tests;

const MAX_BROKER_REQUESTS_PER_SESSION: usize = 1024;

#[derive(Clone)]
struct DirectSecret {
    source: String,
    target: String,
    ciphertext: String,
    ciphertext_sha256: String,
}

pub struct BrokerSession {
    config: Config,
    ssh_target: String,
    policies: PolicyStore,
    direct: Vec<DirectSecret>,
    trust_remote_session: bool,
    mode: BrokerMode,
    enroll_update_policy: Option<String>,
    used_nonces: tokio::sync::Mutex<BTreeSet<String>>,
    operation_lock: tokio::sync::Mutex<()>,
    file_transport_lock: tokio::sync::Mutex<()>,
    interaction: Arc<dyn BrokerInteraction>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BrokerMode {
    Serve,
    Inspect,
    EnrollTrusted,
}

impl BrokerSession {
    #[allow(clippy::too_many_arguments)] // Session security switches stay explicit at construction.
    pub async fn prepare(
        config: &Config,
        ssh_target: &str,
        enabled: bool,
        policy_overrides: &[PathBuf],
        allow_secret: &[String],
        trust_remote_session: bool,
        inspect: bool,
        enroll: bool,
        trust_remote_metadata: bool,
        enroll_update_policy: Option<&str>,
    ) -> Result<Option<Arc<Self>>> {
        let active = enabled || inspect || enroll;
        if !active {
            if !allow_secret.is_empty() || trust_remote_session {
                bail!("--allow-secret/--trust-remote-session requires --secret-broker");
            }
            return Ok(None);
        }
        if enroll && !trust_remote_metadata {
            bail!(
                "--secret-broker-enroll requires the explicit --trust-remote-metadata acknowledgement"
            );
        }
        if enroll_update_policy.is_some() && !enroll {
            bail!("--update-policy requires --secret-broker-enroll");
        }
        if (inspect || enroll) && (!allow_secret.is_empty() || trust_remote_session) {
            bail!("inspect/enroll sessions cannot release secrets");
        }
        let mode = if inspect {
            BrokerMode::Inspect
        } else if enroll {
            BrokerMode::EnrollTrusted
        } else {
            BrokerMode::Serve
        };
        let env = EnvConfig::from_config(config);
        let mut direct = Vec::new();
        for spec in parse_env_specs(allow_secret)? {
            if spec.source.ends_with("_SECRET") {
                bail!(
                    "--allow-secret expects a base key without the _SECRET suffix: {}",
                    spec.source
                );
            }
            if RESERVED_REMOTE_ENV.contains(&spec.target.as_str()) {
                bail!("cannot override shine-managed SSH variable {}", spec.target);
            }
            let storage_key = secret_key(&spec.source);
            let ciphertext = env
                .get(&storage_key)
                .with_context(|| format!("{storage_key} is not set in the active config [env]"))?
                .to_string();
            let ciphertext_sha256 = sha256_hex(ciphertext.as_bytes());
            direct.push(DirectSecret {
                source: spec.source,
                target: spec.target,
                ciphertext,
                ciphertext_sha256,
            });
        }
        let policies = broker::load_stores(config, policy_overrides).await?;
        if trust_remote_session {
            eprintln!(
                "Warning: --trust-remote-session trusts the entire remote SSH session and same-account processes; workspace hashes do not authenticate the caller."
            );
        }
        let interaction = Arc::new(TtyInteraction::capture());
        Ok(Some(Arc::new(Self {
            config: config.clone(),
            ssh_target: ssh_target.to_string(),
            policies,
            direct,
            trust_remote_session,
            mode,
            enroll_update_policy: enroll_update_policy.map(str::to_string),
            used_nonces: tokio::sync::Mutex::new(BTreeSet::new()),
            operation_lock: tokio::sync::Mutex::new(()),
            file_transport_lock: tokio::sync::Mutex::new(()),
            interaction,
        })))
    }

    pub(super) async fn file_transport_guard(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.file_transport_lock.lock().await
    }

    pub fn set_ssh_pid(&self, pid: Option<u32>) {
        self.interaction.set_ssh_pid(pid);
    }

    async fn claim_nonce(&self, nonce: &str) -> Result<()> {
        broker::validate_wire_string(nonce, "nonce")?;
        if nonce.len() < 16 {
            bail!("broker request nonce is too short");
        }
        let mut nonces = self.used_nonces.lock().await;
        if nonces.len() >= MAX_BROKER_REQUESTS_PER_SESSION {
            bail!("SSH secret broker request limit reached for this session");
        }
        if !nonces.insert(nonce.to_string()) {
            bail!("broker request nonce has already been used");
        }
        Ok(())
    }

    pub async fn handle_direct(
        &self,
        specs: &[String],
        argv: &[String],
        nonce: &str,
    ) -> Result<BTreeMap<String, String>> {
        if self.mode != BrokerMode::Serve {
            bail!("this SSH session is inspect/enroll-only and cannot release secrets");
        }
        self.claim_nonce(nonce).await?;
        let _operation = self.operation_lock.lock().await;
        broker::validate_wire_strings(argv, "argv")?;
        let requested = parse_env_specs(specs)?;
        if requested.is_empty() {
            bail!("a direct broker request must name at least one secret");
        }
        let mut selected = Vec::new();
        for spec in requested {
            let item = self
                .direct
                .iter()
                .find(|item| item.source == spec.source && item.target == spec.target)
                .with_context(|| {
                    format!(
                        "{}={} is not authorized for this SSH session",
                        spec.source, spec.target
                    )
                })?;
            selected.push(item.clone());
        }
        let refs = selected
            .iter()
            .map(|item| {
                if item.source == item.target {
                    item.source.clone()
                } else {
                    format!("{}={}", item.source, item.target)
                }
            })
            .collect::<Vec<_>>();
        let _confirmation = self.confirm("direct", argv, &refs).await?;

        let mut values = BTreeMap::new();
        for item in selected {
            // The digest is frozen with the ciphertext at session start and is
            // included in the audit line without exposing the ciphertext.
            let value = secret::decrypt_with_config(&item.ciphertext, &self.config)
                .await
                .with_context(|| format!("decrypting {}_SECRET", item.source))?;
            values.insert(item.target, value);
            eprintln!(
                "shine ssh broker: released direct secret {} ciphertext={}…",
                item.source,
                &item.ciphertext_sha256[..12]
            );
        }
        Ok(values)
    }

    pub async fn handle_workspace(
        &self,
        snapshot: &WorkspaceSnapshot,
        argv: &[String],
        nonce: &str,
    ) -> Result<BTreeMap<String, String>> {
        if self.mode != BrokerMode::Serve {
            bail!("this SSH session is inspect/enroll-only and cannot release secrets");
        }
        self.claim_nonce(nonce).await?;
        let _operation = self.operation_lock.lock().await;
        let matched =
            broker::match_workspace_request(&self.policies, &self.ssh_target, snapshot, argv)?;
        let _confirmation = if self.trust_remote_session {
            None
        } else {
            Some(
                self.confirm(&matched.policy_name, argv, &matched.release)
                    .await?,
            )
        };
        let values =
            broker::decrypt_workspace_snapshot(&self.config, snapshot, &matched.release).await?;
        let argv_sha256 = sha256_hex(argv.join("\0").as_bytes());
        let audit = format!(
            "shine ssh broker: released policy {} project={} argv_sha256={} approval={}",
            matched.policy_name,
            sanitize_label(&matched.project),
            argv_sha256,
            if self.trust_remote_session {
                "trusted-session"
            } else {
                "interactive"
            }
        );
        if self.trust_remote_session {
            raw_tty_safe_log(&audit);
        } else {
            eprintln!("{audit}");
        }
        Ok(values)
    }

    pub async fn handle_description(
        &self,
        snapshot: &WorkspaceSnapshot,
        release: &[String],
        argv: &[String],
        nonce: &str,
    ) -> Result<String> {
        self.claim_nonce(nonce).await?;
        let _operation = self.operation_lock.lock().await;
        if self.mode == BrokerMode::Serve {
            bail!("this SSH session is not in inspect/enroll mode");
        }
        broker::validate_snapshot(snapshot)?;
        let allow = broker::allow_from_snapshot(snapshot, release, argv)?;
        let argv_sha256 = sha256_hex(argv.join("\0").as_bytes());
        let summary = format!(
            "workspace={} mode={} argv_sha256={} sources={} release={}",
            broker::sha256(snapshot.workspace_contents.as_bytes()),
            sanitize_label(&snapshot.mode),
            argv_sha256,
            allow.sources.len(),
            release
                .iter()
                .map(|value| sanitize_label(value))
                .collect::<Vec<_>>()
                .join(",")
        );
        let enrollment_plan = if self.mode == BrokerMode::EnrollTrusted {
            Some(broker::plan_remote_enrollment(
                &self.policies,
                &self.ssh_target,
                snapshot,
                release,
                argv,
                self.enroll_update_policy.as_deref(),
            )?)
        } else {
            None
        };
        let policy_comparison = if let Some(plan) = &enrollment_plan {
            plan.action_label()
        } else {
            match broker::match_workspace_request(&self.policies, &self.ssh_target, snapshot, argv)
            {
                Ok(matched) if matched.release == release => {
                    format!(
                        "exact local policy match: {}",
                        sanitize_label(&matched.policy_name)
                    )
                }
                Ok(matched) => format!(
                    "selector matches local policy {}, but release differs (local: {})",
                    sanitize_label(&matched.policy_name),
                    matched
                        .release
                        .iter()
                        .map(|value| sanitize_label(value))
                        .collect::<Vec<_>>()
                        .join(",")
                ),
                Err(_) => "no exact local policy selector match".to_string(),
            }
        };
        let details =
            format_description_details(&self.ssh_target, snapshot, &allow, &policy_comparison);
        match self.mode {
            BrokerMode::Inspect => {
                self.display_local(format!("Shine secret broker inspection\n\n{details}\n"))
                    .await?;
                Ok(format!("inspected remote broker request: {summary}"))
            }
            BrokerMode::EnrollTrusted => {
                let plan = enrollment_plan.expect("enrollment mode has a prepared plan");
                let diff = plan
                    .diff()?
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| format!("\n\nPolicy diff:\n{}", escape_multiline(&value)))
                    .unwrap_or_default();
                let _confirmation = self
                    .confirm_prompt(format!(
                        "Shine secret broker trusted remote enrollment\n\n{details}{diff}\n\nApprove enrollment? [y/N] "
                    ))
                    .await?;
                let updated = plan.previous.is_some();
                let name = broker::apply_remote_enrollment(&self.config, &plan).await?;
                Ok(if updated {
                    format!("updated local SSH secret broker policy {name}")
                } else {
                    format!("enrolled local SSH secret broker policy {name}")
                })
            }
            BrokerMode::Serve => unreachable!(),
        }
    }

    async fn confirm(
        &self,
        policy: &str,
        argv: &[String],
        refs: &[String],
    ) -> Result<LocalConfirmation> {
        let policy = sanitize_label(policy);
        let target = sanitize_label(&self.ssh_target);
        let argv = argv
            .iter()
            .map(|value| escape_display(value))
            .collect::<Vec<_>>();
        let refs = refs
            .iter()
            .map(|value| escape_display(value))
            .collect::<Vec<_>>();
        let prompt = format!(
            "Shine secret request\n\nSSH target: {target}\nPolicy: {policy}\nCommand argv:\n{}\nSecrets ({}):\n{}\n\nApprove? [y/N] ",
            format_indexed(&argv),
            refs.len(),
            format_bullets(&refs),
        );
        self.confirm_prompt(prompt).await
    }

    async fn confirm_prompt(&self, prompt: String) -> Result<LocalConfirmation> {
        self.interaction.confirm_prompt(prompt).await
    }
    async fn display_local(&self, contents: String) -> Result<()> {
        self.interaction.display_local(contents).await
    }
}

fn format_description_details(
    ssh_target: &str,
    snapshot: &WorkspaceSnapshot,
    allow: &broker::BrokerAllow,
    policy_comparison: &str,
) -> String {
    let release = allow
        .release
        .iter()
        .map(|value| sanitize_label(value))
        .collect::<Vec<_>>();
    let argv = allow
        .argv
        .iter()
        .map(|value| escape_display(value))
        .collect::<Vec<_>>();
    let mut details = format!(
        "SSH target: {}\nWorkspace: {}\nMode: {}\nCommand argv:\n{}\nRelease secrets ({}):\n{}\nSources ({}):",
        sanitize_label(ssh_target),
        sanitize_label(&snapshot.workspace_path),
        sanitize_label(&snapshot.mode),
        format_indexed(&argv),
        release.len(),
        format_bullets(&release),
        allow.sources.len(),
    );
    for source in &allow.sources {
        let declared = source
            .declared_secrets
            .iter()
            .map(|value| sanitize_label(value))
            .collect::<Vec<_>>();
        details.push_str(&format!(
            "\n  - Path: {}\n    SHA-256: {}\n    Declared secrets ({}):\n{}",
            sanitize_label(&source.path),
            source.sha256,
            declared.len(),
            indent_lines(&format_bullets(&declared), 4),
        ));
    }
    details.push_str(&format!(
        "\nPolicy comparison: {}",
        sanitize_label(policy_comparison)
    ));
    details
}

fn format_indexed(values: &[String]) -> String {
    if values.is_empty() {
        return "  (none)".to_string();
    }
    values
        .iter()
        .enumerate()
        .map(|(index, value)| format!("  [{index}] {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_bullets(values: &[String]) -> String {
    if values.is_empty() {
        return "  (none)".to_string();
    }
    values
        .iter()
        .map(|value| format!("  - {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn indent_lines(value: &str, spaces: usize) -> String {
    let prefix = " ".repeat(spaces);
    value
        .lines()
        .map(|line| format!("{prefix}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn sanitize_label(value: &str) -> String {
    escape_display(value).chars().take(256).collect()
}

fn escape_display(value: &str) -> String {
    escaped_display_chars(value).take(4096).collect()
}

fn escaped_display_chars(value: &str) -> impl Iterator<Item = char> + '_ {
    value.chars().flat_map(|ch| match ch {
        '\\' => "\\\\".chars().collect::<Vec<_>>(),
        '\n' => "\\n".chars().collect(),
        '\r' => "\\r".chars().collect(),
        '\t' => "\\t".chars().collect(),
        ch if ch.is_control() => format!("\\u{{{:x}}}", ch as u32).chars().collect(),
        ch => vec![ch],
    })
}

fn escape_multiline(value: &str) -> String {
    value
        .split('\n')
        .map(|line| escaped_display_chars(line).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) enum FileResult {
    Description(String),
    Plain(Vec<zeroize::Zeroizing<Vec<u8>>>),
}

impl BrokerSession {
    pub(super) async fn handle_files(
        &self,
        request: &super::file_broker::FileReleaseRequest,
        describe: bool,
        nonce: &str,
    ) -> Result<FileResult> {
        use super::file_broker::{FilePolicy, descriptor};
        self.claim_nonce(nonce).await?;
        let _operation = self.operation_lock.lock().await;
        let descriptor = descriptor(request)?;
        let matches = self
            .policies
            .file_policies
            .iter()
            .filter(|p| {
                p.ssh_target == self.ssh_target
                    && super::file_broker::policy_matches(&p.descriptor, &descriptor)
            })
            .collect::<Vec<_>>();
        let details = format!(
            "SSH target: {}\nFile release:\n{}",
            escape_display(&self.ssh_target),
            escape_multiline(&serde_json::to_string_pretty(&request.descriptor()?)?)
        );
        if describe {
            match self.mode {
                BrokerMode::Serve => bail!("file descriptions require an inspect/enroll session"),
                BrokerMode::Inspect => {
                    self.display_local(format!(
                        "{details}\nExact policy matches: {}",
                        matches.len()
                    ))
                    .await?;
                    return Ok(FileResult::Description(
                        "file release inspected; no plaintext released".into(),
                    ));
                }
                BrokerMode::EnrollTrusted => {
                    let name = self.enroll_update_policy.clone().unwrap_or_else(|| {
                        format!("files-{}", &sha256_hex(descriptor.as_bytes())[..16])
                    });
                    let candidate = FilePolicy {
                        name: name.clone(),
                        ssh_target: self.ssh_target.clone(),
                        descriptor,
                    };
                    let previous = self
                        .policies
                        .file_policies
                        .iter()
                        .find(|p| p.name == name)
                        .cloned();
                    if self.enroll_update_policy.is_some() && previous.is_none() {
                        bail!("file policy to update does not exist");
                    }
                    let old = previous
                        .as_ref()
                        .map(|p| p.descriptor.as_str())
                        .unwrap_or("(new policy)");
                    let _confirmation = self.confirm_prompt(format!("{details}\nPrevious binding: {}\nMigrate file policies to version 2 and save this exact request? [y/N] ", escape_multiline(old))).await?;
                    let path = self.config.shine_dir().join("ssh-secret-broker.lock");
                    use shine_core::runtime::FileSystemHost;
                    let _guard = shine_core::runtime::RealHost
                        .acquire_operation_lock(&path)
                        .await?;
                    let mut store = broker::load_store(&self.config).await?;
                    if store.file_policies.iter().find(|p| p.name == name) != previous.as_ref() {
                        bail!("file policy changed after review");
                    }
                    store.version = 2;
                    store.file_policies.retain(|p| p.name != name);
                    store.file_policies.push(candidate);
                    broker::store::save_store(&self.config, &store).await?;
                    return Ok(FileResult::Description(format!(
                        "saved file policy {name}; reconnect to release files"
                    )));
                }
            }
        }
        if self.mode != BrokerMode::Serve {
            bail!("inspect/enroll sessions cannot release files");
        }
        if matches.len() != 1 {
            bail!(
                "file request requires exactly one matching local file policy; inspect/enroll the request first"
            );
        }
        let _confirmation = if self.trust_remote_session {
            None
        } else {
            Some(
                self.confirm_prompt(format!(
                    "{details}\nRelease these files to the remote machine? [y/N] "
                ))
                .await?,
            )
        };
        Ok(FileResult::Plain(
            super::file_broker::decrypt(&self.config, request).await?,
        ))
    }
}
