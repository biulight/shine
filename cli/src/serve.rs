use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;

use crate::config::Config;

const DEFAULT_HOST: &str = "127.0.0.1";
const LAUNCHD_LABEL: &str = "top.biulight.shine.http";
const MAX_HEADER_BYTES: usize = 8192;

pub async fn handle_install(config: &Config, port: u16) -> Result<()> {
    ensure_macos_service_support()?;

    let root = http_root(config);
    fs::create_dir_all(&root)
        .await
        .with_context(|| format!("creating {}", root.display()))?;

    let plist_path = launchd_plist_path(config);
    let parent = plist_path
        .parent()
        .context("launchd plist path must have a parent directory")?;
    fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;

    let executable = service_executable(config)?;
    let plist = launchd_plist(&executable, port);
    fs::write(&plist_path, plist)
        .await
        .with_context(|| format!("writing {}", plist_path.display()))?;

    let plist_arg = plist_path.to_string_lossy().to_string();
    let _ = launchctl(&["unload", "-w", &plist_arg]).await;
    launchctl(&["load", "-w", &plist_arg]).await?;

    println!("Installed {LAUNCHD_LABEL}");
    println!("Serving {}", root.display());
    println!("URL base: http://{DEFAULT_HOST}:{port}/");
    Ok(())
}

pub async fn handle_start(config: &Config, port: u16) -> Result<()> {
    let root = http_root(config);
    fs::create_dir_all(&root)
        .await
        .with_context(|| format!("creating {}", root.display()))?;

    let listener = TcpListener::bind((DEFAULT_HOST, port))
        .await
        .with_context(|| format!("binding {DEFAULT_HOST}:{port}"))?;
    println!("Serving {}", root.display());
    println!("URL base: http://{DEFAULT_HOST}:{port}/");

    loop {
        let (stream, _) = listener.accept().await?;
        let root = root.clone();
        tokio::spawn(async move {
            let _ = handle_connection(stream, root).await;
        });
    }
}

pub async fn handle_status(config: &Config) -> Result<()> {
    ensure_macos_service_support()?;

    let plist_path = launchd_plist_path(config);
    if !plist_path.exists() {
        println!("Not installed");
        return Ok(());
    }

    match launchctl(&["list", LAUNCHD_LABEL]).await {
        Ok(()) => println!("Installed and loaded"),
        Err(_) => println!("Installed but not loaded"),
    }
    println!("Plist: {}", plist_path.display());
    Ok(())
}

pub async fn handle_uninstall(config: &Config) -> Result<()> {
    ensure_macos_service_support()?;

    let plist_path = launchd_plist_path(config);
    if plist_path.exists() {
        let plist_arg = plist_path.to_string_lossy().to_string();
        let _ = launchctl(&["unload", "-w", &plist_arg]).await;
        fs::remove_file(&plist_path)
            .await
            .with_context(|| format!("removing {}", plist_path.display()))?;
        println!("Uninstalled {LAUNCHD_LABEL}");
    } else {
        println!("Not installed");
    }
    Ok(())
}

pub fn handle_url(path: &str, port: u16) -> Result<()> {
    println!("{}", public_url(path, port)?);
    Ok(())
}

pub fn public_url(path: &str, port: u16) -> Result<String> {
    let rel = normalize_resource_path(path)?;
    Ok(format!(
        "http://{DEFAULT_HOST}:{port}/{}",
        rel.to_string_lossy()
    ))
}

pub fn http_root(config: &Config) -> PathBuf {
    config.shine_dir().join("http")
}

fn ensure_macos_service_support() -> Result<()> {
    if !cfg!(target_os = "macos") {
        bail!("shine serve install is currently supported on macOS only");
    }
    Ok(())
}

fn service_executable(config: &Config) -> Result<PathBuf> {
    if let Some(dest) = &config.self_install_dest {
        return Ok(dest.clone());
    }
    std::env::current_exe().context("failed to resolve current executable path")
}

fn launchd_plist_path(config: &Config) -> PathBuf {
    config
        .home_dir
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LAUNCHD_LABEL}.plist"))
}

