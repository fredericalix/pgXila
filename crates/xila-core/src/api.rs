//! The pgXila client API, as data: what the server answers and what the
//! clients send. See `docs/api.md`.
//!
//! [`Api`] is the set of calls a client makes. [`crate::server::Server`]
//! implements it over a real connection; tests implement it in memory.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Answer of `xila.hello()`: who and where we are.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Hello {
    pub api: u32,
    pub version: String,
    pub instance: String,
    pub role: String,
    pub author: bool,
    /// The stack a session starts on.
    pub home: String,
}

/// One of the 32 system colours, as returned by `xila.palette()`.
#[derive(Debug, Clone, PartialEq)]
pub struct PaletteColour {
    pub slot: u8,
    pub name: String,
    /// `#rrggbb`
    pub rgb: String,
}

impl PaletteColour {
    /// The red, green and blue components, if `rgb` is well formed.
    pub fn components(&self) -> Option<(u8, u8, u8)> {
        let hex = self.rgb.strip_prefix('#').filter(|hex| hex.len() == 6 && hex.is_ascii())?;
        let part = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
        Some((part(0)?, part(2)?, part(4)?))
    }
}

/// Answer of `xila.image()`: an indexed image, one byte per pixel, row by
/// row. Each byte is a palette slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u16,
    pub height: u16,
    pub pixels: Vec<u8>,
}

impl Image {
    /// The palette slot of a pixel, if it is inside the image.
    pub fn pixel(&self, x: u16, y: u16) -> Option<u8> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.pixels.get(usize::from(y) * usize::from(self.width) + usize::from(x)).copied()
    }
}

/// Answer of `xila.open_stack()`: the definition of a screen.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Screen {
    pub stack: StackInfo,
    pub source: Source,
    pub widgets: Vec<Widget>,
    pub actions: Vec<ActionInfo>,
    pub keys: Vec<KeyBinding>,
    #[serde(default)]
    pub colours: Vec<Colour>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct StackInfo {
    pub name: String,
    pub title: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Source {
    /// False for a static screen, which has a single card and no data.
    pub has_cards: bool,
    pub count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WidgetKind {
    Field,
    Text,
    Button,
    Image,
    Grid,
    Slider,
    Canvas,
}

/// How a field is edited, from the type of its column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    Text,
    Number,
    Date,
    Boolean,
}

/// A widget: what the screen contains, with an optional placement for each
/// renderer.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Widget {
    pub name: String,
    pub kind: WidgetKind,
    pub ord: i32,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    /// The column a field or an image is bound to.
    #[serde(default)]
    pub column: Option<String>,
    #[serde(default, rename = "type")]
    pub field_type: Option<FieldType>,
    /// The action a button or a grid row triggers.
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub props: serde_json::Map<String, serde_json::Value>,
    /// Placements, by renderer name (`text`, `graphic`).
    #[serde(default)]
    pub layout: BTreeMap<String, Placement>,
}

/// Where a widget is drawn by one renderer.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    #[serde(default)]
    pub style: serde_json::Map<String, serde_json::Value>,
}

impl Placement {
    /// True when the widget must not be drawn by this renderer.
    pub fn hidden(&self) -> bool {
        self.style.get("hidden").and_then(|v| v.as_bool()).unwrap_or(false)
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ActionInfo {
    pub name: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct KeyBinding {
    pub key: String,
    pub label: String,
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Colour {
    pub slot: u8,
    pub rgb: String,
}

/// Answer of `xila.card()`: one card. Values travel as text.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Card {
    /// None when the stack has no card, or for a static screen.
    pub key: Option<String>,
    pub position: u64,
    pub count: u64,
    #[serde(default)]
    pub values: BTreeMap<String, Option<String>>,
    /// Computed content of the `text` widgets, by widget name.
    #[serde(default)]
    pub texts: BTreeMap<String, String>,
    /// Widgets that belong to this card only.
    #[serde(default)]
    pub widgets: Vec<Widget>,
}

/// Which card to read, relative to a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    First,
    Last,
    At,
    Next,
    Previous,
}

impl Move {
    pub fn as_str(self) -> &'static str {
        match self {
            Move::First => "first",
            Move::Last => "last",
            Move::At => "at",
            Move::Next => "next",
            Move::Previous => "previous",
        }
    }
}

/// Answer of `xila.grid_rows()`.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct GridPage {
    pub total: u64,
    pub rows: Vec<GridRow>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GridRow {
    pub key: String,
    pub cells: BTreeMap<String, Option<String>>,
}

/// The state of the client, sent with every action.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct Context {
    /// The card on screen. None for a card that is not saved yet.
    pub card: Option<String>,
    /// What the user typed and has not saved, by column.
    pub dirty: BTreeMap<String, String>,
    /// The selected row of the grid, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row: Option<String>,
}

/// What the server asks the client to do after an action.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "do", rename_all = "lowercase")]
pub enum Effect {
    /// Display a stack, at a given card if there is one.
    Show {
        stack: String,
        #[serde(default)]
        card: Option<String>,
    },
    /// Return to the previous screen.
    Back,
    /// Read the current card again.
    Reload,
    /// Put the cursor in a widget.
    Focus { widget: String },
    /// Show a message to the user.
    Message { text: String },
    /// Display an empty card, to be saved as a new one.
    New,
    /// Leave the client.
    Quit,
}

/// An error from the server.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ApiError {
    /// An error meant for the user (SQLSTATE class `XL`). The message is in
    /// English and is shown as it is.
    #[error("{message}")]
    User { code: String, message: String },
    /// Anything else: a lost connection, a bug.
    #[error("{0}")]
    Other(String),
}

