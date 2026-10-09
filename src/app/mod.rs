//! Iced application: state, messages, and native shell / service integration.

use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use iced::futures::stream::{self, BoxStream, StreamExt};
use iced::widget::{column, container, text};
use iced::window;
use iced::{Element, Length, Size, Subscription, Task, Theme};
use tokio::sync::{Mutex as TokioMutex, mpsc};

use crate::branding;
use crate::domain::settings::CaptureSettings;
use crate::platform::windows::Shell;
use crate::platform::{Component, ShellEvent, TrayCommand};
use crate::services::CaptureService;
use crate::storage::repository::{OpenReport, RecordOutcome};
use crate::storage::{Storage, StorageHandle};

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

struct State {
    shell: Shell,
    _storage: StorageHandle,
    capture: Arc<TokioMutex<CaptureService>>,
    events: EventSource,
    window: Option<window::Id>,
    visible: bool,
    clipboard_notifications: u64,
    entries_recorded: usize,
    pinned_count: usize,
    hotkey_presses: u64,
    is_paused: bool,
    db_path: String,
    faults: Vec<(Component, String)>,
}

#[derive(Debug, Clone)]
enum Message {
    Shell(ShellEvent),
    WindowOpened,
    CloseRequested(window::Id),
    StatsLoaded(Result<OpenReport, String>),
    ClipboardCaptured(Result<Option<RecordOutcome>, String>),
    PauseToggled(bool),
}

impl State {
    fn new(
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
        let capture_service =
            CaptureService::new(storage_handle.clone(), CaptureSettings::default());
        let capture = Arc::new(TokioMutex::new(capture_service));

        let mut state = Self {
            shell,
            _storage: storage_handle.clone(),
            capture,
            events,
            window: None,
            visible: false,
            clipboard_notifications: 0,
            entries_recorded: 0,
            pinned_count: 0,
            hotkey_presses: 0,
            is_paused: false,
            db_path,
            faults,
        };

        let initial_show = if options.background {
            Task::none()
        } else {
            state.show()
        };

        // Load initial database stats
        let load_stats = Task::perform(
            async move { storage_handle.report().await.map_err(|e| e.to_string()) },
            Message::StatsLoaded,
        );

        (state, Task::batch([initial_show, load_stats]))
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            Subscription::run_with(self.events.clone(), EventSource::stream),
            window::close_requests().map(Message::CloseRequested),
        ])
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Shell(event) => self.on_shell_event(event),
            Message::WindowOpened => Task::none(),
            Message::CloseRequested(id) => {
                if self.window == Some(id) {
                    self.hide()
                } else {
                    Task::none()
                }
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
                Task::none()
            }
            Message::ClipboardCaptured(Ok(None)) => Task::none(),
            Message::ClipboardCaptured(Err(err)) => {
                tracing::error!(%err, "error during clipboard capture processing");
                Task::none()
            }
            Message::PauseToggled(paused) => {
                self.is_paused = paused;
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
                // Wired in Phase 5
                Task::none()
            }
            ShellEvent::Overflow(dropped) => {
                tracing::warn!(dropped, "shell events were dropped");
                Task::none()
            }
        }
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
        match self.window {
            Some(id) => Task::batch([
                window::set_mode(id, window::Mode::Windowed),
                window::gain_focus(id),
            ]),
            None => {
                let (id, open) = window::open(window_settings());
                self.window = Some(id);
                open.map(|_| Message::WindowOpened)
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

    fn view(&self, _window: window::Id) -> Element<'_, Message> {
        let status_text = if self.is_paused {
            "Status: PAUSED (capturing suspended)"
        } else {
            "Status: ACTIVE (recording clipboard)"
        };

        let mut content = column![
            text("GhostClipboard").size(20),
            text("Native Windows Clipboard History (Phase 2)").size(13),
            text(status_text).size(14),
            text(format!("Storage Location: {}", self.db_path)).size(12),
            text(format!(
                "Clipboard change notifications: {}",
                self.clipboard_notifications
            )),
            text(format!(
                "Entries in database: {} ({} pinned)",
                self.entries_recorded, self.pinned_count
            )),
            text(format!("Shortcut activations: {}", self.hotkey_presses)),
        ]
        .spacing(8);

        for (component, message) in &self.faults {
            content = content.push(text(format!("{component:?} unavailable: {message}")));
        }

        container(content)
            .padding(16)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

fn window_settings() -> window::Settings {
    let mark = branding::render(64);
    window::Settings {
        size: Size::new(640.0, 520.0),
        min_size: Some(Size::new(420.0, 320.0)),
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
