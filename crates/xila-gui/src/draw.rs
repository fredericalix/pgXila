//! The drawing editor: a picture, pixel by pixel.
//!
//! Like the visual editor, this is a tool of the client that decides nothing
//! about a stack. It works on a copy of the picture; leaving it sends what
//! was drawn to the server as one action, which keeps it as a new image.

use serde_json::json;
use xila_core::api::{Api, Image, TRANSPARENT};
use xila_core::input::{edit_line, Key};
use xila_core::session::{Drawing, Session};

use crate::chrome::{
    self, close_box_of, content_of, draw_field, draw_window, WindowStyle, FIELD_HEIGHT,
};
use crate::font::Fonts;
use crate::frame::{Frame, Rect, Sprite};
use crate::palette::slot;

/// The keys of the editor.
pub const KEYS: [(&str, &str); 5] =
    [("F2", "Undo"), ("F3", "Quit"), ("F4", "Revert"), ("F6", "Size"), ("F12", "Back")];

// The three windows leave the strip of the messages free, above the keys.
const WINDOW: Rect = Rect::new(5, 19, 304, 249);
const COLOURS: Rect = Rect::new(315, 19, 160, 150);
const PREVIEW: Rect = Rect::new(315, 175, 160, 93);
/// Where the picture is shown, enlarged: room for an icon at eight times its
/// size.
const VIEW: Rect = Rect::new(WINDOW.x + 33, WINDOW.y + 18, 236, 210);
const CELL: i32 = 22;
const SWATCH: i32 = 18;
const SWATCHES_ACROSS: i32 = 8;
const MAX_ZOOM: i32 = 12;
/// How many steps can be undone.
const UNDO_DEPTH: usize = 60;
/// The largest picture the server keeps.
const MAX_SIZE: (i32, i32) = (480, 300);

/// A picture being drawn: one palette slot per pixel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Canvas {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u8>,
}

impl Canvas {
    /// A picture with nothing on it.
    pub fn empty(width: i32, height: i32) -> Self {
        let (width, height) = (width.clamp(1, MAX_SIZE.0), height.clamp(1, MAX_SIZE.1));
        Canvas { width, height, pixels: vec![TRANSPARENT; (width * height) as usize] }
    }

    fn of(image: &Image) -> Self {
        Canvas {
            width: i32::from(image.width),
            height: i32::from(image.height),
            pixels: image.pixels.clone(),
        }
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.width && y < self.height
    }

    pub fn get(&self, x: i32, y: i32) -> u8 {
        if self.contains(x, y) {
            self.pixels[(y * self.width + x) as usize]
        } else {
            TRANSPARENT
        }
    }

    pub fn set(&mut self, x: i32, y: i32, colour: u8) {
        if self.contains(x, y) {
            self.pixels[(y * self.width + x) as usize] = colour;
        }
    }

    /// A straight line between two pixels, both included.
    fn line(&mut self, from: (i32, i32), to: (i32, i32), colour: u8) {
        let (mut x, mut y) = from;
        let (dx, dy) = ((to.0 - x).abs(), -(to.1 - y).abs());
        let (sx, sy) = (if x < to.0 { 1 } else { -1 }, if y < to.1 { 1 } else { -1 });
        let mut error = dx + dy;
        loop {
            self.set(x, y, colour);
            if (x, y) == to {
                break;
            }
            let twice = 2 * error;
            if twice >= dy {
                error += dy;
                x += sx;
            }
            if twice <= dx {
                error += dx;
                y += sy;
            }
        }
    }

    /// A rectangle between two corners, its edges only or filled.
    fn rectangle(&mut self, from: (i32, i32), to: (i32, i32), colour: u8, filled: bool) {
        let area = spanning(from, to);
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                let edge =
                    x == area.x || y == area.y || x == area.right() - 1 || y == area.bottom() - 1;
                if filled || edge {
                    self.set(x, y, colour);
                }
            }
        }
    }

    /// Paints the area of one colour that a pixel belongs to.
    fn fill(&mut self, at: (i32, i32), colour: u8) {
        if !self.contains(at.0, at.1) {
            return;
        }
        let old = self.get(at.0, at.1);
        if old == colour {
            return;
        }
        let mut pending = vec![at];
        while let Some((x, y)) = pending.pop() {
            if self.contains(x, y) && self.get(x, y) == old {
                self.set(x, y, colour);
                pending.extend([(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]);
            }
        }
    }

    /// The pixels of an area, as a picture of their own.
    fn cut(&self, area: Rect) -> Canvas {
        let mut piece = Canvas::empty(area.w, area.h);
        for y in 0..area.h {
            for x in 0..area.w {
                piece.set(x, y, self.get(area.x + x, area.y + y));
            }
        }
        piece
    }

    /// Puts a picture on this one. What is transparent in it changes nothing.
    fn stamp(&mut self, piece: &Canvas, x: i32, y: i32) {
        for py in 0..piece.height {
            for px in 0..piece.width {
                let colour = piece.get(px, py);
                if colour != TRANSPARENT {
                    self.set(x + px, y + py, colour);
                }
            }
        }
    }

    fn clear(&mut self, area: Rect) {
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                self.set(x, y, TRANSPARENT);
            }
        }
    }

    fn flipped(&self, across: bool) -> Canvas {
        let mut result = Canvas::empty(self.width, self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                let (fx, fy) =
                    if across { (self.width - 1 - x, y) } else { (x, self.height - 1 - y) };
                result.set(x, y, self.get(fx, fy));
            }
        }
        result
    }

    /// A quarter of a turn, clockwise.
    fn turned(&self) -> Canvas {
        let mut result = Canvas::empty(self.height, self.width);
        for y in 0..self.height {
            for x in 0..self.width {
                result.set(self.height - 1 - y, x, self.get(x, y));
            }
        }
        result
    }

    /// The same picture on a canvas of another size: what is there stays at
    /// the top left, what no longer fits is lost.
    fn resized(&self, width: i32, height: i32) -> Canvas {
        let mut result = Canvas::empty(width, height);
        result.stamp(self, 0, 0);
        result
    }

    /// The pixels as the text the server reads: two hexadecimal digits each.
    pub fn hex(&self) -> String {
        self.pixels.iter().map(|pixel| format!("{pixel:02x}")).collect()
    }
}

/// The rectangle that has two pixels as opposite corners, both included.
fn spanning(from: (i32, i32), to: (i32, i32)) -> Rect {
    let (x, y) = (from.0.min(to.0), from.1.min(to.1));
    Rect::new(x, y, (from.0 - to.0).abs() + 1, (from.1 - to.1).abs() + 1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Pencil,
    Eraser,
    Fill,
    Line,
    Rectangle,
    Pipette,
    Select,
}

impl Tool {
    const ALL: [Tool; 7] = [
        Tool::Pencil,
        Tool::Eraser,
        Tool::Fill,
        Tool::Line,
        Tool::Rectangle,
        Tool::Pipette,
        Tool::Select,
    ];

    fn name(self) -> &'static str {
        match self {
            Tool::Pencil => "Pencil",
            Tool::Eraser => "Eraser",
            Tool::Fill => "Fill",
            Tool::Line => "Line",
            Tool::Rectangle => "Rectangle",
            Tool::Pipette => "Pipette",
            Tool::Select => "Selection",
        }
    }

    /// The letter that picks the tool.
    fn letter(self) -> char {
        match self {
            Tool::Pencil => 'p',
            Tool::Eraser => 'e',
            Tool::Fill => 'f',
            Tool::Line => 'l',
            Tool::Rectangle => 'r',
            Tool::Pipette => 'i',
            Tool::Select => 's',
        }
    }
}

/// A button of the editor that is not a drawing tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Rectangles filled or not.
    Filled,
    Grid,
    ZoomIn,
    ZoomOut,
    FlipAcross,
    FlipDown,
    Turn,
}

/// A gesture in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Stroke {
    /// The pencil or the eraser: the last pixel reached.
    Freehand { last: (i32, i32) },
    /// A line or a rectangle, from a pixel, over the picture as it was.
    Shape { from: (i32, i32), under: Canvas },
    /// A selection being traced, from a pixel.
    Marquee { from: (i32, i32) },
    /// A selection being moved: where the pointer was last.
    Carry { last: (i32, i32) },
}

