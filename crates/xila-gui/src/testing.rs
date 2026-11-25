//! A server in memory, for the tests of the graphical client: a desktop, a
//! stack list and an address book with two cards.

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

impl Api for Fake {
    fn hello(&mut self) -> Result<Hello, ApiError> {
        Ok(Hello {
            api: 1,
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

    fn grid_rows(&mut self, _stack: &str, _widget: &str) -> Result<GridPage, ApiError> {
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
            "new_card" => vec![Effect::New],
            "save_card" => vec![Effect::Message { text: "Card saved.".into() }],
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
