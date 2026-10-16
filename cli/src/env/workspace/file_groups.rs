//! Explicit, snapshot-bound binary secret files. No plaintext cache or implicit release.
mod filesystem;
mod platform;

use super::*;
use crate::commands::{EnvRunCommand, EnvSealCommand, EnvWorkspaceExportCommand};
use anyhow::ensure;
use filesystem::{CapturedFile, Materialized, PathLocks, check_git, check_path};
use std::fs;

pub(super) const MAX_FILE: usize = 16 * 1024 * 1024;
const MAX_TOTAL: usize = 64 * 1024 * 1024;
pub(super) const MAX_CIPHER: usize = 32 * 1024 * 1024;
const MAX_CIPHER_TOTAL: usize = 128 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileGroup {
    pub files: Vec<Mapping>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Mapping {
    source: String,
    target: String,
}

#[derive(Clone, Copy)]
enum FileGroupOperation {
    Seal,
    Run,
    Export,
}

impl FileGroupOperation {
    fn input<'a>(self, source: &'a Path, target: &'a Path) -> (&'a Path, usize) {
        match self {
            Self::Seal => (target, MAX_FILE),
            Self::Run | Self::Export => (source, MAX_CIPHER),
        }
    }
    fn verb(self) -> &'static str {
        match self {
            Self::Seal => "seal",
            Self::Run => "decrypt temporarily",
            Self::Export => "export plaintext",
        }
    }
}

#[derive(Clone, Copy)]
enum PreviewOutput {
    Stdout,
    Stderr,
}
impl PreviewOutput {
    fn emit(self, line: String) {
        match self {
            Self::Stdout => println!("{line}"),
            Self::Stderr => eprintln!("{line}"),
        }
    }
}

struct Selection {
    workspace_path: PathBuf,
    workspace_bytes: String,
    workspace: Workspace,
    root: PathBuf,
    mode: String,
    mappings: Vec<(PathBuf, PathBuf)>,
}

impl Selection {
    async fn load(workspace: Option<&Path>, mode: Option<&str>, groups: &[String]) -> Result<Self> {
        let path = find_workspace_optional(workspace)
            .await?
            .context("workspace required")?;
        // Canonicalize only the workspace parent, so linked workspace files remain visible.
        let root = fs::canonicalize(path.parent().context("workspace parent required")?)?;
        let workspace_path = root.join(path.file_name().context("workspace filename required")?);
        check_path(&root, &workspace_path)?;
        let workspace_bytes = fs::read_to_string(&workspace_path)?;
        let workspace = parse_workspace(&workspace_path, &workspace_bytes)?;
        ensure!(
            workspace.version == 3,
            "file groups require workspace version 3"
        );
        let mode = mode
            .or(workspace.env.default_mode.as_deref())
            .context("pass --mode or set env.default_mode")?
            .to_owned();
        validate_mode(&mode)?;
        ensure!(
            workspace.env.modes.contains(&mode)
                || workspace.env.default_mode.as_ref() == Some(&mode),
            "undeclared file-group mode {mode}"
        );
        let mut mappings = Vec::new();
        let mut seen = BTreeSet::new();
        for name in groups {
            if !seen.insert(name) {
                continue;
            }
            let group = workspace
                .file_groups
                .get(name)
                .with_context(|| format!("unknown file group {name}"))?;
            for mapping in &group.files {
                let source = filesystem::resolve(&root, &mapping.source, &mode)?;
                let target = filesystem::resolve(&root, &mapping.target, &mode)?;
                mappings.push((source, target));
            }
        }
        let workspace_key = PathBuf::from(
            String::from_utf8_lossy(&platform::path_key(&workspace_path)).into_owned(),
        );
        let paths = mappings
            .iter()
            .flat_map(|(s, t)| [s, t])
            .map(|path| {
                PathBuf::from(String::from_utf8_lossy(&platform::path_key(path)).into_owned())
            })
            .collect::<Vec<_>>();
        for (i, a) in paths.iter().enumerate() {
            ensure!(
                !a.starts_with(&workspace_key) && !workspace_key.starts_with(a),
                "mapping overlaps workspace file"
            );
            for b in &paths[i + 1..] {
                ensure!(
                    !a.starts_with(b) && !b.starts_with(a),
                    "file-group mappings overlap: {} and {}",
                    a.display(),
                    b.display()
                );
            }
        }
        Ok(Self {
            workspace_path,
            workspace_bytes,
            workspace,
            root,
            mode,
            mappings,
        })
    }

