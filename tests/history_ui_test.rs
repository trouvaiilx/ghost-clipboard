//! Integration tests for GhostClipboard history UI state, navigation, and rendering.

use ghost_clipboard::app::{EntryFilter, Message, Options, State, ViewTab};
use ghost_clipboard::domain::entry::{ContentKind, EntryId, EntrySummary};
use ghost_clipboard::platform::Hotkey;
use ghost_clipboard::platform::windows::{Shell, ShellConfig};
use ghost_clipboard::storage::Storage;
use iced::keyboard;

fn sample_summary(id: i64, preview: &str, pinned: bool, kind: ContentKind) -> EntrySummary {
    EntrySummary {
        id: EntryId(id),
        preview: preview.to_string(),
        truncated: false,
        char_count: preview.chars().count(),
        kind,
        copied_at: 1_700_000_000_000,
        pinned,
        source_app: Some("test_app.exe".to_string()),
    }
}

fn create_test_shell() -> Shell {
    let config = ShellConfig {
        hotkey: Hotkey::DEFAULT,
    };
    Shell::spawn(config).expect("start shell")
}

#[tokio::test]
async fn test_history_ui_navigation_and_clamping() {
    let storage = Storage::spawn_in_memory().expect("spawn in-memory storage");
    let shell = create_test_shell();

    let (mut state, _) = State::new(shell, storage, Options::default(), "in-memory".into());

    // Initially no entries
    assert_eq!(state.entries.len(), 0);
    assert_eq!(state.selected_index, None);

    // Navigating empty list remains None
    let _ = state.navigate_selection(1);
    assert_eq!(state.selected_index, None);

    // Populate with 3 entries
    let entries = vec![
        sample_summary(1, "First snippet", true, ContentKind::Text),
        sample_summary(2, "https://example.com", false, ContentKind::Url),
        sample_summary(3, "Third multiline\ncode snippet", false, ContentKind::Text),
    ];

    let _ = state.update(Message::EntriesLoaded(Ok(entries)));
    assert_eq!(state.entries.len(), 3);
    assert_eq!(state.selected_index, Some(0));
    assert_eq!(state.selected_entry().unwrap().id, EntryId(1));

    // Navigate down
    let _ = state.navigate_selection(1);
    assert_eq!(state.selected_index, Some(1));
    assert_eq!(state.selected_entry().unwrap().id, EntryId(2));

    // Navigate down again
    let _ = state.navigate_selection(1);
    assert_eq!(state.selected_index, Some(2));
    assert_eq!(state.selected_entry().unwrap().id, EntryId(3));

    // Clamp at the bottom
    let _ = state.navigate_selection(10);
    assert_eq!(state.selected_index, Some(2));

    // Navigate up
    let _ = state.navigate_selection(-1);
    assert_eq!(state.selected_index, Some(1));

    // Clamp at the top
    let _ = state.navigate_selection(-10);
    assert_eq!(state.selected_index, Some(0));
}

#[tokio::test]
async fn test_history_ui_search_and_clear_flow() {
    let storage = Storage::spawn_in_memory().expect("spawn in-memory storage");
    let shell = create_test_shell();

    let (mut state, _) = State::new(shell, storage, Options::default(), "in-memory".into());

    // Search query update
    let _ = state.update(Message::SearchInputChanged("rust snippet".into()));
    assert_eq!(state.search_query, "rust snippet");

    // Clear search
    let _ = state.update(Message::ClearSearch);
    assert_eq!(state.search_query, "");

    // Clear confirmation flow
    assert!(!state.confirm_clear);
    let _ = state.update(Message::RequestClearHistory);
    assert!(state.confirm_clear);

    let _ = state.update(Message::CancelClearHistory);
    assert!(!state.confirm_clear);
}

#[tokio::test]
async fn test_history_ui_filtering_and_tabs() {
    let storage = Storage::spawn_in_memory().expect("spawn in-memory storage");
    let shell = create_test_shell();

    let (mut state, _) = State::new(shell, storage, Options::default(), "in-memory".into());

    assert_eq!(state.entry_filter, EntryFilter::All);
    assert_eq!(state.active_tab, ViewTab::History);

    let _ = state.update(Message::FilterChanged(EntryFilter::PinnedOnly));
    assert_eq!(state.entry_filter, EntryFilter::PinnedOnly);

    let _ = state.update(Message::SwitchTab(ViewTab::Settings));
    assert_eq!(state.active_tab, ViewTab::Settings);

    let _ = state.update(Message::SwitchTab(ViewTab::History));
    assert_eq!(state.active_tab, ViewTab::History);
}

