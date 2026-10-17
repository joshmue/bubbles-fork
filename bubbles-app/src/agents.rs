use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use bubbles::Agent;

// Where a bubble's agent can be reached, for as long as it runs. The address and
// token are made at start time rather than derived from the bubble, so this is the
// only place that knows them; bubbles-launch asks over D-Bus instead of guessing.
fn registry() -> &'static Mutex<HashMap<String, Agent>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Agent>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn set(bubble: &str, agent: Agent) {
    registry().lock().unwrap().insert(bubble.to_string(), agent);
}

pub fn clear(bubble: &str) {
    registry().lock().unwrap().remove(bubble);
}

/// `None` while the bubble is not running.
pub fn get(bubble: &str) -> Option<Agent> {
    registry().lock().unwrap().get(bubble).cloned()
}
