use super::*;
#[test]
fn format_self_upgrade_message_handles_stable_channel() {
    assert_eq!(
        format_self_upgrade_message(ReleaseChannel::Stable, "0.21.3", "0.21.4", "v0.21.4",),
        "Installed shine 0.21.4 over 0.21.3."
    );
}

#[test]
fn format_self_upgrade_message_handles_stable_reinstall_and_downgrade() {
    for (previous, installed) in [
        ("2.5.0", "2.5.0"),
        ("2.5.0", "2.4.0"),
        ("2.6.0-preview+abc123", "2.5.0"),
    ] {
        assert_eq!(
            format_self_upgrade_message(
                ReleaseChannel::Stable,
                previous,
                installed,
                &format!("v{installed}"),
            ),
            format!("Installed shine {installed} over {previous}."),
        );
    }
}

#[test]
fn format_self_upgrade_message_handles_stable_to_preview_install() {
    assert_eq!(
        format_self_upgrade_message(
            ReleaseChannel::Preview,
            "0.21.3",
            "1.0.0-preview",
            "preview",
        ),
        "Installed shine preview 1.0.0-preview over stable 0.21.3 (preview)."
    );
}

#[test]
fn format_self_upgrade_message_handles_preview_to_preview_update() {
    assert_eq!(
        format_self_upgrade_message(
            ReleaseChannel::Preview,
            "1.0.0-preview",
            "1.0.1-preview",
            "preview",
        ),
        "Updated shine preview from 1.0.0-preview to 1.0.1-preview (preview)."
    );
}
