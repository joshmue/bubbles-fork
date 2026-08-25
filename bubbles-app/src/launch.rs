//! Nothing reads the stderr of a process started from an application menu, so
//! every failure ends in a dialog.

use std::process::ExitCode;

use gtk::gio::prelude::*;
use gtk::gio::{self, DBusCallFlags};
use gtk::glib;

use bubbles::{
    APP_ID, LAUNCHER_FAILED_ERROR, LAUNCHER_INTERFACE, LAUNCHER_NOT_INSTALLED_ERROR,
    LAUNCHER_NOT_RUNNING_ERROR, LAUNCHER_OBJECT_PATH,
};

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().collect();
    let [_, bubble, app_id] = arguments.as_slice() else {
        eprintln!("usage: bubbles-launch <bubble> <application id>");
        return ExitCode::FAILURE;
    };

    // The address the bubble's agent listens on is claimed at start time and
    // known only to the running app, so the request goes there rather than to
    // the bubble. No .service file is installed, so this call cannot start the
    // app: an unowned name comes back as an error and is reported as such.
    let connection = match gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) {
        Ok(connection) => connection,
        Err(error) => {
            return fail("No connection to the session bus", &error.to_string());
        }
    };

    let result = connection.call_sync(
        Some(APP_ID),
        LAUNCHER_OBJECT_PATH,
        LAUNCHER_INTERFACE,
        "StartApplication",
        Some(&(bubble, app_id).to_variant()),
        None,
        DBusCallFlags::NONE,
        -1,
        gio::Cancellable::NONE,
    );

    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => report(bubble, app_id, &error),
    }
}

fn report(bubble: &str, app_id: &str, error: &glib::Error) -> ExitCode {
    let name = gio::DBusError::remote_error(error).map(|name| name.to_string());
    let mut stripped = error.clone();
    gio::DBusError::strip_remote_error(&mut stripped);
    let detail = stripped.message().to_string();

    match name.as_deref() {
        Some(LAUNCHER_NOT_RUNNING_ERROR) => fail(
            &format!("The bubble “{bubble}” is not running"),
            "Start it in Bubbles first, then launch the application again.",
        ),
        Some(LAUNCHER_NOT_INSTALLED_ERROR) => fail(
            &format!("“{app_id}” is no longer installed"),
            &format!(
                "The application was removed inside “{bubble}”. \
                 You can remove this launcher in the bubble's application list."
            ),
        ),
        Some(LAUNCHER_FAILED_ERROR) => fail(&format!("“{app_id}” could not be started"), &detail),
        // Nobody owns the name and the service file did not start one.
        _ => fail(
            "Bubbles is not running",
            &format!("Start Bubbles, then launch the application again. ({detail})"),
        ),
    }
}

fn fail(message: &str, detail: &str) -> ExitCode {
    eprintln!("bubbles-launch: {message}: {detail}");
    show_dialog(message, detail);
    ExitCode::FAILURE
}

fn show_dialog(message: &str, detail: &str) {
    if gtk::init().is_err() {
        return;
    }
    let main_loop = gtk::glib::MainLoop::new(None, false);
    let dialog = gtk::AlertDialog::builder()
        .modal(true)
        .message(message)
        .detail(detail)
        .buttons(["Close"])
        .build();
    let quit = main_loop.clone();
    dialog.choose(None::<&gtk::Window>, gio::Cancellable::NONE, move |_| {
        quit.quit();
    });
    main_loop.run();
}
