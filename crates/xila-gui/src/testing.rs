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
    /// What the visual editor did to the address book: the rectangles it
    /// gave, the widgets it added and those it removed.
    pub placed: BTreeMap<String, [i32; 4]>,
    pub added: Vec<serde_json::Value>,
    pub removed: Vec<String>,
    /// The extra colours the address book was given, by slot.
    pub tints: BTreeMap<u8, String>,
    /// The pictures that were saved: what they are of, and the picture.
    pub drawings: Vec<(String, Image)>,
    /// True when the server refuses to save a drawing.
    pub refuses_drawings: bool,
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
              "props": { "view": "icons", "icon": "icon", "columns": [
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

impl Fake {
    /// The address book as the editor left it.
    fn edited(&self, name: &str) -> Screen {
        let mut screen = addressbook(name);
        for widget in &self.added {
            screen.widgets.push(serde_json::from_value(widget.clone()).unwrap());
        }
        screen.widgets.retain(|w| !self.removed.contains(&w.name));
        screen.colours = self
            .tints
            .iter()
            .map(|(slot, rgb)| xila_core::api::Colour { slot: *slot, rgb: rgb.clone(), name: None })
            .collect();
        for widget in &mut screen.widgets {
            if let Some([x, y, w, h]) = self.placed.get(&widget.name).copied() {
                let style =
                    widget.layout.get("graphic").map(|p| p.style.clone()).unwrap_or_default();
                let place = xila_core::api::Placement { x, y, w, h, style };
                widget.layout.insert("graphic".to_owned(), place);
            }
        }
        screen
    }

    /// The last action asked for, and what the tool said with it.
    pub fn last(&self) -> (&str, serde_json::Value) {
        let (name, context) = self.actions.last().expect("an action was asked for");
        (name.as_str(), context.input.clone().unwrap_or(serde_json::Value::Null))
    }

    /// How many times an action was asked for.
    pub fn count(&self, action: &str) -> usize {
        self.actions.iter().filter(|(name, _)| name == action).count()
    }
}

fn addressbook(name: &str) -> Screen {
    serde_json::from_value(serde_json::json!({
        "stack": { "name": name, "title": "Address book", "kind": "user", "may_edit": true },
        "source": { "has_cards": true, "count": 2, "managed": true },
        "widgets": [
            { "name": "title", "kind": "text", "ord": 10,
              "props": { "template": "{last_name}, {first_name}", "style": "title" } },
            { "name": "photo", "kind": "image", "ord": 15, "column": "photo" },
            { "name": "last_name", "kind": "field", "ord": 20, "label": "Last name",
              "column": "last_name", "type": "text", "required": true },
            { "name": "first_name", "kind": "field", "ord": 30, "label": "First name",
              "column": "first_name", "type": "text", "required": false },
            { "name": "next", "kind": "button", "ord": 90, "label": "Next",
              "action": "next_card",
              "layout": { "graphic": { "x": 0, "y": 0, "w": 0, "h": 0,
                                       "style": { "arrow": "right" } } } }
        ],
        "actions": [
            { "name": "next_card", "label": "Next" },
            { "name": "new_card", "label": "New" },
            { "name": "undo", "label": "Undo", "for": "editor" }
        ],
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

    /// A portrait whose first two pixels are red and green, to recognise it,
    /// and the icon of a stack: an orange square with a transparent corner.
    fn image(&mut self, id: &str) -> Result<Option<Image>, ApiError> {
        if id == "icon" {
            let mut pixels = vec![slot::AMBER; 28 * 26];
            pixels[0] = xila_core::api::TRANSPARENT;
            return Ok(Some(Image { width: 28, height: 26, pixels }));
        }
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
            other => self.edited(other),
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
        let row = |key: &str, title: &str, cards: &str, icon: Option<&str>| GridRow {
            key: key.into(),
            cells: BTreeMap::from([
                ("title".to_owned(), Some(title.to_owned())),
                ("cards".to_owned(), Some(cards.to_owned())),
                ("icon".to_owned(), icon.map(str::to_owned)),
            ]),
        };
        // One stack has an icon of its own, the other has none.
        Ok(GridPage {
            total: 2,
            rows: vec![
                row("addressbook", "Address book", "2", Some("icon")),
                row("recipes", "Recipes", "2", None),
            ],
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
            // The window gets the visual editor, the terminal the forms.
            "author" if context.renderer == "graphic" => {
                vec![Effect::Design { stack: "addressbook".into() }]
            }
            "author" | "author_form" => vec![show("author", Some("addressbook"))],
            "place_widgets" => {
                let input = context.input.clone().unwrap_or_default();
                for (name, place) in input["placements"].as_object().into_iter().flatten() {
                    let side = |key: &str| place[key].as_i64().unwrap() as i32;
                    self.placed.insert(name.clone(), [side("x"), side("y"), side("w"), side("h")]);
                }
                vec![Effect::Reload]
            }
            "add_widget" => {
                let input = context.input.clone().unwrap_or_default();
                let kind = input["kind"].as_str().unwrap().to_owned();
                let name = format!("{kind}{}", self.added.len() + 1);
                let mut widget = serde_json::json!({
                    "name": name, "kind": kind, "ord": 200 + self.added.len(),
                    "layer": input["layer"],
                });
                if kind == "text" {
                    widget["props"] = serde_json::json!({ "template": "Text" });
                } else {
                    widget["label"] = "Button".into();
                }
                self.added.push(widget);
                let place = &input["props"]["placement"];
                let side = |key: &str| place[key].as_i64().unwrap() as i32;
                self.placed.insert(name.clone(), [side("x"), side("y"), side("w"), side("h")]);
                vec![Effect::Reload, Effect::Focus { widget: name }]
            }
            "set_widget" => vec![Effect::Reload],
            "remove_widget" if !context.confirmed => vec![Effect::Confirm {
                text: "Take it off the screen?".into(),
                label: "Remove".into(),
            }],
            "remove_widget" => {
                let input = context.input.clone().unwrap_or_default();
                self.removed.push(input["widget"].as_str().unwrap().to_owned());
                vec![Effect::Reload, Effect::Message { text: "Removed.".into() }]
            }
            "reset_layout" => {
                self.placed.clear();
                vec![
                    Effect::Reload,
                    Effect::Message { text: "The layout is automatic again.".into() },
                ]
            }
            "undo" => {
                vec![Effect::Reload, Effect::Message { text: "Undone: “city” moved.".into() }]
            }
            "stack_history" => vec![Effect::Message { text: "History.".into() }],
            // Drawing: the picture of the card, the icon of the stack, or a
            // picture of the background.
            "draw" => {
                let input = context.input.clone().unwrap_or_default();
                let target = input["target"].as_str().unwrap_or_default().to_owned();
                let widget = input["widget"].as_str().map(str::to_owned);
                let (image, width, height) = match target.as_str() {
                    "icon" => (None, 28, 26),
                    "card" if context.card.as_deref() == Some("1") => {
                        (Some("portrait".to_owned()), 68, 80)
                    }
                    "card" => (None, 68, 80),
                    _ => (None, 48, 48),
                };
                vec![Effect::Draw { target, widget, image, width, height }]
            }
            "save_drawing" if self.refuses_drawings => {
                return Err(ApiError::User {
                    code: "XL003".into(),
                    message: "An icon is drawn with the 32 system colours only.".into(),
                })
            }
            "save_drawing" => {
                let input = context.input.clone().unwrap_or_default();
                let hex = input["pixels"].as_str().unwrap_or_default();
                let pixels = (0..hex.len() / 2)
                    .map(|at| u8::from_str_radix(&hex[2 * at..2 * at + 2], 16).unwrap())
                    .collect();
                let image = Image {
                    width: input["width"].as_u64().unwrap() as u16,
                    height: input["height"].as_u64().unwrap() as u16,
                    pixels,
                };
                self.drawings.push((input["target"].as_str().unwrap().to_owned(), image));
                vec![Effect::Reload, Effect::Message { text: "Picture saved.".into() }]
            }
            "set_colour" => {
                let input = context.input.clone().unwrap_or_default();
                let slot = match input["slot"].as_u64() {
                    Some(slot) => slot as u8,
                    None => (32..255).find(|slot| !self.tints.contains_key(slot)).unwrap(),
                };
                self.tints.insert(slot, input["rgb"].as_str().unwrap().to_owned());
                vec![Effect::Reload]
            }
            "remove_colour" => {
                let input = context.input.clone().unwrap_or_default();
                self.tints.remove(&(input["slot"].as_u64().unwrap() as u8));
                vec![Effect::Reload]
            }
            // The server answered with the screen as it now is.
            "refresh" => vec![Effect::Reload],
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

/// A session in the visual editor, on the address book: the client says it
/// draws in a window, and F11 does the rest.
pub fn editor_session() -> (Session<Fake>, Ui) {
    let fonts = Fonts::builtin();
    let (mut session, mut ui) = addressbook_session();
    session.be_graphic();
    session.run("author");
    assert!(session.view().designing);
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
