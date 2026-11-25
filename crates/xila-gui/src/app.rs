//! The graphical client without its window: what is on screen, and what
//! keys, clicks and time do to it.
//!
//! The window feeds events in and shows the frame that comes out. Nothing
//! here needs a display, so the whole client can be driven by tests.

use std::path::PathBuf;

use xila_core::api::Api;
use xila_core::connection::ConnectParams;
use xila_core::input::Key;
use xila_core::login::{LoginForm, Outcome};
use xila_core::session::Session;

use crate::font::Fonts;
use crate::frame::Frame;
use crate::login::{self, Button, Target};
use crate::palette::Palette;
use crate::screen::{self, Ui};

/// Where to connect, from the command line.
#[derive(Debug, Clone)]
pub struct Options {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: Option<String>,
    /// Certificate authority that signed the server certificate.
    pub ca: Option<PathBuf>,
}

/// Opens a session, or explains to the user why not.
pub type Connect<A> = Box<dyn FnMut(&ConnectParams) -> Result<Session<A>, String>>;

enum Stage<A: Api> {
    Login(LoginForm),
    Session(Box<Session<A>>, Ui),
}

pub struct App<A: Api> {
    fonts: Fonts,
    options: Options,
    connect: Connect<A>,
    stage: Stage<A>,
    /// The button of the connection screen the pointer is held on.
    pressed: Option<Button>,
    quit: bool,
}

impl<A: Api> App<A> {
    /// Starts on the connection screen.
    pub fn new(options: Options, connect: Connect<A>) -> Self {
        let form =
            LoginForm::new(&options.host, options.port, &options.database, options.user.as_deref());
        App {
            fonts: Fonts::builtin(),
            options,
            connect,
            stage: Stage::Login(form),
            pressed: None,
            quit: false,
        }
    }

    /// Connects straight away with a password that was not typed: the one of
    /// the environment. On failure the connection screen says why.
    pub fn connect_with(&mut self, password: &str) {
        let Stage::Login(form) = &self.stage else { return };
        let mut params = form.params(self.options.port, self.options.ca.clone());
        params.password = password.to_owned();
        self.open(&params);
    }

    fn open(&mut self, params: &ConnectParams) {
        match (self.connect)(params) {
            Ok(session) => {
                let mut session = Box::new(session);
                let mut ui = Ui::default();
                screen::settle(&mut session, &mut ui, &self.fonts);
                self.stage = Stage::Session(session, ui);
            }
            Err(message) => {
                if let Stage::Login(form) = &mut self.stage {
                    form.message = Some(message);
                }
            }
        }
    }

    /// Connects with what the form holds, then forgets the password.
    fn submit(&mut self) {
        let Stage::Login(form) = &mut self.stage else { return };
        let params = form.params(self.options.port, self.options.ca.clone());
        form.clear_password();
        self.open(&params);
    }

    pub fn should_quit(&self) -> bool {
        self.quit
    }

    /// The session, once connected.
    pub fn session(&self) -> Option<&Session<A>> {
        match &self.stage {
            Stage::Session(session, _) => Some(session),
            Stage::Login(_) => None,
        }
    }

    /// The message of the connection screen, while it is shown.
    pub fn login_message(&self) -> Option<&str> {
        match &self.stage {
            Stage::Login(form) => form.message.as_deref(),
            Stage::Session(..) => None,
        }
    }

    pub fn key(&mut self, key: Key) {
        match &mut self.stage {
            Stage::Login(form) => match form.handle(key) {
                Outcome::Quit => self.quit = true,
                Outcome::Connect => self.submit(),
                Outcome::Stay => {}
            },
            Stage::Session(session, ui) => {
                screen::handle_key(session, ui, &self.fonts, key);
                self.quit = session.should_quit();
            }
        }
    }

    pub fn pointer_down(&mut self, x: i32, y: i32) {
        match &mut self.stage {
            Stage::Login(form) => match login::target_at(form, &self.fonts, x, y) {
                Some(Target::Field { index, cursor }) => form.focus_field(index, Some(cursor)),
                Some(Target::Button(button)) => self.pressed = Some(button),
                None => {}
            },
            Stage::Session(session, ui) => {
                screen::pointer_down(session, ui, &self.fonts, x, y);
                self.quit = session.should_quit();
            }
        }
    }

    pub fn pointer_up(&mut self, x: i32, y: i32) {
        match &mut self.stage {
            Stage::Login(form) => {
                let Some(pressed) = self.pressed.take() else { return };
                if login::target_at(form, &self.fonts, x, y) != Some(Target::Button(pressed)) {
                    return;
                }
                match pressed {
                    Button::Quit => self.quit = true,
                    // The same checks as the Enter key.
                    Button::Connect => self.key(Key::Enter),
                }
            }
            Stage::Session(session, ui) => {
                screen::pointer_up(session, ui, &self.fonts, x, y);
                self.quit = session.should_quit();
            }
        }
    }

    /// Looks at what the server pushed since the last call. Returns true
    /// when the screen must be drawn again.
    pub fn tick(&mut self) -> bool {
        match &mut self.stage {
            Stage::Login(_) => false,
            Stage::Session(session, ui) => {
                let changed = session.poll();
                if changed {
                    screen::settle(session, ui, &self.fonts);
                }
                changed
            }
        }
    }

    /// Draws the screen. `clock` is the time shown in the menu bar, `blink`
    /// whether the cursor is in the visible half of its blinking.
    pub fn draw(&self, frame: &mut Frame, clock: &str, blink: bool) {
        match &self.stage {
            Stage::Login(form) => login::draw(form, self.pressed, &self.fonts, frame, clock, blink),
            Stage::Session(session, ui) => {
                screen::draw(session, ui, &self.fonts, frame, clock, blink)
            }
        }
    }

