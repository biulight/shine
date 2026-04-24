use crate::config::Config;
use anyhow::{Context, Result, anyhow};
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue, USER_AGENT};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct UpdateCache {
    latest_version: String,
    checked_at_unix_secs: u64,
}

#[derive(Debug, Deserialize)]
struct GithubLatestRelease {
    tag_name: String,
}

/// Always fetches from GitHub, ignoring the 24-hour cache.
pub(crate) async fn check_for_update_forced(config: &Config) -> Result<UpdateStatus> {
    let current = Version::parse(env!("CARGO_PKG_VERSION"))
        .context("current package version must be valid semver")?;
    let now_secs = unix_timestamp_now()?;
    let cache_path = config.shine_dir().join(UPDATE_CACHE_FILE);

    let latest = fetch_latest_version().await?;
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
            let fetched = fetch_latest_version().await?;
            store_cache(&cache_path, &fetched, now_secs).await?;
            fetched
        }
    };

    Ok(compare_versions(&current, &latest))
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

async fn fetch_latest_version() -> Result<Version> {
    let client = reqwest::Client::builder()
        .default_headers(default_headers()?)
        .timeout(Duration::from_secs(5))
        .build()
        .context("failed to build update-check client")?;

    let response = client
        .get(GITHUB_LATEST_RELEASE_URL)
        .send()
        .await
        .context("failed to query GitHub latest release")?
        .error_for_status()
        .context("GitHub latest release request failed")?;

    let release = response
        .json::<GithubLatestRelease>()
        .await
        .context("failed to decode GitHub latest release response")?;

    parse_release_tag(&release.tag_name)
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn make_temp_dir() -> std::path::PathBuf {
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
