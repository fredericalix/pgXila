//! Default placement of widgets on the text screen.
//!
//! A screen definition says what a screen contains, not where things go. A
//! widget may carry an explicit placement for the text renderer; every other
//! widget is placed here, from the logical model alone, in the manner of an
//! AS/400 screen:
//!
//! ```text
//!  Last name . . . :  Lovelace
//!  First name  . . :  Ada
//! ```
//!
//! This guarantees that any stack is usable in a terminal, even one that was
//! only ever designed for the graphical client.
//!
//! The text screen is 80 columns by 24 rows. The client keeps the top rows for
//! the header and the bottom rows for messages and function keys; widgets are
//! placed in between.

use std::collections::BTreeSet;

use crate::api::{Screen, Widget, WidgetKind};

pub const COLUMNS: u16 = 80;
pub const ROWS: u16 = 24;
/// First and last rows available to widgets.
pub const CONTENT_TOP: u16 = 3;
pub const CONTENT_BOTTOM: u16 = 20;
pub const MESSAGE_ROW: u16 = 22;
pub const KEYS_ROW: u16 = 23;

/// Name under which the "Option ===>" prompt takes the focus.
pub const PROMPT: &str = "#option";

const LEFT: u16 = 1;
/// Width of a field label with its dot leaders.
pub const LABEL_WIDTH: u16 = 17;
/// Column of a field value: the label, two spaces, then the value.
const VALUE_COLUMN: u16 = LEFT + LABEL_WIDTH + 2;
const DEFAULT_FIELD_WIDTH: u16 = 40;
const RENDERER: &str = "text";

/// Something to draw, with its position in character cells.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// The computed content of a `text` widget.
    Text { widget: String, x: u16, y: u16, w: u16 },
    /// A field: its label with dot leaders, then its value.
    Field { widget: String, column: String, label: String, label_x: u16, x: u16, y: u16, w: u16 },
    /// A numbered menu option, for a button that no function key triggers.
    Option { number: usize, action: String, label: String, x: u16, y: u16 },
    /// Where the user types the number of an option.
    Prompt { x: u16, y: u16 },
    /// A list of rows.
    Grid { widget: String, x: u16, y: u16, w: u16, h: u16 },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextLayout {
    pub items: Vec<Item>,
    /// What can take the focus, in order: widget names and [`PROMPT`].
    pub focus_order: Vec<String>,
}

impl TextLayout {
    /// The action of the option with this number, if there is one.
    pub fn option(&self, wanted: usize) -> Option<&str> {
        self.items.iter().find_map(|item| match item {
            Item::Option { number, action, .. } if *number == wanted => Some(action.as_str()),
            _ => None,
        })
    }
}

/// A label followed by dot leaders and a colon: `Last name . . . :`.
pub fn leader(label: &str, width: usize) -> String {
    let mut text: String = label.chars().take(width.saturating_sub(2)).collect();
    let label_length = text.chars().count();
    // Leaders sit on even columns so that they line up from row to row, and
    // never touch the label.
    for column in label_length..width.saturating_sub(2) {
        text.push(if column % 2 == 0 && column > label_length { '.' } else { ' ' });
    }
    text.push_str(" :");
    text
}

