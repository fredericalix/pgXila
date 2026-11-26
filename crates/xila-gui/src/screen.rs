//! Drawing a session in pixels, and turning keys and clicks into intents.
//!
//! Nothing here knows what a stack does. The placement comes from
//! [`crate::layout`], the content from the session, and every key or click
//! that means something is sent to the server as an action.

use xila_core::api::{Api, Widget};
use xila_core::input::{edit_line, Key};
use xila_core::session::{GridState, Question, Session, View};

use crate::chrome::{
    self, button_width, close_box_of, content_of, draw_button, draw_check, draw_choice, draw_field,
    draw_paper, draw_window, field_cursor_at, field_scroll, window_around, wrap, ButtonLook,
    WindowStyle, BUTTON_HEIGHT,
};
use crate::design::{self, Designer};
use crate::draw::{self, Painter};
use crate::font::Fonts;
use crate::frame::{Frame, Rect};
use crate::layout::{
    self, is_icon_view, FieldKind, GraphicLayout, Item, Part, GRID_HEADER, GRID_ROW, TILE,
};
use crate::palette::slot;
use crate::script::{self, Scripter};
use crate::talk;

/// Longest text a field accepts when its widget sets no limit.
const DEFAULT_MAX_LENGTH: usize = 200;
/// Width of a right-aligned column of a list, such as a count.
const NARROW_COLUMN: i32 = 52;
/// Height of a line of a text written on several lines.
const TEXT_LINE: i32 = 11;

/// What the graphical client remembers between two events, besides the
/// session.
#[derive(Debug, Default)]
pub struct Ui {
    /// Position of the cursor in the focused field, in characters.
    cursor: usize,
    /// What the cursor position refers to: stack, card and widget.
    anchor: Option<(String, Option<String>, String)>,
    /// The button the pointer is held on.
    pressed: Option<String>,
    /// How far back the user went in a conversation, in pixels from its end.
    talk_back: i32,
    /// What the visual editor remembers, while a stack is open in it.
    pub designer: Designer,
    /// What the drawing editor remembers, while a picture is open in it.
    pub painter: Painter,
    /// What the script editor remembers, while a script is open in it.
    pub scripter: Scripter,
}

/// Where everything is on the screen, for one view.
struct Geometry {
    layout: GraphicLayout,
    window: Rect,
    content: Rect,
}

impl Geometry {
    fn of(view: &View, fonts: &Fonts) -> Self {
        let layout = layout::layout(&view.screen, &view.card.widgets, fonts);
        let window = window_around(layout.width, layout.height);
        Geometry { content: content_of(window), window, layout }
    }

    /// The rectangle of an item on the screen.
    fn on_screen(&self, area: Rect) -> Rect {
        area.moved(self.content.x, self.content.y)
    }

    /// The item under a point of the screen. What is drawn last is on top.
    fn item_at(&self, x: i32, y: i32) -> Option<&Item> {
        self.layout.items.iter().rev().find(|item| self.on_screen(item.area).contains(x, y))
    }
}

/// The widget that has the focus: the one the session remembers if it is
/// still on screen, or else the first field or list, or else the first
/// button.
fn focus_of(view: &View, layout: &GraphicLayout) -> Option<String> {
    let remembered = view.focus.as_ref().filter(|name| layout.focus_order.contains(name));
    let first_input = layout.focus_order.iter().find(|name| {
        layout
            .item(name)
            .is_some_and(|item| !matches!(item.part, Part::Button { .. } | Part::Picture))
    });
    remembered.or(first_input).or(layout.focus_order.first()).cloned()
}

fn widget<'a>(view: &'a View, name: &str) -> Option<&'a Widget> {
    view.widgets().find(|w| w.name == name)
}

fn max_length(view: &View, name: &str) -> usize {
    widget(view, name)
        .and_then(|w| w.props.get("max_length"))
        .and_then(|v| v.as_u64())
        .map_or(DEFAULT_MAX_LENGTH, |n| n as usize)
}

/// The column of a field in which text is typed, as opposed to one whose
/// value is chosen.
fn typed_column_of(layout: &GraphicLayout, name: &str) -> Option<String> {
    match &layout.item(name)?.part {
        Part::Field { column, kind: FieldKind::Text, .. } => Some(column.clone()),
        _ => None,
    }
}

/// A question is asked in a small window of its own, over the screen.
struct QuestionBox {
    window: Rect,
    lines: Vec<String>,
    /// Where the text starts.
    text: (i32, i32),
    cancel: Rect,
    accept: Rect,
}

const QUESTION_WIDTH: i32 = 250;
const QUESTION_LINE: i32 = 12;

impl QuestionBox {
    fn of(question: &Question, fonts: &Fonts) -> Self {
        let lines = wrap(fonts, &question.text, QUESTION_WIDTH - 28);
        let text_height = lines.len() as i32 * QUESTION_LINE;
        let height = 12 + text_height + 10 + BUTTON_HEIGHT + 10;
        let window = window_around(QUESTION_WIDTH, height);
        let content = content_of(window);

        let look = ButtonLook { primary: true, ..ButtonLook::default() };
        let accept_width = button_width(fonts, &question.label, look).max(70);
        let buttons_y = content.y + 12 + text_height + 10;
        let accept =
            Rect::new(content.right() - 14 - accept_width, buttons_y, accept_width, BUTTON_HEIGHT);
        let cancel = Rect::new(accept.x - 5 - 70, buttons_y, 70, BUTTON_HEIGHT);
        QuestionBox { window, lines, text: (content.x + 14, content.y + 12), cancel, accept }
    }

    fn draw(&self, question: &Question, fonts: &Fonts, frame: &mut Frame) {
        // What is behind steps back while the question is asked.
        frame.dither(chrome::DESKTOP, slot::INK, 8);
        draw_window(frame, fonts, self.window, "Confirm", WindowStyle::SYSTEM);
        for (index, line) in self.lines.iter().enumerate() {
            let baseline = self.text.1 + 9 + index as i32 * QUESTION_LINE;
            fonts.regular.draw(frame, self.text.0, baseline, line, slot::INK);
        }
        draw_button(frame, fonts, self.cancel, "Cancel", ButtonLook::default());
        let look = ButtonLook { primary: true, focused: true, ..ButtonLook::default() };
        draw_button(frame, fonts, self.accept, &question.label, look);
    }
}

/// The keys shown while a question is asked.
fn question_keys(question: &Question) -> [(&str, &str); 2] {
    [("Enter", question.label.as_str()), ("F12", "Cancel")]
}

fn part_of<'a>(layout: &'a GraphicLayout, focus: &Option<String>) -> Option<&'a Part> {
    focus.as_deref().and_then(|name| layout.item(name)).map(|item| &item.part)
}

