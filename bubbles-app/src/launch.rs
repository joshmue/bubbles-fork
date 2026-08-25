//! Nothing reads the stderr of a process started from an application menu, so
//! every failure ends in a dialog.

use std::process::ExitCode;

use gtk::gio;

use bubbles::{start_app_path, unix_request_blocking, vsock_path};

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().collect();
    let [_, bubble, app_id] = arguments.as_slice() else {
        eprintln!("usage: bubbles-launch <bubble> <application id>");
        return ExitCode::FAILURE;
    };

    let socket = vsock_path(bubble);
    if !socket.exists() {
        return fail(
            &format!("The bubble “{bubble}” is not running"),
            "Start it in Bubbles first, then launch the application again.",
        );
    }

    match unix_request_blocking(&socket, "POST", &start_app_path(app_id)) {
        Ok(response) if (200..300).contains(&response.status) => ExitCode::SUCCESS,
        Ok(response) if response.status == 404 => fail(
            &format!("“{app_id}” is no longer installed"),
            &format!(
                "The application was removed inside “{bubble}”. \
                 You can delete this launcher in the bubble's settings."
            ),
        ),
        Ok(response) => fail(
            &format!("“{app_id}” could not be started"),
            response.body.trim(),
        ),
        Err(error) => fail(
            &format!("The bubble “{bubble}” did not answer"),
            &format!("{error}. It may have been shut down in the meantime."),
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

