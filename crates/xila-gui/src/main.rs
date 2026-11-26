//! pgXila graphical client: the window.
//!
//! Everything that is drawn and everything that reacts lives in the library.
//! This file opens a window, feeds its events to [`App`], and shows the
//! screen enlarged by a whole factor.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Parser;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, Ime, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key as WindowKey, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};
use xila_core::input::Key;
use xila_core::login::open_session;
use xila_core::server::Server;
use xila_gui::app::{App, Options};
use xila_gui::frame::{Frame, HEIGHT, WIDTH};
use xila_gui::present::{largest_scale, paint, Viewport};

/// How often the client looks for what the server pushed.
const TICK: Duration = Duration::from_millis(200);
/// Half of the blinking period of the cursor.
const BLINK: Duration = Duration::from_millis(500);
/// The window starts at this factor when the display is large enough.
const PREFERRED_SCALE: u32 = 3;

/// pgXila graphical client: the screens of a pgXila server in pixel art.
///
/// The connection is always encrypted with TLS 1.3 and the server certificate
/// is always verified. The password is asked for on the connection screen, or
/// read from the PGXILA_PASSWORD environment variable.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Host name of the server. It must match the server certificate.
    #[arg(long, env = "PGXILA_HOST", default_value = "localhost")]
    host: String,

    #[arg(long, env = "PGXILA_PORT", default_value_t = 5419)]
    port: u16,

    #[arg(long, env = "PGXILA_DATABASE", default_value = "pgxila")]
    database: String,

    /// PostgreSQL role to connect as.
    #[arg(long, env = "PGXILA_USER")]
    user: Option<String>,

    /// Certificate authority that signed the server certificate (PEM file).
    /// Without it, the authorities of the operating system are trusted.
    #[arg(long, env = "PGXILA_CA")]
    ca: Option<PathBuf>,

    /// How many times the 480 by 300 screen is enlarged when the window
    /// opens. Without it, 3 if the display is large enough, or else less.
    #[arg(long, env = "PGXILA_SCALE", value_parser = clap::value_parser!(u32).range(1..=8))]
    scale: Option<u32>,
}

type Surface = softbuffer::Surface<Arc<Window>, Arc<Window>>;

struct Shell {
    app: App<Server>,
    frame: Frame,
    window: Option<Arc<Window>>,
    surface: Option<Surface>,
    modifiers: ModifiersState,
    /// The screen pixel under the pointer, when it is over the screen.
    pointer: Option<(i32, i32)>,
    next_tick: Instant,
    /// When the user last did something: the cursor blinks from there.
    last_input: Instant,
    /// What the last drawing showed, to know when to draw again.
    drawn: (String, bool),
    /// The factor asked for on the command line.
    scale: Option<u32>,
    /// Why the window could not be shown, if it could not.
    failure: Option<String>,
}