/// Handles a key: moves the focus, edits a field, or asks the server to run
/// an action.
pub fn handle_key<A: Api>(session: &mut Session<A>, ui: &mut Ui, fonts: &Fonts, key: Key) {
    // A question waits for its answer and nothing else.
    if session.question().is_some() {
        match key {
            Key::Enter => session.answer(true),
            Key::Escape | Key::F(12) => session.answer(false),
            _ => {}
        }
        settle(session, ui, fonts);
        return;
    }

    // A script open in the script editor: the keys are those of the editor.
    if session.view().scripting.is_some() {
        script::handle_key(session, &mut ui.scripter, fonts, key);
        settle(session, ui, fonts);
        return;
    }

    // A picture open in the drawing editor: the keys are those of the editor.
    if session.view().drawing.is_some() {
        draw::handle_key(session, &mut ui.painter, key);
        settle(session, ui, fonts);
        return;
    }

    // A stack open in the visual editor: the keys are those of the editor.
    if session.view().designing {
        design::handle_key(session, &mut ui.designer, fonts, key);
        settle(session, ui, fonts);
        return;
    }

    let layout = Geometry::of(session.view(), fonts).layout;
    let focus = focus_of(session.view(), &layout);
    sync_cursor(session, ui, &layout, &focus);

    let on_grid = matches!(part_of(&layout, &focus), Some(Part::Grid));
    let on_picture = matches!(part_of(&layout, &focus), Some(Part::Picture));
    let icon_list = focus
        .as_deref()
        .filter(|_| on_grid)
        .and_then(|name| Some((widget(session.view(), name)?, layout.item(name)?)))
        .filter(|(definition, _)| is_icon_view(definition));
    let on_icons = icon_list.is_some();
    let icons_across = icon_list.map_or(1, |(_, item)| tiles_across(item.area) as isize);
    let on_button = matches!(part_of(&layout, &focus), Some(Part::Button { .. }));
    let on_choice = matches!(
        part_of(&layout, &focus),
        Some(Part::Field { kind: FieldKind::Choice | FieldKind::Check, .. })
    );
    let name = focus.clone().unwrap_or_default();
    match key {
        Key::Interrupt => {}
        // Escape leaves the screen, like the close box of its window.
        Key::Escape => {
            session.press("F12");
        }
        Key::F(number) => {
            if !session.press(&format!("F{number}")) {
                session.say("This key is not active on this screen.");
            }
        }
        // A conversation is read a page at a time.
        Key::PageUp | Key::PageDown => {
            let view = session.view();
            let talk = layout.items.iter().find(|item| item.part == Part::Talk).and_then(|item| {
                let definition = widget(view, &item.widget)?;
                let state = view.grids.get(&item.widget)?;
                let blocks = talk::blocks(fonts, definition, state, item.area.w);
                Some((talk::overflow(&blocks, item.area), talk::page(item.area)))
            });
            if let Some((most, page)) = talk {
                let step = if key == Key::PageUp { page } else { -page };
                ui.talk_back = (ui.talk_back + step).clamp(0, most);
            }
        }
        // A list shown as icons is walked in two directions.
        Key::Left if on_icons => session.select_row(&name, -1),
        Key::Right if on_icons => session.select_row(&name, 1),
        Key::Up if on_icons => session.select_row(&name, -icons_across),
        Key::Down if on_icons => session.select_row(&name, icons_across),
        Key::Up if on_grid => session.select_row(&name, -1),
        Key::Down if on_grid => session.select_row(&name, 1),
        // The picture of a card: Enter or the space bar draws it.
        Key::Enter | Key::Char(' ') if on_picture => {
            session.run_with("draw", serde_json::json!({ "target": "card", "widget": name }));
        }
        Key::Tab | Key::Down => move_focus(session, &layout, &focus, 1),
        Key::BackTab | Key::Up => move_focus(session, &layout, &focus, -1),
        Key::Enter if on_grid || on_button => session.activate(&name),
        Key::Char(' ') if on_button => session.activate(&name),
        // A field whose value is chosen goes through its values.
        Key::Char(' ') | Key::Right if on_choice => {
            session.cycle(&name, true);
        }
        Key::Left if on_choice => {
            session.cycle(&name, false);
        }
        // In a field, Enter is a key like any other; when the screen gives
        // it no meaning, it moves on to the next field.
        Key::Enter => {
            // What is sent is answered at the end of the conversation.
            ui.talk_back = 0;
            if !session.press("Enter") {
                move_focus(session, &layout, &focus, 1);
            }
        }
        _ => {
            if let Some(column) = typed_column_of(&layout, &name) {
                let mut text = session.view().value(&column).to_owned();
                let limit = max_length(session.view(), &name);
                if edit_line(&mut text, &mut ui.cursor, key, limit) {
                    session.edit(&column, text);
                }
            }
        }
    }
    settle(session, ui, fonts);
}

/// After anything that may have changed the card, the stack or the focus:
/// makes the focus and the cursor agree with what is on screen.
pub fn settle<A: Api>(session: &mut Session<A>, ui: &mut Ui, fonts: &Fonts) {
    // The script editor follows what the session says is being written, and
    // the drawing editor what it says is being drawn.
    script::settle(session, &mut ui.scripter);
    draw::settle(session, &mut ui.painter);
    if session.view().scripting.is_some() || session.view().drawing.is_some() {
        return;
    }
    // In the visual editor nothing is typed on the card: there is no field
    // to keep the cursor in, but a selection to keep up to date.
    if session.view().designing {
        design::settle(session, &mut ui.designer, fonts);
        return;
    }
    let layout = Geometry::of(session.view(), fonts).layout;
    let focus = focus_of(session.view(), &layout);
    sync_cursor(session, ui, &layout, &focus);
}

fn move_focus<A: Api>(
    session: &mut Session<A>,
    layout: &GraphicLayout,
    focus: &Option<String>,
    step: isize,
) {
    let order = &layout.focus_order;
    if order.is_empty() {
        return;
    }
    let current = focus.as_ref().and_then(|name| order.iter().position(|n| n == name)).unwrap_or(0);
    let next = (current as isize + step).rem_euclid(order.len() as isize) as usize;
    session.set_focus(Some(order[next].clone()));
}

/// Puts the cursor at the end of the value whenever the focus lands on
/// another field or another card.
fn sync_cursor<A: Api>(
    session: &mut Session<A>,
    ui: &mut Ui,
    layout: &GraphicLayout,
    focus: &Option<String>,
) {
    if session.view().focus != *focus {
        session.set_focus(focus.clone());
    }
    let view = session.view();
    let anchor = focus.as_ref().map(|name| {
        (view.stack().to_owned(), view.card.key.clone().filter(|_| !view.is_new), name.clone())
    });
    if ui.anchor != anchor {
        ui.cursor = focus
            .as_ref()
            .and_then(|name| typed_column_of(layout, name))
            .map_or(0, |column| view.value(&column).chars().count());
        ui.anchor = anchor;
    }
}

/// The pointer was pressed at a point of the screen.
pub fn pointer_down<A: Api>(session: &mut Session<A>, ui: &mut Ui, fonts: &Fonts, x: i32, y: i32) {
    settle(session, ui, fonts);

    // While a question is asked, only its two buttons and its two keys act.
    if let Some(question) = session.question().cloned() {
        let asked = QuestionBox::of(&question, fonts);
        let key = chrome::key_slots(fonts, question_keys(&question).into_iter())
            .into_iter()
            .find(|(_, _, area)| area.contains(x, y))
            .map(|(name, _, _)| name.to_owned());
        if asked.accept.contains(x, y) || key.as_deref() == Some("Enter") {
            session.answer(true);
        } else if asked.cancel.contains(x, y)
            || close_box_of(asked.window).contains(x, y)
            || key.is_some()
        {
            session.answer(false);
        }
        settle(session, ui, fonts);
        return;
    }

    if session.view().scripting.is_some() {
        script::pointer_down(session, &mut ui.scripter, fonts, x, y);
        settle(session, ui, fonts);
        return;
    }
    if session.view().drawing.is_some() {
        draw::pointer_down(session, &mut ui.painter, fonts, x, y);
        settle(session, ui, fonts);
        return;
    }
    if session.view().designing {
        design::pointer_down(session, &mut ui.designer, fonts, x, y);
        settle(session, ui, fonts);
        return;
    }

    let geometry = Geometry::of(session.view(), fonts);

    // The function key bar: a click is the key.
    let bound: Vec<(String, String)> =
        session.view().screen.keys.iter().map(|k| (k.key.clone(), k.label.clone())).collect();
    let clicked_key = chrome::key_slots(fonts, bound.iter().map(|(k, l)| (k.as_str(), l.as_str())))
        .into_iter()
        .find(|(_, _, area)| area.contains(x, y))
        .map(|(name, _, _)| name.to_owned());
    if let Some(name) = clicked_key {
        session.press(&name);
        settle(session, ui, fonts);
        return;
    }

    // The close box goes back, like F12.
    if close_box_of(geometry.window).contains(x, y) {
        session.press("F12");
        settle(session, ui, fonts);
        return;
    }

    let Some(item) = geometry.item_at(x, y).cloned() else { return };
    let area = geometry.on_screen(item.area);
    match &item.part {
        Part::Button { .. } => {
            session.set_focus(Some(item.widget.clone()));
            ui.pressed = Some(item.widget.clone());
        }
        Part::Field { kind: FieldKind::Choice | FieldKind::Check, .. } => {
            // A click moves on to the next value.
            session.set_focus(Some(item.widget.clone()));
            session.cycle(&item.widget, true);
        }
        Part::Field { column, .. } => {
            // The text is scrolled only in the field that has the focus.
            let text = session.view().value(column).to_owned();
            let had_focus = session.view().focus.as_deref() == Some(item.widget.as_str());
            let first =
                if had_focus { field_scroll(fonts, area, &text, Some(ui.cursor)) } else { 0 };
            session.set_focus(Some(item.widget.clone()));
            settle(session, ui, fonts);
            ui.cursor = field_cursor_at(fonts, area, &text, first, x);
        }
        // The picture of a card: a click draws it.
        Part::Picture => {
            let of_a_card = widget(session.view(), &item.widget)
                .is_some_and(|w| w.column.is_some())
                && session.view().screen.source.has_cards;
            if of_a_card {
                session.set_focus(Some(item.widget.clone()));
                session.run_with(
                    "draw",
                    serde_json::json!({ "target": "card", "widget": item.widget }),
                );
            }
        }
        Part::Grid => {
            session.set_focus(Some(item.widget.clone()));
            let Some(state) = session.view().grids.get(&item.widget) else { return };
            let icons = widget(session.view(), &item.widget).is_some_and(is_icon_view);
            let row = if icons { tile_at(area, state, x, y) } else { grid_row_at(area, state, y) };
            let Some(row) = row else { return };
            // A click chooses a row; a click on the chosen row opens it.
            if row == state.selected {
                session.activate(&item.widget);
            } else {
                session.select(&item.widget, row);
            }
        }
        _ => {}
    }
    settle(session, ui, fonts);
}

