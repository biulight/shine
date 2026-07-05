use crate::{config::Config, platform, version};
use anyhow::{Context, Result, anyhow, bail};
use clap::ValueEnum;
use flate2::read::GzDecoder;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tar::Archive;
use tokio::fs;

const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/biulight/shine/releases/latest";
const GITHUB_PREVIEW_RELEASE_URL: &str =
    "https://api.github.com/repos/biulight/shine/releases/tags/preview";
const UPDATE_CACHE_FILE: &str = "update-check.json";
const UPDATE_CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// Defense-in-depth cap on downloaded release archives; the real binaries
/// are a few MB, so anything past this points at a corrupted or malicious asset.
const MAX_RELEASE_ASSET_BYTES: u64 = 200 * 1024 * 1024;

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

#[derive(Debug, Clone, Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    body: String,
    assets: Vec<GithubReleaseAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubReleaseAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReleaseAsset {
    release_tag: String,
    target: String,
    download_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GithubApiError {
    channel: ReleaseChannel,
    status: reqwest::StatusCode,
    message: String,
    rate_limit: GithubRateLimit,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct GithubRateLimit {
    remaining: Option<String>,
    reset: Option<u64>,
    resource: Option<String>,
    retry_after: Option<String>,
}

impl fmt::Display for GithubApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&format_github_api_error(
            self.channel,
            self.status,
            &self.message,
            &self.rate_limit,
        ))
    }
}

impl Error for GithubApiError {}

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
            let cache_path = config.shine_dir().join(UPDATE_CACHE_FILE);
            store_cache_if_possible(&cache_path, &latest, now_secs).await;

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
        let cache_path = config.shine_dir().join(UPDATE_CACHE_FILE);
        store_cache_if_possible(&cache_path, latest, now_secs).await;
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

async fn fetch_latest_release() -> Result<GithubRelease> {
    fetch_release(ReleaseChannel::Stable).await
}

async fn fetch_latest_release_for_version_check(
    cache_path: &Path,
    now_secs: u64,
) -> Result<GithubRelease> {
    let auth_mode = current_auth_mode();
    match fetch_latest_release().await {
        Ok(release) => Ok(release),
        Err(err) => {
            if let Some(reset) = rate_limit_reset_from_error(&err, now_secs) {
                store_rate_limit_cache_if_possible(cache_path, reset, auth_mode).await;
            }
            Err(err)
        }
    }
}

async fn fetch_release(channel: ReleaseChannel) -> Result<GithubRelease> {
    let client = github_client()?;
    let url = match channel {
        ReleaseChannel::Stable => GITHUB_LATEST_RELEASE_URL,
        ReleaseChannel::Preview => GITHUB_PREVIEW_RELEASE_URL,
    };
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("failed to query GitHub {} release", channel.as_str()))?;

    if !response.status().is_success() {
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!(GithubApiError::from_response(
            channel, status, &headers, &body
        )));
    }

    response.json::<GithubRelease>().await.with_context(|| {
        format!(
            "failed to decode GitHub {} release response",
            channel.as_str()
        )
    })
}

async fn download_asset_bytes(download_url: &str) -> Result<Vec<u8>> {
    let client = github_client()?;
    let response = client
        .get(download_url)
        .send()
        .await
        .with_context(|| format!("failed to download release asset from {download_url}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!(
            "release asset request failed for {download_url}: HTTP {status}{}",
            format_error_body_suffix(&body)
        );
    }

    if let Some(len) = response.content_length()
        && len > MAX_RELEASE_ASSET_BYTES
    {
        bail!(
            "release asset at {download_url} reports {len} bytes, exceeding the {MAX_RELEASE_ASSET_BYTES}-byte limit"
        );
    }

    let bytes = response
        .bytes()
        .await
        .context("failed to read release asset bytes")?;

    if bytes.len() as u64 > MAX_RELEASE_ASSET_BYTES {
        bail!(
            "release asset at {download_url} is {} bytes, exceeding the {MAX_RELEASE_ASSET_BYTES}-byte limit",
            bytes.len()
        );
    }

    Ok(bytes.to_vec())
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

fn github_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .default_headers(default_headers()?)
        .timeout(Duration::from_secs(30))
        .build()
        .context("failed to build GitHub client")
}

fn github_auth_token() -> Option<String> {
    github_auth_token_from_vars(|name| std::env::var(name).ok())
}

fn github_auth_token_from_vars(mut var: impl FnMut(&str) -> Option<String>) -> Option<String> {
    for name in ["GITHUB_TOKEN", "GH_TOKEN"] {
        let Some(token) = var(name) else {
            continue;
        };
        let token = token.trim();
        if !token.is_empty() {
            return Some(token.to_string());
        }
    }
    None
}

