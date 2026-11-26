//! Drawing a session on the text screen, and turning keys into intents.
//!
//! Nothing here knows what a stack does. The placement comes from
//! [`xila_core::text_layout`], the content from the session, and every key
//! that means something is sent to the server as an action.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use xila_core::api::{Api, Widget};
use xila_core::session::{GridState, Question, Session, View};
use xila_core::text_layout::{
    self, leader, Item, TextLayout, COLUMNS, KEYS_ROW, LABEL_WIDTH, MESSAGE_ROW, PROMPT, ROWS,
};

use crate::input::{edit_line, Key};
use crate::theme::Theme;

const PROMPT_LABEL: &str = "Option ===> ";
const OPTION_DIGITS: usize = 2;
/// Longest text a field accepts when its widget sets no limit.
const DEFAULT_MAX_LENGTH: usize = 200;

/// What the text client remembers between two keys, besides the session.
#[derive(Debug, Default)]
pub struct Ui {
    /// Position of the cursor in the focused field, in characters.
    cursor: usize,
    /// What was typed at the "Option ===>" prompt.
    option: String,
    /// What the cursor position refers to: stack, card and widget.
    anchor: Option<(String, Option<String>, String)>,
}

/// The 80 by 24 area the screen is drawn in, centred in the terminal, or
/// None when the terminal is too small.
pub fn screen_area(terminal: Rect) -> Option<Rect> {
    if terminal.width < COLUMNS || terminal.height < ROWS {
        return None;
    }
    Some(Rect {
        x: terminal.x + (terminal.width - COLUMNS) / 2,
        y: terminal.y + (terminal.height - ROWS) / 2,
        width: COLUMNS,
        height: ROWS,
    })
}

/// Writes text at a position of the screen area, cut to `width` cells.
pub fn put(buffer: &mut Buffer, area: Rect, x: u16, y: u16, text: &str, width: u16, style: Style) {
    if x >= area.width || y >= area.height {
        return;
    }
    let width = width.min(area.width - x);
    buffer.set_stringn(area.x + x, area.y + y, text, usize::from(width), style);
}

/// Draws a label with dot leaders, then an input field. Returns where the
/// terminal cursor goes when the field has the focus.
#[allow(clippy::too_many_arguments)]
pub fn draw_field(
    buffer: &mut Buffer,
    area: Rect,
    theme: &Theme,
    label: &str,
    label_x: u16,
    x: u16,
    y: u16,
    width: u16,
    text: &str,
    cursor: Option<usize>,
) -> Option<(u16, u16)> {
    put(
        buffer,
        area,
        label_x,
        y,
        &leader(label, usize::from(LABEL_WIDTH)),
        LABEL_WIDTH,
        theme.of(theme.label),
    );

    let focused = cursor.is_some();
    let visible = usize::from(width);
    let length = text.chars().count();
    let cursor = cursor.unwrap_or(0).min(length);
    // Scroll long values so that the cursor stays in sight.
    let start = (cursor + 1).saturating_sub(visible);
    let shown: String = text.chars().skip(start).take(visible).collect();
    put(buffer, area, x, y, &format!("{shown:<visible$}"), width, theme.field(focused));

    focused.then(|| (area.x + x + (cursor - start) as u16, area.y + y))
}

/// The widget that has the focus: the one the session remembers if it is
/// still on screen, or else the first that can take it.
fn focus_of(view: &View, layout: &TextLayout) -> Option<String> {
    view.focus
        .as_ref()
        .filter(|name| layout.focus_order.contains(name))
        .or_else(|| layout.focus_order.first())
        .cloned()
}

fn widget<'a>(view: &'a View, name: &str) -> Option<&'a Widget> {
    view.screen.widgets.iter().find(|w| w.name == name)
}

fn max_length(view: &View, name: &str) -> usize {
    widget(view, name)
        .and_then(|w| w.props.get("max_length"))
        .and_then(|v| v.as_u64())
        .map_or(DEFAULT_MAX_LENGTH, |n| n as usize)
}

/// The column a field widget edits.
fn column_of(layout: &TextLayout, name: &str) -> Option<String> {
    layout.items.iter().find_map(|item| match item {
        Item::Field { widget, column, .. } if widget == name => Some(column.clone()),
        _ => None,
    })
}

/// True for a field whose value is chosen among a few, not typed.
fn is_choice(view: &View, name: &str) -> bool {
    widget(view, name).is_some_and(|w| !w.choices().is_empty())
}

/// The column of a field in which text is typed.
fn typed_column_of(view: &View, layout: &TextLayout, name: &str) -> Option<String> {
    column_of(layout, name).filter(|_| !is_choice(view, name))
}

