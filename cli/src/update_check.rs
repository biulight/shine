use crate::config::Config;
use anyhow::{Context, Result, anyhow, bail};
use flate2::read::GzDecoder;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue, USER_AGENT};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tar::Archive;
use tokio::fs;

const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/biulight/shine/releases/latest";
const UPDATE_CACHE_FILE: &str = "update-check.json";
const UPDATE_CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UpdateStatus {
    UpToDate,
    UpdateAvailable { latest: Version },
    UpdateRequired { latest: Version },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UpgradeResult {
    AlreadyUpToDate,
    Upgraded { previous: Version, latest: Version },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct UpdateCache {
    latest_version: String,
    checked_at_unix_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubReleaseAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubReleaseAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReleaseAsset {
    version: Version,
    target: String,
    download_url: String,
}

/// Always fetches from GitHub, ignoring the 24-hour cache.
pub(crate) async fn check_for_update_forced(config: &Config) -> Result<UpdateStatus> {
    let current = Version::parse(env!("CARGO_PKG_VERSION"))
        .context("current package version must be valid semver")?;
    let now_secs = unix_timestamp_now()?;
    let cache_path = config.shine_dir().join(UPDATE_CACHE_FILE);

    let release = fetch_latest_release().await?;
    let latest = parse_release_tag(&release.tag_name)?;
    store_cache(&cache_path, &latest, now_secs).await?;

    Ok(compare_versions(&current, &latest))
}

pub(crate) async fn check_for_update(config: &Config) -> Result<UpdateStatus> {
    let current = Version::parse(env!("CARGO_PKG_VERSION"))
        .context("current package version must be valid semver")?;
    let now_secs = unix_timestamp_now()?;
    let cache_path = config.shine_dir().join(UPDATE_CACHE_FILE);

    let latest = match load_cached_version_if_fresh(&cache_path, now_secs).await? {
        Some(version) => version,
        None => {
            let release = fetch_latest_release().await?;
            let fetched = parse_release_tag(&release.tag_name)?;
            store_cache(&cache_path, &fetched, now_secs).await?;
            fetched
        }
    };

    Ok(compare_versions(&current, &latest))
}

pub(crate) async fn upgrade_to_latest_release(config: &Config) -> Result<UpgradeResult> {
    let current = Version::parse(env!("CARGO_PKG_VERSION"))
        .context("current package version must be valid semver")?;
    let now_secs = unix_timestamp_now()?;
    let cache_path = config.shine_dir().join(UPDATE_CACHE_FILE);

    let release = fetch_latest_release().await?;
    let latest = parse_release_tag(&release.tag_name)?;
    store_cache(&cache_path, &latest, now_secs).await?;

    if latest <= current {
        return Ok(UpgradeResult::AlreadyUpToDate);
    }

    let asset = find_release_asset(&release, std::env::consts::OS, std::env::consts::ARCH)?;
    let archive_bytes = download_asset_bytes(&asset.download_url).await?;
    let current_exe = std::env::current_exe().context("failed to resolve current executable")?;
    install_downloaded_archive(&archive_bytes, &current_exe).await?;
    store_cache(&cache_path, &latest, now_secs).await?;

    Ok(UpgradeResult::Upgraded {
        previous: current,
        latest,
    })
}

fn compare_versions(current: &Version, latest: &Version) -> UpdateStatus {
    if latest <= current {
        return UpdateStatus::UpToDate;
    }

    if current.major == latest.major && current.minor == latest.minor {
        return UpdateStatus::UpdateRequired {
            latest: latest.clone(),
        };
    }

    UpdateStatus::UpdateAvailable {
        latest: latest.clone(),
    }
}

async fn load_cached_version_if_fresh(cache_path: &Path, now_secs: u64) -> Result<Option<Version>> {
    let cache = match fs::read_to_string(cache_path).await {
        Ok(content) => serde_json::from_str::<UpdateCache>(&content).ok(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(err).context("failed to read update cache"),
    };

    let Some(cache) = cache else {
        return Ok(None);
    };

    if cache.checked_at_unix_secs > now_secs {
        return Ok(None);
    }

    if now_secs - cache.checked_at_unix_secs >= UPDATE_CACHE_TTL.as_secs() {
        return Ok(None);
    }

    Ok(parse_release_tag(&cache.latest_version).ok())
}

async fn store_cache(cache_path: &Path, latest: &Version, checked_at_unix_secs: u64) -> Result<()> {
    let cache = UpdateCache {
        latest_version: latest.to_string(),
        checked_at_unix_secs,
    };
    let encoded = serde_json::to_vec_pretty(&cache).context("failed to serialize update cache")?;
    fs::write(cache_path, encoded)
        .await
        .context("failed to write update cache")?;
    Ok(())
}

async fn fetch_latest_release() -> Result<GithubRelease> {
    let client = github_client()?;
    client
        .get(GITHUB_LATEST_RELEASE_URL)
        .send()
        .await
        .context("failed to query GitHub latest release")?
        .error_for_status()
        .context("GitHub latest release request failed")?
        .json::<GithubRelease>()
        .await
        .context("failed to decode GitHub latest release response")
}

async fn download_asset_bytes(download_url: &str) -> Result<Vec<u8>> {
    let client = github_client()?;
    client
        .get(download_url)
        .send()
        .await
        .with_context(|| format!("failed to download release asset from {download_url}"))?
        .error_for_status()
        .with_context(|| format!("release asset request failed for {download_url}"))?
        .bytes()
        .await
        .context("failed to read release asset bytes")
        .map(|bytes| bytes.to_vec())
}

fn github_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .default_headers(default_headers()?)
        .timeout(Duration::from_secs(30))
        .build()
        .context("failed to build GitHub client")
}

async fn install_downloaded_archive(archive_bytes: &[u8], current_exe: &Path) -> Result<()> {
    let extracted = extract_binary_from_archive(archive_bytes)?;

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
            bail!(
                "cannot replace {} due to insufficient permissions; reinstall with install.sh into a user-writable directory such as ~/.local/bin",
                current_exe.display()
            );
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
        Err(err) => {
            let _ = fs::rename(&backup_path, current_exe).await;
            let _ = fs::remove_file(&staged_path).await;
            Err(err).with_context(|| {
                format!(
                    "failed to install upgraded binary at {}",
                    current_exe.display()
                )
            })
        }
    }
}

fn extract_binary_from_archive(archive_bytes: &[u8]) -> Result<Vec<u8>> {
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

        if path.file_name() == Some(OsStr::new("shine")) {
            let mut extracted = Vec::new();
            std::io::copy(&mut entry, &mut extracted)
                .context("failed to extract shine binary from release archive")?;
            if extracted.is_empty() {
                bail!("release archive contained an empty shine binary");
            }
            return Ok(extracted);
        }
    }

    bail!("release archive does not contain a shine binary")
}

fn find_release_asset(release: &GithubRelease, os: &str, arch: &str) -> Result<ReleaseAsset> {
    let version = parse_release_tag(&release.tag_name)?;
    let target = platform_target(os, arch)?;
    let expected_name = asset_file_name(&version, &target);

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
        version,
        target,
        download_url: asset.browser_download_url.clone(),
    })
}

