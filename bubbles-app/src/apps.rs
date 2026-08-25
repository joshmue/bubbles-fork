use gtk::gdk_pixbuf::Pixbuf;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use relm4::adw::prelude::*;
use relm4::factory::{DynamicIndex, FactoryVecDeque};
use relm4::prelude::FactoryComponent;
use relm4::{ComponentParts, ComponentSender, FactorySender, RelmWidgetExt, SimpleComponent};
use serde::Deserialize;

use bubbles::{unix_request, vsock_path};

use crate::exports;
use crate::portal;

const ICON_SIZE: i32 = 32;

#[derive(Deserialize, Debug, Clone)]
struct GuestIcon {
    format: String,
    data: String,
}

#[derive(Deserialize, Debug, Clone)]
struct GuestApp {
    id: String,
    name: String,
    #[serde(default)]
    comment: String,
    #[serde(default)]
    wm_class: String,
    #[serde(default)]
    icon: Option<GuestIcon>,
}

#[derive(Deserialize, Debug, Default)]
pub struct GuestAppList {
    apps: Vec<GuestApp>,
}

fn clean(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect::<String>()
        .trim()
        .to_string()
}

// Everything the guest sends is hostile until proven otherwise.
fn sanitise(app: GuestApp) -> Option<GuestApp> {
    let id_ok = !app.id.is_empty()
        && app.id.len() <= 128
        && app
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'));
    let name = clean(&app.name, 128);
    if !id_ok || name.is_empty() {
        return None;
    }
    Some(GuestApp {
        id: app.id,
        name,
        comment: clean(&app.comment, 256),
        wm_class: clean(&app.wm_class, 128),
        icon: app
            .icon
            .filter(|icon| matches!(icon.format.as_str(), "png" | "svg")),
    })
}

fn icon_bytes(app: &GuestApp) -> Option<Vec<u8>> {
    let icon = app.icon.as_ref()?;
    let bytes = glib::base64_decode(&icon.data);
    (!bytes.is_empty() && bytes.len() <= portal::MAX_ICON_BYTES).then_some(bytes)
}

// Via gdk-pixbuf, not Texture::from_bytes, which cannot read the SVGs most
// icon themes ship.
fn row_image(app: &GuestApp) -> gtk::Image {
    let texture = icon_bytes(app).and_then(|bytes| {
        let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(bytes));
        let pixbuf =
            Pixbuf::from_stream_at_scale(&stream, ICON_SIZE, ICON_SIZE, true, gio::Cancellable::NONE)
                .ok()?;
        Some(gtk::gdk::Texture::for_pixbuf(&pixbuf))
    });
    let image = match texture {
        Some(texture) => gtk::Image::from_paintable(Some(&texture)),
        None => gtk::Image::from_icon_name("application-x-executable"),
    };
    image.set_pixel_size(ICON_SIZE);
    image
}

#[derive(Debug)]
struct RowInit {
    app: GuestApp,
    exported: bool,
}

#[derive(Debug)]
struct AppRow {
    app: GuestApp,
    image: gtk::Image,
    exported: bool,
    busy: bool,
}

#[derive(Debug)]
enum AppRowOutput {
    Export(DynamicIndex),
    Remove(DynamicIndex),
}

#[relm4::factory]
impl FactoryComponent for AppRow {
    type Init = RowInit;
    type Input = ();
    type Output = AppRowOutput;
    type CommandOutput = ();
    type ParentWidget = gtk::ListBox;

    view! {
        #[root]
        relm4::adw::ActionRow {
            set_title: &glib::markup_escape_text(&self.app.name),
            set_subtitle: &glib::markup_escape_text(&self.app.comment),
            add_prefix: &self.image,
            add_suffix = &gtk::Spinner {
                #[watch]
                set_visible: self.busy,
                #[watch]
                set_spinning: self.busy,
            },
            add_suffix = &gtk::Button {
                set_valign: gtk::Align::Center,
                set_label: "Export",
                set_tooltip_text: Some("Add this application to the host's menu"),
                #[watch]
                set_visible: !self.exported,
                #[watch]
                set_sensitive: !self.busy,
                connect_clicked[sender, index] => move |_| {
                    sender.output(AppRowOutput::Export(index.clone())).unwrap();
                }
            },
            add_suffix = &gtk::Button {
                set_valign: gtk::Align::Center,
                set_icon_name: "user-trash-symbolic",
                set_css_classes: &["flat"],
                set_tooltip_text: Some("Remove this application from the host's menu"),
                #[watch]
                set_visible: self.exported,
                #[watch]
                set_sensitive: !self.busy,
                connect_clicked[sender, index] => move |_| {
                    sender.output(AppRowOutput::Remove(index.clone())).unwrap();
                }
            },
        }
    }

    fn init_model(
        RowInit { app, exported }: Self::Init,
        _index: &DynamicIndex,
        _sender: FactorySender<Self>,
    ) -> Self {
        let image = row_image(&app);
        Self { app, image, exported, busy: false }
    }

