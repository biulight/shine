//! Local, session-scoped SSH secret broker authorization and decryption.

use crate::config::Config;
use crate::env::broker::{self, PolicyStore, WorkspaceSnapshot};
use crate::env::{EnvConfig, parse_env_specs, secret_key};
use crate::secret;
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const RESERVED_TARGETS: &[&str] = &[
    "SHINE_SSH_SESSION",
    "SHINE_SSH_TOKEN",
    "SHINE_SSH_REMOTE_SOCK",
    "SHINE_TERMINAL_THEME",
];
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
    used_nonces: tokio::sync::Mutex<BTreeSet<String>>,
    operation_lock: tokio::sync::Mutex<()>,
    ssh_pid: Mutex<Option<u32>>,
    #[cfg(unix)]
    original_termios: Option<libc::termios>,
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
        let env = EnvConfig::load_or_init(config).await?;
        let mut direct = Vec::new();
        for spec in parse_env_specs(allow_secret)? {
            if spec.source.ends_with("_SECRET") {
                bail!(
                    "--allow-secret expects a base key without the _SECRET suffix: {}",
                    spec.source
                );
            }
            if RESERVED_TARGETS.contains(&spec.target.as_str()) {
                bail!("cannot override shine-managed SSH variable {}", spec.target);
            }
            let storage_key = secret_key(&spec.source);
            let ciphertext = env
                .get(&storage_key)
                .with_context(|| format!("{storage_key} is not set in the active config [env]"))?
                .to_string();
            let ciphertext_sha256 = format!("{:x}", Sha256::digest(ciphertext.as_bytes()));
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
        #[cfg(unix)]
        let original_termios = capture_tty_termios();
        Ok(Some(Arc::new(Self {
            config: config.clone(),
            ssh_target: ssh_target.to_string(),
            policies,
            direct,
            trust_remote_session,
            mode,
            used_nonces: tokio::sync::Mutex::new(BTreeSet::new()),
            operation_lock: tokio::sync::Mutex::new(()),
            ssh_pid: Mutex::new(None),
            #[cfg(unix)]
            original_termios,
        })))
    }

    pub fn set_ssh_pid(&self, pid: Option<u32>) {
        *self.ssh_pid.lock().expect("ssh pid mutex poisoned") = pid;
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
            let value = secret::decrypt_secret(&item.ciphertext, &self.config.age_identities())
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
        let argv_sha256 = format!("{:x}", Sha256::digest(argv.join("\0").as_bytes()));
        eprintln!(
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
        let argv_sha256 = format!("{:x}", Sha256::digest(argv.join("\0").as_bytes()));
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
        let mut details = format!(
            "  ssh target: {}\n  workspace:  {}\n  mode:       {}\n  argv:       {}\n  release:    {}",
            sanitize_label(&self.ssh_target),
            sanitize_label(&snapshot.workspace_path),
            sanitize_label(&snapshot.mode),
            argv.iter()
                .map(|value| escape_display(value))
                .collect::<Vec<_>>()
                .join(" "),
            release
                .iter()
                .map(|value| sanitize_label(value))
                .collect::<Vec<_>>()
                .join(", ")
        );
        for source in &allow.sources {
            details.push_str(&format!(
                "\n  source:     {} sha256={} declared={}",
                sanitize_label(&source.path),
                source.sha256,
                source
                    .declared_secrets
                    .iter()
                    .map(|value| sanitize_label(value))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        let policy_comparison =
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
            };
        details.push_str(&format!("\n  comparison: {policy_comparison}"));
        match self.mode {
            BrokerMode::Inspect => {
                eprintln!("shine ssh broker inspect:\n{details}");
                Ok(format!("inspected remote broker request: {summary}"))
            }
            BrokerMode::EnrollTrusted => {
                eprintln!("shine ssh broker trusted remote enrollment candidate:\n{details}");
                let _confirmation = self
                    .confirm("trusted-remote-enrollment", argv, release)
                    .await?;
                let name = broker::add_policy_from_remote_snapshot(
                    &self.config,
                    &self.ssh_target,
                    snapshot,
                    release,
                    argv,
                )
                .await?;
                Ok(format!("enrolled local SSH secret broker policy {name}"))
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
            "Shine secret request\n  ssh target: {target}\n  policy:     {policy}\n  command:    {}\n  secrets:    {}\nApprove? [y/N] ",
            argv.join(" "),
            refs.join(", ")
        );
        let pid = *self.ssh_pid.lock().expect("ssh pid mutex poisoned");
        #[cfg(unix)]
        {
            let original = self
                .original_termios
                .context("local TTY confirmation is unavailable; use a TTY or a matching workspace policy with --trust-remote-session")?;
            tokio::task::spawn_blocking(move || confirm_on_tty(pid, original, &prompt))
                .await
                .context("joining local broker confirmation")?
        }
        #[cfg(not(unix))]
        {
            let _ = (pid, prompt);
            bail!("interactive SSH secret confirmation is not supported on this platform")
        }
    }
}

#[cfg(unix)]
struct LocalConfirmation {
    _restore: TtyRestore,
}

#[cfg(not(unix))]
struct LocalConfirmation;

#[cfg(unix)]
struct TtyRestore {
    tty: std::fs::File,
    pid: u32,
    termios: libc::termios,
}

#[cfg(unix)]
impl Drop for TtyRestore {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        unsafe {
            libc::tcsetattr(self.tty.as_raw_fd(), libc::TCSANOW, &self.termios);
            libc::kill(self.pid as i32, libc::SIGCONT);
        }
    }
}

fn sanitize_label(value: &str) -> String {
    escape_display(value).chars().take(256).collect()
}

fn escape_display(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| match ch {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect(),
            '\r' => "\\r".chars().collect(),
            '\t' => "\\t".chars().collect(),
            ch if ch.is_control() => format!("\\u{{{:x}}}", ch as u32).chars().collect(),
            ch => vec![ch],
        })
        .take(4096)
        .collect()
}

#[cfg(unix)]
fn capture_tty_termios() -> Option<libc::termios> {
    use std::os::fd::AsRawFd;
    let tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()?;
    let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
    let rc = unsafe { libc::tcgetattr(tty.as_raw_fd(), termios.as_mut_ptr()) };
    (rc == 0).then(|| unsafe { termios.assume_init() })
}

#[cfg(unix)]
fn confirm_on_tty(
    pid: Option<u32>,
    original: libc::termios,
    prompt: &str,
) -> Result<LocalConfirmation> {
    use std::io::{BufRead, Write};
    use std::os::fd::AsRawFd;
    let pid = pid.context("SSH child is not running")?;
    let tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .context("opening /dev/tty for local confirmation")?;
    let fd = tty.as_raw_fd();
    let mut ssh_termios = std::mem::MaybeUninit::<libc::termios>::uninit();
    if unsafe { libc::tcgetattr(fd, ssh_termios.as_mut_ptr()) } != 0 {
        bail!("reading current TTY state");
    }
    let ssh_termios = unsafe { ssh_termios.assume_init() };
    if unsafe { libc::kill(pid as i32, libc::SIGSTOP) } != 0 {
        bail!("pausing SSH for local secret confirmation");
    }
    let mut restore = TtyRestore {
        tty,
        pid,
        termios: ssh_termios,
    };
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &original) } != 0 {
        bail!("restoring canonical TTY state for local confirmation");
    }
    restore.tty.write_all(prompt.as_bytes())?;
    restore.tty.flush()?;
    let mut answer = String::new();
    std::io::BufReader::new(restore.tty.try_clone()?).read_line(&mut answer)?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        bail!("secret request rejected by the local user");
    }
    Ok(LocalConfirmation { _restore: restore })
}
