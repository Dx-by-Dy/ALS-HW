use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::process::ExitCode;

use server::{Config, app, seed};
use ticketing::application::clock::{ScaledClock, SystemClock};
use tower_http::services::ServeDir;

const DEFAULT_PORT: u16 = 3000;
const USAGE: &str = "usage: server [--port <PORT>] [--seed] [--time-scale <N>]";
const SECS_PER_DAY: f64 = 86_400.0;
/// Сюда `trunk build` складывает собранный клиент.
const CLIENT_DIST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../client/dist");

/// Что просили в командной строке.
#[derive(Debug, PartialEq, Eq)]
enum Cli {
    Run(Options),
    Help,
}

#[derive(Debug, PartialEq, Eq)]
struct Options {
    port: u16,
    seed: bool,
    /// Во сколько раз часы системы идут быстрее настоящих.
    time_scale: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            seed: false,
            time_scale: 1,
        }
    }
}

/// Значения можно задать и следующим аргументом, и через `=`: `--port=<PORT>`.
fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Cli, String> {
    let mut options = Options::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) => (name, Some(value.to_owned())),
            None => (arg.as_str(), None),
        };
        match (name, inline) {
            ("-h" | "--help", None) => return Ok(Cli::Help),
            ("--seed", None) => options.seed = true,
            ("-p" | "--port", inline) => {
                let value = inline
                    .or_else(|| args.next())
                    .ok_or("--port needs a value")?;
                options.port = value
                    .parse()
                    .map_err(|_| format!("invalid port '{value}', expected a number 0-65535"))?;
            }
            ("--time-scale", inline) => {
                let value = inline
                    .or_else(|| args.next())
                    .ok_or("--time-scale needs a value")?;
                options.time_scale =
                    value
                        .parse()
                        .ok()
                        .filter(|scale| *scale >= 1)
                        .ok_or_else(|| {
                            format!(
                                "invalid time scale '{value}', expected a positive whole number"
                            )
                        })?;
            }
            _ => return Err(format!("unexpected argument '{arg}'")),
        }
    }
    Ok(Cli::Run(options))
}

#[tokio::main]
async fn main() -> ExitCode {
    let options = match parse_args(std::env::args().skip(1)) {
        Ok(Cli::Run(options)) => options,
        Ok(Cli::Help) => {
            println!(
                "{USAGE}\n\nServes the ticket API and the web client on 127.0.0.1 (default port {DEFAULT_PORT}).\nThe system starts empty; --seed fills it with demo data.\n--time-scale <N> runs the clock N times faster, so deadlines can be watched: with 8640 a day passes every 10 seconds."
            );
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("error: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(options).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run(options: Options) -> Result<(), Box<dyn std::error::Error>> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, options.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("cannot listen on {addr}: {e}"))?;

    let app = match options.time_scale {
        1 => app(SystemClock, Config::default()),
        scale => {
            println!(
                "Clock runs {scale} times faster: a day passes every {:.1} s",
                SECS_PER_DAY / f64::from(scale)
            );
            app(
                ScaledClock::starting_now(scale),
                Config::for_time_scale(scale),
            )
        }
    };
    if options.seed {
        seed(&app).await?;
    }
    if !Path::new(CLIENT_DIST).join("index.html").exists() {
        eprintln!("warning: client is not built, run `trunk build` in hw10/client");
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
        assert_eq!(
            parse(&[]),
            Ok(Cli::Run(Options {
                port: 3000,
                seed: false,
                time_scale: 1
            }))
        );
    }

    #[test]
    fn port_is_taken_from_arguments() {
        let run = |port| {
            Cli::Run(Options {
                port,
                ..Options::default()
            })
        };
        assert_eq!(parse(&["--port", "8000"]), Ok(run(8000)));
        assert_eq!(parse(&["-p", "8001"]), Ok(run(8001)));
        assert_eq!(parse(&["--port=0"]), Ok(run(0)));
    }

    #[test]
    fn seed_is_off_unless_asked_for() {
        assert_eq!(
            parse(&["--seed", "--port", "8000"]),
            Ok(Cli::Run(Options {
                port: 8000,
                seed: true,
                ..Options::default()
            }))
        );
    }

    #[test]
    fn time_scale_is_taken_from_arguments() {
        let run = |time_scale| {
            Cli::Run(Options {
                time_scale,
                ..Options::default()
            })
        };
        assert_eq!(parse(&["--time-scale", "8640"]), Ok(run(8640)));
        assert_eq!(parse(&["--time-scale=60"]), Ok(run(60)));
    }

    #[test]
    fn bad_time_scale_is_reported() {
        let invalid = |value: &str| {
            Err(format!(
                "invalid time scale '{value}', expected a positive whole number"
            ))
        };
        assert_eq!(parse(&["--time-scale", "0"]), invalid("0"));
        assert_eq!(parse(&["--time-scale", "-5"]), invalid("-5"));
        assert_eq!(parse(&["--time-scale=fast"]), invalid("fast"));
        assert_eq!(
            parse(&["--time-scale"]),
            Err("--time-scale needs a value".to_owned())
        );
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
