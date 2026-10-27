use std::path::Path;

use server::{Config, app, seed};
use ticketing::application::clock::SystemClock;
use tower_http::services::ServeDir;

const ADDR: &str = "127.0.0.1:3000";
/// Сюда `trunk build` складывает собранный клиент.
const CLIENT_DIST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../client/dist");

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = app(SystemClock, Config::default());
    seed(&app.commands).await?;

    if !Path::new(CLIENT_DIST).join("index.html").exists() {
        eprintln!("warning: client is not built, run `trunk build` in hw8/client");
    }
    let router = app.router.fallback_service(ServeDir::new(CLIENT_DIST));

    let listener = tokio::net::TcpListener::bind(ADDR).await?;
    println!("Listening on http://{ADDR}");
    axum::serve(listener, router).await?;
    Ok(())
}
