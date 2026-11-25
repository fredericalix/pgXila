//! The visual editor: the background of a stack, with its widgets as things
//! to select, move and resize.
//!
//! The editor is a tool of this client, but it decides nothing about a stack:
//! every change is an action sent to the server, which answers with the screen
//! as it now is. What the editor keeps to itself is what has no meaning for
//! anyone else: which widget is selected, where the pointer is dragging it,
//! which tool is in hand.

use serde_json::json;
use xila_core::api::{Api, Widget};
use xila_core::input::{edit_line, Key};
use xila_core::session::{Session, View};

use crate::chrome::{
    self, close_box_of, content_of, draw_button, draw_check, draw_choice, draw_field, draw_paper,
    draw_window, window_around, ButtonLook, WindowStyle, BUTTON_HEIGHT, FIELD_HEIGHT,
};
use crate::font::Fonts;
use crate::frame::{Frame, Rect, Sprite, WIDTH};
use crate::layout::{self, FieldKind, GraphicLayout, Item, Part};
use crate::palette::slot;

/// The keys of the editor. They belong to the tool, not to the stack.
pub const KEYS: [(&str, &str); 8] = [
    ("F2", "Undo"),
    ("F3", "Quit"),
    ("F4", "Stack"),
    ("F6", "Widget"),
    ("F7", "Properties"),
    ("F10", "History"),
    ("F11", "Browse"),
    ("F12", "Back"),
];

/// Moving and resizing snap to a grid of this many pixels.
const GRID: i32 = 4;
/// The grid is shown as a dot every this many pixels.
const GRID_DOTS: i32 = 8;
/// How close an edge must come to another widget to be aligned with it.
const SNAP: i32 = 3;
const HANDLE: i32 = 5;
const MIN_SIZE: i32 = 8;
/// The tallest content of a window that fits on the desktop.
const MAX_HEIGHT: i32 = 243;
/// How many ticks of the clock a move made with the arrows waits for the
/// next one before it is saved: several presses are one change.
const NUDGE_PATIENCE: u8 = 3;

const PALETTE: Rect = Rect::new(2, 22, 30, 12 + 8 * CELL + 9);
const CELL: i32 = 21;
const PROPERTIES_WIDTH: i32 = 134;
const ROW: i32 = 18;
const MENU_ROW: i32 = 16;

/// What a click on the background makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    /// Nothing: it selects and moves.
    #[default]
    Pointer,
    Field,
    Text,
    Button,
}

/// Where new widgets go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layer {
    /// On every card.
    #[default]
    Background,
    /// On the card on screen only.
    Card,
}

/// A corner or a side of the selection, to resize it by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handle {
    /// -1 for the left edge, 1 for the right one, 0 for neither.
    pub h: i32,
    /// -1 for the top edge, 1 for the bottom one, 0 for neither.
    pub v: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Grip {
    Move,
    Resize(Handle),
    /// The Properties window itself, by its title bar.
    Properties,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Drag {
    widget: String,
    grip: Grip,
    /// Where the pointer was pressed.
    from: (i32, i32),
    origin: Rect,
    current: Rect,
}

/// A move made with the arrow keys and not saved yet.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Nudge {
    widget: String,
    rect: Rect,
    idle: u8,
}

/// One line of the Properties window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prop {
    Label,
    Text,
    Action,
    Required,
    Main,
    OnBackground,
    X,
    Y,
    W,
    H,
    /// The button that makes the layout automatic again.
    Reset,
}

impl Prop {
    /// True for a property that is typed, as opposed to ticked or chosen.
    fn is_typed(self) -> bool {
        matches!(self, Prop::Label | Prop::Text | Prop::X | Prop::Y | Prop::W | Prop::H)
    }
}

/// A property being typed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Typing {
    prop: Prop,
    text: String,
    cursor: usize,
}

/// What the editor remembers between two events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Designer {
    /// The stack this state is about: another stack starts afresh.
    stack: String,
    pub tool: Tool,
    pub layer: Layer,
    pub grid: bool,
    pub guides: bool,
    pub selected: Option<String>,
    drag: Option<Drag>,
    nudge: Option<Nudge>,
    /// Where the user put the Properties window, once it was moved.
    properties_at: Option<(i32, i32)>,
    pub properties_hidden: bool,
    typing: Option<Typing>,
    /// The line of the "New widget" menu that is chosen, while it is open.
    menu: Option<usize>,
    /// The lines that show what the dragged widget is aligned with.
    guide_lines: Vec<Rect>,
}

impl Default for Designer {
    fn default() -> Self {
        Designer {
            stack: String::new(),
            tool: Tool::Pointer,
            layer: Layer::Background,
            grid: true,
            guides: true,
            selected: None,
            drag: None,
            nudge: None,
            properties_at: None,
            properties_hidden: false,
            typing: None,
            menu: None,
            guide_lines: Vec::new(),
        }
    }
}

/// Where everything is, for the stack being edited.
struct Editor {
    layout: GraphicLayout,
    window: Rect,
    content: Rect,
}

impl Editor {
    fn of(view: &View, fonts: &Fonts) -> Self {
        let layout = layout::layout(&view.screen, &view.card.widgets, fonts);
        let window = window_around(layout.width, layout.height);
        Editor { content: content_of(window), window, layout }
    }

    fn on_screen(&self, area: Rect) -> Rect {
        area.moved(self.content.x, self.content.y)
    }

    /// The rectangles of the other widgets, to align with.
    fn others(&self, widget: &str) -> Vec<Rect> {
        self.layout.items.iter().filter(|i| i.widget != widget).map(|i| i.area).collect()
    }

    /// Keeps a rectangle inside the window and no smaller than it can be.
    fn kept_inside(&self, rect: Rect) -> Rect {
        let w = rect.w.clamp(MIN_SIZE, self.layout.width);
        let h = rect.h.clamp(MIN_SIZE, MAX_HEIGHT);
        Rect::new(rect.x.clamp(0, self.layout.width - w), rect.y.clamp(0, MAX_HEIGHT - h), w, h)
    }
}

impl Designer {
    /// Where a widget is right now: where it is being dragged or nudged to,
    /// or else where the layout has it.
    fn rect_of(&self, item: &Item) -> Rect {
        if let Some(drag) = &self.drag {
            if drag.widget == item.widget && drag.grip != Grip::Properties {
                return drag.current;
            }
        }
        match &self.nudge {
            Some(nudge) if nudge.widget == item.widget => nudge.rect,
            _ => item.area,
        }
    }

    /// The selected widget, if it is still on screen.
    fn selection<'a>(&self, editor: &'a Editor) -> Option<&'a Item> {
        self.selected.as_deref().and_then(|name| editor.layout.item(name))
    }
}

fn widget<'a>(view: &'a View, name: &str) -> Option<&'a Widget> {
    view.widgets().find(|w| w.name == name)
}

/// The template of a text widget, as its author wrote it.
fn template(widget: &Widget) -> &str {
    widget.props.get("template").and_then(|v| v.as_str()).unwrap_or("")
}

/// Makes the state agree with the stack on screen: another stack starts
/// afresh, a widget the server asks to show is selected, and a selection
/// that is gone is dropped.
fn sync<A: Api>(session: &mut Session<A>, designer: &mut Designer, editor: &Editor) {
    if designer.stack != session.view().stack() {
        *designer = Designer {
            stack: session.view().stack().to_owned(),
            grid: designer.grid,
            guides: designer.guides,
            properties_hidden: designer.properties_hidden,
            ..Designer::default()
        };
    }
    if let Some(wanted) = session.view().focus.clone() {
        if editor.layout.item(&wanted).is_some() {
            designer.selected = Some(wanted);
        }
        session.set_focus(None);
    }
    if designer.selection(editor).is_none() {
        designer.selected = None;
        designer.typing = None;
    }
}

/// Sends new places to the server. A layout that was automatic stops being so
/// here: the place of every widget is written down with those that moved.
fn place<A: Api>(session: &mut Session<A>, editor: &Editor, moved: &[(String, Rect)]) {
    let mut all = if editor.layout.free { Vec::new() } else { editor.layout.placements() };
    for (name, rect) in moved {
        match all.iter_mut().find(|(known, _)| known == name) {
            Some(entry) => entry.1 = *rect,
            None => all.push((name.clone(), *rect)),
        }
    }
    if all.is_empty() {
        return;
    }
    let placements: serde_json::Map<String, serde_json::Value> = all
        .into_iter()
        .map(|(name, r)| (name, json!({ "x": r.x, "y": r.y, "w": r.w, "h": r.h })))
        .collect();
    session.run_with("place_widgets", json!({ "placements": placements }));
}

/// Saves a move made with the arrows.
fn flush<A: Api>(session: &mut Session<A>, designer: &mut Designer, fonts: &Fonts) {
    if let Some(nudge) = designer.nudge.take() {
        let editor = Editor::of(session.view(), fonts);
        if editor.layout.item(&nudge.widget).is_some_and(|item| item.area != nudge.rect) {
            place(session, &editor, &[(nudge.widget, nudge.rect)]);
        }
    }
}

/// Time passes: a move made with the arrows is saved once the keys rest.
/// Returns true when something was saved.
pub fn tick<A: Api>(session: &mut Session<A>, designer: &mut Designer, fonts: &Fonts) -> bool {
    let Some(nudge) = &mut designer.nudge else { return false };
    nudge.idle += 1;
    if nudge.idle < NUDGE_PATIENCE {
        return false;
    }
    flush(session, designer, fonts);
    true
}

// ---------------------------------------------------------------------------
// Aligning
// ---------------------------------------------------------------------------

/// Brings one coordinate to the nearest of the targets within reach. Returns
/// how far to move, and the target reached.
fn pull(edges: &[i32], targets: &[i32]) -> Option<(i32, i32)> {
    edges
        .iter()
        .flat_map(|edge| targets.iter().map(move |target| (target - edge, *target)))
        .filter(|(distance, _)| distance.abs() <= SNAP)
        .min_by_key(|(distance, _)| distance.abs())
}