    fn update(&mut self, _msg: Self::Input, _sender: FactorySender<Self>) {}
}

#[derive(PartialEq, Debug, Clone)]
enum ListState {
    Loading,
    Loaded,
    Message(String, String),
}

pub struct AppsDialog {
    bubble: String,
    rows: FactoryVecDeque<AppRow>,
    state: ListState,
    title: String,
    toasts: relm4::adw::ToastOverlay,
}

#[derive(Debug)]
pub enum AppsMsg {
    Load(String),
    Refresh,
    Loaded(Box<GuestAppList>),
    Failed(String),
    Export(DynamicIndex),
    Remove(DynamicIndex),
    ExportFinished(String, Result<String, String>),
    RemoveFinished(String, Result<(), String>),
}

#[relm4::component(pub)]
impl SimpleComponent for AppsDialog {
    type Init = ();
    type Input = AppsMsg;
    type Output = ();

    view! {
        dialog = relm4::adw::Dialog {
            set_content_width: 520,
            set_content_height: 620,
            #[watch]
            set_title: &model.title,
            #[wrap(Some)]
            set_child = &relm4::adw::ToolbarView {
                add_top_bar = &relm4::adw::HeaderBar {
                    pack_end = &gtk::Button {
                        set_icon_name: "view-refresh-symbolic",
                        set_tooltip_text: Some("Reload the list"),
                        #[watch]
                        set_sensitive: model.state != ListState::Loading,
                        connect_clicked => AppsMsg::Refresh,
                    },
                },
                #[wrap(Some)]
                set_content: toasts = &relm4::adw::ToastOverlay {
                    #[wrap(Some)]
                    set_child = &gtk::Stack {
                        add_named[Some("loading")] = &relm4::adw::StatusPage {
                            set_title: "Looking for applications",
                            #[wrap(Some)]
                            set_child = &gtk::Spinner {
                                set_spinning: true,
                                set_width_request: 32,
                                set_height_request: 32,
                            },
                        },
                        add_named[Some("message")] = &relm4::adw::StatusPage {
                            set_icon_name: Some("application-x-executable-symbolic"),
                            #[watch]
                            set_title: match &model.state {
                                ListState::Message(title, _) => title.as_str(),
                                _ => "",
                            },
                            #[watch]
                            set_description: Some(match &model.state {
                                ListState::Message(_, description) => description.as_str(),
                                _ => "",
                            }),
                        },
                        add_named[Some("list")] = &gtk::ScrolledWindow {
                            set_vexpand: true,
                            #[wrap(Some)]
                            set_child = &relm4::adw::Clamp {
                                set_margin_all: 12,
                                #[local_ref]
                                rows_listbox -> gtk::ListBox {
                                    set_css_classes: &["boxed-list"],
                                    set_selection_mode: gtk::SelectionMode::None,
                                    set_valign: gtk::Align::Start,
                                },
                            },
                        },
                        // Set last: naming a child only works once it exists.
                        #[watch]
                        set_visible_child_name: match &model.state {
                            ListState::Loading => "loading",
                            ListState::Loaded => "list",
                            ListState::Message(_, _) => "message",
                        },
                    },
                },
            },
        }
    }

