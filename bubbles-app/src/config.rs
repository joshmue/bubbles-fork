use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

pub use bubbles::get_data_dir;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BubbleConfig {
    pub cpus: u32,
    pub ram_mb: u32,
    pub tcp_ports: Vec<String>,
    pub map_host_loopback: bool,
}

impl Default for BubbleConfig {
    fn default() -> Self {
        Self {
            cpus: 4,
            ram_mb: 7000,
            tcp_ports: vec![],
            map_host_loopback: false,
        }
    }
}

fn config_path(vm_name: &str) -> PathBuf {
    get_data_dir().join("vms").join(vm_name).join("config.json")
}

pub fn load_config(vm_name: &str) -> BubbleConfig {
    let path = config_path(vm_name);
    match fs::read_to_string(&path) {
        Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
        Err(_) => BubbleConfig::default(),
    }
}

pub fn save_config(vm_name: &str, config: &BubbleConfig) {
    let path = config_path(vm_name);
    let data = serde_json::to_string_pretty(config).expect("config to serialize");
    fs::write(path, data).expect("config to be written");
}
