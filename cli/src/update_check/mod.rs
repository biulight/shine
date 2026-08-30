use crate::commands::{AppCommands, Commands, TaskCommands};
use crate::{config::Config, version};
use anyhow::{Context, Result, anyhow, bail};
use clap::ValueEnum;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::fs;

mod github;
mod upgrade;

use github::{current_auth_mode, fetch_latest_release_for_version_check};
pub use upgrade::upgrade_to_release;

const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/biulight/shine/releases/latest";
const GITHUB_PREVIEW_RELEASE_URL: &str =
    "https://api.github.com/repos/biulight/shine/releases/tags/preview";
const UPDATE_CACHE_FILE: &str = "update-check.json";
const UPDATE_CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum ReleaseChannel {
    Stable,
    Preview,
}

impl ReleaseChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Preview => "preview",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    UpToDate,
    UpdateAvailable { latest: Version },
    UpdateRequired { latest: Version },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpgradeResult {
    AlreadyUpToDate {
        channel: ReleaseChannel,
        latest: String,
    },
    Upgraded {
        channel: ReleaseChannel,
        previous: Version,
        previous_display: String,
        release_tag: String,
        installed_version: String,
        installed_path: PathBuf,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct UpdateCache {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    latest_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    checked_at_unix_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rate_limited_until_unix_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rate_limited_auth_mode: Option<AuthMode>,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum AuthMode {
    Anonymous,
    Token,
}

/// Always fetches from GitHub, ignoring the 24-hour cache.
pub async fn check_for_update_forced(config: &Config) -> Result<UpdateStatus> {
    let current = Version::parse(env!("CARGO_PKG_VERSION"))
        .context("current package version must be valid semver")?;
    let now_secs = unix_timestamp_now()?;
    let cache_path = config.shine_dir().join(UPDATE_CACHE_FILE);

    guard_rate_limit_cooldown(&cache_path, now_secs, current_auth_mode()).await?;
    let release = fetch_latest_release_for_version_check(&cache_path, now_secs).await?;
    let latest = parse_release_tag(&release.tag_name)?;
    store_cache_if_possible(&cache_path, &latest, now_secs).await;

    Ok(compare_versions(&current, &latest))
}

pub async fn check_for_update(config: &Config) -> Result<UpdateStatus> {
    let current = Version::parse(env!("CARGO_PKG_VERSION"))
        .context("current package version must be valid semver")?;
    let now_secs = unix_timestamp_now()?;
    let cache_path = config.shine_dir().join(UPDATE_CACHE_FILE);

    let latest = match load_cached_version_if_fresh(&cache_path, now_secs).await? {
        Some(version) => version,
        None => {
            guard_rate_limit_cooldown(&cache_path, now_secs, current_auth_mode()).await?;
            let release = fetch_latest_release_for_version_check(&cache_path, now_secs).await?;
            let fetched = parse_release_tag(&release.tag_name)?;
            store_cache_if_possible(&cache_path, &fetched, now_secs).await;
            fetched
        }
    };

    Ok(compare_versions(&current, &latest))
}

/// Runs the background version check for `command`, unless `command` is one
/// that already does its own forced fetch (`shine update`, `shine self
/// upgrade`) or otherwise shouldn't be gated on it (`shine self install`
/// should stay available even when the current binary is version-gated).
///
/// A check failure must never fail the user's command: network errors,
/// GitHub API errors, and the like are swallowed silently here, same as
/// before this was extracted from `main.rs`.
pub async fn maybe_notify(config: &Config, command: &Commands) -> Result<()> {
    // Skip the background version check for update/self commands. `shine update`
    // and `shine self upgrade` do their own forced fetch below; `shine self install`
    // should remain available even when the current binary is version-gated.
    if !skip_background_update_check(command) {
        match check_for_update(config).await {
            Ok(UpdateStatus::UpToDate) => {}
            Ok(UpdateStatus::UpdateAvailable { latest }) => {
                eprintln!(
                    "A newer version of shine is available: {} -> {}. Run `shine self upgrade` when convenient.",
                    version::semver(),
                    latest
                );
            }
            Ok(UpdateStatus::UpdateRequired { latest }) => {
                bail!(
                    "A newer patch release of shine is required: {} -> {}. Run `shine self upgrade` before continuing.",
                    version::semver(),
                    latest
                );
            }
            Err(_) => {}
        }
    }
    Ok(())
}

fn skip_background_update_check(command: &Commands) -> bool {
    matches!(
        command,
        Commands::Update(..)
            | Commands::Preset { .. }
            | Commands::State { .. }
            | Commands::Self_ { .. }
            | Commands::Serve { .. }
            | Commands::Env { .. }
            | Commands::Run(..)
    ) || matches!(command, Commands::Upgrade(cmd) if cmd.pull)
        || matches!(
            command,
            Commands::App {
                command: AppCommands::Recover { .. }
            }
        )
        || matches!(
            command,
            Commands::Task {
                command: TaskCommands::Run(..)
            }
        )
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

async fn load_cached_version_if_fresh(cache_path: &Path, now_secs: u64) -> Result<Option<Version>> {
    let cache = match fs::read_to_string(cache_path).await {
        Ok(content) => serde_json::from_str::<UpdateCache>(&content).ok(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(err).context("failed to read update cache"),
    };

    let Some(cache) = cache else {
        return Ok(None);
    };

    let Some(checked_at_unix_secs) = cache.checked_at_unix_secs else {
        return Ok(None);
    };
    let Some(latest_version) = cache.latest_version else {
        return Ok(None);
    };

    if checked_at_unix_secs > now_secs {
        return Ok(None);
    }

    if now_secs - checked_at_unix_secs >= UPDATE_CACHE_TTL.as_secs() {
        return Ok(None);
    }

    Ok(parse_release_tag(&latest_version).ok())
}

async fn store_cache(cache_path: &Path, latest: &Version, checked_at_unix_secs: u64) -> Result<()> {
    let cache = UpdateCache {
        latest_version: Some(latest.to_string()),
        checked_at_unix_secs: Some(checked_at_unix_secs),
        rate_limited_until_unix_secs: None,
        rate_limited_auth_mode: None,
    };
    write_cache(cache_path, &cache).await
}

async fn store_rate_limit_cache(
    cache_path: &Path,
    rate_limited_until_unix_secs: u64,
    auth_mode: AuthMode,
) -> Result<()> {
    let mut cache = load_cache(cache_path).await?.unwrap_or(UpdateCache {
        latest_version: None,
        checked_at_unix_secs: None,
        rate_limited_until_unix_secs: None,
        rate_limited_auth_mode: None,
    });
    cache.rate_limited_until_unix_secs = Some(rate_limited_until_unix_secs);
    cache.rate_limited_auth_mode = Some(auth_mode);
    write_cache(cache_path, &cache).await
}

async fn write_cache(cache_path: &Path, cache: &UpdateCache) -> Result<()> {
    if let Some(parent) = cache_path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create update cache dir {}", parent.display()))?;
    }

    let encoded = serde_json::to_vec_pretty(&cache).context("failed to serialize update cache")?;
    fs::write(cache_path, encoded)
        .await
        .context("failed to write update cache")?;
    Ok(())
}

async fn store_cache_if_possible(cache_path: &Path, latest: &Version, checked_at_unix_secs: u64) {
    if let Err(e) = store_cache(cache_path, latest, checked_at_unix_secs).await {
        eprintln!("warning: failed to write update cache: {e:#}");
    }
}

async fn store_rate_limit_cache_if_possible(
    cache_path: &Path,
    rate_limited_until_unix_secs: u64,
    auth_mode: AuthMode,
) {
    if let Err(e) =
        store_rate_limit_cache(cache_path, rate_limited_until_unix_secs, auth_mode).await
    {
        eprintln!("warning: failed to write update rate-limit cache: {e:#}");
    }
}

async fn load_cache(cache_path: &Path) -> Result<Option<UpdateCache>> {
    match fs::read_to_string(cache_path).await {
        Ok(content) => Ok(serde_json::from_str::<UpdateCache>(&content).ok()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).context("failed to read update cache"),
    }
}

async fn guard_rate_limit_cooldown(
    cache_path: &Path,
    now_secs: u64,
    auth_mode: AuthMode,
) -> Result<()> {
    let Some(cache) = load_cache(cache_path).await? else {
        return Ok(());
    };
    let Some(rate_limited_until) = cache.rate_limited_until_unix_secs else {
        return Ok(());
    };
    let Some(rate_limited_auth_mode) = cache.rate_limited_auth_mode else {
        return Ok(());
    };

    if rate_limited_auth_mode == auth_mode && rate_limited_until > now_secs {
        bail!(
            "GitHub version check skipped until Unix timestamp {rate_limited_until} due to rate limiting"
        );
    }

    Ok(())
}

/// Removes the on-disk update cache so the next command performs a fresh fetch
/// rather than reading a stale "update required" entry left behind by a failed upgrade.
pub async fn invalidate_update_cache(config: &Config) {
    let cache_path = config.shine_dir().join(UPDATE_CACHE_FILE);
    if let Err(e) = fs::remove_file(&cache_path).await
        && e.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("warning: failed to remove update cache: {e:#}");
    }
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
    use std::path::PathBuf;

    async fn make_temp_dir() -> PathBuf {
        crate::test_support::make_temp_dir("shine-update-check").await
    }

    #[test]
    fn explicit_app_recovery_skips_background_update_gate() {
        let command = Commands::App {
            command: AppCommands::Recover { yes: false },
        };
        assert!(skip_background_update_check(&command));
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
    async fn load_cached_version_supports_legacy_cache_shape() {
        let dir = make_temp_dir().await;
        let cache_path = dir.join(UPDATE_CACHE_FILE);
        fs::write(
            &cache_path,
            br#"{"latest_version":"0.2.3","checked_at_unix_secs":1000}"#,
        )
        .await
        .unwrap();

        let cached = load_cached_version_if_fresh(&cache_path, 1_001)
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

    #[tokio::test]
    async fn store_cache_creates_missing_parent_directory() {
        let dir = make_temp_dir().await;
        let cache_path = dir.join("nested").join(UPDATE_CACHE_FILE);

        store_cache(&cache_path, &Version::parse("0.2.3").unwrap(), 1_000)
            .await
            .unwrap();

        let cached = load_cached_version_if_fresh(&cache_path, 1_000 + 1)
            .await
            .unwrap();
        assert_eq!(cached, Some(Version::parse("0.2.3").unwrap()));

        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn rate_limit_cooldown_skips_same_auth_mode() {
        let dir = make_temp_dir().await;
        let cache_path = dir.join(UPDATE_CACHE_FILE);
        store_rate_limit_cache(&cache_path, 2_000, AuthMode::Anonymous)
            .await
            .unwrap();

        let err = guard_rate_limit_cooldown(&cache_path, 1_000, AuthMode::Anonymous)
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("GitHub version check skipped until Unix timestamp 2000")
        );

        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn rate_limit_cooldown_allows_changed_auth_mode() {
        let dir = make_temp_dir().await;
        let cache_path = dir.join(UPDATE_CACHE_FILE);
        store_rate_limit_cache(&cache_path, 2_000, AuthMode::Anonymous)
            .await
            .unwrap();

        guard_rate_limit_cooldown(&cache_path, 1_000, AuthMode::Token)
            .await
            .unwrap();

        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn rate_limit_cooldown_allows_expired_reset() {
        let dir = make_temp_dir().await;
        let cache_path = dir.join(UPDATE_CACHE_FILE);
        store_rate_limit_cache(&cache_path, 2_000, AuthMode::Token)
            .await
            .unwrap();

        guard_rate_limit_cooldown(&cache_path, 2_001, AuthMode::Token)
            .await
            .unwrap();

        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn successful_cache_write_clears_rate_limit_cooldown() {
        let dir = make_temp_dir().await;
        let cache_path = dir.join(UPDATE_CACHE_FILE);
        store_rate_limit_cache(&cache_path, 2_000, AuthMode::Anonymous)
            .await
            .unwrap();
        store_cache(&cache_path, &Version::parse("0.2.3").unwrap(), 1_000)
            .await
            .unwrap();

        let cache = load_cache(&cache_path).await.unwrap().unwrap();
        assert_eq!(cache.latest_version.as_deref(), Some("0.2.3"));
        assert_eq!(cache.rate_limited_until_unix_secs, None);
        assert_eq!(cache.rate_limited_auth_mode, None);

        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn invalidate_update_cache_removes_existing_cache_file() {
        use crate::config::Config;

        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        let cache_path = dir.join(UPDATE_CACHE_FILE);

        store_cache(&cache_path, &Version::parse("0.2.3").unwrap(), 1_000)
            .await
            .unwrap();
        assert!(
            cache_path.exists(),
            "cache file should exist before invalidation"
        );

        invalidate_update_cache(&config).await;
        assert!(
            !cache_path.exists(),
            "cache file should be removed after invalidation"
        );

        fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn invalidate_update_cache_is_a_no_op_when_cache_absent() {
        use crate::config::Config;

        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);

        // Should not return an error when the cache file does not exist.
        invalidate_update_cache(&config).await;

        fs::remove_dir_all(dir).await.unwrap();
    }
}
