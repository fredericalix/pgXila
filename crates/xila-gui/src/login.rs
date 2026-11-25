//! The connection screen, in pixels.
//!
//! The form itself lives in [`xila_core::login`]; it is shared with the text
//! client. This is the only screen the client draws without the database.

use xila_core::login::{LoginForm, HINT, KEYS, TITLE};

use crate::chrome::{
    self, content_of, draw_button, draw_field, draw_window, field_cursor_at, field_scroll,
    window_around, ButtonLook, WindowStyle, BUTTON_HEIGHT, FIELD_HEIGHT,
};
use crate::font::Fonts;
use crate::frame::{Frame, Rect};
use crate::palette::slot;

const WIDTH: i32 = 300;
const HEIGHT: i32 = 180;
const FIRST_FIELD: i32 = 44;
const FIELD_PITCH: i32 = 20;

/// The buttons of the connection screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Connect,
    Quit,
}

/// What is under the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// A line of the form, and the cursor position the pointer is on.
    Field {
        index: usize,
        cursor: usize,
    },
    Button(Button),
}

struct Geometry {
    window: Rect,
    content: Rect,
    /// The box of each line of the form.
    fields: Vec<Rect>,
    connect: Rect,
    quit: Rect,
}

fn geometry(form: &LoginForm, fonts: &Fonts) -> Geometry {
    let window = window_around(WIDTH, HEIGHT);
    let content = content_of(window);
    let lines = form.fields();
    let label_width = lines.iter().map(|line| fonts.regular.width(line.label)).max().unwrap_or(0);
    let field_x = content.x + 14 + label_width + 5;
    let fields = (0..lines.len() as i32)
        .map(|index| {
            let y = content.y + FIRST_FIELD + index * FIELD_PITCH;
            Rect::new(field_x, y, content.right() - 14 - field_x, FIELD_HEIGHT)
        })
        .collect();
    let buttons_y = content.y + FIRST_FIELD + lines.len() as i32 * FIELD_PITCH + 6;
    Geometry {
        window,
        content,
        fields,
        connect: Rect::new(field_x, buttons_y, 96, BUTTON_HEIGHT),
        quit: Rect::new(field_x + 101, buttons_y, 62, BUTTON_HEIGHT),
    }
}

/// The labels of the buttons, taken from the keys of the screen so that the
/// two always say the same thing.
fn button_label(button: Button) -> &'static str {
    match button {
        Button::Connect => KEYS[0].1,
        Button::Quit => KEYS[1].1,
    }
}

/// Draws the connection screen. `pressed` is the button the pointer is held
/// on, `blink` whether the cursor is in the visible half of its blinking.
pub fn draw(
    form: &LoginForm,
    pressed: Option<Button>,
    fonts: &Fonts,
    frame: &mut Frame,
    clock: &str,
    blink: bool,
) {
    let geometry = geometry(form, fonts);
    chrome::draw_desktop(frame);
    chrome::draw_menu_bar(frame, fonts, None, clock);
    draw_window(frame, fonts, geometry.window, TITLE, WindowStyle::SYSTEM);

    let content = geometry.content;
    // The hint is two sentences: one per line.
    for (line, sentence) in HINT.split_inclusive(". ").enumerate() {
        let baseline = content.y + 17 + line as i32 * 12;
        fonts.regular.draw(frame, content.x + 14, baseline, sentence.trim_end(), slot::INK);
    }

    for (line, area) in form.fields().iter().zip(&geometry.fields) {
        let label_x = area.x - 5 - fonts.regular.width(line.label);
        fonts.regular.draw(frame, label_x, area.y + 11, line.label, slot::COCOA);
        draw_field(frame, fonts, *area, &line.shown, line.cursor, blink);
    }

    for (button, area) in [(Button::Connect, geometry.connect), (Button::Quit, geometry.quit)] {
        let look = ButtonLook {
            primary: button == Button::Connect,
            pressed: pressed == Some(button),
            ..ButtonLook::default()
        };
        draw_button(frame, fonts, area, button_label(button), look);
    }

    if let Some(message) = &form.message {
        chrome::draw_message(frame, fonts, message);
    }
    chrome::draw_key_bar(frame, fonts, KEYS.into_iter());
}