/// An area of the picture, and what was lifted from it to be moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub area: Rect,
    /// The pixels, once lifted off the picture: they float over it.
    lifted: Option<Canvas>,
}

/// What is being typed in the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Typing {
    /// A new size for the picture, as "28x26".
    Size { text: String, cursor: usize },
    /// The value of a colour of the stack, as "#e8703a".
    Value { text: String, cursor: usize },
}

/// What the drawing editor remembers between two events.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Painter {
    /// The picture this state is about: stack, target, widget.
    subject: Option<(String, String, Option<String>)>,
    pub canvas: Option<Canvas>,
    /// The picture as it was when the editor was opened.
    original: Option<Canvas>,
    undo: Vec<Canvas>,
    pub tool: Tool,
    /// The colour in hand.
    pub colour: u8,
    pub filled: bool,
    pub grid: bool,
    pub zoom: i32,
    /// The pixel of the picture shown at the top left of the view.
    origin: (i32, i32),
    /// The pixel the keyboard is on, and whether it is the keyboard that
    /// draws at the moment.
    pub cursor: (i32, i32),
    by_keyboard: bool,
    stroke: Option<Stroke>,
    pub selection: Option<Selection>,
    typing: Option<Typing>,
}

/// The largest zoom at which a picture fits in the view.
fn fitting_zoom(canvas: &Canvas) -> i32 {
    (VIEW.w / canvas.width).min(VIEW.h / canvas.height).clamp(1, 8)
}

impl Painter {
    fn canvas(&self) -> &Canvas {
        self.canvas.as_ref().expect("the editor is open on a picture")
    }

    fn canvas_mut(&mut self) -> &mut Canvas {
        self.canvas.as_mut().expect("the editor is open on a picture")
    }

    /// Keeps the picture as it is, to come back to it with Undo.
    fn remember(&mut self) {
        let now = self.canvas().clone();
        if self.undo.last() != Some(&now) {
            self.undo.push(now);
            if self.undo.len() > UNDO_DEPTH {
                self.undo.remove(0);
            }
        }
    }

    /// The picture with the lifted selection on it: what is on screen, and
    /// what is saved.
    pub fn composed(&self) -> Canvas {
        let mut picture = self.canvas().clone();
        if let Some(Selection { area, lifted: Some(piece) }) = &self.selection {
            picture.stamp(piece, area.x, area.y);
        }
        picture
    }

    pub fn changed(&self) -> bool {
        self.original.as_ref() != Some(&self.composed())
    }

    /// Puts a lifted selection down where it is.
    fn drop_selection(&mut self) {
        if let Some(Selection { area, lifted: Some(piece) }) = self.selection.take() {
            self.canvas_mut().stamp(&piece, area.x, area.y);
        }
        self.selection = None;
    }

    /// Lifts the selected pixels off the picture, so that they can be moved.
    fn lift(&mut self) {
        let Some(selection) = &self.selection else { return };
        if selection.lifted.is_some() {
            return;
        }
        let area = selection.area;
        self.remember();
        let piece = self.canvas().cut(area);
        self.canvas_mut().clear(area);
        self.selection = Some(Selection { area, lifted: Some(piece) });
    }

    /// Where the picture is on screen at the current zoom, and the part of
    /// the view it takes.
    fn shown(&self) -> Rect {
        let canvas = self.canvas();
        let w = ((canvas.width - self.origin.0) * self.zoom).min(VIEW.w / self.zoom * self.zoom);
        let h = ((canvas.height - self.origin.1) * self.zoom).min(VIEW.h / self.zoom * self.zoom);
        Rect::new(VIEW.x + (VIEW.w - w) / 2, VIEW.y + (VIEW.h - h) / 2, w, h)
    }

    /// The pixel of the picture under a point of the screen.
    pub fn pixel_at(&self, x: i32, y: i32) -> Option<(i32, i32)> {
        let shown = self.shown();
        shown.contains(x, y).then(|| {
            (self.origin.0 + (x - shown.x) / self.zoom, self.origin.1 + (y - shown.y) / self.zoom)
        })
    }

    /// The square of a pixel of the picture on screen.
    pub fn square_of(&self, pixel: (i32, i32)) -> Rect {
        let shown = self.shown();
        Rect::new(
            shown.x + (pixel.0 - self.origin.0) * self.zoom,
            shown.y + (pixel.1 - self.origin.1) * self.zoom,
            self.zoom,
            self.zoom,
        )
    }

    /// Moves the view so that a pixel is in it.
    fn bring_in_view(&mut self, pixel: (i32, i32)) {
        let across = (VIEW.w / self.zoom).max(1);
        let down = (VIEW.h / self.zoom).max(1);
        let canvas = self.canvas().clone();
        let fit = |origin: i32, at: i32, room: i32, size: i32| {
            let origin = if at < origin {
                at
            } else if at >= origin + room {
                at - room + 1
            } else {
                origin
            };
            origin.clamp(0, (size - room).max(0))
        };
        self.origin = (
            fit(self.origin.0, pixel.0, across, canvas.width),
            fit(self.origin.1, pixel.1, down, canvas.height),
        );
    }

    fn set_zoom(&mut self, zoom: i32) {
        self.zoom = zoom.clamp(1, MAX_ZOOM);
        self.bring_in_view(self.cursor);
    }

    /// Replaces the whole picture, after an operation on all of it.
    fn replace(&mut self, picture: Canvas) {
        self.drop_selection();
        self.remember();
        self.canvas = Some(picture);
        let canvas = self.canvas();
        self.cursor = (self.cursor.0.min(canvas.width - 1), self.cursor.1.min(canvas.height - 1));
        self.zoom = self.zoom.clamp(1, MAX_ZOOM);
        self.origin = (0, 0);
        self.bring_in_view(self.cursor);
    }
}

/// Opens the editor on the picture the session says is being drawn, unless it
/// is already open on it.
fn sync<A: Api>(session: &Session<A>, painter: &mut Painter) -> Option<Drawing> {
    let drawing = session.view().drawing.clone()?;
    let subject =
        (session.view().stack().to_owned(), drawing.target.clone(), drawing.widget.clone());
    if painter.subject.as_ref() != Some(&subject) || painter.canvas.is_none() {
        let canvas = drawing
            .image
            .as_deref()
            .and_then(|id| session.image(id))
            .map(Canvas::of)
            .unwrap_or_else(|| Canvas::empty(i32::from(drawing.width), i32::from(drawing.height)));
        *painter = Painter {
            subject: Some(subject),
            zoom: fitting_zoom(&canvas),
            original: Some(canvas.clone()),
            canvas: Some(canvas),
            colour: slot::INK,
            grid: true,
            filled: painter.filled,
            ..Painter::default()
        };
    }
    Some(drawing)
}

/// The colours a picture may use, in the order of the Colours window: the
/// system colours, then those of the stack unless the picture is an icon.
fn colours<A: Api>(session: &Session<A>, drawing: &Drawing) -> Vec<u8> {
    let mut all: Vec<u8> = session.palette().iter().map(|colour| colour.slot).collect();
    if all.is_empty() {
        all = (0..32).collect();
    }
    if drawing.target != "icon" {
        all.extend(session.view().screen.colours.iter().map(|colour| colour.slot));
    }
    all
}

/// True when the colours of the stack can be changed from this editor.
fn may_tint<A: Api>(session: &Session<A>, drawing: &Drawing) -> bool {
    drawing.target != "icon" && session.view().screen.stack.may_edit
}

// ---------------------------------------------------------------------------
// Where things are
// ---------------------------------------------------------------------------

/// The cells of the tools, on the left of the picture.
pub fn tool_cells() -> Vec<(Tool, Rect)> {
    Tool::ALL
        .into_iter()
        .enumerate()
        .map(|(index, tool)| {
            (tool, Rect::new(WINDOW.x + 6, VIEW.y + index as i32 * (CELL + 1), CELL, CELL))
        })
        .collect()
}

