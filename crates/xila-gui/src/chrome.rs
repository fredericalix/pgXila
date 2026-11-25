//! The decor every screen shares: desktop, menu bar, function key bar, window
//! frame, and the look of buttons and fields.
//!
//! The measurements come from the mockups, pixel for pixel.

use crate::font::Fonts;
use crate::frame::{Frame, Rect, Sprite, HEIGHT, WIDTH};
use crate::palette::slot;

/// Height of the menu bar, without the line under it.
pub const MENU_HEIGHT: i32 = 14;
/// First row of the function key bar.
pub const KEYS_TOP: i32 = HEIGHT - 14;
/// The desktop: what is left between the menu bar and the function key bar.
pub const DESKTOP: Rect = Rect::new(0, MENU_HEIGHT + 1, WIDTH, KEYS_TOP - MENU_HEIGHT - 1);

/// Height of the title bar of a window, without the frame.
pub const TITLE_HEIGHT: i32 = 12;
/// What the frame of a window adds around its content: the border, the title
/// bar and the line under it.
pub const FRAME_TOP: i32 = 1 + TITLE_HEIGHT + 1;
pub const FRAME_SIDE: i32 = 1;
/// Width of the shadow a window casts on its right and under it.
const SHADOW: i32 = 3;

pub const BUTTON_HEIGHT: i32 = 15;
pub const FIELD_HEIGHT: i32 = 14;

fn key(slots: &[(char, u8)]) -> Vec<(char, u8)> {
    slots.to_vec()
}

/// The pgXila logo of the menu bar: three overlapping windows.
pub fn logo() -> Sprite {
    Sprite::from_art(
        &[
            "########....",
            "#tttttt#....",
            "#t########..",
            "#t#yyyyyy#..",
            "#t#y########",
            "###y#rrrrrr#",
            "..#y#rrrrrr#",
            "..###rrrrrr#",
            "....#rrrrrr#",
            "....########",
        ],
        &key(&[('#', slot::INK), ('t', slot::TURQUOISE), ('y', slot::YELLOW), ('r', slot::RED)]),
    )
}

/// The same logo, larger, for the banner of a system screen.
pub fn large_logo() -> Sprite {
    Sprite::from_art(
        &[
            "#############..........",
            "#ttttttttttt#..........",
            "#ttttttttttt#..........",
            "#ttttttttttt#..........",
            "#ttttttttttt#..........",
            "#tttt#############.....",
            "#tttt#yyyyyyyyyyy#.....",
            "#tttt#yyyyyyyyyyy#.....",
            "#tttt#yyyyyyyyyyy#.....",
            "######yyyyyyyyyyy#.....",
            ".....#yyyy#############",
            ".....#yyyy#rrrrrrrrrrr#",
            ".....#yyyy#rrrrrrrrrrr#",
            ".....#yyyy#rrrrrrrrrrr#",
            ".....######rrrrrrrrrrr#",
            "..........#rrrrrrrrrrr#",
            "..........#rrrrrrrrrrr#",
            "..........#rrrrrrrrrrr#",
            "..........#rrrrrrrrrrr#",
            "..........#############",
        ],
        &key(&[('#', slot::INK), ('t', slot::TURQUOISE), ('y', slot::YELLOW), ('r', slot::RED)]),
    )
}

/// The arrow of the "previous" and "next" buttons.
pub fn arrow(to_the_left: bool) -> Sprite {
    let left = ["....#", "..###", ".####", "#####", ".####", "..###", "....#"];
    let rows: Vec<String> = left
        .iter()
        .map(|row| if to_the_left { row.to_string() } else { row.chars().rev().collect() })
        .collect();
    let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
    Sprite::from_art(&rows, &[('#', slot::INK)])
}

/// The background of the desktop: night blue fading into navy.
pub fn draw_desktop(frame: &mut Frame) {
    frame.gradient(DESKTOP, slot::NIGHT, slot::NAVY);
}

