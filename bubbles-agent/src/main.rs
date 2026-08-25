mod desktop;

use axum::{
    extract::Query,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use tokio::process;

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/ready", get(ready))
        .route("/shutdown", post(shutdown))
        .route("/spawn-terminal", post(spawn_terminal))
        .route("/desktop-apps", get(desktop_apps))
        .route("/start-desktop-app", post(start_desktop_app));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .unwrap();
    axum::serve(listener, app).await.unwrap();
}

fn spawn_failed(program: &str, error: std::io::Error) -> (StatusCode, String) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("failed to start {program}: {error}"),
    )
}

async fn ready() -> &'static str {
    "OK"
}

async fn shutdown() -> impl IntoResponse {
    match process::Command::new("sudo")
        .arg("shutdown")
        .arg("-h")
        .arg("now")
        .spawn()
    {
        Ok(_) => (StatusCode::CREATED, "Shutdown initiated".to_string()),
        Err(error) => spawn_failed("shutdown", error),
    }
}

async fn spawn_terminal() -> impl IntoResponse {
    match graphical_command("x-terminal-emulator").spawn() {
        Ok(_) => (StatusCode::CREATED, "Spawned".to_string()),
        Err(error) => spawn_failed("x-terminal-emulator", error),
    }
}

fn graphical_command(program: &str) -> process::Command {
    let mut command = process::Command::new(program);
    command.current_dir("/home/user").env("XDG_RUNTIME_DIR", "/run/user/1000");
    command
}

async fn desktop_apps() -> impl IntoResponse {
    Json(desktop::list_apps())
}

#[derive(Deserialize)]
struct StartApp {
    app: String,
}

async fn start_desktop_app(Query(params): Query<StartApp>) -> impl IntoResponse {
    let Some(path) = desktop::resolve(&params.app) else {
        return (
            StatusCode::NOT_FOUND,
            format!("no application with id {}", params.app),
        );
    };

    // gio handles field codes and Terminal=true.
    match graphical_command("gio").arg("launch").arg(&path).spawn() {
        Ok(_) => (StatusCode::CREATED, "Launched".to_string()),
        Err(error) => spawn_failed("gio", error),
    }
}
