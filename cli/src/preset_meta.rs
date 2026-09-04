//! Characterization checks for generated preset capability documentation.
//!
//! Discovery and platform filtering are owned by Core; this module only
//! renders the resulting typed categories into the checked-in documentation
//! table used by both public manual locales.

#[cfg(test)]
mod tests {
    use crate::platform::OperatingSystem;
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;

    fn built_in_platform_availability() -> BTreeMap<String, BTreeSet<OperatingSystem>> {
        let mut capabilities = BTreeMap::<String, BTreeSet<OperatingSystem>>::new();
        for platform in OperatingSystem::ALL {
            let runtime = crate::core_runtime::from_embedded_presets_for_platform(platform);
            for category in runtime.app_categories(None).unwrap() {
                capabilities
                    .entry(format!("app/{}", category.name))
                    .or_default()
                    .insert(platform);
            }
            for category in runtime.shell_categories(None).unwrap() {
                for file in category.files {
                    capabilities
                        .entry(format!("shell/{}/{}", category.name, file.command_name))
                        .or_default()
                        .insert(platform);
                }
            }
        }
        capabilities
    }

    fn normalize_generated_block(block: &str) -> String {
        block.replace("\r\n", "\n")
    }

    fn generated_block_replacement(current: &str, expected: &str) -> String {
        if current.contains("\r\n") {
            expected.replace('\n', "\r\n")
        } else {
            expected.to_string()
        }
    }

    #[test]
    fn built_in_preset_platform_capability_docs_are_current() {
        const START: &str = "<!-- BEGIN GENERATED PRESET PLATFORM CAPABILITIES -->";
        const END: &str = "<!-- END GENERATED PRESET PLATFORM CAPABILITIES -->";

        let mut expected = String::from(START);
        expected.push_str("\n| Preset capability | macOS | Linux | Windows |\n");
        expected.push_str("| --- | --- | --- | --- |\n");
        for (target, platforms) in built_in_platform_availability() {
            let supported = |platform| {
                if platforms.contains(&platform) {
                    "✓"
                } else {
                    "—"
                }
            };
            expected.push_str(&format!(
                "| `{target}` | {} | {} | {} |\n",
                supported(OperatingSystem::Macos),
                supported(OperatingSystem::Linux),
                supported(OperatingSystem::Windows),
            ));
        }
        expected.push_str(END);

        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        if !repository_root.join("docs/manual").is_dir() {
            return;
        }
        let update = std::env::var_os("SHINE_UPDATE_PRESET_CAPABILITIES").as_deref()
            == Some(std::ffi::OsStr::new("1"));
        for relative in [
            "docs/manual/reference/built-in-presets.md",
            "website/i18n/zh-Hans/docusaurus-plugin-content-docs/current/reference/built-in-presets.md",
        ] {
            let path = repository_root.join(relative);
            let document = std::fs::read_to_string(&path).unwrap();
            let start = document
                .find(START)
                .unwrap_or_else(|| panic!("{} is missing {START}", path.display()));
            let end = document[start..]
                .find(END)
                .map(|offset| start + offset + END.len())
                .unwrap_or_else(|| panic!("{} is missing {END}", path.display()));
            if update {
                let replacement = generated_block_replacement(&document[start..end], &expected);
                let mut updated = document;
                updated.replace_range(start..end, &replacement);
                std::fs::write(&path, updated).unwrap();
                continue;
            }
            assert_eq!(
                normalize_generated_block(&document[start..end]),
                expected,
                "{} has a stale built-in preset platform capability list; replace its generated block with the right-hand value",
                path.display()
            );
        }
    }

    #[test]
    fn generated_capability_blocks_accept_and_preserve_crlf() {
        let expected = "<!-- start -->\n| row |\n<!-- end -->";
        let checked_out = "<!-- start -->\r\n| row |\r\n<!-- end -->";

        assert_eq!(normalize_generated_block(checked_out), expected);
        assert_eq!(
            generated_block_replacement(checked_out, expected),
            checked_out
        );
        assert_eq!(generated_block_replacement(expected, expected), expected);
    }
}