/// The bar at the top: the logo and the name of the system on the left, where
/// we are connected and what time it is on the right.
pub fn draw_menu_bar(frame: &mut Frame, fonts: &Fonts, status: Option<&str>, clock: &str) {
    frame.fill(Rect::new(0, 0, WIDTH, MENU_HEIGHT), slot::CREAM);
    frame.hline(0, MENU_HEIGHT, WIDTH, slot::INK);
    frame.sprite(&logo(), 5, 2);
    fonts.bold.draw(frame, 22, 11, "pgXila", slot::INK);

    let clock_x = WIDTH - 6 - fonts.regular.width(clock);
    fonts.regular.draw(frame, clock_x, 11, clock, slot::INK);
    if let Some(status) = status {
        let x = clock_x - 12 - fonts.regular.width(status);
        fonts.regular.draw(frame, x, 11, status, slot::INK);
        // The light that says the connection is alive.
        frame.fill(Rect::new(x - 10, 5, 5, 5), slot::INK);
        frame.fill(Rect::new(x - 9, 6, 3, 3), slot::GREEN);
    }
}

/// Where the keys of the function key bar are, to draw them and to click on
/// them: the name of each key and its rectangle.
pub fn key_slots<'a>(
    fonts: &Fonts,
    keys: impl Iterator<Item = (&'a str, &'a str)>,
) -> Vec<(&'a str, &'a str, Rect)> {
    let mut slots = Vec::new();
    let mut x = 9;
    for (name, label) in keys {
        let width = fonts.bold.width(name) + 3 + fonts.regular.width(label);
        if x + width > WIDTH - 4 {
            break;
        }
        slots.push((name, label, Rect::new(x - 3, KEYS_TOP, width + 6, HEIGHT - KEYS_TOP)));
        x += width + 14;
    }
    slots
}

/// The bar at the bottom: the keys of the screen and what they do.
pub fn draw_key_bar<'a>(
    frame: &mut Frame,
    fonts: &Fonts,
    keys: impl Iterator<Item = (&'a str, &'a str)>,
) {
    frame.fill(Rect::new(0, KEYS_TOP, WIDTH, HEIGHT - KEYS_TOP), slot::INK);
    for (name, label, area) in key_slots(fonts, keys) {
        let x = fonts.bold.draw(frame, area.x + 3, KEYS_TOP + 11, name, slot::YELLOW);
        fonts.regular.draw(frame, x + 3, KEYS_TOP + 11, label, slot::CREAM);
    }
}

/// A message for the user, in a strip just above the function key bar.
pub fn draw_message(frame: &mut Frame, fonts: &Fonts, message: &str) {
    let strip = Rect::new(0, KEYS_TOP - 14, WIDTH, 14);
    frame.fill(strip, slot::YELLOW);
    frame.hline(0, strip.y, WIDTH, slot::INK);
    let previous = frame.clip_to(strip.inset(1));
    fonts.regular.draw(frame, 9, strip.y + 11, message, slot::INK);
    frame.set_clip(previous);
}

/// The colours of a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowStyle {
    pub title_bar: u8,
    pub body: u8,
}

impl WindowStyle {
    /// The screens of the system itself.
    pub const SYSTEM: WindowStyle = WindowStyle { title_bar: slot::AZURE, body: slot::CREAM };
    /// The stacks people build.
    pub const STACK: WindowStyle = WindowStyle { title_bar: slot::AMBER, body: slot::SAND };
}

/// The outer rectangle of a window whose content has a given size, centred
/// on the desktop.
pub fn window_around(content_w: i32, content_h: i32) -> Rect {
    let w = content_w + 2 * FRAME_SIDE;
    let h = content_h + FRAME_TOP + 1;
    Rect::new(DESKTOP.x + (DESKTOP.w - w) / 2, DESKTOP.y + (DESKTOP.h - h + 1) / 2, w, h)
}

/// The content area of a window, from its outer rectangle.
pub fn content_of(window: Rect) -> Rect {
    Rect::new(
        window.x + FRAME_SIDE,
        window.y + FRAME_TOP,
        window.w - 2 * FRAME_SIDE,
        window.h - FRAME_TOP - 1,
    )
}

/// The close box of a window, from its outer rectangle.
pub fn close_box_of(window: Rect) -> Rect {
    Rect::new(window.x + 4, window.y + 3, 8, 8)
}

