use super::*;

#[test]
fn format_self_upgrade_message_handles_stable_transitions() {
    for (previous, installed) in [
        ("0.21.3", "0.21.4"),
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
fn format_self_upgrade_message_handles_preview_transitions() {
    for (previous, installed, expected) in [
        (
            "0.21.3",
            "1.0.0-preview",
            "Installed shine preview 1.0.0-preview over stable 0.21.3 (preview).",
        ),
        (
            "1.0.0-preview",
            "1.0.1-preview",
            "Updated shine preview from 1.0.0-preview to 1.0.1-preview (preview).",
        ),
    ] {
        assert_eq!(
            format_self_upgrade_message(ReleaseChannel::Preview, previous, installed, "preview"),
            expected
        );
    }
}
