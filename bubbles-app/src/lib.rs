use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use http_body_util::{BodyExt, Empty, Limited};
use hyper::body::Bytes;
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;

pub const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

pub const APP_ID: &str = "de.gonicus.Bubbles";

// The interface exported launchers call on the running app. Shared so that
// bubbles-launch and the app cannot drift apart on a name.
pub const LAUNCHER_INTERFACE: &str = "de.gonicus.Bubbles.Launcher";
pub const LAUNCHER_OBJECT_PATH: &str = "/de/gonicus/Bubbles/Launcher";
pub const LAUNCHER_NOT_RUNNING_ERROR: &str = "de.gonicus.Bubbles.Launcher.Error.NotRunning";
pub const LAUNCHER_NOT_INSTALLED_ERROR: &str = "de.gonicus.Bubbles.Launcher.Error.NotInstalled";
pub const LAUNCHER_FAILED_ERROR: &str = "de.gonicus.Bubbles.Launcher.Error.Failed";

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

pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

/// One request over a fresh connection: the agent speaks HTTP/1 on a loopback
/// address passt forwards into the bubble, and nothing here is hot enough to
/// want a pool.
pub async fn agent_request(
    addr: SocketAddr,
    method: Method,
    path: &str,
) -> Result<HttpResponse, String> {
    let stream = tokio::net::TcpStream::connect(addr)
        .await
        .map_err(|e| format!("no connection to the bubble: {e}"))?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(|e| format!("no HTTP connection to the bubble: {e}"))?;
    // Nothing moves over the socket unless the connection future is polled,
    // and it ends by itself once the body has been read.
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let request = Request::builder()
        .method(method)
        .uri(path)
        .header(hyper::header::HOST, "localhost")
        .body(Empty::<Bytes>::new())
        .map_err(|e| format!("malformed request: {e}"))?;
    let response = sender
        .send_request(request)
        .await
        .map_err(|e| format!("the bubble did not answer: {e}"))?;

    let status = response.status().as_u16();
    // The bubble fills this list itself, so the size is not ours to trust.
    let body = Limited::new(response.into_body(), MAX_RESPONSE_BYTES)
        .collect()
        .await
        .map_err(|e| format!("could not read the answer: {e}"))?
        .to_bytes();
    Ok(HttpResponse {
        status,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
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
