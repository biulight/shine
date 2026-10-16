//! Sys journal io; resource rules remain domain-owned.
use super::*;

pub(super) async fn read_missing_marker(
    host: &impl FileSystemObservationHost,
    path: &Path,
) -> Result<Vec<u8>> {
    match host.read(path).await {
        Ok(bytes) => Ok(bytes),
        Err(error) if error.is_not_found() => Ok(b"missing".to_vec()),
        Err(error) => Err(error.into_anyhow("reading Sys recovery state")),
    }
}

pub(super) async fn load_sys_operation_journal(
    host: &impl FileSystemObservationHost,
    shine_dir: &Path,
) -> Result<Option<(SysOperationJournalV1, Vec<u8>)>> {
    let path = shine_dir.join(SYS_OPERATION_JOURNAL_FILE);
    let bytes = match host.read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.is_not_found() => return Ok(None),
        Err(error) => return Err(error.into_anyhow("reading Sys operation journal")),
    };
    let journal: SysOperationJournalV1 =
        toml::from_slice(&bytes).context("failed to parse Sys operation journal")?;
    journal.validate()?;
    Ok(Some((journal, bytes)))
}

pub(super) async fn save_sys_operation_journal(
    host: &impl FileSystemHost,
    shine_dir: &Path,
    journal: &SysOperationJournalV1,
) -> Result<()> {
    journal.validate()?;
    let bytes = toml::to_string_pretty(journal).context("serializing Sys operation journal")?;
    host.write_atomic(
        &shine_dir.join(SYS_OPERATION_JOURNAL_FILE),
        bytes.as_bytes(),
    )
    .await
    .map_err(|error| error.into_anyhow("writing Sys operation journal"))
}

pub(super) async fn remove_sys_operation_journal(
    host: &impl FileSystemHost,
    shine_dir: &Path,
) -> Result<()> {
    match host
        .remove_file(&shine_dir.join(SYS_OPERATION_JOURNAL_FILE))
        .await
    {
        Ok(()) => Ok(()),
        Err(error) if error.is_not_found() => Ok(()),
        Err(error) => Err(error.into_anyhow("removing Sys operation journal")),
    }
}
