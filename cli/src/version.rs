use std::sync::OnceLock;

static DISPLAY_VERSION: OnceLock<String> = OnceLock::new();

pub(crate) fn display() -> &'static str {
    DISPLAY_VERSION
        .get_or_init(|| {
            format_version(
                env!("CARGO_PKG_VERSION"),
                option_env!("SHINE_VERSION_METADATA"),
            )
        })
        .as_str()
}

pub(crate) fn package() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn format_version(package_version: &str, metadata: Option<&str>) -> String {
    match metadata
        .map(str::trim)
        .filter(|metadata| !metadata.is_empty())
    {
        Some(metadata) => format!("{package_version}+{metadata}"),
        None => package_version.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::format_version;

    #[test]
    fn format_version_without_metadata_returns_package_version() {
        assert_eq!(format_version("0.20.0", None), "0.20.0");
        assert_eq!(format_version("0.20.0", Some("")), "0.20.0");
    }

    #[test]
    fn format_version_with_preview_metadata_appends_build_metadata() {
        assert_eq!(
            format_version("0.20.0", Some("preview.abc1234")),
            "0.20.0+preview.abc1234"
        );
    }
}