/// What is under a point of the screen.
pub fn target_at(form: &LoginForm, fonts: &Fonts, x: i32, y: i32) -> Option<Target> {
    let geometry = geometry(form, fonts);
    if geometry.connect.contains(x, y) {
        return Some(Target::Button(Button::Connect));
    }
    if geometry.quit.contains(x, y) {
        return Some(Target::Button(Button::Quit));
    }
    let lines = form.fields();
    let index = geometry.fields.iter().position(|area| area.contains(x, y))?;
    let (line, area) = (&lines[index], geometry.fields[index]);
    let first = field_scroll(fonts, area, &line.shown, line.cursor);
    Some(Target::Field { index, cursor: field_cursor_at(fonts, area, &line.shown, first, x) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{count_slot, has_slot, snapshot};
    use xila_core::input::Key;

    fn form() -> LoginForm {
        let mut form = LoginForm::new("localhost", 5419, "pgxila", Some("ada"));
        for c in "secret".chars() {
            form.handle(Key::Char(c));
        }
        form
    }

    #[test]
    fn the_connection_screen_is_a_window_with_five_fields_and_two_buttons() {
        let fonts = Fonts::builtin();
        let form = form();
        let mut frame = Frame::new();
        draw(&form, None, &fonts, &mut frame, "16:54", true);
        snapshot(&frame, "login");

        let geometry = geometry(&form, &fonts);
        assert_eq!(geometry.fields.len(), 5);
        assert!(geometry.fields.iter().all(|area| frame.pixel(area.x, area.y) == Some(slot::INK)));
        // No connection yet: no green light in the menu bar.
        assert!(!has_slot(&frame, Rect::new(0, 0, 480, 14), slot::GREEN));
        // The password has the focus; the others do not.
        let password = geometry.fields[4];
        assert_eq!(frame.pixel(password.x - 1, password.y - 1), Some(slot::BLUE));
        assert_eq!(frame.pixel(password.x - 1, geometry.fields[3].y - 1), Some(slot::CREAM));
        assert!(count_slot(&frame, geometry.connect, slot::YELLOW) > 500);
        assert!(count_slot(&frame, geometry.quit, slot::PEARL) > 300);
        assert!(geometry.quit.right() <= geometry.content.right() - 14);
        assert!(geometry.quit.bottom() < geometry.content.bottom());
    }

    #[test]
    fn a_message_is_shown_in_a_strip() {
        let fonts = Fonts::builtin();
        let mut form = form();
        let strip = Rect::new(0, chrome::KEYS_TOP - 13, 480, 12);
        let mut frame = Frame::new();
        draw(&form, None, &fonts, &mut frame, "16:54", true);
        assert!(!has_slot(&frame, strip, slot::YELLOW));

        form.message = Some("Wrong user name or password.".into());
        draw(&form, None, &fonts, &mut frame, "16:54", true);
        assert!(count_slot(&frame, strip, slot::YELLOW) > 4000);
    }

    #[test]
    fn the_pointer_finds_fields_and_buttons() {
        let fonts = Fonts::builtin();
        let form = form();
        let geometry = geometry(&form, &fonts);

        let host = geometry.fields[0];
        let after_two = host.x + chrome::FIELD_TEXT_X + fonts.regular.width("lo");
        assert_eq!(
            target_at(&form, &fonts, after_two, host.y + 5),
            Some(Target::Field { index: 0, cursor: 2 })
        );
        assert_eq!(
            target_at(&form, &fonts, host.right() - 3, geometry.fields[3].y + 5),
            Some(Target::Field { index: 3, cursor: 3 })
        );
        let connect = geometry.connect;
        assert_eq!(
            target_at(&form, &fonts, connect.x + 10, connect.y + 5),
            Some(Target::Button(Button::Connect))
        );
        assert_eq!(
            target_at(&form, &fonts, geometry.quit.x + 10, connect.y + 5),
            Some(Target::Button(Button::Quit))
        );
        assert_eq!(target_at(&form, &fonts, 2, 2), None);
    }
}
