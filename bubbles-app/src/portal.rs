use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;

use gtk::gio::prelude::*;
use gtk::gio::{self, DBusCallFlags, DBusProxy, DBusProxyFlags, DBusSignalFlags};
use gtk::glib::{self, Variant, VariantDict};

const PORTAL_BUS: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const LAUNCHER_INTERFACE: &str = "org.freedesktop.portal.DynamicLauncher";
const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";

// The portal itself rejects anything over 4 MiB, or png/jpeg above 512x512.
pub const MAX_ICON_BYTES: usize = 1024 * 1024;

static AVAILABLE: OnceLock<bool> = OnceLock::new();

fn proxy() -> Result<DBusProxy, String> {
    DBusProxy::for_bus_sync(
        gio::BusType::Session,
        DBusProxyFlags::NONE,
        None,
        PORTAL_BUS,
        PORTAL_PATH,
        LAUNCHER_INTERFACE,
        gio::Cancellable::NONE,
    )
    .map_err(|e| format!("no connection to the desktop portal: {e}"))
}

// xdg-desktop-portal-wlr and -hyprland ship no DynamicLauncher backend, and
// the frontend cannot answer PrepareInstall without one.
pub fn available() -> bool {
    *AVAILABLE.get_or_init(|| {
        proxy()
            .ok()
            .and_then(|proxy| proxy.cached_property("version"))
            .is_some()
    })
}

fn unique_token(prefix: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{prefix}{}_{n}", std::process::id())
}

fn request_path(connection: &gio::DBusConnection, token: &str) -> Result<String, String> {
    let unique = connection
        .unique_name()
        .ok_or_else(|| "the session bus gave us no unique name".to_string())?;
    let sender = unique.trim_start_matches(':').replace('.', "_");
    Ok(format!("{PORTAL_PATH}/request/{sender}/{token}"))
}

/// `None` means the user cancelled the portal's confirmation dialog.
pub async fn prepare_install(
    parent_window: &str,
    name: &str,
    icon: Option<Vec<u8>>,
) -> Result<Option<String>, String> {
    let proxy = proxy()?;
    let connection = proxy.connection();
    let token = unique_token("bubbles_");
    let path = request_path(&connection, &token)?;

    // Subscribe first: the portal may answer before PrepareInstall returns.
    let (sender, receiver) = tokio::sync::oneshot::channel::<Variant>();
    let sender = Rc::new(RefCell::new(Some(sender)));
    let subscription = connection.subscribe_to_signal(
        Some(PORTAL_BUS),
        Some(REQUEST_INTERFACE),
        Some("Response"),
        Some(&path),
        None,
        DBusSignalFlags::NONE,
        move |signal| {
            if let Some(sender) = sender.borrow_mut().take() {
                let _ = sender.send(signal.parameters.clone());
            }
        },
    );

    let options = VariantDict::new(None);
    options.insert("handle_token", token.as_str());
    options.insert("editable_name", true);
    options.insert("editable_icon", true);

    let icon_variant = icon
        .map(|bytes| gio::BytesIcon::new(&glib::Bytes::from_owned(bytes)))
        .and_then(|icon| icon.serialize())
        .unwrap_or_else(|| {
            gio::ThemedIcon::new("application-x-executable")
                .serialize()
                .expect("a themed icon to serialise")
        });

    let arguments = Variant::tuple_from_iter([
        parent_window.to_variant(),
        name.to_variant(),
        icon_variant,
        options.end(),
    ]);

    let call = proxy
        .call_future("PrepareInstall", Some(&arguments), DBusCallFlags::NONE, -1)
        .await;
    if let Err(error) = call {
        return Err(format!("the portal refused to prepare the launcher: {error}"));
    }

    let response = receiver.await;
    drop(subscription);
    let response = response.map_err(|_| "the portal closed the request without answering".to_string())?;

    let code: u32 = response.child_value(0).get().unwrap_or(2);
    if code != 0 {
        return Ok(None);
    }
    let results = VariantDict::new(Some(&response.child_value(1)));
    results
        .lookup::<String>("token")
        .ok()
        .flatten()
        .map(Some)
        .ok_or_else(|| "the portal returned no install token".to_string())
}

pub async fn install(token: &str, desktop_file_id: &str, desktop_entry: &str) -> Result<(), String> {
    let proxy = proxy()?;
    let arguments = Variant::tuple_from_iter([
        token.to_variant(),
        desktop_file_id.to_variant(),
        desktop_entry.to_variant(),
        VariantDict::new(None).end(),
    ]);
    proxy
        .call_future("Install", Some(&arguments), DBusCallFlags::NONE, -1)
        .await
        .map(|_| ())
        .map_err(|e| format!("the portal could not install the launcher: {e}"))
}

pub async fn uninstall(desktop_file_id: &str) -> Result<(), String> {
    let proxy = proxy()?;
    let arguments = Variant::tuple_from_iter([
        desktop_file_id.to_variant(),
        VariantDict::new(None).end(),
    ]);
    proxy
        .call_future("Uninstall", Some(&arguments), DBusCallFlags::NONE, -1)
        .await
        .map(|_| ())
        .map_err(|e| format!("the portal could not remove the launcher: {e}"))
}

// A real parent handle would need gdk4-wayland vendored into cargo-sources.json.
pub const NO_PARENT_WINDOW: &str = "";