/// A question, cut into the two rows above the function keys.
fn question_lines(question: &Question) -> (String, String) {
    let width = usize::from(COLUMNS) - 2;
    let mut first = String::new();
    let mut second = String::new();
    for word in question.text.split_whitespace() {
        let line = if second.is_empty() && first.chars().count() + 1 + word.chars().count() <= width
        {
            &mut first
        } else {
            &mut second
        };
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    (first, second)
}

fn is_grid(layout: &TextLayout, name: &str) -> bool {
    layout.items.iter().any(|item| matches!(item, Item::Grid { widget, .. } if widget == name))
}

/// Handles a key: moves the focus, edits a field, or asks the server to run
/// an action.
pub fn handle<A: Api>(session: &mut Session<A>, ui: &mut Ui, key: Key) {
    // A question waits for its answer and nothing else.
    if session.question().is_some() {
        match key {
            Key::Enter => session.answer(true),
            Key::Escape | Key::F(12) => session.answer(false),
            _ => {}
        }
        let layout = text_layout::layout(&session.view().screen);
        let focus = focus_of(session.view(), &layout);
        sync_cursor(session, ui, &layout, &focus);
        return;
    }

    let layout = text_layout::layout(&session.view().screen);
    let focus = focus_of(session.view(), &layout);
    sync_cursor(session, ui, &layout, &focus);

    let on_grid = focus.as_deref().is_some_and(|name| is_grid(&layout, name));
    let on_choice = focus.as_deref().is_some_and(|name| is_choice(session.view(), name));
    match key {
        Key::Interrupt => {}
        // Escape leaves the screen, like F12.
        Key::Escape => {
            session.press("F12");
        }
        // A field whose value is chosen goes through its values.
        Key::Char(' ') | Key::Right if on_choice => {
            session.cycle(focus.as_deref().unwrap_or_default(), true);
        }
        Key::Left if on_choice => {
            session.cycle(focus.as_deref().unwrap_or_default(), false);
        }
        Key::F(number) => {
            if !session.press(&format!("F{number}")) {
                session.say("This key is not active on this screen.");
            }
        }
        Key::Up if on_grid => session.select_row(focus.as_deref().unwrap_or_default(), -1),
        Key::Down if on_grid => session.select_row(focus.as_deref().unwrap_or_default(), 1),
        Key::Tab | Key::Down => move_focus(session, &layout, &focus, 1),
        Key::BackTab | Key::Up => move_focus(session, &layout, &focus, -1),
        Key::Enter => match focus.as_deref() {
            Some(PROMPT) => {
                let chosen = ui.option.parse().ok().and_then(|number| layout.option(number));
                match chosen {
                    Some(action) => session.run(action),
                    None => session.say("There is no such option."),
                }
                ui.option.clear();
            }
            Some(name) if on_grid => session.activate(name),
            // In a field, Enter is a key like any other; when the screen
            // gives it no meaning, it moves on to the next field.
            _ => {
                if !session.press("Enter") {
                    move_focus(session, &layout, &focus, 1);
                }
            }
        },
        _ => match focus.as_deref() {
            Some(PROMPT) => match key {
                Key::Char(digit) if digit.is_ascii_digit() && ui.option.len() < OPTION_DIGITS => {
                    ui.option.push(digit);
                }
                Key::Backspace => {
                    ui.option.pop();
                }
                _ => {}
            },
            Some(name) => {
                if let Some(column) = typed_column_of(session.view(), &layout, name) {
                    let mut text = session.view().value(&column).to_owned();
                    let limit = max_length(session.view(), name);
                    if edit_line(&mut text, &mut ui.cursor, key, limit) {
                        session.edit(&column, text);
                    }
                }
            }
            None => {}
        },
    }

    // The action may have changed the card, the stack or the focus.
    let layout = text_layout::layout(&session.view().screen);
    let focus = focus_of(session.view(), &layout);
    sync_cursor(session, ui, &layout, &focus);
}

fn move_focus<A: Api>(
    session: &mut Session<A>,
    layout: &TextLayout,
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
    layout: &TextLayout,
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
        if ui.anchor.as_ref().map(|a| &a.0) != anchor.as_ref().map(|a| &a.0) {
            ui.option.clear();
        }
        ui.cursor = focus
            .as_ref()
            .and_then(|name| typed_column_of(view, layout, name))
            .map_or(0, |column| view.value(&column).chars().count());
        ui.anchor = anchor;
    }
}