fn current_auth_mode() -> AuthMode {
    auth_mode_from_vars(|name| std::env::var(name).ok())
}

fn auth_mode_from_vars(var: impl FnMut(&str) -> Option<String>) -> AuthMode {
    if github_auth_token_from_vars(var).is_some() {
        AuthMode::Token
    } else {
        AuthMode::Anonymous
    }
}

fn auth_header_value(token: &str) -> Result<HeaderValue> {
    HeaderValue::from_str(&format!("Bearer {token}")).context("invalid GitHub token header")
}

fn github_api_message(body: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct ErrorBody {
        message: String,
    }

    serde_json::from_str::<ErrorBody>(body)
        .ok()
        .map(|err| err.message)
        .filter(|message| !message.trim().is_empty())
}

fn header_to_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name)?.to_str().ok()
}

impl GithubApiError {
    fn from_response(
        channel: ReleaseChannel,
        status: reqwest::StatusCode,
        headers: &HeaderMap,
        body: &str,
    ) -> Self {
        Self {
            channel,
            status,
            message: github_api_message(body).unwrap_or_else(|| {
                format_error_body_suffix(body)
                    .trim_start_matches(": ")
                    .to_string()
            }),
            rate_limit: GithubRateLimit::from_headers(headers),
        }
    }
}

impl GithubRateLimit {
    fn from_headers(headers: &HeaderMap) -> Self {
        Self {
            remaining: header_to_str(headers, "x-ratelimit-remaining").map(str::to_string),
            reset: header_to_str(headers, "x-ratelimit-reset")
                .and_then(|value| value.parse::<u64>().ok()),
            resource: header_to_str(headers, "x-ratelimit-resource").map(str::to_string),
            retry_after: header_to_str(headers, "retry-after").map(str::to_string),
        }
    }
}

fn rate_limit_reset_from_error(err: &anyhow::Error, now_secs: u64) -> Option<u64> {
    let err = err.downcast_ref::<GithubApiError>()?;
    if !matches!(
        err.status,
        reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::TOO_MANY_REQUESTS
    ) {
        return None;
    }
    if err.rate_limit.remaining.as_deref() != Some("0") {
        return None;
    }
    let reset = err.rate_limit.reset?;
    (reset > now_secs).then_some(reset)
}

fn format_github_api_error(
    channel: ReleaseChannel,
    status: reqwest::StatusCode,
    api_message: &str,
    rate_limit: &GithubRateLimit,
) -> String {
    let mut formatted = format!(
        "GitHub {} release request failed: HTTP {status}",
        channel.as_str()
    );

    if !api_message.trim().is_empty() {
        formatted.push_str(&format!(": {api_message}"));
    }

    let mut rate_limit_parts = Vec::new();
    if let Some(remaining) = &rate_limit.remaining {
        rate_limit_parts.push(format!("remaining={remaining}"));
    }
    if let Some(reset) = rate_limit.reset {
        rate_limit_parts.push(format!("reset={reset}"));
    }
    if let Some(resource) = &rate_limit.resource {
        rate_limit_parts.push(format!("resource={resource}"));
    }
    if let Some(retry_after) = &rate_limit.retry_after {
        rate_limit_parts.push(format!("retry-after={retry_after}s"));
    }

    if !rate_limit_parts.is_empty() {
        formatted.push_str(&format!(" (rate limit: {})", rate_limit_parts.join(", ")));
    }

    formatted
}

fn format_error_body_suffix(body: &str) -> String {
    let body = body.trim();
    if body.is_empty() {
        return String::new();
    }

    format!(": {}", truncate_for_error(body, 300))
}