#[tokio::test]
async fn test_history_ui_keyboard_events() {
    let storage = Storage::spawn_in_memory().expect("spawn in-memory storage");
    let shell = create_test_shell();

    // Start in background (hidden)
    let options = Options {
        background: true,
        db_path: None,
    };
    let (mut state, _) = State::new(shell, storage, options, "in-memory".into());

    let entries = vec![
        sample_summary(1, "Alpha", false, ContentKind::Text),
        sample_summary(2, "Beta", false, ContentKind::Text),
    ];
    let _ = state.update(Message::EntriesLoaded(Ok(entries)));

    let down_event = keyboard::Event::KeyPressed {
        key: keyboard::Key::Named(keyboard::key::Named::ArrowDown),
        modified_key: keyboard::Key::Named(keyboard::key::Named::ArrowDown),
        physical_key: keyboard::key::Physical::Code(keyboard::key::Code::ArrowDown),
        location: keyboard::Location::Standard,
        modifiers: keyboard::Modifiers::default(),
        repeat: false,
        text: None,
    };

    // When window is hidden, keyboard event is ignored
    let _ = state.update(Message::KeyboardEvent(down_event.clone()));
    assert_eq!(state.selected_index, Some(0));

    // Show window via HotkeyPressed
    let _ = state.update(Message::Shell(
        ghost_clipboard::platform::ShellEvent::HotkeyPressed,
    ));

    // Now when visible, keyboard event navigates down
    let _ = state.update(Message::KeyboardEvent(down_event));
    assert_eq!(state.selected_index, Some(1));

    // Test Escape when search query has text
    let _ = state.update(Message::SearchInputChanged("filter".into()));
    assert_eq!(state.search_query, "filter");

    let escape_event = keyboard::Event::KeyPressed {
        key: keyboard::Key::Named(keyboard::key::Named::Escape),
        modified_key: keyboard::Key::Named(keyboard::key::Named::Escape),
        physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Escape),
        location: keyboard::Location::Standard,
        modifiers: keyboard::Modifiers::default(),
        repeat: false,
        text: None,
    };

    let _ = state.update(Message::KeyboardEvent(escape_event));
    // Escape should have cleared search query first
    assert_eq!(state.search_query, "");
}

#[tokio::test]
async fn test_history_ui_view_rendering() {
    let storage = Storage::spawn_in_memory().expect("spawn in-memory storage");
    let shell = create_test_shell();

    let (mut state, _) = State::new(shell, storage, Options::default(), "in-memory".into());

    let entries = vec![
        sample_summary(1, "Alpha snippet", true, ContentKind::Text),
        sample_summary(2, "https://github.com", false, ContentKind::Url),
    ];
    let _ = state.update(Message::EntriesLoaded(Ok(entries)));

    let win_id = iced::window::Id::unique();

    // Render history view
    {
        let _element = state.view(win_id);
    }

    // Switch to Settings tab and render
    let _ = state.update(Message::SwitchTab(ViewTab::Settings));
    {
        let _settings_element = state.view(win_id);
    }

    // Switch to confirmation view and render
    let _ = state.update(Message::SwitchTab(ViewTab::History));
    let _ = state.update(Message::RequestClearHistory);
    {
        let _confirm_element = state.view(win_id);
    }
}