/// The cells of the other buttons: two under the tools, the rest on the
/// right of the picture.
pub fn command_cells() -> Vec<(Command, Rect)> {
    let left = WINDOW.x + 6;
    let right = VIEW.right() + 6;
    let under = VIEW.y + 7 * (CELL + 1) + 4;
    vec![
        (Command::Filled, Rect::new(left, under, CELL, CELL)),
        (Command::Grid, Rect::new(left, under + CELL + 1, CELL, CELL)),
        (Command::ZoomIn, Rect::new(right, VIEW.y, CELL, CELL)),
        (Command::ZoomOut, Rect::new(right, VIEW.y + CELL + 1, CELL, CELL)),
        (Command::FlipAcross, Rect::new(right, VIEW.y + 3 * (CELL + 1) + 6, CELL, CELL)),
        (Command::FlipDown, Rect::new(right, VIEW.y + 4 * (CELL + 1) + 6, CELL, CELL)),
        (Command::Turn, Rect::new(right, VIEW.y + 5 * (CELL + 1) + 6, CELL, CELL)),
    ]
}

/// Where the size of the picture is written, under it: a click changes it.
fn size_area() -> Rect {
    Rect::new(VIEW.x, VIEW.bottom() + 4, 120, FIELD_HEIGHT)
}

/// The swatches of the Colours window: each colour and its square.
pub fn swatches(colours: &[u8]) -> Vec<(u8, Rect)> {
    let content = content_of(COLOURS);
    colours
        .iter()
        .enumerate()
        .map(|(index, colour)| {
            let (column, row) = (index as i32 % SWATCHES_ACROSS, index as i32 / SWATCHES_ACROSS);
            // The colours of the stack are a little apart from the others.
            let gap = if index >= 32 { 3 } else { 0 };
            let area = Rect::new(
                content.x + 7 + column * SWATCH,
                content.y + 6 + row * SWATCH + gap,
                SWATCH - 2,
                SWATCH - 2,
            );
            (*colour, area)
        })
        .collect()
}

/// The squares that add and remove a colour of the stack, after the last one.
fn tint_cells(colours: &[u8]) -> (Rect, Rect) {
    let content = content_of(COLOURS);
    let place = |index: i32| {
        let (column, row) = (index % SWATCHES_ACROSS, index / SWATCHES_ACROSS);
        Rect::new(
            content.x + 7 + column * SWATCH,
            content.y + 9 + row * SWATCH,
            SWATCH - 2,
            SWATCH - 2,
        )
    };
    let next = (colours.len() as i32).max(32);
    (place(next), place(next + 1))
}

/// Where the value of the colour in hand is written.
fn value_area() -> Rect {
    let content = content_of(COLOURS);
    Rect::new(content.x + 44, content.bottom() - 19, 70, FIELD_HEIGHT)
}

// ---------------------------------------------------------------------------
// Acting
// ---------------------------------------------------------------------------

/// Uses the tool on a pixel: the start of a gesture.
fn begin(painter: &mut Painter, at: (i32, i32)) {
    painter.cursor = at;
    match painter.tool {
        Tool::Pencil | Tool::Eraser => {
            painter.remember();
            let colour = if painter.tool == Tool::Eraser { TRANSPARENT } else { painter.colour };
            painter.canvas_mut().set(at.0, at.1, colour);
            painter.stroke = Some(Stroke::Freehand { last: at });
        }
        Tool::Fill => {
            painter.remember();
            let colour = painter.colour;
            painter.canvas_mut().fill(at, colour);
        }
        Tool::Line | Tool::Rectangle => {
            painter.remember();
            painter.stroke = Some(Stroke::Shape { from: at, under: painter.canvas().clone() });
            extend(painter, at);
        }
        Tool::Pipette => {
            // What is picked up is drawn with next. Nothing is picked up from
            // a transparent pixel but the eraser.
            let picked = painter.composed().get(at.0, at.1);
            if picked == TRANSPARENT {
                painter.tool = Tool::Eraser;
            } else {
                painter.colour = picked;
                painter.tool = Tool::Pencil;
            }
        }
        Tool::Select => {
            let inside = painter.selection.as_ref().is_some_and(|s| s.area.contains(at.0, at.1));
            if inside {
                painter.lift();
                painter.stroke = Some(Stroke::Carry { last: at });
            } else {
                painter.drop_selection();
                painter.stroke = Some(Stroke::Marquee { from: at });
            }
        }
    }
}

/// Carries a gesture on to another pixel.
fn extend(painter: &mut Painter, at: (i32, i32)) {
    painter.cursor = at;
    let colour = if painter.tool == Tool::Eraser { TRANSPARENT } else { painter.colour };
    match painter.stroke.clone() {
        Some(Stroke::Freehand { last }) => {
            painter.canvas_mut().line(last, at, colour);
            painter.stroke = Some(Stroke::Freehand { last: at });
        }
        Some(Stroke::Shape { from, under }) => {
            let mut picture = under;
            if painter.tool == Tool::Line {
                picture.line(from, at, colour);
            } else {
                picture.rectangle(from, at, colour, painter.filled);
            }
            painter.canvas = Some(picture);
        }
        Some(Stroke::Marquee { from }) => {
            let bounds = Rect::new(0, 0, painter.canvas().width, painter.canvas().height);
            let area = spanning(from, at).intersection(&bounds);
            painter.selection = Some(Selection { area, lifted: None });
        }
        Some(Stroke::Carry { last }) => {
            if let Some(selection) = &mut painter.selection {
                selection.area = selection.area.moved(at.0 - last.0, at.1 - last.1);
            }
            painter.stroke = Some(Stroke::Carry { last: at });
        }
        None => {}
    }
}

/// Ends a gesture.
fn finish(painter: &mut Painter) {
    // A selection of a single pixel traced by a click selects nothing.
    if let Some(Stroke::Marquee { .. }) = painter.stroke {
        if painter.selection.as_ref().is_some_and(|s| s.area.w * s.area.h <= 1) {
            painter.selection = None;
        }
    }
    painter.stroke = None;
}

fn run(painter: &mut Painter, command: Command) {
    match command {
        Command::Filled => painter.filled = !painter.filled,
        Command::Grid => painter.grid = !painter.grid,
        Command::ZoomIn => painter.set_zoom(painter.zoom + 1),
        Command::ZoomOut => painter.set_zoom(painter.zoom - 1),
        Command::FlipAcross => {
            let picture = painter.composed().flipped(true);
            painter.replace(picture);
        }
        Command::FlipDown => {
            let picture = painter.composed().flipped(false);
            painter.replace(picture);
        }
        Command::Turn => {
            let picture = painter.composed().turned();
            if picture.width <= MAX_SIZE.0 && picture.height <= MAX_SIZE.1 {
                painter.replace(picture);
                let zoom = fitting_zoom(painter.canvas());
                painter.set_zoom(painter.zoom.min(zoom.max(1)));
            }
        }
    }
}

/// Leaves the editor. What was drawn is saved first, as leaving a card saves
/// what was typed on it; if the server refuses, the editor stays open.
fn leave<A: Api>(session: &mut Session<A>, painter: &mut Painter, drawing: &Drawing) -> bool {
    painter.drop_selection();
    if painter.changed() {
        let picture = painter.canvas().clone();
        let saved = session.run_with(
            "save_drawing",
            json!({
                "target": drawing.target,
                "widget": drawing.widget,
                "width": picture.width,
                "height": picture.height,
                "pixels": picture.hex(),
            }),
        );
        if !saved {
            return false;
        }
    }
    session.stop_drawing();
    *painter = Painter { filled: painter.filled, ..Painter::default() };
    true
}

/// Applies what was typed.
fn commit_typing<A: Api>(session: &mut Session<A>, painter: &mut Painter) {
    match painter.typing.take() {
        Some(Typing::Size { text, .. }) => {
            let mut sides = text.split(['x', 'X', ' ']).filter(|part| !part.is_empty());
            let parsed = (
                sides.next().and_then(|side| side.parse::<i32>().ok()),
                sides.next().and_then(|side| side.parse::<i32>().ok()),
            );
            match parsed {
                (Some(w), Some(h))
                    if (1..=MAX_SIZE.0).contains(&w) && (1..=MAX_SIZE.1).contains(&h) =>
                {
                    let picture = painter.composed().resized(w, h);
                    painter.replace(picture);
                    painter.zoom = fitting_zoom(painter.canvas());
                    painter.origin = (0, 0);
                }
                _ => session.say("A size is written like 28x26, up to 480x300."),
            }
        }
        Some(Typing::Value { text, .. }) => {
            session.run_with("set_colour", json!({ "slot": painter.colour, "rgb": text.trim() }));
        }
        None => {}
    }
}