/// Draws a window: shadow, frame, striped title bar with its close box and
/// its title, and an empty body.
pub fn draw_window(
    frame: &mut Frame,
    fonts: &Fonts,
    window: Rect,
    title: &str,
    style: WindowStyle,
) {
    // The shadow lets the desktop show through.
    frame.checker(Rect::new(window.right(), window.y + SHADOW, SHADOW, window.h), slot::INK);
    frame.checker(
        Rect::new(window.x + SHADOW, window.bottom(), window.w - SHADOW, SHADOW),
        slot::INK,
    );

    frame.fill(window, style.body);
    frame.outline(window, slot::INK);
    let bar = Rect::new(window.x + 1, window.y + 1, window.w - 2, TITLE_HEIGHT);
    frame.fill(bar, style.title_bar);
    frame.hline(window.x, bar.bottom(), window.w, slot::INK);

    let close = close_box_of(window);
    frame.fill(close, slot::CREAM);
    frame.outline(close, slot::INK);

    // Stripes on both sides of the title.
    let stripes = Rect::new(close.right() + 4, bar.y + 2, bar.right() - 3 - close.right() - 4, 7);
    for y in (stripes.y..stripes.bottom()).step_by(2) {
        frame.hline(stripes.x, y, stripes.w, slot::WHITE);
    }
    let width = fonts.bold.width(title).min(stripes.w - 16);
    let title_x = window.x + (window.w - width) / 2;
    frame.fill(Rect::new(title_x - 5, bar.y, width + 10, TITLE_HEIGHT), style.title_bar);
    let previous = frame.clip_to(Rect::new(title_x, bar.y, width, TITLE_HEIGHT));
    fonts.bold.draw(frame, title_x, bar.y + 10, title, slot::INK);
    frame.set_clip(previous);
}

/// A sheet of paper: where the card of a stack is written.
pub fn draw_paper(frame: &mut Frame, area: Rect) {
    frame.fill(area, slot::CREAM);
    frame.outline(area, slot::INK);
    // Its shadow lets the body of the window show through.
    frame.checker(Rect::new(area.right(), area.y + 2, 2, area.h), slot::INK);
    frame.checker(Rect::new(area.x + 2, area.bottom(), area.w - 2, 2), slot::INK);
}

/// What a button looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ButtonLook {
    /// The main button of a screen is yellow.
    pub primary: bool,
    /// Drawn pushed in, while the pointer is held on it.
    pub pressed: bool,
    /// The keyboard is on it.
    pub focused: bool,
    /// An arrow on its left or right side.
    pub arrow: Option<Arrow>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrow {
    Left,
    Right,
}

/// The width a button needs for its label.
pub fn button_width(fonts: &Fonts, label: &str, look: ButtonLook) -> i32 {
    let arrow = if look.arrow.is_some() { 12 } else { 0 };
    (fonts.regular.width(label) + 24 + arrow).max(62)
}

/// Draws a button: rounded ink border, light on top and left, shade on the
/// bottom and right, and its label in the middle.
pub fn draw_button(frame: &mut Frame, fonts: &Fonts, area: Rect, label: &str, look: ButtonLook) {
    let face = if look.primary { slot::YELLOW } else { slot::PEARL };
    let (light, shade) = if look.pressed { (slot::GREY, face) } else { (slot::WHITE, slot::GREY) };

    // The border, without its four corners.
    frame.hline(area.x + 1, area.y, area.w - 2, slot::INK);
    frame.hline(area.x + 1, area.bottom() - 1, area.w - 2, slot::INK);
    frame.vline(area.x, area.y + 1, area.h - 2, slot::INK);
    frame.vline(area.right() - 1, area.y + 1, area.h - 2, slot::INK);

    let inside = area.inset(1);
    frame.fill(inside, face);
    frame.hline(inside.x, inside.y, inside.w, light);
    frame.vline(inside.x, inside.y, inside.h - 1, light);
    frame.hline(inside.x, inside.bottom() - 1, inside.w, shade);
    frame.vline(inside.right() - 1, inside.y, inside.h, shade);

    let push = i32::from(look.pressed);
    let previous = frame.clip_to(inside.inset(1));
    let text_width = fonts.regular.width(label);
    let mut text_x = area.x + (area.w - text_width) / 2;
    match look.arrow {
        Some(Arrow::Left) => {
            frame.sprite(&arrow(true), area.x + 8 + push, area.y + 4 + push);
            text_x += 5;
        }
        Some(Arrow::Right) => {
            frame.sprite(&arrow(false), area.right() - 13 + push, area.y + 4 + push);
            text_x -= 5;
        }
        None => {}
    }
    fonts.regular.draw(frame, text_x + push, area.y + 11 + push, label, slot::INK);
    frame.set_clip(previous);

    if look.focused {
        // The same blue border as a field that has the focus.
        let ring = area.inset(-1);
        frame.hline(ring.x + 2, ring.y, ring.w - 4, slot::BLUE);
        frame.hline(ring.x + 2, ring.bottom() - 1, ring.w - 4, slot::BLUE);
        frame.vline(ring.x, ring.y + 2, ring.h - 4, slot::BLUE);
        frame.vline(ring.right() - 1, ring.y + 2, ring.h - 4, slot::BLUE);
        for (x, y) in [
            (ring.x + 1, ring.y + 1),
            (ring.right() - 2, ring.y + 1),
            (ring.x + 1, ring.bottom() - 2),
            (ring.right() - 2, ring.bottom() - 2),
        ] {
            frame.set(x, y, slot::BLUE);
        }
    }
}