/// The pointer was released. A button acts when the pointer is released on
/// it, so that one can change one's mind by moving away.
pub fn pointer_up<A: Api>(session: &mut Session<A>, ui: &mut Ui, fonts: &Fonts, x: i32, y: i32) {
    if session.view().scripting.is_some() {
        script::pointer_up(&mut ui.scripter);
        return;
    }
    if session.view().drawing.is_some() && session.question().is_none() {
        draw::pointer_up(session, &mut ui.painter, x, y);
        settle(session, ui, fonts);
        return;
    }
    if session.view().designing && session.question().is_none() {
        design::pointer_up(session, &mut ui.designer, fonts, x, y);
        settle(session, ui, fonts);
        return;
    }
    let Some(pressed) = ui.pressed.take() else { return };
    let geometry = Geometry::of(session.view(), fonts);
    let released_on_it = geometry.item_at(x, y).is_some_and(|item| item.widget == pressed);
    if released_on_it {
        session.activate(&pressed);
        settle(session, ui, fonts);
    }
}

/// The pointer moved. Only the visual editor follows it: it drags widgets.
/// Returns true when the screen must be drawn again.
pub fn pointer_move<A: Api>(
    session: &mut Session<A>,
    ui: &mut Ui,
    fonts: &Fonts,
    x: i32,
    y: i32,
) -> bool {
    if session.question().is_some() {
        return false;
    }
    if session.view().scripting.is_some() {
        return script::pointer_move(session, &mut ui.scripter, fonts, x, y);
    }
    if session.view().drawing.is_some() {
        return draw::pointer_move(session, &mut ui.painter, x, y);
    }
    session.view().designing && design::pointer_move(session, &mut ui.designer, fonts, x, y)
}

/// What the clipboard should hold: the text selected in the script editor,
/// which loses it when `cut`. Nothing is selected anywhere else.
pub fn copy<A: Api>(
    session: &mut Session<A>,
    ui: &mut Ui,
    fonts: &Fonts,
    cut: bool,
) -> Option<String> {
    if session.question().is_some() || session.view().scripting.is_none() {
        return None;
    }
    script::copy(session, &mut ui.scripter, fonts, cut)
}

/// Writes the text of the clipboard where the cursor is: in the script
/// editor, or else in the field that has the focus.
pub fn paste<A: Api>(session: &mut Session<A>, ui: &mut Ui, fonts: &Fonts, text: &str) {
    if session.question().is_some() {
        return;
    }
    if session.view().scripting.is_some() {
        script::paste(session, &mut ui.scripter, fonts, text);
        return;
    }
    // Elsewhere a pasted text is typed, on one line.
    for character in text.chars().filter(|c| !c.is_control()) {
        handle_key(session, ui, fonts, Key::Char(character));
    }
}

/// Time passes. Returns true when the screen must be drawn again.
pub fn tick<A: Api>(session: &mut Session<A>, ui: &mut Ui, fonts: &Fonts) -> bool {
    session.view().drawing.is_none()
        && session.view().scripting.is_none()
        && session.view().designing
        && design::tick(session, &mut ui.designer, fonts)
}

/// What to say about the position in the stack.
fn position_text(view: &View) -> String {
    if view.is_new {
        "New card".to_owned()
    } else if view.card.count == 0 {
        "No card".to_owned()
    } else {
        format!("Card {} of {}", view.card.position, view.card.count)
    }
}

/// The text a text widget shows.
fn text_of<'a>(view: &'a View, name: &str) -> &'a str {
    view.text(name)
}

/// Draws the session. `blink` says whether the cursor is in the visible half
/// of its blinking.
pub fn draw<A: Api>(
    session: &Session<A>,
    ui: &Ui,
    fonts: &Fonts,
    frame: &mut Frame,
    clock: &str,
    blink: bool,
) {
    let view = session.view();
    if view.scripting.is_some() && ui.scripter.is_open() {
        script::draw(session, &ui.scripter, fonts, frame, clock, blink);
        draw_over(session, fonts, frame, None, script::KEYS.into_iter());
        return;
    }
    if view.drawing.is_some() && ui.painter.canvas.is_some() {
        draw::draw(session, &ui.painter, fonts, frame, clock, blink);
        let hint = draw::hint(&ui.painter);
        draw_over(session, fonts, frame, Some(&hint), draw::KEYS.into_iter());
        return;
    }
    if view.designing {
        design::draw(session, &ui.designer, fonts, frame, clock, blink);
        draw_over(session, fonts, frame, design::hint(&ui.designer), design::KEYS.into_iter());
        return;
    }
    draw_stack(session, ui, fonts, frame, clock, blink);
    let keys = view.screen.keys.iter().map(|k| (k.key.as_str(), k.label.as_str()));
    draw_over(session, fonts, frame, None, keys);
}

/// What is drawn over any screen: the question being asked, or else the
/// message for the user, and the function keys.
fn draw_over<'a, A: Api>(
    session: &Session<A>,
    fonts: &Fonts,
    frame: &mut Frame,
    hint: Option<&str>,
    keys: impl Iterator<Item = (&'a str, &'a str)>,
) {
    if let Some(question) = session.question() {
        QuestionBox::of(question, fonts).draw(question, fonts, frame);
        chrome::draw_key_bar(frame, fonts, question_keys(question).into_iter());
        return;
    }
    if let Some(message) = session.message().or(hint) {
        chrome::draw_message(frame, fonts, message);
    }
    chrome::draw_key_bar(frame, fonts, keys);
}

