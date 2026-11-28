use anyhow::{Result, bail};

/// Detect the current OS identifier using `std::env::consts::OS` and, on Linux,
/// the `ID=` field from `/etc/os-release`.
pub async fn detect_os_id() -> Result<String> {
    let os_release = tokio::fs::read_to_string("/etc/os-release").await.ok();
    detect_os_id_from(std::env::consts::OS, os_release.as_deref())
}

fn detect_os_id_from(os: &str, os_release: Option<&str>) -> Result<String> {
    match os {
        "macos" => Ok("macos".to_string()),
        "windows" => Ok("windows".to_string()),
        "linux" => {
            if let Some(content) = os_release {
                for line in content.lines() {
                    if let Some(id) = line.strip_prefix("ID=") {
                        return Ok(id.trim_matches('"').to_lowercase());
                    }
                }
            }
            bail!(
                "Could not detect Linux distribution. \
                 Expected ID= in /etc/os-release. Supported built-in Linux presets: ubuntu, arch"
            )
        }
        other => bail!(
            "Unsupported platform '{}'. Supported targets: ubuntu/arch (Linux), macos, windows",
            other
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_linux_targets_use_the_platform_name() {
        for os in ["macos", "windows"] {
            assert_eq!(detect_os_id_from(os, None).unwrap(), os);
        }
    }

    #[test]
    fn detects_linux_distribution_from_os_release() {
        for (os_release, expected) in [
            (
                "PRETTY_NAME=\"Ubuntu 22.04\"\nID=ubuntu\nVERSION_ID=\"22.04\"\n",
                "ubuntu",
            ),
            ("ID=\"ubuntu\"\n", "ubuntu"),
            ("ID=Debian\n", "debian"),
            ("ID=arch\n", "arch"),
            ("ID=manjaro\nID_LIKE=arch\n", "manjaro"),
        ] {
            assert_eq!(
                detect_os_id_from("linux", Some(os_release)).unwrap(),
                expected,
                "{os_release:?}"
            );
        }
    }

    #[test]
    fn detection_errors_identify_the_missing_release_or_unsupported_platform() {
        for (os, diagnostic) in [("linux", "os-release"), ("freebsd", "freebsd")] {
            let err = detect_os_id_from(os, None).unwrap_err();
            assert!(err.to_string().contains(diagnostic), "{os}: {err}");
        }
    }
}
