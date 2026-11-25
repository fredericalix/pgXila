//! Drawing a session on the text screen, and turning keys into intents.
//!
//! Nothing here knows what a stack does. The placement comes from
//! [`xila_core::text_layout`], the content from the session, and every key
//! that means something is sent to the server as an action.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use xila_core::api::{Api, Widget};
use xila_core::session::{GridState, Session, View};
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

fn is_grid(layout: &TextLayout, name: &str) -> bool {
    layout.items.iter().any(|item| matches!(item, Item::Grid { widget, .. } if widget == name))
}

/// Handles a key: moves the focus, edits a field, or asks the server to run
/// an action.
pub fn handle<A: Api>(session: &mut Session<A>, ui: &mut Ui, key: Key) {
    let layout = text_layout::layout(&session.view().screen);
    let focus = focus_of(session.view(), &layout);
    sync_cursor(session, ui, &layout, &focus);

    let on_grid = focus.as_deref().is_some_and(|name| is_grid(&layout, name));
    match key {
        Key::Interrupt => {}
        Key::F(number) => {
            if !session.press(&format!("F{number}")) {
                session.say("Cette touche n'est pas active sur cet écran.");
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
                    None => session.say("Cette option n'existe pas."),
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
                if let Some(column) = column_of(&layout, name) {
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
            .and_then(|name| column_of(layout, name))
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
                let text = view
                    .card
                    .texts
                    .get(name)
                    .map(String::as_str)
                    .or_else(|| widget(view, name).and_then(|w| w.label.as_deref()))
                    .unwrap_or("");
                put(buffer, area, *x, *y, text, *w, theme.base());
            }
            Item::Field { widget: name, column, label, label_x, x, y, w } => {
                let focused = focus.as_deref() == Some(name);
                let at = draw_field(
                    buffer,
                    area,
                    &theme,
                    label,
                    *label_x,
                    *x,
                    *y,
                    *w,
                    view.value(column),
                    focused.then_some(ui.cursor),
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

    let hint = match focus.as_deref() {
        Some(PROMPT) => "Tapez le numéro d'une option, puis appuyez sur Entrée.",
        Some(name) if is_grid(&layout, name) => {
            "Choisissez une ligne avec les flèches, puis appuyez sur Entrée."
        }
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

    if view.screen.source.has_cards {
        let position = if view.is_new {
            "Nouvelle fiche".to_owned()
        } else if view.card.count == 0 {
            "Aucune fiche".to_owned()
        } else {
            format!("Fiche {}/{}", view.card.position, view.card.count)
        };
        put_right(buffer, area, 0, &position, theme.of(theme.title));
    } else {
        let hello = session.hello();
        put(buffer, area, 1, 1, &format!("Instance : {}", hello.instance), 40, theme.base());
        put_right(
            buffer,
            area,
            1,
            &format!("Utilisateur : {}", hello.role.to_uppercase()),
            theme.base(),
        );
    }
}

fn put_right(buffer: &mut Buffer, area: Rect, y: u16, text: &str, style: Style) {
    let width = text.chars().count() as u16;
    put(buffer, area, (COLUMNS - 1).saturating_sub(width), y, text, width, style);
}

/// Last row: the function keys of the screen, `F3=Quitter  F8=Suiv.`.
pub fn draw_keys<'a>(
    buffer: &mut Buffer,
    area: Rect,
    theme: &Theme,
    keys: impl Iterator<Item = (&'a str, &'a str)>,
) {
    let mut x = 1;
    for (key, label) in keys {
        let key = if key == "Enter" { "Entrée" } else { key };
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
        let empty =
            definition.props.get("empty").and_then(|v| v.as_str()).unwrap_or("Aucune ligne.");
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
        ApiError, Card, Context, Effect, GridPage, GridRow, Hello, Move, Notification,
        PaletteColour, Screen,
    };

    /// A server in memory with a menu, a stack list and an address book.
    #[derive(Default)]
    struct Fake {
        actions: Vec<(String, Context)>,
    }

    fn carnet() -> Screen {
        serde_json::from_value(serde_json::json!({
            "stack": { "name": "carnet", "title": "Carnet d'adresses", "kind": "user" },
            "source": { "has_cards": true, "count": 42 },
            "widgets": [
                { "name": "titre", "kind": "text", "ord": 10, "props": { "template": "{nom}" },
                  "layout": { "text": { "x": 0, "y": 0, "w": 0, "h": 0, "style": { "hidden": true } } } },
                { "name": "nom", "kind": "field", "ord": 20, "label": "Nom", "column": "nom" },
                { "name": "prenom", "kind": "field", "ord": 30, "label": "Prénom", "column": "prenom" },
                { "name": "suivante", "kind": "button", "ord": 90, "label": "Suiv.", "action": "next_card" }
            ],
            "actions": [],
            "keys": [
                { "key": "F3", "label": "Quitter", "action": "quit" },
                { "key": "F8", "label": "Suiv.", "action": "next_card" },
                { "key": "Enter", "label": "Enregistrer", "action": "save_card" }
            ]
        }))
        .unwrap()
    }

    fn desktop() -> Screen {
        serde_json::from_value(serde_json::json!({
            "stack": { "name": "desktop", "title": "Menu principal", "kind": "system" },
            "source": { "has_cards": false, "count": 1 },
            "widgets": [
                { "name": "stacks", "kind": "button", "ord": 10, "label": "Mes piles", "action": "open_stacks" }
            ],
            "actions": [],
            "keys": [{ "key": "F3", "label": "Quitter", "action": "quit" }]
        }))
        .unwrap()
    }

    fn stacks() -> Screen {
        serde_json::from_value(serde_json::json!({
            "stack": { "name": "stacks", "title": "Mes piles", "kind": "system" },
            "source": { "has_cards": false, "count": 1 },
            "widgets": [
                { "name": "list", "kind": "grid", "ord": 10, "action": "open", "props": {
                    "columns": [ { "column": "title", "label": "Pile" },
                                 { "column": "cards", "label": "Cartes", "align": "right" } ] } }
            ],
            "actions": [],
            "keys": [{ "key": "F12", "label": "Retour", "action": "back" }]
        }))
        .unwrap()
    }

    impl Api for Fake {
        fn hello(&mut self) -> Result<Hello, ApiError> {
            Ok(Hello {
                api: 1,
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

        fn open_stack(&mut self, stack: &str) -> Result<Screen, ApiError> {
            Ok(match stack {
                "desktop" => desktop(),
                "stacks" => stacks(),
                _ => carnet(),
            })
        }

        fn card(&mut self, stack: &str, _key: Option<&str>, _to: Move) -> Result<Card, ApiError> {
            if stack != "carnet" {
                return Ok(Card { position: 1, count: 1, ..Card::default() });
            }
            Ok(Card {
                key: Some("3".into()),
                position: 3,
                count: 42,
                values: BTreeMap::from([
                    ("nom".to_string(), Some("Lovelace".to_string())),
                    ("prenom".to_string(), Some("Ada".to_string())),
                ]),
                ..Card::default()
            })
        }

        fn grid_rows(&mut self, _stack: &str, _widget: &str) -> Result<GridPage, ApiError> {
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
                    row("carnet", "Carnet d'adresses", "42"),
                    row("recettes", "Recettes", "27"),
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
    fn on_carnet() -> (Session<Fake>, Ui) {
        let mut session = Session::start(Fake::default()).unwrap();
        let mut ui = Ui::default();
        press(&mut session, &mut ui, &[Key::Char('1'), Key::Enter, Key::Enter]);
        assert_eq!(session.view().stack(), "carnet");
        (session, ui)
    }

    #[test]
    fn the_menu_lists_the_buttons_as_numbered_options() {
        let session = Session::start(Fake::default()).unwrap();
        let (lines, cursor) = render(&session, &Ui::default());

        assert_eq!(lines[0], " PGXILA                          Menu principal");
        assert_eq!(
            lines[1],
            " Instance : normandie-01                                      Utilisateur : FAX"
        );
        assert_eq!(lines[3], "    1. Mes piles");
        assert_eq!(lines[5], " Option ===>");
        assert_eq!(lines[22], " Tapez le numéro d'une option, puis appuyez sur Entrée.");
        assert_eq!(lines[23], " F3=Quitter");
        assert_eq!(cursor, Some((13, 5)));
    }

    #[test]
    fn choosing_an_option_runs_its_action() {
        let mut session = Session::start(Fake::default()).unwrap();
        let mut ui = Ui::default();
        press(&mut session, &mut ui, &[Key::Char('1'), Key::Enter]);
        assert_eq!(session.view().stack(), "stacks");

        let (lines, _) = render(&session, &ui);
        assert_eq!(lines[0], " PGXILA                            Mes piles");
        assert_eq!(lines[3], format!("   {:<65} {:>9}", "Pile", "Cartes"));
        assert_eq!(lines[4], format!(" > {:<65} {:>9}", "Carnet d'adresses", "42"));
        assert_eq!(lines[5], format!("   {:<65} {:>9}", "Recettes", "27"));
        assert_eq!(lines[23], " F12=Retour");
    }

    #[test]
    fn an_unknown_option_is_refused_with_a_message() {
        let mut session = Session::start(Fake::default()).unwrap();
        let mut ui = Ui::default();
        press(&mut session, &mut ui, &[Key::Char('7'), Key::Enter]);
        assert_eq!(session.view().stack(), "desktop");
        assert_eq!(session.message(), Some("Cette option n'existe pas."));
    }

    #[test]
    fn arrows_choose_a_row_and_enter_opens_it() {
        let mut session = Session::start(Fake::default()).unwrap();
        let mut ui = Ui::default();
        press(&mut session, &mut ui, &[Key::Char('1'), Key::Enter, Key::Down]);
        let (lines, _) = render(&session, &ui);
        assert!(lines[5].starts_with(" > Recettes"), "{}", lines[5]);

        press(&mut session, &mut ui, &[Key::Enter]);
        let (action, context) = session_actions(&session).last().unwrap();
        assert_eq!(action, "open");
        assert_eq!(context.row.as_deref(), Some("recettes"));
    }

    #[test]
    fn a_card_shows_its_fields_with_leaders_and_its_function_keys() {
        let (session, ui) = on_carnet();
        let (lines, cursor) = render(&session, &ui);

        assert_eq!(
            lines[0],
            " CARNET                        Carnet d'adresses                     Fiche 3/42"
        );
        assert_eq!(lines[3], " Nom . . . . . . :  Lovelace");
        assert_eq!(lines[4], " Prénom  . . . . :  Ada");
        assert_eq!(lines[23], " F3=Quitter  F8=Suiv.  Entrée=Enregistrer");
        // The button is not repeated: F8 already offers its action.
        assert!(!lines.iter().any(|line| line.contains("1. Suiv.")));
        // The cursor waits at the end of the first field.
        assert_eq!(cursor, Some((28, 3)));
    }

    #[test]
    fn typing_edits_the_focused_field() {
        let (mut session, mut ui) = on_carnet();
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

        assert_eq!(session.view().value("prenom"), "Alan");
        assert_eq!(session.view().value("nom"), "Lovelace");
        let (lines, cursor) = render(&session, &ui);
        assert_eq!(lines[4], " Prénom  . . . . :  Alan");
        assert_eq!(cursor, Some((24, 4)));
    }

    #[test]
    fn a_function_key_sends_the_card_and_what_was_typed() {
        let (mut session, mut ui) = on_carnet();
        press(&mut session, &mut ui, &[Key::Char('!'), Key::F(8)]);

        let (action, context) = session_actions(&session).last().unwrap().clone();
        assert_eq!(action, "next_card");
        assert_eq!(context.card.as_deref(), Some("3"));
        assert_eq!(context.dirty["nom"], "Lovelace!");
    }

    #[test]
    fn enter_in_a_field_runs_the_action_bound_to_enter() {
        let (mut session, mut ui) = on_carnet();
        press(&mut session, &mut ui, &[Key::Enter]);
        assert_eq!(session_actions(&session).last().unwrap().0, "save_card");
    }

    #[test]
    fn a_key_with_no_binding_says_so() {
        let (mut session, mut ui) = on_carnet();
        press(&mut session, &mut ui, &[Key::F(5)]);
        let (lines, _) = render(&session, &ui);
        assert_eq!(lines[22], " Cette touche n'est pas active sur cet écran.");
    }

    #[test]
    fn quitting_goes_through_the_server_like_any_action() {
        let (mut session, mut ui) = on_carnet();
        press(&mut session, &mut ui, &[Key::F(3)]);
        assert!(session.should_quit());
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
