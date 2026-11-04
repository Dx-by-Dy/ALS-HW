use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::process::ExitCode;

use server::{Config, app, seed};
use ticketing::application::clock::SystemClock;
use tower_http::services::ServeDir;

const DEFAULT_PORT: u16 = 3000;
const USAGE: &str = "usage: server [--port <PORT>]";
/// Сюда `trunk build` складывает собранный клиент.
const CLIENT_DIST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../client/dist");

/// Что просили в командной строке.
#[derive(Debug, PartialEq, Eq)]
enum Cli {
    Run { port: u16 },
    Help,
}

/// `server [--port <PORT>]`; порт также можно задать как `--port=<PORT>`.
fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Cli, String> {
    let mut port = DEFAULT_PORT;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let value = match arg.as_str() {
            "-h" | "--help" => return Ok(Cli::Help),
            "-p" | "--port" => args.next().ok_or("--port needs a value")?,
            _ => match arg.strip_prefix("--port=") {
                Some(value) => value.to_owned(),
                None => return Err(format!("unexpected argument '{arg}'")),
            },
        };
        port = value
            .parse()
            .map_err(|_| format!("invalid port '{value}', expected a number 0-65535"))?;
    }
    Ok(Cli::Run { port })
}

#[tokio::main]
async fn main() -> ExitCode {
    let port = match parse_args(std::env::args().skip(1)) {
        Ok(Cli::Run { port }) => port,
        Ok(Cli::Help) => {
            println!(
                "{USAGE}\n\nServes the ticket API and the web client on 127.0.0.1 (default port {DEFAULT_PORT})."
            );
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("error: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(port).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run(port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("cannot listen on {addr}: {e}"))?;

    let app = app(SystemClock, Config::default());
    seed(&app.commands).await?;
    if !Path::new(CLIENT_DIST).join("index.html").exists() {
        eprintln!("warning: client is not built, run `trunk build` in hw8/client");
    }
    let router = app.router.fallback_service(ServeDir::new(CLIENT_DIST));

    // Порт 0 — «любой свободный»: печатаем тот, что выдала система.
    println!("Listening on http://{}", listener.local_addr()?);
    axum::serve(listener, router).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, String> {
        parse_args(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn port_defaults_to_3000() {
        assert_eq!(parse(&[]), Ok(Cli::Run { port: 3000 }));
    }

    #[test]
    fn port_is_taken_from_arguments() {
        assert_eq!(parse(&["--port", "8000"]), Ok(Cli::Run { port: 8000 }));
        assert_eq!(parse(&["-p", "8001"]), Ok(Cli::Run { port: 8001 }));
        assert_eq!(parse(&["--port=0"]), Ok(Cli::Run { port: 0 }));
    }

    #[test]
    fn help_is_recognized() {
        assert_eq!(parse(&["--port", "8000", "--help"]), Ok(Cli::Help));
    }

    #[test]
    fn bad_arguments_are_reported() {
        assert_eq!(parse(&["--port"]), Err("--port needs a value".to_owned()));
        assert_eq!(
            parse(&["--port", "70000"]),
            Err("invalid port '70000', expected a number 0-65535".to_owned())
        );
        assert_eq!(
            parse(&["8000"]),
            Err("unexpected argument '8000'".to_owned())
        );
    }
}
