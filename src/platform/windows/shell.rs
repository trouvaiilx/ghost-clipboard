//! The native shell thread.
//!
//! # Why a dedicated thread
//!
//! Three Win32 facilities need a message loop running on the thread that
//! created them:
//!
//! * `AddClipboardFormatListener` delivers `WM_CLIPBOARDUPDATE` to a window,
//! * `RegisterHotKey` delivers `WM_HOTKEY` to the registering thread's window,
//! * `tray-icon` creates a hidden window that receives notification-area
//!   callbacks.
//!
//! Iced runs its winit event loop on the main thread and owns its own windows.
//! Rather than rely on winit incidentally dispatching messages for foreign
//! windows (which couples the tray's lifetime to the GUI event loop and breaks
//! as soon as the GUI window is closed or recreated), all three live on one
//! thread with an explicit `GetMessageW` loop. The thread blocks in the kernel
//! while idle, so there is no polling.
//!
//! Events leave the thread through a *bounded* channel using `try_send`; the
//! window procedure must never block, and a burst of clipboard notifications
//! must not grow memory without limit.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc as std_mpsc;
use std::thread::JoinHandle;

use tokio::sync::mpsc::{self, error::TrySendError};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::Win32::Foundation::{ERROR_HOTKEY_ALREADY_REGISTERED, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, RemoveClipboardFormatListener,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
    UnregisterHotKey,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, HWND_MESSAGE,
    MSG, PostMessageW, PostQuitMessage, RegisterClassExW, TranslateMessage, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_APP, WM_CLIPBOARDUPDATE, WM_DESTROY, WM_HOTKEY, WNDCLASSEXW,
};
use windows::core::w;

use crate::branding;
use crate::platform::{Component, Hotkey, ShellEvent, ShellFault, TrayCommand};

/// Capacity of the event channel towards the application.
const EVENT_QUEUE: usize = 64;
/// Posted to the shell window to request an orderly shutdown.
const WM_SHUTDOWN: u32 = WM_APP + 1;
/// Posted by a secondary instance attempting to launch to activate this existing instance.
pub const WM_ACTIVATE_INSTANCE: u32 = WM_APP + 2;
/// Identifier of the single global shortcut registered on the shell window.
const HOTKEY_ID: i32 = 1;

const ID_OPEN: &str = "ghostclipboard.open";
const ID_PAUSE: &str = "ghostclipboard.pause";
const ID_SETTINGS: &str = "ghostclipboard.settings";
const ID_EXIT: &str = "ghostclipboard.exit";

#[derive(Debug, thiserror::Error)]
pub enum ShellError {
    #[error("could not create the native shell window: {0}")]
    Window(windows::core::Error),
    #[error("the native shell thread could not be started: {0}")]
    Spawn(std::io::Error),
    #[error("the native shell thread stopped during start-up")]
    StartupAborted,
}

/// Static configuration for the shell.
#[derive(Debug, Clone, Copy)]
pub struct ShellConfig {
    /// Global shortcut that toggles the history window.
    pub hotkey: Hotkey,
}

/// Outcome of start-up. Faults are non-fatal; see [`ShellFault`].
#[derive(Debug, Clone, Default)]
pub struct StartupReport {
    pub faults: Vec<ShellFault>,
}

impl StartupReport {
    pub fn fault(&self, component: Component) -> Option<&ShellFault> {
        self.faults.iter().find(|f| f.component == component)
    }
}

/// Owner of the shell thread. Dropping it shuts the thread down and joins it.
pub struct Shell {
    hwnd: isize,
    thread: Option<JoinHandle<()>>,
    events: Option<mpsc::Receiver<ShellEvent>>,
    report: StartupReport,
}

