//! Iced application: state, messages, and native shell / service integration.

use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use iced::futures::stream::{self, BoxStream, StreamExt};
use iced::window;
use iced::{Element, Size, Subscription, Task, Theme, keyboard};
use tokio::sync::{Mutex as TokioMutex, mpsc};

pub mod theme;
pub mod time;
pub mod view;

use crate::branding;
use crate::domain::entry::{EntryId, EntrySummary};
use crate::domain::settings::{AppSettings, DedupMode};
use crate::platform::windows::Shell;
use crate::platform::{self, Component, ShellEvent, TrayCommand};
use crate::services::CaptureService;
use crate::storage::repository::{HistoryQuery, OpenReport, RecordOutcome};
use crate::storage::{Storage, StorageHandle};

/// Widget ID for the search input so it can be focused programmatically.
pub const SEARCH_INPUT_ID: &str = "ghostclipboard_search_input";
/// Widget ID for the scrollable history list.
pub const HISTORY_SCROLL_ID: &str = "ghostclipboard_history_scroll";

/// Command-line derived start-up options.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Start in the tray without showing the history window.
    pub background: bool,
    /// Custom database path (or None for default %APPDATA%/GhostClipboard/history.db).
    pub db_path: Option<PathBuf>,
}

/// Run the application until the user chooses *Exit*.
pub fn run(shell: Shell, storage: Storage, options: Options) -> iced::Result {
    let db_path_display = options
        .db_path
        .clone()
        .or_else(|| crate::storage::worker::default_db_path().ok())
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "in-memory / unavailable".to_string());

    let boot = Mutex::new(Some((shell, storage, options, db_path_display)));

    iced::daemon(
        move || {
            let (shell, storage, options, db_path) = boot
                .lock()
                .expect("boot lock poisoned")
                .take()
                .expect("boot is only invoked once");
            State::new(shell, storage, options, db_path)
        },
        State::update,
        State::view,
    )
    .title(|_: &State, _| String::from("GhostClipboard"))
    .subscription(State::subscription)
    .theme(|_: &State, _| Theme::Dark)
    .run()
}

/// Filter for entries displayed in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EntryFilter {
    #[default]
    All,
    PinnedOnly,
}

/// Active top-level view tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewTab {
    #[default]
    History,
    Settings,
}

pub struct State {
    shell: Shell,
    storage: StorageHandle,
    capture: Arc<TokioMutex<CaptureService>>,
    events: EventSource,
    window: Option<window::Id>,
    pub visible: bool,
    previous_window: Option<isize>,

    // UI state
    pub search_query: String,
    pub search_epoch: u64,
    pub query_limit: usize,
    pub entry_filter: EntryFilter,
    pub active_tab: ViewTab,
    pub entries: Vec<EntrySummary>,
    pub selected_index: Option<usize>,
    pub selected_full_text: Option<String>,
    pub is_loading: bool,
    pub confirm_clear: bool,
    pub status_feedback: Option<(String, bool)>,

    // Settings & privacy state
    pub app_settings: AppSettings,
    pub settings_path: PathBuf,
    pub new_excluded_app_input: String,

    // Diagnostics & service state
    pub clipboard_notifications: u64,
    pub entries_recorded: usize,
    pub pinned_count: usize,
    pub hotkey_presses: u64,
    pub is_paused: bool,
    pub autostart_enabled: bool,
    pub db_path: String,
    pub faults: Vec<(Component, String)>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Shell(ShellEvent),
    WindowOpened,
    CloseRequested(window::Id),
    WindowEvent(window::Id, window::Event),
    StatsLoaded(Result<OpenReport, String>),
    ClipboardCaptured(Result<Option<RecordOutcome>, String>),

