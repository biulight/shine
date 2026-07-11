use super::github::{GithubRelease, download_asset_bytes, fetch_release};
use super::{ReleaseChannel, UpgradeResult, parse_release_tag, unix_timestamp_now};
use crate::config::Config;
use crate::{platform, version};
use anyhow::{Context, Result, anyhow, bail};
use flate2::read::GzDecoder;
use semver::Version;
use std::ffi::OsStr;
use std::path::Path;
use tar::Archive;
use tokio::fs;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReleaseAsset {
    release_tag: String,
    target: String,
    download_url: String,
}

pub async fn upgrade_to_release(
    config: &Config,
    channel: ReleaseChannel,
    force_install: bool,
) -> Result<UpgradeResult> {
    let current = Version::parse(env!("CARGO_PKG_VERSION"))
        .context("current package version must be valid semver")?;
    let current_display = version::display();

    let release = fetch_release(channel).await?;
    let latest = match channel {
        ReleaseChannel::Stable => {
            let latest = parse_release_tag(&release.tag_name)?;
            let now_secs = unix_timestamp_now()?;
            let cache_path = config.shine_dir().join(super::UPDATE_CACHE_FILE);
            super::store_cache_if_possible(&cache_path, &latest, now_secs).await;

            if !force_install && latest <= current {
                return Ok(UpgradeResult::AlreadyUpToDate {
                    channel,
                    latest: latest.to_string(),
                });
            }
            Some(latest)
        }
        ReleaseChannel::Preview => {
            if preview_release_version_label(&release).as_deref() == Some(current_display) {
                return Ok(UpgradeResult::AlreadyUpToDate {
                    channel,
                    latest: current_display.to_string(),
                });
            }
            None
        }
    };

    let asset = find_release_asset(
        &release,
        channel,
        std::env::consts::OS,
        std::env::consts::ARCH,
    )?;
    let archive_bytes = download_asset_bytes(&asset.download_url).await?;
    let current_exe = std::env::current_exe().context("failed to resolve current executable")?;
    install_downloaded_archive(
        &archive_bytes,
        &current_exe,
        platform::current_executable_name(),
    )
    .await?;
    let installed_version = installed_version_label(&current_exe, &asset.release_tag).await;

    if let Some(latest) = &latest {
        let now_secs = unix_timestamp_now()?;
        let cache_path = config.shine_dir().join(super::UPDATE_CACHE_FILE);
        super::store_cache_if_possible(&cache_path, latest, now_secs).await;
    }

    Ok(UpgradeResult::Upgraded {
        channel,
        previous: current,
        previous_display: current_display.to_string(),
        release_tag: asset.release_tag,
        installed_version,
        installed_path: current_exe,
    })
}

async fn install_downloaded_archive(
    archive_bytes: &[u8],
    current_exe: &Path,
    binary_name: &str,
) -> Result<()> {
    let extracted = extract_binary_from_archive(archive_bytes, binary_name)?;

    let parent_dir = current_exe
        .parent()
        .context("current executable path must have a parent directory")?;
    let staged_path = parent_dir.join(format!(".shine-upgrade-{}", uuid::Uuid::new_v4()));
    let backup_path = parent_dir.join(format!(".shine-backup-{}", uuid::Uuid::new_v4()));

    fs::write(&staged_path, extracted).await.with_context(|| {
        format!(
            "failed to stage upgrade binary at {}",
            staged_path.display()
        )
    })?;
    set_executable_permissions(&staged_path).await?;

    match fs::rename(current_exe, &backup_path).await {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            let _ = fs::remove_file(&staged_path).await;
            if cfg!(windows) {
                bail!(
                    "cannot replace {} due to insufficient permissions; rerun from an elevated terminal or install to a user-writable path",
                    current_exe.display()
                );
            } else {
                bail!(
                    "cannot replace {} due to insufficient permissions; reinstall with install.sh into a user-writable directory such as ~/.local/bin",
                    current_exe.display()
                );
            }
        }
        Err(err) => {
            let _ = fs::remove_file(&staged_path).await;
            return Err(err).with_context(|| {
                format!(
                    "failed to prepare existing binary {}",
                    current_exe.display()
                )
            });
        }
    }

    match fs::rename(&staged_path, current_exe).await {
        Ok(()) => {
            let _ = fs::remove_file(&backup_path).await;
            Ok(())
        }
        Err(err) => match fs::rename(&backup_path, current_exe).await {
            Ok(()) => {
                let _ = fs::remove_file(&staged_path).await;
                Err(err).with_context(|| {
                    format!(
                        "failed to install upgraded binary at {}; \
                             original binary has been restored",
                        current_exe.display()
                    )
                })
            }
            Err(rollback_err) => {
                let _ = fs::remove_file(&staged_path).await;
                Err(err).with_context(|| {
                    format!(
                        "failed to install upgraded binary at {} \
                             and rollback also failed ({rollback_err:#}); \
                             {} may be missing — reinstall from install.sh",
                        current_exe.display(),
                        current_exe.display()
                    )
                })
            }
        },
    }
}