/// A notification pushed by the server through LISTEN/NOTIFY.
#[derive(Debug, Clone, PartialEq)]
pub enum Notification {
    /// A card was inserted, updated or deleted.
    Card { stack: Option<String>, key: Option<String> },
    /// A screen definition, or the palette when `stack` is None, changed.
    Screen { stack: Option<String> },
}

impl Notification {
    /// Decodes a notification from its channel and JSON payload.
    pub fn parse(channel: &str, payload: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(payload).ok()?;
        let text = |name: &str| value.get(name).and_then(|v| v.as_str()).map(str::to_owned);
        match channel {
            "xila_card" => Some(Notification::Card { stack: text("stack"), key: text("key") }),
            "xila_screen" => Some(Notification::Screen { stack: text("stack") }),
            _ => None,
        }
    }
}

/// The calls a client makes to the server.
pub trait Api {
    fn hello(&mut self) -> Result<Hello, ApiError>;
    fn palette(&mut self) -> Result<Vec<PaletteColour>, ApiError>;
    /// An image by its identifier, or None when there is no such image.
    fn image(&mut self, id: &str) -> Result<Option<Image>, ApiError>;
    fn open_stack(&mut self, stack: &str) -> Result<Screen, ApiError>;
    fn card(&mut self, stack: &str, key: Option<&str>, to: Move) -> Result<Card, ApiError>;
    fn grid_rows(&mut self, stack: &str, widget: &str) -> Result<GridPage, ApiError>;
    fn act(
        &mut self,
        stack: &str,
        action: &str,
        context: &Context,
    ) -> Result<Vec<Effect>, ApiError>;
    /// Notifications received since the last call.
    fn notifications(&mut self) -> Vec<Notification>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_screen_is_read_from_the_json_of_the_server() {
        let screen: Screen = serde_json::from_value(serde_json::json!({
            "stack": { "name": "addressbook", "title": "Address book", "kind": "user" },
            "source": { "has_cards": true, "count": 42 },
            "widgets": [
                { "name": "last_name", "kind": "field", "layer": "background", "ord": 20,
                  "label": "Last name", "column": "last_name", "type": "text",
                  "props": { "required": true },
                  "layout": { "text": { "x": 20, "y": 4, "w": 30, "h": 1, "style": {} } } },
                { "name": "next", "kind": "button", "layer": "background", "ord": 90,
                  "label": "Next", "action": "next_card", "props": {}, "layout": {} }
            ],
            "actions": [ { "name": "next_card", "label": "Next" } ],
            "keys": [ { "key": "F8", "label": "Next", "action": "next_card" } ],
            "colours": []
        }))
        .unwrap();

        assert_eq!(screen.stack.title, "Address book");
        assert_eq!(screen.widgets[0].kind, WidgetKind::Field);
        assert_eq!(screen.widgets[0].field_type, Some(FieldType::Text));
        assert_eq!(screen.widgets[0].layout["text"].x, 20);
        assert_eq!(screen.widgets[1].action.as_deref(), Some("next_card"));
        assert!(screen.widgets[1].layout.is_empty());
    }

    #[test]
    fn a_palette_colour_gives_its_components() {
        let colour = |rgb: &str| PaletteColour { slot: 24, name: "orange".into(), rgb: rgb.into() };
        assert_eq!(colour("#e8703a").components(), Some((0xe8, 0x70, 0x3a)));
        assert_eq!(colour("e8703a").components(), None);
        assert_eq!(colour("#e870").components(), None);
        assert_eq!(colour("#zzzzzz").components(), None);
    }

    #[test]
    fn an_image_gives_its_pixels_by_position() {
        let image = Image { width: 3, height: 2, pixels: vec![0, 1, 2, 10, 11, 12] };
        assert_eq!(image.pixel(0, 0), Some(0));
        assert_eq!(image.pixel(2, 1), Some(12));
        assert_eq!(image.pixel(3, 0), None);
        assert_eq!(image.pixel(0, 2), None);
    }

    #[test]
    fn effects_are_read_from_their_do_tag() {
        let effects: Vec<Effect> = serde_json::from_value(serde_json::json!([
            { "do": "show", "stack": "addressbook", "card": "42" },
            { "do": "show", "stack": "stacks" },
            { "do": "message", "text": "Card saved." },
            { "do": "new" },
            { "do": "quit" }
        ]))
        .unwrap();
        assert_eq!(
            effects,
            vec![
                Effect::Show { stack: "addressbook".into(), card: Some("42".into()) },
                Effect::Show { stack: "stacks".into(), card: None },
                Effect::Message { text: "Card saved.".into() },
                Effect::New,
                Effect::Quit,
            ]
        );
    }

    #[test]
    fn the_context_is_sent_as_json() {
        let context = Context {
            card: Some("42".into()),
            dirty: BTreeMap::from([("city".to_string(), "Paris".to_string())]),
            row: None,
        };
        assert_eq!(
            serde_json::to_value(&context).unwrap(),
            serde_json::json!({ "card": "42", "dirty": { "city": "Paris" } })
        );
    }

    #[test]
    fn notifications_are_decoded_by_channel() {
        assert_eq!(
            Notification::parse("xila_card", r#"{"stack":"addressbook","key":"42","op":"update"}"#),
            Some(Notification::Card { stack: Some("addressbook".into()), key: Some("42".into()) })
        );
        assert_eq!(
            Notification::parse("xila_screen", r#"{"stack":null}"#),
            Some(Notification::Screen { stack: None })
        );
        assert_eq!(Notification::parse("other", "{}"), None);
        assert_eq!(Notification::parse("xila_card", "not json"), None);
    }
}