#[tokio::test]
async fn test_settings_keyboard_isolation_and_tab_switching() {
    let storage = Storage::spawn_in_memory().expect("spawn in-memory storage");
    let shell = create_test_shell();

    let (mut state, _) = State::new(shell, storage, Options::default(), "in-memory".into());
    // Simulate visible window
    state.visible = true;
    let entries = vec![
        sample_summary(1, "First snippet", true, ContentKind::Text),
        sample_summary(2, "Second snippet", false, ContentKind::Text),
    ];
    let _ = state.update(Message::EntriesLoaded(Ok(entries)));
    state.selected_index = Some(0);

    // Switch to Settings tab
    let _ = state.update(Message::SwitchTab(ViewTab::Settings));
    assert_eq!(state.active_tab, ViewTab::Settings);

    // 1. ArrowDown should not navigate history on Settings tab
    let down_event = keyboard::Event::KeyPressed {
        key: keyboard::Key::Named(keyboard::key::Named::ArrowDown),
        modified_key: keyboard::Key::Named(keyboard::key::Named::ArrowDown),
        physical_key: keyboard::key::Physical::Code(keyboard::key::Code::ArrowDown),
        location: keyboard::Location::Standard,
        modifiers: keyboard::Modifiers::default(),
        repeat: false,
        text: None,
    };
    let _ = state.update(Message::KeyboardEvent(down_event));
    assert_eq!(state.selected_index, Some(0));

    // 2. Delete should not delete history entries on Settings tab
    let del_event = keyboard::Event::KeyPressed {
        key: keyboard::Key::Named(keyboard::key::Named::Delete),
        modified_key: keyboard::Key::Named(keyboard::key::Named::Delete),
        physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Delete),
        location: keyboard::Location::Standard,
        modifiers: keyboard::Modifiers::default(),
        repeat: false,
        text: None,
    };
    let _ = state.update(Message::KeyboardEvent(del_event));
    assert_eq!(state.entries.len(), 2);

    // 3. Escape on Settings tab should switch back to History tab
    let escape_event = keyboard::Event::KeyPressed {
        key: keyboard::Key::Named(keyboard::key::Named::Escape),
        modified_key: keyboard::Key::Named(keyboard::key::Named::Escape),
        physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Escape),
        location: keyboard::Location::Standard,
        modifiers: keyboard::Modifiers::default(),
        repeat: false,
        text: None,
    };
    let _ = state.update(Message::KeyboardEvent(escape_event));
    assert_eq!(state.active_tab, ViewTab::History);

    // 4. Ctrl+2 switches to Settings tab
    let ctrl_2_event = keyboard::Event::KeyPressed {
        key: keyboard::Key::Character("2".into()),
        modified_key: keyboard::Key::Character("2".into()),
        physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Digit2),
        location: keyboard::Location::Standard,
        modifiers: keyboard::Modifiers::CTRL,
        repeat: false,
        text: None,
    };
    let _ = state.update(Message::KeyboardEvent(ctrl_2_event));
    assert_eq!(state.active_tab, ViewTab::Settings);

    // 5. Ctrl+1 switches back to History tab
    let ctrl_1_event = keyboard::Event::KeyPressed {
        key: keyboard::Key::Character("1".into()),
        modified_key: keyboard::Key::Character("1".into()),
        physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Digit1),
        location: keyboard::Location::Standard,
        modifiers: keyboard::Modifiers::CTRL,
        repeat: false,
        text: None,
    };
    let _ = state.update(Message::KeyboardEvent(ctrl_1_event));
    assert_eq!(state.active_tab, ViewTab::History);
}

#[tokio::test]
async fn test_open_url_flow_and_validation() {
    let storage = Storage::spawn_in_memory().expect("spawn in-memory storage");
    let handle = storage.handle();
    let shell = create_test_shell();

    let (mut state, _) = State::new(shell, storage, Options::default(), "in-memory".into());

    // Record a long URL exceeding 400 characters
    let long_url = format!("https://example.com/search?query={}", "a".repeat(450));
    let entry = ghost_clipboard::domain::capture::prepare(
        long_url.clone(),
        &ghost_clipboard::domain::settings::CaptureSettings::default(),
        None,
        1_000,
    )
    .unwrap();
    let outcome = handle
        .record(
            entry,
            ghost_clipboard::domain::settings::DedupMode::Anywhere,
        )
        .await
        .unwrap();
    let id = match outcome {
        ghost_clipboard::storage::RecordOutcome::Inserted(id) => id,
        _ => panic!(),
    };

    // Trigger Message::OpenUrl(id)
    let _ = state.update(Message::OpenUrl(id));
    // Verify handling UrlOpened success
    let _ = state.update(Message::UrlOpened(Ok(())));
    assert_eq!(
        state.status_feedback,
        Some(("Opening link in browser...".to_string(), false))
    );

    // Verify error feedback
    let _ = state.update(Message::UrlOpened(Err(
        "Invalid URL: must start with http:// or https://".to_string(),
    )));
    assert_eq!(
        state.status_feedback,
        Some((
            "Could not open link: Invalid URL: must start with http:// or https://".to_string(),
            true
        ))
    );
}

#[tokio::test]
async fn test_overflow_event_triggers_clipboard_capture() {
    let storage = Storage::spawn_in_memory().expect("spawn storage");
    let shell = create_test_shell();
    let (mut state, _) = State::new(shell, storage, Options::default(), "in-memory".into());

    assert_eq!(state.clipboard_notifications, 0);

    let _ = state.update(Message::Shell(
        ghost_clipboard::platform::ShellEvent::Overflow(5),
    ));

    assert_eq!(state.clipboard_notifications, 1);
}
