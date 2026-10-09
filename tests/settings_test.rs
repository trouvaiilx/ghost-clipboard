use std::time::Duration;

use ghost_clipboard::app::{Message, Options, State, ViewTab};
use ghost_clipboard::domain::settings::{AppSettings, DedupMode, RetentionPolicy};
use ghost_clipboard::platform::Hotkey;
use ghost_clipboard::platform::windows::{Shell, ShellConfig};
use ghost_clipboard::storage::Storage;

fn create_test_shell() -> Shell {
    let config = ShellConfig {
        hotkey: Hotkey::DEFAULT,
    };
    Shell::spawn(config).expect("start shell")
}

#[test]
fn test_settings_persistence_roundtrip() {
    let temp = tempfile::tempdir().expect("create temp dir");
    let path = temp.path().join("subfolder").join("settings.json");

    let mut settings = AppSettings::default();
    settings.capture.max_entry_bytes = 5 * 1024 * 1024;
    settings.capture.dedup = DedupMode::LatestOnly;
    settings.capture.retention = RetentionPolicy {
        max_entries: Some(500),
        max_age: Some(Duration::from_secs(30 * 86_400)),
    };
    settings.excluded_apps.push("custom_vault.exe".to_string());
    settings.autostart = true;

    // Save creates directory and valid JSON
    settings.save(&path).expect("save settings");
    assert!(path.exists());

    let raw = std::fs::read_to_string(&path).expect("read settings file");
    assert!(raw.contains("custom_vault.exe"));
    assert!(raw.contains("latest_only"));

    // Reload matches completely
    let loaded = AppSettings::load_or_default(&path);
    assert_eq!(loaded, settings);
    assert_eq!(loaded.capture.retention.max_age_days(), Some(30));
}

#[test]
fn test_settings_load_fallback_on_corrupted_file() {
    let temp = tempfile::tempdir().expect("create temp dir");
    let path = temp.path().join("broken_settings.json");

    std::fs::write(&path, "{ invalid json content").expect("write invalid json");

    // Must not crash, should fall back to default
    let loaded = AppSettings::load_or_default(&path);
    assert_eq!(loaded, AppSettings::default());
}

#[tokio::test]
async fn test_settings_state_mutations_and_persistence() {
    let temp = tempfile::tempdir().expect("create temp dir");
    let settings_file = temp.path().join("test_settings.json");

    let storage = Storage::spawn_in_memory().expect("spawn in-memory storage");
    let shell = create_test_shell();

    let (mut state, _) = State::new(shell, storage, Options::default(), "in-memory".into());
    state.settings_path = settings_file.clone();

    // 1. Max entry bytes mutation
    let _ = state.update(Message::SetMaxEntryBytes(2 * 1024 * 1024));
    assert_eq!(state.app_settings.capture.max_entry_bytes, 2 * 1024 * 1024);
    assert!(settings_file.exists());

    // 2. Dedup mode mutation
    let _ = state.update(Message::SetDedupMode(DedupMode::LatestOnly));
    assert_eq!(state.app_settings.capture.dedup, DedupMode::LatestOnly);

    // 3. Retention max entries mutation
    let _ = state.update(Message::SetRetentionMaxEntries(Some(1000)));
    assert_eq!(state.app_settings.capture.retention.max_entries, Some(1000));

    // 4. Retention max age days mutation
    let _ = state.update(Message::SetRetentionMaxAgeDays(Some(90)));
    assert_eq!(
        state.app_settings.capture.retention.max_age_days(),
        Some(90)
    );

    // 5. Excluded app input and add with auto .exe appending
    let _ = state.update(Message::ExclusionInputChanged("secret_terminal".into()));
    assert_eq!(state.new_excluded_app_input, "secret_terminal");
    let _ = state.update(Message::AddExcludedApp);
    assert!(
        state
            .app_settings
            .excluded_apps
            .contains(&"secret_terminal.exe".to_string())
    );
    assert!(state.new_excluded_app_input.is_empty());

    // 6. Duplicate add is ignored
    let _ = state.update(Message::ExclusionInputChanged("SECRET_TERMINAL.EXE".into()));
    let _ = state.update(Message::AddExcludedApp);
    assert_eq!(
        state
            .app_settings
            .excluded_apps
            .iter()
            .filter(|a| a.as_str() == "secret_terminal.exe")
            .count(),
        1
    );

    // 7. Remove excluded app
    let _ = state.update(Message::RemoveExcludedApp("secret_terminal.exe".into()));
    assert!(
        !state
            .app_settings
            .excluded_apps
            .contains(&"secret_terminal.exe".to_string())
    );

    // 8. Verify disk file contains updated state
    let loaded = AppSettings::load_or_default(&settings_file);
    assert_eq!(loaded.capture.max_entry_bytes, 2 * 1024 * 1024);
    assert_eq!(loaded.capture.dedup, DedupMode::LatestOnly);
    assert_eq!(loaded.capture.retention.max_entries, Some(1000));
    assert_eq!(loaded.capture.retention.max_age_days(), Some(90));
}

#[tokio::test]
async fn test_settings_tab_view_rendering() {
    let storage = Storage::spawn_in_memory().expect("spawn in-memory storage");
    let shell = create_test_shell();

    let (mut state, _) = State::new(shell, storage, Options::default(), "in-memory".into());
    let _ = state.update(Message::SwitchTab(ViewTab::Settings));
    assert_eq!(state.active_tab, ViewTab::Settings);

    // Render Settings view
    {
        let _element = ghost_clipboard::app::view::view(&state);
    }

    // Toggle pause and verify rendering
    let _ = state.update(Message::PauseToggled(true));
    assert!(state.is_paused);
    {
        let _element_paused = ghost_clipboard::app::view::view(&state);
    }
}
