use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine;
use serde::Serialize;

const MAX_ICON_BYTES: u64 = 64 * 1024;
const MAX_APPS: usize = 200;

const ICON_SIZES: [&str; 5] = ["scalable", "256x256", "128x128", "64x64", "48x48"];

#[derive(Serialize)]
pub struct Icon {
    pub format: &'static str,
    pub data: String,
}

#[derive(Serialize)]
pub struct App {
    pub id: String,
    pub name: String,
    pub comment: String,
    pub wm_class: String,
    pub icon: Option<Icon>,
}

#[derive(Serialize)]
pub struct AppList {
    pub apps: Vec<App>,
}

fn home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/home/user"))
}

// A system service (User=user) never sees the login shell's XDG_DATA_DIRS.
fn data_roots() -> Vec<PathBuf> {
    let home = home();
    vec![
        home.join(".local/share"),
        home.join(".nix-profile/share"),
        home.join(".local/share/flatpak/exports/share"),
        PathBuf::from("/var/lib/flatpak/exports/share"),
        PathBuf::from("/usr/local/share"),
        PathBuf::from("/usr/share"),
    ]
}

type Entry = BTreeMap<String, String>;

fn parse_entry(path: &Path) -> Option<Entry> {
    let content = fs::read_to_string(path).ok()?;
    let mut entry = Entry::new();
    let mut in_group = false;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
            continue;
        }
        if !in_group {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.contains('[') {
            continue;
        }
        entry.insert(key.to_string(), value.trim().to_string());
    }
    (!entry.is_empty()).then_some(entry)
}

fn is_true(entry: &Entry, key: &str) -> bool {
    entry.get(key).map(|v| v == "true").unwrap_or(false)
}

pub fn scan() -> BTreeMap<String, PathBuf> {
    let mut apps: BTreeMap<String, PathBuf> = BTreeMap::new();
    for root in data_roots() {
        let Ok(read_dir) = fs::read_dir(root.join("applications")) else {
            continue;
        };
        for entry in read_dir.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(OsStr::to_str) else {
                continue;
            };
            if name.ends_with(".desktop") && !apps.contains_key(name) {
                apps.insert(name.to_string(), path);
            }
        }
    }
    apps
}

pub fn resolve(id: &str) -> Option<PathBuf> {
    scan().remove(id)
}

fn read_icon(path: &Path) -> Option<(&'static str, Vec<u8>)> {
    let format = match path.extension().and_then(OsStr::to_str)? {
        "svg" => "svg",
        "png" => "png",
        _ => return None,
    };
    let size = fs::metadata(path).ok()?.len();
    if size == 0 || size > MAX_ICON_BYTES {
        return None;
    }
    Some((format, fs::read(path).ok()?))
}

fn find_icon(name: &str) -> Option<(&'static str, Vec<u8>)> {
    if name.is_empty() {
        return None;
    }
    if name.starts_with('/') {
        return read_icon(Path::new(name));
    }
    let name = name.strip_suffix(".png").or(name.strip_suffix(".svg")).unwrap_or(name);
    for root in data_roots() {
        for size in ICON_SIZES {
            for extension in ["svg", "png"] {
                let path = root
                    .join("icons/hicolor")
                    .join(size)
                    .join("apps")
                    .join(format!("{name}.{extension}"));
                if let Some(icon) = read_icon(&path) {
                    return Some(icon);
                }
            }
        }
        // Where applications shipped icons before the icon theme spec.
        for extension in ["png", "svg"] {
            let path = root.join("pixmaps").join(format!("{name}.{extension}"));
            if let Some(icon) = read_icon(&path) {
                return Some(icon);
            }
        }
    }
    None
}

pub fn list_apps() -> AppList {
    let mut apps = Vec::new();

    for (id, path) in scan() {
        if apps.len() >= MAX_APPS {
            break;
        }
        let Some(entry) = parse_entry(&path) else {
            continue;
        };
        if entry.get("Type").map(String::as_str) != Some("Application") {
            continue;
        }
        if is_true(&entry, "NoDisplay") || is_true(&entry, "Hidden") {
            continue;
        }
        let Some(name) = entry.get("Name").filter(|n| !n.is_empty()) else {
            continue;
        };

        let icon = entry
            .get("Icon")
            .and_then(|icon| find_icon(icon))
            .map(|(format, bytes)| Icon {
                format,
                data: base64::engine::general_purpose::STANDARD.encode(bytes),
            });

        apps.push(App {
            id,
            name: name.clone(),
            comment: entry.get("Comment").cloned().unwrap_or_default(),
            wm_class: entry.get("StartupWMClass").cloned().unwrap_or_default(),
            icon,
        });
    }

    AppList { apps }
}
