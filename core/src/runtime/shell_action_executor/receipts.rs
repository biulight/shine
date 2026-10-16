use super::*;

pub(super) fn matching_shell_receipt(
    manifest: &ShellManifest,
    target: &str,
    expected: &ShellLauncherReceiptV1,
) -> ReceiptState {
    let Some(entry) = manifest.find(target) else {
        return ReceiptState::Missing;
    };
    if receipt_contract(entry) == *expected {
        ReceiptState::Matching
    } else {
        ReceiptState::Conflict
    }
}

pub(super) fn shell_receipts_match_previous(
    manifest: &ShellManifest,
    transitions: &[ShellReceiptTransitionV1],
) -> bool {
    transitions
        .iter()
        .all(|transition| match &transition.previous {
            Some(previous) => {
                matching_shell_receipt(manifest, &transition.target, previous)
                    == ReceiptState::Matching
            }
            None => manifest.find(&transition.target).is_none(),
        })
}

pub(super) fn shell_receipts_match_desired(
    manifest: &ShellManifest,
    transitions: &[ShellReceiptTransitionV1],
) -> bool {
    transitions.iter().all(|transition| {
        matching_shell_receipt(manifest, &transition.target, &transition.desired)
            == ReceiptState::Matching
    })
}

pub(super) fn shell_removal_receipts_match_previous(
    manifest: &ShellManifest,
    removals: &[ShellReceiptRemovalV1],
) -> bool {
    removals.iter().all(|removal| {
        matching_shell_receipt(manifest, &removal.target, &removal.previous)
            == ReceiptState::Matching
    })
}

pub(super) fn shell_removal_receipts_match_missing(
    manifest: &ShellManifest,
    removals: &[ShellReceiptRemovalV1],
) -> bool {
    removals
        .iter()
        .all(|removal| manifest.find(&removal.target).is_none())
}

pub(super) fn shell_profile_receipts_match_previous(
    manifest: &ShellManifest,
    transitions: &[ShellReceiptTransitionV1],
    removals: &[ShellReceiptRemovalV1],
    legacy_targets: &[String],
) -> bool {
    shell_receipts_match_previous(manifest, transitions)
        && shell_removal_receipts_match_previous(manifest, removals)
        && legacy_targets
            .iter()
            .all(|target| manifest.find(target).is_none())
}

pub(super) fn shell_profile_receipts_match_desired(
    manifest: &ShellManifest,
    transitions: &[ShellReceiptTransitionV1],
    removals: &[ShellReceiptRemovalV1],
    legacy_targets: &[String],
) -> bool {
    shell_receipts_match_desired(manifest, transitions)
        && shell_removal_receipts_match_missing(manifest, removals)
        && legacy_targets
            .iter()
            .all(|target| manifest.find(target).is_none())
}

pub(super) fn restore_shell_previous_receipts(
    manifest: &mut ShellManifest,
    transitions: &[ShellReceiptTransitionV1],
) -> Result<()> {
    let targets = transitions
        .iter()
        .map(|transition| transition.target.clone())
        .collect::<BTreeSet<_>>();
    let entries = transitions
        .iter()
        .filter_map(|transition| transition.previous.as_deref())
        .map(manifest_entry_from_receipt)
        .collect::<Result<Vec<_>>>()?;
    manifest.replace_targets(&targets, entries);
    Ok(())
}

pub(super) fn restore_shell_removed_receipts(
    manifest: &mut ShellManifest,
    removals: &[ShellReceiptRemovalV1],
) -> Result<()> {
    let targets = removals
        .iter()
        .map(|removal| removal.target.clone())
        .collect::<BTreeSet<_>>();
    let entries = removals
        .iter()
        .map(|removal| manifest_entry_from_receipt(&removal.previous))
        .collect::<Result<Vec<_>>>()?;
    manifest.replace_targets(&targets, entries);
    Ok(())
}
