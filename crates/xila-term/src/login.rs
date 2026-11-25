//! The connection screen.
//!
//! It is the only screen built into the client rather than described in the
//! database: it is shown before any connection exists.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use xila_core::text_layout::{COLUMNS, LABEL_WIDTH, MESSAGE_ROW};

use crate::input::{edit_line, Key};
use crate::screen::{draw_field, draw_keys, put};
use crate::theme::Theme;

const LABELS: [&str; 5] = ["Serveur", "Port", "Base", "Utilisateur", "Mot de passe"];
const PASSWORD: usize = 4;
const FIRST_ROW: u16 = 5;
const FIELD_WIDTH: u16 = 40;
const MAX_LENGTH: usize = 200;

/// What the user wants after a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    Connect,
    Quit,
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
        self.values[1].trim().parse().ok()
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
                    self.message = Some("Indiquez un nom d'utilisateur.".into());
                } else if self.port().is_none() {
                    self.message = Some("Le port doit être un nombre entre 1 et 65535.".into());
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

    /// Draws the form. Returns where the terminal cursor goes.
    pub fn draw(&self, buffer: &mut Buffer, area: Rect) -> Option<(u16, u16)> {
        let theme = Theme::terminal();
        buffer.set_style(area, theme.base());
        put(buffer, area, 1, 0, "PGXILA", 20, theme.of(theme.title));
        put(buffer, area, (COLUMNS - 9) / 2, 0, "Connexion", COLUMNS, theme.of(theme.title));
        put(
            buffer,
            area,
            1,
            2,
            "Identifiez-vous, puis appuyez sur Entrée. La connexion est toujours chiffrée.",
            COLUMNS - 2,
            theme.base(),
        );

        let mut cursor = None;
        for (index, label) in LABELS.iter().enumerate() {
            let shown = if index == PASSWORD {
                "*".repeat(self.values[index].chars().count())
            } else {
                self.values[index].clone()
            };
            let at = draw_field(
                buffer,
                area,
                &theme,
                label,
                1,
                LABEL_WIDTH + 3,
                FIRST_ROW + index as u16,
                FIELD_WIDTH,
                &shown,
                (index == self.focus).then_some(self.cursor),
            );
            cursor = cursor.or(at);
        }

        if let Some(message) = &self.message {
            put(buffer, area, 1, MESSAGE_ROW, message, COLUMNS - 2, theme.of(theme.message));
        }
        draw_keys(buffer, area, &theme, [("Enter", "Se connecter"), ("F3", "Quitter")].into_iter());
        cursor
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xila_core::text_layout::ROWS;

    fn render(form: &LoginForm) -> Vec<String> {
        let area = Rect::new(0, 0, COLUMNS, ROWS);
        let mut buffer = Buffer::empty(area);
        form.draw(&mut buffer, area);
        (0..ROWS)
            .map(|y| {
                let line: String = (0..COLUMNS).map(|x| buffer[(x, y)].symbol()).collect();
                line.trim_end().to_owned()
            })
            .collect()
    }

    fn type_text(form: &mut LoginForm, text: &str) {
        for c in text.chars() {
            form.handle(Key::Char(c));
        }
    }

    #[test]
    fn the_password_is_never_shown() {
        let mut form = LoginForm::new("localhost", 5419, "pgxila", Some("ada"));
        type_text(&mut form, "secret");
        let lines = render(&form);

        assert_eq!(lines[5], " Serveur . . . . :  localhost");
        assert_eq!(lines[8], " Utilisateur . . :  ada");
        assert_eq!(lines[9], " Mot de passe  . :  ******");
        assert!(!lines.iter().any(|line| line.contains("secret")));
        assert_eq!(form.password(), "secret");
        assert_eq!(lines[23], " Entrée=Se connecter  F3=Quitter");
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
        assert_eq!(form.message.as_deref(), Some("Indiquez un nom d'utilisateur."));

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
}
