use super::{AuthMode, GITHUB_LATEST_RELEASE_URL, GITHUB_PREVIEW_RELEASE_URL, ReleaseChannel};
use crate::version;
use anyhow::{Context, Result, anyhow, bail};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};
use serde::Deserialize;
use std::error::Error;
use std::fmt;
use std::path::Path;
use std::time::Duration;

/// Defense-in-depth cap on downloaded release archives; the real binaries
/// are a few MB, so anything past this points at a corrupted or malicious asset.
const MAX_RELEASE_ASSET_BYTES: u64 = 200 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GithubRelease {
    pub(super) tag_name: String,
    #[serde(default)]
    pub(super) body: String,
    pub(super) assets: Vec<GithubReleaseAsset>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GithubReleaseAsset {
    pub(super) name: String,
    pub(super) browser_download_url: String,
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

pub(super) fn current_auth_mode() -> AuthMode {
    auth_mode_from_vars(|name| std::env::var(name).ok())
}

fn auth_mode_from_vars(var: impl FnMut(&str) -> Option<String>) -> AuthMode {
    if github_auth_token_from_vars(var).is_some() {
        AuthMode::Token
    } else {
        AuthMode::Anonymous
    }
}

pub(super) async fn fetch_latest_release_for_version_check(
    cache_path: &Path,
    now_secs: u64,
) -> Result<GithubRelease> {
    let auth_mode = current_auth_mode();
    match fetch_latest_release().await {
        Ok(release) => Ok(release),
        Err(err) => {
            if let Some(reset) = rate_limit_reset_from_error(&err, now_secs) {
                super::store_rate_limit_cache_if_possible(cache_path, reset, auth_mode).await;
            }
            Err(err)
        }
    }
}

async fn fetch_latest_release() -> Result<GithubRelease> {
    fetch_release(ReleaseChannel::Stable).await
}

pub(super) async fn fetch_release(channel: ReleaseChannel) -> Result<GithubRelease> {
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

pub(super) async fn download_asset_bytes(download_url: &str) -> Result<Vec<u8>> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