pub fn handle_key<A: Api>(session: &mut Session<A>, painter: &mut Painter, key: Key) {
    let Some(drawing) = sync(session, painter) else { return };

    if painter.typing.is_some() {
        match key {
            Key::Enter => commit_typing(session, painter),
            Key::Escape | Key::F(_) => painter.typing = None,
            _ => {
                if let Some(Typing::Size { text, cursor } | Typing::Value { text, cursor }) =
                    &mut painter.typing
                {
                    edit_line(text, cursor, key, 9);
                }
            }
        }
        return;
    }

    painter.by_keyboard = true;
    let step = |painter: &mut Painter, dx: i32, dy: i32| {
        let canvas = painter.canvas();
        let to = (
            (painter.cursor.0 + dx).clamp(0, canvas.width - 1),
            (painter.cursor.1 + dy).clamp(0, canvas.height - 1),
        );
        // With a selection, the arrows move it; in the middle of a gesture
        // they carry it on; otherwise they move the cursor.
        if painter.selection.is_some() && painter.stroke.is_none() {
            painter.lift();
            if let Some(selection) = &mut painter.selection {
                selection.area = selection.area.moved(dx, dy);
            }
        } else if painter.stroke.is_some() {
            extend(painter, to);
        } else {
            painter.cursor = to;
        }
        painter.bring_in_view(painter.cursor);
    };

    match key {
        Key::F(12) | Key::Escape if painter.stroke.is_some() || painter.selection.is_some() => {
            // Escape lets go of what is in hand first.
            finish(painter);
            painter.drop_selection();
        }
        Key::F(12) | Key::Escape => {
            leave(session, painter, &drawing);
        }
        Key::F(3) => {
            if leave(session, painter, &drawing) {
                session.press("F3");
            }
        }
        Key::F(2) | Key::Undo => {
            finish(painter);
            painter.selection = None;
            if let Some(previous) = painter.undo.pop() {
                painter.canvas = Some(previous);
                painter.bring_in_view((0, 0));
            } else {
                session.say("There is nothing left to undo.");
            }
        }
        Key::F(4) => {
            if let Some(original) = painter.original.clone() {
                painter.replace(original);
                painter.zoom = fitting_zoom(painter.canvas());
            }
        }
        Key::F(6) => {
            let canvas = painter.canvas();
            let text = format!("{}x{}", canvas.width, canvas.height);
            painter.typing = Some(Typing::Size { cursor: text.chars().count(), text });
        }
        Key::F(_) => session.say("This key is not active in the drawing editor."),
        Key::Left => step(painter, -1, 0),
        Key::Right => step(painter, 1, 0),
        Key::Up => step(painter, 0, -1),
        Key::Down => step(painter, 0, 1),
        // The space bar is the button of the pointer, at the cursor: it
        // starts a line, a rectangle or a selection, and ends it.
        Key::Char(' ') => {
            if painter.stroke.is_some() {
                finish(painter);
            } else {
                let at = painter.cursor;
                begin(painter, at);
                if matches!(
                    painter.stroke,
                    Some(Stroke::Freehand { .. }) | Some(Stroke::Carry { .. })
                ) {
                    finish(painter);
                }
            }
        }
        Key::Enter => {
            finish(painter);
            painter.drop_selection();
        }
        Key::Delete | Key::Backspace => {
            if let Some(selection) = painter.selection.take() {
                if selection.lifted.is_none() {
                    painter.remember();
                    painter.canvas_mut().clear(selection.area);
                }
            }
        }
        Key::Char('+') | Key::Char('=') => run(painter, Command::ZoomIn),
        Key::Char('-') => run(painter, Command::ZoomOut),
        Key::Char(letter) => match letter.to_ascii_lowercase() {
            'g' => run(painter, Command::Grid),
            'x' => run(painter, Command::Filled),
            'h' => run(painter, Command::FlipAcross),
            'v' => run(painter, Command::FlipDown),
            't' => run(painter, Command::Turn),
            // A copy of the selection stays where it is; the selection goes on.
            'd' => {
                if painter.selection.is_some() {
                    painter.lift();
                    if let Some(Selection { area, lifted: Some(piece) }) = painter.selection.clone()
                    {
                        painter.canvas_mut().stamp(&piece, area.x, area.y);
                    }
                }
            }
            other => {
                if let Some(tool) = Tool::ALL.into_iter().find(|tool| tool.letter() == other) {
                    if tool != Tool::Select {
                        painter.drop_selection();
                    }
                    painter.tool = tool;
                }
            }
        },
        _ => {}
    }
}

pub fn pointer_down<A: Api>(
    session: &mut Session<A>,
    painter: &mut Painter,
    fonts: &Fonts,
    x: i32,
    y: i32,
) {
    let Some(drawing) = sync(session, painter) else { return };
    painter.by_keyboard = false;

    // The function key bar: a click is the key.
    let key = chrome::key_slots(fonts, KEYS.into_iter())
        .into_iter()
        .find(|(_, _, area)| area.contains(x, y))
        .and_then(|(name, _, _)| name[1..].parse::<u8>().ok());
    if let Some(number) = key {
        painter.typing = None;
        handle_key(session, painter, Key::F(number));
        return;
    }
    if painter.typing.is_some() {
        commit_typing(session, painter);
    }

    if close_box_of(WINDOW).contains(x, y) {
        handle_key(session, painter, Key::F(12));
        return;
    }
    if let Some((tool, _)) = tool_cells().into_iter().find(|(_, area)| area.contains(x, y)) {
        if tool != Tool::Select {
            painter.drop_selection();
        }
        painter.tool = tool;
        return;
    }
    if let Some((command, _)) = command_cells().into_iter().find(|(_, area)| area.contains(x, y)) {
        run(painter, command);
        return;
    }
    if size_area().contains(x, y) {
        handle_key(session, painter, Key::F(6));
        return;
    }

    // The Colours window.
    let all = colours(session, &drawing);
    if let Some((colour, _)) = swatches(&all).into_iter().find(|(_, area)| area.contains(x, y)) {
        painter.colour = colour;
        if matches!(painter.tool, Tool::Eraser | Tool::Pipette) {
            painter.tool = Tool::Pencil;
        }
        return;
    }
    if may_tint(session, &drawing) {
        let (add, remove) = tint_cells(&all);
        if add.contains(x, y) {
            session.run_with("set_colour", json!({ "rgb": "#808080" }));
            // The colour the server added is the one in hand.
            let now = colours(session, &drawing);
            if let Some(new) = now.iter().find(|colour| !all.contains(colour)) {
                painter.colour = *new;
            }
            return;
        }
        if remove.contains(x, y) && painter.colour >= 32 {
            session.run_with("remove_colour", json!({ "slot": painter.colour }));
            painter.colour = slot::INK;
            return;
        }
        if value_area().contains(x, y) && painter.colour >= 32 {
            let text = value_of(session, painter.colour);
            painter.typing = Some(Typing::Value { cursor: text.chars().count(), text });
            return;
        }
    }

    if let Some(pixel) = painter.pixel_at(x, y) {
        begin(painter, pixel);
    }
}

pub fn pointer_move<A: Api>(
    session: &mut Session<A>,
    painter: &mut Painter,
    x: i32,
    y: i32,
) -> bool {
    if sync(session, painter).is_none() || painter.stroke.is_none() {
        return false;
    }
    // Outside the picture, the gesture goes on along its edge.
    let shown = painter.shown();
    let (x, y) = (x.clamp(shown.x, shown.right() - 1), y.clamp(shown.y, shown.bottom() - 1));
    match painter.pixel_at(x, y) {
        Some(pixel) if pixel != painter.cursor => {
            extend(painter, pixel);
            true
        }
        _ => false,
    }
}

pub fn pointer_up<A: Api>(session: &mut Session<A>, painter: &mut Painter, x: i32, y: i32) {
    pointer_move(session, painter, x, y);
    if sync(session, painter).is_some() {
        finish(painter);
    }
}

// ---------------------------------------------------------------------------
// Drawing the editor
// ---------------------------------------------------------------------------

