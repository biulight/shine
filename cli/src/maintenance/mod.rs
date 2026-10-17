//! Best-effort maintenance advisories, separate from safety and release gates.
mod store;
mod worker;

use crate::commands::{AppCommands, Commands, ShellCommands, SysCommands, TaskCommands};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use shine_core::runtime::{FileSystemHost, ProcessHost, RealHost};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::IsTerminal,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const RETRY: u64 = 60;
const LEASE: u64 = 120;
const RESULT_TTL: u64 = 86_400;
const REMIND_AFTER: u64 = 86_400;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frequency {
    EveryInvocation,
    Minute,
    Hour,
    Day,
}

impl Frequency {
    fn seconds(self) -> u64 {
        match self {
            Self::EveryInvocation => 0,
            Self::Minute => 60,
            Self::Hour => 3600,
            Self::Day => 86_400,
        }
    }
    fn due(self, checked_at: Option<u64>, now: u64) -> bool {
        checked_at
            .and_then(|last| now.checked_sub(last))
            .is_none_or(|age| age >= self.seconds())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Execution {
    Inline,
    Background,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum CheckId {
    ServeRegistration,
    ServeNative,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Action {
    ClearServe,
}

impl Action {
    fn message(self) -> &'static str {
        match self {
            Self::ClearServe => {
                "Maintenance: an older Shine HTTP service remains registered or loaded. Run `shine clear serve` to stop and remove it; published files are retained."
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Rule {
    id: CheckId,
    action: Action,
    revision: u32,
    frequency: Frequency,
    execution: Execution,
}

const RULES: &[Rule] = &[
    Rule {
        id: CheckId::ServeRegistration,
        action: Action::ClearServe,
        revision: 1,
        frequency: Frequency::EveryInvocation,
        execution: Execution::Inline,
    },
    Rule {
        id: CheckId::ServeNative,
        action: Action::ClearServe,
        revision: 1,
        frequency: Frequency::Hour,
        execution: Execution::Background,
    },
];

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Pending {
    ticket: Uuid,
    started_at: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Entry {
    revision: u32,
    checked_at: Option<u64>,
    attempted_at: Option<u64>,
    required: Option<bool>,
    failed: bool,
    pending: Option<Pending>,
}

impl Entry {
    fn for_rule(rule: Rule) -> Self {
        Self {
            revision: rule.revision,
            ..Self::default()
        }
    }
    fn due(&self, rule: Rule, now: u64) -> bool {
        if self.revision != rule.revision {
            return true;
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|p| now.checked_sub(p.started_at).is_some_and(|age| age < LEASE))
        {
            return false;
        }
        if self.failed
            && self
                .attempted_at
                .and_then(|last| now.checked_sub(last))
                .is_some_and(|age| age < RETRY)
        {
            return false;
        }
        rule.frequency.due(self.checked_at, now)
    }
    fn finding(&self, rule: Rule, now: u64) -> bool {
        self.revision == rule.revision
            && self.required == Some(true)
            && self
                .checked_at
                .and_then(|last| now.checked_sub(last))
                .is_some_and(|age| age < RESULT_TTL)
    }
    fn complete(&mut self, result: &Result<bool>, now: u64) {
        self.pending = None;
        self.attempted_at = Some(now);
        self.failed = result.is_err();
        if let Ok(required) = result {
            self.checked_at = Some(now);
            self.required = Some(*required);
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Cache {
    schema: u32,
    scope: String,
    build: String,
    generation: Uuid,
    entries: BTreeMap<CheckId, Entry>,
    notified_at: BTreeMap<Action, u64>,
}

impl Cache {
    fn new(scope: &Scope) -> Self {
        Self {
            schema: 1,
            scope: scope.key.clone(),
            build: crate::version::display().into(),
            generation: Uuid::new_v4(),
            entries: BTreeMap::new(),
            notified_at: BTreeMap::new(),
        }
    }
    fn entry(&mut self, rule: Rule) -> &mut Entry {
        let entry = self
            .entries
            .entry(rule.id)
            .or_insert_with(|| Entry::for_rule(rule));
        if entry.revision != rule.revision {
            *entry = Entry::for_rule(rule);
            self.notified_at.remove(&rule.action);
        }
        entry
    }
}

#[derive(Clone, Debug)]
struct Scope {
    root: PathBuf,
    config_dir: PathBuf,
    home: PathBuf,
    xdg: Option<PathBuf>,
    platform: String,
    key: String,
}

impl Scope {
    fn new(
        config_dir: PathBuf,
        home: PathBuf,
        xdg: Option<PathBuf>,
        platform: &str,
    ) -> Result<Self> {
        use shine_core::digest::sha256_hex;
        fn absolute(path: PathBuf) -> Result<PathBuf> {
            let mut existing = std::path::absolute(path)?;
            let mut missing = Vec::new();
            loop {
                match std::fs::canonicalize(&existing) {
                    Ok(mut base) => {
                        for name in missing.into_iter().rev() {
                            base.push(name);
                        }
                        return Ok(base);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        missing.push(existing.file_name().ok_or(error)?.to_os_string());
                        existing.pop();
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
        let config_dir = absolute(config_dir)?;
        let home = absolute(home)?;
        let xdg = xdg.filter(|p| p.is_absolute()).map(absolute).transpose()?;
        #[cfg(unix)]
        let user = unsafe { libc::geteuid() }.to_string();
        #[cfg(not(unix))]
        let user = format!(
            "{}\\{}",
            std::env::var("USERDOMAIN").unwrap_or_default(),
            std::env::var("USERNAME").unwrap_or_default()
        );
        let identity = serde_json::to_vec(&(&config_dir, &home, &xdg, platform, user))?;
        let key = sha256_hex(&identity);
        let root = config_dir.join("maintenance").join(&key);
        Ok(Self {
            root,
            config_dir,
            home,
            xdg,
            platform: platform.into(),
            key,
        })
    }
    fn from_env() -> Result<Self> {
        let home = crate::home::effective_home_dir();
        let config = std::env::var("SHINE_CONFIG_DIR")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .map(|v| PathBuf::from(crate::home::tilde_expand(v.trim())))
            .unwrap_or_else(|| home.join(".shine"));
        Self::new(
            config,
            home,
            std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
            std::env::consts::OS,
        )
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn eligible(command: &Commands) -> bool {
    !matches!(
        command,
        Commands::Init(_)
            | Commands::Completions { .. }
            | Commands::State { .. }
            | Commands::Clear { .. }
            | Commands::Self_ { .. }
            | Commands::Preset { .. }
            | Commands::Theme { .. }
            | Commands::ShellLaunch { .. }
            | Commands::ShellRender { .. }
            | Commands::MaintenanceWorker { .. }
            | Commands::Env { .. }
            | Commands::Run(..)
            | Commands::Ssh { .. }
            | Commands::Local { .. }
            | Commands::Task {
                command: TaskCommands::Run(..)
            }
            | Commands::App {
                command: AppCommands::Recover { .. }
            }
            | Commands::Shell {
                command: ShellCommands::Recover { .. }
            }
            | Commands::Sys {
                command: SysCommands::Recover { .. }
            }
    )
}

/// Called before config loading. Failures never block the user's primary command.
pub async fn maybe_notify(command: &Commands) {
    if !eligible(command) || !std::io::stderr().is_terminal() {
        return;
    }
    if let Ok(scope) = Scope::from_env()
        && let Ok(report) = inspect(&scope, &RealHost, RULES, false, now()).await
    {
        print_advisories(&report.notifications, 0);
        if let Some(ticket) = report.worker
            && worker::spawn(&scope, ticket).is_err()
        {
            let _ = failed_spawn(&scope, ticket).await;
        }
    }
}

/// Explicit synchronous inspection, available even with invalid/retired config.
pub async fn handle_check() -> Result<()> {
    let scope = Scope::from_env()?;
    let report = inspect(&scope, &RealHost, RULES, true, now()).await?;
    if report.superseded {
        print_advisory_messages([
            "Warning: maintenance check was superseded by another check or cleanup; retry `shine state check`.",
        ]);
        return Err(crate::output::ReportedCommandFailure.into());
    }
    print_advisories(&report.notifications, report.failures);
    println!(
        "Maintenance checks complete: {} action(s) required, {} check(s) incomplete.",
        report.actions.len(),
        report.failures
    );
    Ok(())
}

fn print_advisories(actions: &BTreeSet<Action>, failures: usize) {
    print_advisory_messages(actions.iter().map(|action| action.message()).chain(
        std::iter::repeat_n(
            "Warning: a maintenance check could not complete; retry `shine state check`.",
            failures,
        ),
    ));
}

fn print_advisory_messages(messages: impl IntoIterator<Item = &'static str>) {
    let mut printed = false;
    for message in messages {
        eprintln!("{}", crate::colors::bold_yellow_stderr(message));
        printed = true;
    }
    if printed {
        eprintln!();
    }
}

pub async fn invalidate_serve() {
    // No failure can turn successful native cleanup into a failed command. Results also expire.
    let result = match Scope::from_env() {
        Ok(scope) => invalidate(&scope).await,
        Err(error) => Err(error),
    };
    if result.is_err() {
        eprintln!(
            "Warning: service cleanup succeeded, but cached maintenance reminders could not be reset."
        );
    }
}

async fn invalidate(scope: &Scope) -> Result<()> {
    let _guard = store::lock_wait(&scope.root.join("state.lock")).await?;
    let mut cache = store::load(scope).await;
    cache.generation = Uuid::new_v4();
    for rule in RULES
        .iter()
        .filter(|rule| rule.action == Action::ClearServe)
    {
        cache.entries.remove(&rule.id);
    }
    cache.notified_at.remove(&Action::ClearServe);
    store::save(scope, &cache).await
}

async fn failed_spawn(scope: &Scope, ticket: Uuid) -> Result<()> {
    let _guard = store::lock(&scope.root.join("state.lock"))?;
    let mut cache = store::load(scope).await;
    for entry in cache.entries.values_mut() {
        if entry.pending.as_ref().is_some_and(|p| p.ticket == ticket) {
            entry.complete(&Err(anyhow::anyhow!("worker could not start")), now());
        }
    }
    store::save(scope, &cache).await
}

#[derive(Default)]
struct Report {
    notifications: BTreeSet<Action>,
    actions: BTreeSet<Action>,
    failures: usize,
    worker: Option<Uuid>,
    superseded: bool,
}

async fn probe(
    scope: &Scope,
    host: &(impl FileSystemHost + ProcessHost),
    id: CheckId,
) -> Result<bool> {
    match id {
        CheckId::ServeRegistration => {
            crate::legacy_serve::has_registration(
                host,
                &scope.platform,
                &scope.home,
                scope.xdg.as_deref(),
            )
            .await
        }
        CheckId::ServeNative => {
            crate::legacy_serve::has_native_service(host, &scope.platform).await
        }
    }
}

async fn inspect(
    scope: &Scope,
    host: &(impl FileSystemHost + ProcessHost),
    rules: &[Rule],
    force: bool,
    started: u64,
) -> Result<Report> {
    let (generation, cached_findings, inline, ticket) = {
        let _guard = store::lock(&scope.root.join("state.lock"))?;
        let mut cache = store::load(scope).await;
        // Explicit inspection supersedes in-flight work, without holding locks while probing.
        if force {
            cache.generation = Uuid::new_v4();
        }
        let mut inline = Vec::new();
        let mut cached = Vec::new();
        let mut ticket = None;
        for &rule in rules {
            let entry = cache.entry(rule);
            if force || entry.due(rule, started) {
                if force || rule.execution == Execution::Inline {
                    inline.push(rule);
                } else {
                    let ticket = *ticket.get_or_insert_with(Uuid::new_v4);
                    entry.pending = Some(Pending {
                        ticket,
                        started_at: started,
                    });
                    entry.attempted_at = Some(started);
                }
            }
            if !force && rule.execution == Execution::Background && entry.finding(rule, started) {
                cached.push((rule.id, entry.checked_at));
            }
        }
        store::save(scope, &cache).await?;
        (cache.generation, cached, inline, ticket)
    };
    let mut results = Vec::new();
    for rule in inline {
        let limit = if rule.execution == Execution::Inline {
            Duration::from_millis(100)
        } else {
            Duration::from_secs(12)
        };
        let result = tokio::time::timeout(limit, probe(scope, host, rule.id))
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("maintenance check timed out")));
        results.push((rule, result));
    }
    let _guard = store::lock(&scope.root.join("state.lock"))?;
    let mut cache = store::load(scope).await;
    if cache.generation != generation {
        return Ok(Report {
            superseded: force,
            ..Report::default()
        });
    }
    let failures = results.iter().filter(|(_, result)| result.is_err()).count();
    for (rule, result) in results {
        cache.entry(rule).complete(&result, started);
    }
    // Use only background results already present at invocation start.
    let actions = rules
        .iter()
        .filter_map(|&rule| {
            let entry = cache.entry(rule);
            (entry.finding(rule, started)
                && (force
                    || rule.execution == Execution::Inline
                    || cached_findings
                        .iter()
                        .any(|(id, checked)| *id == rule.id && *checked == entry.checked_at)))
            .then_some(rule.action)
        })
        .collect::<BTreeSet<_>>();
    let notifications = actions
        .iter()
        .copied()
        .filter(|action| {
            force
                || cache
                    .notified_at
                    .get(action)
                    .and_then(|last| started.checked_sub(*last))
                    .is_none_or(|age| age >= REMIND_AFTER)
        })
        .collect::<BTreeSet<_>>();
    for action in &notifications {
        cache.notified_at.insert(*action, started);
    }
    let active = rules
        .iter()
        .filter_map(|&rule| {
            cache
                .entry(rule)
                .finding(rule, started)
                .then_some(rule.action)
        })
        .collect::<BTreeSet<_>>();
    cache
        .notified_at
        .retain(|action, _| active.contains(action));
    store::save(scope, &cache).await?;
    Ok(Report {
        notifications,
        actions,
        failures,
        worker: ticket,
        superseded: false,
    })
}

async fn run_worker(
    scope: &Scope,
    ticket: Uuid,
    host: &(impl FileSystemHost + ProcessHost),
) -> Result<()> {
    let _worker = store::lock(&scope.root.join("worker.lock"))?;
    let (generation, rules) = {
        let _guard = store::lock_wait(&scope.root.join("state.lock")).await?;
        let cache = store::load(scope).await;
        let rules = RULES
            .iter()
            .copied()
            .filter(|rule| {
                rule.execution == Execution::Background
                    && cache.entries.get(&rule.id).is_some_and(|entry| {
                        entry.revision == rule.revision
                            && entry.pending.as_ref().is_some_and(|p| {
                                p.ticket == ticket
                                    && now()
                                        .checked_sub(p.started_at)
                                        .is_some_and(|age| age < LEASE)
                            })
                    })
            })
            .collect::<Vec<_>>();
        (cache.generation, rules)
    };
    for rule in rules {
        let result = tokio::time::timeout(Duration::from_secs(12), probe(scope, host, rule.id))
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("maintenance check timed out")));
        let _guard = store::lock_wait(&scope.root.join("state.lock")).await?;
        let mut cache = store::load(scope).await;
        if cache.generation != generation {
            return Ok(());
        }
        let Some(entry) = cache.entries.get_mut(&rule.id) else {
            continue;
        };
        if entry.revision == rule.revision
            && entry.pending.as_ref().is_some_and(|p| p.ticket == ticket)
        {
            entry.complete(&result, now());
            store::save(scope, &cache).await?;
        }
    }
    Ok(())
}

pub async fn handle_worker(expected_scope: &str, ticket: Uuid) -> Result<()> {
    let scope = Scope::from_env()?;
    anyhow::ensure!(
        scope.key == expected_scope,
        "maintenance worker scope changed"
    );
    tokio::time::timeout(
        Duration::from_secs(30),
        run_worker(&scope, ticket, &RealHost),
    )
    .await??;
    Ok(())
}

#[cfg(test)]
mod tests;
