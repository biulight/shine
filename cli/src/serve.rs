use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;

use crate::config::Config;

const DEFAULT_HOST: &str = "127.0.0.1";
const LAUNCHD_LABEL: &str = "top.biulight.shine.http";
const SYSTEMD_UNIT: &str = "shine-http.service";
const WINDOWS_TASK: &str = "Shine HTTP Server";
const MAX_HEADER_BYTES: usize = 8192;
/// Bounds how long a single connection may take end-to-end (read request + write
/// response), so a slow-loris style local client can't hold a task open forever.
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);

// --- Trust boundary ---------------------------------------------------------------------
//
// This server binds loopback-only (`DEFAULT_HOST` = 127.0.0.1) and is never reachable from
// the network, but it has NO authentication of its own: any local OS user account on a
// shared/multi-user machine can connect to `127.0.0.1:<port>` and read any file under
// `http_root()`, bypassing the normal filesystem permissions that would otherwise keep
// other accounts out of this user's home directory. Preset authors must never route secrets
// or otherwise sensitive content through a `dest` that resolves under `~/.shine/http`.
// See docs/kb/architecture/invariants.md § Local HTTP server.

pub async fn handle_install(config: &Config, port: u16) -> Result<()> {
    let root = http_root(config);
    fs::create_dir_all(&root)
        .await
        .with_context(|| format!("creating {}", root.display()))?;

    match std::env::consts::OS {
        "macos" => install_launchd(config, port).await?,
        "linux" => install_systemd(config, port).await?,
        "windows" => install_windows_task(config, port).await?,
        os => bail!("shine serve install is not supported on {os}"),
    }

    println!("Serving {}", root.display());
    println!("URL base: http://{DEFAULT_HOST}:{port}/");
    Ok(())
}

async fn install_launchd(config: &Config, port: u16) -> Result<()> {
    let plist_path = launchd_plist_path(config);
    let parent = plist_path
        .parent()
        .context("launchd plist path must have a parent directory")?;
    fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;

    let log_dir = launchd_log_dir(config);
    fs::create_dir_all(&log_dir)
        .await
        .with_context(|| format!("creating {}", log_dir.display()))?;

    let executable = service_executable(config)?;
    let plist = launchd_plist(&executable, config.shine_dir(), port, &log_dir);
    fs::write(&plist_path, plist)
        .await
        .with_context(|| format!("writing {}", plist_path.display()))?;

    let plist_arg = plist_path.to_string_lossy().to_string();
    let _ = launchctl(&["unload", "-w", &plist_arg]).await;
    launchctl(&["load", "-w", &plist_arg]).await?;

    println!("Installed {LAUNCHD_LABEL}");
    Ok(())
}

async fn install_systemd(config: &Config, port: u16) -> Result<()> {
    let unit_path = systemd_unit_path(config);
    let parent = unit_path
        .parent()
        .context("systemd unit path must have a parent directory")?;
    fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;

    let executable = service_executable(config)?;
    let unit = systemd_unit(&executable, config.shine_dir(), port)?;
    fs::write(&unit_path, unit)
        .await
        .with_context(|| format!("writing {}", unit_path.display()))?;

    systemctl(&["daemon-reload"]).await?;
    systemctl(&["enable", SYSTEMD_UNIT]).await?;
    systemctl(&["restart", SYSTEMD_UNIT]).await?;
    println!("Installed {SYSTEMD_UNIT}");
    Ok(())
}