impl Shell {
    fn new(app: App<Server>, scale: Option<u32>) -> Self {
        let now = Instant::now();
        Shell {
            app,
            frame: Frame::new(),
            window: None,
            surface: None,
            modifiers: ModifiersState::empty(),
            pointer: None,
            next_tick: now,
            last_input: now,
            drawn: (String::new(), false),
            scale,
            failure: None,
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, message: String) {
        self.failure = Some(message);
        event_loop.exit();
    }

    fn clock() -> String {
        chrono::Local::now().format("%H:%M").to_string()
    }

    fn blink(&self) -> bool {
        (self.last_input.elapsed().as_millis() / BLINK.as_millis()).is_multiple_of(2)
    }

    /// Something the user did: the cursor is shown, and the screen redrawn.
    fn touched(&mut self) {
        self.last_input = Instant::now();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Copies or cuts what is selected to the clipboard of the system, or
    /// pastes what it holds. A clipboard that cannot be reached is no reason
    /// to stop: nothing is copied, and the message says so.
    fn clipboard(&mut self, order: ClipboardOrder) {
        let done = arboard::Clipboard::new().and_then(|mut clipboard| match order {
            ClipboardOrder::Paste => clipboard.get_text().map(|text| self.app.paste(&text)),
            ClipboardOrder::Copy | ClipboardOrder::Cut => {
                match self.app.copy(order == ClipboardOrder::Cut) {
                    Some(text) => clipboard.set_text(text),
                    None => Ok(()),
                }
            }
        });
        if let Err(error) = done {
            // An empty clipboard is not worth a word.
            if !matches!(error, arboard::Error::ContentNotAvailable) {
                eprintln!("xila-gui: clipboard: {error}");
            }
        }
        self.touched();
    }

    fn keys(&mut self, keys: Vec<Key>) {
        if keys.is_empty() {
            return;
        }
        for key in keys {
            self.app.key(key);
        }
        self.touched();
    }

    fn draw(&mut self) -> Result<(), softbuffer::SoftBufferError> {
        let (clock, blink) = (Shell::clock(), self.blink());
        let (Some(window), Some(surface)) = (&self.window, &mut self.surface) else {
            return Ok(());
        };
        let size = window.inner_size();
        let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        else {
            return Ok(());
        };
        surface.resize(width, height)?;

        self.app.draw(&mut self.frame, &clock, blink);
        let mut buffer = surface.buffer_mut()?;
        paint(&self.frame, &self.app.palette(), &mut buffer, size.width, size.height);
        window.pre_present_notify();
        buffer.present()?;
        self.drawn = (clock, blink);
        Ok(())
    }
}

/// The size the window starts with, in physical pixels: the factor asked
/// for, or else the preferred one, or less on a small display.
fn starting_size(event_loop: &ActiveEventLoop, asked: Option<u32>) -> PhysicalSize<u32> {
    let monitor = event_loop.primary_monitor().or_else(|| event_loop.available_monitors().next());
    let (fitting, pixel_ratio) = match monitor {
        Some(monitor) => {
            let pixel_ratio = monitor.scale_factor();
            let size = monitor.size().to_logical::<f64>(pixel_ratio);
            // Leave room for the title bar, the menu bar and the dock.
            (largest_scale(size.width * 0.96, size.height * 0.85, PREFERRED_SCALE), pixel_ratio)
        }
        None => (2, 1.0),
    };
    let scale = asked.unwrap_or(fitting);
    // A whole number of physical pixels per screen pixel, whatever the ratio.
    let factor = (f64::from(scale) * pixel_ratio).round().max(1.0) as u32;
    PhysicalSize::new(WIDTH as u32 * factor, HEIGHT as u32 * factor)
}

/// Translates a key of the window into the keys the client understands.
/// `command` is true while Control or Command is held: such keys are
/// shortcuts, not text.
fn translate(key: &WindowKey, text: Option<&str>, shift: bool, command: bool) -> Vec<Key> {
    const FUNCTION_KEYS: [NamedKey; 24] = [
        NamedKey::F1,
        NamedKey::F2,
        NamedKey::F3,
        NamedKey::F4,
        NamedKey::F5,
        NamedKey::F6,
        NamedKey::F7,
        NamedKey::F8,
        NamedKey::F9,
        NamedKey::F10,
        NamedKey::F11,
        NamedKey::F12,
        NamedKey::F13,
        NamedKey::F14,
        NamedKey::F15,
        NamedKey::F16,
        NamedKey::F17,
        NamedKey::F18,
        NamedKey::F19,
        NamedKey::F20,
        NamedKey::F21,
        NamedKey::F22,
        NamedKey::F23,
        NamedKey::F24,
    ];
    match key {
        WindowKey::Named(named) => {
            if let Some(index) = FUNCTION_KEYS.iter().position(|f| f == named) {
                return vec![Key::F(index as u8 + 1)];
            }
            vec![match named {
                NamedKey::Enter => Key::Enter,
                NamedKey::Tab if shift => Key::BackTab,
                NamedKey::Tab => Key::Tab,
                NamedKey::ArrowUp if shift => Key::ShiftUp,
                NamedKey::ArrowDown if shift => Key::ShiftDown,
                NamedKey::ArrowLeft if shift => Key::ShiftLeft,
                NamedKey::ArrowRight if shift => Key::ShiftRight,
                NamedKey::ArrowUp => Key::Up,
                NamedKey::ArrowDown => Key::Down,
                NamedKey::ArrowLeft => Key::Left,
                NamedKey::ArrowRight => Key::Right,
                NamedKey::Home => Key::Home,
                NamedKey::End => Key::End,
                NamedKey::PageUp => Key::PageUp,
                NamedKey::PageDown => Key::PageDown,
                NamedKey::Backspace => Key::Backspace,
                NamedKey::Delete => Key::Delete,
                NamedKey::Escape => Key::Escape,
                NamedKey::Space if !command => Key::Char(' '),
                // Other named keys carry no text worth typing.
                _ => return Vec::new(),
            }]
        }
        // Control-Z, or Command-Z on a Mac, undoes; with the shift key, or as
        // Control-Y, it does again what was undone.
        WindowKey::Character(letter) if command && letter.eq_ignore_ascii_case("z") => {
            vec![if shift { Key::Redo } else { Key::Undo }]
        }
        WindowKey::Character(letter) if command && letter.eq_ignore_ascii_case("y") => {
            vec![Key::Redo]
        }
        WindowKey::Character(letter) if command && letter.eq_ignore_ascii_case("a") => {
            vec![Key::SelectAll]
        }
        _ if command => Vec::new(),
        _ => typed(text.unwrap_or_default()),
    }
}

/// What Control or Command with C, X or V asks of the clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipboardOrder {
    Copy,
    Cut,
    Paste,
}

impl ClipboardOrder {
    fn of(letter: &str) -> Option<Self> {
        match letter.to_ascii_lowercase().as_str() {
            "c" => Some(ClipboardOrder::Copy),
            "x" => Some(ClipboardOrder::Cut),
            "v" => Some(ClipboardOrder::Paste),
            _ => None,
        }
    }
}

/// The characters of a text that can be typed into a field.
fn typed(text: &str) -> Vec<Key> {
    text.chars().filter(|c| !c.is_control()).map(Key::Char).collect()
}

impl ApplicationHandler for Shell {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("pgXila")
            .with_inner_size(starting_size(event_loop, self.scale))
            .with_min_inner_size(PhysicalSize::new(WIDTH as u32, HEIGHT as u32));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => return self.fail(event_loop, format!("cannot open a window: {error}")),
        };
        let surface = softbuffer::Context::new(window.clone())
            .and_then(|context| softbuffer::Surface::new(&context, window.clone()));
        match surface {
            Ok(surface) => self.surface = Some(surface),
            Err(error) => {
                return self.fail(event_loop, format!("cannot draw in the window: {error}"))
            }
        }
        // Accents typed with dead keys and input methods arrive as committed
        // text.
        window.set_ime_allowed(true);
        window.request_redraw();
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.draw() {
                    self.fail(event_loop, format!("cannot draw in the window: {error}"));
                }
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let command = self.modifiers.control_key() || self.modifiers.super_key();
                // Copy, cut and paste go through the clipboard of the system.
                if let (true, WindowKey::Character(letter)) = (command, &event.logical_key) {
                    if let Some(order) = ClipboardOrder::of(letter) {
                        self.clipboard(order);
                        return;
                    }
                }
                let keys = translate(
                    &event.logical_key,
                    event.text.as_deref(),
                    self.modifiers.shift_key(),
                    command,
                );
                self.keys(keys);
            }
            WindowEvent::Ime(Ime::Commit(text)) => self.keys(typed(&text)),
            WindowEvent::CursorMoved { position, .. } => {
                let size = self.window.as_ref().map(|window| window.inner_size());
                self.pointer = size.and_then(|size| {
                    Viewport::fit(size.width, size.height).to_screen(position.x, position.y)
                });
                // Something may be following the pointer: a widget dragged in
                // the visual editor.
                if let Some((x, y)) = self.pointer {
                    if self.app.pointer_move(x, y) {
                        self.touched();
                    }
                }
            }
            WindowEvent::CursorLeft { .. } => self.pointer = None,
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                // A release outside the screen still ends the press.
                let (x, y) = self.pointer.unwrap_or((-1, -1));
                match state {
                    ElementState::Pressed => self.app.pointer_down(x, y),
                    ElementState::Released => self.app.pointer_up(x, y),
                }
                self.touched();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.app.should_quit() {
            event_loop.exit();
            return;
        }
        let now = Instant::now();
        if now >= self.next_tick {
            self.next_tick = now + TICK;
            let changed = self.app.tick();
            let showing = (Shell::clock(), self.blink());
            if changed || showing != self.drawn {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_tick));
    }
}

