//! App journal io.

use super::*;

pub(super) async fn load_app_operation_journal(
    host: &impl FileSystemObservationHost,
    shine_dir: &Path,
) -> Result<Option<(AppOperationJournalV1, Vec<u8>)>> {
    let path = shine_dir.join(APP_OPERATION_JOURNAL_FILE);
    let bytes = match host.read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.is_not_found() => return Ok(None),
        Err(error) => return Err(error.into_anyhow("failed to read App operation journal")),
    };
    let journal: AppOperationJournalV1 =
        toml::from_slice(&bytes).context("failed to parse App operation journal")?;
    journal.validate()?;
    Ok(Some((journal, bytes)))
}

pub(super) async fn save_app_operation_journal(
    host: &impl FileSystemHost,
    shine_dir: &Path,
    journal: &AppOperationJournalV1,
) -> Result<()> {
    journal.validate()?;
    let bytes =
        toml::to_string_pretty(journal).context("failed to serialize App operation journal")?;
    host.write_atomic(
        &shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        bytes.as_bytes(),
    )
    .await
    .map_err(|error| error.into_anyhow("failed to write App operation journal"))
}

pub(super) async fn read_optional(
    host: &impl FileSystemObservationHost,
    path: &Path,
) -> Result<Option<Vec<u8>>> {
    match host.read(path).await {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.is_not_found() => Ok(None),
        Err(error) => Err(error.into_anyhow("failed to observe App recovery resource")),
    }
}

pub(super) async fn path_exists(
    host: &impl FileSystemObservationHost,
    path: &Path,
) -> Result<bool> {
    match host.metadata(path).await {
        Ok(_) => Ok(true),
        Err(error) if error.is_not_found() => Ok(false),
        Err(error) => Err(error.into_anyhow("failed to observe App recovery path")),
    }
}