/// Places the widgets of a screen on the text grid.
pub fn layout(screen: &Screen) -> TextLayout {
    let bound_to_a_key: BTreeSet<&str> = screen.keys.iter().map(|k| k.action.as_str()).collect();

    let mut widgets: Vec<&Widget> = screen.widgets.iter().collect();
    widgets.sort_by(|a, b| (a.ord, &a.name).cmp(&(b.ord, &b.name)));

    let mut result = TextLayout::default();
    let mut row = CONTENT_TOP;
    let mut previous: Option<WidgetKind> = None;
    let mut options = 0;
    let mut last_option_row = None;
    let mut grids = Vec::new();

    for widget in widgets {
        let placement = widget.layout.get(RENDERER);
        if placement.is_some_and(|p| p.hidden()) {
            continue;
        }
        // An explicit placement wins. Otherwise the widget flows down the screen.
        let explicit = placement
            .filter(|p| p.w > 0)
            .map(|p| (clamp(p.x), clamp(p.y), clamp(p.w), clamp(p.h.max(1))));

        match widget.kind {
            WidgetKind::Text => {
                let (x, y, w, _) = explicit.unwrap_or_else(|| {
                    row = flowing_row(row, previous, widget.kind);
                    (LEFT, row, COLUMNS - 2 * LEFT, 1)
                });
                if explicit.is_none() {
                    row += 1;
                    previous = Some(widget.kind);
                }
                result.items.push(Item::Text { widget: widget.name.clone(), x, y, w });
            }
            WidgetKind::Field => {
                let Some(column) = widget.column.clone() else {
                    continue;
                };
                let wanted = widget
                    .props
                    .get("max_length")
                    .and_then(|v| v.as_u64())
                    .map_or(DEFAULT_FIELD_WIDTH, |n| n.min(u64::from(COLUMNS)) as u16);
                let (x, y, w, _) = explicit.unwrap_or_else(|| {
                    row = flowing_row(row, previous, widget.kind);
                    (VALUE_COLUMN, row, wanted.min(COLUMNS - VALUE_COLUMN - LEFT), 1)
                });
                if explicit.is_none() {
                    row += 1;
                    previous = Some(widget.kind);
                }
                result.items.push(Item::Field {
                    widget: widget.name.clone(),
                    column,
                    label: widget.label.clone().unwrap_or_else(|| widget.name.clone()),
                    label_x: x.saturating_sub(LABEL_WIDTH + 2),
                    x,
                    y,
                    w,
                });
                result.focus_order.push(widget.name.clone());
            }
            WidgetKind::Button => {
                let Some(action) = widget.action.clone() else {
                    continue;
                };
                // The function key bar already offers this action.
                if bound_to_a_key.contains(action.as_str()) {
                    continue;
                }
                row = flowing_row(row, previous, widget.kind);
                options += 1;
                result.items.push(Item::Option {
                    number: options,
                    action,
                    label: widget.label.clone().unwrap_or_else(|| widget.name.clone()),
                    x: LEFT + 2,
                    y: row,
                });
                last_option_row = Some(row);
                row += 1;
                previous = Some(widget.kind);
            }
            WidgetKind::Grid => {
                let (x, y, w, h) = explicit.unwrap_or_else(|| {
                    row = flowing_row(row, previous, widget.kind);
                    let height = (CONTENT_BOTTOM + 1).saturating_sub(row).max(2);
                    (LEFT, row, COLUMNS - 2 * LEFT, height)
                });
                if explicit.is_none() {
                    row += h;
                    previous = Some(widget.kind);
                }
                result.items.push(Item::Grid { widget: widget.name.clone(), x, y, w, h });
                grids.push(widget.name.clone());
            }
            // Images, sliders and canvases have no text form yet.
            WidgetKind::Image | WidgetKind::Slider | WidgetKind::Canvas => {}
        }
    }

    if let Some(last) = last_option_row {
        result.items.push(Item::Prompt { x: LEFT, y: last + 2 });
        result.focus_order.push(PROMPT.to_owned());
    }
    result.focus_order.extend(grids);
    result
}

/// The row a flowing widget goes to: a blank row separates blocks of
/// different kinds.
fn flowing_row(row: u16, previous: Option<WidgetKind>, kind: WidgetKind) -> u16 {
    match previous {
        Some(before) if before != kind => row + 1,
        _ => row,
    }
}

