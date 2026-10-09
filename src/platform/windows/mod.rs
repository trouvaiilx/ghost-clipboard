//! Windows-specific integration.

pub mod autostart;
pub mod clipboard;
pub mod foreground;
pub mod shell;
pub mod single_instance;

pub use autostart::{is_autostart_enabled, set_autostart_enabled};
pub use shell::{Shell, ShellConfig, ShellError, StartupReport};
pub use single_instance::SingleInstance;
