//! Secret ownership and recoverable mutation. Platform I/O never decides ownership.
use super::*;
use crate::lifecycle::LifecycleOperation;
use crate::runtime::{AppCategory, AppPlanRequest, CoreRuntime};
use zeroize::Zeroizing;

pub const STORE: &str = "app-secret-manifest.json";
pub const JOURNAL: &str = "app-secret-operation.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretObservation {
    pub fingerprint: String,
    pub permissions: SecretFilePermissions,
    pub identity: String,
}

/// Primitives only. The domain owns release selection, conflict rules and recovery.
pub trait SecretFileIo: Send + Sync {
    fn resolve_permissions(&self, permissions: &SecretFilePermissions) -> SecretFilePermissions;
    fn observe(&self, path: &Path) -> Result<Option<SecretObservation>>;
    fn fingerprint(&self, bytes: &[u8]) -> Result<String>;
    fn state(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>>;
    fn save_state(&self, name: &str, bytes: Option<&[u8]>) -> Result<()>;
    fn prepare_key(&self) -> Result<()>;
    fn lock_paths(&self, paths: &[PathBuf]) -> Result<Box<dyn Send>>;
    fn create(&self, path: &Path, bytes: &[u8], permissions: &SecretFilePermissions) -> Result<()>;
    fn set_permissions(&self, path: &Path, permissions: &SecretFilePermissions) -> Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> Result<()>;
    fn remove(&self, path: &Path) -> Result<()>;
    fn check_target(&self, path: &Path, permissions: &SecretFilePermissions) -> Result<()>;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretEntry {
    pub category: String,
    pub source: PathBuf,
    pub target: PathBuf,
    pub source_version: String,
    pub fingerprint: String,
    pub permissions: SecretFilePermissions,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretStore {
    pub version: u32,
    pub entries: Vec<SecretEntry>,
}
impl Default for SecretStore {
    fn default() -> Self {
        Self {
            version: 2,
            entries: Vec::new(),
        }
    }
}
impl SecretStore {
    pub fn load(io: &dyn SecretFileProvider) -> Result<Self> {
        let Some(bytes) = io.state(STORE)? else {
            return Ok(Self::default());
        };
        let state: Self =
            serde_json::from_slice(&bytes).context("invalid private App secret manifest")?;
        ensure!(
            state.version == 2,
            "unsupported App secret manifest version"
        );
        let mut paths = BTreeSet::new();
        for entry in &state.entries {
            ensure!(
                entry.target.is_absolute() && paths.insert(path_key(&entry.target)),
                "invalid/duplicate secret receipt target"
            );
        }
        Ok(state)
    }
    fn save(&self, io: &dyn SecretFileProvider) -> Result<()> {
        io.save_state(STORE, Some(&serde_json::to_vec(self)?))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    target: PathBuf,
    rollback: PathBuf,
    before: Option<SecretObservation>,
    protected: Option<SecretObservation>,
    applied: Option<SecretObservation>,
    previous: Option<SecretEntry>,
    desired: Option<SecretEntry>,
}

struct RecoveryFacts {
    journal: Journal,
    store: SecretStore,
    current: Option<SecretObservation>,
    rollback: Option<SecretObservation>,
    committed: bool,
}

struct Pending {
    target: PathBuf,
    previous: Option<SecretEntry>,
    before: Option<SecretObservation>,
    desired: Option<SecretEntry>,
    bytes: Option<Zeroizing<Vec<u8>>>,
}
pub(crate) struct PreparedSecrets {
    pending: Vec<Pending>,
    _paths: Box<dyn Send>,
}

impl PreparedSecrets {
    pub(crate) fn has_changes(&self) -> bool {
        !self.pending.is_empty()
    }
}

pub(crate) fn rollback_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".shine.secret.rollback");
    PathBuf::from(name)
}
pub(crate) fn source_version(
    snapshot: &PresetSnapshot,
    category: &str,
    file: &AppSecretFile,
) -> Result<String> {
    let prefix = format!("app/{category}/");
    let cipher = snapshot
        .get(&format!(
            "{prefix}{}",
            crate::runtime::path_text::preset_path(&file.source)
        ))
        .context("ciphertext missing")?;
    let workspace = snapshot
        .get(&format!(
            "{prefix}{}",
            crate::runtime::path_text::preset_path(&file.workspace)
        ))
        .context("workspace missing")?;
    Ok(crate::digest::sha256_hex(&serde_json::to_vec(&(
        crate::digest::sha256_hex(workspace),
        crate::digest::sha256_hex(cipher),
        &file.mode,
        &file.group,
        &file.permissions,
    ))?))
}
pub(crate) fn matches(entry: &SecretEntry, observation: &SecretObservation) -> bool {
    entry.fingerprint == observation.fingerprint && entry.permissions == observation.permissions
}

impl<H> CoreRuntime<H> {
    pub fn secret_store(&self) -> Result<SecretStore> {
        match &self.secret_provider {
            Some(io) => SecretStore::load(io.as_ref()),
            None => Ok(SecretStore::default()),
        }
    }
    pub(crate) fn secret_observe(&self, path: &Path) -> Result<Option<SecretObservation>> {
        match &self.secret_provider {
            Some(io) => io.observe(path),
            None => Ok(None),
        }
    }
    pub(crate) fn secret_journal(&self) -> Result<Option<Zeroizing<Vec<u8>>>> {
        match &self.secret_provider {
            Some(io) => io.state(JOURNAL),
            None => Ok(None),
        }
    }
    pub fn app_file_release_requests(
        &self,
        category: &AppCategory,
        operation: &str,
    ) -> Result<Vec<FileReleaseRequest>> {
        let prefix = format!("app/{}/", category.name);
        let metadata = std::str::from_utf8(
            self.presets()
                .get(&format!("{prefix}shine.toml"))
                .context("App metadata missing")?,
        )?
        .to_string();
        let snapshot_sha256 = crate::digest::sha256_hex(&serde_json::to_vec(
            &self
                .presets()
                .files()
                .iter()
                .filter(|(p, _)| p.starts_with(&prefix))
                .collect::<BTreeMap<_, _>>(),
        )?);
        let mut requests: BTreeMap<(PathBuf, String), FileReleaseRequest> = BTreeMap::new();
        for file in category.secret_files.iter().filter(|f| f.auto_decrypt) {
            let workspace_target = self.secret_destination(category, &file.workspace)?;
            let request = requests
                .entry((file.workspace.clone(), file.mode.clone()))
                .or_insert_with(|| FileReleaseRequest {
                    operation: operation.into(),
                    workspace_path: workspace_target.to_string_lossy().into(),
                    workspace_contents: String::new(),
                    mode: file.mode.clone(),
                    groups: Vec::new(),
                    release: Vec::new(),
                    ciphertexts: BTreeMap::new(),
                    permissions: BTreeMap::new(),
                    app: Some(AppReleaseBinding {
                        category: category.name.clone(),
                        metadata: metadata.clone(),
                        snapshot_sha256: snapshot_sha256.clone(),
                    }),
                });
            request.workspace_contents = std::str::from_utf8(
                self.presets()
                    .get(&format!(
                        "{prefix}{}",
                        crate::runtime::path_text::preset_path(&file.workspace)
                    ))
                    .context("workspace missing")?,
            )?
            .into();
            if !request.groups.contains(&file.group) {
                request.groups.push(file.group.clone());
            }
            let parent = file
                .workspace
                .parent()
                .context("workspace parent missing")?;
            let source = crate::runtime::path_text::preset_path(file.source.strip_prefix(parent)?);
            let target = crate::runtime::path_text::preset_path(file.target.strip_prefix(parent)?);
            request.release.push(source.clone());
            request.ciphertexts.insert(
                source,
                std::str::from_utf8(
                    self.presets()
                        .get(&format!(
                            "{prefix}{}",
                            crate::runtime::path_text::preset_path(&file.source)
                        ))
                        .context("ciphertext missing")?,
                )?
                .into(),
            );
            request.permissions.insert(target, file.permissions.clone());
        }
        for request in requests.values() {
            request.mappings()?;
        }
        Ok(requests.into_values().collect())
    }
    pub(crate) fn secret_destination(
        &self,
        category: &AppCategory,
        relative: &Path,
    ) -> Result<PathBuf> {
        let file = AppFile {
            source_rel: relative.into(),
            target_rel: relative.into(),
            destination_root: None,
            description: None,
            display_name: None,
            legacy_dest_annotation: None,
            transforms: Vec::new(),
            install_strategy: AppInstallStrategy::Copy,
            requires_admin: false,
            restart_hint: None,
            generator: None,
        };
        self.app_destination(category, &file)
    }
    pub(crate) async fn prepare_secrets(
        &self,
        request: &AppPlanRequest,
        plan: &crate::plan::PlanV1,
    ) -> Result<Option<PreparedSecrets>> {
        let store = self.secret_store()?;
        let categories = if request.operation == LifecycleOperation::Uninstall {
            Vec::new()
        } else {
            self.app_categories(request.target.as_deref())?
        };
        if store.entries.is_empty() && categories.iter().all(|c| c.secret_files.is_empty()) {
            return Ok(None);
        }
        let io = self
            .secret_provider
            .as_ref()
            .context("App secret file provider is unavailable")?;
        ensure!(
            io.state(JOURNAL)?.is_none(),
            "App secret recovery required; run shine app recover"
        );
        let selected = |e: &&SecretEntry| request.target.as_ref().is_none_or(|c| c == &e.category);
        let mut paths = store
            .entries
            .iter()
            .filter(selected)
            .map(|e| e.target.clone())
            .collect::<Vec<_>>();
        for category in &categories {
            for file in &category.secret_files {
                paths.push(self.secret_destination(category, &file.target)?);
            }
        }
        let locks = io.lock_paths(&paths)?;
        let mut pending = Vec::new();
        let mut total_plaintext = 0usize;
        let mut total_ciphertext = 0usize;
        let mut active = BTreeSet::new();
        if request.operation != LifecycleOperation::Uninstall {
            for category in &categories {
                let mut desired = Vec::new();
                for file in &category.secret_files {
                    active.insert((category.name.clone(), file.source.clone()));
                    let target = self.secret_destination(category, &file.target)?;
                    let previous = store
                        .entries
                        .iter()
                        .find(|e| e.category == category.name && e.source == file.source)
                        .cloned();
                    ensure!(
                        previous.as_ref().is_none_or(|e| e.target == target),
                        "secret destination changed; explicitly uninstall the old target first"
                    );
                    ensure!(
                        !store
                            .entries
                            .iter()
                            .any(|e| path_key(&e.target) == path_key(&target)
                                && (e.category != category.name || e.source != file.source)),
                        "secret target owned by another resource"
                    );
                    if !file.auto_decrypt {
                        continue;
                    }
                    if !plan.steps.iter().any(|s| {
                        s.target == format!("app/{}", category.name)
                            && s.resource.as_deref()
                                == Some(format!("secret:{}", file.target.display()).as_str())
                            && matches!(
                                s.action,
                                crate::plan::PlanActionV1::Create
                                    | crate::plan::PlanActionV1::Update
                            )
                    }) {
                        continue;
                    }
                    io.check_target(&target, &file.permissions)?;
                    let before = io.observe(&target)?;
                    if let (Some(entry), Some(current)) = (&previous, &before) {
                        if !matches(entry, current) && !request.force {
                            continue;
                        }
                        if matches(entry, current)
                            && entry.source_version
                                == source_version(self.presets(), &category.name, file)?
                        {
                            continue;
                        }
                    }
                    desired.push((file, target, previous, before));
                }
                if desired.is_empty() {
                    continue;
                }
                // Capture and decrypt the complete selected group, but write only reviewed targets.
                let operation = if request.operation == LifecycleOperation::Upgrade {
                    "upgrade"
                } else {
                    "install"
                };
                let mut released = BTreeMap::new();
                for mut release in self.app_file_release_requests(category, operation)? {
                    let release_root = Path::new(&release.workspace_path)
                        .parent()
                        .context("workspace parent missing")?
                        .to_owned();
                    let targets = desired
                        .iter()
                        .map(|(_, t, _, _)| t)
                        .collect::<BTreeSet<_>>();
                    release.release = release
                        .mappings()?
                        .iter()
                        .filter(|m| targets.contains(&release_root.join(&m.target)))
                        .map(|m| m.source.clone())
                        .collect();
                    if release.release.is_empty() {
                        continue;
                    }
                    total_ciphertext +=
                        release.ciphertexts.values().map(String::len).sum::<usize>();
                    ensure!(
                        total_ciphertext <= MAX_CIPHER_TOTAL,
                        "selected ciphertext exceeds 128 MiB"
                    );
                    let bytes = io.decrypt(&release).await?;
                    for file in &bytes {
                        ensure!(
                            file.len() <= MAX_FILE,
                            "secret provider file exceeds 16 MiB"
                        );
                        total_plaintext += file.len();
                        ensure!(
                            total_plaintext <= MAX_TOTAL,
                            "selected plaintext exceeds 64 MiB"
                        );
                    }
                    let mappings = release.released_mappings()?;
                    ensure!(
                        bytes.len() == mappings.len(),
                        "incomplete secret provider response"
                    );
                    let root = Path::new(&release.workspace_path)
                        .parent()
                        .context("workspace parent missing")?;
                    for (mapping, bytes) in mappings.into_iter().zip(bytes) {
                        released.insert(root.join(mapping.target), bytes);
                    }
                }
                for (file, target, previous, before) in desired {
                    let bytes = released
                        .remove(&target)
                        .context("secret provider omitted a file")?;
                    let fingerprint = io.fingerprint(&bytes)?;
                    if previous.is_none()
                        && let Some(current) = &before
                    {
                        ensure!(
                            current.fingerprint == fingerprint,
                            "existing secret differs from decrypted content; refusing adoption"
                        );
                    }
                    let permissions = io.resolve_permissions(&file.permissions);
                    let desired = SecretEntry {
                        category: category.name.clone(),
                        source: file.source.clone(),
                        target: target.clone(),
                        source_version: source_version(self.presets(), &category.name, file)?,
                        fingerprint,
                        permissions,
                    };
                    pending.push(Pending {
                        target,
                        previous,
                        before,
                        desired: Some(desired),
                        bytes: Some(bytes),
                    });
                }
            }
        }
        for entry in store.entries.iter().filter(selected) {
            if request.operation != LifecycleOperation::Uninstall
                && (!request.prune_stale
                    || active.contains(&(entry.category.clone(), entry.source.clone())))
            {
                continue;
            }
            if !plan.steps.iter().any(|s| {
                s.target == format!("app/{}", entry.category)
                    && s.resource.as_deref()
                        == Some(format!("secret:{}", entry.target.display()).as_str())
                    && s.action == crate::plan::PlanActionV1::Remove
            }) {
                continue;
            }
            let before = io.observe(&entry.target)?;
            if before.as_ref().is_some_and(|o| !matches(entry, o)) && !request.force {
                continue;
            }
            pending.push(Pending {
                target: entry.target.clone(),
                previous: Some(entry.clone()),
                before,
                desired: None,
                bytes: None,
            });
        }
        for item in &pending {
            ensure!(
                io.observe(&rollback_path(&item.target))?.is_none(),
                "secret rollback path is occupied"
            );
        }
        Ok(Some(PreparedSecrets {
            pending,
            _paths: locks,
        }))
    }

    pub(crate) fn apply_secrets(
        &self,
        prepared: Option<PreparedSecrets>,
    ) -> Result<BTreeSet<String>> {
        let Some(prepared) = prepared else {
            return Ok(BTreeSet::new());
        };
        let io = self
            .secret_provider
            .as_ref()
            .context("secret provider missing")?;
        if prepared.pending.is_empty() {
            return Ok(BTreeSet::new());
        }
        io.prepare_key()?;
        let mut store = SecretStore::load(io.as_ref())?;
        let mut changed = BTreeSet::new();
        for item in prepared.pending {
            ensure!(
                io.observe(&item.target)? == item.before,
                "secret target changed after review"
            );
            let mut journal = Journal {
                version: 2,
                target: item.target.clone(),
                rollback: rollback_path(&item.target),
                protected: item.before.as_ref().map(|before| SecretObservation {
                    permissions: io.resolve_permissions(&SecretFilePermissions {
                        uid: None,
                        gid: None,
                        mode: 0o600,
                    }),
                    ..before.clone()
                }),
                before: item.before,
                applied: None,
                previous: item.previous,
                desired: item.desired,
            };
            ensure!(
                io.observe(&journal.rollback)?.is_none(),
                "secret rollback path is occupied"
            );
            io.save_state(JOURNAL, Some(&serde_json::to_vec(&journal)?))?;
            if let Some(protected) = &journal.protected {
                io.set_permissions(&journal.target, &protected.permissions)?;
                ensure!(
                    io.observe(&journal.target)?.as_ref() == Some(protected),
                    "secret changed while securing rollback"
                );
                io.rename(&journal.target, &journal.rollback)?;
            }
            if let (Some(bytes), Some(entry)) = (item.bytes.as_ref(), journal.desired.as_ref()) {
                io.create(&journal.target, bytes, &entry.permissions)?;
            }
            if let Some(entry) = &journal.desired {
                let observed = io
                    .observe(&journal.target)?
                    .context("secret disappeared before receipt commit")?;
                ensure!(
                    matches(entry, &observed),
                    "secret changed before receipt commit"
                );
                journal.applied = Some(observed.clone());
                io.save_state(JOURNAL, Some(&serde_json::to_vec(&journal)?))?;
                let mut entry = entry.clone();
                entry.permissions = observed.permissions;
                store.entries.retain(|e| e.target != journal.target);
                store.entries.push(entry.clone());
                changed.insert(entry.category);
            } else {
                ensure!(
                    io.observe(&journal.target)?.is_none(),
                    "removed secret target was replaced"
                );
                if let Some(entry) = &journal.previous {
                    changed.insert(entry.category.clone());
                }
                store.entries.retain(|e| e.target != journal.target);
            }
            store.save(io.as_ref())?;
            ensure!(
                io.observe(&journal.target)? == journal.applied,
                "secret changed after receipt commit; recovery required"
            );
            if let Some(protected) = &journal.protected {
                let current = io
                    .observe(&journal.rollback)?
                    .context("secret rollback disappeared")?;
                ensure!(
                    current == *protected,
                    "secret rollback changed; recovery required"
                );
                io.remove(&journal.rollback)?;
            }
            io.save_state(JOURNAL, None)?;
        }
        Ok(changed)
    }

    fn secret_recovery_facts(&self) -> Result<Option<RecoveryFacts>> {
        let Some(bytes) = self.secret_journal()? else {
            return Ok(None);
        };
        let journal: Journal =
            serde_json::from_slice(&bytes).context("invalid secret recovery journal")?;
        ensure!(
            journal.version == 2
                && journal.target.is_absolute()
                && journal.rollback == rollback_path(&journal.target),
            "invalid secret recovery contract"
        );
        let store = self.secret_store()?;
        let receipt = store.entries.iter().find(|e| e.target == journal.target);
        let committed = receipt == journal.desired.as_ref()
            && (journal.previous != journal.desired || journal.applied.is_some());
        ensure!(
            committed || receipt == journal.previous.as_ref(),
            "secret receipt changed; recovery blocked"
        );
        let current = self.secret_observe(&journal.target)?;
        let rollback = self.secret_observe(&journal.rollback)?;
        Ok(Some(RecoveryFacts {
            journal,
            store,
            current,
            rollback,
            committed,
        }))
    }

    pub(crate) fn plan_secret_recovery(&self) -> Result<Option<crate::plan::PlanV1>> {
        use crate::plan::*;
        let Some(RecoveryFacts {
            journal,
            store,
            current,
            rollback,
            committed,
        }) = self.secret_recovery_facts()?
        else {
            return Ok(None);
        };
        let desired_matches = match (&current, &journal.desired) {
            (None, None) => true,
            (Some(o), Some(e)) => matches(e, o),
            _ => false,
        };
        // A missing observation is not evidence of ownership. A crash (or failed
        // journal write) after creation leaves an indistinguishable replacement.
        let applied_matches = current == journal.applied;
        let safe = if committed {
            desired_matches
                && applied_matches
                && (rollback.is_none() || rollback == journal.protected)
        } else if rollback.is_none() {
            current == journal.before
                || current == journal.protected
                || (journal.before.is_none() && desired_matches && applied_matches)
        } else {
            rollback == journal.protected
                && (current.is_none() || (desired_matches && applied_matches))
        };
        let mut state = SnapshotDigestV1::builder("state:app-secret-recovery-v2");
        state.add_observation(
            "facts",
            serde_json::to_vec(&(&journal, &store, &current, &rollback))?,
        )?;
        let mut permissions = PermissionSetV1::default();
        for path in [
            &journal.target,
            &journal.rollback,
            &self.context().shine_dir.join(JOURNAL),
        ] {
            for access in [FilesystemAccessV1::Write, FilesystemAccessV1::Remove] {
                permissions.insert(PermissionV1::Filesystem {
                    access,
                    path: crate::runtime::review_path::review_path(self.context(), path),
                });
            }
        }
        if !journal.target.starts_with(&self.context().home_dir) {
            permissions.insert(PermissionV1::Administrator);
        }
        Ok(Some(PlanV1::new(
            PlanOperationV1::AppRecovery,
            PlanInputsV1 {
                preset: self.presets().digest_v1()?,
                state: state.finish(),
            },
            vec![
                PlanStepV1::new(
                    "app",
                    Some("secret-operation"),
                    if safe {
                        PlanActionV1::Update
                    } else {
                        PlanActionV1::Blocked
                    },
                )
                .with_diagnostic_code(if safe {
                    "app_secret_recovery"
                } else {
                    "app_secret_recovery_conflict"
                }),
            ],
            permissions.clone(),
            &permissions,
            std::iter::empty::<String>(),
        )))
    }

    pub(crate) fn recover_secrets(
        &self,
        approval: &crate::plan::PlanApprovalV1,
    ) -> Result<crate::runtime::AppRecoveryReportV1> {
        let io = self
            .secret_provider
            .as_ref()
            .context("secret provider missing")?;
        let Some(RecoveryFacts { journal, .. }) = self.secret_recovery_facts()? else {
            bail!("no secret recovery journal");
        };
        let _paths = io.lock_paths(&[journal.target.clone(), journal.rollback.clone()])?;
        approval.validate(
            &self
                .plan_secret_recovery()?
                .context("secret journal disappeared")?,
        )?;
        let RecoveryFacts {
            journal,
            current,
            rollback,
            committed,
            ..
        } = self
            .secret_recovery_facts()?
            .context("secret journal disappeared")?;
        if committed {
            if rollback.is_some() {
                io.remove(&journal.rollback)?;
            }
        } else if rollback.is_some() {
            if current.is_some() {
                io.remove(&journal.target)?;
            }
            io.rename(&journal.rollback, &journal.target)?;
            if let Some(before) = &journal.before {
                io.set_permissions(&journal.target, &before.permissions)?;
            }
        } else if let Some(before) = &journal.before {
            if current == journal.protected {
                io.set_permissions(&journal.target, &before.permissions)?;
            }
        } else if current.is_some() {
            io.remove(&journal.target)?;
        }
        io.save_state(JOURNAL, None)?;
        Ok(crate::runtime::AppRecoveryReportV1 {
            operation_id: "app-secret-operation".into(),
            rolled_back_actions: if committed {
                Vec::new()
            } else {
                vec!["secret-file".into()]
            },
        })
    }
}

impl<H> CoreRuntime<H> {
    pub(crate) fn secret_report_rows(
        &self,
        plan: &crate::plan::PlanV1,
        preview: bool,
    ) -> Result<Vec<crate::runtime::AppFileLifecycleReport>> {
        use crate::lifecycle::LifecycleStatus as Status;
        use crate::plan::PlanActionV1;
        use crate::runtime::{AppFileAction as Action, AppFileLifecycleReport};
        let mut rows = Vec::new();
        for step in &plan.steps {
            let Some(resource) = step
                .resource
                .as_deref()
                .and_then(|s| s.strip_prefix("secret:"))
            else {
                continue;
            };
            let Some(category) = step.target.strip_prefix("app/") else {
                continue;
            };
            let destination = if Path::new(resource).is_absolute() {
                PathBuf::from(resource)
            } else {
                let categories = self.app_categories(Some(category))?;
                self.secret_destination(
                    categories
                        .first()
                        .context("secret report category missing")?,
                    Path::new(resource),
                )?
            };
            let (action, status) = match step.action {
                PlanActionV1::Create | PlanActionV1::Update if preview => {
                    (Action::PreviewInstall, Status::Previewed)
                }
                PlanActionV1::Create | PlanActionV1::Update => (Action::Installed, Status::Changed),
                PlanActionV1::Remove if preview => (Action::PreviewRemove, Status::Previewed),
                PlanActionV1::Remove => (Action::Removed, Status::Changed),
                PlanActionV1::Preserve | PlanActionV1::Blocked => {
                    (Action::UserModified, Status::Preserved)
                }
                _ => (Action::Unchanged, Status::Skipped),
            };
            rows.push(AppFileLifecycleReport {
                category: category.into(),
                source: PathBuf::from(format!("secret:{resource}")),
                destination,
                transforms: vec!["secret-file".into()],
                backup: None,
                uninstall_strategy: None,
                restart_hint: None,
                generator_error: None,
                error: None,
                status,
                action,
            });
        }
        Ok(rows)
    }
    pub(crate) fn secret_report_outcome(
        &self,
        row: &crate::runtime::AppFileLifecycleReport,
    ) -> crate::lifecycle::LifecycleOutcomeV1 {
        use crate::lifecycle::{LifecycleEffect as Effect, LifecycleOutcomeV1};
        use crate::runtime::AppFileAction;
        let effects = match row.action {
            AppFileAction::Installed => vec![Effect::ResourceWritten, Effect::ReceiptWritten],
            AppFileAction::Removed => vec![Effect::ResourceRemoved, Effect::ReceiptRemoved],
            AppFileAction::UserModified => vec![Effect::UserResourcePreserved],
            AppFileAction::PreviewInstall => vec![Effect::ResourceWritePreviewed],
            AppFileAction::PreviewRemove => vec![Effect::ResourceRemovePreviewed],
            _ => Vec::new(),
        };
        LifecycleOutcomeV1::new(
            format!("app/{}", row.category),
            Some(row.source.display().to_string()),
            row.status,
            effects,
        )
        .with_diagnostic_code("app_secret_file")
    }

    pub(crate) fn inspect_secret_files(
        &self,
        categories: &[AppCategory],
    ) -> Result<Vec<crate::runtime::AppFileInspection>> {
        use crate::runtime::{AppFileInspection, InspectionChange, InspectionFileStatus as Status};
        let store = self.secret_store()?;
        let mut result = Vec::new();
        for category in categories {
            for file in &category.secret_files {
                let destination = self.secret_destination(category, &file.target)?;
                let entry = store
                    .entries
                    .iter()
                    .find(|e| e.category == category.name && e.source == file.source);
                let current = self.secret_observe(&destination)?;
                let status = match (entry, &current) {
                    (None, _) => Status::NotInstalled,
                    (Some(_), None) => Status::Missing,
                    (Some(e), Some(o)) if !matches(e, o) => Status::UserModified,
                    (Some(e), Some(_))
                        if file.auto_decrypt
                            && e.source_version
                                != source_version(self.presets(), &category.name, file)? =>
                    {
                        Status::UpdateAvail
                    }
                    _ => Status::UpToDate,
                };
                result.push(AppFileInspection {
                    category: category.clone(),
                    file: AppFile {
                        source_rel: PathBuf::from(format!("secret:{}", file.target.display())),
                        target_rel: file.target.clone(),
                        destination_root: None,
                        description: Some("Managed secret file (content redacted)".into()),
                        display_name: None,
                        legacy_dest_annotation: None,
                        transforms: Vec::new(),
                        install_strategy: AppInstallStrategy::Copy,
                        requires_admin: file.permissions.uid.is_some(),
                        restart_hint: None,
                        generator: None,
                    },
                    destination: Some(destination),
                    status,
                    manifest_entry: None,
                    desired_content: None,
                    current_content: None,
                    changes: if status == Status::UpdateAvail {
                        vec![InspectionChange::ContentChanged]
                    } else {
                        Vec::new()
                    },
                    code_analysis: None,
                    assessment_error: None,
                    assessment_diagnostic: Some("app_secret_content_redacted"),
                });
            }
        }
        Ok(result)
    }
}
