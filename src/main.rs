// Release builds are GUI-subsystem so no console window appears.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ghost_clipboard::app::{self, Options};
use ghost_clipboard::platform::Hotkey;
use ghost_clipboard::platform::windows::{Shell, ShellConfig};
use ghost_clipboard::storage::{Storage, default_db_path};

fn main() {
    let logs_dir = ghost_clipboard::storage::default_data_dir()
        .map(|d| d.join("logs"))
        .unwrap_or_else(|_| std::path::PathBuf::from("logs"));
    let _ = ghost_clipboard::logging::init_logging(&logs_dir);

    let _single_instance = match ghost_clipboard::platform::acquire_single_instance() {
        Ok(Some(guard)) => Some(guard),
        Ok(None) => {
            tracing::info!("GhostClipboard is already running. Activated existing instance.");
            return;
        }
        Err(error) => {
            tracing::warn!(%error, "could not acquire single-instance mutex; proceeding anyway");
            None
        }
    };

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
            ghost_clipboard::platform::show_error_dialog(
                "GhostClipboard - Startup Error",
                &format!("Failed to start the native shell: {error}"),
            );
            std::process::exit(1);
        }
    };

    let db_path = match default_db_path() {
        Ok(p) => p,
        Err(error) => {
            tracing::error!(%error, "failed to determine application data path");
            ghost_clipboard::platform::show_error_dialog(
                "GhostClipboard - Startup Error",
                &format!("Failed to determine application data path: {error}"),
            );
            std::process::exit(1);
        }
    };

    let storage = match Storage::spawn(db_path.clone()) {
        Ok(storage) => storage,
        Err(ghost_clipboard::storage::StorageError::Corrupt(err)) => {
            tracing::error!(%err, "history database corrupted");
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let file_name = db_path
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or("history.db");
            let corrupt_name = format!("{file_name}.corrupt-{timestamp}");
            let msg = format!(
                "The history database is damaged and could not be read:\n\n{err}\n\nWould you like to rename the corrupt database to '{corrupt_name}' and start fresh?"
            );
            if ghost_clipboard::platform::show_confirm_dialog(
                "GhostClipboard - Database Corrupt",
                &msg,
            ) {
                if let Err(e) = ghost_clipboard::storage::rename_corrupt_db(&db_path, timestamp) {
                    ghost_clipboard::platform::show_error_dialog(
                        "GhostClipboard - Recovery Failed",
                        &format!("Failed to rename corrupt database: {e}"),
                    );
                    std::process::exit(1);
                }
                match Storage::spawn(db_path) {
                    Ok(fresh_storage) => {
                        tracing::info!("started fresh database after corruption recovery");
                        fresh_storage
                    }
                    Err(second_err) => {
                        ghost_clipboard::platform::show_error_dialog(
                            "GhostClipboard - Fatal Startup Error",
                            &format!("Failed to create new history database: {second_err}"),
                        );
                        std::process::exit(1);
                    }
                }
            } else {
                std::process::exit(1);
            }
        }
        Err(error) => {
            tracing::error!(%error, "failed to initialize SQLite storage");
            ghost_clipboard::platform::show_error_dialog(
                "GhostClipboard - Storage Error",
                &format!("Failed to initialize database: {error}"),
            );
            std::process::exit(1);
        }
    };

    if let Err(error) = app::run(shell, storage, options) {
        tracing::error!(%error, "the user interface stopped unexpectedly");
        ghost_clipboard::platform::show_error_dialog(
            "GhostClipboard - Error",
            &format!("The user interface stopped unexpectedly: {error}"),
        );
        std::process::exit(1);
    }
}
