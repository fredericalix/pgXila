//! pgXila text client.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;
use ratatui::crossterm::event::{self, Event};
use ratatui::DefaultTerminal;
use xila_core::connection::ConnectParams;
use xila_core::input::Key;
use xila_core::login::{open_session, LoginForm, Outcome};
use xila_core::server::Server;
use xila_core::session::Session;
use xila_term::input::key_from_event;
use xila_term::login;
use xila_term::screen::{self, put, Ui};
use xila_term::theme::Theme;

/// How often the client looks for notifications when no key is pressed.
const TICK: Duration = Duration::from_millis(200);

/// pgXila text client: the screens of a pgXila server in a terminal.
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
}

enum Stage {
    Login(LoginForm),
    Session(Box<Session<Server>>, Ui),
}

fn main() -> ExitCode {
    let args = Args::parse();
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &args);
    ratatui::restore();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xila-term: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(terminal: &mut DefaultTerminal, args: &Args) -> std::io::Result<()> {
    let mut form = LoginForm::new(&args.host, args.port, &args.database, args.user.as_deref());

    // With a user and a password in the environment, connect straight away.
    let mut stage = match (&args.user, std::env::var("PGXILA_PASSWORD")) {
        (Some(user), Ok(password)) => {
            let params = ConnectParams {
                host: args.host.clone(),
                port: args.port,
                database: args.database.clone(),
                user: user.clone(),
                password,
                ca_file: args.ca.clone(),
            };
            match open_session(&params) {
                Ok(session) => Stage::Session(Box::new(session), Ui::default()),
                Err(message) => {
                    form.message = Some(message);
                    Stage::Login(form)
                }
            }
        }
        _ => Stage::Login(form),
    };

    loop {
        terminal.draw(|frame| {
            let whole = frame.area();
            let Some(area) = screen::screen_area(whole) else {
                let theme = Theme::terminal();
                let message = "pgXila needs a terminal of at least 80 columns by 24 rows.";
                put(frame.buffer_mut(), whole, 0, 0, message, whole.width, theme.base());
                return;
            };
            let cursor = match &stage {
                Stage::Login(form) => login::draw(form, frame.buffer_mut(), area),
                Stage::Session(session, ui) => screen::draw(session, ui, frame.buffer_mut(), area),
            };
            if let Some(position) = cursor {
                frame.set_cursor_position(position);
            }
        })?;

        let key = if event::poll(TICK)? {
            match event::read()? {
                Event::Key(event) => key_from_event(event),
                _ => None,
            }
        } else {
            None
        };
        if key == Some(Key::Interrupt) {
            return Ok(());
        }

        match &mut stage {
            Stage::Login(form) => match key.map(|key| form.handle(key)) {
                Some(Outcome::Quit) => return Ok(()),
                Some(Outcome::Connect) => {
                    let params = form.params(args.port, args.ca.clone());
                    form.clear_password();
                    match open_session(&params) {
                        Ok(session) => stage = Stage::Session(Box::new(session), Ui::default()),
                        Err(message) => form.message = Some(message),
                    }
                }
                _ => {}
            },
            Stage::Session(session, ui) => {
                if let Some(key) = key {
                    screen::handle(session, ui, key);
                }
                session.poll();
                if session.should_quit() {
                    return Ok(());
                }
            }
        }
    }
}
