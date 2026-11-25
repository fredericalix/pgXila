//! A server in memory, for the tests of the graphical client: a desktop, a
//! stack list, an address book with two cards, and the author screens.

use std::collections::BTreeMap;

use xila_core::api::{
    Api, ApiError, Card, Context, Effect, GridPage, GridRow, Hello, Image, Move, Notification,
    PaletteColour, Screen,
};
use xila_core::session::Session;

use crate::font::Fonts;
use crate::frame::{Frame, Rect};
use crate::palette::slot;
use crate::screen::{self, Ui};

#[derive(Default)]
pub struct Fake {
    /// The actions asked for, with what the client knew at that time.
    pub actions: Vec<(String, Context)>,
    pub pending: Vec<Notification>,
}

fn desktop() -> Screen {
    serde_json::from_value(serde_json::json!({
        "stack": { "name": "desktop", "title": "Main menu", "kind": "system" },
        "source": { "has_cards": false, "count": 1 },
        "widgets": [
            { "name": "banner", "kind": "text", "ord": 10,
              "props": { "template": "pgXila", "style": "title" } },
            { "name": "slogan", "kind": "text", "ord": 20,
              "props": { "template": "A fantasy OS inside PostgreSQL", "style": "subtitle" } },
            { "name": "instance", "kind": "text", "ord": 30, "label": "Instance",
              "props": { "template": "{instance}" } },
            { "name": "user", "kind": "text", "ord": 40, "label": "User",
              "props": { "template": "{user}" } },
            { "name": "stacks", "kind": "button", "ord": 50, "label": "My stacks",
              "action": "open_stacks",
              "layout": { "graphic": { "x": 0, "y": 0, "w": 0, "h": 0,
                                       "style": { "variant": "primary" } } } }
        ],
        "actions": [],
        "keys": [
            { "key": "F3", "label": "Quit", "action": "quit" },
            { "key": "F12", "label": "Back", "action": "back" }
        ]
    }))
    .unwrap()
}

fn stacks() -> Screen {
    serde_json::from_value(serde_json::json!({
        "stack": { "name": "stacks", "title": "My stacks", "kind": "system" },
        "source": { "has_cards": false, "count": 1 },
        "widgets": [
            { "name": "list", "kind": "grid", "ord": 10, "action": "open",
              "props": { "columns": [
                  { "column": "title", "label": "Stack" },
                  { "column": "cards", "label": "Cards", "align": "right" } ] } }
        ],
        "actions": [],
        "keys": [
            { "key": "F3", "label": "Quit", "action": "quit" },
            { "key": "F12", "label": "Back", "action": "back" }
        ]
    }))
    .unwrap()
}

fn addressbook(name: &str) -> Screen {
    serde_json::from_value(serde_json::json!({
        "stack": { "name": name, "title": "Address book", "kind": "user" },
        "source": { "has_cards": true, "count": 2 },
        "widgets": [
            { "name": "title", "kind": "text", "ord": 10,
              "props": { "template": "{last_name}, {first_name}", "style": "title" } },
            { "name": "photo", "kind": "image", "ord": 15, "column": "photo" },
            { "name": "last_name", "kind": "field", "ord": 20, "label": "Last name", "column": "last_name" },
            { "name": "first_name", "kind": "field", "ord": 30, "label": "First name", "column": "first_name" },
            { "name": "next", "kind": "button", "ord": 90, "label": "Next",
              "action": "next_card",
              "layout": { "graphic": { "x": 0, "y": 0, "w": 0, "h": 0,
                                       "style": { "arrow": "right" } } } }
        ],
        "actions": [],
        "keys": [
            { "key": "F3", "label": "Quit", "action": "quit" },
            { "key": "F6", "label": "New", "action": "new_card" },
            { "key": "F8", "label": "Next", "action": "next_card" },
            { "key": "F12", "label": "Back", "action": "back" },
            { "key": "Enter", "label": "Save", "action": "save_card" }
        ]
    }))
    .unwrap()
}

