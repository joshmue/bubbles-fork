mod desktop;

use std::sync::Arc;

use axum::{
    extract::{Query, Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use tokio::process;

const TOKEN_CREDENTIAL: &str = "bubbles.agent-token";

#[tokio::main]
async fn main() {
    let token = read_token();
    let app = Router::new()
        .route("/ready", get(ready))
        .route("/shutdown", post(shutdown))
        .route("/spawn-terminal", post(spawn_terminal))
        .route("/desktop-apps", get(desktop_apps))
        .route("/start-desktop-app", post(start_desktop_app))
        .layer(middleware::from_fn_with_state(token, authorize));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:11111")
        .await
        .unwrap();
    axum::serve(listener, app).await.unwrap();
}

// The host passes the token as a systemd credential; without one, anyone who
// can reach the port could drive the bubble, so there is no fallback.
fn read_token() -> Arc<[u8]> {
    let directory = std::env::var("CREDENTIALS_DIRECTORY")
        .unwrap_or_else(|_| panic!("no credentials directory, is {TOKEN_CREDENTIAL} loaded?"));
    let token = std::fs::read(std::path::Path::new(&directory).join(TOKEN_CREDENTIAL))
        .unwrap_or_else(|error| panic!("cannot read {TOKEN_CREDENTIAL}: {error}"));
    let token = token.trim_ascii();
    assert!(token.len() >= 32, "{TOKEN_CREDENTIAL} is too short");
    token.into()
}

async fn authorize(State(token): State<Arc<[u8]>>, request: Request, next: Next) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.as_bytes().strip_prefix(b"Bearer "));
    match presented {
        Some(presented) if presented == &token[..] => next.run(request).await,
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
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
