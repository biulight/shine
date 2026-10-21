use super::*;
use crate::runtime::{
    HostOperation, InMemoryHost, NullObserver, PresetSnapshot, PresetSourceKind, RuntimeContext,
    RuntimePlatform,
};
use std::{future::Future, pin::Pin};

struct Interaction {
    allow: bool,
    admin_calls: usize,
}
impl RuntimeInteraction for Interaction {
    fn confirm(&mut self, _: &'static str, default: bool) -> Result<bool> {
        Ok(default)
    }
    fn authorize_admin<'a>(
        &'a mut self,
        _: usize,
    ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send + 'a>> {
        self.admin_calls += 1;
        Box::pin(async { Ok(self.allow) })
    }
    fn select_many(
        &mut self,
        _: &'static str,
        _: &[String],
        defaults: &[String],
    ) -> Result<Vec<String>> {
        Ok(defaults.to_vec())
    }
}
fn runtime(missing_env: bool) -> CoreRuntime<InMemoryHost> {
    let root = std::env::temp_dir().join("shine-bootstrap-contracts");
    let metadata = format!(
        "version = 2\n[[items]]\nid = 'first'\nlabel = 'First'\ndetect = {{ kind = 'path', path = '~/.first' }}\ninstall = {{ kind = 'package', provider = 'apt', package = 'first' }}\n[[items]]\nid = 'second'\nlabel = 'Second'\nrequired_env = [{}]\ndetect = {{ kind = 'path', path = '~/.second' }}\ninstall = {{ kind = 'package', provider = 'apt', package = 'second' }}\n",
        if missing_env { "'TOKEN'" } else { "" }
    );
    CoreRuntime::new(
        InMemoryHost::new(),
        RuntimeContext::isolated(
            root.join("home"),
            root.join("state"),
            root.join("presets"),
            root.join("bin"),
            RuntimePlatform::Linux,
        ),
        PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file("sys/ubuntu/shine.toml", metadata.into_bytes())
            .build(),
    )
}
fn request() -> SysBootstrapBatchRequest {
    SysBootstrapBatchRequest {
        os_id: "ubuntu".into(),
        requested: vec!["first".into(), "second".into()],
        preset: None,
        interactive: false,
        sys_shell: "bash".into(),
        dry_run: false,
        force_profile: false,
    }
}
#[tokio::test]
async fn batch_preflights_later_items_before_any_probe_install_or_admin_prompt() {
    let runtime = runtime(true);
    let mut interaction = Interaction {
        allow: true,
        admin_calls: 0,
    };
    let error = runtime
        .run_sys_bootstrap_batch(request(), &mut interaction, &mut NullObserver)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("requires environment variable `TOKEN`"),
        "{error:#}"
    );
    assert_eq!(interaction.admin_calls, 0);
    assert!(
        runtime
            .host()
            .operations()
            .iter()
            .all(|op| matches!(op, HostOperation::Read(_)))
    );
}
#[tokio::test]
async fn batch_stops_after_first_timeout_and_denied_authorization_never_runs_installer() {
    for allow in [true, false] {
        let runtime = runtime(false);
        runtime.host().queue_process_output(Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "deadline",
        )
        .into()));
        let mut interaction = Interaction {
            allow,
            admin_calls: 0,
        };
        let report = runtime
            .run_sys_bootstrap_batch(request(), &mut interaction, &mut NullObserver)
            .await
            .unwrap();
        assert_eq!(report.outcomes.len(), 1);
        assert_eq!(report.outcomes[0].item_id, "first");
        assert_eq!(report.outcomes[0].status, SysItemStatus::Failed);
        assert_eq!(interaction.admin_calls, 1);
        assert_eq!(runtime.host().process_requests().len(), usize::from(allow));
        assert!(report.outcomes[0].detail.contains(if allow {
            "timed out after 30 minutes"
        } else {
            "administrator authorization was not granted"
        }));
    }
}
#[tokio::test]
async fn selection_deduplicates_in_request_order_without_process_effects() {
    let runtime = runtime(false);
    let mut interaction = Interaction {
        allow: false,
        admin_calls: 0,
    };
    let selection = runtime
        .resolve_sys_bootstrap_selection(
            "ubuntu",
            &["second".into(), "first".into(), "second".into()],
            None,
            false,
            &mut interaction,
            &mut NullObserver,
        )
        .await
        .unwrap();
    assert_eq!(selection.item_ids, ["second", "first"]);
    assert_eq!(selection.source, SelectionSource::Items);
    assert!(runtime.host().process_requests().is_empty());
}
#[tokio::test]
async fn provider_invocation_keeps_sudo_proxy_order_limits_and_platform_rejection() {
    let mut runtime = runtime(false);
    runtime.context_mut_for_test().proxy_env =
        BTreeMap::from([("HTTPS_PROXY".into(), "http://proxy.test".into())]);
    runtime
        .host()
        .queue_process_output(Ok(crate::runtime::ProcessOutput::default()));
    runtime
        .run_package("ubuntu", SysPackageProvider::Apt, "pkg", true)
        .await
        .unwrap();
    let requests = runtime.host().process_requests();
    let request = &requests[0];
    assert_eq!(request.program, "sudo");
    assert_eq!(
        request.args,
        [
            "-n",
            "env",
            "HTTPS_PROXY=http://proxy.test",
            "apt-get",
            "install",
            "-y",
            "pkg"
        ]
    );
    assert_eq!(request.timeout, Some(BOOTSTRAP_TIMEOUT));
    assert_eq!(request.stdout_limit, Some(MAX_LOG_BYTES));
    assert_eq!(request.stderr_limit, Some(MAX_LOG_BYTES));
    assert!(request.inherit_stdin);
    assert_eq!(
        package_preview("ubuntu", SysPackageProvider::Apt, "pkg").unwrap(),
        "sudo apt-get install -y pkg"
    );
    assert!(package_preview("windows", SysPackageProvider::Homebrew, "pkg").is_err());
    assert!(
        runtime
            .run_package("macos", SysPackageProvider::Apt, "pkg", false)
            .await
            .is_err()
    );
    assert_eq!(runtime.host().process_requests().len(), 1);
    let winget = provider::package_command(
        "windows",
        SysPackageProvider::Winget,
        "Publisher.Tool",
        &BTreeMap::from([("SHINE_SYS_PROXY".into(), "http://proxy.test".into())]),
    )
    .unwrap();
    assert_eq!(winget.program, "winget");
    assert_eq!(
        winget.args,
        [
            "install",
            "--exact",
            "--id",
            "Publisher.Tool",
            "--accept-package-agreements",
            "--accept-source-agreements",
            "--proxy",
            "http://proxy.test"
        ]
    );
    assert!(!winget.requires_admin);
}