fn tool_icon(tool: Tool) -> Sprite {
    let ink = ('#', slot::INK);
    let rows: &[&str] = match tool {
        Tool::Pencil => &[
            ".........##.",
            "........#aa#",
            ".......#aaa#",
            "......#aaa#.",
            ".....#aaa#..",
            "....#aaa#...",
            "...#aaa#....",
            "..#aaa#.....",
            ".##aa#......",
            ".###........",
            "##..........",
        ],
        Tool::Eraser => &[
            "....#######.",
            "...#rrrrr##.",
            "..#rrrrr#o#.",
            ".#rrrrr#oo#.",
            "#######ooo#.",
            "#ooooo#oo#..",
            "#ooooo#o#...",
            "#ooooo##....",
            "#######.....",
        ],
        Tool::Fill => &[
            "....##......",
            "...#..#.....",
            "..#.##.#....",
            ".#.#oo#.#...",
            "#.#oooo#.#b.",
            ".#oooooo#.b.",
            "..#oooo#..bb",
            "...#oo#...bb",
            "....##.....b",
        ],
        Tool::Line => &[
            ".........##",
            "........##.",
            ".......#...",
            "......#....",
            ".....#.....",
            "....#......",
            "...#.......",
            "..#........",
            ".##........",
            "##.........",
        ],
        Tool::Rectangle => &[
            "############",
            "#gggggggggg#",
            "#gggggggggg#",
            "#gggggggggg#",
            "#gggggggggg#",
            "#gggggggggg#",
            "#gggggggggg#",
            "############",
        ],
        Tool::Pipette => &[
            "........###",
            ".......#bb#",
            "......##b#.",
            ".....#o##..",
            "....#oo#...",
            "...#oo#....",
            "..#oo#.....",
            ".#oo#......",
            ".##........",
            "#..........",
        ],
        Tool::Select => &[
            "##.##.##.##",
            "#.........#",
            "...........",
            "#.........#",
            "#.........#",
            "...........",
            "#.........#",
            "##.##.##.##",
        ],
    };
    Sprite::from_art(
        rows,
        &[
            ink,
            ('a', slot::AMBER),
            ('r', slot::RED),
            ('o', slot::WHITE),
            ('b', slot::BLUE),
            ('g', slot::GREEN),
        ],
    )
}

fn command_icon(command: Command) -> Sprite {
    let rows: &[&str] = match command {
        Command::Filled => &[
            "#########",
            "#ggggggg#",
            "#ggggggg#",
            "#ggggggg#",
            "#ggggggg#",
            "#ggggggg#",
            "#########",
        ],
        Command::Grid => &[
            "#.#.#.#.#",
            ".........",
            "#...#...#",
            ".........",
            "#.#.#.#.#",
            ".........",
            "#...#...#",
            ".........",
            "#.#.#.#.#",
        ],
        Command::ZoomIn => &[
            "...##...", "...##...", "...##...", "########", "########", "...##...", "...##...",
            "...##...",
        ],
        Command::ZoomOut => &["########", "########"],
        Command::FlipAcross => &["..#...#..", ".##...##.", "###.#.###", ".##...##.", "..#...#.."],
        Command::FlipDown => {
            &["..#..", ".###.", "#####", ".....", "..#..", ".....", "#####", ".###.", "..#.."]
        }
        Command::Turn => &[
            "..####...",
            ".#....#.#",
            "#......##",
            "#.....###",
            "#........",
            "#......#.",
            ".#....#..",
            "..####...",
        ],
    };
    Sprite::from_art(rows, &[('#', slot::INK), ('g', slot::GREEN)])
}

fn draw_cell(frame: &mut Frame, area: Rect, on: bool, sprite: &Sprite) {
    frame.fill(area, if on { slot::YELLOW } else { slot::WHITE });
    frame.outline(area, slot::INK);
    frame.sprite(
        sprite,
        area.x + (area.w - sprite.width) / 2,
        area.y + (area.h - sprite.height) / 2,
    );
}

/// The name of a colour, its number and its value, for the Colours window.
fn describe<A: Api>(session: &Session<A>, colour: u8) -> (String, String) {
    let system = session.palette().iter().find(|known| known.slot == colour);
    let stack = session.view().screen.colours.iter().find(|known| known.slot == colour);
    let name = system
        .map(|known| known.name.clone())
        .or_else(|| stack.and_then(|known| known.name.clone()))
        .unwrap_or_else(|| if colour >= 32 { "stack colour".to_owned() } else { String::new() });
    (name, format!("colour n\u{b0} {colour}"))
}

fn value_of<A: Api>(session: &Session<A>, colour: u8) -> String {
    let system = session.palette().iter().find(|known| known.slot == colour).map(|k| k.rgb.clone());
    let stack =
        session.view().screen.colours.iter().find(|k| k.slot == colour).map(|k| k.rgb.clone());
    system.or(stack).unwrap_or_default()
}

/// The title of the window: what is being drawn.
fn title<A: Api>(session: &Session<A>, drawing: &Drawing) -> String {
    let stack = &session.view().screen.stack.title;
    match (drawing.target.as_str(), &drawing.widget) {
        ("icon", _) => format!("Drawing: icon of {stack}"),
        (_, Some(widget)) => format!("Drawing: {widget}"),
        _ => "Drawing".to_owned(),
    }
}

/// Draws the picture at a scale, on a checkerboard where it is transparent.
fn draw_picture(frame: &mut Frame, picture: &Canvas, origin: (i32, i32), area: Rect, zoom: i32) {
    let clip = frame.clip_to(area);
    for y in 0..(area.h / zoom) {
        for x in 0..(area.w / zoom) {
            let (px, py) = (origin.0 + x, origin.1 + y);
            let colour = match picture.get(px, py) {
                TRANSPARENT => {
                    if (px + py) % 2 == 0 {
                        slot::WHITE
                    } else {
                        slot::PEARL
                    }
                }
                colour => colour,
            };
            frame.fill(Rect::new(area.x + x * zoom, area.y + y * zoom, zoom, zoom), colour);
        }
    }
    frame.set_clip(clip);
}

/// Draws the picture at its real size on a ground: transparent pixels show it.
fn draw_real(frame: &mut Frame, picture: &Canvas, area: Rect, scale: i32) {
    let clip = frame.clip_to(area);
    let (w, h) = (picture.width * scale, picture.height * scale);
    let (left, top) = (area.x + (area.w - w) / 2, area.y + (area.h - h) / 2);
    for y in 0..picture.height {
        for x in 0..picture.width {
            let colour = picture.get(x, y);
            if colour != TRANSPARENT {
                frame.fill(Rect::new(left + x * scale, top + y * scale, scale, scale), colour);
            }
        }
    }
    frame.set_clip(clip);
}