    fn verify(&self) -> Result<()> {
        check_path(&self.root, &self.workspace_path)?;
        ensure!(
            fs::read_to_string(&self.workspace_path)? == self.workspace_bytes,
            "workspace changed during file-group operation"
        );
        for (source, target) in &self.mappings {
            check_path(&self.root, source)?;
            check_path(&self.root, target)?;
            check_git(&self.root, target)?;
        }
        Ok(())
    }

    fn preflight(&self, operation: FileGroupOperation) -> Result<()> {
        self.verify()?;
        for (source, target) in &self.mappings {
            let (required, limit) = operation.input(source, target);
            ensure!(
                required.is_file(),
                "required file missing: {}",
                required.display()
            );
            ensure!(
                fs::metadata(required)?.len() <= limit as u64,
                "file exceeds size limit: {}",
                required.display()
            );
            if matches!(
                operation,
                FileGroupOperation::Run | FileGroupOperation::Export
            ) {
                ensure!(
                    !target.try_exists()?,
                    "plaintext target already exists; inspect it before retrying: {}",
                    target.display()
                );
            }
        }
        Ok(())
    }

    fn preview(&self, operation: FileGroupOperation, output: PreviewOutput) {
        output.emit(format!(
            "File groups for mode {} (metadata only)",
            self.mode
        ));
        for (source, target) in &self.mappings {
            let (from, to) = match operation {
                FileGroupOperation::Seal => (target, source),
                FileGroupOperation::Run | FileGroupOperation::Export => (source, target),
            };
            output.emit(format!(
                "Would {} {} -> {}",
                operation.verb(),
                from.display(),
                to.display()
            ));
        }
    }
}

/// Seal target plaintext into source ciphertext; ordinary env sealing stays separate.
pub async fn handle_seal(config: &Config, command: &EnvSealCommand) -> Result<()> {
    platform::prepare(seal_inner(config, command)).await
}

async fn seal_inner(config: &Config, command: &EnvSealCommand) -> Result<()> {
    let selected = Selection::load(
        command.workspace.as_deref(),
        command.mode.as_deref(),
        &command.file_groups,
    )
    .await?;
    selected.preview(FileGroupOperation::Seal, PreviewOutput::Stdout);
    selected.preflight(FileGroupOperation::Seal)?;
    if command.dry_run {
        return Ok(());
    }
    let _locks = PathLocks::acquire(&selected.root, &selected.workspace_path, &selected.mappings)?;
    selected.preflight(FileGroupOperation::Seal)?;
    let mut config = local_cache::load_config(config, &selected.workspace_path).await?;
    let mut recipients = resolve_seal_encryption(
        command.backend.as_deref(),
        &command.recipients,
        Some(&selected.workspace.env.encryption),
        &config,
    )?
    .context("file sealing requires encryption recipients")?;
    let mut plain = Vec::new();
    let mut previous = Vec::new();
    let mut cipher_total = 0;
    let mut total = 0;
    for (source, target) in &selected.mappings {
        let file = CapturedFile::read(&selected.root, target, MAX_FILE.min(MAX_TOTAL - total))?;
        total += file.bytes.len();
        ensure!(total <= MAX_TOTAL, "selected plaintext exceeds 64 MiB");
        plain.push(file);
        let old = CapturedFile::optional(
            &selected.root,
            source,
            MAX_CIPHER.min(MAX_CIPHER_TOTAL - cipher_total),
        )?;
        if let Some(old) = &old {
            cipher_total += old.bytes.len();
            validate_cipher(&old.bytes)?;
        }
        previous.push(old);
    }
    match &mut recipients {
        EncryptRecipients::Hybrid(recipients) => recipients.prepare().await?,
        EncryptRecipients::Age(recipients) => secret::preflight_age_recipients(recipients).await?,
        EncryptRecipients::Gpg(_) => (),
    }
    let mut encrypted = Vec::new();
    let mut encrypted_total = 0;
    for file in &plain {
        let value = secret::encrypt_bytes(&file.bytes, &recipients)
            .await
            .map_err(|_| anyhow::anyhow!("file encryption failed or was cancelled"))?;
        ensure!(value.len() <= MAX_CIPHER, "ciphertext exceeds size limit");
        encrypted_total += value.len();
        ensure!(
            encrypted_total <= MAX_CIPHER_TOTAL,
            "selected ciphertext exceeds 128 MiB"
        );
        validate_cipher(value.as_bytes())?;
        encrypted.push(value);
    }
    selected.verify()?;
    for file in &plain {
        file.verify(&selected.root)?;
    }
    for (index, (source, _)) in selected.mappings.iter().enumerate() {
        let result = async {
            selected.verify()?;
            plain[index].verify(&selected.root)?;
            filesystem::verify_optional(
                &selected.root,
                source,
                previous[index].as_ref(),
                MAX_CIPHER,
            )?;
            filesystem::write_cipher(
                &selected.root,
                source,
                encrypted[index].as_bytes(),
                previous[index].take(),
            )
            .await
        }
        .await;
        result.with_context(|| format!("sealing stopped: {index} completed; {} and later files not confirmed; plaintext retained", source.display()))?;
        println!("Sealed {}", source.display());
    }
    if command.remove_plaintext {
        if encrypted.iter().any(|value| value.starts_with("hybrid:"))
            && config.hybrid_decrypt_backend.is_none()
        {
            let backend = secret::hybrid::select(&config.resolved_age_identities(), None).await?;
            config.hybrid_decrypt_backend = Some(
                if backend == BackendKind::Gpg {
                    "gpg"
                } else {
                    "age"
                }
                .into(),
            );
        }
        let mut written = Vec::new();
        for ((source, _), expected) in selected.mappings.iter().zip(&plain) {
            let file = CapturedFile::read(&selected.root, source, MAX_CIPHER)?;
            let text = validate_cipher(&file.bytes)?;
            let decoded = secret::decrypt_bytes(text, &config, MAX_FILE)
                .await
                .map_err(|_| {
                    anyhow::anyhow!("verification failed or was cancelled; plaintext retained")
                })?;
            ensure!(
                *decoded == *expected.bytes,
                "ciphertext verification mismatch; plaintext retained"
            );
            written.push(file);
        }
        selected.verify()?;
        for file in plain.iter().chain(&written) {
            file.verify(&selected.root)?;
        }
        for (removed, file) in plain.iter().enumerate() {
            let result = (|| {
                selected.verify()?;
                for written in &written {
                    written.verify(&selected.root)?;
                }
                file.verify(&selected.root)?;
                fs::remove_file(&file.path)?;
                Ok::<_, anyhow::Error>(())
            })();
            result.with_context(|| {
                format!(
                    "plaintext cleanup stopped: {removed} removed; {} and later files retained",
                    file.path.display()
                )
            })?;
            println!("Removed plaintext {}", file.path.display());
        }
    }
    Ok(())
}

