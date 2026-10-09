// Release builds are GUI-subsystem so no console window appears.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ghost_clipboard::app::{self, Options};
use ghost_clipboard::platform::Hotkey;
use ghost_clipboard::platform::windows::{Shell, ShellConfig};
use ghost_clipboard::storage::{Storage, default_db_path};

use tracing_subscriber::EnvFilter;

fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("ghost_clipboard=info")),
        )
        .init();

    let options = Options {
        background: std::env::args().any(|arg| arg == "--background"),
        db_path: None,
    };

    let config = ShellConfig {
        hotkey: Hotkey::DEFAULT,
    };

    let shell = match Shell::spawn(config) {
        Ok(shell) => shell,
        Err(error) => {
            tracing::error!(%error, "failed to start the native shell");
            std::process::exit(1);
        }
    };

    let db_path = match default_db_path() {
        Ok(p) => p,
        Err(error) => {
            tracing::error!(%error, "failed to determine application data path");
            std::process::exit(1);
        }
    };

    let storage = match Storage::spawn(db_path) {
        Ok(storage) => storage,
        Err(error) => {
            tracing::error!(%error, "failed to initialize SQLite storage");
            std::process::exit(1);
        }
    };

    if let Err(error) = app::run(shell, storage, options) {
        tracing::error!(%error, "the user interface stopped unexpectedly");
        std::process::exit(1);
    }
}
