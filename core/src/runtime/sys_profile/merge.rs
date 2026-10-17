//! Optional Git merge adapter with the existing bounded process and fallback rules.
use super::files::MergeInputs;
use super::three_way::conflict_marker_merge;
use super::*;

pub(super) enum ProfileMerge {
    Clean(Vec<u8>),
    Conflict(Vec<u8>),
}

pub(super) async fn merge_sys_profile(
    host: &impl ProcessHost,
    inputs: &MergeInputs<'_>,
) -> Result<ProfileMerge> {
    if inputs.allow_git_merge
        && let Some(result) = try_git_merge_file(
            host,
            inputs.active_path,
            inputs.base_path,
            inputs.template_path,
        )
        .await?
    {
        return Ok(result);
    }

    Ok(
        match fallback_three_way_merge(inputs.base, inputs.active, inputs.template) {
            Some(merged) => ProfileMerge::Clean(merged),
            None => ProfileMerge::Conflict(conflict_marker_merge(
                inputs.base,
                inputs.active,
                inputs.template,
            )),
        },
    )
}

async fn try_git_merge_file(
    host: &impl ProcessHost,
    active_path: &Path,
    base_path: &Path,
    template_path: &Path,
) -> Result<Option<ProfileMerge>> {
    let output = match host
        .run(ProcessRequest {
            program: "git".to_string(),
            args: vec![
                "merge-file".to_string(),
                "-p".to_string(),
                active_path.to_string_lossy().into_owned(),
                base_path.to_string_lossy().into_owned(),
                template_path.to_string_lossy().into_owned(),
            ],
            io: ProcessIo::Captured,
            stdout_limit: Some(4 * 1024 * 1024),
            stderr_limit: Some(1024 * 1024),
            ..ProcessRequest::default()
        })
        .await
    {
        Ok(output) => output,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error.context("running git merge-file")),
    };

    match output.exit_code {
        Some(0) => Ok(Some(ProfileMerge::Clean(output.stdout))),
        Some(1) => Ok(Some(ProfileMerge::Conflict(output.stdout))),
        _ => Ok(None),
    }
}