fn platform_target(os: &str, arch: &str) -> Result<String> {
    let normalized_os = match os {
        "macos" => "darwin",
        "linux" => "linux",
        other => bail!("unsupported operating system: {other}"),
    };

    let normalized_arch = match arch {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => bail!("unsupported architecture: {other}"),
    };

    Ok(format!("{normalized_os}-{normalized_arch}"))
}

fn asset_file_name(version: &Version, target: &str) -> String {
    format!("shine-v{version}-{target}.tar.gz")
}

fn parse_release_tag(tag_name: &str) -> Result<Version> {
    let normalized = tag_name.trim().trim_start_matches('v');
    let version = Version::parse(normalized)
        .with_context(|| format!("invalid release tag version: {tag_name}"))?;

    if !version.pre.is_empty() {
        return Err(anyhow!(
            "pre-release tags are not eligible for update checks"
        ));
    }

    Ok(version)
}

fn default_headers() -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(
        USER_AGENT,
        HeaderValue::from_str(&format!("shine/{}", env!("CARGO_PKG_VERSION")))
            .context("invalid user-agent header")?,
    );
    Ok(headers)
}

fn unix_timestamp_now() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before unix epoch")?
        .as_secs())
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
    use super::*;
    use std::path::PathBuf;

    async fn make_temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shine-update-check-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).await.unwrap();
        dir
    }

    #[test]
    fn compare_versions_is_up_to_date_when_latest_is_not_newer() {
        let current = Version::parse("0.2.0").unwrap();
        let latest = Version::parse("0.2.0").unwrap();

        assert_eq!(compare_versions(&current, &latest), UpdateStatus::UpToDate);
    }

    #[test]
    fn compare_versions_requires_update_for_newer_patch_release() {
        let current = Version::parse("0.2.0").unwrap();
        let latest = Version::parse("0.2.1").unwrap();

        assert_eq!(
            compare_versions(&current, &latest),
            UpdateStatus::UpdateRequired { latest }
        );
    }

    #[test]
    fn compare_versions_warns_for_newer_minor_release() {
        let current = Version::parse("0.2.0").unwrap();
        let latest = Version::parse("0.3.0").unwrap();

        assert_eq!(
            compare_versions(&current, &latest),
            UpdateStatus::UpdateAvailable { latest }
        );
    }

    #[test]
    fn parse_release_tag_accepts_v_prefix() {
        let version = parse_release_tag("v1.2.3").unwrap();
        assert_eq!(version, Version::parse("1.2.3").unwrap());
    }

    #[test]
    fn parse_release_tag_rejects_prerelease_versions() {
        assert!(parse_release_tag("v1.2.3-beta.1").is_err());
    }

    #[test]
    fn platform_target_maps_supported_targets() {
        assert_eq!(
            platform_target("macos", "aarch64").unwrap(),
            "darwin-aarch64"
        );
        assert_eq!(platform_target("linux", "x86_64").unwrap(), "linux-x86_64");
    }

    #[test]
    fn asset_file_name_uses_versioned_target_name() {
        let version = Version::parse("1.2.3").unwrap();
        assert_eq!(
            asset_file_name(&version, "darwin-aarch64"),
            "shine-v1.2.3-darwin-aarch64.tar.gz"
        );
    }

    #[test]
    fn find_release_asset_selects_matching_asset() {
        let release = GithubRelease {
            tag_name: "v1.2.3".to_string(),
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

        let asset = find_release_asset(&release, "macos", "aarch64").unwrap();
        assert_eq!(asset.version, Version::parse("1.2.3").unwrap());
        assert_eq!(asset.target, "darwin-aarch64");
        assert_eq!(asset.download_url, "https://example.test/macos");
    }

    #[test]
    fn find_release_asset_errors_when_target_missing() {
        let release = GithubRelease {
            tag_name: "v1.2.3".to_string(),
            assets: vec![],
        };

        let error = find_release_asset(&release, "linux", "x86_64").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("no release asset named shine-v1.2.3-linux-x86_64.tar.gz")
        );
    }

    #[tokio::test]
    async fn load_cached_version_returns_none_when_cache_missing() {
        let dir = make_temp_dir().await;
        let cache_path = dir.join(UPDATE_CACHE_FILE);

        let cached = load_cached_version_if_fresh(&cache_path, UPDATE_CACHE_TTL.as_secs())
            .await
            .unwrap();
        assert_eq!(cached, None);

        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn load_cached_version_uses_fresh_cache() {
        let dir = make_temp_dir().await;
        let cache_path = dir.join(UPDATE_CACHE_FILE);
        store_cache(&cache_path, &Version::parse("0.2.3").unwrap(), 1_000)
            .await
            .unwrap();

        let cached =
            load_cached_version_if_fresh(&cache_path, 1_000 + UPDATE_CACHE_TTL.as_secs() - 1)
                .await
                .unwrap();
        assert_eq!(cached, Some(Version::parse("0.2.3").unwrap()));

        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn load_cached_version_ignores_stale_cache() {
        let dir = make_temp_dir().await;
        let cache_path = dir.join(UPDATE_CACHE_FILE);
        store_cache(&cache_path, &Version::parse("0.2.3").unwrap(), 1_000)
            .await
            .unwrap();

        let cached = load_cached_version_if_fresh(&cache_path, 1_000 + UPDATE_CACHE_TTL.as_secs())
            .await
            .unwrap();
        assert_eq!(cached, None);

        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn load_cached_version_ignores_invalid_cache_contents() {
        let dir = make_temp_dir().await;
        let cache_path = dir.join(UPDATE_CACHE_FILE);
        fs::write(&cache_path, b"{not valid json").await.unwrap();

        let cached = load_cached_version_if_fresh(&cache_path, UPDATE_CACHE_TTL.as_secs())
            .await
            .unwrap();
        assert_eq!(cached, None);

        fs::remove_dir_all(dir).await.unwrap();
    }
}