fn main() -> ExitCode {
    let args = Args::parse();
    let options = Options {
        host: args.host,
        port: args.port,
        database: args.database,
        user: args.user,
        ca: args.ca,
    };
    let with_user = options.user.is_some();
    let mut app = App::new(options, Box::new(open_session));
    // With a user and a password in the environment, connect straight away.
    if let (true, Ok(password)) = (with_user, std::env::var("PGXILA_PASSWORD")) {
        app.connect_with(&password);
    }

    let mut shell = Shell::new(app, args.scale);
    let outcome = EventLoop::new().and_then(|event_loop| event_loop.run_app(&mut shell));
    match (outcome, shell.failure) {
        (Ok(()), None) => ExitCode::SUCCESS,
        (_, Some(message)) => {
            eprintln!("xila-gui: {message}");
            ExitCode::FAILURE
        }
        (Err(error), None) => {
            eprintln!("xila-gui: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(key: NamedKey) -> WindowKey {
        WindowKey::Named(key)
    }

    fn character(text: &str) -> WindowKey {
        WindowKey::Character(text.into())
    }

    #[test]
    fn function_and_editing_keys_are_recognised() {
        assert_eq!(translate(&named(NamedKey::F3), None, false, false), [Key::F(3)]);
        assert_eq!(translate(&named(NamedKey::F12), None, false, false), [Key::F(12)]);
        assert_eq!(translate(&named(NamedKey::F24), None, false, false), [Key::F(24)]);
        assert_eq!(translate(&named(NamedKey::Enter), Some("\r"), false, false), [Key::Enter]);
        assert_eq!(translate(&named(NamedKey::Tab), Some("\t"), false, false), [Key::Tab]);
        assert_eq!(translate(&named(NamedKey::Tab), Some("\t"), true, false), [Key::BackTab]);
        assert_eq!(
            translate(&named(NamedKey::Backspace), Some("\u{8}"), false, false),
            [Key::Backspace]
        );
        assert_eq!(translate(&named(NamedKey::ArrowDown), None, false, false), [Key::Down]);
        assert_eq!(translate(&named(NamedKey::ArrowDown), None, true, false), [Key::ShiftDown]);
        // Control-Z or Command-Z undoes; without the modifier it is a letter.
        assert_eq!(translate(&character("z"), Some("z"), false, true), [Key::Undo]);
        assert_eq!(translate(&character("z"), Some("z"), false, false), [Key::Char('z')]);
        // Escape is a key of its own, not the character it comes with.
        assert_eq!(
            translate(&named(NamedKey::Escape), Some("\u{1b}"), false, false),
            [Key::Escape]
        );
    }

    #[test]
    fn text_is_typed_character_by_character() {
        assert_eq!(translate(&character("é"), Some("é"), false, false), [Key::Char('é')]);
        assert_eq!(translate(&character("a"), Some("A"), true, false), [Key::Char('A')]);
        assert_eq!(translate(&named(NamedKey::Space), Some(" "), false, false), [Key::Char(' ')]);
        assert_eq!(typed("œuf"), [Key::Char('œ'), Key::Char('u'), Key::Char('f')]);
    }

    #[test]
    fn shortcuts_and_keys_without_text_type_nothing() {
        assert!(translate(&character("q"), Some("q"), false, true).is_empty());
        assert!(translate(&named(NamedKey::Shift), None, true, false).is_empty());
        assert!(translate(&named(NamedKey::CapsLock), None, false, false).is_empty());
        assert!(translate(&character("^"), None, false, false).is_empty());
        // A function key is not a shortcut of the operating system here.
        assert_eq!(translate(&named(NamedKey::F3), None, false, true), [Key::F(3)]);
    }
}