fn clamp(value: i32) -> u16 {
    value.clamp(0, i32::from(COLUMNS)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(widgets: serde_json::Value, keys: serde_json::Value) -> Screen {
        serde_json::from_value(serde_json::json!({
            "stack": { "name": "test", "title": "Test", "kind": "user" },
            "source": { "has_cards": true, "count": 1 },
            "widgets": widgets,
            "actions": [],
            "keys": keys,
        }))
        .unwrap()
    }

    #[test]
    fn leaders_fill_the_label_column() {
        assert_eq!(leader("Last name", 17), "Last name . . . :");
        assert_eq!(leader("First name", 17), "First name  . . :");
        assert_eq!(leader("A label that is far too long", 17), "A label that is :");
    }

    #[test]
    fn fields_without_placement_flow_down_in_logical_order() {
        let layout = layout(&screen(
            serde_json::json!([
                { "name": "city", "kind": "field", "ord": 30, "label": "City", "column": "city" },
                { "name": "last_name", "kind": "field", "ord": 10, "label": "Last name", "column": "last_name" },
                { "name": "first_name", "kind": "field", "ord": 20, "label": "First name", "column": "first_name" }
            ]),
            serde_json::json!([]),
        ));

        let placed: Vec<_> = layout
            .items
            .iter()
            .map(|item| match item {
                Item::Field { widget, label_x, x, y, w, .. } => {
                    (widget.as_str(), *label_x, *x, *y, *w)
                }
                other => panic!("unexpected item: {other:?}"),
            })
            .collect();
        assert_eq!(
            placed,
            [("last_name", 1, 20, 3, 40), ("first_name", 1, 20, 4, 40), ("city", 1, 20, 5, 40)]
        );
        assert_eq!(layout.focus_order, ["last_name", "first_name", "city"]);
    }

    #[test]
    fn an_explicit_placement_wins_over_the_default_one() {
        let layout = layout(&screen(
            serde_json::json!([
                { "name": "last_name", "kind": "field", "ord": 10, "label": "Last name", "column": "last_name",
                  "layout": { "text": { "x": 50, "y": 10, "w": 12, "h": 1 } } },
                { "name": "city", "kind": "field", "ord": 20, "label": "City", "column": "city" }
            ]),
            serde_json::json!([]),
        ));
        assert!(matches!(layout.items[0], Item::Field { x: 50, y: 10, w: 12, .. }));
        // The widget placed by hand does not push the others down.
        assert!(matches!(layout.items[1], Item::Field { x: 20, y: 3, .. }));
    }

    #[test]
    fn a_widget_hidden_for_the_text_renderer_is_left_out() {
        let layout = layout(&screen(
            serde_json::json!([
                { "name": "title", "kind": "text", "ord": 10,
                  "layout": { "text": { "x": 0, "y": 0, "w": 0, "h": 0, "style": { "hidden": true } } } },
                { "name": "last_name", "kind": "field", "ord": 20, "label": "Last name", "column": "last_name" }
            ]),
            serde_json::json!([]),
        ));
        assert_eq!(layout.items.len(), 1);
        assert!(matches!(layout.items[0], Item::Field { y: 3, .. }));
    }

    #[test]
    fn buttons_become_a_numbered_menu() {
        let layout = layout(&screen(
            serde_json::json!([
                { "name": "piles", "kind": "button", "ord": 10, "label": "My stacks", "action": "open_stacks" },
                { "name": "docs", "kind": "button", "ord": 20, "label": "Documents", "action": "open_docs" }
            ]),
            serde_json::json!([]),
        ));
        assert_eq!(
            layout.items,
            [
                Item::Option {
                    number: 1,
                    action: "open_stacks".into(),
                    label: "My stacks".into(),
                    x: 3,
                    y: 3
                },
                Item::Option {
                    number: 2,
                    action: "open_docs".into(),
                    label: "Documents".into(),
                    x: 3,
                    y: 4
                },
                Item::Prompt { x: 1, y: 6 },
            ]
        );
        assert_eq!(layout.option(2), Some("open_docs"));
        assert_eq!(layout.option(3), None);
        assert_eq!(layout.focus_order, [PROMPT]);
    }

    #[test]
    fn a_button_whose_action_has_a_function_key_is_not_repeated() {
        let layout = layout(&screen(
            serde_json::json!([
                { "name": "last_name", "kind": "field", "ord": 10, "label": "Last name", "column": "last_name" },
                { "name": "next", "kind": "button", "ord": 20, "label": "Next", "action": "next_card" }
            ]),
            serde_json::json!([{ "key": "F8", "label": "Next", "action": "next_card" }]),
        ));
        assert_eq!(layout.items.len(), 1);
        assert_eq!(layout.focus_order, ["last_name"]);
    }

    #[test]
    fn a_blank_row_separates_blocks_and_a_grid_takes_the_rest() {
        let layout = layout(&screen(
            serde_json::json!([
                { "name": "intro", "kind": "text", "ord": 10 },
                { "name": "liste", "kind": "grid", "ord": 20, "action": "open" }
            ]),
            serde_json::json!([]),
        ));
        assert_eq!(
            layout.items,
            [
                Item::Text { widget: "intro".into(), x: 1, y: 3, w: 78 },
                Item::Grid { widget: "liste".into(), x: 1, y: 5, w: 78, h: 16 },
            ]
        );
        assert_eq!(layout.focus_order, ["liste"]);
    }

    #[test]
    fn widgets_with_no_text_form_are_skipped() {
        let layout = layout(&screen(
            serde_json::json!([
                { "name": "photo", "kind": "image", "ord": 10, "column": "photo" },
                { "name": "volume", "kind": "slider", "ord": 20 }
            ]),
            serde_json::json!([]),
        ));
        assert!(layout.items.is_empty());
    }
}