/// Where the text of a field starts, from the left of its box.
pub const FIELD_TEXT_X: i32 = 5;

/// Which part of a text a field shows, so that the cursor stays in sight:
/// the index of the first character drawn.
pub fn field_scroll(fonts: &Fonts, area: Rect, text: &str, cursor: Option<usize>) -> usize {
    let room = area.w - 2 * FIELD_TEXT_X;
    let Some(cursor) = cursor else { return 0 };
    let characters: Vec<char> = text.chars().collect();
    let cursor = cursor.min(characters.len());
    let mut first = 0;
    loop {
        let shown: String = characters[first..cursor].iter().collect();
        if fonts.regular.width(&shown) < room || first >= cursor {
            return first;
        }
        first += 1;
    }
}

/// Draws the box of a field and its text. With a cursor position, the field
/// has the focus: it gets a blue border and the cursor is drawn when
/// `cursor_visible` says so (it blinks).
pub fn draw_field(
    frame: &mut Frame,
    fonts: &Fonts,
    area: Rect,
    text: &str,
    cursor: Option<usize>,
    cursor_visible: bool,
) {
    if cursor.is_some() {
        frame.outline(area.inset(-1), slot::BLUE);
    }
    frame.fill(area, slot::WHITE);
    frame.outline(area, slot::INK);
    frame.hline(area.x + 1, area.y + 1, area.w - 2, slot::PEARL);

    let first = field_scroll(fonts, area, text, cursor);
    let shown: String = text.chars().skip(first).collect();
    let previous = frame.clip_to(area.inset(2));
    let baseline = area.y + 11;
    fonts.regular.draw(frame, area.x + FIELD_TEXT_X, baseline, &shown, slot::INK);
    if let (Some(cursor), true) = (cursor, cursor_visible) {
        let before: String = shown.chars().take(cursor.saturating_sub(first)).collect();
        let x = area.x + FIELD_TEXT_X + fonts.regular.width(&before);
        frame.vline(x - 1, area.y + 3, 9, slot::INK);
    }
    frame.set_clip(previous);
}