fn extract_binary_from_archive(archive_bytes: &[u8], binary_name: &str) -> Result<Vec<u8>> {
    let decoder = GzDecoder::new(std::io::Cursor::new(archive_bytes));
    let mut archive = Archive::new(decoder);

    for entry_result in archive
        .entries()
        .context("failed to read archive entries")?
    {
        let mut entry = entry_result.context("failed to read release archive entry")?;
        let path = entry
            .path()
            .context("failed to inspect archive entry path")?;

        if path.file_name() == Some(OsStr::new(binary_name)) {
            let mut extracted = Vec::new();
            std::io::copy(&mut entry, &mut extracted)
                .context("failed to extract shine binary from release archive")?;
            if extracted.is_empty() {
                bail!("release archive contained an empty shine binary");
            }
            return Ok(extracted);
        }
    }

    bail!("release archive does not contain a {binary_name} binary")
}

fn find_release_asset(
    release: &GithubRelease,
    channel: ReleaseChannel,
    os: &str,
    arch: &str,
) -> Result<ReleaseAsset> {
    let target = platform_target(os, arch)?;
    let expected_name = asset_file_name(release, channel, &target)?;

    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == expected_name)
        .ok_or_else(|| {
            anyhow!(
                "no release asset named {expected_name} found for {os}/{arch}; expected it to be published with the release"
            )
        })?;

    Ok(ReleaseAsset {
        release_tag: release.tag_name.clone(),
        target,
        download_url: asset.browser_download_url.clone(),
    })
}

fn platform_target(os: &str, arch: &str) -> Result<String> {
    platform::release_target(os, arch)
}

fn asset_file_name(
    release: &GithubRelease,
    channel: ReleaseChannel,
    target: &str,
) -> Result<String> {
    match channel {
        ReleaseChannel::Stable => {
            let version = parse_release_tag(&release.tag_name)?;
            Ok(format!("shine-v{version}-{target}.tar.gz"))
        }
        ReleaseChannel::Preview => Ok(format!("shine-preview-{target}.tar.gz")),
    }
}

async fn installed_version_label(current_exe: &Path, fallback: &str) -> String {
    match tokio::process::Command::new(current_exe)
        .arg("--version")
        .output()
        .await
    {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            parse_binary_version_output(&stdout)
                .map(str::to_string)
                .unwrap_or_else(|| fallback.to_string())
        }
        _ => fallback.to_string(),
    }
}

fn parse_binary_version_output(output: &str) -> Option<&str> {
    output.trim().strip_prefix("shine ")
}

fn preview_release_version_label(release: &GithubRelease) -> Option<String> {
    let commit = parse_preview_release_commit(&release.body)?;
    let short_commit: String = commit.chars().take(7).collect();
    if short_commit.len() != 7 {
        return None;
    }

    Some(format!("{}+preview.{short_commit}", version::package()))
}

fn parse_preview_release_commit(body: &str) -> Option<&str> {
    body.lines().find_map(|line| {
        line.trim()
            .strip_prefix("- Commit: `")
            .and_then(|rest| rest.strip_suffix('`'))
    })
}

#[cfg(unix)]
async fn set_executable_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .await
        .with_context(|| format!("failed to read metadata for {}", path.display()))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
        .await
        .with_context(|| format!("failed to mark {} as executable", path.display()))
}