/// Author mode on one stack: a dialog that shows a card of a view.
fn author() -> Screen {
    serde_json::from_value(serde_json::json!({
        "stack": { "name": "author", "title": "Author mode", "kind": "system" },
        "source": { "has_cards": true, "count": 3 },
        "widgets": [
            { "name": "heading", "kind": "text", "ord": 10,
              "props": { "template": "{title}", "style": "title" } },
            { "name": "mode", "kind": "text", "ord": 20,
              "props": { "template": "Author mode", "style": "subtitle" } },
            { "name": "title", "kind": "field", "ord": 30, "label": "Title",
              "column": "title", "type": "text" },
            { "name": "card_title", "kind": "field", "ord": 40, "label": "Card title",
              "column": "card_title", "type": "text" },
            { "name": "fields", "kind": "grid", "ord": 50, "action": "edit_field",
              "props": { "columns": [
                  { "column": "label", "label": "Field" },
                  { "column": "name", "label": "Name" },
                  { "column": "type_label", "label": "Type" },
                  { "column": "required_label", "label": "Required" } ] } },
            { "name": "new_field", "kind": "button", "ord": 60, "label": "New field",
              "action": "new_field",
              "layout": { "graphic": { "x": 0, "y": 0, "w": 0, "h": 0,
                                       "style": { "variant": "primary" } } } },
            { "name": "history", "kind": "button", "ord": 70, "label": "History",
              "action": "history" },
            { "name": "delete_stack", "kind": "button", "ord": 80, "label": "Delete stack",
              "action": "delete_stack" }
        ],
        "actions": [],
        "keys": [
            { "key": "F3", "label": "Quit", "action": "quit" },
            { "key": "F6", "label": "New field", "action": "new_field" },
            { "key": "F7", "label": "Up", "action": "up" },
            { "key": "F8", "label": "Down", "action": "down" },
            { "key": "F10", "label": "History", "action": "history" },
            { "key": "F11", "label": "Browse", "action": "browse" },
            { "key": "F12", "label": "Back", "action": "back" },
            { "key": "Enter", "label": "Save", "action": "save" }
        ]
    }))
    .unwrap()
}

/// One field of a stack: a text, a choice and a check box.
fn field() -> Screen {
    serde_json::from_value(serde_json::json!({
        "stack": { "name": "field", "title": "Field", "kind": "system" },
        "source": { "has_cards": true, "count": 3 },
        "widgets": [
            { "name": "heading", "kind": "text", "ord": 10,
              "props": { "template": "Field", "style": "title" } },
            { "name": "label", "kind": "field", "ord": 30, "label": "Label",
              "column": "label", "type": "text" },
            { "name": "name", "kind": "field", "ord": 40, "label": "Name",
              "column": "name", "type": "text" },
            { "name": "type", "kind": "field", "ord": 50, "label": "Type",
              "column": "type", "type": "text",
              "props": { "choices": [
                  { "value": "text", "label": "Text" },
                  { "value": "number", "label": "Number" },
                  { "value": "date", "label": "Date" },
                  { "value": "boolean", "label": "Yes or no" } ] } },
            { "name": "required", "kind": "field", "ord": 60, "label": "Required",
              "column": "required", "type": "boolean" },
            { "name": "save", "kind": "button", "ord": 70, "label": "Save", "action": "save",
              "layout": { "graphic": { "x": 0, "y": 0, "w": 0, "h": 0,
                                       "style": { "variant": "primary" } } } },
            { "name": "remove", "kind": "button", "ord": 80, "label": "Remove",
              "action": "remove" },
            { "name": "cancel", "kind": "button", "ord": 90, "label": "Cancel",
              "action": "back" }
        ],
        "actions": [],
        "keys": [
            { "key": "F3", "label": "Quit", "action": "quit" },
            { "key": "F12", "label": "Back", "action": "back" },
            { "key": "Enter", "label": "Save", "action": "save" }
        ]
    }))
    .unwrap()
}

/// The keys a stack built in author mode has: more than the bar shows at
/// its usual spacing.
pub fn keys_of_a_built_stack() -> Vec<(&'static str, &'static str)> {
    vec![
        ("F3", "Quit"),
        ("F4", "Delete"),
        ("F6", "New"),
        ("F7", "Prev."),
        ("F8", "Next"),
        ("F10", "History"),
        ("F11", "Author"),
        ("F12", "Back"),
        ("Enter", "Save"),
    ]
}