/// Draws the editor. The message, the question and the key bar are drawn by
/// the caller, as for any screen.
pub fn draw<A: Api>(
    session: &Session<A>,
    painter: &Painter,
    fonts: &Fonts,
    frame: &mut Frame,
    clock: &str,
    blink: bool,
) {
    let Some(drawing) = session.view().drawing.as_ref() else { return };
    if painter.canvas.is_none() {
        return;
    }
    let picture = painter.composed();

    chrome::draw_desktop(frame);
    chrome::draw_menu_bar(frame, fonts, Some(&session.hello().instance), clock);
    draw_window(frame, fonts, WINDOW, &title(session, drawing), WindowStyle::STACK);

    for (tool, area) in tool_cells() {
        draw_cell(frame, area, painter.tool == tool, &tool_icon(tool));
    }
    for (command, area) in command_cells() {
        let on = match command {
            Command::Filled => painter.filled,
            Command::Grid => painter.grid,
            _ => false,
        };
        draw_cell(frame, area, on, &command_icon(command));
    }

    // The picture, enlarged, in its frame.
    let shown = painter.shown();
    frame.outline(shown.inset(-1), slot::INK);
    draw_picture(frame, &picture, painter.origin, shown, painter.zoom);
    if painter.grid && painter.zoom >= 4 {
        for x in (shown.x..shown.right()).step_by(painter.zoom as usize).skip(1) {
            frame.vline(x, shown.y, shown.h, slot::SILVER);
        }
        for y in (shown.y..shown.bottom()).step_by(painter.zoom as usize).skip(1) {
            frame.hline(shown.x, y, shown.w, slot::SILVER);
        }
    }
    let clip = frame.clip_to(shown);
    if let Some(selection) = &painter.selection {
        let from = painter.square_of((selection.area.x, selection.area.y));
        let area = Rect::new(
            from.x,
            from.y,
            selection.area.w * painter.zoom,
            selection.area.h * painter.zoom,
        );
        for x in area.x..area.right() {
            for y in [area.y, area.bottom() - 1] {
                frame.set(x, y, if (x + y) % 4 < 2 { slot::INK } else { slot::WHITE });
            }
        }
        for y in area.y..area.bottom() {
            for x in [area.x, area.right() - 1] {
                frame.set(x, y, if (x + y) % 4 < 2 { slot::INK } else { slot::WHITE });
            }
        }
    }
    // The cursor of the keyboard: a square around its pixel.
    if painter.by_keyboard && blink {
        let square = painter.square_of(painter.cursor);
        frame.outline(square.inset(-1), slot::INK);
        frame.outline(square, slot::WHITE);
    }
    frame.set_clip(clip);

    // Under the picture: its size, which can be typed, and the zoom.
    let size = size_area();
    match &painter.typing {
        Some(Typing::Size { text, cursor }) => {
            draw_field(frame, fonts, size, text, Some(*cursor), blink)
        }
        _ => {
            let text = format!("{} x {} pixels", picture.width, picture.height);
            fonts.regular.draw(frame, size.x, size.y + 11, &text, slot::COCOA);
        }
    }
    let zoom = format!("x{}", painter.zoom);
    fonts.bold.draw(frame, VIEW.right() - fonts.bold.width(&zoom), size.y + 11, &zoom, slot::GREY);

    // The Colours window.
    draw_window(frame, fonts, COLOURS, "Colours", WindowStyle::SYSTEM);
    let all = colours(session, drawing);
    for (colour, area) in swatches(&all) {
        frame.fill(area, colour);
        frame.outline(area, slot::INK);
        if colour == painter.colour {
            frame.outline(area.inset(-1), slot::WHITE);
            frame.outline(area.inset(-2), slot::INK);
        }
    }
    if may_tint(session, drawing) {
        let (add, remove) = tint_cells(&all);
        for (area, sign) in [(add, "+"), (remove, "-")] {
            frame.outline(area, slot::GREY);
            let x = area.x + (area.w - fonts.bold.width(sign)) / 2;
            fonts.bold.draw(frame, x, area.y + 12, sign, slot::GREY);
        }
    }
    let content = content_of(COLOURS);
    let sample = Rect::new(content.x + 7, content.bottom() - 36, 30, 30);
    frame.fill(sample, painter.colour);
    frame.outline(sample, slot::INK);
    let (name, number) = describe(session, painter.colour);
    fonts.bold.draw(frame, sample.right() + 7, sample.y + 8, &name, slot::INK);
    fonts.regular.draw(frame, sample.right() + 7, sample.y + 18, &number, slot::GREY);
    let value = value_area();
    match &painter.typing {
        Some(Typing::Value { text, cursor }) => {
            draw_field(frame, fonts, value, text, Some(*cursor), blink)
        }
        _ => {
            fonts.regular.draw(
                frame,
                value.x,
                value.y + 11,
                &value_of(session, painter.colour),
                slot::GREY,
            );
        }
    }

    // The Preview window: at its real size on the desktop, enlarged on paper.
    draw_window(frame, fonts, PREVIEW, "Preview", WindowStyle::SYSTEM);
    let content = content_of(PREVIEW).inset(6);
    let half = (content.w - 6) / 2;
    let on_desktop = Rect::new(content.x, content.y, half, content.h);
    let on_paper = Rect::new(content.right() - half, content.y, half, content.h);
    frame.gradient(on_desktop, slot::NIGHT, slot::NAVY);
    frame.outline(on_desktop, slot::INK);
    draw_real(frame, &picture, on_desktop.inset(1), 1);
    frame.fill(on_paper, slot::CREAM);
    frame.outline(on_paper, slot::INK);
    let twice = picture.width * 2 <= half - 2 && picture.height * 2 <= content.h - 2;
    draw_real(frame, &picture, on_paper.inset(1), if twice { 2 } else { 1 });
}

/// What to tell the user about the tool in hand.
pub fn hint(painter: &Painter) -> String {
    let letter = painter.tool.letter().to_ascii_uppercase();
    format!(
        "{} ({letter}). Tools: P E F L R I S. Space draws, + - zoom, G grid, H V T turn.",
        painter.tool.name()
    )
}

/// Makes the state agree with the session: the editor is opened on the
/// picture the server said to draw.
pub fn settle<A: Api>(session: &Session<A>, painter: &mut Painter) {
    if sync(session, painter).is_none() && painter.canvas.is_some() {
        *painter = Painter { filled: painter.filled, ..Painter::default() };
    }
}

/// Test access to what is otherwise private to the editor.
#[cfg(test)]
pub mod probe {
    use super::*;

    pub fn value_area() -> Rect {
        super::value_area()
    }

    pub fn size_area() -> Rect {
        super::size_area()
    }

    pub fn tint_cells<A: Api>(session: &Session<A>) -> (Rect, Rect) {
        let drawing = session.view().drawing.clone().expect("a drawing");
        super::tint_cells(&colours(session, &drawing))
    }

    pub fn swatch_of<A: Api>(session: &Session<A>, colour: u8) -> Rect {
        let drawing = session.view().drawing.clone().expect("a drawing");
        let all = colours(session, &drawing);
        swatches(&all).into_iter().find(|(known, _)| *known == colour).expect("a swatch").1
    }

    pub fn is_typing(painter: &Painter) -> bool {
        painter.typing.is_some()
    }

    pub fn undo_depth(painter: &Painter) -> usize {
        painter.undo.len()
    }

    pub fn window() -> Rect {
        WINDOW
    }
}

#[cfg(test)]
mod tests {
    use super::probe;
    use super::*;
    use crate::screen::{self, Ui};
    use crate::testing::{addressbook_session, count_slot, has_slot, snapshot, Fake};

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

    fn centre(area: Rect) -> (i32, i32) {
        (area.x + area.w / 2, area.y + area.h / 2)
    }

    /// Presses on a pixel of the picture, drags to another, and lets go.
    fn stroke(session: &mut Session<Fake>, ui: &mut Ui, from: (i32, i32), to: (i32, i32)) {
        let start = centre(ui.painter.square_of(from));
        let end = centre(ui.painter.square_of(to));
        screen::pointer_down(session, ui, &fonts(), start.0, start.1);
        screen::pointer_move(session, ui, &fonts(), end.0, end.1);
        screen::pointer_up(session, ui, &fonts(), end.0, end.1);
    }

    fn pick(session: &mut Session<Fake>, ui: &mut Ui, tool: Tool) {
        let (_, cell) = tool_cells().into_iter().find(|(known, _)| *known == tool).unwrap();
        let (x, y) = centre(cell);
        click(session, ui, x, y);
        assert_eq!(ui.painter.tool, tool);
    }

    fn command(session: &mut Session<Fake>, ui: &mut Ui, wanted: Command) {
        let (_, cell) = command_cells().into_iter().find(|(known, _)| *known == wanted).unwrap();
        let (x, y) = centre(cell);
        click(session, ui, x, y);
    }

    fn pixel(ui: &Ui, x: i32, y: i32) -> u8 {
        ui.painter.composed().get(x, y)
    }

    /// The drawing editor, open on the icon of the address book: 28 by 26
    /// transparent pixels.
    fn icon_editor() -> (Session<Fake>, Ui) {
        let (mut session, mut ui) = addressbook_session();
        session.be_graphic();
        session.run_with("draw", json!({ "target": "icon" }));
        screen::settle(&mut session, &mut ui, &fonts());
        assert!(session.view().drawing.is_some());
        (session, ui)
    }

    #[test]
    fn a_click_on_the_picture_of_a_card_opens_the_editor_on_it() {
        let (mut session, mut ui) = addressbook_session();
        session.be_graphic();
        // The photo on the card, found by its red and green first pixels.
        let frame = render(&session, &ui);
        let (x, y) = (0..crate::frame::WIDTH * crate::frame::HEIGHT)
            .map(|at| (at % crate::frame::WIDTH, at / crate::frame::WIDTH))
            .find(|(x, y)| {
                frame.pixel(*x, *y) == Some(slot::RED)
                    && frame.pixel(x + 1, *y) == Some(slot::GREEN)
            })
            .expect("the portrait is on the card");
        click(&mut session, &mut ui, x + 10, y + 10);

        let drawing = session.view().drawing.clone().expect("the editor is open");
        assert_eq!((drawing.target.as_str(), drawing.widget.as_deref()), ("card", Some("photo")));
        // It starts from the portrait: 68 by 80, enlarged as much as fits.
        let canvas = ui.painter.canvas.clone().unwrap();
        assert_eq!((canvas.width, canvas.height), (68, 80));
        assert_eq!((canvas.get(0, 0), canvas.get(1, 0)), (slot::RED, slot::GREEN));
        assert_eq!(ui.painter.zoom, 2);
        let frame = render(&session, &ui);
        snapshot(&frame, "draw-portrait");
        let window = probe::window();
        assert_eq!(frame.pixel(window.x + 2, window.y + 2), Some(slot::AMBER));

        // Nothing was drawn: leaving saves nothing.
        press(&mut session, &mut ui, &[Key::F(12)]);
        assert_eq!(session.view().drawing, None);
        assert!(session.api().drawings.is_empty());
        assert_eq!(session.view().stack(), "addressbook");
    }

