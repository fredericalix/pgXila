//! Where the widgets of a screen go in a window.
//!
//! The database says what a screen contains, not where things are drawn. This
//! module derives a placement from that logical model, in the manner of the
//! mockups:
//!
//! - a stack with cards is a sheet of paper with a title, an optional picture
//!   on the left, the fields in a column, and the buttons under the sheet;
//! - a static screen is a banner, lines of text, a list, and its buttons.
//!
//! A widget may also carry its own placement for the `graphic` renderer. With
//! a size, it is drawn exactly there. Without a size, only its style is used.
//!
//! All coordinates are relative to the content area of the window.

use xila_core::api::{FieldType, Placement, Screen, Widget, WidgetKind};

use crate::chrome::{button_width, Arrow, ButtonLook, BUTTON_HEIGHT, FIELD_HEIGHT};
use crate::font::Fonts;
use crate::frame::Rect;

/// Name of the renderer in `xila.layout`.
pub const RENDERER: &str = "graphic";

/// Width of the content of a window that shows cards, as in the mockups.
const CARD_WIDTH: i32 = 406;
/// Width of the content of a static screen or of a dialog, and of one whose
/// list has more than two columns.
const STATIC_WIDTH: i32 = 260;
const WIDE_STATIC_WIDTH: i32 = 400;
/// The tallest content a dialog can have while its window stays clear of the
/// strip where messages are shown.
const MAX_HEIGHT: i32 = 221;
/// Space between the frame of the window and what it contains.
const MARGIN: i32 = 7;
/// Distance between two fields, top to top.
const FIELD_PITCH: i32 = 20;
/// Distance between two lines of text of a static screen.
const LINE_PITCH: i32 = 15;
const BANNER_HEIGHT: i32 = 44;
/// Size of a picture when nothing says otherwise: a portrait.
const PICTURE: (i32, i32) = (70, 82);
const GRID_ROWS: i32 = 10;
const GRID_ROWS_MIN: i32 = 3;
pub const GRID_HEADER: i32 = 13;
pub const GRID_ROW: i32 = 12;
const BUTTON_GAP: i32 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    /// The big title of a card.
    Title,
    /// The banner of a static screen: a big title and a line under it.
    Banner {
        subtitle: Option<String>,
    },
    /// A line of text.
    Text,
    /// A line of text with its label in front, as in a summary.
    Info {
        label: String,
        value_x: i32,
    },
    Field {
        column: String,
        label: String,
        kind: FieldKind,
    },
    Picture,
    Grid,
    Button {
        label: String,
        look: ButtonLook,
    },
}

/// How a field is edited and drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    /// A line of text.
    Text,
    /// One of a few values, chosen by going through them.
    Choice,
    /// Yes or no: a check box.
    Check,
}

fn field_kind(widget: &Widget) -> FieldKind {
    if widget.props.contains_key("choices") {
        FieldKind::Choice
    } else if widget.field_type == Some(FieldType::Boolean) {
        FieldKind::Check
    } else {
        FieldKind::Text
    }
}

fn field_part(widget: &Widget) -> Part {
    Part::Field {
        column: widget.column.clone().unwrap_or_default(),
        label: label(widget),
        kind: field_kind(widget),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Name of the widget.
    pub widget: String,
    pub area: Rect,
    pub part: Part,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GraphicLayout {
    /// Size of the content area of the window.
    pub width: i32,
    pub height: i32,
    /// The sheet of paper of a card.
    pub paper: Option<Rect>,
    /// The header of the sheet: where the title and the position go.
    pub header: Option<Rect>,
    /// Thin lines under the rows: ruled paper, or dotted lines.
    pub rules: Vec<Rect>,
    pub items: Vec<Item>,
    /// The widgets that can take the focus, in order.
    pub focus_order: Vec<String>,
    /// True when every widget on screen has a rectangle of its own: the
    /// layout was made by hand, in the visual editor.
    pub free: bool,
    /// True when at least one widget of a card has a rectangle of its own.
    pub by_hand: bool,
}

impl GraphicLayout {
    pub fn item(&self, widget: &str) -> Option<&Item> {
        self.items.iter().find(|item| item.widget == widget)
    }

    /// Where each widget is: what the visual editor writes to the database
    /// when a layout stops being automatic.
    pub fn placements(&self) -> Vec<(String, Rect)> {
        self.items.iter().map(|item| (item.widget.clone(), item.area)).collect()
    }
}

fn placement(widget: &Widget) -> Option<&Placement> {
    widget.layout.get(RENDERER)
}

fn hidden(widget: &Widget) -> bool {
    placement(widget).is_some_and(Placement::hidden)
}

/// The rectangle a widget asks for, when it asks for one.
fn fixed(widget: &Widget) -> Option<Rect> {
    placement(widget).filter(|p| p.w > 0 && p.h > 0).map(|p| Rect::new(p.x, p.y, p.w, p.h))
}

fn style<'a>(widget: &'a Widget, name: &str) -> Option<&'a str> {
    placement(widget)?.style.get(name)?.as_str()
}