/// Explicitly persist selected plaintext; never compile environment sources or launch a child.
pub async fn handle_export(config: &Config, command: &EnvWorkspaceExportCommand) -> Result<()> {
    let selected = Selection::load(
        command.workspace.as_deref(),
        Some(&command.mode),
        &command.file_groups,
    )
    .await?;
    selected.preflight(FileGroupOperation::Export)?;
    if command.dry_run {
        selected.preview(FileGroupOperation::Export, PreviewOutput::Stdout);
        return Ok(());
    }
    selected.preview(FileGroupOperation::Export, PreviewOutput::Stderr);
    platform::prepare(export_inner(config, &selected)).await
}

async fn export_inner(config: &Config, selected: &Selection) -> Result<()> {
    let _locks = PathLocks::acquire(&selected.root, &selected.workspace_path, &selected.mappings)?;
    selected.preflight(FileGroupOperation::Export)?;
    let mut config = local_cache::load_config(config, &selected.workspace_path).await?;
    let (captured, plain) = decrypt_files(&mut config, selected).await?;
    selected.preflight(FileGroupOperation::Export)?;
    for file in &captured {
        file.verify(&selected.root)?;
    }
    persist_export(selected, &captured, &plain).await?;
    for (_, target) in &selected.mappings {
        println!("Exported {}", target.display());
    }
    println!(
        "Exported {} files for mode {}",
        selected.mappings.len(),
        selected.mode
    );
    eprintln!(
        "Warning: exported plaintext files remain on disk; manage or remove them yourself. Do not commit them."
    );
    Ok(())
}

