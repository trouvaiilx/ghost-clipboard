//! Iced application: state, messages and the glue to the native shell.
//!
//! Phase 1 only wires the shell events into a minimal window to prove the
//! integration; the real history interface replaces `view` in a later phase.

use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use iced::futures::stream::{self, BoxStream, StreamExt};
use iced::widget::{column, container, text};
use iced::window;
use iced::{Element, Length, Size, Subscription, Task, Theme};
use tokio::sync::mpsc;

use crate::branding;
use crate::platform::windows::Shell;
use crate::platform::{Component, ShellEvent, TrayCommand};

/// Command-line derived start-up options.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Start in the tray without showing the history window.
    pub background: bool,
}

/// Run the application until the user chooses *Exit*.
pub fn run(shell: Shell, options: Options) -> iced::Result {
    let boot = Mutex::new(Some((shell, options)));

    iced::daemon(
        move || {
            let (shell, options) = boot
                .lock()
                .expect("boot lock poisoned")
                .take()
                .expect("boot is only invoked once");
            State::new(shell, options)
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
    events: EventSource,
    window: Option<window::Id>,
    visible: bool,
    clipboard_changes: u64,
    hotkey_presses: u64,
    faults: Vec<(Component, String)>,
}

#[derive(Debug, Clone)]
enum Message {
    Shell(ShellEvent),
    WindowOpened,
    CloseRequested(window::Id),
}

impl State {
    fn new(mut shell: Shell, options: Options) -> (Self, Task<Message>) {
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
        let mut state = Self {
            shell,
            events,
            window: None,
            visible: false,
            clipboard_changes: 0,
            hotkey_presses: 0,
            faults,
        };
        let task = if options.background {
            Task::none()
        } else {
            state.show()
        };
        (state, task)
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
                // Closing hides to the tray; the daemon keeps running.
                if self.window == Some(id) {
                    self.hide()
                } else {
                    Task::none()
                }
            }
        }
    }

    fn on_shell_event(&mut self, event: ShellEvent) -> Task<Message> {
        tracing::debug!(?event, "shell event");
        match event {
            ShellEvent::ClipboardChanged => {
                self.clipboard_changes += 1;
                Task::none()
            }
            ShellEvent::HotkeyPressed => {
                self.hotkey_presses += 1;
                self.toggle()
            }
            ShellEvent::TrayActivated | ShellEvent::Menu(TrayCommand::OpenHistory) => self.show(),
            ShellEvent::Menu(TrayCommand::Exit) => {
                self.shell.shutdown();
                iced::exit()
            }
            ShellEvent::Menu(TrayCommand::TogglePause | TrayCommand::OpenSettings) => {
                // Wired up in later phases.
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
        let mut content = column![
            text("GhostClipboard").size(20),
            text("Native integration check (Phase 1)").size(13),
            text(format!(
                "Clipboard change notifications: {}",
                self.clipboard_changes
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
        // The daemon must outlive its window: close requests hide to the tray.
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
        // There is only ever one event source; a constant identity keeps the
        // subscription alive across state updates.
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