/// The logical style of a text widget: "title", "subtitle" or nothing.
fn text_style(widget: &Widget) -> Option<&str> {
    widget.props.get("style").and_then(|v| v.as_str())
}

fn label(widget: &Widget) -> String {
    widget.label.clone().unwrap_or_else(|| widget.name.clone())
}

fn look(widget: &Widget) -> ButtonLook {
    ButtonLook {
        primary: style(widget, "variant") == Some("primary"),
        arrow: match style(widget, "arrow") {
            Some("left") => Some(Arrow::Left),
            Some("right") => Some(Arrow::Right),
            _ => None,
        },
        ..ButtonLook::default()
    }
}

/// Places the widgets of a screen. `card_widgets` are those that belong to
/// the card on screen only.
pub fn layout(screen: &Screen, card_widgets: &[Widget], fonts: &Fonts) -> GraphicLayout {
    let mut widgets: Vec<&Widget> =
        screen.widgets.iter().chain(card_widgets).filter(|w| !hidden(w)).collect();
    widgets.sort_by_key(|w| w.ord);

    let (placed, flowing): (Vec<&Widget>, Vec<&Widget>) =
        widgets.into_iter().partition(|w| fixed(w).is_some());

    // The screens of the system are dialogs, even when they show a card.
    let cards = screen.source.has_cards && !screen.stack.is_system();
    let has_placed = !placed.is_empty();
    let mut layout = if cards && !placed.is_empty() {
        free_card_layout(&placed, &flowing, fonts)
    } else if cards {
        card_layout(&flowing, fonts)
    } else {
        let mut layout = static_layout(&flowing, fonts);
        for widget in placed {
            let area = fixed(widget).expect("partitioned on having a rectangle");
            if let Some(part) = part_of(widget, fonts) {
                layout.width = layout.width.max(area.right() + MARGIN);
                layout.height = layout.height.max(area.bottom() + MARGIN);
                layout.items.push(Item { widget: widget.name.clone(), area, part });
            }
        }
        layout
    };

    let mut focusable: Vec<&Item> = layout
        .items
        .iter()
        .filter(|item| matches!(item.part, Part::Field { .. } | Part::Grid | Part::Button { .. }))
        .collect();
    // Placed by hand, the widgets are visited as they are read: down the
    // window, and from left to right.
    if cards && has_placed {
        focusable.sort_by_key(|item| (item.area.y, item.area.x));
    }
    layout.focus_order = focusable.into_iter().map(|item| item.widget.clone()).collect();
    layout
}

/// What a widget with its own rectangle is drawn as.
fn part_of(widget: &Widget, fonts: &Fonts) -> Option<Part> {
    Some(match widget.kind {
        WidgetKind::Field => {
            widget.column.as_ref()?;
            field_part(widget)
        }
        WidgetKind::Text if text_style(widget) == Some("title") => Part::Title,
        WidgetKind::Text => match &widget.label {
            Some(label) => {
                Part::Info { label: label.clone(), value_x: fonts.regular.width(label) + 8 }
            }
            None => Part::Text,
        },
        WidgetKind::Button => Part::Button { label: label(widget), look: look(widget) },
        WidgetKind::Image => Part::Picture,
        WidgetKind::Grid => Part::Grid,
        // Not drawn yet.
        WidgetKind::Slider | WidgetKind::Canvas => return None,
    })
}

/// Lays buttons out in rows, left to right, from (x, y). Returns the bottom
/// of the last row.
fn button_rows(
    buttons: &[&Widget],
    fonts: &Fonts,
    x: i32,
    y: i32,
    right: i32,
    minimum: i32,
    items: &mut Vec<Item>,
) -> i32 {
    let mut pen = x;
    let mut row = y;
    for widget in buttons {
        let look = look(widget);
        let label = label(widget);
        let width = button_width(fonts, &label, look).max(minimum).min(right - x);
        if pen > x && pen + width > right {
            pen = x;
            row += BUTTON_HEIGHT + BUTTON_GAP;
        }
        items.push(Item {
            widget: widget.name.clone(),
            area: Rect::new(pen, row, width, BUTTON_HEIGHT),
            part: Part::Button { label, look },
        });
        pen += width + BUTTON_GAP;
    }
    row + BUTTON_HEIGHT
}

