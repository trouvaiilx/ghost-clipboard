//! Windows-specific integration.

pub mod clipboard;
pub mod foreground;
pub mod shell;

pub use shell::{Shell, ShellConfig, ShellError, StartupReport};
