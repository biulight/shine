use super::*;
#[test]
fn update_check_failure_warning_is_non_fatal_wording() {
    let err = anyhow::anyhow!(
        "GitHub stable release request failed: HTTP 403 Forbidden: API rate limit exceeded"
    );
    let warning = format_update_check_failure_warning(&err);

    assert!(warning.contains("warning: skipped shine version check"));
    assert!(warning.contains("HTTP 403 Forbidden"));
    assert!(!warning.contains("Update check failed"));
}
