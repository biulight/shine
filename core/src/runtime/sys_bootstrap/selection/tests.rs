use super::*;
use crate::runtime::{
    HostOperation, InMemoryHost, NullObserver, PresetSnapshot, PresetSourceKind, RuntimeContext,
    RuntimePlatform, ShellType,
};

#[derive(Default)]
struct Interaction {
    choices: Vec<String>,
    defaults: Vec<String>,
    calls: usize,
}
impl RuntimeInteraction for Interaction {
    fn authorize_admin<'a>(
        &'a mut self,
        _: usize,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool>> + Send + 'a>> {
        panic!("selection must not request administrator access")
    }
    fn confirm(&mut self, _: &'static str, _: bool) -> Result<bool> {
        panic!("selection must not request approval")
    }
    fn select_many(
        &mut self,
        _: &'static str,
        choices: &[String],
        defaults: &[String],
    ) -> Result<Vec<String>> {
        self.calls += 1;
        self.choices = choices.to_vec();
        self.defaults = defaults.to_vec();
        Ok(defaults.to_vec())
    }
}

fn metadata() -> String {
    let mut text = "version = 2\ndefault_profile = 'recommended'\n".to_string();
    for (id, shells) in [
        ("plain", vec![]),
        ("zsh-only", vec!["['zsh']"]),
        ("shared", vec!["['bash', 'zsh']"]),
        ("either", vec!["['powershell']", "['bash']"]),
    ] {
        text.push_str(&format!(
            "[[items]]\nid = '{id}'\nlabel = '{id}'\ndefault = true\ndetect = {{ kind = 'path', path = '~/.present' }}\ninstall = {{ kind = 'package', provider = 'apt', package = '{id}' }}\n"
        ));
        for shells in shells {
            text.push_str(&format!(
                "[[items.shell]]\nshells = {shells}\nphase = 'post'\npath = '$HOME/{id}/bin'\nwhen_command = 'missing-command'\n"
            ));
        }
    }
    text.push_str("[profiles.recommended]\nitems = ['either', 'zsh-only', 'plain', 'shared']\n[profiles.zsh]\nitems = ['zsh-only']\n");
    text
}

fn runtime(metadata: &str, shell: ShellType) -> CoreRuntime<InMemoryHost> {
    let root = std::env::temp_dir().join("shine-shell-selection");
    let mut context = RuntimeContext::isolated(
        root.join("home"),
        root.join("state"),
        root.join("presets"),
        root.join("bin"),
        RuntimePlatform::Linux,
    );
    context.shell = shell;
    CoreRuntime::new(
        InMemoryHost::new(),
        context,
        PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file("sys/ubuntu/shine.toml", metadata.as_bytes().to_vec())
            .build(),
    )
}

#[tokio::test]
async fn automatic_selection_matches_declared_shells_and_preserves_order_and_source() {
    for (shell, expected) in [
        (ShellType::Bash, vec!["either", "plain", "shared"]),
        (ShellType::Zsh, vec!["zsh-only", "plain", "shared"]),
        (ShellType::PowerShell, vec!["either", "plain"]),
        (ShellType::Fish, vec!["plain"]),
        (ShellType::Elvish, vec!["plain"]),
    ] {
        let runtime = runtime(&metadata(), shell);
        // Presence and when_command do not decide visibility or automatic selection.
        runtime
            .host()
            .put_file(runtime.context().home_dir.join(".present"), vec![]);
        for preset in [None, Some("recommended")] {
            let mut interaction = Interaction::default();
            let selected = runtime
                .resolve_sys_bootstrap_selection(
                    "ubuntu",
                    &[],
                    preset,
                    false,
                    &mut interaction,
                    &mut NullObserver,
                )
                .await
                .unwrap();
            assert_eq!(selected.item_ids, expected);
            assert_eq!(
                selected.source,
                if preset.is_some() {
                    SelectionSource::Profile("recommended".into())
                } else {
                    SelectionSource::DefaultProfile("recommended".into())
                }
            );
            let report = runtime
                .preview_sys_bootstrap(
                    SysBootstrapBatchRequest {
                        os_id: "ubuntu".into(),
                        requested: vec![],
                        preset: preset.map(str::to_string),
                        interactive: false,
                        sys_shell: <&str>::from(shell).into(),
                        dry_run: true,
                        force_profile: false,
                    },
                    &mut interaction,
                    &mut NullObserver,
                )
                .await
                .unwrap();
            assert_eq!(report.selection, selected);
            assert_eq!(report.loaded.manifest.items.len(), 4);
            assert_eq!(interaction.calls, 0);
            assert!(runtime.host().process_requests().is_empty());
        }
    }
}

