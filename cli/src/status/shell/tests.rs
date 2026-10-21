use super::*;

#[tokio::test]
async fn shell_row_projection_preserves_all_status_symbols_and_attention_precedence() {
    let runtime = crate::test_support::installed_shell_runtime().await;
    let inspection = runtime
        .inspect_shells()
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.file.command_name == "run")
        .unwrap();
    for (status, expected) in [
        (FileStatus::NotInstalled, "✗"),
        (FileStatus::UpToDate, "✓"),
        (FileStatus::UpdateAvail, "↑"),
        (FileStatus::Missing, "!"),
        (FileStatus::Partial, "~"),
        (FileStatus::UserModified, "~"),
        (FileStatus::GeneratorNotEvaluated, "~"),
        (FileStatus::GeneratorEvaluationFailed, "~"),
        (FileStatus::GeneratorTrustRequired, "~"),
    ] {
        for (conflict, missing) in [(false, false), (true, false), (false, true), (true, true)] {
            let mut file = inspection.clone();
            file.status = status;
            file.link_conflict = conflict;
            file.preset_missing = missing;
            let row = ShellRow::from(file);
            assert_eq!(row.file_status, status);
            assert_eq!(
                row.status_sym,
                if conflict || missing { "!" } else { expected }
            );
            assert_eq!(row.symbol, colors::symbol(row.status_sym));
            assert_eq!(row.status_text, inspection.status_text);
            assert_eq!(row.label, "demo/run");
            assert_eq!(row.selector(), "demo/run");
            assert_eq!(row.target(), "shell/demo/run");
            assert_eq!(row.changes, inspection.changes);
        }
    }
}

#[tokio::test]
async fn shell_row_preserves_legacy_receipt_target_normalization_independently_of_label() {
    let runtime = crate::test_support::installed_shell_runtime().await;
    let inspection = runtime
        .inspect_shells()
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.file.command_name == "run")
        .unwrap();
    for (command, selector, target) in [
        ("run", "demo/run", "shell/demo/run"),
        ("nested/run", "demo/nested/run", "shell/demo/run"),
        ("", "demo/", "shell/demo/"),
    ] {
        let mut file = inspection.clone();
        file.file.command_name = command.into();
        let mut row = ShellRow::from(file);
        row.label = "custom display/other".into();
        assert_eq!(row.selector(), selector);
        assert_eq!(row.target(), target);
    }
}