fn launchd_plist(executable: &Path, port: u16) -> String {
    let executable = xml_escape(&executable.display().to_string());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LAUNCHD_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{executable}</string>
    <string>serve</string>
    <string>start</string>
    <string>--port</string>
    <string>{port}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>/tmp/{LAUNCHD_LABEL}.out.log</string>
  <key>StandardErrorPath</key>
  <string>/tmp/{LAUNCHD_LABEL}.err.log</string>
</dict>
</plist>
"#
    )
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

async fn launchctl(args: &[&str]) -> Result<()> {
    let output = Command::new("launchctl")
        .args(args)
        .output()
        .await
        .with_context(|| format!("running launchctl {}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = stderr.trim();
        let detail = if detail.is_empty() {
            stdout.trim()
        } else {
            detail
        };
        if detail.is_empty() {
            bail!("launchctl {} failed with {}", args.join(" "), output.status);
        }
        bail!(
            "launchctl {} failed with {}: {detail}",
            args.join(" "),
            output.status
        );
    }
    Ok(())
}

async fn handle_connection(mut stream: TcpStream, root: PathBuf) -> Result<()> {
    let request = match read_request(&mut stream).await {
        Ok(request) => request,
        Err(_) => {
            write_response(
                &mut stream,
                400,
                "Bad Request",
                "text/plain",
                b"",
                false,
                None,
            )
            .await?;
            return Ok(());
        }
    };

    if request.method != "GET" && request.method != "HEAD" {
        write_response(
            &mut stream,
            405,
            "Method Not Allowed",
            "text/plain",
            b"",
            false,
            None,
        )
        .await?;
        return Ok(());
    }

    let rel = match normalize_resource_path(&request.path) {
        Ok(rel) => rel,
        Err(_) => {
            write_response(
                &mut stream,
                404,
                "Not Found",
                "text/plain",
                b"",
                false,
                None,
            )
            .await?;
            return Ok(());
        }
    };
    let root_canon = fs::canonicalize(&root).await?;
    let candidate = root.join(rel);
    let file_canon = match fs::canonicalize(&candidate).await {
        Ok(path) => path,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            write_response(
                &mut stream,
                404,
                "Not Found",
                "text/plain",
                b"",
                false,
                None,
            )
            .await?;
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    if !file_canon.starts_with(&root_canon) || !file_canon.is_file() {
        write_response(
            &mut stream,
            404,
            "Not Found",
            "text/plain",
            b"",
            false,
            None,
        )
        .await?;
        return Ok(());
    }

    let bytes = fs::read(&file_canon).await?;
    let etag = entity_tag(&bytes);
    if request
        .if_none_match
        .as_deref()
        .is_some_and(|value| etag_matches(value, &etag))
    {
        write_response(
            &mut stream,
            304,
            "Not Modified",
            content_type(&file_canon),
            b"",
            true,
            Some(&etag),
        )
        .await?;
        return Ok(());
    }
    write_response(
        &mut stream,
        200,
        "OK",
        content_type(&file_canon),
        &bytes,
        request.method == "HEAD",
        Some(&etag),
    )
    .await?;
    Ok(())
}

struct Request {
    method: String,
    path: String,
    if_none_match: Option<String>,
}

async fn read_request(stream: &mut TcpStream) -> Result<Request> {
    let mut data = Vec::new();
    let mut buf = [0u8; 1024];
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
        if data.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if data.len() > MAX_HEADER_BYTES {
            bail!("request header too large");
        }
    }

    let header = std::str::from_utf8(&data).context("request header must be utf-8")?;
    let line = header.lines().next().context("missing request line")?;
    let mut parts = line.split_whitespace();
    let method = parts.next().context("missing method")?.to_string();
    let target = parts.next().context("missing path")?;
    let path = target
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(target)
        .to_string();
    let if_none_match = header.lines().skip(1).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("if-none-match")
            .then(|| value.trim().to_string())
    });
    Ok(Request {
        method,
        path,
        if_none_match,
    })
}