/// A card whose widgets were placed by hand. Each one is drawn in its own
/// rectangle; the sheet of paper, its header and its ruled lines follow them,
/// so that a layout that was automatic looks the same once its rectangles are
/// written down.
fn free_card_layout(placed: &[&Widget], flowing: &[&Widget], fonts: &Fonts) -> GraphicLayout {
    let mut layout =
        GraphicLayout { width: CARD_WIDTH, free: true, by_hand: true, ..GraphicLayout::default() };
    for widget in placed {
        let area = fixed(widget).expect("partitioned on having a rectangle");
        if let Some(part) = part_of(widget, fonts) {
            layout.items.push(Item { widget: widget.name.clone(), area, part });
        }
    }

    // A widget that has no rectangle yet, such as one added in SQL, goes
    // under the others: fields and texts one per row, buttons in a row.
    let is_button = |item: &Item| matches!(item.part, Part::Button { .. });
    let lowest = |items: &[Item]| items.iter().map(|item| item.area.bottom()).max();
    let left = MARGIN + 14;
    let late_fields: Vec<&&Widget> = flowing
        .iter()
        .filter(|w| matches!(w.kind, WidgetKind::Field | WidgetKind::Text))
        .filter(|w| w.kind != WidgetKind::Field || w.column.is_some())
        .collect();
    let label_width = late_fields
        .iter()
        .filter(|w| w.kind == WidgetKind::Field)
        .map(|w| fonts.regular.width(&label(w)))
        .max()
        .unwrap_or(0);
    let mut y = lowest(&layout.items).map_or(MARGIN + 37, |bottom| bottom + 6);
    for widget in late_fields {
        let x = if widget.kind == WidgetKind::Field { left + label_width + 5 } else { left };
        if let Some(part) = part_of(widget, fonts) {
            let area = Rect::new(x, y, CARD_WIDTH - MARGIN - 14 - x, FIELD_HEIGHT);
            layout.items.push(Item { widget: widget.name.clone(), area, part });
            layout.free = false;
            y += FIELD_PITCH;
        }
    }
    let late_buttons: Vec<&Widget> =
        flowing.iter().filter(|w| w.kind == WidgetKind::Button).copied().collect();
    if !late_buttons.is_empty() {
        let top = lowest(&layout.items).map_or(y, |bottom| bottom + 8);
        button_rows(&late_buttons, fonts, MARGIN, top, CARD_WIDTH - MARGIN, 0, &mut layout.items);
        layout.free = false;
    }

    layout.width = layout
        .items
        .iter()
        .map(|item| item.area.right() + MARGIN)
        .max()
        .unwrap_or(0)
        .max(CARD_WIDTH);
    let paper_w = layout.width - 2 * MARGIN;

    // The sheet goes down to under the lowest thing that is not a button.
    let sheet_bottom = layout
        .items
        .iter()
        .filter(|item| !is_button(item))
        .map(|item| {
            item.area.bottom()
                + match item.part {
                    Part::Title => 18,
                    Part::Field { .. } | Part::Text | Part::Info { .. } | Part::Grid => 11,
                    _ => 8,
                }
        })
        .max()
        .unwrap_or(MARGIN + 36);
    let paper = Rect::new(MARGIN, MARGIN, paper_w, sheet_bottom - MARGIN);
    layout.paper = Some(paper);

    // The header is where the title is; without a title, a strip at the top.
    layout.header = Some(
        layout
            .items
            .iter()
            .find(|item| item.part == Part::Title)
            .map_or(Rect::new(MARGIN + 1, MARGIN + 1, paper_w - 2, 17), |title| title.area),
    );
    layout.rules = layout
        .items
        .iter()
        .filter(|item| {
            matches!(item.part, Part::Field { .. } | Part::Text | Part::Info { .. } | Part::Grid)
        })
        .map(|item| Rect::new(MARGIN + 1, item.area.bottom() + 3, paper_w - 2, 1))
        .filter(|rule| rule.y < paper.bottom() - 1)
        .collect();

    let buttons_bottom =
        layout.items.iter().filter(|item| is_button(item)).map(|i| i.area.bottom());
    layout.height = buttons_bottom.max().unwrap_or(0).max(paper.bottom() + 2) + MARGIN;
    layout
}

