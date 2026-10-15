use super::*;

pub(super) async fn load_shell_operation_journal(
    host: &impl FileSystemObservationHost,
    shine_dir: &Path,
) -> Result<Option<(ShellOperationJournalV1, Vec<u8>)>> {
    let path = shine_dir.join(SHELL_OPERATION_JOURNAL_FILE);
    let bytes = match host.read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.is_not_found() => return Ok(None),
        Err(error) => return Err(error.into_anyhow("reading Shell operation journal")),
    };
    let journal: ShellOperationJournalV1 =
        toml::from_slice(&bytes).context("failed to parse Shell operation journal")?;
    journal.validate()?;
    Ok(Some((journal, bytes)))
}

pub(super) async fn save_shell_operation_journal(
    host: &impl FileSystemHost,
    shine_dir: &Path,
    journal: &ShellOperationJournalV1,
) -> Result<()> {
    journal.validate()?;
    let bytes = toml::to_string_pretty(journal).context("serializing Shell operation journal")?;
    host.write_atomic(
        &shine_dir.join(SHELL_OPERATION_JOURNAL_FILE),
        bytes.as_bytes(),
    )
    .await
    .map_err(|error| error.into_anyhow("writing Shell operation journal"))
}

pub(super) async fn remove_shell_operation_journal(
    host: &impl FileSystemHost,
    shine_dir: &Path,
) -> Result<()> {
    match host
        .remove_file(&shine_dir.join(SHELL_OPERATION_JOURNAL_FILE))
        .await
    {
        Ok(()) => Ok(()),
        Err(error) if error.is_not_found() => Ok(()),
        Err(error) => Err(error.into_anyhow("removing Shell operation journal")),
    }
}