#[cfg(not(unix))]
async fn set_executable_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::github::GithubReleaseAsset;
    use super::*;
    use flate2::{Compression, write::GzEncoder};
    use tar::{Builder, Header};

    #[test]
    fn parse_binary_version_output_reads_shine_version() {
        assert_eq!(
            parse_binary_version_output("shine 0.21.3+preview.5ed8416\n"),
            Some("0.21.3+preview.5ed8416")
        );
    }

    #[test]
    fn parse_binary_version_output_rejects_unexpected_output() {
        assert_eq!(parse_binary_version_output("0.21.3"), None);
    }

    fn archive_with_file(name: &str, content: &[u8]) -> Vec<u8> {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut builder = Builder::new(encoder);
        let mut header = Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, name, content)
            .expect("archive entry should be written");
        let encoder = builder.into_inner().expect("archive should finish");
        encoder.finish().expect("gzip should finish")
    }

    #[test]
    fn extract_binary_from_archive_uses_platform_binary_name() {
        let archive = archive_with_file("shine.exe", b"windows-binary");
        assert_eq!(
            extract_binary_from_archive(&archive, "shine.exe").unwrap(),
            b"windows-binary"
        );

        let err = extract_binary_from_archive(&archive, "shine").unwrap_err();
        assert!(err.to_string().contains("shine binary"));
    }

    #[test]
    fn platform_target_maps_supported_targets() {
        assert_eq!(
            platform_target("macos", "aarch64").unwrap(),
            "darwin-aarch64"
        );
        assert_eq!(platform_target("linux", "x86_64").unwrap(), "linux-x86_64");
        assert_eq!(
            platform_target("windows", "x86_64").unwrap(),
            "windows-x86_64"
        );
        assert_eq!(
            platform_target("windows", "aarch64").unwrap(),
            "windows-aarch64"
        );
    }

    #[test]
    fn asset_file_name_uses_versioned_target_name_for_stable() {
        let release = GithubRelease {
            tag_name: "v1.2.3".to_string(),
            body: String::new(),
            assets: vec![],
        };
        assert_eq!(
            asset_file_name(&release, ReleaseChannel::Stable, "darwin-aarch64").unwrap(),
            "shine-v1.2.3-darwin-aarch64.tar.gz"
        );
    }

    #[test]
    fn asset_file_name_uses_fixed_target_name_for_preview() {
        let release = GithubRelease {
            tag_name: "preview".to_string(),
            body: String::new(),
            assets: vec![],
        };
        assert_eq!(
            asset_file_name(&release, ReleaseChannel::Preview, "linux-x86_64").unwrap(),
            "shine-preview-linux-x86_64.tar.gz"
        );
        assert_eq!(
            asset_file_name(&release, ReleaseChannel::Preview, "windows-x86_64").unwrap(),
            "shine-preview-windows-x86_64.tar.gz"
        );
    }

    #[test]
    fn find_release_asset_selects_matching_stable_asset() {
        let release = GithubRelease {
            tag_name: "v1.2.3".to_string(),
            body: String::new(),
            assets: vec![
                GithubReleaseAsset {
                    name: "shine-v1.2.3-linux-x86_64.tar.gz".to_string(),
                    browser_download_url: "https://example.test/linux".to_string(),
                },
                GithubReleaseAsset {
                    name: "shine-v1.2.3-darwin-aarch64.tar.gz".to_string(),
                    browser_download_url: "https://example.test/macos".to_string(),
                },
            ],
        };

        let asset =
            find_release_asset(&release, ReleaseChannel::Stable, "macos", "aarch64").unwrap();
        assert_eq!(asset.release_tag, "v1.2.3");
        assert_eq!(asset.target, "darwin-aarch64");
        assert_eq!(asset.download_url, "https://example.test/macos");
    }

    #[test]
    fn find_release_asset_selects_matching_preview_asset_without_semver_tag() {
        let release = GithubRelease {
            tag_name: "preview".to_string(),
            body: String::new(),
            assets: vec![GithubReleaseAsset {
                name: "shine-preview-linux-x86_64.tar.gz".to_string(),
                browser_download_url: "https://example.test/preview".to_string(),
            }],
        };

        let asset =
            find_release_asset(&release, ReleaseChannel::Preview, "linux", "x86_64").unwrap();
        assert_eq!(asset.release_tag, "preview");
        assert_eq!(asset.target, "linux-x86_64");
        assert_eq!(asset.download_url, "https://example.test/preview");
    }

    #[test]
    fn find_release_asset_selects_matching_windows_asset() {
        let release = GithubRelease {
            tag_name: "v1.2.3".to_string(),
            body: String::new(),
            assets: vec![GithubReleaseAsset {
                name: "shine-v1.2.3-windows-x86_64.tar.gz".to_string(),
                browser_download_url: "https://example.test/windows".to_string(),
            }],
        };

        let asset =
            find_release_asset(&release, ReleaseChannel::Stable, "windows", "x86_64").unwrap();
        assert_eq!(asset.release_tag, "v1.2.3");
        assert_eq!(asset.target, "windows-x86_64");
        assert_eq!(asset.download_url, "https://example.test/windows");
    }

    #[test]
    fn find_release_asset_errors_when_target_missing() {
        let release = GithubRelease {
            tag_name: "v1.2.3".to_string(),
            body: String::new(),
            assets: vec![],
        };

        let error =
            find_release_asset(&release, ReleaseChannel::Stable, "linux", "x86_64").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("no release asset named shine-v1.2.3-linux-x86_64.tar.gz")
        );
    }

    #[test]
    fn parse_preview_release_commit_reads_commit_line_from_release_body() {
        let body = "Automated preview build from the release branch.\n- Commit: `a618d4af0a8ec0f0d0c5d4f6f9e3e2970cb12345`\n";

        assert_eq!(
            parse_preview_release_commit(body),
            Some("a618d4af0a8ec0f0d0c5d4f6f9e3e2970cb12345")
        );
    }

    #[test]
    fn preview_release_version_label_uses_package_version_and_short_commit() {
        let release = GithubRelease {
            tag_name: "preview".to_string(),
            body:
                "Automated preview build.\n- Commit: `a618d4af0a8ec0f0d0c5d4f6f9e3e2970cb12345`\n"
                    .to_string(),
            assets: vec![],
        };

        assert_eq!(
            preview_release_version_label(&release),
            Some(format!("{}+preview.a618d4a", version::package()))
        );
    }
}