async fn install_windows_task(config: &Config, port: u16) -> Result<()> {
    let executable = service_executable(config)?;
    let task_run = windows_task_command(&executable, config.shine_dir(), port)?;
    if task_run.encode_utf16().count() > 261 {
        bail!("Windows scheduled task command exceeds the 261-character schtasks limit");
    }
    let run_as = windows_current_user()?;
    let _ = schtasks(&[
        OsStr::new("/End"),
        OsStr::new("/TN"),
        OsStr::new(WINDOWS_TASK),
    ])
    .await;
    schtasks(&[
        OsStr::new("/Create"),
        OsStr::new("/SC"),
        OsStr::new("ONLOGON"),
        OsStr::new("/TN"),
        OsStr::new(WINDOWS_TASK),
        OsStr::new("/TR"),
        OsStr::new(&task_run),
        OsStr::new("/RL"),
        OsStr::new("LIMITED"),
        OsStr::new("/RU"),
        &run_as,
        OsStr::new("/NP"),
        OsStr::new("/F"),
    ])
    .await?;
    schtasks(&[
        OsStr::new("/Run"),
        OsStr::new("/TN"),
        OsStr::new(WINDOWS_TASK),
    ])
    .await?;
    println!("Installed {WINDOWS_TASK}");
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
            match tokio::time::timeout(CONNECTION_TIMEOUT, handle_connection(stream, root)).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => eprintln!("shine serve: connection error: {e:#}"),
                Err(_) => {
                    eprintln!("shine serve: connection timed out after {CONNECTION_TIMEOUT:?}")
                }
            }
        });
    }
}

pub async fn handle_status(config: &Config) -> Result<()> {
    match std::env::consts::OS {
        "macos" => status_launchd(config).await,
        "linux" => status_systemd(config).await,
        "windows" => status_windows_task().await,
        os => bail!("shine serve status is not supported on {os}"),
    }
}

pub async fn handle_uninstall(config: &Config) -> Result<()> {
    match std::env::consts::OS {
        "macos" => uninstall_launchd(config).await,
        "linux" => uninstall_systemd(config).await,
        "windows" => uninstall_windows_task().await,
        os => bail!("shine serve uninstall is not supported on {os}"),
    }
}

async fn status_launchd(config: &Config) -> Result<()> {
    let plist_path = launchd_plist_path(config);
    if !plist_path.exists() {
        println!("Not installed");
        return Ok(());
    }
    match launchctl(&["list", LAUNCHD_LABEL]).await {
        Ok(()) => println!("Installed and running"),
        Err(_) => println!("Installed but not running"),
    }
    println!("Plist: {}", plist_path.display());
    Ok(())
}

async fn status_systemd(config: &Config) -> Result<()> {
    let unit_path = systemd_unit_path(config);
    if !unit_path.exists() {
        println!("Not installed");
        return Ok(());
    }
    match systemctl(&["is-active", "--quiet", SYSTEMD_UNIT]).await {
        Ok(()) => println!("Installed and running"),
        Err(_) => println!("Installed but not running"),
    }
    println!("Unit: {}", unit_path.display());
    Ok(())
}

async fn status_windows_task() -> Result<()> {
    match schtasks(&[
        OsStr::new("/Query"),
        OsStr::new("/TN"),
        OsStr::new(WINDOWS_TASK),
    ])
    .await
    {
        Ok(()) => println!("Installed"),
        Err(_) => println!("Not installed"),
    }
    Ok(())
}

