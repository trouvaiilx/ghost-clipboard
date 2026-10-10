//! Logging and diagnostic utilities.
//!
//! Provides rolling file logs under `%APPDATA%\GhostClipboard\logs`
//! and a panic hook suited for `panic = "abort"` release builds.

use std::path::Path;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Format panic info into message and location only.
pub fn format_panic_info(info: &std::panic::PanicHookInfo<'_>) -> (String, String) {
    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "unknown location".to_string());
    let message = if let Some(s) = info.payload().downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_string()
    };
    (message, location)
}

/// Set up a panic hook that logs message and location only.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let (message, location) = format_panic_info(info);
        tracing::error!(%location, %message, "application panicked");
    }));
}

/// Initialize tracing subscriber with stderr and rolling file appender.
pub fn init_logging(
    logs_dir: impl AsRef<Path>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let logs_dir = logs_dir.as_ref();
    std::fs::create_dir_all(logs_dir)?;

    let file_appender = tracing_appender::rolling::daily(logs_dir, "ghostclipboard.log");

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("ghost_clipboard=info"));

    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(true);

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_appender)
        .with_ansi(false);

    tracing_subscriber::registry()
        .with(filter)
        .with(stderr_layer)
        .with(file_layer)
        .try_init()?;

    install_panic_hook();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_panic_info_extracts_message_and_location() {
        let orig_hook = std::panic::take_hook();
        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_clone = called.clone();
        std::panic::set_hook(Box::new(move |info| {
            let (msg, loc) = format_panic_info(info);
            assert_eq!(msg, "test panic payload");
            assert!(loc.contains("logging.rs"));
            called_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        }));

        let result = std::panic::catch_unwind(|| {
            panic!("test panic payload");
        });
        assert!(result.is_err());

        std::panic::set_hook(orig_hook);
        assert!(called.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn test_rolling_file_appender_creates_logs() {
        let temp = tempfile::tempdir().unwrap();
        let logs_dir = temp.path().join("logs");

        let appender = tracing_appender::rolling::daily(&logs_dir, "test.log");
        use std::io::Write;
        let mut writer = appender;
        writeln!(writer, "test log line").unwrap();

        assert!(logs_dir.exists());
        let entries: Vec<_> = std::fs::read_dir(&logs_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(entries.len(), 1);
        let file_name = entries[0].file_name().to_string_lossy().to_string();
        assert!(file_name.starts_with("test.log"));
    }
}