fn to_grid(value: i32) -> i32 {
    (value + GRID / 2).div_euclid(GRID) * GRID
}

/// Where a dragged rectangle settles: aligned with another widget when one of
/// its edges or its middle comes close, on the grid otherwise. Also returns
/// the guide lines to show, in the coordinates of the content.
fn settle_drag(
    designer: &Designer,
    editor: &Editor,
    drag: &Drag,
    wanted: Rect,
) -> (Rect, Vec<Rect>) {
    let others = editor.others(&drag.widget);
    let mut rect = wanted;
    let mut guides = Vec::new();
    let (moves_left, moves_right, moves_top, moves_bottom) = match drag.grip {
        Grip::Resize(handle) => (handle.h < 0, handle.h > 0, handle.v < 0, handle.v > 0),
        _ => (true, true, true, true),
    };
    let moving = drag.grip == Grip::Move;

    // Across: the left edges, the right edges and, for a move, the middles.
    let mut edges = Vec::new();
    if moves_left {
        edges.push(rect.x);
    }
    if moves_right {
        edges.push(rect.right());
    }
    let mut targets: Vec<i32> = others.iter().flat_map(|o| [o.x, o.right()]).collect();
    let mut pulled = if designer.guides { pull(&edges, &targets) } else { None };
    if moving && designer.guides && pulled.is_none() {
        targets = others.iter().map(|o| o.x + o.w / 2).collect();
        pulled = pull(&[rect.x + rect.w / 2], &targets);
    }
    let mut across = None;
    match pulled {
        Some((distance, target)) => {
            across = Some(target);
            if moving {
                rect.x += distance;
            } else if moves_left {
                rect.w -= distance;
                rect.x += distance;
            } else {
                rect.w += distance;
            }
        }
        None if designer.grid => {
            if moving {
                rect.x = to_grid(rect.x);
            } else if moves_left {
                let right = rect.right();
                rect.x = to_grid(rect.x);
                rect.w = right - rect.x;
            } else if moves_right {
                rect.w = to_grid(rect.right()) - rect.x;
            }
        }
        None => {}
    }

    // Down: the same with the top edges, the bottom edges and the middles.
    let mut edges = Vec::new();
    if moves_top {
        edges.push(rect.y);
    }
    if moves_bottom {
        edges.push(rect.bottom());
    }
    let mut targets: Vec<i32> = others.iter().flat_map(|o| [o.y, o.bottom()]).collect();
    let mut pulled = if designer.guides { pull(&edges, &targets) } else { None };
    if moving && designer.guides && pulled.is_none() {
        targets = others.iter().map(|o| o.y + o.h / 2).collect();
        pulled = pull(&[rect.y + rect.h / 2], &targets);
    }
    let mut down = None;
    match pulled {
        Some((distance, target)) => {
            down = Some(target);
            if moving {
                rect.y += distance;
            } else if moves_top {
                rect.h -= distance;
                rect.y += distance;
            } else {
                rect.h += distance;
            }
        }
        None if designer.grid => {
            if moving {
                rect.y = to_grid(rect.y);
            } else if moves_top {
                let bottom = rect.bottom();
                rect.y = to_grid(rect.y);
                rect.h = bottom - rect.y;
            } else if moves_bottom {
                rect.h = to_grid(rect.bottom()) - rect.y;
            }
        }
        None => {}
    }

    let rect = editor.kept_inside(rect);
    // A guide runs through the dragged widget and those it is aligned with.
    if let Some(x) = across {
        let touched = others.iter().filter(|o| [o.x, o.right(), o.x + o.w / 2].contains(&x));
        let span = touched.fold(rect, |span, other| span.union(other));
        guides.push(Rect::new(x, span.y - 3, 1, span.h + 6));
    }
    if let Some(y) = down {
        let touched = others.iter().filter(|o| [o.y, o.bottom(), o.y + o.h / 2].contains(&y));
        let span = touched.fold(rect, |span, other| span.union(other));
        guides.push(Rect::new(span.x - 3, y, span.w + 6, 1));
    }
    (rect, guides)
}

/// The rectangle a drag asks for, before it settles.
fn dragged(drag: &Drag, x: i32, y: i32) -> Rect {
    let (dx, dy) = (x - drag.from.0, y - drag.from.1);
    let origin = drag.origin;
    match drag.grip {
        Grip::Resize(handle) => {
            let mut rect = origin;
            if handle.h < 0 {
                let dx = dx.min(origin.w - MIN_SIZE);
                rect.x += dx;
                rect.w -= dx;
            } else if handle.h > 0 {
                rect.w = (origin.w + dx).max(MIN_SIZE);
            }
            if handle.v < 0 {
                let dy = dy.min(origin.h - MIN_SIZE);
                rect.y += dy;
                rect.h -= dy;
            } else if handle.v > 0 {
                rect.h = (origin.h + dy).max(MIN_SIZE);
            }
            rect
        }
        _ => origin.moved(dx, dy),
    }
}

/// The eight handles of a rectangle on screen.
pub fn handles(area: Rect) -> Vec<(Handle, Rect)> {
    let mut all = Vec::new();
    for v in [-1, 0, 1] {
        for h in [-1, 0, 1] {
            if (h, v) == (0, 0) {
                continue;
            }
            let x = area.x + (area.w - 1) * (h + 1) / 2 - HANDLE / 2;
            let y = area.y + (area.h - 1) * (v + 1) / 2 - HANDLE / 2;
            all.push((Handle { h, v }, Rect::new(x, y, HANDLE, HANDLE)));
        }
    }
    all
}

// ---------------------------------------------------------------------------
// The tool palette
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    Tool(Tool),
    Layer(Layer),
    Grid,
    Guides,
}

/// The cells of the tool palette and where they are on screen.
pub fn palette_cells() -> Vec<(Cell, Rect)> {
    let cells = [
        Cell::Tool(Tool::Pointer),
        Cell::Tool(Tool::Field),
        Cell::Tool(Tool::Text),
        Cell::Tool(Tool::Button),
        Cell::Layer(Layer::Background),
        Cell::Layer(Layer::Card),
        Cell::Grid,
        Cell::Guides,
    ];
    let mut y = PALETTE.y + 9;
    let mut placed = Vec::new();
    for (index, cell) in cells.into_iter().enumerate() {
        // A little room between the tools, the layers and the helpers.
        if index == 4 || index == 6 {
            y += 4;
        }
        placed.push((cell, Rect::new(PALETTE.x + 2, y, PALETTE.w - 4, CELL - 1)));
        y += CELL;
    }
    placed
}

fn icon(cell: Cell) -> Sprite {
    let ink = ('#', slot::INK);
    let white = ('o', slot::WHITE);
    match cell {
        Cell::Tool(Tool::Pointer) => Sprite::from_art(
            &[
                "#.......", "##......", "#o#.....", "#oo#....", "#ooo#...", "#oooo#..", "#ooooo#.",
                "#oo####.", "#o#.....", "##......",
            ],
            &[ink, white],
        ),
        Cell::Tool(Tool::Field) => Sprite::from_art(
            &[
                "################",
                "#oooooooooooooo#",
                "#o#oooooooooooo#",
                "#o#oooooooooooo#",
                "#o#oooooooooooo#",
                "#o#oooooooooooo#",
                "#oooooooooooooo#",
                "################",
            ],
            &[ink, white],
        ),
        // "Aa" is written with the font, not drawn.
        Cell::Tool(Tool::Text) => Sprite::from_art(&[], &[]),
        Cell::Tool(Tool::Button) => Sprite::from_art(
            &[
                ".##############.",
                "#pppppppppppppp#",
                "#pppppppppppppg#",
                "#ppp########ppg#",
                "#pppppppppppppg#",
                "#pggggggggggggg#",
                ".##############.",
            ],
            &[ink, ('p', slot::PEARL), ('g', slot::GREY)],
        ),
        // Every card: a pile of sheets. One card: the sheet on top.
        Cell::Layer(Layer::Background) => Sprite::from_art(
            &[
                "...###########",
                "...#ooooooooo#",
                ".###########o#",
                ".#ooooooooo#o#",
                "###########o##",
                "#ooooooooo#o#.",
                "#ooooooooo###.",
                "#ooooooooo#...",
                "###########...",
            ],
            &[ink, white],
        ),
        Cell::Layer(Layer::Card) => Sprite::from_art(
            &[
                "###########",
                "#ooooooooo#",
                "#o#######o#",
                "#ooooooooo#",
                "#o#####ooo#",
                "#ooooooooo#",
                "#ooooooooo#",
                "#ooooooooo#",
                "###########",
            ],
            &[ink, white],
        ),
        Cell::Grid => Sprite::from_art(
            &[
                "#..#..#..#",
                "..........",
                "..........",
                "#..#..#..#",
                "..........",
                "..........",
                "#..#..#..#",
            ],
            &[ink],
        ),
        Cell::Guides => Sprite::from_art(
            &[
                "...r......",
                ".###......",
                ".#r#......",
                ".###.####.",
                "...r.#oo#.",
                "...r.####.",
                "...r......",
            ],
            &[ink, white, ('r', slot::RED)],
        ),
    }
}