#[tokio::test]
async fn interactive_candidates_and_both_default_sources_use_the_same_filter() {
    for named_default in [true, false] {
        let metadata = if named_default {
            metadata()
        } else {
            metadata().replace("default_profile = 'recommended'\n", "")
        };
        let runtime = runtime(&metadata, ShellType::Bash);
        let mut interaction = Interaction::default();
        let selected = runtime
            .resolve_sys_bootstrap_selection(
                "ubuntu",
                &[],
                None,
                true,
                &mut interaction,
                &mut NullObserver,
            )
            .await
            .unwrap();
        assert_eq!(interaction.choices, ["plain", "shared", "either"]);
        assert_eq!(
            interaction.defaults,
            if named_default {
                vec!["either", "plain", "shared"]
            } else {
                vec!["plain", "shared", "either"]
            }
        );
        assert_eq!(selected.item_ids, interaction.defaults);
        assert_eq!(selected.source, SelectionSource::Interactive);
        assert_eq!(interaction.calls, 1);
    }
}

#[tokio::test]
async fn explicit_mixed_items_are_not_filtered_and_invalid_requests_still_fail() {
    let runtime = runtime(&metadata(), ShellType::Bash);
    let mut interaction = Interaction::default();
    let selected = runtime
        .resolve_sys_bootstrap_selection(
            "ubuntu",
            &["zsh-only".into(), "plain".into(), "zsh-only".into()],
            None,
            false,
            &mut interaction,
            &mut NullObserver,
        )
        .await
        .unwrap();
    assert_eq!(selected.item_ids, ["zsh-only", "plain"]);
    assert_eq!(selected.source, SelectionSource::Items);
    for (ids, preset, expected) in [
        (vec!["missing".into()], None, "unknown sys bootstrap item"),
        (vec![], Some("missing"), "unknown sys bootstrap profile"),
        (vec!["plain".into()], Some("zsh"), "cannot be combined"),
    ] {
        let error = runtime
            .resolve_sys_bootstrap_selection(
                "ubuntu",
                &ids,
                preset,
                false,
                &mut interaction,
                &mut NullObserver,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{error:#}");
    }
}

#[tokio::test]
async fn empty_filtered_selection_preserves_source_without_hiding_default_profile_errors() {
    let text = metadata().replace("['either', 'zsh-only', 'plain', 'shared']", "['zsh-only']");
    let mut manifest = crate::runtime::parse_sys_manifest(&text).unwrap();
    // Retain a real init item, but none matching the target shell.
    manifest.items.retain(|item| item.id == "zsh-only");
    for (preset, interactive, source) in [
        (Some("zsh"), false, SelectionSource::Profile("zsh".into())),
        (
            None,
            false,
            SelectionSource::DefaultProfile("recommended".into()),
        ),
        (None, true, SelectionSource::Interactive),
    ] {
        let mut interaction = Interaction::default();
        let selected = resolve_selection(
            &manifest,
            &[],
            preset,
            interactive,
            "bash",
            &mut interaction,
            &mut NullObserver,
        )
        .unwrap();
        assert!(selected.item_ids.is_empty());
        assert_eq!(selected.source, source);
        assert_eq!(interaction.calls, 0);
    }
    manifest.default_profile = None;
    let error = resolve_selection(
        &manifest,
        &[],
        None,
        false,
        "bash",
        &mut Interaction::default(),
        &mut NullObserver,
    )
    .unwrap_err();
    assert!(error.to_string().contains("requires `default_profile`"));
    manifest.items.clear();
    let selected = resolve_selection(
        &manifest,
        &[],
        None,
        false,
        "bash",
        &mut Interaction::default(),
        &mut NullObserver,
    )
    .unwrap();
    assert_eq!(selected.source, SelectionSource::NoItems);
}

#[tokio::test]
async fn shell_filter_cannot_hide_invalid_manifest_items_or_profile_references() {
    for (text, expected) in [
        (
            metadata().replace("package = 'zsh-only'", "package = '--invalid'"),
            "invalid package identifier",
        ),
        (
            metadata().replace("items = ['zsh-only']", "items = ['unknown']"),
            "unknown item `unknown`",
        ),
    ] {
        let runtime = runtime(&text, ShellType::Fish);
        let error = runtime
            .resolve_sys_bootstrap_selection(
                "ubuntu",
                &[],
                None,
                false,
                &mut Interaction::default(),
                &mut NullObserver,
            )
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains(expected), "{error:#}");
        assert!(
            runtime
                .host()
                .operations()
                .iter()
                .all(|op| matches!(op, HostOperation::Read(_)))
        );
    }
}

#[tokio::test]
async fn filtered_approved_bootstrap_preserves_unselected_enabled_integrations_and_receipts() {
    use crate::plan::PlanApprovalV1;
    use crate::runtime::{FileSystemObservationHost, SysBootstrapPlanRequest, SysRunManifest};

    let text = format!(
        "{}\n[profiles.small]\nitems = ['plain', 'zsh-only']\n",
        metadata()
    );
    let runtime = runtime(&text, ShellType::Bash);
    runtime
        .host()
        .put_file(runtime.context().home_dir.join(".present"), vec![]);
    let mut receipts = SysRunManifest::default();
    for id in ["shared", "zsh-only"] {
        receipts.entries.push(SysRunEntry {
            os_id: "ubuntu".into(),
            item_id: id.into(),
            label: id.into(),
            status: SysItemStatus::Installed,
            detail: "earlier run".into(),
            updated_at: "1".into(),
            managed: false,
            profile_enabled: true,
            receipt: None,
        });
    }
    runtime.host().put_file(
        runtime.context().shine_dir.join("sys-manifest.toml"),
        toml::to_string(&receipts).unwrap().into_bytes(),
    );
    let selected = runtime
        .resolve_sys_bootstrap_selection(
            "ubuntu",
            &[],
            Some("small"),
            false,
            &mut Interaction::default(),
            &mut NullObserver,
        )
        .await
        .unwrap();
    assert_eq!(selected.item_ids, ["plain"]);
    let request = SysBootstrapPlanRequest {
        os_id: "ubuntu".into(),
        item_ids: selected.item_ids,
        sys_shell: "bash".into(),
        force_profile: false,
        input_versions: Default::default(),
    };
    let plan = runtime.plan_sys_bootstrap(request.clone()).await.unwrap();
    let report = runtime
        .run_sys_bootstrap_approved(
            request,
            &PlanApprovalV1::for_reviewed_plan(&plan).unwrap(),
            &mut Interaction::default(),
            &mut NullObserver,
        )
        .await
        .unwrap();
    assert_eq!(report.selection.item_ids, ["plain"]);
    assert_eq!(report.loaded.manifest.items.len(), 4);
    assert_eq!(report.outcomes[0].status, SysItemStatus::AlreadyInstalled);
    let post = String::from_utf8(
        runtime
            .host()
            .read(
                &runtime
                    .context()
                    .home_dir
                    .join(".shine/profile/ubuntu-sys.post.sh"),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(post.contains("shared/bin"), "{post}");
    assert!(!post.contains("zsh-only/bin"), "{post}");
    assert!(post.contains("missing-command"), "{post}");
    let after = SysRunManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    for before in receipts.entries {
        let kept = after
            .entries
            .iter()
            .find(|entry| entry.item_id == before.item_id)
            .unwrap();
        assert_eq!(kept, &before);
    }
    assert!(runtime.host().process_requests().is_empty());
}

#[tokio::test]
async fn approved_empty_selection_never_expands_a_default_profile() {
    use crate::plan::PlanApprovalV1;
    use crate::runtime::SysBootstrapPlanRequest;

    for has_default in [true, false] {
        let text = if has_default {
            metadata()
        } else {
            metadata().replace("default_profile = 'recommended'\n", "")
        };
        let runtime = runtime(&text, ShellType::Bash);
        runtime
            .host()
            .put_file(runtime.context().home_dir.join(".present"), vec![]);
        // The named selection is valid but becomes empty for Bash, while the default
        // profile (when present) still contains installable items.
        let selection = runtime
            .resolve_sys_bootstrap_selection(
                "ubuntu",
                &[],
                Some("zsh"),
                false,
                &mut Interaction::default(),
                &mut NullObserver,
            )
            .await
            .unwrap();
        assert!(selection.item_ids.is_empty());
        let request = SysBootstrapPlanRequest {
            os_id: "ubuntu".into(),
            item_ids: selection.item_ids,
            sys_shell: "bash".into(),
            force_profile: false,
            input_versions: Default::default(),
        };
        let plan = runtime.plan_sys_bootstrap(request.clone()).await.unwrap();
        assert!(plan.steps.is_empty());
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        let since = runtime.host().operations().len();
        // The empty fast path must still reject approval for a different request.
        for change_items in [true, false] {
            let mut changed = request.clone();
            if change_items {
                changed.item_ids = vec!["plain".into()];
            } else {
                changed.sys_shell = "zsh".into();
            }
            let error = runtime
                .run_sys_bootstrap_approved(
                    changed,
                    &approval,
                    &mut Interaction::default(),
                    &mut NullObserver,
                )
                .await
                .unwrap_err();
            let expected = if change_items {
                crate::plan::PlanApprovalError::PermissionSetChanged
            } else {
                crate::plan::PlanApprovalError::PlanChanged
            };
            assert_eq!(
                error.downcast_ref::<crate::plan::PlanApprovalError>(),
                Some(&expected),
                "{error:#}"
            );
        }
        let report = runtime
            .run_sys_bootstrap_approved(
                request,
                &approval,
                &mut Interaction::default(),
                &mut NullObserver,
            )
            .await
            .unwrap();
        assert!(report.selection.item_ids.is_empty());
        assert_eq!(report.selection.source, SelectionSource::Items);
        assert!(report.previews.is_empty());
        assert!(report.outcomes.is_empty());
        assert!(
            runtime.host().operations()[since..]
                .iter()
                .all(|op| matches!(
                    op,
                    HostOperation::Read(_) | HostOperation::AcquireOperationLock(_)
                ))
        );
    }
}
