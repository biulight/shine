use std::sync::OnceLock;

static SEMVER: OnceLock<String> = OnceLock::new();
static DISPLAY_VERSION: OnceLock<String> = OnceLock::new();

pub fn semver() -> &'static str {
    SEMVER
        .get_or_init(|| {
            format_semver(
                env!("CARGO_PKG_VERSION"),
                option_env!("SHINE_VERSION_METADATA"),
            )
        })
        .as_str()
}

pub fn display() -> &'static str {
    DISPLAY_VERSION
        .get_or_init(|| {
            format_display(
                semver(),
                option_env!("SHINE_GIT_SHA"),
                option_env!("SHINE_GIT_DATE"),
            )
        })
        .as_str()
}

pub fn package() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub fn preview_commit() -> Option<&'static str> {
    is_preview()
        .then_some(option_env!("SHINE_GIT_SHA"))
        .flatten()
}

fn is_preview() -> bool {
    option_env!("SHINE_VERSION_METADATA").is_some_and(|metadata| {
        let metadata = metadata.trim();
        metadata == "preview" || metadata.starts_with("preview.")
    })
}

fn format_semver(package_version: &str, metadata: Option<&str>) -> String {
    match metadata
        .map(str::trim)
        .filter(|metadata| !metadata.is_empty())
    {
        Some("preview") => format!("{package_version}-preview"),
        Some(metadata) if metadata.starts_with("preview.") => {
            format!("{package_version}-preview")
        }
        Some(metadata) => format!("{package_version}+{metadata}"),
        None => package_version.to_string(),
    }
}

fn format_display(semver: &str, sha: Option<&str>, date: Option<&str>) -> String {
    match (sha.and_then(trimmed), date.and_then(trimmed)) {
        (Some(sha), Some(date)) => format!("{semver} ({sha} {date})"),
        _ => semver.to_string(),
    }
}

fn trimmed(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::{format_display, format_semver};

    #[test]
    fn format_version_without_metadata_returns_package_version() {
        assert_eq!(format_semver("1.0.0", None), "1.0.0");
        assert_eq!(format_semver("1.0.0", Some("")), "1.0.0");
    }

    #[test]
    fn preview_uses_prerelease_label_without_duplicate_commit() {
        assert_eq!(format_semver("1.0.0", Some("preview")), "1.0.0-preview");
        assert_eq!(
            format_semver("1.0.0", Some("preview.abc1234")),
            "1.0.0-preview"
        );
    }

    #[test]
    fn display_matches_cargo_style_when_provenance_is_available() {
        assert_eq!(
            format_display("1.0.0", Some("30a34c682"), Some("2026-05-25")),
            "1.0.0 (30a34c682 2026-05-25)"
        );
        assert_eq!(
            format_display("1.0.0-preview", Some("30a34c682"), Some("2026-05-25")),
            "1.0.0-preview (30a34c682 2026-05-25)"
        );
    }

    #[test]
    fn display_falls_back_to_semver_when_provenance_is_incomplete() {
        assert_eq!(format_display("1.0.0", None, None), "1.0.0");
        assert_eq!(format_display("1.0.0", Some("30a34c682"), None), "1.0.0");
    }
}
