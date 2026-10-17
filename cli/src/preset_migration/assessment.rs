//! Same-scope candidate validation, managed overlay policy and released metadata baselines.
use super::*;

pub(super) fn sys_categories_for_item(snapshot: &PresetSnapshot, item: &str) -> BTreeSet<String> {
    snapshot
        .files()
        .iter()
        .filter_map(|(logical, bytes)| {
            let category = logical.strip_prefix("sys/")?.strip_suffix("/shine.toml")?;
            if category == item {
                return Some(format!("sys/{category}"));
            }
            let value = toml::from_slice::<toml::Value>(bytes).ok()?;
            value
                .get("items")
                .and_then(toml::Value::as_array)
                .is_some_and(|items| {
                    items
                        .iter()
                        .any(|entry| entry.get("id").and_then(toml::Value::as_str) == Some(item))
                })
                .then(|| format!("sys/{category}"))
        })
        .collect()
}

pub(super) fn mark_managed_overlay_read_only(plan: &mut PresetMigrationPlan, root: &Path) {
    let blocked = plan
        .edits
        .iter()
        .filter(|edit| edit.physical_path.starts_with(root))
        .map(|edit| edit.logical_path.clone())
        .collect::<BTreeSet<_>>();
    if blocked.is_empty() {
        return;
    }
    plan.edits
        .retain(|edit| !blocked.contains(&edit.logical_path));
    for target in blocked {
        plan.report.diagnostics.push(PresetMigrationDiagnosticV1 {
            severity: PresetMigrationSeverityV1::Blocker,
            code: "managed_overlay_read_only".to_string(),
            target,
            source_layer: Some("overlay".to_string()),
            message: "the active Git-managed overlay is force-mirrored; migrate its upstream checkout instead".to_string(),
        });
    }
    sync_report_with_edits(plan);
}

pub(super) async fn validate_candidate(snapshot: &PresetSnapshot, plan: &mut PresetMigrationPlan) {
    // `validate_preset_path` derives a synthetic home beneath this root. Keep the
    // root absolute according to the native path semantics so its all-platform
    // metadata validation can run on Windows too.
    #[cfg(windows)]
    let root = Path::new(r"C:\shine-preset-migration");
    #[cfg(not(windows))]
    let root = Path::new("/shine-preset-migration");
    let host = InMemoryHost::new();
    let candidate_targets = plan
        .edits
        .iter()
        .map(|edit| {
            edit.logical_path
                .split('/')
                .take(2)
                .collect::<Vec<_>>()
                .join("/")
        })
        .collect::<BTreeSet<_>>();
    if candidate_targets.is_empty() {
        return;
    }
    let mut files = snapshot.files().clone();
    for edit in &plan.edits {
        match &edit.candidate {
            Some(candidate) => {
                files.insert(edit.logical_path.clone(), candidate.clone());
            }
            None => {
                if let Some(base) = snapshot.base_bytes(&edit.logical_path) {
                    files.insert(edit.logical_path.clone(), base.to_vec());
                } else {
                    files.remove(&edit.logical_path);
                }
            }
        }
    }
    for (logical, bytes) in files {
        let target = logical.split('/').take(2).collect::<Vec<_>>().join("/");
        if !candidate_targets.contains(&target) {
            continue;
        }
        host.put_file(root.join(logical), bytes);
    }
    let validation = validate_preset_path(&host, root, root).await;
    let invalid_targets = validation
        .categories
        .iter()
        .filter(|category| !category.valid)
        .map(|category| format!("{}/{}", category.kind, category.name))
        .collect::<BTreeSet<_>>();
    for category in validation
        .categories
        .iter()
        .filter(|category| !category.valid)
    {
        let target = format!("{}/{}", category.kind, category.name);
        for item in category
            .diagnostics
            .iter()
            .filter(|item| item.severity == PresetDiagnosticSeverity::Error)
        {
            plan.report.diagnostics.push(PresetMigrationDiagnosticV1 {
                severity: PresetMigrationSeverityV1::Blocker,
                code: format!("candidate_{}", item.code),
                target: target.clone(),
                source_layer: report_source_layer(plan, &target),
                message: item.message.clone(),
            });
        }
    }
    for item in validation
        .diagnostics
        .iter()
        .filter(|item| item.severity == PresetDiagnosticSeverity::Error)
    {
        plan.report.diagnostics.push(PresetMigrationDiagnosticV1 {
            severity: PresetMigrationSeverityV1::Blocker,
            code: format!("candidate_{}", item.code),
            target: plan.report.scope.clone(),
            source_layer: None,
            message: item.message.clone(),
        });
    }
    if !invalid_targets.is_empty() {
        plan.edits.retain(|edit| {
            let target = edit
                .logical_path
                .split('/')
                .take(2)
                .collect::<Vec<_>>()
                .join("/");
            !invalid_targets.contains(&target)
        });
    }
    sync_report_with_edits(plan);
}

pub(super) fn report_source_layer(plan: &PresetMigrationPlan, target: &str) -> Option<String> {
    plan.report
        .files
        .iter()
        .find(|file| file.target.starts_with(target))
        .map(|file| file.source_layer.clone())
}