impl Api for Fake {
    fn hello(&mut self) -> Result<Hello, ApiError> {
        Ok(Hello {
            api: 2,
            version: "0".into(),
            instance: "normandie-01".into(),
            role: "ada".into(),
            author: false,
            home: "desktop".into(),
        })
    }

    fn palette(&mut self) -> Result<Vec<PaletteColour>, ApiError> {
        Ok(Vec::new())
    }

    /// A portrait whose first two pixels are red and green, to recognise it.
    fn image(&mut self, id: &str) -> Result<Option<Image>, ApiError> {
        if id != "portrait" {
            return Ok(None);
        }
        let mut pixels = vec![slot::AZURE; 68 * 80];
        pixels[0] = slot::RED;
        pixels[1] = slot::GREEN;
        Ok(Some(Image { width: 68, height: 80, pixels }))
    }

    fn open_stack(&mut self, stack: &str) -> Result<Screen, ApiError> {
        Ok(match stack {
            "desktop" => desktop(),
            "stacks" => stacks(),
            "author" => author(),
            "field" => field(),
            other => addressbook(other),
        })
    }

    fn card(&mut self, stack: &str, key: Option<&str>, _to: Move) -> Result<Card, ApiError> {
        let text = |value: &str| Some(value.to_owned());
        Ok(match stack {
            "desktop" => Card {
                position: 1,
                count: 1,
                texts: BTreeMap::from([
                    ("banner".to_owned(), "pgXila".to_owned()),
                    ("slogan".to_owned(), "A fantasy OS inside PostgreSQL".to_owned()),
                    ("instance".to_owned(), "normandie-01".to_owned()),
                    ("user".to_owned(), "ada".to_owned()),
                ]),
                ..Card::default()
            },
            "stacks" => Card { position: 1, count: 1, ..Card::default() },
            "author" => Card {
                key: text(key.unwrap_or("addressbook")),
                position: 1,
                count: 3,
                values: BTreeMap::from([
                    ("title".to_owned(), text("Address book")),
                    ("card_title".to_owned(), text("{last_name}, {first_name}")),
                ]),
                texts: BTreeMap::from([
                    ("heading".to_owned(), "Address book".to_owned()),
                    ("mode".to_owned(), "Author mode".to_owned()),
                ]),
                ..Card::default()
            },
            "field" => Card {
                key: text(key.unwrap_or("f1")),
                position: 1,
                count: 3,
                values: BTreeMap::from([
                    ("label".to_owned(), text("Last name")),
                    ("name".to_owned(), text("last_name")),
                    ("type".to_owned(), text("text")),
                    ("required".to_owned(), text("true")),
                ]),
                texts: BTreeMap::from([("heading".to_owned(), "Field".to_owned())]),
                ..Card::default()
            },
            _ if key == Some("2") => Card {
                key: text("2"),
                position: 2,
                count: 2,
                values: BTreeMap::from([
                    ("last_name".to_owned(), text("Turing")),
                    ("first_name".to_owned(), text("Alan")),
                    ("photo".to_owned(), None),
                ]),
                texts: BTreeMap::from([("title".to_owned(), "Turing, Alan".to_owned())]),
                ..Card::default()
            },
            _ => Card {
                key: text("1"),
                position: 1,
                count: 2,
                values: BTreeMap::from([
                    ("last_name".to_owned(), text("Lovelace")),
                    ("first_name".to_owned(), text("Ada")),
                    ("photo".to_owned(), text("portrait")),
                ]),
                texts: BTreeMap::from([("title".to_owned(), "Lovelace, Ada".to_owned())]),
                ..Card::default()
            },
        })
    }

    fn grid_rows(
        &mut self,
        stack: &str,
        _widget: &str,
        card: Option<&str>,
    ) -> Result<GridPage, ApiError> {
        if stack == "author" {
            let row = |key: &str, cells: [&str; 4]| GridRow {
                key: key.into(),
                cells: ["label", "name", "type_label", "required_label"]
                    .into_iter()
                    .zip(cells)
                    .map(|(column, cell)| (column.to_owned(), Some(cell.to_owned())))
                    .collect(),
            };
            // The fields belong to the stack on screen: none without it.
            let rows = if card.is_some() {
                vec![
                    row("f1", ["Last name", "last_name", "Text", "Yes"]),
                    row("f2", ["First name", "first_name", "Text", ""]),
                    row("f3", ["Birthday", "birthday", "Date", ""]),
                ]
            } else {
                Vec::new()
            };
            return Ok(GridPage { total: rows.len() as u64, rows });
        }
        let row = |key: &str, title: &str, cards: &str| GridRow {
            key: key.into(),
            cells: BTreeMap::from([
                ("title".to_owned(), Some(title.to_owned())),
                ("cards".to_owned(), Some(cards.to_owned())),
            ]),
        };
        Ok(GridPage {
            total: 2,
            rows: vec![row("addressbook", "Address book", "2"), row("recipes", "Recipes", "2")],
        })
    }