/// Draws a stack being browsed, or a dialog: its window and what is in it.
fn draw_stack<A: Api>(
    session: &Session<A>,
    ui: &Ui,
    fonts: &Fonts,
    frame: &mut Frame,
    clock: &str,
    blink: bool,
) {
    let view = session.view();
    let geometry = Geometry::of(view, fonts);
    let layout = &geometry.layout;
    let focus = focus_of(view, layout);
    let system = view.screen.stack.kind == "system";

    chrome::draw_desktop(frame);
    chrome::draw_menu_bar(frame, fonts, Some(&session.hello().instance), clock);
    let style = if system { WindowStyle::SYSTEM } else { WindowStyle::STACK };
    draw_window(frame, fonts, geometry.window, &view.screen.stack.title, style);

    let previous = frame.clip_to(geometry.content);
    if let Some(paper) = layout.paper {
        draw_paper(frame, geometry.on_screen(paper));
    }
    for rule in &layout.rules {
        let rule = geometry.on_screen(*rule);
        if layout.paper.is_some() {
            frame.fill(rule, slot::ICE);
        } else {
            for x in (rule.x..rule.right()).step_by(2) {
                frame.set(x, rule.y, slot::OCHRE);
            }
        }
    }
    if let Some(header) = layout.header {
        let header = geometry.on_screen(header);
        frame.hline(header.x, header.bottom() - 1, header.w, slot::RED);
        let position = position_text(view);
        let x = header.right() - 9 - fonts.regular.width(&position);
        fonts.regular.draw(frame, x, header.bottom() - 9, &position, slot::GREY);
    }

    for item in &layout.items {
        let area = geometry.on_screen(item.area);
        let focused = focus.as_deref() == Some(item.widget.as_str());
        match &item.part {
            Part::Title => {
                let room = area.w - 30 - fonts.regular.width(&position_text(view));
                let clip = frame.clip_to(Rect::new(area.x, area.y, room, area.h));
                let text = text_of(view, &item.widget);
                fonts.bold.draw_scaled(frame, area.x + 13, area.y + 21, text, slot::INK, 2);
                frame.set_clip(clip);
            }
            Part::Banner { subtitle } => {
                frame.gradient(
                    Rect::new(area.x, area.y, area.w, area.h - 1),
                    slot::NAVY,
                    slot::BLUE,
                );
                frame.hline(area.x, area.bottom() - 1, area.w, slot::INK);
                let mut x = area.x + 14;
                if system {
                    frame.sprite(&chrome::large_logo(), area.x + 13, area.y + 8);
                    x = area.x + 48;
                }
                let title = text_of(view, &item.widget);
                fonts.bold.draw_scaled(frame, x, area.y + 25, title, slot::WHITE, 2);
                if let Some(subtitle) = subtitle {
                    let text = text_of(view, subtitle);
                    fonts.regular.draw(frame, x, area.y + 39, text, slot::ICE);
                }
            }
            // A text in a rectangle taller than a line is written on as
            // many lines as it needs, within the rectangle.
            Part::Text if area.h >= 2 * TEXT_LINE => {
                let clip = frame.clip_to(area);
                let text = text_of(view, &item.widget);
                for (index, line) in wrapped(fonts, text, area.w).iter().enumerate() {
                    let baseline = area.y + 8 + index as i32 * TEXT_LINE;
                    fonts.regular.draw(frame, area.x, baseline, line, slot::INK);
                }
                frame.set_clip(clip);
            }
            Part::Text => {
                let clip = frame.clip_to(area);
                let text = text_of(view, &item.widget);
                fonts.regular.draw(frame, area.x, text_baseline(area), text, slot::INK);
                frame.set_clip(clip);
            }
            // The same for a text with a label: the label is then a heading
            // over it.
            Part::Info { label, .. } if area.h >= 2 * TEXT_LINE => {
                let clip = frame.clip_to(area);
                fonts.bold.draw(frame, area.x, area.y + 8, label, slot::COCOA);
                let text = text_of(view, &item.widget);
                for (index, line) in wrapped(fonts, text, area.w - 6).iter().enumerate() {
                    let baseline = area.y + 8 + (index as i32 + 1) * TEXT_LINE;
                    fonts.regular.draw(frame, area.x + 6, baseline, line, slot::INK);
                }
                frame.set_clip(clip);
            }
            Part::Info { label, value_x } => {
                let clip = frame.clip_to(area);
                fonts.regular.draw(frame, area.x, text_baseline(area), label, slot::GREY);
                let text = text_of(view, &item.widget);
                fonts.regular.draw(frame, area.x + value_x, text_baseline(area), text, slot::INK);
                frame.set_clip(clip);
            }
            Part::Field { column, label, kind } => {
                let label_x = area.x - 5 - fonts.regular.width(label);
                fonts.regular.draw(frame, label_x, area.y + 11, label, slot::COCOA);
                let value = view.value(column);
                match kind {
                    FieldKind::Text => {
                        let cursor = focused.then_some(ui.cursor);
                        draw_field(frame, fonts, area, value, cursor, blink);
                    }
                    FieldKind::Choice => {
                        let shown = widget(view, &item.widget)
                            .map_or_else(|| value.to_owned(), |w| w.shown(value));
                        draw_choice(frame, fonts, area, &shown, focused);
                    }
                    FieldKind::Check => draw_check(frame, area, value == "true", focused),
                }
            }
            Part::Picture => {
                if focused {
                    frame.outline(area.inset(-1), slot::BLUE);
                }
                frame.outline(area, slot::INK);
                let inside = area.inset(1);
                let clip = frame.clip_to(inside);
                let image = widget(view, &item.widget)
                    .and_then(|w| view.image_id(w))
                    .and_then(|id| session.image(id));
                match image {
                    Some(image) => {
                        // What is transparent in a picture shows the paper.
                        frame.fill(inside, slot::CREAM);
                        frame.blit(image, inside.x, inside.y);
                    }
                    None => {
                        // An empty frame, waiting for a picture.
                        frame.fill(inside, slot::ICE);
                        frame.dither(inside, slot::AZURE, 4);
                    }
                }
                frame.set_clip(clip);
            }
            Part::Grid => {
                if let (Some(definition), Some(state)) =
                    (widget(view, &item.widget), view.grids.get(&item.widget))
                {
                    if is_icon_view(definition) {
                        draw_icons(frame, fonts, session, area, definition, state, focused);
                    } else {
                        draw_grid(frame, fonts, area, definition, state, focused);
                    }
                }
            }
            Part::Talk => {
                if let (Some(definition), Some(state)) =
                    (widget(view, &item.widget), view.grids.get(&item.widget))
                {
                    talk::draw(frame, fonts, area, definition, state, ui.talk_back);
                }
            }
            Part::Button { label, look } => {
                let look = ButtonLook {
                    pressed: ui.pressed.as_deref() == Some(item.widget.as_str()),
                    focused,
                    ..*look
                };
                draw_button(frame, fonts, area, label, look);
            }
        }
    }
    frame.set_clip(previous);
}

/// A text cut into lines no wider than `width` pixels. A word wider than a
/// line, such as the name of a model, is cut too.
fn wrapped(fonts: &Fonts, text: &str, width: i32) -> Vec<String> {
    xila_core::talk::wrap(text, width.max(1) as usize, |piece| {
        fonts.regular.width(piece).max(0) as usize
    })
}

/// The baseline of a line of text in its rectangle: aligned with the text of
/// a field when the rectangle is as tall as one.
fn text_baseline(area: Rect) -> i32 {
    area.y + if area.h >= chrome::FIELD_HEIGHT { 11 } else { 8 }
}

struct Column<'a> {
    name: &'a str,
    label: &'a str,
    right: bool,
    x: i32,
    width: i32,
}

fn grid_columns<'a>(definition: &'a Widget, area: Rect) -> Vec<Column<'a>> {
    let mut columns: Vec<Column> = definition
        .props
        .get("columns")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let name = c.get("column")?.as_str()?;
            Some(Column {
                name,
                label: c.get("label").and_then(|l| l.as_str()).unwrap_or(name),
                right: c.get("align").and_then(|a| a.as_str()) == Some("right"),
                x: 0,
                width: 0,
            })
        })
        .collect();

    // Right-aligned columns are narrow; the others share what is left.
    let narrow = columns.iter().filter(|c| c.right).count() as i32;
    let wide = (columns.len() as i32 - narrow).max(1);
    let wide_width = (area.w - 2 - narrow * NARROW_COLUMN) / wide;
    let mut x = area.x + 1;
    for column in &mut columns {
        column.x = x;
        column.width = if column.right { NARROW_COLUMN } else { wide_width };
        x += column.width;
    }
    columns
}

/// How many rows of a list fit in its rectangle.
fn grid_capacity(area: Rect) -> usize {
    ((area.h - GRID_HEADER - 2) / GRID_ROW).max(1) as usize
}

/// The first row shown, so that the selected one stays in sight.
fn grid_first(area: Rect, state: &GridState) -> usize {
    (state.selected + 1).saturating_sub(grid_capacity(area))
}

/// The row of a list under a point, if there is one.
fn grid_row_at(area: Rect, state: &GridState, y: i32) -> Option<usize> {
    let top = area.y + 1 + GRID_HEADER;
    if y < top {
        return None;
    }
    let line = ((y - top) / GRID_ROW) as usize;
    let row = grid_first(area, state) + line;
    (line < grid_capacity(area) && row < state.page.rows.len()).then_some(row)
}

/// How many tiles of a list shown as icons fit side by side, and one under
/// the other.
fn tiles_across(area: Rect) -> usize {
    (area.w / TILE.0).max(1) as usize
}

fn tiles_down(area: Rect) -> usize {
    (area.h / TILE.1).max(1) as usize
}

/// The first row of tiles shown, so that the selected one stays in sight.
fn first_tile_row(area: Rect, state: &GridState) -> usize {
    (state.selected / tiles_across(area) + 1).saturating_sub(tiles_down(area))
}

/// The rectangle of a tile on screen, by its place among those in sight.
pub fn tile_area(area: Rect, place: usize) -> Rect {
    let across = tiles_across(area);
    // The tiles share the width, so that they are centred as a whole.
    let pitch = area.w / across as i32;
    Rect::new(
        area.x + (place % across) as i32 * pitch + (pitch - TILE.0) / 2,
        area.y + (place / across) as i32 * TILE.1,
        TILE.0,
        TILE.1,
    )
}

/// The row of a list shown as icons under a point, if there is one.
fn tile_at(area: Rect, state: &GridState, x: i32, y: i32) -> Option<usize> {
    let first = first_tile_row(area, state) * tiles_across(area);
    let in_sight = tiles_across(area) * tiles_down(area);
    (0..in_sight)
        .find(|place| tile_area(area, *place).contains(x, y))
        .map(|place| first + place)
        .filter(|row| *row < state.page.rows.len())
}

/// The icon of a row that has none: a pile of cards.
fn generic_icon() -> crate::frame::Sprite {
    crate::frame::Sprite::from_art(
        &[
            "......####################..",
            "......#sssssssssssssssssss#.",
            "...####################sss#.",
            "...#ssssssssssssssssss#sss#.",
            "####################ss#sss#.",
            "#aaaaaaaaaaaaaaaaaa#ss#sss#.",
            "#aaaaaaaaaaaaaaaaaa#ss#sss#.",
            "#aaaaaaaaaaaaaaaaaa#ss#sss#.",
            "####################ss#sss#.",
            "#oooooooooooooooooo#ss#sss#.",
            "#oiiiiioooooooooooo#ss#sss#.",
            "#oiiiiiooggggggggoo#ss#sss#.",
            "#oiiiiioooooooooooo#ss#sss#.",
            "#oiiiiiooggggggggoo#ss#sss#.",
            "#oiiiiioooooooooooo#ss#sss#.",
            "#oiiiiiooggggggggoo#ss#sss#.",
            "#oiiiiioooooooooooo#ss#sss#.",
            "#oooooooooooooooooo#ss#sss#.",
            "#ogggggoooooooooooo#ss#sss#.",
            "#oooooooooooooooooo#ss#####.",
            "#oooooooooooooooooo#ss#.....",
            "#oooooooooooooooooo####.....",
            "#oooooooooooooooooo#........",
            "####################........",
        ],
        &[
            ('#', slot::INK),
            ('s', slot::OCHRE),
            ('a', slot::AMBER),
            ('o', slot::CREAM),
            ('i', slot::ICE),
            ('g', slot::SILVER),
        ],
    )
}

