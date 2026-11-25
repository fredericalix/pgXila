//! The connection form, and opening a session from it.
//!
//! The connection screen is the only screen built into the clients rather
//! than described in the database: it is shown before any connection exists.
//! What it contains and how its keys behave is shared; each client draws it.

use std::path::PathBuf;

use crate::connection::{ConnectError, ConnectParams};
use crate::input::{edit_line, Key};
use crate::server::Server;
use crate::session::Session;

pub const TITLE: &str = "Sign in";
pub const HINT: &str = "Sign in, then press Enter. The connection is always encrypted.";
/// The keys of the connection screen, as shown in the function key bar.
pub const KEYS: [(&str, &str); 2] = [("Enter", "Sign in"), ("F3", "Quit")];

const LABELS: [&str; 5] = ["Server", "Port", "Database", "User", "Password"];
const PASSWORD: usize = 4;
const MAX_LENGTH: usize = 200;

/// What the user wants after a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    Connect,
    Quit,
}

/// One line of the form, as a client draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormField {
    pub label: &'static str,
    /// What to show: the value, or one `*` per character of the password.
    pub shown: String,
    /// Position of the cursor, in characters, when the field has the focus.
    pub cursor: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct LoginForm {
    /// Server, port, database, user and password, in the order of the screen.
    values: [String; 5],
    focus: usize,
    cursor: usize,
    pub message: Option<String>,
}

impl LoginForm {
    pub fn new(host: &str, port: u16, database: &str, user: Option<&str>) -> Self {
        let mut form = LoginForm {
            values: [
                host.to_owned(),
                port.to_string(),
                database.to_owned(),
                user.unwrap_or_default().to_owned(),
                String::new(),
            ],
            ..LoginForm::default()
        };
        // Start where something is missing.
        form.focus_on(if user.is_some() { PASSWORD } else { PASSWORD - 1 });
        form
    }

    pub fn host(&self) -> &str {
        &self.values[0]
    }

    /// The port, when it is a valid one.
    pub fn port(&self) -> Option<u16> {
        self.values[1].trim().parse().ok().filter(|port| *port != 0)
    }

    pub fn database(&self) -> &str {
        &self.values[2]
    }

    pub fn user(&self) -> &str {
        &self.values[3]
    }

    pub fn password(&self) -> &str {
        &self.values[PASSWORD]
    }

    /// Forgets the password, after a connection attempt.
    pub fn clear_password(&mut self) {
        self.values[PASSWORD].clear();
        self.focus_on(PASSWORD);
    }

    /// The lines of the form, in the order of the screen. The password is
    /// never part of what is returned.
    pub fn fields(&self) -> Vec<FormField> {
        LABELS
            .iter()
            .enumerate()
            .map(|(index, label)| FormField {
                label,
                shown: if index == PASSWORD {
                    "*".repeat(self.values[index].chars().count())
                } else {
                    self.values[index].clone()
                },
                cursor: (index == self.focus).then_some(self.cursor),
            })
            .collect()
    }

    /// Puts the focus on a line of the form, for a client with a pointer.
    /// `cursor` counts characters; None puts it at the end.
    pub fn focus_field(&mut self, field: usize, cursor: Option<usize>) {
        if field < LABELS.len() {
            self.focus_on(field);
            if let Some(cursor) = cursor {
                self.cursor = cursor.min(self.cursor);
            }
        }
    }

    fn focus_on(&mut self, field: usize) {
        self.focus = field;
        self.cursor = self.values[field].chars().count();
    }

    pub fn handle(&mut self, key: Key) -> Outcome {
        match key {
            Key::F(3) | Key::Interrupt => return Outcome::Quit,
            Key::Tab | Key::Down => self.focus_on((self.focus + 1) % LABELS.len()),
            Key::BackTab | Key::Up => self.focus_on((self.focus + LABELS.len() - 1) % LABELS.len()),
            Key::Enter => {
                if self.user().is_empty() {
                    self.message = Some("Enter a user name.".into());
                } else if self.port().is_none() {
                    self.message = Some("The port must be a number between 1 and 65535.".into());
                } else {
                    self.message = None;
                    return Outcome::Connect;
                }
            }
            _ => {
                edit_line(&mut self.values[self.focus], &mut self.cursor, key, MAX_LENGTH);
            }
        }
        Outcome::Stay
    }

    /// What to connect with. `fallback_port` is used when the port typed is
    /// not valid, which [`LoginForm::handle`] does not let through.
    pub fn params(&self, fallback_port: u16, ca_file: Option<PathBuf>) -> ConnectParams {
        ConnectParams {
            host: self.host().to_owned(),
            port: self.port().unwrap_or(fallback_port),
            database: self.database().to_owned(),
            user: self.user().to_owned(),
            password: self.password().to_owned(),
            ca_file,
        }
    }
}