    fn act(
        &mut self,
        stack: &str,
        action: &str,
        context: &Context,
    ) -> Result<Vec<Effect>, ApiError> {
        self.actions.push((action.to_owned(), context.clone()));
        let show = |stack: &str, card: Option<&str>| Effect::Show {
            stack: stack.to_owned(),
            card: card.map(str::to_owned),
        };
        Ok(match action {
            "open_stacks" => vec![show("stacks", None)],
            "open" => vec![show(context.row.as_deref().unwrap_or("addressbook"), None)],
            "next_card" => vec![show(stack, Some("2"))],
            "new_card" => vec![Effect::New { values: BTreeMap::new() }],
            "save_card" => vec![Effect::Message { text: "Card saved.".into() }],
            "author" => vec![show("author", Some("addressbook"))],
            "edit_field" => vec![show("field", context.row.as_deref())],
            "new_field" => vec![
                show("field", None),
                Effect::New {
                    values: BTreeMap::from([
                        ("stack".to_owned(), "addressbook".to_owned()),
                        ("type".to_owned(), "text".to_owned()),
                    ]),
                },
            ],
            "delete_stack" if !context.confirmed => vec![Effect::Confirm {
                text: "Delete the stack “Address book”? Its cards are kept, and the stack \
                       can be restored."
                    .into(),
                label: "Delete".into(),
            }],
            "delete_stack" => {
                vec![Effect::Back, Effect::Message { text: "Stack “Address book” deleted.".into() }]
            }
            "save" => vec![Effect::Back, Effect::Message { text: "Field saved.".into() }],
            "back" => vec![Effect::Back],
            "quit" => vec![Effect::Quit],
            other => {
                return Err(ApiError::User {
                    code: "XL002".into(),
                    message: format!("The action “{other}” does not exist."),
                })
            }
        })
    }

    fn notifications(&mut self) -> Vec<Notification> {
        std::mem::take(&mut self.pending)
    }
}

/// A session on the first card of the address book, reached the way a user
/// reaches it: desktop, stack list, first row.
pub fn addressbook_session() -> (Session<Fake>, Ui) {
    let fonts = Fonts::builtin();
    let mut session = Session::start(Fake::default()).unwrap();
    let mut ui = Ui::default();
    session.run("open_stacks");
    session.activate("list");
    assert_eq!(session.view().stack(), "addressbook");
    screen::settle(&mut session, &mut ui, &fonts);
    (session, ui)
}

/// How many pixels of a rectangle have a given colour.
pub fn count_slot(frame: &Frame, area: Rect, slot: u8) -> usize {
    (area.y..area.bottom())
        .flat_map(|y| (area.x..area.right()).map(move |x| (x, y)))
        .filter(|(x, y)| frame.pixel(*x, *y) == Some(slot))
        .count()
}

pub fn has_slot(frame: &Frame, area: Rect, slot: u8) -> bool {
    count_slot(frame, area, slot) > 0
}

/// Writes a frame as a PPM picture in the directory named by the
/// `XILA_SNAPSHOT_DIR` environment variable, to look at what the tests draw.
pub fn snapshot(frame: &Frame, name: &str) {
    let Ok(directory) = std::env::var("XILA_SNAPSHOT_DIR") else { return };
    let palette = crate::palette::Palette::boot();
    let mut data =
        format!("P6\n{} {}\n255\n", crate::frame::WIDTH, crate::frame::HEIGHT).into_bytes();
    for slot in frame.pixels() {
        let rgb = palette.rgb(*slot);
        data.extend([(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8]);
    }
    std::fs::write(std::path::Path::new(&directory).join(format!("{name}.ppm")), data).unwrap();
}
