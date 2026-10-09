//! Platform-neutral domain model: history entries, capture rules and policies.
//!
//! Nothing in this module touches the operating system, the database or the
//! GUI, so every rule here is covered by plain unit tests.

pub mod capture;
pub mod entry;
pub mod settings;