/// A stack with cards: a sheet of paper, and the buttons under it.
fn card_layout(widgets: &[&Widget], fonts: &Fonts) -> GraphicLayout {
    let mut layout = GraphicLayout { width: CARD_WIDTH, ..GraphicLayout::default() };
    let paper_x = MARGIN;
    let paper_w = CARD_WIDTH - 2 * MARGIN;
    let paper_y = MARGIN;
    let inside_right = paper_x + paper_w - 14;

    // The header: the title on the left, the position in the stack on the
    // right, and a line under them.
    let title =
        widgets.iter().find(|w| w.kind == WidgetKind::Text && text_style(w) == Some("title"));
    let header =
        Rect::new(paper_x + 1, paper_y + 1, paper_w - 2, if title.is_some() { 26 } else { 17 });
    layout.header = Some(header);
    if let Some(title) = title {
        layout.items.push(Item { widget: title.name.clone(), area: header, part: Part::Title });
    }

    // The picture, on the left under the header.
    let body_top = header.bottom() + 10;
    let mut left = paper_x + 14;
    let mut bottom = body_top;
    if let Some(picture) = widgets.iter().find(|w| w.kind == WidgetKind::Image) {
        let area = Rect::new(paper_x + 12, body_top + 1, PICTURE.0, PICTURE.1);
        layout.items.push(Item { widget: picture.name.clone(), area, part: Part::Picture });
        left = area.right() + 12;
        bottom = area.bottom();
    }

    // The rows: fields with their labels in a column, and lines of text.
    let rows: Vec<&&Widget> = widgets
        .iter()
        .filter(|w| match w.kind {
            WidgetKind::Field => w.column.is_some(),
            WidgetKind::Text => text_style(w) != Some("title"),
            WidgetKind::Grid => true,
            _ => false,
        })
        .collect();
    let label_width = rows
        .iter()
        .filter(|w| w.kind == WidgetKind::Field)
        .map(|w| fonts.regular.width(&label(w)))
        .max()
        .unwrap_or(0);
    let field_x = left + label_width + 5;
    let mut y = body_top;
    for widget in rows {
        match widget.kind {
            WidgetKind::Field => {
                layout.items.push(Item {
                    widget: widget.name.clone(),
                    area: Rect::new(field_x, y, inside_right - field_x, FIELD_HEIGHT),
                    part: field_part(widget),
                });
                y += FIELD_PITCH;
            }
            WidgetKind::Grid => {
                let height = GRID_HEADER + 5 * GRID_ROW + 2;
                layout.items.push(Item {
                    widget: widget.name.clone(),
                    area: Rect::new(left, y, inside_right - left, height),
                    part: Part::Grid,
                });
                y += height + FIELD_PITCH - FIELD_HEIGHT;
            }
            _ => {
                layout.items.push(Item {
                    widget: widget.name.clone(),
                    area: Rect::new(left, y, inside_right - left, FIELD_HEIGHT),
                    part: match &widget.label {
                        Some(label) => Part::Info {
                            label: label.clone(),
                            value_x: label_width.max(fonts.regular.width(label)) + 5,
                        },
                        None => Part::Text,
                    },
                });
                y += FIELD_PITCH;
            }
        }
        // Ruled paper: a line under each row.
        layout.rules.push(Rect::new(paper_x + 1, y - 3, paper_w - 2, 1));
    }
    bottom = bottom.max(y - 3) + 7;

    let paper = Rect::new(paper_x, paper_y, paper_w, bottom + 1 - paper_y);
    layout.paper = Some(paper);

    let buttons: Vec<&Widget> =
        widgets.iter().filter(|w| w.kind == WidgetKind::Button).copied().collect();
    layout.height = if buttons.is_empty() {
        paper.bottom() + 2 + MARGIN
    } else {
        let top = paper.bottom() + 8;
        button_rows(&buttons, fonts, paper_x, top, paper_x + paper_w, 0, &mut layout.items) + MARGIN
    };
    layout
}

/// The number of columns of a list.
fn grid_columns(widget: &Widget) -> usize {
    widget.props.get("columns").and_then(|v| v.as_array()).map_or(0, Vec::len)
}