/// Draws the session. Returns where the terminal cursor goes, if anywhere.
pub fn draw<A: Api>(
    session: &Session<A>,
    ui: &Ui,
    buffer: &mut Buffer,
    area: Rect,
) -> Option<(u16, u16)> {
    let theme = Theme::from_palette(session.palette());
    let view = session.view();
    let layout = text_layout::layout(&view.screen);
    let focus = focus_of(view, &layout);
    let mut cursor = None;

    buffer.set_style(area, theme.base());
    draw_header(session, buffer, area, &theme);

    for item in &layout.items {
        match item {
            Item::Text { widget: name, x, y, w } => {
                put(buffer, area, *x, *y, view.text(name), *w, theme.base());
            }
            Item::Field { widget: name, column, label, label_x, x, y, w } => {
                let focused = focus.as_deref() == Some(name);
                // A chosen value is shown by its label, the cursor at its start.
                let chosen = is_choice(view, name);
                let shown = match widget(view, name) {
                    Some(definition) if chosen => definition.shown(view.value(column)),
                    _ => view.value(column).to_owned(),
                };
                let at = draw_field(
                    buffer,
                    area,
                    &theme,
                    label,
                    *label_x,
                    *x,
                    *y,
                    *w,
                    &shown,
                    focused.then_some(if chosen { 0 } else { ui.cursor }),
                );
                cursor = cursor.or(at);
            }
            Item::Option { number, label, x, y, .. } => {
                put(buffer, area, *x, *y, &format!("{number:>2}. {label}"), COLUMNS, theme.base());
            }
            Item::Prompt { x, y } => {
                put(buffer, area, *x, *y, PROMPT_LABEL, COLUMNS, theme.of(theme.label));
                let input_x = *x + PROMPT_LABEL.len() as u16;
                let focused = focus.as_deref() == Some(PROMPT);
                let width = OPTION_DIGITS as u16;
                put(
                    buffer,
                    area,
                    input_x,
                    *y,
                    &format!("{:<OPTION_DIGITS$}", ui.option),
                    width,
                    theme.field(focused),
                );
                if focused {
                    cursor = Some((area.x + input_x + ui.option.len() as u16, area.y + *y));
                }
            }
            Item::Grid { widget: name, x, y, w, h } => {
                if let (Some(definition), Some(state)) = (widget(view, name), view.grids.get(name))
                {
                    draw_grid(buffer, area, &theme, definition, state, (*x, *y, *w, *h));
                }
            }
        }
    }

    // A question takes the place of the message and of the keys.
    if let Some(question) = session.question() {
        let (first, second) = question_lines(question);
        let style = theme.of(theme.message);
        if second.is_empty() {
            put(buffer, area, 1, MESSAGE_ROW, &first, COLUMNS - 2, style);
        } else {
            put(buffer, area, 1, MESSAGE_ROW - 1, &first, COLUMNS - 2, style);
            put(buffer, area, 1, MESSAGE_ROW, &second, COLUMNS - 2, style);
        }
        let keys = [("Enter", question.label.as_str()), ("F12", "Cancel")];
        draw_keys(buffer, area, &theme, keys.into_iter());
        return None;
    }

    let hint = match focus.as_deref() {
        Some(PROMPT) => "Type the number of an option, then press Enter.",
        Some(name) if is_grid(&layout, name) => "Choose a row with the arrows, then press Enter.",
        Some(name) if is_choice(view, name) => "Press the space bar to change this value.",
        _ => "",
    };
    match session.message() {
        Some(message) => {
            put(buffer, area, 1, MESSAGE_ROW, message, COLUMNS - 2, theme.of(theme.message))
        }
        None => put(buffer, area, 1, MESSAGE_ROW, hint, COLUMNS - 2, theme.of(theme.label)),
    }

    draw_keys(
        buffer,
        area,
        &theme,
        view.screen.keys.iter().map(|k| (k.key.as_str(), k.label.as_str())),
    );
    cursor
}

/// First row: stack name, title, position in the stack. Static screens also
/// say where and who we are.
fn draw_header<A: Api>(session: &Session<A>, buffer: &mut Buffer, area: Rect, theme: &Theme) {
    let view = session.view();
    let title = &view.screen.stack.title;
    // The screens of the system itself are signed by the system.
    let name = match view.screen.stack.kind.as_str() {
        "system" => "PGXILA".to_owned(),
        _ => view.screen.stack.name.to_uppercase(),
    };
    put(buffer, area, 1, 0, &name, 20, theme.of(theme.title));
    let title_x = (COLUMNS.saturating_sub(title.chars().count() as u16)) / 2;
    put(buffer, area, title_x, 0, title, COLUMNS, theme.of(theme.title));

    // The dialogs of the system show a card too, but not one of a stack.
    if view.screen.source.has_cards && !view.screen.stack.is_system() {
        let position = if view.is_new {
            "New card".to_owned()
        } else if view.card.count == 0 {
            "No card".to_owned()
        } else {
            format!("Card {}/{}", view.card.position, view.card.count)
        };
        put_right(buffer, area, 0, &position, theme.of(theme.title));
    } else {
        let hello = session.hello();
        put(buffer, area, 1, 1, &format!("Instance: {}", hello.instance), 40, theme.base());
        put_right(buffer, area, 1, &format!("User: {}", hello.role.to_uppercase()), theme.base());
    }
}