/// The character position a click falls on in a field: the gap between two
/// characters nearest to the pointer.
pub fn field_cursor_at(fonts: &Fonts, area: Rect, text: &str, first: usize, x: i32) -> usize {
    let mut pen = area.x + FIELD_TEXT_X;
    let mut position = first;
    for character in text.chars().skip(first) {
        let advance = fonts.regular.width(character.encode_utf8(&mut [0; 4]));
        if x < pen + (advance + 1) / 2 {
            break;
        }
        pen += advance;
        position += 1;
    }
    position
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fonts() -> Fonts {
        Fonts::builtin()
    }

    fn has_ink(frame: &Frame, area: Rect, slot: u8) -> bool {
        (area.y..area.bottom())
            .any(|y| (area.x..area.right()).any(|x| frame.pixel(x, y) == Some(slot)))
    }

    #[test]
    fn the_bars_frame_the_desktop() {
        let mut frame = Frame::new();
        let fonts = fonts();
        draw_desktop(&mut frame);
        draw_menu_bar(&mut frame, &fonts, Some("normandie-01"), "16:54");
        draw_key_bar(&mut frame, &fonts, [("F3", "Quit")].into_iter());

        assert_eq!(frame.pixel(100, 0), Some(slot::CREAM));
        assert_eq!(frame.pixel(100, MENU_HEIGHT), Some(slot::INK));
        assert_eq!(frame.pixel(0, MENU_HEIGHT + 1), Some(slot::NIGHT));
        assert_eq!(frame.pixel(0, KEYS_TOP - 1), Some(slot::NAVY));
        assert_eq!(frame.pixel(200, KEYS_TOP), Some(slot::INK));
        // The logo, the connection light, the key and its label.
        assert_eq!(frame.pixel(6, 3), Some(slot::TURQUOISE));
        assert!(has_ink(&frame, Rect::new(300, 5, 150, 5), slot::GREEN));
        assert!(has_ink(&frame, Rect::new(9, KEYS_TOP, 14, 14), slot::YELLOW));
        assert!(has_ink(&frame, Rect::new(26, KEYS_TOP, 40, 14), slot::CREAM));
    }

    #[test]
    fn the_menu_bar_shows_no_connection_before_there_is_one() {
        let mut frame = Frame::new();
        draw_menu_bar(&mut frame, &fonts(), None, "16:54");
        assert!(!has_ink(&frame, Rect::new(0, 0, WIDTH, MENU_HEIGHT), slot::GREEN));
    }

    #[test]
    fn keys_that_do_not_fit_are_left_out_of_the_bar() {
        let fonts = fonts();
        let many: Vec<(String, String)> =
            (1..=24).map(|n| (format!("F{n}"), "An action".to_owned())).collect();
        let slots = key_slots(&fonts, many.iter().map(|(k, l)| (k.as_str(), l.as_str())));
        assert!(slots.len() < many.len());
        assert!(slots.iter().all(|(_, _, area)| area.right() <= WIDTH));
        assert!(slots.windows(2).all(|pair| pair[0].2.right() <= pair[1].2.x));
    }

    #[test]
    fn a_window_is_centred_and_its_content_is_inside_the_frame() {
        let window = window_around(406, 243);
        assert_eq!(window, Rect::new(36, 22, 408, 258), "the window of the mockups");
        assert_eq!(content_of(window), Rect::new(37, 36, 406, 243));

        let mut frame = Frame::new();
        let fonts = fonts();
        let title = "Address book";
        draw_desktop(&mut frame);
        draw_window(&mut frame, &fonts, window, title, WindowStyle::STACK);
        assert_eq!(frame.pixel(36, 22), Some(slot::INK));
        assert_eq!(frame.pixel(38, 24), Some(slot::AMBER));
        assert_eq!(frame.pixel(41, 26), Some(slot::CREAM), "the close box");
        assert_eq!(frame.pixel(60, 25), Some(slot::WHITE), "a stripe");
        assert_eq!(frame.pixel(60, 26), Some(slot::AMBER));
        assert_eq!(frame.pixel(100, 35), Some(slot::INK));
        assert_eq!(frame.pixel(100, 36), Some(slot::SAND));
        assert_eq!(frame.pixel(443, 279), Some(slot::INK));
        // The title sits on a gap in the stripes; the shadow is a checker.
        let title_x = 240 - fonts.bold.width(title) / 2;
        assert_eq!(frame.pixel(title_x - 3, 25), Some(slot::AMBER));
        assert!(has_ink(&frame, Rect::new(200, 24, 80, 10), slot::INK));
        assert_ne!(frame.pixel(444, 100), frame.pixel(445, 100));
    }

    #[test]
    fn a_button_shows_its_kind_and_its_state() {
        let fonts = fonts();
        let area = Rect::new(10, 10, 62, BUTTON_HEIGHT);
        let mut frame = Frame::new();
        frame.fill(Rect::SCREEN, slot::SAND);

        draw_button(&mut frame, &fonts, area, "Prev.", ButtonLook::default());
        assert_eq!(frame.pixel(10, 10), Some(slot::SAND), "corners are rounded");
        assert_eq!(frame.pixel(11, 10), Some(slot::INK));
        assert_eq!(frame.pixel(14, 13), Some(slot::PEARL));
        assert_eq!(frame.pixel(14, 11), Some(slot::WHITE));
        assert_eq!(frame.pixel(14, 23), Some(slot::GREY));

        let primary = ButtonLook { primary: true, ..ButtonLook::default() };
        draw_button(&mut frame, &fonts, area, "New", primary);
        assert_eq!(frame.pixel(14, 13), Some(slot::YELLOW));

        let pressed = ButtonLook { pressed: true, ..ButtonLook::default() };
        draw_button(&mut frame, &fonts, area, "Prev.", pressed);
        assert_eq!(frame.pixel(14, 11), Some(slot::GREY));

        // The keyboard is on it: a blue border around it, corners rounded.
        assert_eq!(frame.pixel(20, 9), Some(slot::SAND));
        let focused = ButtonLook { focused: true, ..ButtonLook::default() };
        draw_button(&mut frame, &fonts, area, "Prev.", focused);
        assert_eq!(frame.pixel(20, 9), Some(slot::BLUE));
        assert_eq!(frame.pixel(9, 9), Some(slot::SAND));
        assert_eq!(frame.pixel(10, 10), Some(slot::BLUE));
    }

    #[test]
    fn a_button_is_wide_enough_for_its_label_and_its_arrow() {
        let fonts = fonts();
        let plain = ButtonLook::default();
        let with_arrow = ButtonLook { arrow: Some(Arrow::Left), ..plain };
        assert_eq!(button_width(&fonts, "Next", plain), 62);
        let long = button_width(&fonts, "A new card for the address book", plain);
        assert_eq!(long, fonts.regular.width("A new card for the address book") + 24);
        assert_eq!(button_width(&fonts, "A new card for the address book", with_arrow), long + 12);
    }

    #[test]
    fn a_field_with_the_focus_has_a_blue_border_and_a_cursor() {
        let fonts = fonts();
        let area = Rect::new(20, 20, 100, FIELD_HEIGHT);
        let mut frame = Frame::new();
        frame.fill(Rect::SCREEN, slot::CREAM);

        draw_field(&mut frame, &fonts, area, "Ada", None, true);
        assert_eq!(frame.pixel(19, 19), Some(slot::CREAM));
        assert_eq!(frame.pixel(20, 20), Some(slot::INK));
        assert_eq!(frame.pixel(30, 21), Some(slot::PEARL));
        assert_eq!(frame.pixel(110, 27), Some(slot::WHITE));

        draw_field(&mut frame, &fonts, area, "Ada", Some(3), true);
        assert_eq!(frame.pixel(19, 19), Some(slot::BLUE));
        let cursor_x = 20 + FIELD_TEXT_X + fonts.regular.width("Ada") - 1;
        assert_eq!(frame.pixel(cursor_x, 25), Some(slot::INK));

        // The cursor blinks: same field, cursor hidden.
        draw_field(&mut frame, &fonts, area, "Ada", Some(3), false);
        assert_eq!(frame.pixel(cursor_x, 23), Some(slot::WHITE));
    }

    #[test]
    fn a_long_value_scrolls_to_keep_the_cursor_in_sight() {
        let fonts = fonts();
        let area = Rect::new(0, 0, 60, FIELD_HEIGHT);
        let text = "Chantenay-Saint-Imbert";
        assert_eq!(field_scroll(&fonts, area, text, None), 0);
        assert_eq!(field_scroll(&fonts, area, text, Some(3)), 0);

        let first = field_scroll(&fonts, area, text, Some(22));
        assert!(first > 0);
        let visible: String = text.chars().skip(first).collect();
        assert!(fonts.regular.width(&visible) < 60 - 2 * FIELD_TEXT_X);
    }

    #[test]
    fn a_click_in_a_field_lands_between_two_characters() {
        let fonts = fonts();
        let area = Rect::new(100, 0, 200, FIELD_HEIGHT);
        let start = 100 + FIELD_TEXT_X;
        assert_eq!(field_cursor_at(&fonts, area, "Ada", 0, 0), 0);
        assert_eq!(field_cursor_at(&fonts, area, "Ada", 0, start + 1), 0);
        assert_eq!(field_cursor_at(&fonts, area, "Ada", 0, start + fonts.regular.width("A")), 1);
        assert_eq!(field_cursor_at(&fonts, area, "Ada", 0, 290), 3);
        // With the text scrolled, positions count from the first character shown.
        assert_eq!(field_cursor_at(&fonts, area, "Ada", 2, start + 1), 2);
    }
}
