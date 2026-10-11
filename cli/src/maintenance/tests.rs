impl Report {
    fn notified(&self) -> bool {
        !self.notifications.is_empty()
    }
    fn required(&self) -> bool {
        !self.actions.is_empty()
    }
}
use super::*;
use clap::Parser;
use shine_core::runtime::{HostOperation, InMemoryHost, ProcessOutput};

struct Fixture {
    root: PathBuf,
    scope: Scope,
    host: InMemoryHost,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("shine-maintenance-{}", Uuid::new_v4()));
        let scope = Scope::new(root.join("state"), root.join("home"), None, "macos").unwrap();
        Self {
            root,
            scope,
            host: InMemoryHost::new(),
        }
    }
    fn registration(&self) -> PathBuf {
        self.scope
            .home
            .join("Library/LaunchAgents/top.biulight.shine.http.plist")
    }
    fn queue(&self, code: i32, output: &str) {
        self.host.queue_process_output(Ok(ProcessOutput {
            exit_code: Some(code),
            stdout: output.as_bytes().to_vec(),
            ..ProcessOutput::default()
        }));
    }
    async fn inspect(&self, force: bool, at: u64) -> Report {
        inspect(&self.scope, &self.host, RULES, force, at)
            .await
            .unwrap()
    }
    fn runs(&self) -> usize {
        self.host
            .operations()
            .iter()
            .filter(|op| matches!(op, HostOperation::Run { .. }))
            .count()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn four_frequencies_have_independent_boundaries_and_clock_rollback_is_due() {
    for (frequency, seconds) in [
        (Frequency::EveryInvocation, 0),
        (Frequency::Minute, 60),
        (Frequency::Hour, 3600),
        (Frequency::Day, 86400),
    ] {
        assert!(frequency.due(None, 100));
        assert!(frequency.due(Some(101), 100));
        assert!(frequency.due(Some(100), 100 + seconds));
        if seconds > 0 {
            assert!(!frequency.due(Some(100), 99 + seconds));
        }
    }
}

#[test]
fn leases_failures_and_revisions_control_retries() {
    let rule = RULES[1];
    let mut entry = Entry::for_rule(rule);
    entry.pending = Some(Pending {
        ticket: Uuid::new_v4(),
        started_at: 100,
    });
    assert!(!entry.due(rule, 219));
    assert!(entry.due(rule, 220));
    assert!(entry.due(rule, 99));
    entry.complete(&Err(anyhow::anyhow!("offline")), 300);
    assert!(!entry.due(rule, 359));
    assert!(entry.due(rule, 360));
    assert!(entry.due(
        Rule {
            revision: 2,
            ..rule
        },
        301
    ));
    entry.complete(&Ok(true), 400);
    assert!(entry.finding(rule, 400));
    assert!(!entry.finding(rule, 399));
    assert!(!entry.finding(rule, 400 + RESULT_TTL));
}

#[tokio::test]
async fn inline_registration_notifies_now_and_deduplicates_daily() {
    let f = Fixture::new();
    f.host.put_file(f.registration(), b"old service".to_vec());
    let at = now();
    let first = f.inspect(false, at).await;
    assert!(first.notified() && first.required() && first.worker.is_some());
    assert_eq!(f.runs(), 0);
    assert!(!f.inspect(false, at + 1).await.notified());
    assert!(f.inspect(false, at + REMIND_AFTER).await.notified());
    f.host.remove_file(&f.registration()).await.unwrap();
    assert!(!f.inspect(false, at + REMIND_AFTER + 1).await.required());
    f.host
        .put_file(f.registration(), b"new registration".to_vec());
    assert!(f.inspect(false, at + REMIND_AFTER + 2).await.notified());
}

#[tokio::test]
async fn native_result_is_published_by_worker_and_shown_on_next_invocation() {
    let f = Fixture::new();
    let first = f.inspect(false, now()).await;
    assert!(!first.notified() && !first.required());
    assert_eq!(f.runs(), 0);
    assert!(f.inspect(false, now()).await.worker.is_none());
    f.queue(0, "- 0 top.biulight.shine.http\n");
    run_worker(&f.scope, first.worker.unwrap(), &f.host)
        .await
        .unwrap();
    assert_eq!(f.runs(), 1);
    let next = f.inspect(false, now()).await;
    assert!(next.notified() && next.required() && next.worker.is_none());
    assert!(!f.inspect(false, now()).await.notified());
    invalidate(&f.scope).await.unwrap();
    assert!(!f.inspect(false, now()).await.required());
}

#[tokio::test]
async fn native_failures_back_off_without_becoming_findings() {
    let f = Fixture::new();
    let first = f.inspect(false, now()).await;
    f.queue(1, "");
    run_worker(&f.scope, first.worker.unwrap(), &f.host)
        .await
        .unwrap();
    let cache = store::load(&f.scope).await;
    let failed = cache.entries[&CheckId::ServeNative].attempted_at.unwrap();
    assert!(!f.inspect(false, failed + 1).await.notified());
    assert!(f.inspect(false, failed + 1).await.worker.is_none());
    assert!(f.inspect(false, failed + RETRY).await.worker.is_some());
}

#[tokio::test]
async fn explicit_check_runs_heavy_probes_immediately_and_does_not_change_resources() {
    let f = Fixture::new();
    f.queue(0, "- 0 top.biulight.shine.http\n");
    let report = f.inspect(true, now()).await;
    assert!(report.notified() && report.required() && report.worker.is_none());
    assert_eq!(f.runs(), 1);
    assert!(
        !f.host
            .operations()
            .iter()
            .any(|op| matches!(op, HostOperation::Remove(_) | HostOperation::Write(_)))
    );
    f.queue(0, "");
    assert!(!f.inspect(true, now()).await.required());
}

#[tokio::test]
async fn corrupt_old_build_foreign_scope_and_new_rule_revision_are_cache_misses() {
    let f = Fixture::new();
    std::fs::create_dir_all(&f.scope.root).unwrap();
    std::fs::write(f.scope.root.join("checks.json"), b"broken json").unwrap();
    let first = f.inspect(false, now()).await;
    assert!(first.worker.is_some());
    let mut cache = store::load(&f.scope).await;
    let epoch = cache.generation;
    cache.build = "old build".into();
    store::save(&f.scope, &cache).await.unwrap();
    assert_ne!(store::load(&f.scope).await.generation, epoch);
    cache.build = crate::version::display().into();
    cache.scope = "another user/config".into();
    store::save(&f.scope, &cache).await.unwrap();
    assert_ne!(store::load(&f.scope).await.generation, epoch);
    let rule = RULES[1];
    let mut entry = Entry::for_rule(rule);
    entry.complete(&Ok(true), now());
    cache = Cache::new(&f.scope);
    cache.entries.insert(rule.id, entry);
    cache.notified_at.insert(Action::ClearServe, now());
    store::save(&f.scope, &cache).await.unwrap();
    let revised = [Rule {
        revision: 2,
        ..rule
    }];
    let report = inspect(&f.scope, &f.host, &revised, false, now())
        .await
        .unwrap();
    assert!(!report.required() && report.worker.is_some());
    assert!(store::load(&f.scope).await.notified_at.is_empty());
}

#[tokio::test]
async fn invalidated_tickets_and_duplicate_workers_do_not_execute_probes() {
    let f = Fixture::new();
    let ticket = f.inspect(false, now()).await.worker.unwrap();
    invalidate(&f.scope).await.unwrap();
    run_worker(&f.scope, ticket, &f.host).await.unwrap();
    assert_eq!(f.runs(), 0);
    let ticket = f.inspect(false, now()).await.worker.unwrap();
    let guard = store::lock(&f.scope.root.join("worker.lock")).unwrap();
    assert!(run_worker(&f.scope, ticket, &f.host).await.is_err());
    assert_eq!(f.runs(), 0);
    drop(guard);
    f.queue(0, "");
    run_worker(&f.scope, ticket, &f.host).await.unwrap();
    run_worker(&f.scope, ticket, &f.host).await.unwrap();
    assert_eq!(f.runs(), 1);
}

#[tokio::test]
async fn failed_spawn_is_retryable_after_backoff_and_does_not_replace_cleared_state() {
    let f = Fixture::new();
    let ticket = f.inspect(false, now()).await.worker.unwrap();
    failed_spawn(&f.scope, ticket).await.unwrap();
    let cache = store::load(&f.scope).await;
    let attempted = cache.entries[&CheckId::ServeNative].attempted_at.unwrap();
    assert!(
        f.inspect(false, attempted + RETRY - 1)
            .await
            .worker
            .is_none()
    );
    assert!(f.inspect(false, attempted + RETRY).await.worker.is_some());
    invalidate(&f.scope).await.unwrap();
    failed_spawn(&f.scope, ticket).await.unwrap();
    assert!(store::load(&f.scope).await.entries.is_empty());
}

#[tokio::test]
async fn scope_is_stable_when_missing_directories_are_created() {
    let f = Fixture::new();
    std::fs::create_dir_all(&f.scope.config_dir).unwrap();
    std::fs::create_dir_all(&f.scope.home).unwrap();
    let after = Scope::new(
        f.scope.config_dir.clone(),
        f.scope.home.clone(),
        None,
        "macos",
    )
    .unwrap();
    assert_eq!(after.key, f.scope.key);
}

#[test]
fn scopes_separate_users_config_dirs_and_native_config_locations() {
    let f = Fixture::new();
    for scope in [
        Scope::new(
            f.root.join("other-state"),
            f.scope.home.clone(),
            None,
            "macos",
        )
        .unwrap(),
        Scope::new(
            f.scope.config_dir.clone(),
            f.root.join("other-home"),
            None,
            "macos",
        )
        .unwrap(),
        Scope::new(
            f.scope.config_dir.clone(),
            f.scope.home.clone(),
            Some(f.root.join("xdg")),
            "macos",
        )
        .unwrap(),
    ] {
        assert_ne!(scope.key, f.scope.key);
    }
}

#[test]
fn automation_recovery_and_maintenance_commands_skip_automatic_advisories() {
    for args in [
        vec!["state", "check"],
        vec!["clear", "serve"],
        vec!["theme", "sync", "--auto", "--quiet"],
        vec!["completions", "bash"],
        vec!["env", "list"],
        vec!["run", "demo"],
        vec!["app", "recover"],
        vec!["shell", "recover"],
        vec!["sys", "recover"],
    ] {
        let cli =
            crate::commands::Cli::try_parse_from(std::iter::once("shine").chain(args)).unwrap();
        assert!(!eligible(&cli.command), "{:?}", cli.command);
    }
    let cli = crate::commands::Cli::try_parse_from(["shine", "task", "list"]).unwrap();
    assert!(eligible(&cli.command));
}