    #[test]
    fn enter_on_the_picture_of_a_card_draws_it_too() {
        let (mut session, mut ui) = addressbook_session();
        session.be_graphic();
        session.set_focus(Some("photo".into()));
        press(&mut session, &mut ui, &[Key::Enter]);
        assert!(session.view().drawing.is_some());
    }

    #[test]
    fn the_pencil_draws_and_leaving_saves_the_picture() {
        let (mut session, mut ui) = icon_editor();
        assert_eq!(ui.painter.zoom, 8, "an icon is shown eight times its size");
        assert_eq!(pixel(&ui, 3, 4), TRANSPARENT);

        // A click is a dot, a drag a line.
        stroke(&mut session, &mut ui, (3, 4), (3, 4));
        assert_eq!(pixel(&ui, 3, 4), slot::INK);
        stroke(&mut session, &mut ui, (5, 5), (9, 5));
        for x in 5..=9 {
            assert_eq!(pixel(&ui, x, 5), slot::INK, "{x}");
        }
        assert_eq!(pixel(&ui, 10, 5), TRANSPARENT);

        // Another colour, from the Colours window.
        let (x, y) = centre(probe::swatch_of(&session, slot::RED));
        click(&mut session, &mut ui, x, y);
        assert_eq!(ui.painter.colour, slot::RED);
        stroke(&mut session, &mut ui, (0, 0), (0, 0));
        assert_eq!(pixel(&ui, 0, 0), slot::RED);

        let frame = render(&session, &ui);
        snapshot(&frame, "draw-icon");
        // The picture enlarged, on its checkerboard, and the preview.
        let square = ui.painter.square_of((0, 0));
        assert_eq!(frame.pixel(square.x + 3, square.y + 3), Some(slot::RED));
        assert!(has_slot(&frame, PREVIEW, slot::RED));

        // Leaving saves it, as one picture of 28 by 26.
        press(&mut session, &mut ui, &[Key::F(12)]);
        assert_eq!(session.view().drawing, None);
        assert_eq!(session.message(), Some("Picture saved."));
        let (target, image) = session.api().drawings.last().unwrap().clone();
        assert_eq!((target.as_str(), image.width, image.height), ("icon", 28, 26));
        assert_eq!(image.pixel(0, 0), Some(slot::RED));
        assert_eq!(image.pixel(7, 5), Some(slot::INK));
        assert_eq!(image.pixel(27, 25), Some(TRANSPARENT));
        assert_eq!(ui.painter.canvas, None, "the editor is closed");
    }

    #[test]
    fn a_drawing_the_server_refuses_stays_in_the_editor() {
        let (mut session, mut ui) = icon_editor();
        stroke(&mut session, &mut ui, (1, 1), (1, 1));
        session.api_mut().refuses_drawings = true;
        press(&mut session, &mut ui, &[Key::F(12)]);
        assert!(session.view().drawing.is_some(), "nothing is lost");
        assert_eq!(pixel(&ui, 1, 1), slot::INK);
        assert_eq!(session.message(), Some("An icon is drawn with the 32 system colours only."));
    }

    #[test]
    fn each_tool_does_its_job() {
        let (mut session, mut ui) = icon_editor();

        // A rectangle, outlined then filled.
        pick(&mut session, &mut ui, Tool::Rectangle);
        stroke(&mut session, &mut ui, (2, 2), (8, 6));
        assert_eq!(pixel(&ui, 2, 2), slot::INK);
        assert_eq!(pixel(&ui, 8, 6), slot::INK);
        assert_eq!(pixel(&ui, 5, 4), TRANSPARENT, "only its edges");
        command(&mut session, &mut ui, Command::Filled);
        stroke(&mut session, &mut ui, (12, 2), (15, 4));
        assert_eq!(pixel(&ui, 13, 3), slot::INK);

        // The fill paints the inside of the first rectangle, and stops at it.
        let (x, y) = centre(probe::swatch_of(&session, slot::AMBER));
        click(&mut session, &mut ui, x, y);
        pick(&mut session, &mut ui, Tool::Fill);
        stroke(&mut session, &mut ui, (5, 4), (5, 4));
        assert_eq!(pixel(&ui, 3, 3), slot::AMBER);
        assert_eq!(pixel(&ui, 7, 5), slot::AMBER);
        assert_eq!(pixel(&ui, 2, 2), slot::INK);
        assert_eq!(pixel(&ui, 0, 0), TRANSPARENT);

        // A line, from corner to corner.
        pick(&mut session, &mut ui, Tool::Line);
        stroke(&mut session, &mut ui, (0, 20), (5, 25));
        for step in 0..=5 {
            assert_eq!(pixel(&ui, step, 20 + step), slot::AMBER);
        }

        // The pipette takes a colour from the picture, and hands the pencil.
        pick(&mut session, &mut ui, Tool::Pipette);
        stroke(&mut session, &mut ui, (2, 2), (2, 2));
        assert_eq!((ui.painter.colour, ui.painter.tool), (slot::INK, Tool::Pencil));

        // The eraser makes pixels transparent again.
        pick(&mut session, &mut ui, Tool::Eraser);
        stroke(&mut session, &mut ui, (2, 2), (4, 2));
        assert_eq!(pixel(&ui, 3, 2), TRANSPARENT);
        assert_eq!(pixel(&ui, 5, 2), slot::INK);
        let frame = render(&session, &ui);
        snapshot(&frame, "draw-tools");
    }

    #[test]
    fn undo_goes_back_step_by_step_and_revert_all_the_way() {
        let (mut session, mut ui) = icon_editor();
        stroke(&mut session, &mut ui, (1, 1), (1, 1));
        stroke(&mut session, &mut ui, (2, 2), (6, 2));
        press(&mut session, &mut ui, &[Key::F(2)]);
        assert_eq!(pixel(&ui, 4, 2), TRANSPARENT);
        assert_eq!(pixel(&ui, 1, 1), slot::INK);
        press(&mut session, &mut ui, &[Key::Undo]);
        assert_eq!(pixel(&ui, 1, 1), TRANSPARENT);
        press(&mut session, &mut ui, &[Key::F(2)]);
        assert_eq!(session.message(), Some("There is nothing left to undo."));

        // Revert goes back to the picture as it was opened, and can be undone.
        stroke(&mut session, &mut ui, (1, 1), (9, 9));
        press(&mut session, &mut ui, &[Key::F(4)]);
        assert!(!ui.painter.changed());
        press(&mut session, &mut ui, &[Key::F(2)]);
        assert_eq!(pixel(&ui, 9, 9), slot::INK);
    }