/// A list shown as icons, as on a desktop: each row is an icon at twice its
/// size, with the text of its first column under it and that of the second,
/// with its heading, under that ("12 cards").
fn draw_icons<A: Api>(
    frame: &mut Frame,
    fonts: &Fonts,
    session: &Session<A>,
    area: Rect,
    definition: &Widget,
    state: &GridState,
    focused: bool,
) {
    const ICON: (i32, i32) = (56, 52);
    let columns = grid_columns(definition, area);
    let icon_column = definition.props.get("icon").and_then(|v| v.as_str()).unwrap_or_default();
    if state.page.rows.is_empty() {
        let empty = definition.props.get("empty").and_then(|v| v.as_str()).unwrap_or("No rows.");
        fonts.regular.draw(frame, area.x + 6, area.y + 14, empty, slot::GREY);
        return;
    }

    let first = first_tile_row(area, state) * tiles_across(area);
    let in_sight = tiles_across(area) * tiles_down(area);
    for (place, (index, row)) in
        state.page.rows.iter().enumerate().skip(first).take(in_sight).enumerate()
    {
        let tile = tile_area(area, place);
        let cell = |column: Option<&Column>| {
            column.and_then(|c| row.cells.get(c.name)).and_then(|v| v.as_deref()).unwrap_or("")
        };

        // The icon, enlarged, in the middle of its box; a generic one if the
        // row has none.
        let image =
            row.cells.get(icon_column).and_then(|v| v.as_deref()).and_then(|id| session.image(id));
        match image {
            Some(image) => {
                let scale = if i32::from(image.width) * 2 <= ICON.0
                    && i32::from(image.height) * 2 <= ICON.1
                {
                    2
                } else {
                    1
                };
                let (w, h) = (i32::from(image.width) * scale, i32::from(image.height) * scale);
                let clip = frame.clip_to(Rect::new(
                    tile.x + (TILE.0 - ICON.0) / 2,
                    tile.y + 4,
                    ICON.0,
                    ICON.1,
                ));
                frame.blit_scaled(
                    image,
                    tile.x + (TILE.0 - w) / 2,
                    tile.y + 4 + (ICON.1 - h) / 2,
                    scale,
                );
                frame.set_clip(clip);
            }
            None => {
                let sprite = generic_icon();
                let x = tile.x + (TILE.0 - sprite.width * 2) / 2;
                let y = tile.y + 4 + (ICON.1 - sprite.height * 2) / 2;
                for (at, pixel) in sprite.pixels.iter().enumerate() {
                    if let Some(colour) = pixel {
                        let (px, py) = (at as i32 % sprite.width, at as i32 / sprite.width);
                        frame.fill(Rect::new(x + px * 2, y + py * 2, 2, 2), *colour);
                    }
                }
            }
        }

        // The title, on a bar when the row is selected, and the line under it.
        let title = cell(columns.first());
        let width = fonts.bold.width(title).min(TILE.0 - 6);
        let title_x = tile.x + (TILE.0 - width) / 2;
        let title_y = tile.y + 4 + ICON.1 + 4;
        let colour = if index == state.selected {
            let bar = Rect::new(title_x - 3, title_y - 1, width + 6, 12);
            frame.fill(bar, if focused { slot::BLUE } else { slot::SILVER });
            slot::WHITE
        } else {
            slot::INK
        };
        let clip = frame.clip_to(Rect::new(title_x, title_y, width, 11));
        fonts.bold.draw(frame, title_x, title_y + 9, title, colour);
        frame.set_clip(clip);

        if let Some(second) = columns.get(1) {
            let line = format!("{} {}", cell(Some(second)), second.label.to_lowercase());
            let x = tile.x + (TILE.0 - fonts.regular.width(&line)) / 2;
            fonts.regular.draw(frame, x, title_y + 21, &line, slot::GREY);
        }
    }
}

