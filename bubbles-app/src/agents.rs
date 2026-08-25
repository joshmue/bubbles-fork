use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Mutex, OnceLock};

// Where a bubble's agent can be reached, for as long as it runs. The address is
// claimed at start time rather than derived from the bubble, so this is the
// only place that knows it; bubbles-launch asks over D-Bus instead of guessing.
fn registry() -> &'static Mutex<HashMap<String, SocketAddr>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, SocketAddr>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn set(bubble: &str, addr: SocketAddr) {
    registry().lock().unwrap().insert(bubble.to_string(), addr);
}

pub fn clear(bubble: &str) {
    registry().lock().unwrap().remove(bubble);
}

/// `None` while the bubble is not running.
pub fn get(bubble: &str) -> Option<SocketAddr> {
    registry().lock().unwrap().get(bubble).copied()
}