fn put_right(buffer: &mut Buffer, area: Rect, y: u16, text: &str, style: Style) {
    let width = text.chars().count() as u16;
    put(buffer, area, (COLUMNS - 1).saturating_sub(width), y, text, width, style);
}

/// Last row: the function keys of the screen, `F3=Quit  F8=Next`.
pub fn draw_keys<'a>(
    buffer: &mut Buffer,
    area: Rect,
    theme: &Theme,
    keys: impl Iterator<Item = (&'a str, &'a str)>,
) {
    let mut x = 1;
    for (key, label) in keys {
        let width = (key.chars().count() + 1 + label.chars().count()) as u16;
        if x + width >= COLUMNS {
            break;
        }
        put(buffer, area, x, KEYS_ROW, key, COLUMNS, theme.of(theme.key));
        put(
            buffer,
            area,
            x + key.chars().count() as u16,
            KEYS_ROW,
            &format!("={label}"),
            COLUMNS,
            theme.base(),
        );
        x += width + 2;
    }
}

/// A list of rows with a header, the selected row highlighted.
fn draw_grid(
    buffer: &mut Buffer,
    area: Rect,
    theme: &Theme,
    definition: &Widget,
    state: &GridState,
    (x, y, w, h): (u16, u16, u16, u16),
) {
    const MARKER: u16 = 2;
    const NARROW: u16 = 10;

    struct Column<'a> {
        name: &'a str,
        label: &'a str,
        right: bool,
        width: u16,
    }
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
                width: 0,
            })
        })
        .collect();

    // Right-aligned columns are narrow; the others share what is left.
    let narrow = columns.iter().filter(|c| c.right).count() as u16;
    let wide = (columns.len() as u16 - narrow).max(1);
    let wide_width = w.saturating_sub(MARKER + narrow * NARROW) / wide;
    for column in &mut columns {
        column.width = if column.right { NARROW } else { wide_width };
    }

    let cell = |text: &str, column: &Column| {
        let width = usize::from(column.width.saturating_sub(1));
        let cut: String = text.chars().take(width).collect();
        if column.right {
            format!("{cut:>width$} ")
        } else {
            format!("{cut:<width$} ")
        }
    };

    let header: String = columns.iter().map(|c| cell(c.label, c)).collect();
    put(buffer, area, x + MARKER, y, &header, w - MARKER, theme.of(theme.label));

    if state.page.rows.is_empty() {
        let empty = definition.props.get("empty").and_then(|v| v.as_str()).unwrap_or("No rows.");
        put(buffer, area, x + MARKER, y + 1, empty, w - MARKER, theme.base());
        return;
    }

    let visible = usize::from(h.saturating_sub(1)).max(1);
    let first = (state.selected + 1).saturating_sub(visible);
    for (line, (index, row)) in
        state.page.rows.iter().enumerate().skip(first).take(visible).enumerate()
    {
        let selected = index == state.selected;
        let style = if selected { theme.selected() } else { theme.base() };
        let text: String = columns
            .iter()
            .map(|c| cell(row.cells.get(c.name).and_then(|v| v.as_deref()).unwrap_or(""), c))
            .collect();
        let row_y = y + 1 + line as u16;
        put(buffer, area, x, row_y, if selected { "> " } else { "  " }, MARKER, style);
        put(
            buffer,
            area,
            x + MARKER,
            row_y,
            &format!("{text:<width$}", width = usize::from(w - MARKER)),
            w - MARKER,
            style,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use xila_core::api::{
        ApiError, Card, Context, Effect, GridPage, GridRow, Hello, Image, Move, Notification,
        PaletteColour, Screen,
    };

    /// A server in memory with a menu, a stack list and an address book.
    #[derive(Default)]
    struct Fake {
        actions: Vec<(String, Context)>,
    }

    fn addressbook() -> Screen {
        serde_json::from_value(serde_json::json!({
            "stack": { "name": "addressbook", "title": "Address book", "kind": "user" },
            "source": { "has_cards": true, "count": 42 },
            "widgets": [
                { "name": "title", "kind": "text", "ord": 10, "props": { "template": "{last_name}" },
                  "layout": { "text": { "x": 0, "y": 0, "w": 0, "h": 0, "style": { "hidden": true } } } },
                { "name": "last_name", "kind": "field", "ord": 20, "label": "Last name", "column": "last_name" },
                { "name": "first_name", "kind": "field", "ord": 30, "label": "First name", "column": "first_name" },
                { "name": "next", "kind": "button", "ord": 90, "label": "Next", "action": "next_card" }
            ],
            "actions": [],
            "keys": [
                { "key": "F3", "label": "Quit", "action": "quit" },
                { "key": "F8", "label": "Next", "action": "next_card" },
                { "key": "Enter", "label": "Save", "action": "save_card" }
            ]
        }))
        .unwrap()
    }

    fn desktop() -> Screen {
        serde_json::from_value(serde_json::json!({
            "stack": { "name": "desktop", "title": "Main menu", "kind": "system" },
            "source": { "has_cards": false, "count": 1 },
            "widgets": [
                { "name": "stacks", "kind": "button", "ord": 10, "label": "My stacks", "action": "open_stacks" }
            ],
            "actions": [],
            "keys": [{ "key": "F3", "label": "Quit", "action": "quit" }]
        }))
        .unwrap()
    }

    fn stacks() -> Screen {
        serde_json::from_value(serde_json::json!({
            "stack": { "name": "stacks", "title": "My stacks", "kind": "system" },
            "source": { "has_cards": false, "count": 1 },
            "widgets": [
                { "name": "list", "kind": "grid", "ord": 10, "action": "open", "props": {
                    "columns": [ { "column": "title", "label": "Stack" },
                                 { "column": "cards", "label": "Cards", "align": "right" } ] } }
            ],
            "actions": [],
            "keys": [{ "key": "F12", "label": "Back", "action": "back" }]
        }))
        .unwrap()
    }

    /// Author mode on one stack: fields, a list, and a button without a key.
    fn author() -> Screen {
        serde_json::from_value(serde_json::json!({
            "stack": { "name": "author", "title": "Author mode", "kind": "system" },
            "source": { "has_cards": true, "count": 3 },
            "widgets": [
                { "name": "heading", "kind": "text", "ord": 10, "props": { "template": "{title}" },
                  "layout": { "text": { "x": 0, "y": 0, "w": 0, "h": 0, "style": { "hidden": true } } } },
                { "name": "title", "kind": "field", "ord": 30, "label": "Title", "column": "title", "type": "text" },
                { "name": "kind", "kind": "field", "ord": 35, "label": "Type", "column": "kind", "type": "text",
                  "props": { "choices": [ { "value": "text", "label": "Text" },
                                          { "value": "date", "label": "Date" } ] } },
                { "name": "shared", "kind": "field", "ord": 37, "label": "Shared", "column": "shared", "type": "boolean" },
                { "name": "fields", "kind": "grid", "ord": 50, "action": "edit_field", "props": {
                    "columns": [ { "column": "title", "label": "Field" },
                                 { "column": "cards", "label": "Cards", "align": "right" } ] } },
                { "name": "new_field", "kind": "button", "ord": 60, "label": "New field", "action": "new_field" },
                { "name": "delete_stack", "kind": "button", "ord": 80, "label": "Delete stack", "action": "delete_stack" }
            ],
            "actions": [],
            "keys": [
                { "key": "F6", "label": "New field", "action": "new_field" },
                { "key": "F12", "label": "Back", "action": "back" },
                { "key": "Enter", "label": "Save", "action": "save" }
            ]
        }))
        .unwrap()
    }

    impl Api for Fake {
        fn hello(&mut self) -> Result<Hello, ApiError> {
            Ok(Hello {
                api: xila_core::session::API_VERSION,
                version: "0.1.0".into(),
                instance: "normandie-01".into(),
                role: "fax".into(),
                author: false,
                home: "desktop".into(),
            })
        }

        fn palette(&mut self) -> Result<Vec<PaletteColour>, ApiError> {
            Ok(vec![])
        }

        fn image(&mut self, _id: &str) -> Result<Option<Image>, ApiError> {
            Ok(None)
        }

        fn open_stack(&mut self, stack: &str) -> Result<Screen, ApiError> {
            Ok(match stack {
                "desktop" => desktop(),
                "stacks" => stacks(),
                "author" => author(),
                _ => addressbook(),
            })
        }

        fn card(&mut self, stack: &str, _key: Option<&str>, _to: Move) -> Result<Card, ApiError> {
            if stack == "author" {
                return Ok(Card {
                    key: Some("addressbook".into()),
                    position: 1,
                    count: 3,
                    values: BTreeMap::from([
                        ("title".to_string(), Some("Address book".to_string())),
                        ("kind".to_string(), Some("text".to_string())),
                        ("shared".to_string(), Some("false".to_string())),
                    ]),
                    ..Card::default()
                });
            }
            if stack != "addressbook" {
                return Ok(Card { position: 1, count: 1, ..Card::default() });
            }
            Ok(Card {
                key: Some("3".into()),
                position: 3,
                count: 42,
                values: BTreeMap::from([
                    ("last_name".to_string(), Some("Lovelace".to_string())),
                    ("first_name".to_string(), Some("Ada".to_string())),
                ]),
                ..Card::default()
            })
        }

        fn grid_rows(
            &mut self,
            _stack: &str,
            _widget: &str,
            _card: Option<&str>,
        ) -> Result<GridPage, ApiError> {
            let row = |key: &str, title: &str, cards: &str| GridRow {
                key: key.into(),
                cells: BTreeMap::from([
                    ("title".to_string(), Some(title.to_string())),
                    ("cards".to_string(), Some(cards.to_string())),
                ]),
            };
            Ok(GridPage {
                total: 2,
                rows: vec![
                    row("addressbook", "Address book", "42"),
                    row("recipes", "Recipes", "27"),
                ],
            })
        }

        fn act(
            &mut self,
            _stack: &str,
            action: &str,
            context: &Context,
        ) -> Result<Vec<Effect>, ApiError> {
            self.actions.push((action.to_owned(), context.clone()));
            Ok(match action {
                "open_stacks" => vec![Effect::Show { stack: "stacks".into(), card: None }],
                "open" => vec![Effect::Show { stack: context.row.clone().unwrap(), card: None }],
                "back" => vec![Effect::Back],
                "quit" => vec![Effect::Quit],
                "author" => vec![Effect::Show { stack: "author".into(), card: None }],
                "delete_stack" if !context.confirmed => vec![Effect::Confirm {
                    text: "Delete the stack “Address book”? Its cards are kept, and the stack \
                           can be restored."
                        .into(),
                    label: "Delete".into(),
                }],
                "delete_stack" => vec![Effect::Message { text: "Stack deleted.".into() }],
                _ => vec![],
            })
        }

        fn notifications(&mut self) -> Vec<Notification> {
            vec![]
        }
    }

    /// The screen as lines of text, and the cursor position.
    fn render(session: &Session<Fake>, ui: &Ui) -> (Vec<String>, Option<(u16, u16)>) {
        let area = Rect::new(0, 0, COLUMNS, ROWS);
        let mut buffer = Buffer::empty(area);
        let cursor = draw(session, ui, &mut buffer, area);
        let lines = (0..ROWS)
            .map(|y| {
                let line: String = (0..COLUMNS).map(|x| buffer[(x, y)].symbol()).collect();
                line.trim_end().to_owned()
            })
            .collect();
        (lines, cursor)
    }

    fn press(session: &mut Session<Fake>, ui: &mut Ui, keys: &[Key]) {
        for key in keys {
            handle(session, ui, *key);
        }
    }

    /// A session taken from the menu to the address book.
    fn on_addressbook() -> (Session<Fake>, Ui) {
        let mut session = Session::start(Fake::default()).unwrap();
        let mut ui = Ui::default();
        press(&mut session, &mut ui, &[Key::Char('1'), Key::Enter, Key::Enter]);
        assert_eq!(session.view().stack(), "addressbook");
        (session, ui)
    }

    #[test]
    fn the_menu_lists_the_buttons_as_numbered_options() {
        let session = Session::start(Fake::default()).unwrap();
        let (lines, cursor) = render(&session, &Ui::default());

        assert_eq!(lines[0], " PGXILA                            Main menu");
        assert_eq!(lines[1], format!(" {:<69}{}", "Instance: normandie-01", "User: FAX"));
        assert_eq!(lines[3], "    1. My stacks");
        assert_eq!(lines[5], " Option ===>");
        assert_eq!(lines[22], " Type the number of an option, then press Enter.");
        assert_eq!(lines[23], " F3=Quit");
        assert_eq!(cursor, Some((13, 5)));
    }

    #[test]
    fn choosing_an_option_runs_its_action() {
        let mut session = Session::start(Fake::default()).unwrap();
        let mut ui = Ui::default();
        press(&mut session, &mut ui, &[Key::Char('1'), Key::Enter]);
        assert_eq!(session.view().stack(), "stacks");

        let (lines, _) = render(&session, &ui);
        assert_eq!(lines[0], " PGXILA                            My stacks");
        assert_eq!(lines[3], format!("   {:<65} {:>9}", "Stack", "Cards"));
        assert_eq!(lines[4], format!(" > {:<65} {:>9}", "Address book", "42"));
        assert_eq!(lines[5], format!("   {:<65} {:>9}", "Recipes", "27"));
        assert_eq!(lines[23], " F12=Back");
    }

    #[test]
    fn an_unknown_option_is_refused_with_a_message() {
        let mut session = Session::start(Fake::default()).unwrap();
        let mut ui = Ui::default();
        press(&mut session, &mut ui, &[Key::Char('7'), Key::Enter]);
        assert_eq!(session.view().stack(), "desktop");
        assert_eq!(session.message(), Some("There is no such option."));
    }

    #[test]
    fn arrows_choose_a_row_and_enter_opens_it() {
        let mut session = Session::start(Fake::default()).unwrap();
        let mut ui = Ui::default();
        press(&mut session, &mut ui, &[Key::Char('1'), Key::Enter, Key::Down]);
        let (lines, _) = render(&session, &ui);
        assert!(lines[5].starts_with(" > Recipes"), "{}", lines[5]);

        press(&mut session, &mut ui, &[Key::Enter]);
        let (action, context) = session_actions(&session).last().unwrap();
        assert_eq!(action, "open");
        assert_eq!(context.row.as_deref(), Some("recipes"));
    }

    #[test]
    fn a_card_shows_its_fields_with_leaders_and_its_function_keys() {
        let (session, ui) = on_addressbook();
        let (lines, cursor) = render(&session, &ui);

        assert_eq!(
            lines[0],
            " ADDRESSBOOK                      Address book                        Card 3/42"
        );
        assert_eq!(lines[3], " Last name . . . :  Lovelace");
        assert_eq!(lines[4], " First name  . . :  Ada");
        assert_eq!(lines[23], " F3=Quit  F8=Next  Enter=Save");
        // The button is not repeated: F8 already offers its action.
        assert!(!lines.iter().any(|line| line.contains("1. Next")));
        // The cursor waits at the end of the first field.
        assert_eq!(cursor, Some((28, 3)));
    }

    #[test]
    fn typing_edits_the_focused_field() {
        let (mut session, mut ui) = on_addressbook();
        press(
            &mut session,
            &mut ui,
            &[
                Key::Tab,
                Key::Backspace,
                Key::Backspace,
                Key::Char('l'),
                Key::Char('a'),
                Key::Char('n'),
            ],
        );

        assert_eq!(session.view().value("first_name"), "Alan");
        assert_eq!(session.view().value("last_name"), "Lovelace");
        let (lines, cursor) = render(&session, &ui);
        assert_eq!(lines[4], " First name  . . :  Alan");
        assert_eq!(cursor, Some((24, 4)));
    }

    #[test]
    fn a_function_key_sends_the_card_and_what_was_typed() {
        let (mut session, mut ui) = on_addressbook();
        press(&mut session, &mut ui, &[Key::Char('!'), Key::F(8)]);

        let (action, context) = session_actions(&session).last().unwrap().clone();
        assert_eq!(action, "next_card");
        assert_eq!(context.card.as_deref(), Some("3"));
        assert_eq!(context.dirty["last_name"], "Lovelace!");
    }

    #[test]
    fn enter_in_a_field_runs_the_action_bound_to_enter() {
        let (mut session, mut ui) = on_addressbook();
        press(&mut session, &mut ui, &[Key::Enter]);
        assert_eq!(session_actions(&session).last().unwrap().0, "save_card");
    }

    #[test]
    fn a_key_with_no_binding_says_so() {
        let (mut session, mut ui) = on_addressbook();
        press(&mut session, &mut ui, &[Key::F(5)]);
        let (lines, _) = render(&session, &ui);
        assert_eq!(lines[22], " This key is not active on this screen.");
    }

    #[test]
    fn quitting_goes_through_the_server_like_any_action() {
        let (mut session, mut ui) = on_addressbook();
        press(&mut session, &mut ui, &[Key::F(3)]);
        assert!(session.should_quit());
    }

    /// A session in author mode on the address book.
    fn in_author_mode() -> (Session<Fake>, Ui) {
        let (mut session, mut ui) = on_addressbook();
        session.run("author");
        handle(&mut session, &mut ui, Key::Home);
        assert_eq!(session.view().stack(), "author");
        (session, ui)
    }

    #[test]
    fn a_dialog_shows_fields_a_list_and_its_options() {
        let (session, ui) = in_author_mode();
        let (lines, _) = render(&session, &ui);

        // A dialog of the system: no position in a stack, but where we are.
        assert_eq!(lines[0], " PGXILA                           Author mode");
        assert!(lines[1].starts_with(" Instance: normandie-01"), "{}", lines[1]);
        assert_eq!(lines[3], " Title . . . . . :  Address book");
        assert_eq!(lines[4], " Type  . . . . . :  Text");
        assert_eq!(lines[5], " Shared  . . . . :  No");
        // The list leaves room for the option under it and for the prompt.
        assert!(lines[7].trim_start().starts_with("Field"), "{}", lines[7]);
        assert_eq!(
            lines[8],
            " > Address book                                                             42"
        );
        assert_eq!(lines[18], "    1. Delete stack");
        assert_eq!(lines[20], " Option ===>");
        assert_eq!(lines[23], " F6=New field  F12=Back  Enter=Save");
    }

    #[test]
    fn a_field_with_choices_goes_through_them() {
        let (mut session, mut ui) = in_author_mode();
        press(&mut session, &mut ui, &[Key::Tab]);
        assert_eq!(session.view().focus.as_deref(), Some("kind"));
        assert_eq!(render(&session, &ui).0[22], " Press the space bar to change this value.");

        // Typing does not change a choice; space and the arrows do.
        press(&mut session, &mut ui, &[Key::Char('x')]);
        assert_eq!(session.view().value("kind"), "text");
        press(&mut session, &mut ui, &[Key::Char(' ')]);
        assert_eq!(session.view().value("kind"), "date");
        assert_eq!(render(&session, &ui).0[4], " Type  . . . . . :  Date");
        press(&mut session, &mut ui, &[Key::Left]);
        assert_eq!(session.view().value("kind"), "text");

        // Yes or no is such a choice.
        press(&mut session, &mut ui, &[Key::Tab, Key::Char(' ')]);
        assert_eq!(session.view().value("shared"), "true");
        assert_eq!(render(&session, &ui).0[5], " Shared  . . . . :  Yes");
    }

    #[test]
    fn a_question_takes_the_bottom_of_the_screen_until_it_is_answered() {
        let (mut session, mut ui) = in_author_mode();
        // The option under the list: Tab past the fields and the list to the
        // prompt, then 1 and Enter.
        let to_the_prompt = [Key::Tab, Key::Tab, Key::Tab, Key::Tab];
        press(&mut session, &mut ui, &to_the_prompt);
        assert_eq!(session.view().focus.as_deref(), Some(PROMPT));
        press(&mut session, &mut ui, &[Key::Char('1'), Key::Enter]);
        assert!(session.question().is_some());
        let (lines, cursor) = render(&session, &ui);
        assert_eq!(
            lines[21],
            " Delete the stack “Address book”? Its cards are kept, and the stack can be"
        );
        assert_eq!(lines[22], " restored.");
        assert_eq!(lines[23], " Enter=Delete  F12=Cancel");
        assert_eq!(cursor, None, "nothing is typed while a question is asked");

        // Other keys leave it there; Escape gives up.
        let asked = session.api().actions.len();
        press(&mut session, &mut ui, &[Key::F(6), Key::Char('x')]);
        assert!(session.question().is_some());
        press(&mut session, &mut ui, &[Key::Escape]);
        assert_eq!(session.question(), None);
        assert_eq!(session.api().actions.len(), asked);

        // Asked again and accepted.
        press(&mut session, &mut ui, &[Key::Char('1'), Key::Enter, Key::Enter]);
        let (action, context) = session.api().actions.last().unwrap();
        assert_eq!(action, "delete_stack");
        assert!(context.confirmed);
        assert_eq!(render(&session, &ui).0[22], " Stack deleted.");
    }

    #[test]
    fn escape_goes_back_like_f12() {
        let (mut session, mut ui) = in_author_mode();
        press(&mut session, &mut ui, &[Key::Escape]);
        assert_eq!(session.view().stack(), "addressbook");
    }

    #[test]
    fn a_terminal_smaller_than_the_screen_has_no_area() {
        assert_eq!(screen_area(Rect::new(0, 0, 79, 24)), None);
        assert_eq!(screen_area(Rect::new(0, 0, 100, 30)), Some(Rect::new(10, 3, 80, 24)));
    }

    fn session_actions(session: &Session<Fake>) -> &[(String, Context)] {
        &session.api().actions
    }
}