async fn uninstall_launchd(config: &Config) -> Result<()> {
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

async fn uninstall_systemd(config: &Config) -> Result<()> {
    let unit_path = systemd_unit_path(config);
    if !unit_path.exists() {
        println!("Not installed");
        return Ok(());
    }
    systemctl(&["disable", "--now", SYSTEMD_UNIT]).await?;
    fs::remove_file(&unit_path)
        .await
        .with_context(|| format!("removing {}", unit_path.display()))?;
    systemctl(&["daemon-reload"]).await?;
    println!("Uninstalled {SYSTEMD_UNIT}");
    Ok(())
}

async fn uninstall_windows_task() -> Result<()> {
    let query = schtasks(&[
        OsStr::new("/Query"),
        OsStr::new("/TN"),
        OsStr::new(WINDOWS_TASK),
    ])
    .await;
    if query.is_err() {
        println!("Not installed");
        return Ok(());
    }
    let _ = schtasks(&[
        OsStr::new("/End"),
        OsStr::new("/TN"),
        OsStr::new(WINDOWS_TASK),
    ])
    .await;
    schtasks(&[
        OsStr::new("/Delete"),
        OsStr::new("/TN"),
        OsStr::new(WINDOWS_TASK),
        OsStr::new("/F"),
    ])
    .await?;
    println!("Uninstalled {WINDOWS_TASK}");
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

/// Directory for the launchd service's stdout/stderr logs. Deliberately kept out of
/// `http_root()` (`shine_dir/http`) so log contents are never servable over HTTP, and kept
/// under the user's own `shine_dir` (not a shared path like `/tmp`) so two OS user accounts
/// running `shine serve install` never collide on the same log file.
fn launchd_log_dir(config: &Config) -> PathBuf {
    config.shine_dir().join("run").join("http")
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

fn launchd_plist(executable: &Path, shine_dir: &Path, port: u16, log_dir: &Path) -> String {
    let executable = xml_escape(&executable.display().to_string());
    let shine_dir = xml_escape(&shine_dir.display().to_string());
    let out_log = xml_escape(&log_dir.join("serve.out.log").display().to_string());
    let err_log = xml_escape(&log_dir.join("serve.err.log").display().to_string());
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
    <string>--config-dir</string>
    <string>{shine_dir}</string>
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
  <string>{out_log}</string>
  <key>StandardErrorPath</key>
  <string>{err_log}</string>
</dict>
</plist>
"#
    )
}

fn systemd_unit_path(config: &Config) -> PathBuf {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| config.home_dir.join(".config"));
    config_home.join("systemd").join("user").join(SYSTEMD_UNIT)
}

fn systemd_unit(executable: &Path, shine_dir: &Path, port: u16) -> Result<String> {
    let executable = executable
        .to_str()
        .context("shine executable path is not valid UTF-8")?;
    let shine_dir = shine_dir
        .to_str()
        .context("shine config directory is not valid UTF-8")?;
    Ok(format!(
        "[Unit]\nDescription=Shine local HTTP server\n\n[Service]\nType=simple\nExecStart={} --config-dir {} serve start --port {port}\nRestart=on-failure\nRestartSec=2s\n\n[Install]\nWantedBy=default.target\n",
        systemd_quote(executable),
        systemd_quote(shine_dir),
    ))
}

fn systemd_quote(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
        .replace('%', "%%")
        .replace('$', "$$");
    format!("\"{escaped}\"")
}

fn windows_task_command(executable: &Path, shine_dir: &Path, port: u16) -> Result<String> {
    let executable = executable
        .to_str()
        .context("shine executable path is not valid Unicode")?;
    let shine_dir = shine_dir
        .to_str()
        .context("shine config directory is not valid Unicode")?;
    Ok([
        executable.to_string(),
        "--config-dir".to_string(),
        shine_dir.to_string(),
        "serve".to_string(),
        "start".to_string(),
        "--port".to_string(),
        port.to_string(),
    ]
    .iter()
    .map(|arg| windows_quote_arg(arg))
    .collect::<Vec<_>>()
    .join(" "))
}

fn windows_current_user() -> Result<OsString> {
    let username = std::env::var_os("USERNAME")
        .filter(|value| !value.is_empty())
        .context("USERNAME is not set; cannot register the current-user scheduled task")?;
    if let Some(domain) = std::env::var_os("USERDOMAIN").filter(|value| !value.is_empty()) {
        let mut qualified = domain;
        qualified.push("\\");
        qualified.push(username);
        Ok(qualified)
    } else {
        Ok(username)
    }
}

// Quote one argv element according to the CommandLineToArgvW rules used by Rust's
// Windows process startup. Backslashes need doubling only when they precede a quote
// or the closing quote.
fn windows_quote_arg(value: &str) -> String {
    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for ch in value.chars() {
        if ch == '\\' {
            backslashes += 1;
        } else if ch == '"' {
            quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
            quoted.push('"');
            backslashes = 0;
        } else {
            quoted.push_str(&"\\".repeat(backslashes));
            backslashes = 0;
            quoted.push(ch);
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
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
    run_command("launchctl", args.iter().map(OsStr::new)).await
}

async fn systemctl(args: &[&str]) -> Result<()> {
    let args = std::iter::once(OsStr::new("--user")).chain(args.iter().map(OsStr::new));
    run_command("systemctl", args).await
}

async fn schtasks(args: &[&OsStr]) -> Result<()> {
    run_command("schtasks.exe", args.iter().copied()).await
}

async fn run_command<'a>(program: &str, args: impl IntoIterator<Item = &'a OsStr>) -> Result<()> {
    let args: Vec<OsString> = args.into_iter().map(OsStr::to_os_string).collect();
    let output = Command::new(program)
        .args(&args)
        .output()
        .await
        .with_context(|| format!("running {program}"))?;
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
            bail!("{program} failed with {}", output.status);
        }
        bail!("{program} failed with {}: {detail}", output.status);
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
        cache_headers, entity_tag, etag_matches, launchd_log_dir, launchd_plist,
        launchd_plist_path, normalize_resource_path, public_url, systemd_unit, systemd_unit_path,
        windows_quote_arg, windows_task_command,
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
        let log_dir = Path::new("/Users/tester/.shine/run/http");
        let plist = launchd_plist(
            Path::new("/opt/shine & tools/shine"),
            Path::new("/Users/tester/.shine & tools"),
            6188,
            log_dir,
        );
        assert!(plist.contains("<string>top.biulight.shine.http</string>"));
        assert!(plist.contains("<string>/opt/shine &amp; tools/shine</string>"));
        assert!(plist.contains("<string>--config-dir</string>"));
        assert!(plist.contains("<string>/Users/tester/.shine &amp; tools</string>"));
        assert!(plist.contains("<string>serve</string>"));
        assert!(plist.contains("<string>start</string>"));
        assert!(plist.contains("<string>--port</string>"));
        assert!(plist.contains("<string>6188</string>"));
    }

    #[cfg(unix)]
    #[test]
    fn launchd_plist_logs_are_scoped_under_the_user_shine_dir_not_shared_tmp() {
        let log_dir = Path::new("/Users/tester/.shine/run/http");
        let plist = launchd_plist(
            Path::new("/opt/shine/shine"),
            Path::new("/Users/tester/.shine"),
            6188,
            log_dir,
        );
        assert!(plist.contains("<string>/Users/tester/.shine/run/http/serve.out.log</string>"));
        assert!(plist.contains("<string>/Users/tester/.shine/run/http/serve.err.log</string>"));
        assert!(!plist.contains("/tmp/"));
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

    #[test]
    fn launchd_log_dir_lives_under_shine_dir_run_not_shared_tmp() {
        let root = PathBuf::from("/tmp/shine-home");
        let config = Config::new_for_test(&root);
        assert_eq!(launchd_log_dir(&config), root.join("run").join("http"));
    }

    #[test]
    fn systemd_unit_runs_and_restarts_the_user_server() {
        let unit = systemd_unit(
            Path::new("/opt/shine % tools/shine"),
            Path::new("/home/tester/.shine $ state"),
            6188,
        )
        .unwrap();
        assert!(unit.contains("Type=simple"));
        assert!(unit.contains(
            "ExecStart=\"/opt/shine %% tools/shine\" --config-dir \"/home/tester/.shine $$ state\" serve start --port 6188"
        ));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn systemd_unit_path_uses_the_standard_user_unit_suffix() {
        let root = PathBuf::from("/tmp/shine-home");
        let config = Config::new_for_test(&root);
        assert!(systemd_unit_path(&config).ends_with("systemd/user/shine-http.service"));
    }

    #[test]
    fn windows_task_command_preserves_spaces_quotes_and_trailing_backslashes() {
        let command = windows_task_command(
            Path::new(r#"C:\Program Files\Shine\shine.exe"#),
            Path::new(r#"C:\Users\Tester\shine state\"#),
            6199,
        )
        .unwrap();
        assert!(command.starts_with(r#""C:\Program Files\Shine\shine.exe" "--config-dir""#));
        assert!(command.contains(r#""C:\Users\Tester\shine state\\""#));
        assert!(command.ends_with(r#""serve" "start" "--port" "6199""#));
        assert_eq!(
            windows_quote_arg(r#"C:\path with space\"#),
            r#""C:\path with space\\""#
        );
        assert_eq!(windows_quote_arg(r#"a"b"#), r#""a\"b""#);
    }
}
