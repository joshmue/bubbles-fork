use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

pub fn get_data_dir() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").expect("HOME")).join(".local/share"));
    base.join("bubbles")
}

pub fn is_flatpak() -> bool {
    Path::new("/.flatpak-info").exists()
}

pub fn vm_dir(bubble: &str) -> PathBuf {
    get_data_dir().join("vms").join(bubble)
}

pub fn vsock_path(bubble: &str) -> PathBuf {
    vm_dir(bubble).join("vsock")
}

pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

fn request_line(method: &str, path: &str) -> String {
    format!(
        "{} {} HTTP/1.0\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n",
        method, path
    )
}

fn parse_response(raw: &[u8]) -> HttpResponse {
    let status = raw
        .split(|b| *b == b'\n')
        .next()
        .and_then(|line| String::from_utf8_lossy(line).split_whitespace().nth(1).and_then(|s| s.parse().ok()))
        .unwrap_or(0);
    let body = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| String::from_utf8_lossy(&raw[i + 4..]).into_owned())
        .unwrap_or_default();
    HttpResponse { status, body }
}

pub async fn unix_request(socket: &Path, method: &str, path: &str) -> std::io::Result<HttpResponse> {
    let mut stream = tokio::net::UnixStream::connect(socket).await?;
    stream.write_all(request_line(method, path).as_bytes()).await?;
    let mut buf = Vec::new();
    stream.take(MAX_RESPONSE_BYTES as u64).read_to_end(&mut buf).await?;
    Ok(parse_response(&buf))
}

// bubbles-launch does one request and exits, so it needs no async runtime.
pub fn unix_request_blocking(socket: &Path, method: &str, path: &str) -> std::io::Result<HttpResponse> {
    let mut stream = std::os::unix::net::UnixStream::connect(socket)?;
    stream.write_all(request_line(method, path).as_bytes())?;
    let mut buf = Vec::new();
    Read::take(stream, MAX_RESPONSE_BYTES as u64).read_to_end(&mut buf)?;
    Ok(parse_response(&buf))
}

pub fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

pub fn start_app_path(app_id: &str) -> String {
    format!("/start-desktop-app?app={}", percent_encode(app_id))
}