    // UI interactions
    SearchInputChanged(String),
    DebouncedSearch(u64, String),
    ClearSearch,
    EntriesLoaded(Result<Vec<EntrySummary>, String>),
    SelectedTextLoaded {
        id: EntryId,
        text: Result<Option<String>, String>,
    },
    SelectEntry(usize),
    FilterChanged(EntryFilter),
    CopyEntry(EntryId),
    EntryCopied(Result<(), String>),
    TogglePin(EntryId),
    PinToggled(Result<Option<bool>, String>),
    DeleteEntry(EntryId),
    EntryDeleted(Result<bool, String>),
    RequestClearHistory,
    CancelClearHistory,
    ClearUnpinned,
    ClearAll,
    HistoryCleared(Result<usize, String>),
    TogglePause,
    PauseToggled(bool),
    ToggleAutostart,
    SwitchTab(ViewTab),
    KeyboardEvent(keyboard::Event),
    DismissFeedback,
    OpenUrl(EntryId),
    UrlOpened(Result<(), String>),
    ToggleCloseOnBlur,
    LoadMoreEntries,

    // Settings actions
    SetMaxEntryBytes(usize),
    SetDedupMode(DedupMode),
    SetRetentionMaxEntries(Option<usize>),
    SetRetentionMaxAgeDays(Option<u64>),
    ExclusionInputChanged(String),
    AddExcludedApp,
    RemoveExcludedApp(String),
}