async fn write_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
    head_only: bool,
    etag: Option<&str>,
) -> Result<()> {
    let cache_headers = cache_headers(etag);
    let headers = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\n{cache_headers}Connection: close\r\n\r\n",
        body.len(),
    );
    stream.write_all(headers.as_bytes()).await?;
    if !head_only {
        stream.write_all(body).await?;
    }
    Ok(())
}

fn cache_headers(etag: Option<&str>) -> String {
    let mut headers =
        "Cache-Control: no-cache, max-age=0, must-revalidate\r\nPragma: no-cache\r\n".to_string();
    if let Some(etag) = etag {
        headers.push_str(&format!("ETag: {etag}\r\n"));
    }
    headers
}

fn entity_tag(bytes: &[u8]) -> String {
    format!("\"sha256-{:x}\"", Sha256::digest(bytes))
}

fn etag_matches(if_none_match: &str, etag: &str) -> bool {
    if_none_match
        .split(',')
        .map(str::trim)
        .any(|candidate| candidate == "*" || candidate.trim_start_matches("W/") == etag)
}

fn normalize_resource_path(path: &str) -> Result<PathBuf> {
    let path = path.trim_start_matches('/');
    if path.is_empty() {
        bail!("resource path must not be empty");
    }
    let decoded = percent_decode(path)?;
    let rel = Path::new(&decoded);
    if rel.is_absolute() {
        bail!("resource path must be relative");
    }
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        bail!("resource path must not contain traversal");
    }
    Ok(rel.to_path_buf())
}

fn percent_decode(input: &str) -> Result<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                bail!("invalid percent encoding");
            }
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3])?;
            out.push(u8::from_str_radix(hex, 16).context("invalid percent encoding")?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).context("decoded path must be utf-8")
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("json") => "application/json",
        Some("sgmodule" | "txt" | "toml") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        cache_headers, entity_tag, etag_matches, launchd_plist, launchd_plist_path,
        normalize_resource_path, public_url,
    };
    use crate::config::Config;
    use std::path::{Path, PathBuf};

    #[test]
    fn public_url_uses_single_local_server_root() {
        assert_eq!(
            public_url("/app/surge/custom-rules.sgmodule", 6174).unwrap(),
            "http://127.0.0.1:6174/app/surge/custom-rules.sgmodule"
        );
    }

    #[test]
    fn resource_paths_reject_traversal() {
        assert!(normalize_resource_path("../secret").is_err());
        assert!(normalize_resource_path("app/../secret").is_err());
        assert!(normalize_resource_path("/app/surge/custom-rules.sgmodule").is_ok());
        assert_eq!(
            normalize_resource_path("app/surge/custom-rules.sgmodule").unwrap(),
            Path::new("app/surge/custom-rules.sgmodule")
        );
    }

    #[test]
    fn resource_responses_require_revalidation_and_offer_an_etag() {
        let etag = entity_tag(b"current rules");
        let headers = cache_headers(Some(&etag));
        assert!(headers.contains("Cache-Control: no-cache, max-age=0, must-revalidate"));
        assert!(headers.contains("Pragma: no-cache"));
        assert!(headers.contains(&format!("ETag: {etag}")));
        assert!(etag_matches(&etag, &etag));
        assert!(etag_matches(&format!("W/{etag}, \"older\""), &etag));
        assert!(!etag_matches("\"older\"", &etag));
    }

    #[test]
    fn launchd_plist_runs_the_single_foreground_server() {
        let plist = launchd_plist(Path::new("/opt/shine & tools/shine"), 6188);
        assert!(plist.contains("<string>top.biulight.shine.http</string>"));
        assert!(plist.contains("<string>/opt/shine &amp; tools/shine</string>"));
        assert!(plist.contains("<string>serve</string>"));
        assert!(plist.contains("<string>start</string>"));
        assert!(plist.contains("<string>--port</string>"));
        assert!(plist.contains("<string>6188</string>"));
    }

    #[test]
    fn launchd_plist_path_lives_under_user_launch_agents() {
        let root = PathBuf::from("/tmp/shine-home");
        let config = Config::new_for_test(&root);
        assert_eq!(
            launchd_plist_path(&config),
            root.join("Library/LaunchAgents/top.biulight.shine.http.plist")
        );
    }
}