fn truncate_for_error(value: &str, max_chars: usize) -> String {
    let mut truncated: String = value.chars().take(max_chars).collect();
    if value.chars().count() > max_chars {
        truncated.push_str("...");
    }
    truncated
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
        HeaderValue::from_str(&format!("shine/{}", version::package()))
            .context("invalid user-agent header")?,
    );
    if let Some(token) = github_auth_token() {
        headers.insert(AUTHORIZATION, auth_header_value(&token)?);
    }
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
    use flate2::{Compression, write::GzEncoder};
    use std::path::PathBuf;
    use tar::{Builder, Header};

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

    #[test]
    fn github_auth_token_uses_github_token_first() {
        let token = github_auth_token_from_vars(|name| match name {
            "GITHUB_TOKEN" => Some(" github-value ".to_string()),
            "GH_TOKEN" => Some("gh-value".to_string()),
            _ => None,
        });

        assert_eq!(token.as_deref(), Some("github-value"));
    }

    #[test]
    fn github_auth_token_uses_gh_token_when_github_token_is_absent() {
        let token = github_auth_token_from_vars(|name| match name {
            "GH_TOKEN" => Some("gh-value".to_string()),
            _ => None,
        });

        assert_eq!(token.as_deref(), Some("gh-value"));
    }

    #[test]
    fn github_auth_token_ignores_empty_values() {
        let token = github_auth_token_from_vars(|name| match name {
            "GITHUB_TOKEN" => Some("  ".to_string()),
            "GH_TOKEN" => Some("gh-value".to_string()),
            _ => None,
        });

        assert_eq!(token.as_deref(), Some("gh-value"));
    }

    #[test]
    fn github_auth_token_returns_none_without_token() {
        assert_eq!(github_auth_token_from_vars(|_| None), None);
    }

    #[test]
    fn auth_mode_tracks_token_presence_without_storing_token_value() {
        assert_eq!(auth_mode_from_vars(|_| None), AuthMode::Anonymous);
        assert_eq!(
            auth_mode_from_vars(|name| match name {
                "GH_TOKEN" => Some(" token ".to_string()),
                _ => None,
            }),
            AuthMode::Token
        );
    }

    #[test]
    fn auth_header_value_formats_bearer_token() {
        assert_eq!(
            auth_header_value("secret-token").unwrap(),
            HeaderValue::from_static("Bearer secret-token")
        );
    }

    #[test]
    fn github_api_error_includes_json_message_and_rate_limit_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-ratelimit-remaining", HeaderValue::from_static("0"));
        headers.insert("x-ratelimit-reset", HeaderValue::from_static("1780937643"));
        headers.insert("x-ratelimit-resource", HeaderValue::from_static("core"));

        let err = GithubApiError::from_response(
            ReleaseChannel::Stable,
            reqwest::StatusCode::FORBIDDEN,
            &headers,
            r#"{"message":"API rate limit exceeded for 156.0.200.135."}"#,
        );
        let message = err.to_string();

        assert!(message.contains("GitHub stable release request failed: HTTP 403 Forbidden"));
        assert!(message.contains("API rate limit exceeded for 156.0.200.135."));
        assert!(message.contains("remaining=0"));
        assert!(message.contains("reset=1780937643"));
        assert!(message.contains("resource=core"));
    }

    #[test]
    fn github_api_error_extracts_rate_limit_reset_for_primary_limit() {
        let mut headers = HeaderMap::new();
        headers.insert("x-ratelimit-remaining", HeaderValue::from_static("0"));
        headers.insert("x-ratelimit-reset", HeaderValue::from_static("1780937643"));
        let err = anyhow!(GithubApiError::from_response(
            ReleaseChannel::Stable,
            reqwest::StatusCode::FORBIDDEN,
            &headers,
            r#"{"message":"API rate limit exceeded"}"#,
        ));

        assert_eq!(
            rate_limit_reset_from_error(&err, 1_780_000_000),
            Some(1_780_937_643)
        );
    }

    #[test]
    fn github_api_error_ignores_non_rate_limit_responses() {
        let mut headers = HeaderMap::new();
        headers.insert("x-ratelimit-remaining", HeaderValue::from_static("12"));
        headers.insert("x-ratelimit-reset", HeaderValue::from_static("1780937643"));
        let err = anyhow!(GithubApiError::from_response(
            ReleaseChannel::Stable,
            reqwest::StatusCode::FORBIDDEN,
            &headers,
            r#"{"message":"forbidden"}"#,
        ));
        assert_eq!(rate_limit_reset_from_error(&err, 1_780_000_000), None);

        headers.insert("x-ratelimit-remaining", HeaderValue::from_static("0"));
        let err = anyhow!(GithubApiError::from_response(
            ReleaseChannel::Stable,
            reqwest::StatusCode::BAD_GATEWAY,
            &headers,
            "gateway failure",
        ));
        assert_eq!(rate_limit_reset_from_error(&err, 1_780_000_000), None);
    }

    #[test]
    fn github_api_error_includes_truncated_non_json_body() {
        let body = "x".repeat(350);
        let err = GithubApiError::from_response(
            ReleaseChannel::Preview,
            reqwest::StatusCode::BAD_GATEWAY,
            &HeaderMap::new(),
            &body,
        );
        let message = err.to_string();

        assert!(message.contains("GitHub preview release request failed: HTTP 502 Bad Gateway"));
        assert!(message.contains(&"x".repeat(300)));
        assert!(message.ends_with("..."));
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