/// Connects and opens the home stack, or explains to the user why not.
pub fn open_session(params: &ConnectParams) -> Result<Session<Server>, String> {
    let server = Server::connect(params).map_err(|error| match &error {
        ConnectError::Postgres(postgres) => match postgres.as_db_error() {
            // Wrong password, unknown role, or a role that may not log in.
            Some(refusal) if refusal.code().code().starts_with("28") => {
                "Wrong user name or password.".to_owned()
            }
            Some(refusal) => format!("Connection refused: {}", refusal.message()),
            None => format!("Cannot connect: {}", root_cause(postgres)),
        },
        other => format!("Cannot connect: {other}"),
    })?;
    Session::start(server).map_err(|error| format!("This account cannot use pgXila: {error}"))
}

/// The innermost cause of an error, which is where the useful text is.
fn root_cause(error: &dyn std::error::Error) -> String {
    let mut cause = error;
    while let Some(inner) = cause.source() {
        cause = inner;
    }
    cause.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_text(form: &mut LoginForm, text: &str) {
        for c in text.chars() {
            form.handle(Key::Char(c));
        }
    }

    #[test]
    fn the_password_is_never_part_of_what_is_drawn() {
        let mut form = LoginForm::new("localhost", 5419, "pgxila", Some("ada"));
        type_text(&mut form, "secret");
        let fields = form.fields();

        assert_eq!(fields[0].label, "Server");
        assert_eq!(fields[0].shown, "localhost");
        assert_eq!(fields[3].shown, "ada");
        assert_eq!(fields[4].label, "Password");
        assert_eq!(fields[4].shown, "******");
        assert_eq!(fields[4].cursor, Some(6));
        assert!(fields.iter().all(|field| !field.shown.contains("secret")));
        assert_eq!(form.password(), "secret");
    }

    #[test]
    fn the_form_starts_on_the_first_missing_value() {
        let mut without_user = LoginForm::new("localhost", 5419, "pgxila", None);
        type_text(&mut without_user, "ada");
        assert_eq!(without_user.user(), "ada");

        let mut with_user = LoginForm::new("localhost", 5419, "pgxila", Some("ada"));
        type_text(&mut with_user, "x");
        assert_eq!(with_user.password(), "x");
    }

    #[test]
    fn enter_connects_only_when_the_form_is_complete() {
        let mut form = LoginForm::new("localhost", 5419, "pgxila", None);
        assert_eq!(form.handle(Key::Enter), Outcome::Stay);
        assert_eq!(form.message.as_deref(), Some("Enter a user name."));

        type_text(&mut form, "ada");
        assert_eq!(form.handle(Key::Enter), Outcome::Connect);
        assert_eq!(form.message, None);
    }

    #[test]
    fn a_port_that_is_not_a_number_is_refused() {
        let mut form = LoginForm::new("localhost", 5419, "pgxila", Some("ada"));
        form.handle(Key::BackTab);
        form.handle(Key::BackTab);
        form.handle(Key::BackTab);
        type_text(&mut form, "x");
        assert_eq!(form.handle(Key::Enter), Outcome::Stay);
        assert_eq!(form.port(), None);
    }

    #[test]
    fn f3_quits() {
        let mut form = LoginForm::new("localhost", 5419, "pgxila", None);
        assert_eq!(form.handle(Key::F(3)), Outcome::Quit);
    }

    #[test]
    fn a_pointer_can_choose_a_field_and_a_cursor_position() {
        let mut form = LoginForm::new("localhost", 5419, "pgxila", Some("ada"));
        form.focus_field(0, Some(5));
        type_text(&mut form, "-");
        assert_eq!(form.host(), "local-host");

        form.focus_field(3, None);
        type_text(&mut form, "!");
        assert_eq!(form.user(), "ada!");

        // A field that does not exist changes nothing.
        form.focus_field(9, None);
        type_text(&mut form, "?");
        assert_eq!(form.user(), "ada!?");
    }

    #[test]
    fn the_form_gives_the_connection_parameters() {
        let form = LoginForm::new("db.example.org", 5419, "pgxila", Some("ada"));
        let params = form.params(5419, Some(PathBuf::from("ca.crt")));
        assert_eq!(params.host, "db.example.org");
        assert_eq!(params.port, 5419);
        assert_eq!(params.user, "ada");
        assert_eq!(params.ca_file, Some(PathBuf::from("ca.crt")));
    }
}