/// Keep rollback armed until every target has been written and all captured inputs rechecked.
async fn persist_export(
    selected: &Selection,
    captured: &[CapturedFile],
    plain: &[Zeroizing<Vec<u8>>],
) -> Result<()> {
    let mut materialized = Materialized::new(&selected.root);
    let result = async {
        for ((_, target), bytes) in selected.mappings.iter().zip(plain) {
            selected.verify()?;
            for file in captured {
                file.verify(&selected.root)?;
            }
            materialized.create(target, bytes)?;
            platform::sync_cipher_parents(&selected.root, target)?;
            // Permit controlled cancellation to drop the ownership guard between file writes.
            tokio::task::yield_now().await;
        }
        selected.verify()?;
        for file in captured {
            file.verify(&selected.root)?;
        }
        materialized.persist()
    }
    .await;
    if let Err(error) = result {
        if let Err(cleanup) = materialized.cleanup() {
            return Err(error.context(format!(
                "export failed; plaintext may remain because cleanup failed: {cleanup:#}"
            )));
        }
        return Err(error.context("export failed; files created by this export were removed"));
    }
    Ok(())
}

pub async fn handle_run(config: &Config, command: &EnvRunCommand) -> Result<()> {
    let selected = Selection::load(
        command.workspace.as_deref(),
        command.mode.as_deref(),
        &command.file_groups,
    )
    .await?;
    selected.preview(
        FileGroupOperation::Run,
        if command.dry_run {
            PreviewOutput::Stdout
        } else {
            PreviewOutput::Stderr
        },
    );
    selected.preflight(FileGroupOperation::Run)?;
    if command.dry_run {
        return Ok(());
    }
    let mut ready = platform::prepare(prepare_run(config, command, &selected)).await?;
    let result = platform::command_status(
        &command.command,
        &ready.values,
        ready.override_env,
        &ready.explicit,
    )
    .await;
    let cleanup = ready
        .materialized
        .cleanup()
        .context("file-group cleanup failed");
    // Release all locks and captured buffers before an exit-code-preserving process exit.
    drop(ready);
    match result {
        Ok(status) if !status.success() => {
            if let Err(error) = cleanup {
                eprintln!("File-group cleanup failed: {error:#}");
            }
            finish_command_status(status)
        }
        Ok(_) => cleanup,
        Err(error) => {
            if let Err(cleanup) = cleanup {
                eprintln!("File-group cleanup failed: {cleanup:#}");
            }
            Err(error)
        }
    }
}

struct Ready {
    _locks: PathLocks,
    _captured: Vec<CapturedFile>,
    materialized: Materialized,
    values: SensitiveValues,
    explicit: SensitiveValues,
    override_env: bool,
}

impl Drop for Ready {
    fn drop(&mut self) {
        if let Err(error) = self.materialized.cleanup() {
            eprintln!("File-group cleanup failed: {error:#}");
        }
    }
}

async fn prepare_run(
    config: &Config,
    command: &EnvRunCommand,
    selected: &Selection,
) -> Result<Ready> {
    let _locks = PathLocks::acquire(&selected.root, &selected.workspace_path, &selected.mappings)?;
    selected.preflight(FileGroupOperation::Run)?;
    let mut config = local_cache::load_config(config, &selected.workspace_path).await?;
    let (captured, plain) = decrypt_files(&mut config, selected).await?;
    let (values, override_process_env) = load_run_values_effective(
        &config,
        &selected.workspace_path,
        Some(&selected.mode),
        true,
    )
    .await?;
    let explicit = resolve_explicit_values(&config, &command.with).await?;
    selected.verify()?;
    for file in &captured {
        file.verify(&selected.root)?;
    }
    let mut materialized = Materialized::new(&selected.root);
    for ((_, target), bytes) in selected.mappings.iter().zip(&plain) {
        materialized.create(target, bytes)?;
    }
    Ok(Ready {
        _locks,
        _captured: captured,
        materialized,
        values,
        explicit,
        override_env: override_process_env,
    })
}

fn validate_cipher(bytes: &[u8]) -> Result<&str> {
    let text = std::str::from_utf8(bytes).context("file ciphertext must be encoded text")?;
    secret::validate_file_ciphertext(text)?;
    Ok(text)
}