fn draw_palette(frame: &mut Frame, fonts: &Fonts, designer: &Designer) {
    frame.checker(Rect::new(PALETTE.right(), PALETTE.y + 3, 3, PALETTE.h), slot::INK);
    frame.checker(Rect::new(PALETTE.x + 3, PALETTE.bottom(), PALETTE.w - 3, 3), slot::INK);
    frame.fill(PALETTE, slot::CREAM);
    frame.outline(PALETTE, slot::INK);
    frame.fill(Rect::new(PALETTE.x + 1, PALETTE.y + 1, PALETTE.w - 2, 6), slot::YELLOW);
    frame.hline(PALETTE.x, PALETTE.y + 7, PALETTE.w, slot::INK);

    for (cell, area) in palette_cells() {
        let on = match cell {
            Cell::Tool(tool) => designer.tool == tool,
            Cell::Layer(layer) => designer.layer == layer,
            Cell::Grid => designer.grid,
            Cell::Guides => designer.guides,
        };
        frame.fill(area, if on { slot::YELLOW } else { slot::WHITE });
        frame.outline(area, slot::INK);
        if cell == Cell::Tool(Tool::Text) {
            let x = area.x + (area.w - fonts.bold.width("Aa")) / 2;
            fonts.bold.draw(frame, x, area.y + 14, "Aa", slot::INK);
        } else {
            let sprite = icon(cell);
            frame.sprite(
                &sprite,
                area.x + (area.w - sprite.width) / 2,
                area.y + (area.h - sprite.height) / 2,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The Properties window
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Control {
    /// A box to type in.
    Typed,
    /// A box to tick.
    Check(bool),
    /// A value among a few: a click goes to the next.
    Choice,
    Button,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    prop: Prop,
    label: &'static str,
    control: Control,
    /// What the property is now, as text.
    value: String,
    /// Where the control is on screen.
    area: Rect,
}

struct Properties {
    window: Rect,
    /// What the window is about, and a line about it.
    heading: String,
    detail: String,
    /// More lines, when nothing is selected.
    notes: Vec<String>,
    rows: Vec<Row>,
}

/// The actions a button of the stack may run: those of the stack, without
/// the ones that serve the author screens.
fn button_actions(view: &View) -> Vec<(String, String)> {
    view.screen
        .actions
        .iter()
        .filter(|action| action.kept_for.is_none())
        .map(|action| (action.name.clone(), action.label.clone()))
        .collect()
}

fn kind_of(part: &Part, widget: Option<&Widget>) -> String {
    match part {
        Part::Field { kind, .. } => match (kind, widget.and_then(|w| w.field_type)) {
            (FieldKind::Check, _) => "Field, yes or no".to_owned(),
            (_, Some(xila_core::api::FieldType::Number)) => "Field, number".to_owned(),
            (_, Some(xila_core::api::FieldType::Date)) => "Field, date".to_owned(),
            _ => "Field, text".to_owned(),
        },
        Part::Title => "Title of the card".to_owned(),
        Part::Text | Part::Info { .. } | Part::Banner { .. } => "Text".to_owned(),
        Part::Button { .. } => "Button".to_owned(),
        Part::Picture => "Picture".to_owned(),
        Part::Grid => "List".to_owned(),
    }
}

impl Properties {
    fn of(view: &View, designer: &Designer, editor: &Editor) -> Self {
        let mut rows: Vec<(Prop, &'static str, Control, String)> = Vec::new();
        let mut notes = Vec::new();
        let (heading, detail);

        match designer.selection(editor) {
            Some(item) => {
                let definition = widget(view, &item.widget);
                heading = item.widget.clone();
                let on_card = definition.is_some_and(Widget::on_card);
                detail =
                    kind_of(&item.part, definition) + if on_card { ", on this card" } else { "" };
                match &item.part {
                    Part::Field { label, .. } => {
                        rows.push((Prop::Label, "Label", Control::Typed, label.clone()));
                        if view.screen.source.managed {
                            let required = definition.and_then(|w| w.required).unwrap_or(false);
                            rows.push((
                                Prop::Required,
                                "Required",
                                Control::Check(required),
                                String::new(),
                            ));
                        }
                    }
                    Part::Button { label, look } => {
                        rows.push((Prop::Label, "Label", Control::Typed, label.clone()));
                        let action = definition.and_then(|w| w.action.clone()).unwrap_or_default();
                        let shown = button_actions(view)
                            .into_iter()
                            .find(|(name, _)| *name == action)
                            .map_or_else(|| "Nothing".to_owned(), |(_, label)| label);
                        rows.push((Prop::Action, "Does", Control::Choice, shown));
                        rows.push((
                            Prop::Main,
                            "Main button",
                            Control::Check(look.primary),
                            String::new(),
                        ));
                        rows.push((
                            Prop::OnBackground,
                            "On every card",
                            Control::Check(!on_card),
                            String::new(),
                        ));
                    }
                    Part::Title | Part::Text | Part::Info { .. } => {
                        let text = definition.map(template).unwrap_or_default().to_owned();
                        rows.push((Prop::Text, "Text", Control::Typed, text));
                        if item.part != Part::Title {
                            rows.push((
                                Prop::OnBackground,
                                "On every card",
                                Control::Check(!on_card),
                                String::new(),
                            ));
                        }
                    }
                    _ => {}
                }
                let rect = designer.rect_of(item);
                for (prop, label, value) in [
                    (Prop::X, "x", rect.x),
                    (Prop::Y, "y", rect.y),
                    (Prop::W, "w", rect.w),
                    (Prop::H, "h", rect.h),
                ] {
                    rows.push((prop, label, Control::Typed, value.to_string()));
                }
            }
            None => {
                heading = view.screen.stack.title.clone();
                detail = "Nothing is selected".to_owned();
                let widgets = editor.layout.items.len();
                notes.push(format!("{widgets} widget{}", if widgets == 1 { "" } else { "s" }));
                let cards = view.screen.source.count;
                notes.push(format!("{cards} card{}", if cards == 1 { "" } else { "s" }));
                if editor.layout.by_hand {
                    notes.push("Layout made by hand".to_owned());
                    rows.push((Prop::Reset, "Reset layout", Control::Button, String::new()));
                } else {
                    notes.push("Automatic layout".to_owned());
                }
            }
        }

        // The size of the window, then where its lines are.
        let pairs = rows.iter().filter(|row| matches!(row.0, Prop::X | Prop::W)).count() as i32;
        let singles = rows.len() as i32 - 2 * pairs;
        let height =
            chrome::FRAME_TOP + 6 + 24 + notes.len() as i32 * 11 + (singles + pairs) * ROW + 5;
        let window = designer.window_for(editor, height);
        let left = window.x + 6;
        let right = window.right() - 6;
        let mut y = window.y + chrome::FRAME_TOP + 6 + 24 + notes.len() as i32 * 11;

        let mut placed = Vec::new();
        let mut pending: Option<i32> = None;
        for (prop, label, control, value) in rows {
            let area = match (&control, prop) {
                // x and y share a line, and so do w and h.
                (_, Prop::X | Prop::W) => {
                    pending = Some(y);
                    Rect::new(left + 12, y, 44, FIELD_HEIGHT)
                }
                (_, Prop::Y | Prop::H) => {
                    let line = pending.take().unwrap_or(y);
                    y = line + ROW;
                    Rect::new(left + 76, line, 44, FIELD_HEIGHT)
                }
                (Control::Check(_), _) => {
                    let area = Rect::new(left, y, right - left, FIELD_HEIGHT);
                    y += ROW;
                    area
                }
                (Control::Button, _) => {
                    let area = Rect::new(left, y, right - left, BUTTON_HEIGHT);
                    y += ROW;
                    area
                }
                _ => {
                    let area = Rect::new(left + 34, y, right - left - 34, FIELD_HEIGHT);
                    y += ROW;
                    area
                }
            };
            placed.push(Row { prop, label, control, value, area });
        }
        Properties { window, heading, detail, notes, rows: placed }
    }

    fn title_bar(&self) -> Rect {
        Rect::new(self.window.x, self.window.y, self.window.w, chrome::FRAME_TOP)
    }
}

impl Designer {
    /// Where the Properties window is: where the user put it, or else in a
    /// corner of the screen away from the selected widget.
    fn window_for(&self, editor: &Editor, height: i32) -> Rect {
        let w = PROPERTIES_WIDTH + 2;
        if let Some((x, y)) = self.properties_at {
            return Rect::new(
                x.clamp(0, WIDTH - w),
                y.clamp(chrome::MENU_HEIGHT + 1, chrome::KEYS_TOP - 20),
                w,
                height,
            );
        }
        // The four corners of the desktop, the top right one first: the first
        // that leaves the selected widget in sight, with its label.
        let (left, right) = (PALETTE.right() + 6, WIDTH - w - 5);
        let (top, bottom) = (21, chrome::KEYS_TOP - height - 4);
        let corners = [(right, top), (right, bottom), (left, top), (left, bottom)];
        let selected = self.selection(editor).map(|item| {
            let area = editor.on_screen(self.rect_of(item));
            Rect::new(area.x - 60, area.y - 4, area.w + 64, area.h + 8)
        });
        let free = corners.into_iter().find(|(x, y)| {
            selected.is_none_or(|area| area.intersection(&Rect::new(*x, *y, w, height)).is_empty())
        });
        let (x, y) = free.unwrap_or((right, top));
        Rect::new(x, y, w, height)
    }
}

fn draw_properties(
    frame: &mut Frame,
    fonts: &Fonts,
    designer: &Designer,
    properties: &Properties,
    blink: bool,
) {
    draw_window(frame, fonts, properties.window, "Properties", WindowStyle::SYSTEM);
    let content = content_of(properties.window);
    let previous = frame.clip_to(content);
    let x = content.x + 5;
    fonts.bold.draw(frame, x, content.y + 15, &properties.heading, slot::INK);
    fonts.regular.draw(frame, x, content.y + 26, &properties.detail, slot::GREY);
    for (index, note) in properties.notes.iter().enumerate() {
        fonts.regular.draw(frame, x, content.y + 39 + index as i32 * 11, note, slot::INK);
    }

    for row in &properties.rows {
        let typing = designer.typing.as_ref().filter(|typing| typing.prop == row.prop);
        match &row.control {
            Control::Typed => {
                let label_x = row.area.x - 5 - fonts.regular.width(row.label);
                fonts.regular.draw(frame, label_x, row.area.y + 11, row.label, slot::COCOA);
                match typing {
                    Some(typing) => {
                        draw_field(frame, fonts, row.area, &typing.text, Some(typing.cursor), blink)
                    }
                    None => draw_field(frame, fonts, row.area, &row.value, None, false),
                }
            }
            Control::Choice => {
                let label_x = row.area.x - 5 - fonts.regular.width(row.label);
                fonts.regular.draw(frame, label_x, row.area.y + 11, row.label, slot::COCOA);
                draw_choice(frame, fonts, row.area, &row.value, false);
            }
            Control::Check(checked) => {
                draw_check(frame, row.area, *checked, false);
                fonts.regular.draw(frame, row.area.x + 17, row.area.y + 11, row.label, slot::INK);
            }
            Control::Button => {
                draw_button(frame, fonts, row.area, row.label, ButtonLook::default());
            }
        }
    }
    frame.set_clip(previous);
}

/// Starts typing a property, from what it is now.
fn start_typing(designer: &mut Designer, properties: &Properties, prop: Prop) {
    if let Some(row) = properties.rows.iter().find(|row| row.prop == prop) {
        designer.typing =
            Some(Typing { prop, text: row.value.clone(), cursor: row.value.chars().count() });
    }
}

/// Sends what was typed in a property to the server.
fn commit_typing<A: Api>(session: &mut Session<A>, designer: &mut Designer, fonts: &Fonts) {
    let Some(typing) = designer.typing.take() else { return };
    let editor = Editor::of(session.view(), fonts);
    let Some(item) = designer.selection(&editor) else { return };
    let name = item.widget.clone();
    match typing.prop {
        Prop::Label => {
            session.run_with(
                "set_widget",
                json!({ "widget": name, "changes": { "label": typing.text } }),
            );
        }
        Prop::Text => {
            session.run_with(
                "set_widget",
                json!({ "widget": name, "changes": { "text": typing.text } }),
            );
        }
        Prop::X | Prop::Y | Prop::W | Prop::H => {
            let Ok(value) = typing.text.trim().parse::<i32>() else {
                session.say("A whole number is expected here.");
                return;
            };
            let mut rect = designer.rect_of(item);
            match typing.prop {
                Prop::X => rect.x = value,
                Prop::Y => rect.y = value,
                Prop::W => rect.w = value,
                _ => rect.h = value,
            }
            let rect = editor.kept_inside(rect);
            if rect != item.area {
                place(session, &editor, &[(name, rect)]);
            }
        }
        _ => {}
    }
}

/// Acts on a property that is not typed: a tick, a choice, a button.
fn press_property<A: Api>(session: &mut Session<A>, designer: &Designer, fonts: &Fonts, row: &Row) {
    let editor = Editor::of(session.view(), fonts);
    let selected = designer.selection(&editor).map(|item| item.widget.clone());
    let change = |session: &mut Session<A>, changes: serde_json::Value| {
        if let Some(name) = &selected {
            session.run_with("set_widget", json!({ "widget": name, "changes": changes }));
        }
    };
    match (row.prop, &row.control) {
        (Prop::Required, Control::Check(on)) => change(session, json!({ "required": !on })),
        (Prop::Main, Control::Check(on)) => change(session, json!({ "main": !on })),
        (Prop::OnBackground, Control::Check(on)) => {
            change(session, json!({ "layer": if *on { "card" } else { "background" } }));
        }
        (Prop::Action, _) => {
            // The next action of the stack, then none, going round.
            let actions = button_actions(session.view());
            let current = selected
                .as_deref()
                .and_then(|name| widget(session.view(), name))
                .and_then(|w| w.action.clone());
            let at = current.and_then(|name| actions.iter().position(|(known, _)| *known == name));
            let next = match at {
                Some(at) => actions.get(at + 1),
                None => actions.first(),
            };
            let name = next.map_or("", |(name, _)| name.as_str());
            change(session, json!({ "action": name }));
        }
        (Prop::Reset, _) => session.run("reset_layout"),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// The "New widget" menu
// ---------------------------------------------------------------------------

const MENU: [(Tool, &str); 3] =
    [(Tool::Field, "Field"), (Tool::Text, "Text"), (Tool::Button, "Button")];

fn menu_window() -> Rect {
    window_around(110, MENU.len() as i32 * MENU_ROW + 10)
}

fn menu_rows() -> Vec<Rect> {
    let content = content_of(menu_window());
    (0..MENU.len() as i32)
        .map(|line| {
            Rect::new(content.x + 4, content.y + 5 + line * MENU_ROW, content.w - 8, MENU_ROW)
        })
        .collect()
}

fn draw_menu(frame: &mut Frame, fonts: &Fonts, chosen: usize) {
    draw_window(frame, fonts, menu_window(), "New widget", WindowStyle::SYSTEM);
    for (index, (row, (_, label))) in menu_rows().into_iter().zip(MENU).enumerate() {
        let colour = if index == chosen {
            frame.fill(row, slot::BLUE);
            slot::WHITE
        } else {
            slot::INK
        };
        fonts.regular.draw(frame, row.x + 6, row.y + 11, label, colour);
    }
}

/// Makes a widget of the kind a tool makes. A field has a column behind it:
/// it goes through its dialog. A text or a button is put at a place of the
/// content, or under what is already there.
fn create<A: Api>(
    session: &mut Session<A>,
    designer: &mut Designer,
    fonts: &Fonts,
    tool: Tool,
    at: Option<(i32, i32)>,
) {
    flush(session, designer, fonts);
    designer.tool = Tool::Pointer;
    let (kind, size) = match tool {
        Tool::Pointer => return,
        Tool::Field => {
            session.run("new_field");
            return;
        }
        Tool::Text => ("text", (90, FIELD_HEIGHT)),
        Tool::Button => ("button", (62, BUTTON_HEIGHT)),
    };

    // A widget with a place of its own ends the automatic layout: the others
    // are written down where they are first, so that they stay there.
    let editor = Editor::of(session.view(), fonts);
    if !editor.layout.free {
        place(session, &editor, &[]);
    }
    let editor = Editor::of(session.view(), fonts);
    let (x, y) = at.unwrap_or_else(|| {
        let lowest = editor.layout.items.iter().map(|item| item.area.bottom()).max().unwrap_or(30);
        (21, lowest + 6)
    });
    let rect = editor.kept_inside(Rect::new(to_grid(x), to_grid(y), size.0, size.1));
    let layer = if designer.layer == Layer::Card { "card" } else { "background" };
    session.run_with(
        "add_widget",
        json!({
            "kind": kind,
            "layer": layer,
            "props": { "placement": { "x": rect.x, "y": rect.y, "w": rect.w, "h": rect.h } },
        }),
    );
}

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

/// The widgets in the order Tab visits them: down the window, left to right.
fn visiting_order(editor: &Editor) -> Vec<String> {
    let mut items: Vec<&Item> = editor.layout.items.iter().collect();
    items.sort_by_key(|item| (item.area.y, item.area.x));
    items.into_iter().map(|item| item.widget.clone()).collect()
}

fn select_next(designer: &mut Designer, editor: &Editor, step: isize) {
    let order = visiting_order(editor);
    if order.is_empty() {
        return;
    }
    let next =
        match designer.selected.as_ref().and_then(|name| order.iter().position(|n| n == name)) {
            Some(at) => (at as isize + step).rem_euclid(order.len() as isize) as usize,
            None if step > 0 => 0,
            None => order.len() - 1,
        };
    designer.selected = Some(order[next].clone());
}

/// Moves or resizes the selection by a pixel, without saving yet.
fn nudge(designer: &mut Designer, editor: &Editor, by: (i32, i32), grow: (i32, i32)) {
    let Some(item) = designer.selection(editor) else { return };
    let base = designer.rect_of(item);
    let rect = editor.kept_inside(Rect::new(
        base.x + by.0,
        base.y + by.1,
        base.w + grow.0,
        base.h + grow.1,
    ));
    designer.nudge = Some(Nudge { widget: item.widget.clone(), rect, idle: 0 });
}

pub fn handle_key<A: Api>(
    session: &mut Session<A>,
    designer: &mut Designer,
    fonts: &Fonts,
    key: Key,
) {
    let editor = Editor::of(session.view(), fonts);
    sync(session, designer, &editor);

    // The menu takes the keys while it is open.
    if let Some(chosen) = designer.menu {
        match key {
            Key::Up => designer.menu = Some(chosen.saturating_sub(1)),
            Key::Down | Key::Tab => designer.menu = Some((chosen + 1).min(MENU.len() - 1)),
            Key::Enter => {
                designer.menu = None;
                create(session, designer, fonts, MENU[chosen].0, None);
            }
            Key::Escape | Key::F(_) => designer.menu = None,
            _ => {}
        }
        return;
    }

    // So does a property being typed.
    if let Some(prop) = designer.typing.as_ref().map(|typing| typing.prop) {
        match key {
            Key::Enter => commit_typing(session, designer, fonts),
            Key::Escape => designer.typing = None,
            Key::Tab | Key::BackTab => {
                commit_typing(session, designer, fonts);
                let editor = Editor::of(session.view(), fonts);
                let properties = Properties::of(session.view(), designer, &editor);
                let typed: Vec<Prop> = properties
                    .rows
                    .iter()
                    .map(|row| row.prop)
                    .filter(|prop| prop.is_typed())
                    .collect();
                if let Some(at) = typed.iter().position(|known| *known == prop) {
                    let step = if key == Key::Tab { 1 } else { typed.len() - 1 };
                    start_typing(designer, &properties, typed[(at + step) % typed.len()]);
                }
            }
            _ => {
                if let Some(typing) = &mut designer.typing {
                    edit_line(&mut typing.text, &mut typing.cursor, key, 200);
                }
            }
        }
        return;
    }

    match key {
        Key::F(number @ (3 | 12)) => {
            flush(session, designer, fonts);
            session.press(&format!("F{number}"));
        }
        Key::F(2) | Key::Undo => {
            flush(session, designer, fonts);
            session.run("undo");
        }
        Key::F(4) => {
            flush(session, designer, fonts);
            session.run("author_form");
        }
        Key::F(6) => designer.menu = Some(0),
        Key::F(7) => designer.properties_hidden = !designer.properties_hidden,
        Key::F(10) => {
            flush(session, designer, fonts);
            session.run("stack_history");
        }
        Key::F(11) => {
            flush(session, designer, fonts);
            session.stop_designing();
        }
        Key::F(_) => session.say("This key is not active in the editor."),
        Key::Tab => {
            flush(session, designer, fonts);
            select_next(designer, &Editor::of(session.view(), fonts), 1);
        }
        Key::BackTab => {
            flush(session, designer, fonts);
            select_next(designer, &Editor::of(session.view(), fonts), -1);
        }
        Key::Left => nudge(designer, &editor, (-1, 0), (0, 0)),
        Key::Right => nudge(designer, &editor, (1, 0), (0, 0)),
        Key::Up => nudge(designer, &editor, (0, -1), (0, 0)),
        Key::Down => nudge(designer, &editor, (0, 1), (0, 0)),
        Key::ShiftLeft => nudge(designer, &editor, (0, 0), (-1, 0)),
        Key::ShiftRight => nudge(designer, &editor, (0, 0), (1, 0)),
        Key::ShiftUp => nudge(designer, &editor, (0, 0), (0, -1)),
        Key::ShiftDown => nudge(designer, &editor, (0, 0), (0, 1)),
        Key::Delete | Key::Backspace => {
            if let Some(name) = designer.selected.clone() {
                flush(session, designer, fonts);
                session.run_with("remove_widget", json!({ "widget": name }));
            }
        }
        // Enter goes to the first thing to type about the selection.
        Key::Enter => {
            flush(session, designer, fonts);
            let editor = Editor::of(session.view(), fonts);
            let properties = Properties::of(session.view(), designer, &editor);
            if let Some(row) = properties.rows.iter().find(|row| row.prop.is_typed()) {
                designer.properties_hidden = false;
                start_typing(designer, &properties, row.prop);
            }
        }
        Key::Escape => {
            if designer.tool != Tool::Pointer {
                designer.tool = Tool::Pointer;
            } else if designer.selected.is_some() {
                flush(session, designer, fonts);
                designer.selected = None;
            } else {
                session.stop_designing();
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// The pointer
// ---------------------------------------------------------------------------

/// The rectangle a click selects a widget by: a field is also selected by
/// its label, which is written on its left.
fn reach(fonts: &Fonts, item: &Item, area: Rect) -> Rect {
    match &item.part {
        Part::Field { label, .. } => {
            let label = fonts.regular.width(label) + 5;
            Rect::new(area.x - label, area.y, area.w + label, area.h)
        }
        _ => area,
    }
}

pub fn pointer_down<A: Api>(
    session: &mut Session<A>,
    designer: &mut Designer,
    fonts: &Fonts,
    x: i32,
    y: i32,
) {
    let editor = Editor::of(session.view(), fonts);
    sync(session, designer, &editor);

    // The function key bar: a click is the key.
    let key = chrome::key_slots(fonts, KEYS.into_iter())
        .into_iter()
        .find(|(_, _, area)| area.contains(x, y))
        .and_then(|(name, _, _)| name[1..].parse::<u8>().ok());
    if let Some(number) = key {
        designer.menu = None;
        if designer.typing.is_some() {
            commit_typing(session, designer, fonts);
        }
        handle_key(session, designer, fonts, Key::F(number));
        return;
    }

    if designer.menu.is_some() {
        let chosen = menu_rows().iter().position(|row| row.contains(x, y));
        designer.menu = None;
        if let Some(chosen) = chosen {
            create(session, designer, fonts, MENU[chosen].0, None);
        }
        return;
    }

    // The Properties window is over everything else.
    if !designer.properties_hidden {
        let properties = Properties::of(session.view(), designer, &editor);
        if properties.window.contains(x, y) {
            let row = properties.rows.iter().find(|row| row.area.contains(x, y)).cloned();
            match row {
                Some(row) if row.control == Control::Typed => {
                    if designer.typing.as_ref().is_some_and(|typing| typing.prop != row.prop) {
                        commit_typing(session, designer, fonts);
                    }
                    if designer.typing.is_none() {
                        let editor = Editor::of(session.view(), fonts);
                        let properties = Properties::of(session.view(), designer, &editor);
                        start_typing(designer, &properties, row.prop);
                    }
                }
                Some(row) => {
                    commit_typing(session, designer, fonts);
                    flush(session, designer, fonts);
                    press_property(session, designer, fonts, &row);
                }
                None if close_box_of(properties.window).contains(x, y) => {
                    designer.properties_hidden = true;
                    designer.typing = None;
                }
                None if properties.title_bar().contains(x, y) => {
                    designer.drag = Some(Drag {
                        widget: String::new(),
                        grip: Grip::Properties,
                        from: (x, y),
                        origin: properties.window,
                        current: properties.window,
                    });
                }
                None => {}
            }
            return;
        }
    }
    // A click anywhere else ends what was being typed.
    if designer.typing.is_some() {
        commit_typing(session, designer, fonts);
    }
    let editor = Editor::of(session.view(), fonts);

    if let Some((cell, _)) = palette_cells().into_iter().find(|(_, area)| area.contains(x, y)) {
        match cell {
            Cell::Tool(tool) => designer.tool = tool,
            Cell::Layer(layer) => designer.layer = layer,
            Cell::Grid => designer.grid = !designer.grid,
            Cell::Guides => designer.guides = !designer.guides,
        }
        return;
    }

    // The close box goes back, like F12.
    if close_box_of(editor.window).contains(x, y) {
        handle_key(session, designer, fonts, Key::F(12));
        return;
    }

    // A handle of the selection resizes it.
    if designer.tool == Tool::Pointer {
        if let Some(item) = designer.selection(&editor) {
            let rect = designer.rect_of(item);
            let grabbed = handles(editor.on_screen(rect))
                .into_iter()
                .find(|(_, area)| area.inset(-1).contains(x, y));
            if let Some((handle, _)) = grabbed {
                let widget = item.widget.clone();
                designer.nudge = None;
                designer.drag = Some(Drag {
                    widget,
                    grip: Grip::Resize(handle),
                    from: (x, y),
                    origin: rect,
                    current: rect,
                });
                return;
            }
        }
    }

    if !editor.content.contains(x, y) {
        return;
    }
    let (cx, cy) = (x - editor.content.x, y - editor.content.y);
    if designer.tool != Tool::Pointer {
        let tool = designer.tool;
        create(session, designer, fonts, tool, Some((cx, cy)));
        return;
    }

    // A widget under the pointer is selected, and follows it if it moves.
    // What is drawn last is on top.
    let hit = editor
        .layout
        .items
        .iter()
        .rev()
        .find(|item| reach(fonts, item, designer.rect_of(item)).contains(cx, cy));
    match hit {
        Some(item) => {
            let name = item.widget.clone();
            if designer.selected.as_deref() != Some(name.as_str()) {
                flush(session, designer, fonts);
            }
            let editor = Editor::of(session.view(), fonts);
            let Some(item) = editor.layout.item(&name) else { return };
            let rect = designer.rect_of(item);
            designer.selected = Some(name.clone());
            designer.drag = Some(Drag {
                widget: name,
                grip: Grip::Move,
                from: (x, y),
                origin: rect,
                current: rect,
            });
        }
        None => {
            flush(session, designer, fonts);
            designer.selected = None;
        }
    }
}

/// How far the pointer must go, a button held, before a click becomes a drag.
const DRAG_THRESHOLD: i32 = 2;

/// After an event: makes what the editor remembers agree with the screen the
/// server answered with.
pub fn settle<A: Api>(session: &mut Session<A>, designer: &mut Designer, fonts: &Fonts) {
    let editor = Editor::of(session.view(), fonts);
    sync(session, designer, &editor);
}

/// The pointer moved, a button held or not. Returns true when something on
/// screen followed it.
pub fn pointer_move<A: Api>(
    session: &mut Session<A>,
    designer: &mut Designer,
    fonts: &Fonts,
    x: i32,
    y: i32,
) -> bool {
    let Some(drag) = designer.drag.clone() else { return false };
    // A click that barely trembles is a click: nothing moves.
    let travelled = (x - drag.from.0).abs().max((y - drag.from.1).abs());
    if travelled < DRAG_THRESHOLD && drag.current == drag.origin {
        return false;
    }
    let wanted = dragged(&drag, x, y);
    if drag.grip == Grip::Properties {
        designer.properties_at = Some((wanted.x, wanted.y));
        return true;
    }
    let editor = Editor::of(session.view(), fonts);
    let (current, guides) = settle_drag(designer, &editor, &drag, wanted);
    designer.guide_lines = guides;
    if let Some(drag) = &mut designer.drag {
        drag.current = current;
    }
    true
}

/// The pointer was released: what was dragged is where it now belongs.
pub fn pointer_up<A: Api>(
    session: &mut Session<A>,
    designer: &mut Designer,
    fonts: &Fonts,
    x: i32,
    y: i32,
) {
    pointer_move(session, designer, fonts, x, y);
    designer.guide_lines.clear();
    let Some(drag) = designer.drag.take() else { return };
    if drag.grip != Grip::Properties && drag.current != drag.origin {
        designer.nudge = None;
        let editor = Editor::of(session.view(), fonts);
        place(session, &editor, &[(drag.widget, drag.current)]);
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// "Author mode", in the menu bar.
fn draw_badge(frame: &mut Frame, fonts: &Fonts) {
    let text = "Author mode";
    let width = fonts.bold.width(text) + 10;
    let area = Rect::new(150, 1, width, chrome::MENU_HEIGHT - 2);
    frame.fill(area, slot::INK);
    fonts.bold.draw(frame, area.x + 5, 11, text, slot::YELLOW);
}

/// A dotted line: one pixel out of two.
fn dotted(frame: &mut Frame, area: Rect, colour: u8) {
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if (x + y) % 2 == 0 {
                frame.set(x, y, colour);
            }
        }
    }
}

fn dotted_outline(frame: &mut Frame, area: Rect, colour: u8) {
    dotted(frame, Rect::new(area.x, area.y, area.w, 1), colour);
    dotted(frame, Rect::new(area.x, area.bottom() - 1, area.w, 1), colour);
    dotted(frame, Rect::new(area.x, area.y, 1, area.h), colour);
    dotted(frame, Rect::new(area.right() - 1, area.y, 1, area.h), colour);
}

/// A widget as the editor shows it: its structure, not the values of a card.
fn draw_item(frame: &mut Frame, fonts: &Fonts, view: &View, item: &Item, area: Rect) {
    let definition = widget(view, &item.widget);
    match &item.part {
        Part::Title | Part::Banner { .. } => {
            let clip = frame.clip_to(area);
            let text = definition.map(template).unwrap_or_default();
            fonts.bold.draw_scaled(frame, area.x + 13, area.y + 21, text, slot::SILVER, 2);
            frame.set_clip(clip);
        }
        Part::Text | Part::Info { .. } => {
            let clip = frame.clip_to(area);
            let text = definition.map(template).unwrap_or_default();
            let baseline = area.y + if area.h >= FIELD_HEIGHT { 11 } else { 8 };
            fonts.regular.draw(frame, area.x, baseline, text, slot::INK);
            frame.set_clip(clip);
        }
        Part::Field { column, label, kind } => {
            let label_x = area.x - 5 - fonts.regular.width(label);
            fonts.regular.draw(frame, label_x, area.y + 11, label, slot::COCOA);
            match kind {
                FieldKind::Check => draw_check(frame, area, false, false),
                FieldKind::Choice => draw_choice(frame, fonts, area, "", false),
                FieldKind::Text => draw_field(frame, fonts, area, "", None, false),
            }
            // The name of the column, where its values will be.
            if *kind != FieldKind::Check {
                let clip = frame.clip_to(area.inset(2));
                fonts.regular.draw(
                    frame,
                    area.x + chrome::FIELD_TEXT_X,
                    area.y + 11,
                    column,
                    slot::SILVER,
                );
                frame.set_clip(clip);
            }
        }
        Part::Picture => {
            frame.fill(area, slot::ICE);
            frame.dither(area, slot::AZURE, 4);
            frame.outline(area, slot::INK);
        }
        Part::Grid => {
            frame.fill(area, slot::WHITE);
            frame.outline(area, slot::INK);
        }
        Part::Button { label, look } => draw_button(frame, fonts, area, label, *look),
    }
    // A widget of this card only wears a violet dotted line.
    if definition.is_some_and(Widget::on_card) {
        dotted_outline(frame, area.inset(-2), slot::VIOLET);
    }
}

/// Draws the editor. The message, the question and the key bar are drawn by
/// the caller, as for any screen.
pub fn draw<A: Api>(
    session: &Session<A>,
    designer: &Designer,
    fonts: &Fonts,
    frame: &mut Frame,
    clock: &str,
    blink: bool,
) {
    let view = session.view();
    let editor = Editor::of(view, fonts);

    chrome::draw_desktop(frame);
    chrome::draw_menu_bar(frame, fonts, Some(&session.hello().instance), clock);
    draw_badge(frame, fonts);
    let title = format!("{}: background", view.screen.stack.title);
    draw_window(frame, fonts, editor.window, &title, WindowStyle::STACK);

    let previous = frame.clip_to(editor.content);
    if let Some(paper) = editor.layout.paper {
        draw_paper(frame, editor.on_screen(paper));
    }
    for rule in &editor.layout.rules {
        frame.fill(editor.on_screen(*rule), slot::ICE);
    }
    if let Some(header) = editor.layout.header {
        let header = editor.on_screen(header);
        frame.hline(header.x, header.bottom() - 1, header.w, slot::RED);
    }
    if designer.grid {
        for y in (0..editor.content.h).step_by(GRID_DOTS as usize) {
            for x in (0..editor.content.w).step_by(GRID_DOTS as usize) {
                frame.set(editor.content.x + x, editor.content.y + y, slot::OCHRE);
            }
        }
    }

    for item in &editor.layout.items {
        draw_item(frame, fonts, view, item, editor.on_screen(designer.rect_of(item)));
    }

    // The selection: a dotted line around it, and its eight handles.
    if let Some(item) = designer.selection(&editor) {
        let area = editor.on_screen(designer.rect_of(item));
        dotted_outline(frame, area.inset(-1), slot::BLUE);
        for (_, handle) in handles(area) {
            frame.fill(handle, slot::WHITE);
            frame.outline(handle, slot::BLUE);
        }
    }
    for guide in &designer.guide_lines {
        dotted(frame, editor.on_screen(*guide), slot::RED);
    }
    frame.set_clip(previous);

    draw_palette(frame, fonts, designer);
    if !designer.properties_hidden {
        let properties = Properties::of(view, designer, &editor);
        draw_properties(frame, fonts, designer, &properties, blink);
    }
    if let Some(chosen) = designer.menu {
        draw_menu(frame, fonts, chosen);
    }
}

/// What to tell the user about the tool in hand, when it is not obvious.
pub fn hint(designer: &Designer) -> Option<&'static str> {
    match designer.tool {
        Tool::Pointer => None,
        Tool::Field => Some("Field: click on the card to add one. Esc gives up."),
        Tool::Text => Some("Text: click where it goes. Esc gives up."),
        Tool::Button => Some("Button: click where it goes. Esc gives up."),
    }
}

/// Test access to what is otherwise private to the editor.
#[cfg(test)]
pub mod probe {
    use super::*;

    /// The rectangle of a widget on screen, as the editor draws it.
    pub fn area_of(view: &View, designer: &Designer, fonts: &Fonts, name: &str) -> Rect {
        let editor = Editor::of(view, fonts);
        let item = editor.layout.item(name).unwrap_or_else(|| panic!("no widget {name}"));
        editor.on_screen(designer.rect_of(item))
    }

    /// The rectangle of a widget in the content of its window.
    pub fn place_of(view: &View, designer: &Designer, fonts: &Fonts, name: &str) -> Rect {
        let editor = Editor::of(view, fonts);
        designer.rect_of(editor.layout.item(name).unwrap_or_else(|| panic!("no widget {name}")))
    }

    /// The control of a property on screen, and what it shows.
    pub fn property(
        view: &View,
        designer: &Designer,
        fonts: &Fonts,
        prop: Prop,
    ) -> Option<(Rect, String)> {
        let editor = Editor::of(view, fonts);
        let properties = Properties::of(view, designer, &editor);
        properties.rows.into_iter().find(|row| row.prop == prop).map(|row| (row.area, row.value))
    }

    pub fn properties_window(view: &View, designer: &Designer, fonts: &Fonts) -> Rect {
        let editor = Editor::of(view, fonts);
        Properties::of(view, designer, &editor).window
    }

    pub fn guides(designer: &Designer) -> usize {
        designer.guide_lines.len()
    }

    pub fn is_typing(designer: &Designer) -> Option<(Prop, String)> {
        designer.typing.as_ref().map(|typing| (typing.prop, typing.text.clone()))
    }

    pub fn menu_is_open(designer: &Designer) -> bool {
        designer.menu.is_some()
    }

    pub fn is_free(view: &View, fonts: &Fonts) -> bool {
        Editor::of(view, fonts).layout.free
    }
}

#[cfg(test)]
mod tests {
    use super::probe::*;
    use super::*;
    use crate::screen::{self, Ui};
    use crate::testing::{count_slot, editor_session, has_slot, snapshot, Fake};

    fn fonts() -> Fonts {
        Fonts::builtin()
    }

    fn render(session: &Session<Fake>, ui: &Ui) -> Frame {
        let mut frame = Frame::new();
        screen::draw(session, ui, &fonts(), &mut frame, "16:54", true);
        frame
    }

    fn press(session: &mut Session<Fake>, ui: &mut Ui, keys: &[Key]) {
        for key in keys {
            screen::handle_key(session, ui, &fonts(), *key);
        }
    }

    fn type_text(session: &mut Session<Fake>, ui: &mut Ui, text: &str) {
        for c in text.chars() {
            screen::handle_key(session, ui, &fonts(), Key::Char(c));
        }
    }

    fn click(session: &mut Session<Fake>, ui: &mut Ui, x: i32, y: i32) {
        screen::pointer_down(session, ui, &fonts(), x, y);
        screen::pointer_up(session, ui, &fonts(), x, y);
    }

    /// Presses on a point, moves the pointer by some pixels, and lets go.
    fn drag(session: &mut Session<Fake>, ui: &mut Ui, from: (i32, i32), by: (i32, i32)) {
        screen::pointer_down(session, ui, &fonts(), from.0, from.1);
        screen::pointer_move(session, ui, &fonts(), from.0 + by.0 / 2, from.1 + by.1 / 2);
        screen::pointer_move(session, ui, &fonts(), from.0 + by.0, from.1 + by.1);
        screen::pointer_up(session, ui, &fonts(), from.0 + by.0, from.1 + by.1);
    }

    fn centre(area: Rect) -> (i32, i32) {
        (area.x + area.w / 2, area.y + area.h / 2)
    }

    fn area(session: &Session<Fake>, ui: &Ui, name: &str) -> Rect {
        area_of(session.view(), &ui.designer, &fonts(), name)
    }

    fn place(session: &Session<Fake>, ui: &Ui, name: &str) -> Rect {
        place_of(session.view(), &ui.designer, &fonts(), name)
    }

    /// The placements the last "place_widgets" action carried, by widget.
    fn sent_places(session: &Session<Fake>) -> Vec<String> {
        let (action, input) = session.api().last();
        assert_eq!(action, "place_widgets");
        input["placements"].as_object().unwrap().keys().cloned().collect()
    }

    #[test]
    fn the_editor_shows_the_background_with_its_tools() {
        let (session, ui) = editor_session();
        let frame = render(&session, &ui);
        snapshot(&frame, "editor");

        // The window of the stack, in the colours of a stack, with the paper.
        let last_name = area(&session, &ui, "last_name");
        assert_eq!(frame.pixel(last_name.x, last_name.y), Some(slot::INK));
        // A field shows the name of its column, in grey, not a value.
        assert!(has_slot(&frame, last_name.inset(2), slot::SILVER));
        assert!(!has_slot(&frame, last_name.inset(2), slot::INK));
        // The mark of author mode in the menu bar, and the tool palette.
        assert!(count_slot(&frame, Rect::new(150, 1, 80, 12), slot::YELLOW) > 40);
        let (_, pointer) = palette_cells()[0];
        assert_eq!(
            frame.pixel(pointer.x + 1, pointer.y + 1),
            Some(slot::YELLOW),
            "the tool in hand"
        );
        // The keys are those of the editor.
        let keys = chrome::key_slots(&fonts(), KEYS.into_iter());
        assert_eq!(keys.len(), KEYS.len());
        // Nothing is selected: the Properties window is about the stack.
        let properties = properties_window(session.view(), &ui.designer, &fonts());
        assert!(properties.x > 300);
        assert!(property(session.view(), &ui.designer, &fonts(), Prop::Reset).is_none());
        assert!(!is_free(session.view(), &fonts()));
    }

    #[test]
    fn dragging_a_widget_writes_down_where_everything_is() {
        let (mut session, mut ui) = editor_session();
        let before = place(&session, &ui, "first_name");
        let untouched = place(&session, &ui, "last_name");

        let from = centre(area(&session, &ui, "first_name"));
        drag(&mut session, &mut ui, from, (-17, 22));

        // The layout was automatic: every widget gets its rectangle at once.
        let mut names = sent_places(&session);
        names.sort();
        assert_eq!(names, ["first_name", "last_name", "next", "photo", "title"]);
        assert!(is_free(session.view(), &fonts()));
        // The widget moved, on the grid; the others stayed where they were.
        let after = place(&session, &ui, "first_name");
        assert_eq!((after.w, after.h), (before.w, before.h));
        assert_eq!(after.y % GRID, 0);
        assert!((after.y - (before.y + 22)).abs() <= GRID / 2);
        assert!((after.x - (before.x - 17)).abs() <= SNAP);
        assert_eq!(place(&session, &ui, "last_name"), untouched);
        assert_eq!(ui.designer.selected.as_deref(), Some("first_name"));

        // From now on a move only sends what moved.
        let from = centre(area(&session, &ui, "first_name"));
        drag(&mut session, &mut ui, from, (0, 12));
        assert_eq!(sent_places(&session), ["first_name"]);
        let frame = render(&session, &ui);
        snapshot(&frame, "editor-moved");
        // The Properties window now offers to go back to the automatic layout.
        click(&mut session, &mut ui, 3, 285);
        press(&mut session, &mut ui, &[Key::Escape]);
        assert!(property(session.view(), &ui.designer, &fonts(), Prop::Reset).is_some());
    }

    #[test]
    fn a_click_without_a_move_only_selects() {
        let (mut session, mut ui) = editor_session();
        let (x, y) = centre(area(&session, &ui, "last_name"));
        click(&mut session, &mut ui, x, y);
        assert_eq!(ui.designer.selected.as_deref(), Some("last_name"));
        assert_eq!(session.api().count("place_widgets"), 0);
        // A field is also selected by its label.
        let first_name = area(&session, &ui, "first_name");
        click(&mut session, &mut ui, first_name.x - 20, first_name.y + 5);
        assert_eq!(ui.designer.selected.as_deref(), Some("first_name"));

        // The selection has a dotted line and handles.
        let frame = render(&session, &ui);
        let (_, corner) = handles(first_name)[0];
        assert_eq!(frame.pixel(corner.x, corner.y), Some(slot::BLUE));
        assert_eq!(frame.pixel(corner.x + 2, corner.y + 2), Some(slot::WHITE));
        // A click on the paper selects nothing.
        let title = area(&session, &ui, "title");
        click(&mut session, &mut ui, title.right() - 4, title.bottom() + 4);
        assert_eq!(ui.designer.selected, None);
    }

    #[test]
    fn a_handle_resizes_the_selection() {
        let (mut session, mut ui) = editor_session();
        let before = place(&session, &ui, "last_name");
        let on_screen = area(&session, &ui, "last_name");
        click(&mut session, &mut ui, on_screen.x + 20, on_screen.y + 5);

        // The handle in the middle of the right side.
        let (handle, grip) =
            handles(on_screen).into_iter().find(|(h, _)| (h.h, h.v) == (1, 0)).unwrap();
        assert_eq!(handle, Handle { h: 1, v: 0 });
        drag(&mut session, &mut ui, centre(grip), (-60, 9));

        let after = place(&session, &ui, "last_name");
        assert_eq!(
            (after.x, after.y, after.h),
            (before.x, before.y, before.h),
            "only the right side moved"
        );
        assert!((after.w - (before.w - 60)).abs() <= GRID, "{after:?}");
        assert_eq!(after.right() % GRID, 0);
    }

    #[test]
    fn a_widget_aligns_with_its_neighbours_and_says_so() {
        let (mut session, mut ui) = editor_session();
        // Make the layout free and push the first name to the right.
        let from = centre(area(&session, &ui, "first_name"));
        drag(&mut session, &mut ui, from, (40, 40));
        let last_name = place(&session, &ui, "last_name");
        let moved = place(&session, &ui, "first_name");
        assert_ne!(moved.x, last_name.x);

        // Dragged back to within a pixel or two of the other field's left
        // edge, it takes that edge exactly, and a guide shows the alignment.
        let from = centre(area(&session, &ui, "first_name"));
        let back = last_name.x - moved.x + 2;
        screen::pointer_down(&mut session, &mut ui, &fonts(), from.0, from.1);
        screen::pointer_move(&mut session, &mut ui, &fonts(), from.0 + back, from.1);
        assert_eq!(place(&session, &ui, "first_name").x, last_name.x);
        assert!(guides(&ui.designer) >= 1);
        let frame = render(&session, &ui);
        snapshot(&frame, "editor-guide");
        let guide_x = area(&session, &ui, "last_name").x;
        let column = Rect::new(guide_x, 30, 1, 240);
        assert!(count_slot(&frame, column, slot::RED) > 10, "a dotted red line");
        screen::pointer_up(&mut session, &mut ui, &fonts(), from.0 + back, from.1);
        assert_eq!(guides(&ui.designer), 0);
        assert_eq!(place(&session, &ui, "first_name").x, last_name.x);

        // With the guides off, it lands on the grid instead.
        let (_, guides_cell) =
            palette_cells().into_iter().find(|(cell, _)| *cell == Cell::Guides).unwrap();
        let (x, y) = centre(guides_cell);
        click(&mut session, &mut ui, x, y);
        assert!(!ui.designer.guides);
        let from = centre(area(&session, &ui, "first_name"));
        drag(&mut session, &mut ui, from, (2, 0));
        assert_eq!(place(&session, &ui, "first_name").x % GRID, 0);
    }

    #[test]
    fn the_keyboard_selects_moves_and_resizes() {
        let (mut session, mut ui) = editor_session();
        // Tab goes through the widgets, down the window.
        press(&mut session, &mut ui, &[Key::Tab]);
        assert_eq!(ui.designer.selected.as_deref(), Some("title"));
        press(&mut session, &mut ui, &[Key::Tab, Key::Tab]);
        let selected = ui.designer.selected.clone().unwrap();
        let before = place(&session, &ui, &selected);

        // The arrows move it pixel by pixel; nothing is sent at each press.
        press(
            &mut session,
            &mut ui,
            &[Key::Right, Key::Right, Key::Right, Key::Down, Key::ShiftRight],
        );
        let moved = place(&session, &ui, &selected);
        assert_eq!((moved.x, moved.y, moved.w), (before.x + 3, before.y + 1, before.w + 1));
        assert_eq!(session.api().count("place_widgets"), 0);

        // Once the keys rest, the move is saved as one change.
        assert!(!screen::tick(&mut session, &mut ui, &fonts()));
        assert!(!screen::tick(&mut session, &mut ui, &fonts()));
        assert!(screen::tick(&mut session, &mut ui, &fonts()));
        assert_eq!(session.api().count("place_widgets"), 1);
        assert_eq!(place(&session, &ui, &selected), moved);
        assert!(!screen::tick(&mut session, &mut ui, &fonts()), "there is nothing left to save");

        // Selecting something else saves at once.
        press(&mut session, &mut ui, &[Key::Left, Key::BackTab]);
        assert_eq!(session.api().count("place_widgets"), 2);
        assert_ne!(ui.designer.selected.as_deref(), Some(selected.as_str()));
    }

    #[test]
    fn the_properties_of_a_field_are_changed_in_their_window() {
        let (mut session, mut ui) = editor_session();
        let (x, y) = centre(area(&session, &ui, "first_name"));
        click(&mut session, &mut ui, x, y);
        let frame = render(&session, &ui);
        snapshot(&frame, "editor-properties");

        // The window keeps away from the selected widget.
        let window = properties_window(session.view(), &ui.designer, &fonts());
        assert!(window.intersection(&area(&session, &ui, "first_name")).is_empty());

        // The label: a click, the text, Enter.
        let (label, shown) = property(session.view(), &ui.designer, &fonts(), Prop::Label).unwrap();
        assert_eq!(shown, "First name");
        click(&mut session, &mut ui, label.x + 10, label.y + 5);
        assert_eq!(is_typing(&ui.designer), Some((Prop::Label, "First name".to_owned())));
        type_text(&mut session, &mut ui, "s");
        press(&mut session, &mut ui, &[Key::Enter]);
        assert_eq!(
            session.api().last(),
            (
                "set_widget",
                json!({ "widget": "first_name", "changes": { "label": "First names" } })
            )
        );
        assert_eq!(is_typing(&ui.designer), None);

        // Required: a click on the box.
        let (required, _) =
            property(session.view(), &ui.designer, &fonts(), Prop::Required).unwrap();
        click(&mut session, &mut ui, required.x + 4, required.y + 5);
        assert_eq!(
            session.api().last(),
            ("set_widget", json!({ "widget": "first_name", "changes": { "required": true } }))
        );

        // The position, as a number; Tab goes on to the next one.
        let before = place(&session, &ui, "first_name");
        let (x_box, _) = property(session.view(), &ui.designer, &fonts(), Prop::X).unwrap();
        click(&mut session, &mut ui, x_box.x + 10, x_box.y + 5);
        press(&mut session, &mut ui, &[Key::Backspace, Key::Backspace, Key::Backspace]);
        type_text(&mut session, &mut ui, "120");
        press(&mut session, &mut ui, &[Key::Tab]);
        assert_eq!(place(&session, &ui, "first_name").x, 120);
        assert_eq!(place(&session, &ui, "first_name").y, before.y);
        assert_eq!(is_typing(&ui.designer).map(|(prop, _)| prop), Some(Prop::Y));
        // What is not a number is refused, and says so.
        type_text(&mut session, &mut ui, "x");
        press(&mut session, &mut ui, &[Key::Enter]);
        assert_eq!(session.message(), Some("A whole number is expected here."));
        // Escape gives up what is being typed.
        press(&mut session, &mut ui, &[Key::Enter]);
        assert!(is_typing(&ui.designer).is_some(), "Enter starts typing the first property");
        press(&mut session, &mut ui, &[Key::Escape]);
        assert_eq!(is_typing(&ui.designer), None);
        assert_eq!(ui.designer.selected.as_deref(), Some("first_name"));
    }

    #[test]
    fn the_properties_window_moves_and_hides() {
        let (mut session, mut ui) = editor_session();
        let window = properties_window(session.view(), &ui.designer, &fonts());
        // Dragged by its title bar, it stays where it is put.
        drag(&mut session, &mut ui, (window.x + 60, window.y + 6), (-120, 40));
        let moved = properties_window(session.view(), &ui.designer, &fonts());
        assert_eq!((moved.x, moved.y), (window.x - 120, window.y + 40));
        assert_eq!(session.api().count("place_widgets"), 0);

        // F7 hides it and shows it again; so does its close box.
        assert_eq!(render(&session, &ui).pixel(moved.x + 2, moved.y + 2), Some(slot::AZURE));
        press(&mut session, &mut ui, &[Key::F(7)]);
        assert!(ui.designer.properties_hidden);
        assert_ne!(render(&session, &ui).pixel(moved.x + 2, moved.y + 2), Some(slot::AZURE));
        press(&mut session, &mut ui, &[Key::F(7)]);
        let close = close_box_of(moved);
        click(&mut session, &mut ui, close.x + 3, close.y + 3);
        assert!(ui.designer.properties_hidden);
    }

    #[test]
    fn a_tool_adds_a_text_or_a_button_where_it_is_clicked() {
        let (mut session, mut ui) = editor_session();
        let (_, text_tool) =
            palette_cells().into_iter().find(|(c, _)| *c == Cell::Tool(Tool::Text)).unwrap();
        let (x, y) = centre(text_tool);
        click(&mut session, &mut ui, x, y);
        assert_eq!(ui.designer.tool, Tool::Text);
        assert_eq!(
            render(&session, &ui).pixel(text_tool.x + 1, text_tool.y + 1),
            Some(slot::YELLOW)
        );

        // A click on the card: the layout is written down first, then the
        // text is added there, and selected.
        let title = area(&session, &ui, "title");
        click(&mut session, &mut ui, title.x + 201, title.bottom() + 71);
        assert_eq!(session.api().count("place_widgets"), 1, "the automatic layout is frozen");
        let (action, input) = session.api().last();
        assert_eq!(action, "add_widget");
        assert_eq!(input["kind"], "text");
        assert_eq!(input["layer"], "background");
        assert_eq!(input["props"]["placement"]["w"], 90);
        assert_eq!(ui.designer.selected.as_deref(), Some("text1"));
        assert_eq!(ui.designer.tool, Tool::Pointer, "the tool is put down");
        let added = place(&session, &ui, "text1");
        assert_eq!((added.x % GRID, added.y % GRID), (0, 0));

        // On the card layer, a button belongs to the card on screen.
        let (_, card_layer) =
            palette_cells().into_iter().find(|(c, _)| *c == Cell::Layer(Layer::Card)).unwrap();
        let (x, y) = centre(card_layer);
        click(&mut session, &mut ui, x, y);
        press(&mut session, &mut ui, &[Key::F(6), Key::Down, Key::Down, Key::Enter]);
        let (action, input) = session.api().last();
        assert_eq!(
            (action, input["kind"].as_str(), input["layer"].as_str()),
            ("add_widget", Some("button"), Some("card"))
        );
        assert_eq!(session.api().actions.last().unwrap().1.card.as_deref(), Some("1"));
        let frame = render(&session, &ui);
        snapshot(&frame, "editor-added");
        // It wears the violet line of what belongs to one card.
        let button = area(&session, &ui, "button2");
        assert!(has_slot(&frame, button.inset(-3), slot::VIOLET));
    }

    #[test]
    fn the_field_tool_goes_through_the_field_dialog() {
        let (mut session, mut ui) = editor_session();
        press(&mut session, &mut ui, &[Key::F(6)]);
        assert!(menu_is_open(&ui.designer));
        let frame = render(&session, &ui);
        snapshot(&frame, "editor-menu");
        press(&mut session, &mut ui, &[Key::Enter]);
        assert!(!menu_is_open(&ui.designer));
        assert_eq!(session.view().stack(), "field");
        assert!(session.view().is_new);

        // Back from the dialog, the editor is still there.
        press(&mut session, &mut ui, &[Key::F(12)]);
        assert_eq!(session.view().stack(), "addressbook");
        assert!(session.view().designing);
    }

    #[test]
    fn delete_asks_before_it_removes() {
        let (mut session, mut ui) = editor_session();
        let (x, y) = centre(area(&session, &ui, "first_name"));
        click(&mut session, &mut ui, x, y);
        press(&mut session, &mut ui, &[Key::Delete]);
        assert!(session.question().is_some());
        assert_eq!(session.api().last(), ("remove_widget", json!({ "widget": "first_name" })));
        // The question is asked over the editor.
        let frame = render(&session, &ui);
        snapshot(&frame, "editor-question");

        press(&mut session, &mut ui, &[Key::Enter]);
        let (_, context) = session.api().actions.last().unwrap();
        assert!(context.confirmed);
        assert_eq!(context.input, Some(json!({ "widget": "first_name" })));
        assert_eq!(ui.designer.selected, None, "what is gone is no longer selected");
        assert_eq!(session.message(), Some("Removed."));
    }

    #[test]
    fn undo_is_a_key_and_a_shortcut() {
        let (mut session, mut ui) = editor_session();
        press(&mut session, &mut ui, &[Key::F(2)]);
        assert_eq!(session.api().last().0, "undo");
        assert_eq!(session.message(), Some("Undone: “city” moved."));
        press(&mut session, &mut ui, &[Key::Undo]);
        assert_eq!(session.api().count("undo"), 2);
        // The key bar of the editor is clickable like any other.
        let (_, _, f2) = chrome::key_slots(&fonts(), KEYS.into_iter())[0];
        click(&mut session, &mut ui, f2.x + 4, f2.y + 4);
        assert_eq!(session.api().count("undo"), 3);
    }

    #[test]
    fn the_editor_leads_to_the_forms_and_back_to_the_stack() {
        let (mut session, mut ui) = editor_session();
        // F4: the author dialog of 2a, over the editor.
        press(&mut session, &mut ui, &[Key::F(4)]);
        assert_eq!(session.view().stack(), "author");
        press(&mut session, &mut ui, &[Key::Escape]);
        assert_eq!(session.view().stack(), "addressbook");
        assert!(session.view().designing);

        // Escape lets go of things one at a time, then leaves the editor.
        let (_, button_tool) =
            palette_cells().into_iter().find(|(c, _)| *c == Cell::Tool(Tool::Button)).unwrap();
        let (x, y) = centre(button_tool);
        click(&mut session, &mut ui, x, y);
        press(&mut session, &mut ui, &[Key::Tab]);
        press(&mut session, &mut ui, &[Key::Escape]);
        assert_eq!(ui.designer.tool, Tool::Pointer);
        assert!(ui.designer.selected.is_some());
        press(&mut session, &mut ui, &[Key::Escape]);
        assert_eq!(ui.designer.selected, None);
        press(&mut session, &mut ui, &[Key::Escape]);
        assert!(!session.view().designing);
        assert_eq!(session.view().stack(), "addressbook");

        // F11 does it in one key; the stack is browsed again, fields and all.
        let (mut session, mut ui) = editor_session();
        press(&mut session, &mut ui, &[Key::F(11)]);
        assert!(!session.view().designing);
        assert_eq!(session.view().focus.as_deref(), Some("last_name"));
        let frame = render(&session, &ui);
        let last_name = area(&session, &ui, "last_name");
        assert!(has_slot(&frame, last_name.inset(2), slot::INK), "the value of the card is back");
    }
}