pub(super) fn sync_report_with_edits(plan: &mut PresetMigrationPlan) {
    let edited = plan
        .edits
        .iter()
        .map(|edit| edit.logical_path.as_str())
        .collect::<BTreeSet<_>>();
    plan.report
        .files
        .retain(|file| edited.contains(file.target.as_str()));
    plan.report.summary.changes = plan.edits.len();
    plan.report.summary.blockers = plan
        .report
        .diagnostics
        .iter()
        .filter(|item| item.severity == PresetMigrationSeverityV1::Blocker)
        .count();
    plan.report.status = if plan.report.summary.blockers > 0 {
        PresetMigrationStatusV1::Blocked
    } else if plan.edits.is_empty() {
        PresetMigrationStatusV1::Current
    } else {
        PresetMigrationStatusV1::Pending
    };
}

pub(super) fn legacy_metadata_hashes() -> BTreeMap<String, BTreeSet<String>> {
    // Unique shine.toml SHA-256 values shipped by every v1.0.0 through v1.8.0
    // release tag (including patch releases). Sys fingerprints remain evidence
    // only: Core deliberately refuses to turn a v1 dispatcher into v2 items.
    const ENTRIES: &[(&str, &str)] = &[
        (
            "app/JetBrains/shine.toml",
            "0d6545cdfa392b6d4742bcddc288b4faf7382df3a23395a69300508e7c09e8f1",
        ),
        (
            "app/archey4/shine.toml",
            "97355899e41859f3c63323c5f4419a8e2c5f6723673765b7bf7935d3f8e2b6c2",
        ),
        (
            "app/clash-verge/shine.toml",
            "1a90f41ca438622b212de8fd5732a89c302f19216b53ac9c633bf8bef43ce97f",
        ),
        (
            "app/clash-verge/shine.toml",
            "842c227f8e53e3ddd6113402b53f293b495d4833d2530fa79ef0f8337a04f5bf",
        ),
        (
            "app/docker-desktop/shine.toml",
            "9ed5a6b310a152639bbacc3cc157d2e408fe2c4187d9d8f92b346941f6b6314b",
        ),
        (
            "app/docker-engine/shine.toml",
            "658acb03b9dc488f214daa0b2b13dd2516028047de8952e30557de26cbbc63a0",
        ),
        (
            "app/fastfetch/shine.toml",
            "2dd7d716ddaaf13f07649a24f3ce580d23275649a8744d7359f66edf3ada3731",
        ),
        (
            "app/ghostty/shine.toml",
            "7c8201f5059a7bb3e81382cf5436a1da14906656a62aaa243cdc970b149f9ef4",
        ),
        (
            "app/surge/shine.toml",
            "5df30183647d35bb9359c9a09ad7efe94fe8b5c212b0d486ded7c6288b349034",
        ),
        (
            "app/surge/shine.toml",
            "ac5db93291294515aba2c8f457af1790d1520732e5a7c337fcbbd71a60144a23",
        ),
        (
            "app/vim/shine.toml",
            "75d27a891409dc484bb833ca8b1c192461ee993ff7ec49ea71e0d5c05f5735a1",
        ),
        (
            "shell/agent/shine.toml",
            "e8eb84b91e3dfd958a81cc36d2425a0847257f9e940932643edfff5e80d53fd9",
        ),
        (
            "shell/image-tools/shine.toml",
            "5d18cec5a585d58f897ad8aa74073c805f95c2af84db714ca09c54d02ce92a28",
        ),
        (
            "shell/proxy/shine.toml",
            "9e1dbfca07fab117c067ea8a8244cc7a8d64684e204fb5ecf8690f17fe743d6e",
        ),
        (
            "shell/utils/shine.toml",
            "b670ab168bc4eaf4cc75b20b7637a6c109bd0ea6db309464d6b0cde81497997d",
        ),
        (
            "sys/macos/shine.toml",
            "18cf178c1f3e8b6d456731356c62c6db3af005a388b647f10e38513c5d25d49d",
        ),
        (
            "sys/macos/shine.toml",
            "31d2809917dfb40d6c5642acbca4d4fb8a5d0514eb3b4208bc840becdedf735e",
        ),
        (
            "sys/ubuntu/shine.toml",
            "72788f06f29e7e554be80f25bfac1239b8759f89e504285f3045db12b6cc9a96",
        ),
        (
            "sys/ubuntu/shine.toml",
            "957133cacf805a4041e91bf08f86ff727415fd5165ca52a34c4da8c2044e9d57",
        ),
        (
            "sys/ubuntu/shine.toml",
            "f52d5ae3be3506269b1e8b0d34c5daab0fd5ea3dc87fa22b0500c05bbc4fd4b5",
        ),
        (
            "sys/ubuntu/shine.toml",
            "fb0ec4efa47a16618d63eab975066eb19a0157a682399505d0bae9f67e559769",
        ),
        (
            "sys/windows/shine.toml",
            "707ad4c963722a983c853705d1bd7ee4d0c72f00c3e8cb5f3bf0490e7d346708",
        ),
        (
            "sys/windows/shine.toml",
            "916f8f87ac7f36c37d2970417176e3a49ac07b4fa8dee40a3ed409e94feefce2",
        ),
        (
            "sys/windows/shine.toml",
            "edb1ec46dd84cb5ca4c799164b6f4e21591a1e988876bf1d3fdbc8fc20870ad8",
        ),
    ];
    let mut map = BTreeMap::<String, BTreeSet<String>>::new();
    for (path, hash) in ENTRIES {
        map.entry((*path).to_string())
            .or_default()
            .insert((*hash).to_string());
    }
    map
}
