use anyhow::{Context, Result, bail};
use std::path::{Component, Path, PathBuf};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::config::Config;

const DEFAULT_HOST: &str = "127.0.0.1";
const MAX_HEADER_BYTES: usize = 8192;

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

async fn handle_connection(mut stream: TcpStream, root: PathBuf) -> Result<()> {
    let request = match read_request(&mut stream).await {
        Ok(request) => request,
        Err(_) => {
            write_response(&mut stream, 400, "Bad Request", "text/plain", b"", false).await?;
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
        )
        .await?;
        return Ok(());
    }

    let rel = match normalize_resource_path(&request.path) {
        Ok(rel) => rel,
        Err(_) => {
            write_response(&mut stream, 404, "Not Found", "text/plain", b"", false).await?;
            return Ok(());
        }
    };
    let root_canon = fs::canonicalize(&root).await?;
    let candidate = root.join(rel);
    let file_canon = match fs::canonicalize(&candidate).await {
        Ok(path) => path,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            write_response(&mut stream, 404, "Not Found", "text/plain", b"", false).await?;
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    if !file_canon.starts_with(&root_canon) || !file_canon.is_file() {
        write_response(&mut stream, 404, "Not Found", "text/plain", b"", false).await?;
        return Ok(());
    }

    let bytes = fs::read(&file_canon).await?;
    write_response(
        &mut stream,
        200,
        "OK",
        content_type(&file_canon),
        &bytes,
        request.method == "HEAD",
    )
    .await?;
    Ok(())
}

struct Request {
    method: String,
    path: String,
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
    Ok(Request { method, path })
}

async fn write_response(
    stream: &mut TcpStream,
    status: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
    head_only: bool,
) -> Result<()> {
    let headers = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(headers.as_bytes()).await?;
    if !head_only {
        stream.write_all(body).await?;
    }
    Ok(())
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
    use super::{normalize_resource_path, public_url};
    use std::path::Path;

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
}
