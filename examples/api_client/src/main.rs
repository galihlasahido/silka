//! The binary: parse the command line, start the bundled server, open a window.
//!
//! Everything else lives in the library half of this crate, which is what lets
//! the behaviour tests drive the very same [`Shell`](silka_api_client::app::Shell)
//! this file opens a window around.

use silka_api_client::app;
use silka_api_client::serve::DummyServer;
use silka_platform::{window, PlatformError};
use silka_theme::{Appearance, Preset, Theme};
use silka_widgets::{install_fonts, Fonts};

/// The window title.
pub const TITLE: &str = "API Client — silka";

fn main() -> Result<(), PlatformError> {
    let options = Options::from_args(std::env::args().skip(1));

    // The panic hook, installed **once**, before anything can panic: it is what
    // gives a caught report its `file:line:column`, and without it every
    // boundary in this application would report a message with no address
    // (REKOMENDASI §9.7).
    silka_core::recover::install_hook();

    // One text engine for the whole application: scanning system fonts is
    // expensive and the glyph atlas has to be shared, so the same glyph is
    // never rasterised twice (§3.3).
    let fonts = Fonts::new();
    install_fonts(&fonts);

    // The server is kept alive for as long as `main` runs — dropping it stops
    // it, so it must outlive the window rather than the statement that made it.
    let server = if options.serve {
        match DummyServer::start(options.port) {
            Ok(server) => {
                println!("silka-api-client: test server on {}", server.base_url());
                Some(server)
            }
            Err(e) => {
                eprintln!("silka-api-client: could not start the test server: {e}");
                eprintln!("silka-api-client: the saved requests will fail until you pass --url");
                None
            }
        }
    } else {
        None
    };

    let base = options.base(server.as_ref());
    println!("silka-api-client: saved requests point at {base}");

    let mut config = window(TITLE)
        .size(1280.0, 860.0)
        .min_size(760.0, 520.0)
        .preset(options.preset);
    config = match options.appearance {
        Some(a) => config.appearance(a),
        None => config.follow_system_appearance(),
    };

    let result = app::run(
        config,
        Theme::new(options.preset, options.appearance.unwrap_or_default()),
        base,
    );
    // Named rather than `_`: `let _ = server;` would drop it immediately, which
    // is exactly the bug this line exists to avoid.
    drop(server);
    result
}

/// The command line, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// `--preset cupertino|tailwind`.
    pub preset: Preset,
    /// `--appearance light|dark`; `None` follows the OS.
    pub appearance: Option<Appearance>,
    /// `--port N` for the bundled server; `0` takes any free port.
    pub port: u16,
    /// `--no-server` leaves it unstarted.
    pub serve: bool,
    /// `--url http://host:port` — where the saved requests point instead.
    pub url: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            preset: Preset::Cupertino,
            appearance: None,
            port: 0,
            serve: true,
            url: None,
        }
    }
}

impl Options {
    /// Parse the arguments after the program name.
    pub fn from_args(args: impl Iterator<Item = String>) -> Options {
        let mut options = Options::default();
        let args: Vec<String> = args.collect();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--preset" => {
                    if let Some(v) = args.get(i + 1) {
                        options.preset = match v.as_str() {
                            "tailwind" | "shadcn" => Preset::Tailwind,
                            _ => Preset::Cupertino,
                        };
                        i += 1;
                    }
                }
                "--appearance" => {
                    if let Some(v) = args.get(i + 1) {
                        options.appearance = match v.as_str() {
                            "dark" => Some(Appearance::Dark),
                            "light" => Some(Appearance::Light),
                            _ => None,
                        };
                        i += 1;
                    }
                }
                "--port" => {
                    if let Some(v) = args.get(i + 1).and_then(|v| v.parse().ok()) {
                        options.port = v;
                        i += 1;
                    }
                }
                "--url" => {
                    if let Some(v) = args.get(i + 1) {
                        options.url = Some(v.trim_end_matches('/').to_string());
                        i += 1;
                    }
                }
                "--no-server" => options.serve = false,
                _ => {}
            }
            i += 1;
        }
        options
    }

    /// Where the saved requests point.
    ///
    /// `--url` wins, then the server that actually started; if neither exists
    /// the samples still have to be *something* parsable, so they point at the
    /// port that was asked for and fail honestly.
    pub fn base(&self, server: Option<&DummyServer>) -> String {
        if let Some(url) = &self.url {
            return url.clone();
        }
        match server {
            Some(server) => server.base_url(),
            None => format!("http://127.0.0.1:{}", self.port),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(args: &[&str]) -> Options {
        Options::from_args(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn without_arguments_it_starts_its_own_server_on_a_free_port() {
        let o = options(&[]);
        assert_eq!(o.preset, Preset::Cupertino);
        assert!(o.appearance.is_none());
        assert!(o.serve);
        assert_eq!(o.port, 0);
        assert_eq!(o.base(None), "http://127.0.0.1:0");
    }

    #[test]
    fn every_switch_can_be_pinned_from_the_command_line() {
        assert_eq!(options(&["--preset", "tailwind"]).preset, Preset::Tailwind);
        assert_eq!(options(&["--preset", "nonsense"]).preset, Preset::Cupertino);
        assert_eq!(
            options(&["--appearance", "dark"]).appearance,
            Some(Appearance::Dark)
        );
        assert_eq!(options(&["--port", "9100"]).port, 9100);
        assert!(!options(&["--no-server"]).serve);
        // A trailing slash would produce `http://h//ok` in every sample.
        assert_eq!(
            options(&["--url", "http://h:3000/"]).base(None),
            "http://h:3000"
        );
    }

    #[test]
    fn a_flag_with_no_value_is_ignored_rather_than_a_panic() {
        assert_eq!(options(&["--url"]).url, None);
        assert_eq!(options(&["--port"]).port, 0);
        assert_eq!(options(&["--port", "notanumber"]).port, 0);
        assert_eq!(options(&["--preset"]).preset, Preset::Cupertino);
    }
}
