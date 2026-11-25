//! Drawing the connection screen in the terminal.
//!
//! The form itself lives in [`xila_core::login`]; it is shared with the
//! graphical client.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use xila_core::login::{LoginForm, HINT, KEYS, TITLE};
use xila_core::text_layout::{COLUMNS, LABEL_WIDTH, MESSAGE_ROW};

use crate::screen::{draw_field, draw_keys, put};
use crate::theme::Theme;

const FIRST_ROW: u16 = 5;
const FIELD_WIDTH: u16 = 40;

/// Draws the form. Returns where the terminal cursor goes.
pub fn draw(form: &LoginForm, buffer: &mut Buffer, area: Rect) -> Option<(u16, u16)> {
    let theme = Theme::terminal();
    buffer.set_style(area, theme.base());
    put(buffer, area, 1, 0, "PGXILA", 20, theme.of(theme.title));
    let title_x = (COLUMNS - TITLE.chars().count() as u16) / 2;
    put(buffer, area, title_x, 0, TITLE, COLUMNS, theme.of(theme.title));
    put(buffer, area, 1, 2, HINT, COLUMNS - 2, theme.base());

    let mut cursor = None;
    for (index, field) in form.fields().iter().enumerate() {
        let at = draw_field(
            buffer,
            area,
            &theme,
            field.label,
            1,
            LABEL_WIDTH + 3,
            FIRST_ROW + index as u16,
            FIELD_WIDTH,
            &field.shown,
            field.cursor,
        );
        cursor = cursor.or(at);
    }

    if let Some(message) = &form.message {
        put(buffer, area, 1, MESSAGE_ROW, message, COLUMNS - 2, theme.of(theme.message));
    }
    draw_keys(buffer, area, &theme, KEYS.into_iter());
    cursor
}

#[cfg(test)]
mod tests {
    use super::*;
    use xila_core::input::Key;
    use xila_core::text_layout::ROWS;

    fn render(form: &LoginForm) -> Vec<String> {
        let area = Rect::new(0, 0, COLUMNS, ROWS);
        let mut buffer = Buffer::empty(area);
        draw(form, &mut buffer, area);
        (0..ROWS)
            .map(|y| {
                let line: String = (0..COLUMNS).map(|x| buffer[(x, y)].symbol()).collect();
                line.trim_end().to_owned()
            })
            .collect()
    }

    #[test]
    fn the_password_is_never_shown() {
        let mut form = LoginForm::new("localhost", 5419, "pgxila", Some("ada"));
        for c in "secret".chars() {
            form.handle(Key::Char(c));
        }
        let lines = render(&form);

        assert_eq!(lines[0].trim(), "PGXILA                             Sign in");
        assert_eq!(lines[5], " Server  . . . . :  localhost");
        assert_eq!(lines[8], " User  . . . . . :  ada");
        assert_eq!(lines[9], " Password  . . . :  ******");
        assert!(!lines.iter().any(|line| line.contains("secret")));
        assert_eq!(lines[23], " Enter=Sign in  F3=Quit");
    }
}