/// A list of rows under a header, the selected row highlighted.
fn draw_grid(
    frame: &mut Frame,
    fonts: &Fonts,
    area: Rect,
    definition: &Widget,
    state: &GridState,
    focused: bool,
) {
    frame.fill(area, slot::WHITE);
    frame.outline(area, slot::INK);
    let header = Rect::new(area.x + 1, area.y + 1, area.w - 2, GRID_HEADER - 1);
    frame.fill(header, slot::SAND);
    frame.hline(area.x, header.bottom(), area.w, slot::INK);

    let columns = grid_columns(definition, area);
    let cell = |frame: &mut Frame, column: &Column, y: i32, text: &str, colour: u8| {
        let room = Rect::new(column.x + 5, y, column.width - 10, GRID_ROW);
        let clip = frame.clip_to(room);
        let x = if column.right { room.right() - fonts.regular.width(text) } else { room.x };
        fonts.regular.draw(frame, x, y + 9, text, colour);
        frame.set_clip(clip);
    };
    for column in &columns {
        cell(frame, column, header.y, column.label, slot::COCOA);
    }

    let top = header.bottom() + 1;
    if state.page.rows.is_empty() {
        let empty = definition.props.get("empty").and_then(|v| v.as_str()).unwrap_or("No rows.");
        fonts.regular.draw(frame, area.x + 6, top + 10, empty, slot::GREY);
        return;
    }

    let first = grid_first(area, state);
    let rows = state.page.rows.iter().enumerate().skip(first).take(grid_capacity(area));
    for (line, (index, row)) in rows.enumerate() {
        let y = top + line as i32 * GRID_ROW;
        let selected = index == state.selected;
        let colour = if selected {
            let bar = Rect::new(area.x + 1, y, area.w - 2, GRID_ROW);
            frame.fill(bar, if focused { slot::BLUE } else { slot::SILVER });
            slot::WHITE
        } else {
            slot::INK
        };
        for column in &columns {
            let text = row.cells.get(column.name).and_then(|v| v.as_deref()).unwrap_or("");
            cell(frame, column, y, text, colour);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{addressbook_session, count_slot, has_slot, snapshot, Fake};
    use xila_core::api::Context;

    fn fonts() -> Fonts {
        Fonts::builtin()
    }

    fn render(session: &Session<Fake>, ui: &Ui) -> Frame {
        let mut frame = Frame::new();
        draw(session, ui, &fonts(), &mut frame, "16:54", true);
        frame
    }

    fn press(session: &mut Session<Fake>, ui: &mut Ui, keys: &[Key]) {
        let fonts = fonts();
        for key in keys {
            handle_key(session, ui, &fonts, *key);
        }
    }

    fn click(session: &mut Session<Fake>, ui: &mut Ui, x: i32, y: i32) {
        let fonts = fonts();
        pointer_down(session, ui, &fonts, x, y);
        pointer_up(session, ui, &fonts, x, y);
    }

    /// A session on the screen of the agent, with a conversation.
    fn agent_session(talk: &[(&str, &str)]) -> (Session<Fake>, Ui) {
        let server = Fake {
            talk: talk.iter().map(|(who, said)| ((*who).to_owned(), (*said).to_owned())).collect(),
            ..Fake::default()
        };
        let mut session = Session::start(server).unwrap();
        let mut ui = Ui::default();
        session.press("F9");
        assert_eq!(session.view().stack(), "desktop", "the fake desktop has no F9 key");
        session.run("agent");
        assert_eq!(session.view().stack(), "agent");
        settle(&mut session, &mut ui, &fonts());
        (session, ui)
    }

    const TALK: [(&str, &str); 4] = [
        ("user", "Who lives in London?"),
        ("tool", "Looked for “London” in Address book: 2 cards."),
        ("agent", "Two people live in London: Charles Babbage, of Difference Works, and Ada Lovelace, of Analytical Engines."),
        ("error", "The model did not answer in time."),
    ];

    #[test]
    fn the_agent_has_a_window_with_the_conversation_and_what_it_is() {
        let (session, ui) = agent_session(&TALK);
        let frame = render(&session, &ui);
        snapshot(&frame, "agent");

        // The conversation: a bubble for each of the two, what the agent did
        // and what went wrong in between.
        let talk = area_of(&session, "talk");
        assert!(has_slot(&frame, talk, slot::ICE), "the bubble of the person");
        assert!(has_slot(&frame, talk, slot::WHITE), "the bubble of the agent");
        assert!(has_slot(&frame, talk, slot::LAVENDER), "the face of the agent");
        assert!(has_slot(&frame, talk, slot::RED), "what went wrong");
        // The person is on the right, the agent on the left.
        let right = Rect::new(talk.x + talk.w * 3 / 4, talk.y, talk.w / 4, talk.h);
        let left = Rect::new(talk.x, talk.y, talk.w / 8, talk.h);
        assert!(has_slot(&frame, right, slot::ICE));
        assert!(!has_slot(&frame, left, slot::ICE));
        assert!(has_slot(&frame, left, slot::LAVENDER));

        // Beside it: the model, where it is, the tools, each under its heading.
        for name in ["model", "host", "tools", "guard", "today", "about"] {
            assert!(has_slot(&frame, area_of(&session, name), slot::INK), "{name}");
        }
        // A text that does not fit on a line goes on with the next one.
        let tools = area_of(&session, "tools");
        let third_line = Rect::new(tools.x, tools.y + 24, tools.w, 10);
        assert!(has_slot(&frame, third_line, slot::INK));

        // The line to write on has the focus; the conversation takes none.
        assert_eq!(session.view().focus.as_deref(), Some("ask"));
    }

    #[test]
    fn a_question_is_typed_and_sent_from_the_window() {
        let (mut session, mut ui) = agent_session(&[]);
        let empty = render(&session, &ui);
        let talk = area_of(&session, "talk");
        assert!(!has_slot(&empty, talk, slot::ICE));

        press(&mut session, &mut ui, &[Key::Char('H'), Key::Char('i'), Key::Enter]);
        let (action, context) = session.api().actions.last().unwrap();
        assert_eq!(action, "send");
        assert_eq!(context.dirty.get("ask").map(String::as_str), Some("Hi"));
        assert_eq!(context.card.as_deref(), Some("talk-1"));

        // The question is in the conversation, the line is empty again, and
        // the agent is said to be working.
        assert_eq!(session.view().value("ask"), "");
        let frame = render(&session, &ui);
        assert!(has_slot(&frame, talk, slot::ICE));
        assert!(has_slot(&frame, talk, slot::AMBER));

        // The answer comes later: the server says so, and the window shows it.
        let api = session.api_mut();
        api.talk.pop();
        api.talk.push(("agent".into(), "Hello.".into()));
        api.pending.push(xila_core::api::Notification::Card {
            stack: Some("agent".into()),
            key: Some("talk-1".into()),
        });
        assert!(session.poll());
        let frame = render(&session, &ui);
        assert!(has_slot(&frame, talk, slot::WHITE));
        assert!(!has_slot(&frame, talk, slot::AMBER));

        // The button does the same as the key.
        press(&mut session, &mut ui, &[Key::Char('?')]);
        let send = area_of(&session, "send");
        click(&mut session, &mut ui, send.x + 5, send.y + 5);
        assert_eq!(session.api().count("send"), 2);
    }

    #[test]
    fn a_long_conversation_is_read_a_page_at_a_time() {
        let turns: Vec<(&str, &str)> =
            (0..30).flat_map(|_| [("user", "And then?"), ("agent", "Then this.")]).collect();
        let (mut session, mut ui) = agent_session(&turns);
        let talk = area_of(&session, "talk");
        let at_the_end = render(&session, &ui);
        // A bar on the right says there is more.
        let bar = Rect::new(talk.right() - 5, talk.y + 2, 3, talk.h - 4);
        assert!(has_slot(&at_the_end, bar, slot::SAND));

        press(&mut session, &mut ui, &[Key::PageUp]);
        assert!(ui.talk_back > 0);
        let back = render(&session, &ui);
        assert_ne!(back.pixels(), at_the_end.pixels());

        // Going back stops at the first line, and coming forward at the last.
        press(&mut session, &mut ui, &[Key::PageUp; 40]);
        let most = ui.talk_back;
        press(&mut session, &mut ui, &[Key::PageUp]);
        assert_eq!(ui.talk_back, most);
        press(&mut session, &mut ui, &[Key::PageDown; 40]);
        assert_eq!(ui.talk_back, 0);
        assert_eq!(render(&session, &ui).pixels(), at_the_end.pixels());
    }

    /// The rectangle of a widget on the screen.
    fn area_of(session: &Session<Fake>, name: &str) -> Rect {
        let geometry = Geometry::of(session.view(), &fonts());
        geometry.on_screen(geometry.layout.item(name).unwrap().area)
    }

    fn centre(area: Rect) -> (i32, i32) {
        (area.x + area.w / 2, area.y + area.h / 2)
    }

    fn actions(session: &Session<Fake>) -> Vec<(&str, &Context)> {
        session.api().actions.iter().map(|(name, context)| (name.as_str(), context)).collect()
    }

    #[test]
    fn the_desktop_is_a_window_with_a_banner_and_a_main_button() {
        let session = Session::start(Fake::default()).unwrap();
        let frame = render(&session, &Ui::default());
        let geometry = Geometry::of(session.view(), &fonts());

        // A system window: azure title bar, cream body.
        let window = geometry.window;
        assert_eq!(frame.pixel(window.x + 2, window.y + 2), Some(slot::AZURE));
        assert_eq!(frame.pixel(window.x + 2, window.bottom() - 3), Some(slot::CREAM));
        // The banner, with the logo and white letters.
        let banner = area_of(&session, "banner");
        assert!(has_slot(&frame, banner, slot::TURQUOISE));
        assert!(has_slot(&frame, banner, slot::WHITE));
        // The main button is yellow and has the focus.
        let button = area_of(&session, "stacks");
        assert!(count_slot(&frame, button, slot::YELLOW) > 500);
        assert_eq!(session.view().focus, None, "drawing changes nothing");
    }

    #[test]
    fn enter_on_the_main_button_opens_the_stack_list() {
        let mut session = Session::start(Fake::default()).unwrap();
        let mut ui = Ui::default();
        press(&mut session, &mut ui, &[Key::Enter]);
        assert_eq!(session.view().stack(), "stacks");
        assert_eq!(actions(&session)[0].0, "open_stacks");
    }

    #[test]
    fn a_click_acts_when_released_on_the_button() {
        let fonts = fonts();
        let mut session = Session::start(Fake::default()).unwrap();
        let mut ui = Ui::default();
        let (x, y) = centre(area_of(&session, "stacks"));

        // Pressed, then released somewhere else: nothing happens.
        pointer_down(&mut session, &mut ui, &fonts, x, y);
        assert_eq!(ui.pressed.as_deref(), Some("stacks"));
        pointer_up(&mut session, &mut ui, &fonts, 2, 100);
        assert_eq!(session.view().stack(), "desktop");
        assert_eq!(ui.pressed, None);

        click(&mut session, &mut ui, x, y);
        assert_eq!(session.view().stack(), "stacks");
    }

    #[test]
    fn a_list_of_icons_is_browsed_with_the_arrows_or_the_pointer() {
        let mut session = Session::start(Fake::default()).unwrap();
        let mut ui = Ui::default();
        press(&mut session, &mut ui, &[Key::Enter]);
        let frame = render(&session, &ui);
        let list = area_of(&session, "list");
        let (first, second) = (tile_area(list, 0), tile_area(list, 1));
        // The first stack is selected: its title is on a blue bar.
        let title_of = |tile: Rect| Rect::new(tile.x, tile.y + 58, tile.w, 14);
        assert!(has_slot(&frame, title_of(first), slot::BLUE));
        assert!(!has_slot(&frame, title_of(second), slot::BLUE));

        // A click on the second one chooses it; a click on it again opens it.
        let (x, y) = centre(second);
        click(&mut session, &mut ui, x, y);
        assert_eq!(session.view().grids["list"].selected, 1);
        assert_eq!(session.view().stack(), "stacks");
        click(&mut session, &mut ui, x, y);
        assert_eq!(session.view().stack(), "recipes");
        assert_eq!(actions(&session).last().unwrap().1.row.as_deref(), Some("recipes"));
    }

    #[test]
    fn a_click_beside_the_icons_of_a_list_does_nothing() {
        let mut session = Session::start(Fake::default()).unwrap();
        let mut ui = Ui::default();
        press(&mut session, &mut ui, &[Key::Enter, Key::Right]);
        let list = area_of(&session, "list");
        // Where a third and a fourth stack would be.
        for place in [2, 4] {
            let (x, y) = centre(tile_area(list, place));
            click(&mut session, &mut ui, x, y);
        }
        assert_eq!(session.view().stack(), "stacks");
        assert_eq!(session.view().grids["list"].selected, 1);
    }

    #[test]
    fn a_list_of_rows_is_browsed_with_the_arrows_or_the_pointer() {
        let (mut session, mut ui) = author_session();
        let frame = render(&session, &ui);
        let list = area_of(&session, "fields");
        // The first row is selected; the list has no focus, so its bar is grey.
        assert_eq!(frame.pixel(list.x + 2, list.y + 1 + GRID_HEADER + 2), Some(slot::SILVER));
        assert_eq!(frame.pixel(list.x + 2, list.y + 3), Some(slot::SAND));

        // A click on the second row chooses it; a click on it again opens it.
        let second = list.y + 1 + GRID_HEADER + GRID_ROW + 4;
        click(&mut session, &mut ui, list.x + 30, second);
        assert_eq!(session.view().grids["fields"].selected, 1);
        assert_eq!(session.view().stack(), "author");
        assert_eq!(render(&session, &ui).pixel(list.x + 2, second), Some(slot::BLUE));
        // A click under the last row, or on the header, does nothing.
        click(&mut session, &mut ui, list.x + 30, list.bottom() - 4);
        click(&mut session, &mut ui, list.x + 30, list.y + 4);
        assert_eq!(session.view().grids["fields"].selected, 1);
        assert_eq!(session.view().stack(), "author");

        click(&mut session, &mut ui, list.x + 30, second);
        assert_eq!(session.view().stack(), "field");
        assert_eq!(actions(&session).last().unwrap().1.row.as_deref(), Some("f2"));
    }

    /// A session in author mode on the address book.
    fn author_session() -> (Session<Fake>, Ui) {
        let (mut session, mut ui) = addressbook_session();
        session.run("author");
        settle(&mut session, &mut ui, &fonts());
        assert_eq!(session.view().stack(), "author");
        (session, ui)
    }

    #[test]
    fn author_mode_is_a_dialog_with_fields_a_list_and_buttons() {
        let (session, ui) = author_session();
        let frame = render(&session, &ui);
        snapshot(&frame, "author");
        let geometry = Geometry::of(session.view(), &fonts());

        // A system window, although it shows a card: no sheet of paper.
        let window = geometry.window;
        assert_eq!(frame.pixel(window.x + 2, window.y + 2), Some(slot::AZURE));
        assert_eq!(geometry.layout.paper, None);
        assert!(window.bottom() <= chrome::KEYS_TOP - 3, "the window fits on the desktop");

        // The two fields start at the same place, under one another.
        let (title, card_title) = (area_of(&session, "title"), area_of(&session, "card_title"));
        assert_eq!(title.x, card_title.x);
        assert!(card_title.y > title.y);
        // The list shows the fields of the stack, in four columns.
        let list = area_of(&session, "fields");
        assert!(list.w > 300);
        assert_eq!(session.view().grids["fields"].page.rows.len(), 3);
        assert!(has_slot(&frame, list, slot::INK));
        // Every key of the screen is in the bar.
        let bound = session.view().screen.keys.len();
        let shown = chrome::key_slots(
            &fonts(),
            session.view().screen.keys.iter().map(|k| (k.key.as_str(), k.label.as_str())),
        );
        assert_eq!(shown.len(), bound);
    }

    #[test]
    fn the_key_bar_makes_room_for_many_keys() {
        let keys = crate::testing::keys_of_a_built_stack();
        let slots = chrome::key_slots(&fonts(), keys.iter().copied());
        assert_eq!(slots.len(), keys.len());
        let last = slots.last().unwrap().2;
        assert!(last.right() <= crate::frame::WIDTH, "{last:?}");
        // They do not overlap.
        for pair in slots.windows(2) {
            assert!(pair[0].2.right() <= pair[1].2.x, "{:?} {:?}", pair[0].2, pair[1].2);
        }
        // A few keys keep their usual places.
        let few = chrome::key_slots(&fonts(), [("F3", "Quit"), ("F12", "Back")].into_iter());
        assert_eq!(few[0].2.x, 6);
    }

    #[test]
    fn enter_saves_from_a_field_and_opens_from_the_list() {
        let (mut session, mut ui) = author_session();
        // The first field has the focus: Enter is the key of the screen.
        press(&mut session, &mut ui, &[Key::Char('!'), Key::Enter]);
        let (action, context) = *actions(&session).last().unwrap();
        assert_eq!(action, "save");
        assert_eq!(context.card.as_deref(), Some("addressbook"));
        assert_eq!(context.dirty["title"], "Address book!");

        // On the list, Enter opens the selected field.
        let (mut session, mut ui) = author_session();
        press(&mut session, &mut ui, &[Key::Tab, Key::Tab, Key::Down, Key::Enter]);
        assert_eq!(session.view().stack(), "field");
        assert_eq!(actions(&session).last().unwrap().1.row.as_deref(), Some("f2"));
    }

    #[test]
    fn a_field_with_choices_goes_through_them() {
        let (mut session, mut ui) = author_session();
        press(&mut session, &mut ui, &[Key::Tab, Key::Tab, Key::Enter]);
        assert_eq!(session.view().stack(), "field");
        let frame = render(&session, &ui);
        snapshot(&frame, "field");

        // Typing does not change a choice; space and the arrows do.
        press(&mut session, &mut ui, &[Key::Tab, Key::Tab]);
        assert_eq!(session.view().focus.as_deref(), Some("type"));
        press(&mut session, &mut ui, &[Key::Char('x')]);
        assert_eq!(session.view().value("type"), "text");
        press(&mut session, &mut ui, &[Key::Char(' ')]);
        assert_eq!(session.view().value("type"), "number");
        press(&mut session, &mut ui, &[Key::Right]);
        assert_eq!(session.view().value("type"), "date");
        press(&mut session, &mut ui, &[Key::Left, Key::Left, Key::Left]);
        assert_eq!(session.view().value("type"), "boolean", "it goes round");

        // A click moves on to the next value.
        let (x, y) = centre(area_of(&session, "type"));
        click(&mut session, &mut ui, x, y);
        assert_eq!(session.view().value("type"), "text");
    }

    #[test]
    fn a_boolean_field_is_a_check_box() {
        let (mut session, mut ui) = author_session();
        press(&mut session, &mut ui, &[Key::Tab, Key::Tab, Key::Enter]);
        let required = area_of(&session, "required");
        let square = chrome::check_box_of(required);

        let ticked = render(&session, &ui);
        assert!(has_slot(&ticked, square, slot::FERN), "yes is a tick");
        click(&mut session, &mut ui, square.x + 4, square.y + 4);
        assert_eq!(session.view().value("required"), "false");
        let empty = render(&session, &ui);
        assert!(!has_slot(&empty, square, slot::FERN));
        // The box has the focus: a blue line around it.
        assert_eq!(empty.pixel(square.x - 1, square.y - 1), Some(slot::BLUE));

        press(&mut session, &mut ui, &[Key::Char(' ')]);
        assert_eq!(session.view().value("required"), "true");
    }

    #[test]
    fn a_new_field_starts_with_what_the_server_filled_in() {
        let (mut session, mut ui) = author_session();
        press(&mut session, &mut ui, &[Key::F(6)]);
        assert_eq!(session.view().stack(), "field");
        assert!(session.view().is_new);
        assert_eq!(session.view().value("type"), "text");
        assert_eq!(session.view().value("label"), "");

        press(&mut session, &mut ui, &[Key::Char('C'), Key::Char('i'), Key::Enter]);
        let (action, context) = *actions(&session).last().unwrap();
        assert_eq!(action, "save");
        assert_eq!(context.card, None);
        assert_eq!(context.dirty["label"], "Ci");
        assert_eq!(context.dirty["stack"], "addressbook", "the dialog knows its stack");
    }

    #[test]
    fn a_question_is_asked_in_a_window_and_waits_for_its_answer() {
        let (mut session, mut ui) = author_session();
        let (x, y) = centre(area_of(&session, "delete_stack"));
        click(&mut session, &mut ui, x, y);
        let question = session.question().expect("deleting asks first").clone();
        let frame = render(&session, &ui);
        snapshot(&frame, "question");

        let asked = QuestionBox::of(&question, &fonts());
        assert!(asked.lines.len() > 1, "a long question is cut into lines");
        assert!(count_slot(&frame, asked.accept, slot::YELLOW) > 300);
        assert_eq!(frame.pixel(asked.window.x + 2, asked.window.y + 2), Some(slot::AZURE));
        // The bar shows the two answers and nothing else.
        let before = actions(&session).len();

        // Any other key leaves the question where it is.
        press(&mut session, &mut ui, &[Key::F(6), Key::Char('x'), Key::Tab]);
        assert!(session.question().is_some());
        assert_eq!(actions(&session).len(), before);

        // Escape gives up.
        press(&mut session, &mut ui, &[Key::Escape]);
        assert_eq!(session.question(), None);
        assert_eq!(session.view().stack(), "author");
        assert_eq!(actions(&session).len(), before);

        // Asked again and accepted with Enter: the action runs for good.
        click(&mut session, &mut ui, x, y);
        press(&mut session, &mut ui, &[Key::Enter]);
        let (action, context) = *actions(&session).last().unwrap();
        assert_eq!(action, "delete_stack");
        assert!(context.confirmed);
        assert_eq!(session.message(), Some("Stack “Address book” deleted."));
    }

    #[test]
    fn a_question_is_answered_with_the_pointer_too() {
        let (mut session, mut ui) = author_session();
        session.run("delete_stack");
        let question = session.question().unwrap().clone();
        let asked = QuestionBox::of(&question, &fonts());

        // A click elsewhere does nothing; Cancel gives up.
        click(&mut session, &mut ui, 3, 100);
        assert!(session.question().is_some());
        let (x, y) = centre(asked.cancel);
        click(&mut session, &mut ui, x, y);
        assert_eq!(session.question(), None);

        session.run("delete_stack");
        let (x, y) = centre(asked.accept);
        click(&mut session, &mut ui, x, y);
        assert!(actions(&session).last().unwrap().1.confirmed);
    }

    #[test]
    fn escape_leaves_a_screen_like_its_close_box() {
        let (mut session, mut ui) = author_session();
        press(&mut session, &mut ui, &[Key::Escape]);
        assert_eq!(session.view().stack(), "addressbook");
    }

    #[test]
    fn a_card_is_a_sheet_of_paper_in_an_amber_window() {
        let (session, ui) = addressbook_session();
        let frame = render(&session, &ui);
        let geometry = Geometry::of(session.view(), &fonts());
        let window = geometry.window;
        assert_eq!(frame.pixel(window.x + 2, window.y + 2), Some(slot::AMBER));
        assert_eq!(frame.pixel(window.x + 3, window.bottom() - 3), Some(slot::SAND));

        let paper = geometry.on_screen(geometry.layout.paper.unwrap());
        assert_eq!(frame.pixel(paper.x, paper.y), Some(slot::INK));
        assert_eq!(frame.pixel(paper.x + 3, paper.y + 3), Some(slot::CREAM));
        // The red line under the title, the ruled lines under the fields.
        let header = geometry.on_screen(geometry.layout.header.unwrap());
        assert_eq!(frame.pixel(header.x + 40, header.bottom() - 1), Some(slot::RED));
        assert!(has_slot(&frame, paper, slot::ICE));
        // The title in large letters, the position in grey on its right.
        assert!(count_slot(&frame, header, slot::INK) > 200);
        assert!(has_slot(&frame, header, slot::GREY));
        // The first field has the focus: blue border around its box.
        let last_name = area_of(&session, "last_name");
        assert_eq!(frame.pixel(last_name.x - 1, last_name.y - 1), Some(slot::BLUE));
        let first_name = area_of(&session, "first_name");
        assert_eq!(frame.pixel(first_name.x - 1, first_name.y - 1), Some(slot::CREAM));
        // The picture of the card.
        let photo = area_of(&session, "photo");
        assert_eq!(frame.pixel(photo.x, photo.y), Some(slot::INK));
        assert_eq!(frame.pixel(photo.x + 1, photo.y + 1), Some(slot::RED));
        assert_eq!(frame.pixel(photo.x + 2, photo.y + 1), Some(slot::GREEN));
    }

    #[test]
    fn a_card_without_a_picture_shows_an_empty_frame() {
        let (mut session, mut ui) = addressbook_session();
        press(&mut session, &mut ui, &[Key::F(6)]);
        assert!(session.view().is_new);
        let frame = render(&session, &ui);
        let photo = area_of(&session, "photo");
        assert_eq!(frame.pixel(photo.x, photo.y), Some(slot::INK));
        assert!(has_slot(&frame, photo.inset(1), slot::ICE));
        assert!(!has_slot(&frame, photo.inset(1), slot::RED));
    }

    #[test]
    fn typing_edits_the_focused_field() {
        let (mut session, mut ui) = addressbook_session();
        press(
            &mut session,
            &mut ui,
            &[Key::Tab, Key::Char('!'), Key::Home, Key::Char('>'), Key::End, Key::Backspace],
        );
        assert_eq!(session.view().value("first_name"), ">Ada");
        assert_eq!(session.view().dirty().len(), 1);

        press(&mut session, &mut ui, &[Key::BackTab, Key::Char('s')]);
        assert_eq!(session.view().value("last_name"), "Lovelaces");
    }

    #[test]
    fn a_click_in_a_field_puts_the_cursor_there() {
        let (mut session, mut ui) = addressbook_session();
        let fonts = fonts();
        let first_name = area_of(&session, "first_name");
        let after_first_letter = first_name.x + chrome::FIELD_TEXT_X + fonts.regular.width("A");
        click(&mut session, &mut ui, after_first_letter, first_name.y + 5);
        assert_eq!(session.view().focus.as_deref(), Some("first_name"));

        press(&mut session, &mut ui, &[Key::Char('-')]);
        assert_eq!(session.view().value("first_name"), "A-da");
    }

    #[test]
    fn a_function_key_sends_the_card_and_what_was_typed() {
        let (mut session, mut ui) = addressbook_session();
        press(&mut session, &mut ui, &[Key::Char('s'), Key::F(8)]);
        let (action, context) = *actions(&session).last().unwrap();
        assert_eq!(action, "next_card");
        assert_eq!(context.card.as_deref(), Some("1"));
        assert_eq!(context.dirty.get("last_name").map(String::as_str), Some("Lovelaces"));
    }

    #[test]
    fn enter_in_a_field_runs_the_action_bound_to_enter() {
        let (mut session, mut ui) = addressbook_session();
        press(&mut session, &mut ui, &[Key::Enter]);
        assert_eq!(actions(&session).last().unwrap().0, "save_card");
    }

    #[test]
    fn the_keys_of_the_bar_can_be_clicked() {
        let (mut session, mut ui) = addressbook_session();
        let fonts = fonts();
        let keys: Vec<(String, String)> =
            session.view().screen.keys.iter().map(|k| (k.key.clone(), k.label.clone())).collect();
        let slots = chrome::key_slots(&fonts, keys.iter().map(|(k, l)| (k.as_str(), l.as_str())));
        let (_, _, f8) = slots.iter().find(|(name, _, _)| *name == "F8").unwrap();
        let (x, y) = centre(*f8);
        click(&mut session, &mut ui, x, y);
        assert_eq!(actions(&session).last().unwrap().0, "next_card");
    }

    #[test]
    fn the_close_box_goes_back() {
        let (mut session, mut ui) = addressbook_session();
        let window = Geometry::of(session.view(), &fonts()).window;
        let (x, y) = centre(close_box_of(window));
        click(&mut session, &mut ui, x, y);
        assert_eq!(actions(&session).last().unwrap().0, "back");
        assert_eq!(session.view().stack(), "stacks");
    }

    #[test]
    fn a_key_with_no_binding_says_so_in_a_strip() {
        let (mut session, mut ui) = addressbook_session();
        let strip = Rect::new(0, chrome::KEYS_TOP - 13, 480, 12);
        assert!(!has_slot(&render(&session, &ui), strip, slot::YELLOW));

        press(&mut session, &mut ui, &[Key::F(20)]);
        assert_eq!(session.message(), Some("This key is not active on this screen."));
        assert!(count_slot(&render(&session, &ui), strip, slot::YELLOW) > 4000);
    }

    #[test]
    fn the_cursor_blinks() {
        let (session, ui) = addressbook_session();
        let fonts = fonts();
        let last_name = area_of(&session, "last_name");
        let cursor_x = last_name.x + chrome::FIELD_TEXT_X + fonts.regular.width("Lovelace") - 1;

        let mut frame = Frame::new();
        draw(&session, &ui, &fonts, &mut frame, "16:54", true);
        assert_eq!(frame.pixel(cursor_x, last_name.y + 4), Some(slot::INK));
        draw(&session, &ui, &fonts, &mut frame, "16:54", false);
        assert_eq!(frame.pixel(cursor_x, last_name.y + 4), Some(slot::WHITE));
    }

    #[test]
    fn tab_reaches_the_buttons_and_space_presses_them() {
        let (mut session, mut ui) = addressbook_session();
        press(&mut session, &mut ui, &[Key::Tab, Key::Tab]);
        assert_eq!(session.view().focus.as_deref(), Some("next"));
        press(&mut session, &mut ui, &[Key::Char(' ')]);
        assert_eq!(actions(&session).last().unwrap().0, "next_card");
    }

    /// Not a check: writes what the screens look like, when asked to.
    #[test]
    fn the_screens_can_be_looked_at() {
        let mut session = Session::start(Fake::default()).unwrap();
        let mut ui = Ui::default();
        snapshot(&render(&session, &ui), "desktop");
        press(&mut session, &mut ui, &[Key::Enter]);
        snapshot(&render(&session, &ui), "stacks");
        let (mut session, mut ui) = addressbook_session();
        snapshot(&render(&session, &ui), "addressbook");
        press(&mut session, &mut ui, &[Key::F(20), Key::Tab]);
        snapshot(&render(&session, &ui), "addressbook-message");
    }
}