/// A static screen or a dialog: a banner, lines of text, fields, a list, and
/// buttons.
fn static_layout(widgets: &[&Widget], fonts: &Fonts) -> GraphicLayout {
    let wide = widgets.iter().any(|w| w.kind == WidgetKind::Grid && grid_columns(w) > 2);
    let width = if wide { WIDE_STATIC_WIDTH } else { STATIC_WIDTH };
    let mut layout = GraphicLayout { width, ..GraphicLayout::default() };
    let left = 14;
    let right = width - 14;
    let mut y = 0;

    let title =
        widgets.iter().find(|w| w.kind == WidgetKind::Text && text_style(w) == Some("title"));
    let subtitle =
        widgets.iter().find(|w| w.kind == WidgetKind::Text && text_style(w) == Some("subtitle"));
    if let Some(title) = title {
        layout.items.push(Item {
            widget: title.name.clone(),
            area: Rect::new(0, 0, width, BANNER_HEIGHT + 1),
            part: Part::Banner { subtitle: subtitle.map(|w| w.name.clone()) },
        });
        y = BANNER_HEIGHT + 1;
    }
    y += 9;

    // Lines of text. Those with a label line up as a summary.
    let lines: Vec<&&Widget> = widgets
        .iter()
        .filter(|w| w.kind == WidgetKind::Text)
        .filter(|w| !matches!(text_style(w), Some("title")))
        .filter(|w| title.is_none() || text_style(w) != Some("subtitle"))
        .collect();
    let label_width = lines
        .iter()
        .filter_map(|w| w.label.as_ref())
        .map(|label| fonts.regular.width(label))
        .max()
        .unwrap_or(0);
    let value_x = (label_width + 14).max(51);
    for (index, widget) in lines.iter().enumerate() {
        layout.items.push(Item {
            widget: widget.name.clone(),
            area: Rect::new(left, y, right - left, LINE_PITCH - 4),
            part: match &widget.label {
                Some(label) => Part::Info { label: label.clone(), value_x },
                None => Part::Text,
            },
        });
        if index + 1 < lines.len() {
            layout.rules.push(Rect::new(left - 1, y + 11, right - left + 2, 1));
        }
        y += LINE_PITCH;
    }
    if !lines.is_empty() {
        y += 4;
    }

    // The fields, in a column, their labels lined up on the right.
    let fields: Vec<&&Widget> =
        widgets.iter().filter(|w| w.kind == WidgetKind::Field && w.column.is_some()).collect();
    let field_x =
        left + fields.iter().map(|w| fonts.regular.width(&label(w))).max().unwrap_or(0) + 5;
    for widget in &fields {
        layout.items.push(Item {
            widget: widget.name.clone(),
            area: Rect::new(field_x, y, right - field_x, FIELD_HEIGHT),
            part: field_part(widget),
        });
        y += FIELD_PITCH;
    }
    if !fields.is_empty() {
        y += 3;
    }

    let buttons: Vec<&Widget> =
        widgets.iter().filter(|w| w.kind == WidgetKind::Button).copied().collect();
    // Buttons share the width when there are several, up to a point.
    let count = buttons.len().max(1) as i32;
    let minimum = ((right - left - (count - 1) * BUTTON_GAP) / count).clamp(62, 96);
    let buttons_height = if buttons.is_empty() {
        0
    } else {
        button_rows(&buttons, fonts, left, 0, right, minimum, &mut Vec::new()) + 14
    };

    // A list takes the room that is left, within reason.
    let grids: Vec<&&Widget> = widgets.iter().filter(|w| w.kind == WidgetKind::Grid).collect();
    let has_picture = widgets.iter().any(|w| w.kind == WidgetKind::Image);
    for widget in &grids {
        let fixed = GRID_HEADER + 2 + 9;
        let room = MAX_HEIGHT - y - buttons_height - if has_picture { PICTURE.1 + 9 } else { 0 };
        let rows = ((room / grids.len() as i32 - fixed) / GRID_ROW).clamp(GRID_ROWS_MIN, GRID_ROWS);
        let height = GRID_HEADER + rows * GRID_ROW + 2;
        layout.items.push(Item {
            widget: widget.name.clone(),
            area: Rect::new(MARGIN, y, width - 2 * MARGIN, height),
            part: Part::Grid,
        });
        y += height + 9;
    }

    if let Some(picture) = widgets.iter().find(|w| w.kind == WidgetKind::Image) {
        let area = Rect::new(left, y, PICTURE.0, PICTURE.1);
        layout.items.push(Item { widget: picture.name.clone(), area, part: Part::Picture });
        y = area.bottom() + 9;
    }

    layout.height = if buttons.is_empty() {
        y
    } else {
        button_rows(&buttons, fonts, left, y, right, minimum, &mut layout.items) + 14
    };
    layout
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fonts() -> Fonts {
        Fonts::builtin()
    }

    /// The same screen once the editor has written down where everything is.
    fn frozen(screen: &Screen, placed: &GraphicLayout) -> Screen {
        let mut screen = screen.clone();
        for (name, area) in placed.placements() {
            let widget = screen.widgets.iter_mut().find(|w| w.name == name).unwrap();
            let style = widget.layout.get(RENDERER).map(|p| p.style.clone()).unwrap_or_default();
            let place = Placement { x: area.x, y: area.y, w: area.w, h: area.h, style };
            widget.layout.insert(RENDERER.to_owned(), place);
        }
        screen
    }

    #[test]
    fn a_layout_written_down_looks_like_the_automatic_one() {
        for automatic_screen in [addressbook(), {
            // Without a picture, and with a line of text among the fields.
            let mut screen = addressbook();
            screen.widgets.retain(|w| w.name != "photo");
            screen.widgets.push(
                serde_json::from_value(serde_json::json!(
                    { "name": "note", "kind": "text", "ord": 25, "props": { "template": "A note" } }
                ))
                .unwrap(),
            );
            screen
        }] {
            let automatic = layout(&automatic_screen, &[], &fonts());
            assert!(!automatic.free);

            let by_hand = layout(&frozen(&automatic_screen, &automatic), &[], &fonts());
            assert!(by_hand.free);
            assert_eq!(by_hand.paper, automatic.paper);
            assert_eq!(by_hand.header, automatic.header);
            assert_eq!((by_hand.width, by_hand.height), (automatic.width, automatic.height));
            let sorted = |layout: &GraphicLayout| {
                let mut items = layout.items.clone();
                items.sort_by(|a, b| a.widget.cmp(&b.widget));
                let mut rules = layout.rules.clone();
                rules.sort_by_key(|rule| rule.y);
                (items, rules)
            };
            assert_eq!(sorted(&by_hand), sorted(&automatic));
        }
    }

    #[test]
    fn a_moved_widget_stays_where_it_was_put_and_the_sheet_follows() {
        let screen = addressbook();
        let automatic = layout(&screen, &[], &fonts());
        let mut by_hand = frozen(&screen, &automatic);
        let company = by_hand.widgets.iter_mut().find(|w| w.name == "company").unwrap();
        let place = company.layout.get_mut(RENDERER).unwrap();
        (place.x, place.y) = (200, 150);

        let moved = layout(&by_hand, &[], &fonts());
        assert_eq!(
            moved.item("company").unwrap().area,
            Rect::new(200, 150, place_width(&automatic), 14)
        );
        // The sheet grew to hold it, and the window with the sheet.
        assert_eq!(moved.paper.unwrap().bottom(), 150 + 14 + 11);
        assert!(moved.height > automatic.height);
        // The buttons did not move.
        assert_eq!(moved.item("new").unwrap().area, automatic.item("new").unwrap().area);
    }

    fn place_width(layout: &GraphicLayout) -> i32 {
        layout.item("company").unwrap().area.w
    }

    #[test]
    fn a_widget_without_a_place_goes_under_those_that_have_one() {
        let screen = addressbook();
        let automatic = layout(&screen, &[], &fonts());
        let mut by_hand = frozen(&screen, &automatic);
        // A field and a button added later, in SQL, without a rectangle.
        by_hand.widgets.extend(
            serde_json::from_value::<Vec<Widget>>(serde_json::json!([
                { "name": "city", "kind": "field", "ord": 40, "label": "City", "column": "city" },
                { "name": "extra", "kind": "button", "ord": 120, "label": "Extra", "action": "x" }
            ]))
            .unwrap(),
        );
        let mixed = layout(&by_hand, &[], &fonts());
        assert!(!mixed.free, "the editor will write the missing rectangles down");
        let city = mixed.item("city").unwrap().area;
        let lowest_before = automatic.items.iter().map(|item| item.area.bottom()).max().unwrap();
        assert!(city.y > lowest_before);
        assert!(mixed.item("extra").unwrap().area.y > city.bottom());
        assert!(mixed.height > automatic.height);
    }

    fn screen(has_cards: bool, widgets: serde_json::Value) -> Screen {
        serde_json::from_value(serde_json::json!({
            "stack": { "name": "s", "title": "S", "kind": "user" },
            "source": { "has_cards": has_cards, "count": 3 },
            "widgets": widgets, "actions": [], "keys": [],
        }))
        .unwrap()
    }

    fn addressbook() -> Screen {
        screen(
            true,
            serde_json::json!([
                { "name": "title", "kind": "text", "ord": 10,
                  "props": { "template": "{last_name}", "style": "title" } },
                { "name": "photo", "kind": "image", "ord": 15, "column": "photo" },
                { "name": "last_name", "kind": "field", "ord": 20, "label": "Last name", "column": "last_name" },
                { "name": "company", "kind": "field", "ord": 30, "label": "Company",
                  "column": "company" },
                { "name": "previous", "kind": "button", "ord": 90, "label": "Prev.",
                  "action": "previous_card",
                  "layout": { "graphic": { "x": 0, "y": 0, "w": 0, "h": 0,
                                           "style": { "arrow": "left" } } } },
                { "name": "new", "kind": "button", "ord": 110, "label": "New card",
                  "action": "new_card",
                  "layout": { "graphic": { "x": 0, "y": 0, "w": 0, "h": 0,
                                           "style": { "variant": "primary" } } } }
            ]),
        )
    }

    #[test]
    fn a_card_is_a_sheet_with_a_title_a_picture_and_a_column_of_fields() {
        let layout = layout(&addressbook(), &[], &fonts());
        let paper = layout.paper.unwrap();
        assert_eq!(layout.width, 406);
        assert_eq!((paper.x, paper.y, paper.w), (7, 7, 392));

        let title = layout.item("title").unwrap();
        assert_eq!(title.part, Part::Title);
        assert_eq!(Some(title.area), layout.header);

        let photo = layout.item("photo").unwrap();
        assert_eq!((photo.area.w, photo.area.h), (70, 82));
        assert!(photo.area.y > title.area.bottom());

        // Fields are right of the picture, aligned, 20 pixels apart.
        let last_name = layout.item("last_name").unwrap();
        let company = layout.item("company").unwrap();
        assert_eq!(last_name.area.x, company.area.x);
        assert_eq!(last_name.area.right(), company.area.right());
        assert_eq!(company.area.y - last_name.area.y, 20);
        assert_eq!(last_name.area.h, FIELD_HEIGHT);
        assert!(last_name.area.x > photo.area.right() + fonts().regular.width("Company"));
        assert!(last_name.area.right() < paper.right());

        // The sheet holds the picture; the buttons are under it.
        assert!(paper.bottom() > photo.area.bottom());
        let previous = layout.item("previous").unwrap();
        let new = layout.item("new").unwrap();
        assert!(previous.area.y >= paper.bottom() + 2);
        assert_eq!(previous.area.y, new.area.y);
        assert!(new.area.x > previous.area.right());
        assert_eq!(layout.height, previous.area.bottom() + 7);
        assert_eq!(layout.rules.len(), 2);
    }

    #[test]
    fn the_style_of_a_placement_without_size_changes_the_look_only() {
        let layout = layout(&addressbook(), &[], &fonts());
        let Part::Button { look, .. } = &layout.item("previous").unwrap().part else {
            panic!("not a button");
        };
        assert_eq!(look.arrow, Some(Arrow::Left));
        assert!(!look.primary);
        let Part::Button { look, .. } = &layout.item("new").unwrap().part else {
            panic!("not a button");
        };
        assert!(look.primary);
    }

    #[test]
    fn the_focus_goes_through_fields_then_buttons() {
        let layout = layout(&addressbook(), &[], &fonts());
        assert_eq!(layout.focus_order, ["last_name", "company", "previous", "new"]);
    }

    #[test]
    fn a_static_screen_is_a_banner_a_summary_and_buttons() {
        let desktop = screen(
            false,
            serde_json::json!([
                { "name": "banner", "kind": "text", "ord": 10,
                  "props": { "template": "pgXila", "style": "title" } },
                { "name": "slogan", "kind": "text", "ord": 20,
                  "props": { "template": "A fantasy OS", "style": "subtitle" } },
                { "name": "instance", "kind": "text", "ord": 30, "label": "Instance",
                  "props": { "template": "{instance}" } },
                { "name": "user", "kind": "text", "ord": 40, "label": "User",
                  "props": { "template": "{user}" } },
                { "name": "stacks", "kind": "button", "ord": 50, "label": "My stacks",
                  "action": "open_stacks" }
            ]),
        );
        let layout = layout(&desktop, &[], &fonts());
        assert_eq!(layout.width, 260);
        assert_eq!(layout.paper, None);

        let banner = layout.item("banner").unwrap();
        assert_eq!(banner.part, Part::Banner { subtitle: Some("slogan".into()) });
        assert_eq!(banner.area, Rect::new(0, 0, 260, 45));
        assert!(layout.item("slogan").is_none(), "the subtitle is part of the banner");

        let instance = layout.item("instance").unwrap();
        let user = layout.item("user").unwrap();
        assert_eq!(user.area.y - instance.area.y, 15);
        let (Part::Info { value_x: first, .. }, Part::Info { value_x: second, .. }) =
            (&instance.part, &user.part)
        else {
            panic!("not summary lines");
        };
        assert_eq!(first, second, "the values line up");
        assert_eq!(layout.rules.len(), 1, "a dotted line between the two, none after");

        let button = layout.item("stacks").unwrap();
        assert!(button.area.y > user.area.bottom());
        assert_eq!(button.area.w, 96);
        assert_eq!(layout.height, button.area.bottom() + 14);
        assert_eq!(layout.focus_order, ["stacks"]);
    }

    #[test]
    fn a_list_fills_the_width_of_a_static_screen() {
        let stacks = screen(
            false,
            serde_json::json!([{ "name": "list", "kind": "grid", "ord": 10, "action": "open" }]),
        );
        let layout = layout(&stacks, &[], &fonts());
        let grid = layout.item("list").unwrap();
        assert_eq!((grid.area.x, grid.area.w), (7, 246));
        assert_eq!(grid.area.h, GRID_HEADER + 10 * GRID_ROW + 2);
        assert_eq!(layout.height, grid.area.bottom() + 9);
        assert_eq!(layout.focus_order, ["list"]);
    }

    #[test]
    fn a_widget_with_its_own_rectangle_is_drawn_exactly_there() {
        let screen = screen(
            true,
            serde_json::json!([
                { "name": "last_name", "kind": "field", "ord": 20, "label": "Last name", "column": "last_name" },
                { "name": "note", "kind": "field", "ord": 30, "label": "Note", "column": "note",
                  "layout": { "graphic": { "x": 300, "y": 100, "w": 120, "h": 14, "style": {} } } }
            ]),
        );
        let layout = layout(&screen, &[], &fonts());
        assert_eq!(layout.item("note").unwrap().area, Rect::new(300, 100, 120, 14));
        // The window grows to hold it. The field that has no rectangle goes
        // under it, and the sheet holds both.
        assert_eq!(layout.width, 427);
        let last_name = layout.item("last_name").unwrap().area;
        assert!(last_name.y > 114, "{last_name:?}");
        assert_eq!(layout.paper.unwrap().bottom(), last_name.bottom() + 11);
        assert_eq!(layout.rules.len(), 2);
        assert!(!layout.free);
        // The focus goes down the window.
        assert_eq!(layout.focus_order, ["note", "last_name"]);
    }

    #[test]
    fn hidden_widgets_and_widgets_not_drawn_yet_take_no_room() {
        let screen = screen(
            true,
            serde_json::json!([
                { "name": "last_name", "kind": "field", "ord": 20, "label": "Last name", "column": "last_name" },
                { "name": "secret", "kind": "field", "ord": 30, "label": "Secret", "column": "s",
                  "layout": { "graphic": { "x": 0, "y": 0, "w": 0, "h": 0,
                                           "style": { "hidden": true } } } },
                { "name": "volume", "kind": "slider", "ord": 40 }
            ]),
        );
        let layout = layout(&screen, &[], &fonts());
        assert_eq!(layout.items.len(), 1);
        assert_eq!(layout.focus_order, ["last_name"]);
    }

    #[test]
    fn the_widgets_of_a_card_join_those_of_the_background_in_order() {
        let screen = screen(
            true,
            serde_json::json!([
                { "name": "last_name", "kind": "field", "ord": 20, "label": "Last name", "column": "last_name" },
                { "name": "city", "kind": "field", "ord": 40, "label": "City", "column": "city" }
            ]),
        );
        let extra: Vec<Widget> = serde_json::from_value(serde_json::json!([
            { "name": "note", "kind": "field", "ord": 30, "label": "Note", "column": "note" }
        ]))
        .unwrap();
        let layout = layout(&screen, &extra, &fonts());
        assert_eq!(layout.focus_order, ["last_name", "note", "city"]);
    }

    #[test]
    fn buttons_that_do_not_fit_go_to_the_next_row() {
        let buttons: Vec<serde_json::Value> = (0..8)
            .map(|n| {
                serde_json::json!({ "name": format!("b{n}"), "kind": "button", "ord": n,
                                    "label": "A rather long button" })
            })
            .collect();
        let layout = layout(&screen(true, serde_json::json!(buttons)), &[], &fonts());
        let first = layout.item("b0").unwrap().area;
        let last = layout.item("b7").unwrap().area;
        assert!(last.y > first.y);
        assert!(layout.items.iter().all(|item| item.area.right() <= 406 - 7));
        assert_eq!(layout.height, last.bottom() + 7);
    }
}