    fn init(_init: Self::Init, _root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let rows: FactoryVecDeque<AppRow> = FactoryVecDeque::builder()
            .launch_default()
            .forward(sender.input_sender(), |output| match output {
                AppRowOutput::Export(index) => AppsMsg::Export(index),
                AppRowOutput::Remove(index) => AppsMsg::Remove(index),
            });
        let rows_listbox_widget = rows.widget().clone();

        let mut model = AppsDialog {
            bubble: String::new(),
            rows,
            state: ListState::Loading,
            title: "Applications".to_string(),
            toasts: relm4::adw::ToastOverlay::new(),
        };

        let rows_listbox = &rows_listbox_widget;
        let widgets = view_output!();
        model.toasts = widgets.toasts.clone();

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            AppsMsg::Load(bubble) => {
                self.bubble = bubble.clone();
                self.title = format!("{bubble} Applications");
                exports::reconcile(&bubble);
                sender.input(AppsMsg::Refresh);
            }
            AppsMsg::Refresh => {
                if self.bubble.is_empty() {
                    return;
                }
                self.state = ListState::Loading;
                self.rows.guard().clear();
                let socket = vsock_path(&self.bubble);
                relm4::spawn_local(async move {
                    match unix_request(&socket, "GET", "/desktop-apps").await {
                        Ok(response) if response.status == 200 => {
                            match serde_json::from_str::<GuestAppList>(&response.body) {
                                Ok(list) => sender.input(AppsMsg::Loaded(Box::new(list))),
                                Err(error) => sender.input(AppsMsg::Failed(format!(
                                    "The bubble sent something unreadable: {error}"
                                ))),
                            }
                        }
                        Ok(response) => sender.input(AppsMsg::Failed(format!(
                            "The agent answered with status {}. This bubble may be running an older image.",
                            response.status
                        ))),
                        Err(error) => {
                            sender.input(AppsMsg::Failed(format!("No answer from the bubble: {error}")))
                        }
                    }
                });
            }
            AppsMsg::Loaded(list) => {
                let exported = exports::load(&self.bubble);
                let mut guard = self.rows.guard();
                guard.clear();
                for app in list.apps.into_iter().filter_map(sanitise) {
                    let is_exported =
                        exported.contains_key(&exports::desktop_file_id(&self.bubble, &app.id));
                    guard.push_back(RowInit { app, exported: is_exported });
                }
                let count = guard.len();
                drop(guard);
                self.state = if count == 0 {
                    ListState::Message(
                        "No applications".to_string(),
                        "This bubble has nothing installed that would show up in a menu."
                            .to_string(),
                    )
                } else {
                    ListState::Loaded
                };
            }
            AppsMsg::Failed(reason) => {
                self.state =
                    ListState::Message("Could not read the application list".to_string(), reason);
            }
            AppsMsg::Export(index) => {
                let Some(app) = self.rows.get(index.current_index()).map(|row| row.app.clone())
                else {
                    return;
                };
                self.set_busy(&app.id, true);
                let bubble = self.bubble.clone();
                relm4::spawn_local(async move {
                    let result = export_one(&bubble, &app).await;
                    sender.input(AppsMsg::ExportFinished(app.id.clone(), result));
                });
            }
            AppsMsg::ExportFinished(app_id, result) => {
                self.set_busy(&app_id, false);
                match result {
                    // Empty name: the user closed the portal's dialog.
                    Ok(name) if name.is_empty() => {}
                    Ok(name) => {
                        self.set_exported(&app_id, true);
                        self.toast(&format!("“{name}” added to the application menu"));
                    }
                    Err(error) => self.toast(&error),
                }
            }
            AppsMsg::Remove(index) => {
                let Some(app_id) = self.rows.get(index.current_index()).map(|row| row.app.id.clone())
                else {
                    return;
                };
                self.set_busy(&app_id, true);
                let bubble = self.bubble.clone();
                relm4::spawn_local(async move {
                    let desktop_file_id = exports::desktop_file_id(&bubble, &app_id);
                    let result = portal::uninstall(&desktop_file_id).await;
                    if result.is_ok() {
                        exports::remove(&bubble, &desktop_file_id);
                    }
                    sender.input(AppsMsg::RemoveFinished(app_id, result));
                });
            }
            AppsMsg::RemoveFinished(app_id, result) => {
                self.set_busy(&app_id, false);
                match result {
                    Ok(()) => {
                        self.set_exported(&app_id, false);
                        self.toast("Removed from the application menu");
                    }
                    Err(error) => self.toast(&error),
                }
            }
        }
    }
}

impl AppsDialog {
    // By id, not index: a refresh mid-export would move the answer onto
    // whatever row sits at that index now.
    fn position_of(&self, app_id: &str) -> Option<usize> {
        self.rows.iter().position(|row| row.app.id == app_id)
    }

    fn set_busy(&mut self, app_id: &str, busy: bool) {
        if let Some(position) = self.position_of(app_id) {
            if let Some(row) = self.rows.guard().get_mut(position) {
                row.busy = busy;
            }
        }
    }

    fn set_exported(&mut self, app_id: &str, exported: bool) {
        if let Some(position) = self.position_of(app_id) {
            if let Some(row) = self.rows.guard().get_mut(position) {
                row.exported = exported;
            }
        }
    }

    fn toast(&self, message: &str) {
        self.toasts.add_toast(relm4::adw::Toast::new(message));
    }
}

/// The name the launcher ended up with, empty if the user cancelled.
async fn export_one(bubble: &str, app: &GuestApp) -> Result<String, String> {
    let proposed = format!("{} ({bubble})", app.name);
    let token = portal::prepare_install(portal::NO_PARENT_WINDOW, &proposed, icon_bytes(app)).await?;
    let Some(token) = token else {
        return Ok(String::new());
    };

    let desktop_file_id = exports::desktop_file_id(bubble, &app.id);
    let entry = exports::desktop_entry(bubble, &app.id, &app.comment, &app.wm_class);
    portal::install(&token, &desktop_file_id, &entry).await?;
    exports::insert(bubble, &app.id, &proposed);
    Ok(proposed)
}

// The caller reads the ids first: the file naming them lives in the directory
// that is about to be deleted.
pub async fn uninstall_launchers(desktop_file_ids: Vec<String>) {
    for desktop_file_id in desktop_file_ids {
        if let Err(error) = portal::uninstall(&desktop_file_id).await {
            eprintln!("could not remove launcher {desktop_file_id}: {error}");
        }
    }
}