    #[test]
    fn a_selection_is_moved_duplicated_and_erased() {
        let (mut session, mut ui) = icon_editor();
        command(&mut session, &mut ui, Command::Filled);
        pick(&mut session, &mut ui, Tool::Rectangle);
        stroke(&mut session, &mut ui, (2, 2), (4, 4));

        // Tracing an area selects it; dragging it moves what is in it.
        pick(&mut session, &mut ui, Tool::Select);
        stroke(&mut session, &mut ui, (1, 1), (5, 5));
        assert_eq!(ui.painter.selection.as_ref().map(|s| s.area), Some(Rect::new(1, 1, 5, 5)));
        let frame = render(&session, &ui);
        snapshot(&frame, "draw-selection");
        stroke(&mut session, &mut ui, (3, 3), (13, 6));
        assert_eq!(pixel(&ui, 3, 3), TRANSPARENT, "it left where it was");
        assert_eq!(pixel(&ui, 13, 6), slot::INK);
        assert_eq!(pixel(&ui, 12, 5), slot::INK);

        // The arrows move it too; D leaves a copy behind.
        press(&mut session, &mut ui, &[Key::Char('d'), Key::Down, Key::Down, Key::Down, Key::Down]);
        assert_eq!(pixel(&ui, 13, 6), slot::INK, "the copy");
        assert_eq!(pixel(&ui, 13, 10), slot::INK, "the selection, four pixels lower");
        // Enter puts it down.
        press(&mut session, &mut ui, &[Key::Enter]);
        assert_eq!(ui.painter.selection, None);
        assert_eq!(pixel(&ui, 13, 10), slot::INK);

        // Delete erases what is selected.
        stroke(&mut session, &mut ui, (11, 4), (15, 8));
        press(&mut session, &mut ui, &[Key::Delete]);
        assert_eq!(pixel(&ui, 13, 6), TRANSPARENT);
        assert_eq!(pixel(&ui, 13, 10), slot::INK, "outside the selection");
        // A click that traces nothing selects nothing.
        stroke(&mut session, &mut ui, (20, 20), (20, 20));
        assert_eq!(ui.painter.selection, None);
    }

    #[test]
    fn the_whole_picture_is_flipped_turned_and_resized() {
        let (mut session, mut ui) = icon_editor();
        stroke(&mut session, &mut ui, (0, 0), (0, 0));

        command(&mut session, &mut ui, Command::FlipAcross);
        assert_eq!((pixel(&ui, 0, 0), pixel(&ui, 27, 0)), (TRANSPARENT, slot::INK));
        command(&mut session, &mut ui, Command::FlipDown);
        assert_eq!(pixel(&ui, 27, 25), slot::INK);
        // A quarter of a turn: the picture is now 26 by 28.
        command(&mut session, &mut ui, Command::Turn);
        let canvas = ui.painter.composed();
        assert_eq!((canvas.width, canvas.height), (26, 28));
        assert_eq!(pixel(&ui, 0, 27), slot::INK);

        // Another size, typed: what is drawn stays at the top left.
        press(&mut session, &mut ui, &[Key::F(6)]);
        assert!(probe::is_typing(&ui.painter));
        for _ in 0..5 {
            press(&mut session, &mut ui, &[Key::Backspace]);
        }
        type_text(&mut session, &mut ui, "32x40");
        press(&mut session, &mut ui, &[Key::Enter]);
        let canvas = ui.painter.composed();
        assert_eq!((canvas.width, canvas.height), (32, 40));
        assert_eq!(pixel(&ui, 0, 27), slot::INK);
        assert_eq!(pixel(&ui, 31, 39), TRANSPARENT);
        // What is not a size is refused.
        press(&mut session, &mut ui, &[Key::F(6)]);
        type_text(&mut session, &mut ui, "0");
        press(&mut session, &mut ui, &[Key::Enter]);
        assert_eq!(session.message(), Some("A size is written like 28x26, up to 480x300."));
        // Each of these can be undone.
        press(&mut session, &mut ui, &[Key::F(2)]);
        assert_eq!(ui.painter.composed().width, 26);
    }

    #[test]
    fn the_keyboard_draws_too() {
        let (mut session, mut ui) = icon_editor();
        // The arrows move the cursor, the space bar draws there.
        press(&mut session, &mut ui, &[Key::Right, Key::Right, Key::Down, Key::Char(' ')]);
        assert_eq!(ui.painter.cursor, (2, 1));
        assert_eq!(pixel(&ui, 2, 1), slot::INK);
        let frame = render(&session, &ui);
        let square = ui.painter.square_of((2, 1));
        assert_eq!(frame.pixel(square.x - 1, square.y - 1), Some(slot::INK), "the cursor");

        // A line: space starts it, the arrows stretch it, space ends it.
        press(&mut session, &mut ui, &[Key::Char('l'), Key::Down, Key::Down, Key::Char(' ')]);
        assert_eq!(ui.painter.tool, Tool::Line);
        press(&mut session, &mut ui, &[Key::Right, Key::Right, Key::Right, Key::Char(' ')]);
        for x in 2..=5 {
            assert_eq!(pixel(&ui, x, 3), slot::INK, "{x}");
        }
        // Letters pick tools and commands; + and - change the zoom.
        press(&mut session, &mut ui, &[Key::Char('E'), Key::Char('g'), Key::Char('-')]);
        assert_eq!(ui.painter.tool, Tool::Eraser);
        assert!(!ui.painter.grid);
        assert_eq!(ui.painter.zoom, 7);
        press(&mut session, &mut ui, &[Key::Char('+'), Key::Char('+')]);
        assert_eq!(ui.painter.zoom, 9);
        // Zoomed in beyond the view, the cursor stays in sight.
        press(&mut session, &mut ui, &[Key::Char('+'), Key::Char('+'), Key::Char('+')]);
        for _ in 0..30 {
            press(&mut session, &mut ui, &[Key::Right, Key::Down]);
        }
        assert_eq!(ui.painter.cursor, (27, 25));
        let square = ui.painter.square_of(ui.painter.cursor);
        assert!(VIEW.contains(square.x, square.y));
        assert!(VIEW.contains(square.right() - 1, square.bottom() - 1));
    }

    #[test]
    fn a_stack_gets_colours_of_its_own_from_the_colours_window() {
        // A picture of a card may use the colours of its stack.
        let (mut session, mut ui) = addressbook_session();
        session.be_graphic();
        session.run_with("draw", json!({ "target": "card", "widget": "photo" }));
        screen::settle(&mut session, &mut ui, &fonts());

        // "+" adds a colour, which is then in hand.
        let (add, _) = probe::tint_cells(&session);
        let (x, y) = centre(add);
        click(&mut session, &mut ui, x, y);
        assert_eq!(ui.painter.colour, 32);
        assert_eq!(session.view().screen.colours.len(), 1);
        // Its value is typed.
        let value = probe::value_area();
        click(&mut session, &mut ui, value.x + 4, value.y + 5);
        assert!(probe::is_typing(&ui.painter));
        for _ in 0..7 {
            press(&mut session, &mut ui, &[Key::Backspace]);
        }
        type_text(&mut session, &mut ui, "#ab12cd");
        press(&mut session, &mut ui, &[Key::Enter]);
        assert_eq!(session.api().tints[&32], "#ab12cd");
        let frame = render(&session, &ui);
        snapshot(&frame, "draw-colours");
        assert!(count_slot(&frame, probe::swatch_of(&session, 32), 32) > 100);

        // "-" removes the colour in hand.
        let (_, remove) = probe::tint_cells(&session);
        let (x, y) = centre(remove);
        click(&mut session, &mut ui, x, y);
        assert!(session.view().screen.colours.is_empty());
        assert_eq!(ui.painter.colour, slot::INK);
    }

    #[test]
    fn an_icon_only_has_the_colours_of_the_system() {
        let (mut session, mut ui) = icon_editor();
        // The stack has a colour of its own, which the screen knows about.
        session.api_mut().tints.insert(40, "#102030".into());
        session.run("refresh");
        assert_eq!(session.view().screen.colours.len(), 1);
        let drawing = session.view().drawing.clone().unwrap();
        assert_eq!(colours(&session, &drawing).len(), 32);
        // Nor is a colour added from here.
        let before = session.api().actions.len();
        let (add, _) = probe::tint_cells(&session);
        let (x, y) = centre(add);
        click(&mut session, &mut ui, x, y);
        assert_eq!(session.api().actions.len(), before);
    }

    #[test]
    fn the_list_of_stacks_shows_their_icons() {
        let mut session = Session::start(Fake::default()).unwrap();
        let mut ui = Ui::default();
        press(&mut session, &mut ui, &[Key::Enter]);
        assert_eq!(session.view().stack(), "stacks");
        let frame = render(&session, &ui);
        snapshot(&frame, "stacks-icons");
        // The first stack has an icon of its own, drawn twice its size; the
        // second gets the generic one. Both have their title under them.
        assert!(count_slot(&frame, Rect::new(0, 15, 240, 200), slot::AMBER) > 2000);
        // The arrows go from one to the other, and Enter opens it.
        press(&mut session, &mut ui, &[Key::Right]);
        assert_eq!(session.view().grids["list"].selected, 1);
        press(&mut session, &mut ui, &[Key::Left, Key::Enter]);
        assert_eq!(session.view().stack(), "addressbook");
    }
}
