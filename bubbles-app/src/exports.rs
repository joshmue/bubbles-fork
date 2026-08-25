use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use gtk::glib;

use bubbles::{is_flatpak, vm_dir};

pub const APP_ID: &str = "de.gonicus.Bubbles";

/// `desktop_file_id -> the name the launcher carries in the host menu`
pub type Exports = BTreeMap<String, String>;

fn exports_path(bubble: &str) -> PathBuf {
    vm_dir(bubble).join("exports.json")
}

pub fn load(bubble: &str) -> Exports {
    fs::read_to_string(exports_path(bubble))
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default()
}

fn save(bubble: &str, exports: &Exports) {
    if let Ok(data) = serde_json::to_string_pretty(exports) {
        let _ = fs::write(exports_path(bubble), data);
    }
}

// The portal requires flatpak_is_valid_name() below our app id: dot-separated
// `[A-Za-z_][A-Za-z0-9_-]*`, hence the `b_` prefix and the hash.
pub fn desktop_file_id(bubble: &str, guest_app_id: &str) -> String {
    let sanitised: String = bubble
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    let hash = glib::compute_checksum_for_string(glib::ChecksumType::Sha256, guest_app_id)
        .map(|hash| hash.to_string())
        .unwrap_or_default();
    format!("{APP_ID}.b_{sanitised}_{:.8}.desktop", hash)
}

pub fn insert(bubble: &str, guest_app_id: &str, name: &str) {
    let mut exports = load(bubble);
    exports.insert(desktop_file_id(bubble, guest_app_id), name.to_string());
    save(bubble, &exports);
}

pub fn remove(bubble: &str, desktop_file_id: &str) {
    let mut exports = load(bubble);
    exports.remove(desktop_file_id);
    save(bubble, &exports);
}

// Built from $HOME: inside the sandbox XDG_DATA_HOME points at ~/.var/app/...
fn portal_applications_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".local/share/xdg-desktop-portal/applications")
}

/// Drop entries whose launcher the user removed from the host menu themselves.
pub fn reconcile(bubble: &str) {
    let dir = portal_applications_dir();
    if !dir.is_dir() {
        return;
    }
    let exports = load(bubble);
    let kept: Exports = exports
        .iter()
        .filter(|(id, _)| dir.join(id).exists())
        .map(|(id, name)| (id.clone(), name.clone()))
        .collect();
    if kept.len() != exports.len() {
        save(bubble, &kept);
    }
}

// Inside Flatpak the portal rewrites this into `flatpak run --command=...`;
// outside it we count as a host app and the line is taken as written.
fn exec_line(bubble: &str, guest_app_id: &str) -> String {
    let launcher = if is_flatpak() {
        PathBuf::from("bubbles-launch")
    } else {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("bubbles-launch")))
            .unwrap_or_else(|| PathBuf::from("bubbles-launch"))
    };
    format!("{} {} {}", launcher.display(), bubble, guest_app_id)
}

// KeyFile, not concatenation: a newline in a guest field would inject keys.
// Name and Icon are left out; the portal overwrites them from PrepareInstall.
pub fn desktop_entry(bubble: &str, guest_app_id: &str, comment: &str, wm_class: &str) -> String {
    let key_file = glib::KeyFile::new();
    key_file.set_string("Desktop Entry", "Type", "Application");
    key_file.set_string("Desktop Entry", "Exec", &exec_line(bubble, guest_app_id));
    if !comment.is_empty() {
        key_file.set_string("Desktop Entry", "Comment", comment);
    }
    if !wm_class.is_empty() {
        key_file.set_string("Desktop Entry", "StartupWMClass", wm_class);
    }
    key_file.set_string("Desktop Entry", "X-Bubbles-Bubble", bubble);
    key_file.to_data().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `flatpak_is_valid_name()`: dot-separated `[A-Za-z_][A-Za-z0-9_-]*`.
    fn is_valid_flatpak_name(name: &str) -> bool {
        !name.is_empty()
            && name.split('.').count() >= 2
            && name.split('.').all(|element| {
                let mut chars = element.chars();
                chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                    && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            })
    }

    #[test]
    fn the_portal_accepts_what_we_hand_it() {
        for bubble in ["work", "9lives", "with-dash", "aaa.bbb", "ünïcode"] {
            let id = desktop_file_id(bubble, "chromium.desktop");
            let name = id.strip_suffix(".desktop").expect("a .desktop suffix");
            assert!(name.starts_with(APP_ID), "{id} is not below the app id");
            assert!(is_valid_flatpak_name(name), "{id} would be rejected");
        }

        let entry = desktop_entry(
            "work",
            "chromium.desktop",
            "harmless\nExec=rm -rf ~",
            "wm\nTerminal=true",
        );
        let key_file = glib::KeyFile::new();
        key_file
            .load_from_data(&entry, glib::KeyFileFlags::NONE)
            .expect("the entry we produced to parse");
        assert_eq!(key_file.groups().len(), 1);
        assert!(key_file.string("Desktop Entry", "Terminal").is_err());
        assert_eq!(
            key_file.string("Desktop Entry", "Exec").unwrap(),
            exec_line("work", "chromium.desktop")
        );
    }
}