impl Shell {
    /// Start the shell thread and wait until initialisation has finished.
    pub fn spawn(config: ShellConfig) -> Result<Self, ShellError> {
        let (events_tx, events_rx) = mpsc::channel(EVENT_QUEUE);
        let (ready_tx, ready_rx) = std_mpsc::sync_channel(1);

        let thread = std::thread::Builder::new()
            .name("ghostclipboard-shell".into())
            .spawn(move || run(config, events_tx, ready_tx))
            .map_err(ShellError::Spawn)?;

        match ready_rx.recv() {
            Ok(Ok((hwnd, report))) => Ok(Self {
                hwnd,
                thread: Some(thread),
                events: Some(events_rx),
                report,
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = thread.join();
                Err(ShellError::StartupAborted)
            }
        }
    }

    /// Start-up faults for individual components.
    pub fn report(&self) -> &StartupReport {
        &self.report
    }

    /// Hand the event receiver to the consumer. Returns `None` after the first call.
    pub fn take_events(&mut self) -> Option<mpsc::Receiver<ShellEvent>> {
        self.events.take()
    }

    /// Native window handle for the shell message-only window.
    pub fn hwnd(&self) -> isize {
        self.hwnd
    }

    /// Stop the thread and release the listener, shortcut and tray icon.
    pub fn shutdown(&mut self) {
        if let Some(thread) = self.thread.take() {
            // SAFETY: the window belongs to the shell thread, which is still
            // alive because we hold its JoinHandle. PostMessageW is safe to
            // call from any thread, and unlike PostThreadMessageW it is still
            // delivered while a tray menu's modal loop is running.
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(self.hwnd as *mut _)),
                    WM_SHUTDOWN,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
            if thread.join().is_err() {
                tracing::error!("native shell thread panicked");
            }
        }
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// --- shell thread -----------------------------------------------------------

thread_local! {
    /// Sender used by the window procedure (which cannot capture state).
    static SINK: RefCell<Option<mpsc::Sender<ShellEvent>>> = const { RefCell::new(None) };
}

/// Notifications dropped since the last successful delivery.
static DROPPED: AtomicU32 = AtomicU32::new(0);

type Ready = Result<(isize, StartupReport), ShellError>;

fn deliver(tx: &mpsc::Sender<ShellEvent>, event: ShellEvent) {
    // Report earlier drops first so the consumer learns it missed something.
    let dropped = DROPPED.load(Ordering::Relaxed);
    if dropped > 0 && tx.try_send(ShellEvent::Overflow(dropped)).is_ok() {
        DROPPED.fetch_sub(dropped, Ordering::Relaxed);
    }
    match tx.try_send(event) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => {
            DROPPED.fetch_add(1, Ordering::Relaxed);
        }
        Err(TrySendError::Closed(_)) => {}
    }
}

fn run(config: ShellConfig, events: mpsc::Sender<ShellEvent>, ready: std_mpsc::SyncSender<Ready>) {
    SINK.with(|sink| *sink.borrow_mut() = Some(events.clone()));

    let hwnd = match create_window() {
        Ok(hwnd) => hwnd,
        Err(error) => {
            let _ = ready.send(Err(ShellError::Window(error)));
            return;
        }
    };

    let mut faults = Vec::new();

    // SAFETY: `hwnd` is a valid window owned by this thread.
    if let Err(error) = unsafe { AddClipboardFormatListener(hwnd) } {
        faults.push(ShellFault {
            component: Component::ClipboardListener,
            message: format!(
                "Windows refused clipboard notifications: {}",
                error.message()
            ),
        });
    }

    if let Err(message) = register_hotkey(hwnd, &config.hotkey) {
        faults.push(ShellFault {
            component: Component::GlobalHotkey,
            message,
        });
    }

    // The tray icon must be created and dropped on this thread.
    let tray = match create_tray(events) {
        Ok(tray) => Some(tray),
        Err(message) => {
            faults.push(ShellFault {
                component: Component::TrayIcon,
                message,
            });
            None
        }
    };

    let _ = ready.send(Ok((hwnd.0 as isize, StartupReport { faults })));

    message_loop();

    // Orderly teardown on the owning thread. The shell window (and with it the
    // clipboard listener and hotkey) was already released in WM_DESTROY.
    drop(tray);
    SINK.with(|sink| *sink.borrow_mut() = None);
    tracing::debug!("native shell thread stopped");
}

fn message_loop() {
    let mut msg = MSG::default();
    // SAFETY: standard message pump on the thread that owns the windows.
    // GetMessageW returns 0 for WM_QUIT and -1 on error; both end the loop.
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn create_window() -> windows::core::Result<HWND> {
    // SAFETY: plain Win32 class registration and window creation; all pointers
    // passed are valid for the duration of the calls.
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: w!("GhostClipboardShell"),
            ..Default::default()
        };
        // Re-registering an existing class fails harmlessly; the window is
        // created against whichever registration exists.
        RegisterClassExW(&class);
        // A message-only window receives clipboard and hotkey messages but
        // never appears in the task bar, Alt+Tab or window enumeration.
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("GhostClipboardShell"),
            w!("GhostClipboard shell"),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance.into()),
            None,
        )
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CLIPBOARDUPDATE => {
            notify(ShellEvent::ClipboardChanged);
            LRESULT(0)
        }
        WM_HOTKEY if wparam.0 == HOTKEY_ID as usize => {
            notify(ShellEvent::HotkeyPressed);
            LRESULT(0)
        }
        WM_ACTIVATE_INSTANCE => {
            notify(ShellEvent::HotkeyPressed);
            LRESULT(0)
        }
        WM_SHUTDOWN => {
            // SAFETY: destroying our own window from its own thread.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: both were registered for this window; removing something
            // that was never registered (e.g. after a conflict) fails harmlessly.
            unsafe {
                let _ = RemoveClipboardFormatListener(hwnd);
                let _ = UnregisterHotKey(Some(hwnd), HOTKEY_ID);
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        // SAFETY: default handling for everything else.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn notify(event: ShellEvent) {
    SINK.with(|sink| {
        if let Some(tx) = sink.borrow().as_ref() {
            deliver(tx, event);
        }
    });
}

/// Register the global shortcut, returning a user-facing explanation on failure.
fn register_hotkey(hwnd: HWND, hotkey: &Hotkey) -> Result<(), String> {
    // MOD_NOREPEAT: holding the keys must not toggle the window repeatedly.
    let mut modifiers: HOT_KEY_MODIFIERS = MOD_NOREPEAT;
    for (enabled, flag) in [
        (hotkey.ctrl, MOD_CONTROL),
        (hotkey.shift, MOD_SHIFT),
        (hotkey.alt, MOD_ALT),
        (hotkey.win, MOD_WIN),
    ] {
        if enabled {
            modifiers |= flag;
        }
    }
    // SAFETY: `hwnd` is a valid window owned by this thread.
    unsafe { RegisterHotKey(Some(hwnd), HOTKEY_ID, modifiers, hotkey.vk) }.map_err(|error| {
        if error.code() == ERROR_HOTKEY_ALREADY_REGISTERED.to_hresult() {
            format!("{hotkey} is already used by another application. Choose a different shortcut in Settings.")
        } else {
            format!("Windows could not register {hotkey}: {}", error.message())
        }
    })
}

fn create_tray(events: mpsc::Sender<ShellEvent>) -> Result<TrayIcon, String> {
    let menu = Menu::new();
    let open = MenuItem::with_id(ID_OPEN, "Open history", true, None);
    let pause = MenuItem::with_id(ID_PAUSE, "Pause capture", true, None);
    let settings = MenuItem::with_id(ID_SETTINGS, "Settings", true, None);
    let exit = MenuItem::with_id(ID_EXIT, "Exit", true, None);
    menu.append_items(&[
        &open,
        &pause,
        &PredefinedMenuItem::separator(),
        &settings,
        &PredefinedMenuItem::separator(),
        &exit,
    ])
    .map_err(|e| e.to_string())?;

    let mark = branding::render(32);
    let icon = Icon::from_rgba(mark.rgba, mark.size, mark.size).map_err(|e| e.to_string())?;

    let menu_events = events.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let command = match event.id.as_ref() {
            ID_OPEN => TrayCommand::OpenHistory,
            ID_PAUSE => TrayCommand::TogglePause,
            ID_SETTINGS => TrayCommand::OpenSettings,
            ID_EXIT => TrayCommand::Exit,
            _ => return,
        };
        deliver(&menu_events, ShellEvent::Menu(command));
    }));

    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            deliver(&events, ShellEvent::TrayActivated);
        }
    }));

    TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("GhostClipboard")
        .with_icon(icon)
        .build()
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    //! These tests exercise the real Win32 shell thread (window, message loop,
    //! clipboard listener, `RegisterHotKey`, tray icon). They need a Windows
    //! desktop session but no user interaction. Synthetic messages are posted
    //! to the shell window to drive the window procedure deterministically;
    //! genuine keyboard-driven activation is verified manually (see
    //! docs/VALIDATION.md).

    use super::*;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    /// The shell uses process-global state (tray handlers, drop counter, hotkey
    /// registration), so tests must not overlap.
    static SERIAL: Mutex<()> = Mutex::new(());

    /// An unlikely shortcut so tests do not collide with a running instance.
    const TEST_HOTKEY: Hotkey = Hotkey {
        ctrl: true,
        shift: true,
        alt: true,
        win: false,
        vk: 0x87, // F24
    };

    fn serial() -> std::sync::MutexGuard<'static, ()> {
        SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn spawn() -> Shell {
        DROPPED.store(0, Ordering::Relaxed);
        Shell::spawn(ShellConfig {
            hotkey: TEST_HOTKEY,
        })
        .expect("shell should start")
    }

    fn post(shell: &Shell, message: u32, wparam: usize) {
        // SAFETY: the shell window is alive while `shell` exists.
        unsafe {
            PostMessageW(
                Some(HWND(shell.hwnd as *mut _)),
                message,
                WPARAM(wparam),
                LPARAM(0),
            )
            .expect("posting to the shell window");
        }
    }

    fn next_event(rx: &mut mpsc::Receiver<ShellEvent>) -> ShellEvent {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Ok(event) = rx.try_recv() {
                return event;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for a shell event"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn starts_without_faults_and_shuts_down() {
        let _guard = serial();
        let mut shell = spawn();
        assert!(
            shell.report().faults.is_empty(),
            "unexpected faults: {:?}",
            shell.report().faults
        );
        shell.shutdown();
        // Shutting down twice must be harmless.
        shell.shutdown();
    }

    #[test]
    fn shutdown_releases_the_hotkey() {
        let _guard = serial();
        let mut first = spawn();
        assert!(first.report().fault(Component::GlobalHotkey).is_none());
        first.shutdown();

        // If the registration leaked, this would report a conflict.
        let second = spawn();
        assert!(
            second.report().fault(Component::GlobalHotkey).is_none(),
            "hotkey was not released: {:?}",
            second.report().faults
        );
    }

    #[test]
    fn hotkey_conflict_is_reported_with_an_explanation() {
        let _guard = serial();
        let first = spawn();
        assert!(first.report().fault(Component::GlobalHotkey).is_none());

        let second = spawn();
        let fault = second
            .report()
            .fault(Component::GlobalHotkey)
            .expect("registering the same shortcut twice must fail");
        assert!(
            fault.message.contains("already used"),
            "message should explain the conflict: {}",
            fault.message
        );
        assert!(fault.message.contains("Ctrl+Shift+Alt+F24"));
        // The conflict must not take down the other components.
        assert!(
            second
                .report()
                .fault(Component::ClipboardListener)
                .is_none()
        );
    }

    #[test]
    fn hotkey_and_clipboard_messages_become_events() {
        let _guard = serial();
        let mut shell = spawn();
        let mut events = shell.take_events().expect("receiver available once");
        assert!(shell.take_events().is_none());

        post(&shell, WM_HOTKEY, HOTKEY_ID as usize);
        assert_eq!(next_event(&mut events), ShellEvent::HotkeyPressed);

        post(&shell, WM_CLIPBOARDUPDATE, 0);
        assert_eq!(next_event(&mut events), ShellEvent::ClipboardChanged);

        // A WM_HOTKEY for an identifier we never registered is ignored.
        post(&shell, WM_HOTKEY, 99);
        post(&shell, WM_CLIPBOARDUPDATE, 0);
        assert_eq!(next_event(&mut events), ShellEvent::ClipboardChanged);
    }

    #[test]
    fn bursts_are_bounded_and_overflow_is_reported() {
        let _guard = serial();
        let mut shell = spawn();
        let mut events = shell.take_events().unwrap();

        let burst = EVENT_QUEUE + 36;
        for _ in 0..burst {
            post(&shell, WM_CLIPBOARDUPDATE, 0);
        }
        // Let the shell thread work through its message queue.
        std::thread::sleep(Duration::from_millis(500));

        let mut queued = 0;
        while let Ok(event) = events.try_recv() {
            assert_eq!(event, ShellEvent::ClipboardChanged);
            queued += 1;
        }
        assert_eq!(queued, EVENT_QUEUE, "queue must be bounded, not unbounded");

        // The next delivery tells the consumer that notifications were lost.
        post(&shell, WM_CLIPBOARDUPDATE, 0);
        match next_event(&mut events) {
            ShellEvent::Overflow(dropped) => assert!(dropped >= 36, "dropped = {dropped}"),
            other => panic!("expected an overflow report, got {other:?}"),
        }
        assert_eq!(next_event(&mut events), ShellEvent::ClipboardChanged);
    }
}