/// Capture every ciphertext before invoking any backend; retain zeroizing buffers through publication.
async fn decrypt_files(
    config: &mut Config,
    selected: &Selection,
) -> Result<(Vec<CapturedFile>, Vec<Zeroizing<Vec<u8>>>)> {
    let mut captured = Vec::new();
    let mut cipher_total = 0;
    for (source, _) in &selected.mappings {
        let file = CapturedFile::read(
            &selected.root,
            source,
            MAX_CIPHER.min(MAX_CIPHER_TOTAL - cipher_total),
        )?;
        cipher_total += file.bytes.len();
        captured.push(file);
    }
    for file in &captured {
        validate_cipher(&file.bytes)?;
    }
    // Freeze a hybrid choice across all files and environment sources for this invocation.
    if captured.iter().any(|f| f.bytes.starts_with(b"hybrid:"))
        && config.hybrid_decrypt_backend.is_none()
    {
        let backend = secret::hybrid::select(&config.resolved_age_identities(), None).await?;
        config.hybrid_decrypt_backend = Some(
            if backend == BackendKind::Gpg {
                "gpg"
            } else {
                "age"
            }
            .into(),
        );
    }
    let mut plain = Vec::new();
    let mut total = 0;
    for file in &captured {
        let decoded = secret::decrypt_bytes(
            validate_cipher(&file.bytes)?,
            config,
            MAX_FILE.min(MAX_TOTAL - total),
        )
        .await
        .map_err(|_| anyhow::anyhow!("file decryption failed or was cancelled"))?;
        total += decoded.len();
        ensure!(total <= MAX_TOTAL, "selected plaintext exceeds 64 MiB");
        plain.push(decoded);
    }
    Ok((captured, plain))
}

#[cfg(test)]
mod export_tests {
    use super::*;
    use std::{
        future::Future,
        task::{Context, Waker},
    };

    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    impl Fixture {
        async fn new() -> (Self, Selection, Vec<CapturedFile>, Vec<Zeroizing<Vec<u8>>>) {
            let root = std::env::temp_dir()
                .join(format!("shine-export-rollback-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(root.join("existing")).unwrap();
            let path = root.join("shine.workspace.toml");
            fs::write(&path, "version=3\n[env]\ndefault_mode='test'\n[[file_groups.signing.files]]\nsource='one.enc'\ntarget='new/one'\n[[file_groups.signing.files]]\nsource='two.enc'\ntarget='existing/two'\n").unwrap();
            fs::write(root.join("one.enc"), "one").unwrap();
            fs::write(root.join("two.enc"), "two").unwrap();
            let selected = Selection::load(Some(&path), Some("test"), &["signing".into()])
                .await
                .unwrap();
            let captured = selected
                .mappings
                .iter()
                .map(|(source, _)| CapturedFile::read(&selected.root, source, MAX_CIPHER).unwrap())
                .collect();
            let plain = vec![Zeroizing::new(vec![0, 255]), Zeroizing::new(Vec::new())];
            (Self(root), selected, captured, plain)
        }
    }

    #[tokio::test]
    async fn file_export_late_write_failure_rolls_back_only_owned_files() {
        let (f, selected, captured, plain) = Fixture::new().await;
        let mut future = Box::pin(persist_export(&selected, &captured, &plain));
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        assert!(f.0.join("new/one").exists());
        // Inject a competing writer after the first target is completely written.
        fs::write(f.0.join("existing/two"), "winner").unwrap();
        let error = future.await.unwrap_err();
        assert!(format!("{error:#}").contains("files created by this export were removed"));
        assert!(!f.0.join("new").exists());
        assert_eq!(fs::read(f.0.join("existing/two")).unwrap(), b"winner");
        assert!(f.0.join("existing").is_dir());
    }

    #[tokio::test]
    async fn file_export_cancelled_write_future_rolls_back_owned_files() {
        let (f, selected, captured, plain) = Fixture::new().await;
        let mut future = Box::pin(persist_export(&selected, &captured, &plain));
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        assert!(f.0.join("new/one").exists());
        drop(future);
        assert!(!f.0.join("new").exists());
        assert!(f.0.join("existing").is_dir());
        assert!(!f.0.join("existing/two").exists());
    }

    #[tokio::test]
    async fn file_export_rollback_retains_replacements_and_reports_residue() {
        let (f, selected, captured, plain) = Fixture::new().await;
        let mut future = Box::pin(persist_export(&selected, &captured, &plain));
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        fs::remove_file(f.0.join("new/one")).unwrap();
        fs::write(f.0.join("new/one"), "replacement").unwrap();
        fs::write(f.0.join("existing/two"), "winner").unwrap();
        let error = future.await.unwrap_err();
        let diagnostic = format!("{error:#}");
        assert!(
            diagnostic.contains("plaintext may remain")
                && diagnostic.contains("plaintext target was replaced"),
            "{diagnostic}"
        );
        assert_eq!(fs::read(f.0.join("new/one")).unwrap(), b"replacement");
        assert_eq!(fs::read(f.0.join("existing/two")).unwrap(), b"winner");
    }
}