    /// What the palette slots look like right now: the colours of the server
    /// once connected, the built-in ones before.
    pub fn palette(&self) -> Palette {
        match &self.stage {
            Stage::Login(_) => Palette::boot(),
            Stage::Session(session, _) => {
                Palette::of_session(session.palette(), &session.view().screen.colours)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::Rect;
    use crate::palette::slot;
    use crate::testing::{has_slot, Fake};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn options(user: Option<&str>) -> Options {
        Options {
            host: "localhost".into(),
            port: 5419,
            database: "pgxila".into(),
            user: user.map(str::to_owned),
            ca: None,
        }
    }

    /// An application whose server accepts the password "secret" only, and
    /// the list of the connection attempts it saw.
    fn app(user: Option<&str>) -> (App<Fake>, Rc<RefCell<Vec<ConnectParams>>>) {
        let attempts = Rc::new(RefCell::new(Vec::new()));
        let seen = attempts.clone();
        let connect: Connect<Fake> = Box::new(move |params| {
            seen.borrow_mut().push(params.clone());
            if params.password == "secret" {
                Session::start(Fake::default()).map_err(|error| error.to_string())
            } else {
                Err("Wrong user name or password.".to_owned())
            }
        });
        (App::new(options(user), connect), attempts)
    }

    fn type_text(app: &mut App<Fake>, text: &str) {
        for c in text.chars() {
            app.key(Key::Char(c));
        }
    }

    fn render(app: &App<Fake>) -> Frame {
        let mut frame = Frame::new();
        app.draw(&mut frame, "16:54", true);
        frame
    }

    #[test]
    fn typing_the_password_and_enter_opens_the_desktop() {
        let (mut app, attempts) = app(Some("ada"));
        assert!(app.session().is_none());
        type_text(&mut app, "secret");
        app.key(Key::Enter);

        let session = app.session().expect("connected");
        assert_eq!(session.view().stack(), "desktop");
        let attempts = attempts.borrow();
        assert_eq!(attempts.len(), 1);
        assert_eq!((attempts[0].user.as_str(), attempts[0].port), ("ada", 5419));
        // Connected: the menu bar shows the green light.
        assert!(has_slot(&render(&app), Rect::new(0, 0, 480, 14), slot::GREEN));
    }

    #[test]
    fn a_refused_connection_says_why_and_forgets_the_password() {
        let (mut app, attempts) = app(Some("ada"));
        type_text(&mut app, "wrong");
        app.key(Key::Enter);
        assert!(app.session().is_none());
        assert_eq!(app.login_message(), Some("Wrong user name or password."));

        // The field is empty again: typing the right one is enough.
        type_text(&mut app, "secret");
        app.key(Key::Enter);
        assert!(app.session().is_some());
        assert_eq!(attempts.borrow()[1].password, "secret");
    }

    #[test]
    fn a_password_from_the_environment_connects_without_the_form() {
        let (mut app, _) = app(Some("ada"));
        app.connect_with("secret");
        assert!(app.session().is_some());

        let (mut app, _) = self::app(Some("ada"));
        app.connect_with("wrong");
        assert!(app.session().is_none());
        assert!(app.login_message().is_some());
    }

    #[test]
    fn the_buttons_of_the_connection_screen_work_with_the_pointer() {
        let (mut app, attempts) = app(Some("ada"));
        type_text(&mut app, "secret");
        let Stage::Login(form) = &app.stage else { panic!("not on the connection screen") };
        let fonts = Fonts::builtin();
        // Find the buttons by scanning the screen, as a user would.
        let find = |wanted: Button| {
            (0..300)
                .flat_map(|y| (0..480).map(move |x| (x, y)))
                .find(|(x, y)| {
                    login::target_at(form, &fonts, *x, *y) == Some(Target::Button(wanted))
                })
                .unwrap()
        };
        let (connect, quit) = (find(Button::Connect), find(Button::Quit));

        // Pressed on a button, released elsewhere: nothing.
        app.pointer_down(connect.0 + 3, connect.1 + 3);
        app.pointer_up(2, 2);
        assert!(attempts.borrow().is_empty());

        app.pointer_down(connect.0 + 3, connect.1 + 3);
        app.pointer_up(connect.0 + 3, connect.1 + 3);
        assert!(app.session().is_some());

        let (mut app, _) = self::app(None);
        app.pointer_down(quit.0 + 3, quit.1 + 3);
        app.pointer_up(quit.0 + 3, quit.1 + 3);
        assert!(app.should_quit());
    }

    #[test]
    fn quitting_goes_through_the_server_once_connected() {
        let (mut app, _) = app(Some("ada"));
        app.connect_with("secret");
        assert!(!app.should_quit());
        app.key(Key::F(3));
        assert!(app.should_quit());
        assert_eq!(app.session().unwrap().api().actions.last().unwrap().0, "quit");
    }

    #[test]
    fn a_change_pushed_by_the_server_asks_for_a_new_drawing() {
        use xila_core::api::Notification;
        let connect: Connect<Fake> = Box::new(|_| {
            let pending = vec![Notification::Screen { stack: None }];
            Session::start(Fake { pending, ..Fake::default() }).map_err(|error| error.to_string())
        });
        let mut app = App::new(options(Some("ada")), connect);
        assert!(!app.tick(), "nothing to poll before the connection");

        app.connect_with("anything");
        assert!(app.tick(), "the palette changed");
        assert!(!app.tick(), "and then nothing more");
        assert_eq!(app.palette().rgb(slot::CREAM), 0xf6ecd2);
    }
}