impl State {
    pub fn new(
        mut shell: Shell,
        storage: Storage,
        options: Options,
        db_path: String,
    ) -> (Self, Task<Message>) {
        let events = EventSource(Arc::new(Mutex::new(shell.take_events())));
        let faults: Vec<(Component, String)> = shell
            .report()
            .faults
            .iter()
            .map(|f| (f.component, f.message.clone()))
            .collect();
        for (component, message) in &faults {
            tracing::warn!(?component, %message, "native component unavailable");
        }
        tracing::info!("native shell started");

        let storage_handle = storage.handle();

        let settings_path = crate::storage::default_settings_path()
            .unwrap_or_else(|_| PathBuf::from("settings.json"));
        let (mut app_settings, settings_notice) = AppSettings::load_with_notice(&settings_path);
        let autostart_enabled = platform::is_autostart_enabled().unwrap_or(false);
        app_settings.autostart = autostart_enabled;

        let mut capture_service =
            CaptureService::new(storage_handle.clone(), app_settings.capture.clone());
        capture_service.set_excluded_apps(app_settings.excluded_apps.clone());
        let capture = Arc::new(TokioMutex::new(capture_service));

        let mut state = Self {
            shell,
            storage: storage_handle.clone(),
            capture,
            events,
            window: None,
            visible: false,
            previous_window: None,

            search_query: String::new(),
            search_epoch: 0,
            query_limit: 100,
            entry_filter: EntryFilter::All,
            active_tab: ViewTab::History,
            entries: Vec::new(),
            selected_index: None,
            selected_full_text: None,
            is_loading: true,
            confirm_clear: false,
            status_feedback: settings_notice.map(|msg| (msg, true)),

            app_settings,
            settings_path,
            new_excluded_app_input: String::new(),

            clipboard_notifications: 0,
            entries_recorded: 0,
            pinned_count: 0,
            hotkey_presses: 0,
            is_paused: false,
            autostart_enabled,
            db_path,
            faults,
        };

        let initial_show = if options.background {
            Task::none()
        } else {
            state.show()
        };

        let initial_load = state.reload_entries();
        let load_stats = Task::perform(
            async move { storage_handle.report().await.map_err(|e| e.to_string()) },
            Message::StatsLoaded,
        );

        (state, Task::batch([initial_show, initial_load, load_stats]))
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            Subscription::run_with(self.events.clone(), EventSource::stream),
            window::close_requests().map(Message::CloseRequested),
            window::events().map(|(id, event)| Message::WindowEvent(id, event)),
            keyboard::listen().map(Message::KeyboardEvent),
        ])
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Shell(event) => self.on_shell_event(event),
            Message::WindowOpened => {
                iced::widget::operation::focus(iced::widget::Id::new(SEARCH_INPUT_ID))
            }
            Message::CloseRequested(id) => {
                if self.window == Some(id) {
                    self.hide()
                } else {
                    Task::none()
                }
            }
            Message::WindowEvent(_id, event) => match event {
                window::Event::Unfocused => {
                    if self.app_settings.close_on_blur && self.visible {
                        self.hide()
                    } else {
                        Task::none()
                    }
                }
                _ => Task::none(),
            },
            Message::SearchInputChanged(query) => {
                self.search_query = query.clone();
                self.search_epoch += 1;
                let epoch = self.search_epoch;
                if query.trim().is_empty() {
                    self.reload_entries()
                } else {
                    Task::perform(
                        async move {
                            tokio::time::sleep(std::time::Duration::from_millis(80)).await;
                            (epoch, query)
                        },
                        |(epoch, query)| Message::DebouncedSearch(epoch, query),
                    )
                }
            }
            Message::DebouncedSearch(epoch, _query) => {
                if self.search_epoch == epoch {
                    self.reload_entries()
                } else {
                    Task::none()
                }
            }
            Message::ClearSearch => {
                self.search_query.clear();
                self.reload_entries()
            }
            Message::EntriesLoaded(Ok(entries)) => {
                self.is_loading = false;
                self.entries = entries;
                if self.entries.is_empty() {
                    self.selected_index = None;
                    self.selected_full_text = None;
                    Task::none()
                } else {
                    let idx = if self.search_query.trim().is_empty() {
                        self.selected_index.unwrap_or(0).min(self.entries.len() - 1)
                    } else {
                        0
                    };
                    self.select_index(idx)
                }
            }
            Message::EntriesLoaded(Err(err)) => {
                self.is_loading = false;
                self.set_status_feedback(format!("Failed to load entries: {err}"), true)
            }
            Message::SelectedTextLoaded { id, text } => {
                if let Some(entry) = self.selected_entry()
                    && entry.id == id
                {
                    match text {
                        Ok(t) => self.selected_full_text = t,
                        Err(e) => {
                            tracing::error!(%e, "failed to load full text for entry");
                        }
                    }
                }
                Task::none()
            }
            Message::SelectEntry(index) => self.select_index(index),
            Message::FilterChanged(filter) => {
                self.entry_filter = filter;
                Task::none()
            }
            Message::CopyEntry(id) => self.perform_copy(id),
            Message::EntryCopied(Ok(())) => {
                self.set_status_feedback("Copied to clipboard".to_string(), false)
            }
            Message::EntryCopied(Err(err)) => {
                self.set_status_feedback(format!("Copy failed: {err}"), true)
            }
            Message::TogglePin(id) => {
                let storage = self.storage.clone();
                Task::perform(
                    async move { storage.toggle_pin(id).await.map_err(|e| e.to_string()) },
                    Message::PinToggled,
                )
            }
            Message::PinToggled(Ok(new_pin)) => {
                let msg = match new_pin {
                    Some(true) => "Entry pinned",
                    Some(false) => "Entry unpinned",
                    None => "Entry not found",
                };
                let feedback_task = self.set_status_feedback(msg.to_string(), false);
                Task::batch([feedback_task, self.refresh_stats_and_entries()])
            }
            Message::PinToggled(Err(err)) => {
                self.set_status_feedback(format!("Pin toggle failed: {err}"), true)
            }
            Message::DeleteEntry(id) => {
                let storage = self.storage.clone();
                Task::perform(
                    async move { storage.delete(id).await.map_err(|e| e.to_string()) },
                    Message::EntryDeleted,
                )
            }
            Message::EntryDeleted(Ok(deleted)) => {
                let feedback_task = if deleted {
                    self.set_status_feedback("Entry deleted".to_string(), false)
                } else {
                    Task::none()
                };
                Task::batch([feedback_task, self.refresh_stats_and_entries()])
            }
            Message::EntryDeleted(Err(err)) => {
                self.set_status_feedback(format!("Delete failed: {err}"), true)
            }
            Message::RequestClearHistory => {
                self.confirm_clear = true;
                Task::none()
            }
            Message::CancelClearHistory => {
                self.confirm_clear = false;
                Task::none()
            }
            Message::ClearUnpinned => {
                self.confirm_clear = false;
                let storage = self.storage.clone();
                Task::perform(
                    async move { storage.clear_unpinned().await.map_err(|e| e.to_string()) },
                    Message::HistoryCleared,
                )
            }
            Message::ClearAll => {
                self.confirm_clear = false;
                let storage = self.storage.clone();
                Task::perform(
                    async move { storage.clear_all().await.map_err(|e| e.to_string()) },
                    Message::HistoryCleared,
                )
            }
            Message::HistoryCleared(Ok(count)) => {
                let feedback_task =
                    self.set_status_feedback(format!("Cleared {count} entries"), false);
                Task::batch([feedback_task, self.refresh_stats_and_entries()])
            }
            Message::HistoryCleared(Err(err)) => {
                self.set_status_feedback(format!("Clear history failed: {err}"), true)
            }
            Message::TogglePause => {
                let capture = self.capture.clone();
                Task::perform(
                    async move {
                        let mut guard = capture.lock().await;
                        guard.toggle_paused()
                    },
                    Message::PauseToggled,
                )
            }
            Message::PauseToggled(paused) => {
                self.is_paused = paused;
                let msg = if paused {
                    "Clipboard capture paused"
                } else {
                    "Clipboard capture resumed"
                };
                self.set_status_feedback(msg.to_string(), false)
            }
            Message::ToggleAutostart => {
                let new_val = !self.autostart_enabled;
                match platform::set_autostart_enabled(new_val) {
                    Ok(()) => {
                        self.autostart_enabled = new_val;
                        self.app_settings.autostart = new_val;
                        self.persist_settings();
                        let msg = if new_val {
                            "Launch-at-login enabled"
                        } else {
                            "Launch-at-login disabled"
                        };
                        self.set_status_feedback(msg.to_string(), false)
                    }
                    Err(err) => {
                        self.set_status_feedback(format!("Failed to update autostart: {err}"), true)
                    }
                }
            }
            Message::SwitchTab(tab) => {
                self.active_tab = tab;
                Task::none()
            }
            Message::SetMaxEntryBytes(bytes) => {
                self.app_settings.capture.max_entry_bytes = bytes;
                self.persist_settings();
                Task::none()
            }
            Message::SetDedupMode(mode) => {
                self.app_settings.capture.dedup = mode;
                self.persist_settings();
                Task::none()
            }
            Message::SetRetentionMaxEntries(entries) => {
                self.app_settings.capture.retention.max_entries = entries;
                self.persist_settings();
                Task::none()
            }
            Message::SetRetentionMaxAgeDays(days) => {
                self.app_settings.capture.retention.max_age =
                    days.map(|d| std::time::Duration::from_secs(d * 86_400));
                self.persist_settings();
                Task::none()
            }
            Message::OpenUrl(id) => {
                let storage = self.storage.clone();
                Task::perform(
                    async move {
                        let text = storage
                            .get_text(id)
                            .await
                            .map_err(|e| e.to_string())?
                            .ok_or_else(|| "Entry not found".to_string())?;

                        let trimmed = text.trim();
                        if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
                            return Err(
                                "Invalid URL: must start with http:// or https://".to_string()
                            );
                        }

                        platform::open_url(trimmed)
                    },
                    Message::UrlOpened,
                )
            }
            Message::UrlOpened(Ok(())) => {
                self.set_status_feedback("Opening link in browser...".to_string(), false)
            }
            Message::UrlOpened(Err(err)) => {
                self.set_status_feedback(format!("Could not open link: {err}"), true)
            }
            Message::ToggleCloseOnBlur => {
                self.app_settings.close_on_blur = !self.app_settings.close_on_blur;
                self.persist_settings();
                let msg = if self.app_settings.close_on_blur {
                    "Close-on-focus-loss enabled"
                } else {
                    "Close-on-focus-loss disabled"
                };
                self.set_status_feedback(msg.to_string(), false)
            }
            Message::LoadMoreEntries => {
                self.query_limit += 100;
                self.reload_entries()
            }
            Message::ExclusionInputChanged(text) => {
                self.new_excluded_app_input = text;
                Task::none()
            }
            Message::AddExcludedApp => {
                let trimmed = self.new_excluded_app_input.trim().to_ascii_lowercase();
                if !trimmed.is_empty() {
                    let exe_name = if !trimmed.ends_with(".exe") && !trimmed.contains('.') {
                        format!("{trimmed}.exe")
                    } else {
                        trimmed
                    };
                    if !self
                        .app_settings
                        .excluded_apps
                        .iter()
                        .any(|a| a.eq_ignore_ascii_case(&exe_name))
                    {
                        self.app_settings.excluded_apps.push(exe_name);
                        self.new_excluded_app_input.clear();
                        self.persist_settings();
                    }
                }
                Task::none()
            }
            Message::RemoveExcludedApp(app) => {
                self.app_settings
                    .excluded_apps
                    .retain(|a| !a.eq_ignore_ascii_case(&app));
                self.persist_settings();
                Task::none()
            }
            Message::StatsLoaded(Ok(report)) => {
                self.entries_recorded = report.total_entries;
                self.pinned_count = report.pinned_entries;
                Task::none()
            }
            Message::StatsLoaded(Err(err)) => {
                tracing::error!(%err, "failed to load storage report");
                Task::none()
            }
            Message::ClipboardCaptured(Ok(Some(outcome))) => {
                match outcome {
                    RecordOutcome::Inserted(_) => {
                        self.entries_recorded += 1;
                        tracing::debug!("new clipboard entry captured and persisted");
                    }
                    RecordOutcome::Deduplicated { moved_to_top, .. } => {
                        tracing::debug!(moved_to_top, "clipboard entry deduplicated");
                    }
                }
                if self.visible {
                    self.reload_entries()
                } else {
                    Task::none()
                }
            }
            Message::ClipboardCaptured(Ok(None)) => Task::none(),
            Message::ClipboardCaptured(Err(err)) => {
                tracing::error!(%err, "error during clipboard capture processing");
                Task::none()
            }
            Message::KeyboardEvent(event) => self.handle_keyboard_event(event),
            Message::DismissFeedback => {
                self.status_feedback = None;
                Task::none()
            }
        }
    }

    fn on_shell_event(&mut self, event: ShellEvent) -> Task<Message> {
        tracing::debug!(?event, "shell event");
        match event {
            ShellEvent::ClipboardChanged => {
                self.clipboard_notifications += 1;
                let capture = self.capture.clone();
                Task::perform(
                    async move {
                        let mut guard = capture.lock().await;
                        guard
                            .on_clipboard_changed()
                            .await
                            .map_err(|e| e.to_string())
                    },
                    Message::ClipboardCaptured,
                )
            }
            ShellEvent::HotkeyPressed => {
                self.hotkey_presses += 1;
                self.toggle()
            }
            ShellEvent::TrayActivated | ShellEvent::Menu(TrayCommand::OpenHistory) => self.show(),
            ShellEvent::Menu(TrayCommand::TogglePause) => {
                let capture = self.capture.clone();
                Task::perform(
                    async move {
                        let mut guard = capture.lock().await;
                        guard.toggle_paused()
                    },
                    Message::PauseToggled,
                )
            }
            ShellEvent::Menu(TrayCommand::Exit) => {
                self.shell.shutdown();
                iced::exit()
            }
            ShellEvent::Menu(TrayCommand::OpenSettings) => {
                self.active_tab = ViewTab::Settings;
                self.show()
            }
            ShellEvent::Overflow(dropped) => {
                tracing::warn!(dropped, "shell events were dropped");
                Task::none()
            }
        }
    }

    fn handle_keyboard_event(&mut self, event: keyboard::Event) -> Task<Message> {
        let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
            return Task::none();
        };

        if !self.visible {
            return Task::none();
        }

        // Global tab shortcuts
        if modifiers.control() {
            match key {
                keyboard::Key::Character(ref c) if c == "1" => {
                    self.active_tab = ViewTab::History;
                    return iced::widget::operation::focus(iced::widget::Id::new(SEARCH_INPUT_ID));
                }
                keyboard::Key::Character(ref c) if c == "2" => {
                    self.active_tab = ViewTab::Settings;
                    return Task::none();
                }
                _ => {}
            }
        }

        match self.active_tab {
            ViewTab::Settings => match key {
                keyboard::Key::Named(keyboard::key::Named::Escape) => {
                    self.active_tab = ViewTab::History;
                    iced::widget::operation::focus(iced::widget::Id::new(SEARCH_INPUT_ID))
                }
                _ => Task::none(),
            },
            ViewTab::History => match key {
                keyboard::Key::Named(keyboard::key::Named::Escape) => {
                    if self.confirm_clear {
                        self.confirm_clear = false;
                        Task::none()
                    } else if !self.search_query.is_empty() {
                        self.search_query.clear();
                        self.reload_entries()
                    } else {
                        self.hide()
                    }
                }
                keyboard::Key::Named(keyboard::key::Named::ArrowDown) => self.navigate_selection(1),
                keyboard::Key::Named(keyboard::key::Named::ArrowUp) => self.navigate_selection(-1),
                keyboard::Key::Named(keyboard::key::Named::Enter) => {
                    if let Some(entry) = self.selected_entry() {
                        return self.perform_copy(entry.id);
                    }
                    Task::none()
                }
                keyboard::Key::Named(keyboard::key::Named::Delete) => {
                    // Plain Delete only deletes an entry if the search query is empty or with modifiers,
                    // preventing accidental entry deletion during inline search text editing.
                    let can_delete =
                        modifiers.shift() || modifiers.control() || self.search_query.is_empty();
                    if let Some(entry) = self.selected_entry().filter(|_| can_delete) {
                        let id = entry.id;
                        return self.update(Message::DeleteEntry(id));
                    }
                    Task::none()
                }
                keyboard::Key::Character(ref c)
                    if modifiers.control() && (c.eq_ignore_ascii_case("p")) =>
                {
                    if let Some(entry) = self.selected_entry() {
                        let id = entry.id;
                        return self.update(Message::TogglePin(id));
                    }
                    Task::none()
                }
                keyboard::Key::Character(ref c)
                    if modifiers.control() && (c.eq_ignore_ascii_case("d")) =>
                {
                    if let Some(entry) = self.selected_entry() {
                        let id = entry.id;
                        return self.update(Message::DeleteEntry(id));
                    }
                    Task::none()
                }
                _ => Task::none(),
            },
        }
    }

    pub fn selected_entry(&self) -> Option<&EntrySummary> {
        self.selected_index.and_then(|idx| self.entries.get(idx))
    }

    pub fn set_status_feedback(&mut self, text: String, is_err: bool) -> Task<Message> {
        self.status_feedback = Some((text, is_err));
        Task::perform(
            async {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            },
            |_| Message::DismissFeedback,
        )
    }

    pub fn persist_settings(&mut self) {
        if let Err(e) = self.app_settings.save(&self.settings_path) {
            tracing::error!(%e, "failed to save settings to disk");
            self.status_feedback = Some((format!("Failed to save settings: {e}"), true));
        } else {
            self.status_feedback = Some(("Settings saved".to_string(), false));
        }

        let capture = self.capture.clone();
        let capture_settings = self.app_settings.capture.clone();
        let excluded_apps = self.app_settings.excluded_apps.clone();
        tokio::spawn(async move {
            let mut guard = capture.lock().await;
            guard.set_settings(capture_settings);
            guard.set_excluded_apps(excluded_apps);
        });
    }

    pub fn navigate_selection(&mut self, delta: isize) -> Task<Message> {
        let count = self.entries.len();
        if count == 0 {
            self.selected_index = None;
            return Task::none();
        }

        let current = self.selected_index.unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, count as isize - 1) as usize;
        self.select_index(next)
    }

    pub fn select_index(&mut self, index: usize) -> Task<Message> {
        if index >= self.entries.len() {
            self.selected_index = None;
            self.selected_full_text = None;
            return Task::none();
        }

        self.selected_index = Some(index);
        let id = self.entries[index].id;
        let storage = self.storage.clone();

        let load_task = Task::perform(
            async move {
                let text = storage.get_text(id).await.map_err(|e| e.to_string())?;
                Ok(text)
            },
            move |text| Message::SelectedTextLoaded { id, text },
        );

        let scroll_y = if index > 2 {
            (index as f32 - 1.5) * 80.0
        } else {
            0.0
        };
        let scroll_task = iced::widget::operation::scroll_to(
            iced::widget::Id::new(HISTORY_SCROLL_ID),
            iced::widget::operation::AbsoluteOffset {
                x: None,
                y: Some(scroll_y),
            },
        );

        Task::batch([load_task, scroll_task])
    }

    fn perform_copy(&mut self, id: EntryId) -> Task<Message> {
        let storage = self.storage.clone();
        let capture = self.capture.clone();
        let previous_window = self.previous_window.take();

        let hide_task = self.hide();

        let copy_task = Task::perform(
            async move {
                let text = storage
                    .get_text(id)
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "Entry not found".to_string())?;

                let seq = platform::write_clipboard_text(&text)?;
                {
                    let mut cap = capture.lock().await;
                    cap.ignore_sequence(seq);
                }

                if let Some(handle) = previous_window {
                    platform::restore_active_window(handle);
                }

                Ok(())
            },
            Message::EntryCopied,
        );

        Task::batch([hide_task, copy_task])
    }

    fn reload_entries(&mut self) -> Task<Message> {
        self.is_loading = true;
        let storage = self.storage.clone();
        let query_term = if self.search_query.trim().is_empty() {
            None
        } else {
            Some(self.search_query.trim().to_string())
        };
        let limit = self.query_limit;

        Task::perform(
            async move {
                let query = HistoryQuery {
                    search_term: query_term,
                    limit,
                    offset: 0,
                };
                storage.list(query).await.map_err(|e| e.to_string())
            },
            Message::EntriesLoaded,
        )
    }

    fn refresh_stats_and_entries(&mut self) -> Task<Message> {
        let reload = self.reload_entries();
        let storage = self.storage.clone();
        let report = Task::perform(
            async move { storage.report().await.map_err(|e| e.to_string()) },
            Message::StatsLoaded,
        );
        Task::batch([reload, report])
    }

    #[allow(dead_code)]
    fn apply_retention_and_reload(&mut self) -> Task<Message> {
        let storage = self.storage.clone();
        let policy = self.app_settings.capture.retention;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let reload = self.reload_entries();
        let report = Task::perform(
            async move {
                let _ = storage.apply_retention(policy, now_ms).await;
                storage.report().await.map_err(|e| e.to_string())
            },
            Message::StatsLoaded,
        );
        Task::batch([reload, report])
    }

    fn toggle(&mut self) -> Task<Message> {
        if self.visible {
            self.hide()
        } else {
            self.show()
        }
    }

    fn show(&mut self) -> Task<Message> {
        self.visible = true;
        self.previous_window = platform::active_window_handle();

        let reload = self.reload_entries();
        let focus_input = iced::widget::operation::focus(iced::widget::Id::new(SEARCH_INPUT_ID));

        match self.window {
            Some(id) => Task::batch([
                window::set_mode(id, window::Mode::Windowed),
                window::gain_focus(id),
                reload,
                focus_input,
            ]),
            None => {
                let (id, open) = window::open(window_settings());
                self.window = Some(id);
                Task::batch([open.map(|_| Message::WindowOpened), reload, focus_input])
            }
        }
    }

    fn hide(&mut self) -> Task<Message> {
        self.visible = false;
        match self.window {
            Some(id) => window::set_mode(id, window::Mode::Hidden),
            None => Task::none(),
        }
    }

    pub fn view(&self, _window: window::Id) -> Element<'_, Message> {
        view::view(self)
    }
}

fn window_settings() -> window::Settings {
    let mark = branding::render(64);
    window::Settings {
        size: Size::new(760.0, 560.0),
        min_size: Some(Size::new(540.0, 380.0)),
        exit_on_close_request: false,
        icon: window::icon::from_rgba(mark.rgba, mark.size, mark.size).ok(),
        ..window::Settings::default()
    }
}

/// Hands the shell's event receiver to exactly one subscription.
#[derive(Clone)]
struct EventSource(Arc<Mutex<Option<mpsc::Receiver<ShellEvent>>>>);

impl Hash for EventSource {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "ghostclipboard-shell-events".hash(state);
    }
}

impl EventSource {
    fn stream(&self) -> BoxStream<'static, Message> {
        let receiver = self.0.lock().expect("event source lock poisoned").take();
        match receiver {
            Some(rx) => stream::unfold(rx, |mut rx| async move {
                rx.recv().await.map(|event| (Message::Shell(event), rx))
            })
            .boxed(),
            None => stream::empty().boxed(),
        }
    }
}
