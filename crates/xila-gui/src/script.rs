//! The script editor: the Lua source of a button, or of a screen, is typed,
//! tested and saved.
//!
//! It is a tool of the graphical client, as the visual editor and the drawing
//! editor are. It knows how to edit text and how to colour Lua; what a script
//! means is the business of the server, which checks it, tests it and saves
//! it through the actions `test_script` and `save_script`.

use serde_json::json;
use xila_core::api::Api;
use xila_core::input::Key;
use xila_core::session::{Scripting, Session};

use crate::chrome::{self, close_box_of, draw_window, ButtonLook, WindowStyle, BUTTON_HEIGHT};
use crate::font::Fonts;
use crate::frame::{Frame, Rect};
use crate::palette::slot;

/// The function keys of the editor.
pub const KEYS: [(&str, &str); 3] = [("F3", "Quit"), ("F5", "Test"), ("F12", "Back")];

const WINDOW: Rect = Rect::new(5, 19, 326, 249);
const EVENTS: Rect = Rect::new(337, 19, 138, 104);
const REPORT: Rect = Rect::new(337, 129, 138, 139);

/// Height of a line of source, and how many are shown.
const LINE: i32 = 11;
const LINES: usize = 17;
/// Where the source is written, with the line numbers on its left.
const CODE: Rect = Rect::new(WINDOW.x + 6, WINDOW.y + 18, WINDOW.w - 12, LINE * LINES as i32);
const GUTTER: i32 = 22;
/// Where the text of the source itself goes.
const TEXT: Rect = Rect::new(CODE.x + GUTTER + 4, CODE.y, CODE.w - GUTTER - 6, CODE.h);
const STATUS_BASELINE: i32 = CODE.y + CODE.h + 13;
const BUTTONS_Y: i32 = WINDOW.y + WINDOW.h - BUTTON_HEIGHT - 7;
/// Height of a row of the list of events.
const ROW: i32 = 13;

const INDENT: &str = "  ";
const UNDO_DEPTH: usize = 200;

// ---------------------------------------------------------------------------
// The text
// ---------------------------------------------------------------------------

/// A place in the text: line, then column, both counted from zero. Columns
/// count characters.
pub type Place = (usize, usize);

/// The text being edited, the cursor and the selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Buffer {
    /// The lines of the text; there is always at least one.
    lines: Vec<String>,
    pub cursor: Place,
    /// Where the selection started, when there is one.
    anchor: Option<Place>,
}

impl Default for Buffer {
    fn default() -> Self {
        Buffer { lines: vec![String::new()], cursor: (0, 0), anchor: None }
    }
}

fn byte_at(line: &str, column: usize) -> usize {
    line.char_indices().nth(column).map_or(line.len(), |(index, _)| index)
}

fn length(line: &str) -> usize {
    line.chars().count()
}

/// Text as the editor keeps it: lines ending with a line feed only, and
/// spaces where there were tabulations.
fn clean(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n").replace('\t', INDENT)
}

impl Buffer {
    pub fn of(text: &str) -> Self {
        Buffer {
            lines: clean(text).split('\n').map(str::to_owned).collect(),
            cursor: (0, 0),
            anchor: None,
        }
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// The selection, from its first place to its last; None when nothing
    /// is selected.
    pub fn selection(&self) -> Option<(Place, Place)> {
        let anchor = self.anchor.filter(|anchor| *anchor != self.cursor)?;
        Some((anchor.min(self.cursor), anchor.max(self.cursor)))
    }

    pub fn selected_text(&self) -> Option<String> {
        let ((first, from), (last, to)) = self.selection()?;
        if first == last {
            let line = &self.lines[first];
            return Some(line[byte_at(line, from)..byte_at(line, to)].to_owned());
        }
        let mut text = self.lines[first][byte_at(&self.lines[first], from)..].to_owned();
        for line in &self.lines[first + 1..last] {
            text.push('\n');
            text.push_str(line);
        }
        text.push('\n');
        text.push_str(&self.lines[last][..byte_at(&self.lines[last], to)]);
        Some(text)
    }

    fn drop_selection(&mut self) {
        self.anchor = None;
    }

    /// Removes what is selected. Returns false when nothing was.
    fn delete_selection(&mut self) -> bool {
        let Some(((first, from), (last, to))) = self.selection() else {
            self.anchor = None;
            return false;
        };
        let tail = self.lines[last][byte_at(&self.lines[last], to)..].to_owned();
        let cut = byte_at(&self.lines[first], from);
        self.lines[first].truncate(cut);
        self.lines[first].push_str(&tail);
        self.lines.drain(first + 1..=last);
        self.cursor = (first, from);
        self.anchor = None;
        true
    }

    /// Writes a text at the cursor, in place of what is selected.
    pub fn insert(&mut self, text: &str) {
        self.delete_selection();
        let (line, column) = self.cursor;
        let at = byte_at(&self.lines[line], column);
        let tail = self.lines[line].split_off(at);
        let text = clean(text);
        let mut parts = text.split('\n');
        self.lines[line].push_str(parts.next().unwrap_or_default());
        let mut last = line;
        for part in parts {
            last += 1;
            self.lines.insert(last, part.to_owned());
        }
        self.cursor = (last, length(&self.lines[last]));
        self.lines[last].push_str(&tail);
    }

    /// Writes a character that was typed. A word that closes a block, typed
    /// alone on its line, goes back one step: `end` lines up with what it
    /// ends.
    fn typed(&mut self, character: char) {
        self.insert(character.encode_utf8(&mut [0; 4]));
        let (line, column) = self.cursor;
        let written = &self.lines[line];
        let closes = ["end", "else", "elseif", "until", "}"].contains(&written.trim_start());
        let indented_like_above = line > 0
            && written.starts_with(INDENT)
            && written.len() - written.trim_start().len()
                >= self.lines[line - 1].len() - self.lines[line - 1].trim_start().len();
        // "elseif" was already moved back when it was "else".
        let moved_already = written.trim_start() == "elseif";
        if closes && column == length(written) && indented_like_above && !moved_already {
            self.lines[line].replace_range(..INDENT.len(), "");
            self.cursor.1 -= INDENT.len();
        }
    }

    /// Starts a new line, indented like the one it comes from, and one step
    /// further after a line that opens a block.
    fn enter(&mut self) {
        self.delete_selection();
        let (line, column) = self.cursor;
        let before: String = self.lines[line].chars().take(column).collect();
        let mut indent: String = before.chars().take_while(|c| *c == ' ').collect();
        let opens = ["then", "do", "else", "repeat", "{", "("];
        let trimmed = before.trim_end();
        if opens.iter().any(|word| trimmed.ends_with(word))
            || (trimmed.trim_start().starts_with("function") && trimmed.ends_with(')'))
        {
            indent.push_str(INDENT);
        }
        self.insert(&format!("\n{indent}"));
    }

    fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }
        let (line, column) = self.cursor;
        if column > 0 {
            // The spaces of an indentation go a step at a time.
            let before: String = self.lines[line].chars().take(column).collect();
            let step = if before.ends_with(INDENT) && before.trim().is_empty() { 2 } else { 1 };
            let from = byte_at(&self.lines[line], column - step);
            let to = byte_at(&self.lines[line], column);
            self.lines[line].replace_range(from..to, "");
            self.cursor = (line, column - step);
        } else if line > 0 {
            let tail = self.lines.remove(line);
            self.cursor = (line - 1, length(&self.lines[line - 1]));
            self.lines[line - 1].push_str(&tail);
        }
    }

    fn delete(&mut self) {
        if self.delete_selection() {
            return;
        }
        let (line, column) = self.cursor;
        if column < length(&self.lines[line]) {
            let at = byte_at(&self.lines[line], column);
            self.lines[line].remove(at);
        } else if line + 1 < self.lines.len() {
            let tail = self.lines.remove(line + 1);
            self.lines[line].push_str(&tail);
        }
    }

    /// The lines the selection touches, or the line of the cursor.
    fn touched(&self) -> std::ops::RangeInclusive<usize> {
        match self.selection() {
            Some(((first, _), (last, to))) if to == 0 && last > first => first..=last - 1,
            Some(((first, _), (last, _))) => first..=last,
            None => self.cursor.0..=self.cursor.0,
        }
    }

    /// Moves the touched lines one step to the right, or to the left.
    fn shift(&mut self, right: bool) {
        for line in self.touched() {
            let removed = if right {
                self.lines[line].insert_str(0, INDENT);
                -(INDENT.len() as isize)
            } else {
                let spaces = self.lines[line].chars().take(2).take_while(|c| *c == ' ').count();
                self.lines[line].replace_range(..spaces, "");
                spaces as isize
            };
            for place in [Some(&mut self.cursor), self.anchor.as_mut()].into_iter().flatten() {
                if place.0 == line {
                    place.1 = place.1.saturating_add_signed(-removed);
                }
            }
        }
    }

    /// Puts the cursor somewhere; the selection follows when `selecting`.
    pub fn move_to(&mut self, place: Place, selecting: bool) {
        let line = place.0.min(self.lines.len() - 1);
        let column = place.1.min(length(&self.lines[line]));
        match (selecting, self.anchor) {
            (true, None) => self.anchor = Some(self.cursor),
            (false, _) => self.anchor = None,
            _ => {}
        }
        self.cursor = (line, column);
    }

    fn step(&mut self, key: Key) -> bool {
        let (line, column) = self.cursor;
        let last = self.lines.len() - 1;
        let (place, selecting) = match key {
            Key::Left | Key::ShiftLeft => {
                // Without the shift key, an arrow first lets go of the selection.
                if let (Key::Left, Some((first, _))) = (key, self.selection()) {
                    (first, false)
                } else if column > 0 {
                    ((line, column - 1), key == Key::ShiftLeft)
                } else if line > 0 {
                    ((line - 1, usize::MAX), key == Key::ShiftLeft)
                } else {
                    ((line, column), key == Key::ShiftLeft)
                }
            }
            Key::Right | Key::ShiftRight => {
                if let (Key::Right, Some((_, end))) = (key, self.selection()) {
                    (end, false)
                } else if column < length(&self.lines[line]) {
                    ((line, column + 1), key == Key::ShiftRight)
                } else if line < last {
                    ((line + 1, 0), key == Key::ShiftRight)
                } else {
                    ((line, column), key == Key::ShiftRight)
                }
            }
            Key::Up | Key::ShiftUp => ((line.saturating_sub(1), column), key == Key::ShiftUp),
            Key::Down | Key::ShiftDown => ((line + 1, column), key == Key::ShiftDown),
            Key::PageUp => ((line.saturating_sub(LINES - 1), column), false),
            Key::PageDown => ((line + LINES - 1, column), false),
            Key::Home => {
                // First to where the line starts being written, then to its edge.
                let written = self.lines[line].chars().take_while(|c| *c == ' ').count();
                ((line, if column == written { 0 } else { written }), false)
            }
            Key::End => ((line, usize::MAX), false),
            _ => return false,
        };
        self.move_to(place, selecting);
        true
    }

    fn select_all(&mut self) {
        let last = self.lines.len() - 1;
        self.anchor = Some((0, 0));
        self.cursor = (last, length(&self.lines[last]));
    }
}

// ---------------------------------------------------------------------------
// Colours
// ---------------------------------------------------------------------------

/// What a piece of source is, for its colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ink {
    Plain,
    Comment,
    Text,
    Number,
    Keyword,
    /// A name pgXila gives to scripts, or one it looks for in them.
    Known,
}

impl Ink {
    fn slot(self) -> u8 {
        match self {
            Ink::Plain => slot::CREAM,
            Ink::Comment => slot::GREY,
            Ink::Text => slot::GREEN,
            Ink::Number => slot::AMBER,
            Ink::Keyword => slot::AZURE,
            Ink::Known => slot::YELLOW,
        }
    }
}

/// What a line leaves open for the next one: a long comment or a long
/// string, with the number of equal signs that closes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Open {
    #[default]
    Nothing,
    Comment(usize),
    Text(usize),
}

const KEYWORDS: [&str; 22] = [
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

const KNOWN: [&str; 33] = [
    "card", "stack", "save", "delete", "go_to", "open", "focus", "message", "confirm", "act",
    "query", "execute", "today", "now", "user", "instance", "print", "pcall", "error", "assert",
    "pairs", "ipairs", "tostring", "tonumber", "type", "string", "table", "math", "utf8", "keys",
    "on_click", "on_key", "on_open",
];

/// The level of a long bracket that opens at the start of `text`: `[[` is 0,
/// `[=[` is 1.
fn long_bracket(text: &[char]) -> Option<usize> {
    if text.first() != Some(&'[') {
        return None;
    }
    let level = text[1..].iter().take_while(|c| **c == '=').count();
    (text.get(1 + level) == Some(&'[')).then_some(level)
}

/// Where a long bracket of this level closes in `text`: the place after it.
fn long_close(text: &[char], level: usize) -> Option<usize> {
    (0..text.len()).find_map(|at| {
        let closes = text[at] == ']'
            && text[at + 1..].iter().take(level).filter(|c| **c == '=').count() == level
            && text.get(at + 1 + level) == Some(&']');
        closes.then_some(at + level + 2)
    })
}

/// Cuts a line of Lua into pieces of one colour each: how many characters,
/// and what they are. `open` is what the lines before left open.
pub fn colour(line: &str, open: &mut Open) -> Vec<(usize, Ink)> {
    let text: Vec<char> = line.chars().collect();
    let mut pieces = Vec::new();
    let mut at = 0;

    // The rest of a long comment or string started above.
    if let Open::Comment(level) | Open::Text(level) = *open {
        let ink = if matches!(open, Open::Comment(_)) { Ink::Comment } else { Ink::Text };
        match long_close(&text, level) {
            Some(end) => {
                pieces.push((end, ink));
                at = end;
                *open = Open::Nothing;
            }
            None => return vec![(text.len(), ink)],
        }
    }

    while at < text.len() {
        let rest = &text[at..];
        let c = rest[0];
        let (taken, ink) = if rest.starts_with(&['-', '-']) {
            match long_bracket(&rest[2..]) {
                Some(level) => match long_close(&rest[2..], level) {
                    Some(end) => (2 + end, Ink::Comment),
                    None => {
                        *open = Open::Comment(level);
                        (rest.len(), Ink::Comment)
                    }
                },
                None => (rest.len(), Ink::Comment),
            }
        } else if let Some(level) = long_bracket(rest) {
            match long_close(&rest[level + 2..], level) {
                Some(end) => (level + 2 + end, Ink::Text),
                None => {
                    *open = Open::Text(level);
                    (rest.len(), Ink::Text)
                }
            }
        } else if c == '"' || c == '\'' {
            let mut end = 1;
            while end < rest.len() && rest[end] != c {
                end += if rest[end] == '\\' { 2 } else { 1 };
            }
            ((end + 1).min(rest.len()), Ink::Text)
        } else if c.is_ascii_digit() {
            let end = rest.iter().take_while(|c| c.is_ascii_alphanumeric() || **c == '.').count();
            (end, Ink::Number)
        } else if c.is_alphabetic() || c == '_' {
            let end = rest.iter().take_while(|c| c.is_alphanumeric() || **c == '_').count();
            let word: String = rest[..end].iter().collect();
            let ink = if KEYWORDS.contains(&word.as_str()) {
                Ink::Keyword
            } else if KNOWN.contains(&word.as_str()) {
                Ink::Known
            } else {
                Ink::Plain
            };
            (end, ink)
        } else {
            (1, Ink::Plain)
        };
        // Neighbours of the same colour are one piece.
        match pieces.last_mut() {
            Some((count, last)) if *last == ink => *count += taken,
            _ => pieces.push((taken, ink)),
        }
        at += taken;
    }
    pieces
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// An event a script may handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Click,
    Key,
    Open,
}

impl Event {
    fn label(self) -> &'static str {
        match self {
            Event::Click => "click",
            Event::Key => "key",
            Event::Open => "open card",
        }
    }

    /// The function that handles it.
    fn function(self) -> &'static str {
        match self {
            Event::Click => "on_click",
            Event::Key => "on_key",
            Event::Open => "on_open",
        }
    }

    /// What the server is told to test it.
    fn name(self) -> &'static str {
        match self {
            Event::Click => "click",
            Event::Key => "key",
            Event::Open => "open",
        }
    }

    /// An empty function for it, to start from.
    fn skeleton(self) -> &'static str {
        match self {
            Event::Click => "function on_click()\n  \nend",
            Event::Key => "keys = { F6 = \"Label\" }\n\nfunction on_key(key)\n  \nend",
            Event::Open => "function on_open()\n  \nend",
        }
    }
}

/// The events that are listed and cannot be handled yet.
const LATER: [&str; 3] = ["timer", "database change", "message received"];

/// The events the script of a button, or of a screen, may handle.
fn events_of(scripting: &Scripting) -> [Event; 2] {
    if scripting.widget.is_some() {
        [Event::Click, Event::Key]
    } else {
        [Event::Key, Event::Open]
    }
}

// ---------------------------------------------------------------------------
// The editor
// ---------------------------------------------------------------------------

/// What the script editor remembers between two events.
#[derive(Debug, Default)]
pub struct Scripter {
    /// The script this state is about: stack and button.
    subject: Option<(String, Option<String>)>,
    pub buffer: Buffer,
    /// The text as it was when the editor opened: what is saved.
    saved: String,
    undo: Vec<Buffer>,
    redo: Vec<Buffer>,
    /// True while characters are typed one after the other: they are undone
    /// together.
    typing: bool,
    /// The first line shown, and how far the text is pushed to the left.
    top: usize,
    left: i32,
    /// The event to test: its place in the list.
    pub event: usize,
    /// True while the pointer drags a selection.
    dragging: bool,
}

impl Scripter {
    pub fn is_open(&self) -> bool {
        self.subject.is_some()
    }

    pub fn changed(&self) -> bool {
        self.buffer.text() != self.saved
    }

    /// Remembers the text before it changes, so that the change can be
    /// undone.
    fn remember(&mut self, typing: bool) {
        if !(typing && self.typing) {
            self.undo.push(self.buffer.clone());
            if self.undo.len() > UNDO_DEPTH {
                self.undo.remove(0);
            }
        }
        self.typing = typing;
        self.redo.clear();
    }

    fn undo(&mut self) -> bool {
        self.typing = false;
        match self.undo.pop() {
            Some(before) => {
                self.redo.push(std::mem::replace(&mut self.buffer, before));
                true
            }
            None => false,
        }
    }

    fn redo(&mut self) -> bool {
        self.typing = false;
        match self.redo.pop() {
            Some(after) => {
                self.undo.push(std::mem::replace(&mut self.buffer, after));
                true
            }
            None => false,
        }
    }

    /// Scrolls so that the cursor is in sight.
    fn follow(&mut self, fonts: &Fonts) {
        let (line, column) = self.buffer.cursor;
        if line < self.top {
            self.top = line;
        } else if line >= self.top + LINES {
            self.top = line + 1 - LINES;
        }
        let before: String = self.buffer.lines[line].chars().take(column).collect();
        let x = fonts.regular.width(&before);
        if x < self.left {
            self.left = (x - 24).max(0);
        } else if x > self.left + TEXT.w - 4 {
            self.left = x - TEXT.w + 24;
        }
    }

    /// The place in the text under a point of the screen.
    fn place_at(&self, fonts: &Fonts, x: i32, y: i32) -> Place {
        let row = ((y - TEXT.y).max(0) / LINE) as usize;
        let line = (self.top + row).min(self.buffer.lines.len() - 1);
        let column = fonts.regular.fitting(&self.buffer.lines[line], x - TEXT.x + self.left + 2);
        (line, column)
    }

    /// True when the text defines the function of an event.
    fn handles(&self, event: Event) -> bool {
        let mut open = Open::Nothing;
        self.buffer.lines.iter().any(|line| {
            let was = open;
            colour(line, &mut open);
            was == Open::Nothing && defines(line.trim_start(), event.function())
        })
    }
}

/// True when a line starts the definition of a function of this name:
/// `function on_click(` or `on_click = `.
fn defines(written: &str, function: &str) -> bool {
    let ends = |rest: &str| !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_');
    match written.strip_prefix("function ") {
        Some(rest) => rest.trim_start().strip_prefix(function).is_some_and(ends),
        None => written.strip_prefix(function).is_some_and(|rest| {
            let rest = rest.trim_start();
            rest.starts_with('=') && !rest.starts_with("==")
        }),
    }
}

/// Opens the editor on the script the session says is being written, unless
/// it is already open on it.
fn sync<A: Api>(session: &Session<A>, scripter: &mut Scripter) -> Option<Scripting> {
    let scripting = session.view().scripting.clone()?;
    let subject = (session.view().stack().to_owned(), scripting.widget.clone());
    if scripter.subject.as_ref() != Some(&subject) {
        let buffer = Buffer::of(&scripting.source);
        *scripter = Scripter {
            subject: Some(subject),
            saved: buffer.text(),
            buffer,
            ..Scripter::default()
        };
    }
    Some(scripting)
}

/// Makes the state agree with the session: the editor is open on the script
/// the server said to edit, and on nothing else.
pub fn settle<A: Api>(session: &Session<A>, scripter: &mut Scripter) {
    if sync(session, scripter).is_none() && scripter.is_open() {
        *scripter = Scripter::default();
    }
}

/// Leaves the editor. A script that changed is saved first, as leaving a card
/// saves what was typed on it; if the server refuses, the editor stays open.
fn leave<A: Api>(session: &mut Session<A>, scripter: &mut Scripter, scripting: &Scripting) -> bool {
    if scripter.changed() {
        let saved = session.run_with(
            "save_script",
            json!({ "widget": scripting.widget, "source": scripter.buffer.text() }),
        );
        if !saved {
            return false;
        }
    }
    session.stop_scripting();
    *scripter = Scripter::default();
    true
}

/// Tries the chosen event on the card that was on screen. Nothing is kept:
/// the server undoes what the script wrote, and says what it would have done.
fn test<A: Api>(session: &mut Session<A>, scripter: &mut Scripter, scripting: &Scripting) {
    let event = events_of(scripting)[scripter.event.min(1)];
    session.run_with(
        "test_script",
        json!({
            "widget": scripting.widget,
            "source": scripter.buffer.text(),
            "event": { "on": event.name() },
        }),
    );
    // The line of an error comes in sight.
    let line = session.view().scripting.as_ref().and_then(|s| s.report.as_ref()?.line);
    if let Some(line) = line {
        let line = (line as usize).saturating_sub(1).min(scripter.buffer.lines.len() - 1);
        if line < scripter.top || line >= scripter.top + LINES {
            scripter.top = line.saturating_sub(LINES / 2);
        }
    }
}

/// Applies a change to the text, so that it can be undone, and forgets what
/// the last test said: it was about another text.
fn change<A: Api>(
    session: &mut Session<A>,
    scripter: &mut Scripter,
    typing: bool,
    change: impl FnOnce(&mut Buffer),
) {
    scripter.remember(typing);
    change(&mut scripter.buffer);
    session.forget_report();
}

pub fn handle_key<A: Api>(
    session: &mut Session<A>,
    scripter: &mut Scripter,
    fonts: &Fonts,
    key: Key,
) {
    let Some(scripting) = sync(session, scripter) else { return };
    match key {
        Key::F(5) => test(session, scripter, &scripting),
        Key::F(12) => {
            leave(session, scripter, &scripting);
            return;
        }
        Key::F(3) => {
            if leave(session, scripter, &scripting) {
                session.press("F3");
            }
            return;
        }
        Key::F(_) => session.say("This key is not active in the script editor."),
        Key::Escape => {
            if scripter.buffer.selection().is_some() {
                scripter.buffer.drop_selection();
            } else {
                leave(session, scripter, &scripting);
                return;
            }
        }
        Key::Undo => {
            if scripter.undo() {
                session.forget_report();
            }
        }
        Key::Redo => {
            if scripter.redo() {
                session.forget_report();
            }
        }
        Key::SelectAll => scripter.buffer.select_all(),
        Key::Char(c) => {
            // A space ends a word: what comes next is undone on its own.
            change(session, scripter, c != ' ', |buffer| buffer.typed(c));
        }
        Key::Enter => change(session, scripter, false, Buffer::enter),
        Key::Backspace => change(session, scripter, false, Buffer::backspace),
        Key::Delete => change(session, scripter, false, Buffer::delete),
        Key::Tab => {
            if scripter.buffer.selection().is_some() {
                change(session, scripter, false, |buffer| buffer.shift(true));
            } else {
                change(session, scripter, false, |buffer| buffer.insert(INDENT));
            }
        }
        Key::BackTab => change(session, scripter, false, |buffer| buffer.shift(false)),
        Key::Interrupt => {}
        _ => {
            scripter.typing = false;
            scripter.buffer.step(key);
        }
    }
    scripter.follow(fonts);
}

/// The text to put on the clipboard: what is selected. When `cut`, it is
/// also taken out of the script.
pub fn copy<A: Api>(
    session: &mut Session<A>,
    scripter: &mut Scripter,
    fonts: &Fonts,
    cut: bool,
) -> Option<String> {
    sync(session, scripter)?;
    let text = scripter.buffer.selected_text()?;
    if cut {
        change(session, scripter, false, |buffer| {
            buffer.delete_selection();
        });
        scripter.follow(fonts);
    }
    Some(text)
}

/// Writes the text of the clipboard at the cursor.
pub fn paste<A: Api>(session: &mut Session<A>, scripter: &mut Scripter, fonts: &Fonts, text: &str) {
    if sync(session, scripter).is_none() || text.is_empty() {
        return;
    }
    change(session, scripter, false, |buffer| buffer.insert(text));
    scripter.follow(fonts);
}

// ---------------------------------------------------------------------------
// The pointer
// ---------------------------------------------------------------------------

/// The two buttons under the source.
fn buttons(fonts: &Fonts) -> [(&'static str, Rect); 2] {
    let test = chrome::button_width(fonts, "Test", ButtonLook::default());
    let close = chrome::button_width(fonts, "Close", ButtonLook::default());
    [
        ("Test", Rect::new(CODE.x, BUTTONS_Y, test, BUTTON_HEIGHT)),
        ("Close", Rect::new(CODE.right() - close, BUTTONS_Y, close, BUTTON_HEIGHT)),
    ]
}

/// The row of the list of events at a place in that list.
fn event_row(index: usize) -> Rect {
    Rect::new(EVENTS.x + 1, EVENTS.y + 17 + index as i32 * ROW, EVENTS.w - 2, ROW)
}

pub fn pointer_down<A: Api>(
    session: &mut Session<A>,
    scripter: &mut Scripter,
    fonts: &Fonts,
    x: i32,
    y: i32,
) {
    let Some(scripting) = sync(session, scripter) else { return };
    // The function key bar: a click is the key.
    let key = chrome::key_slots(fonts, KEYS.into_iter())
        .into_iter()
        .find(|(_, _, area)| area.contains(x, y))
        .and_then(|(name, _, _)| name[1..].parse::<u8>().ok());
    if let Some(number) = key {
        handle_key(session, scripter, fonts, Key::F(number));
        return;
    }
    if close_box_of(WINDOW).contains(x, y) {
        leave(session, scripter, &scripting);
        return;
    }
    for (label, area) in buttons(fonts) {
        if area.contains(x, y) {
            if label == "Test" {
                test(session, scripter, &scripting);
            } else {
                leave(session, scripter, &scripting);
            }
            return;
        }
    }
    if CODE.contains(x, y) {
        scripter.typing = false;
        let place = scripter.place_at(fonts, x, y);
        scripter.buffer.move_to(place, false);
        scripter.dragging = true;
        return;
    }
    // An event is chosen for the test; one the script does not handle yet is
    // given its empty function, at the end of the text.
    for (index, event) in events_of(&scripting).into_iter().enumerate() {
        if event_row(index).contains(x, y) {
            scripter.event = index;
            if !scripter.handles(event) {
                change(session, scripter, false, |buffer| {
                    let last = buffer.lines.len() - 1;
                    buffer.move_to((last, usize::MAX), false);
                    let blank = buffer.text().trim().is_empty();
                    let gap = if blank { "" } else { "\n\n" };
                    buffer.insert(&format!("{gap}{}", event.skeleton()));
                    // The cursor waits inside the new function.
                    let inside = buffer.lines.len() - 2;
                    buffer.move_to((inside, usize::MAX), false);
                });
                scripter.follow(fonts);
            }
            return;
        }
    }
}

/// The pointer moved. Returns true when the screen must be drawn again.
pub fn pointer_move<A: Api>(
    session: &Session<A>,
    scripter: &mut Scripter,
    fonts: &Fonts,
    x: i32,
    y: i32,
) -> bool {
    if !scripter.dragging || session.view().scripting.is_none() {
        return false;
    }
    let place = scripter.place_at(fonts, x, y.clamp(TEXT.y - LINE, TEXT.bottom() + LINE - 1));
    // Above and below the text, the selection goes on line by line.
    let place = if y < TEXT.y {
        (scripter.top.saturating_sub(1), place.1)
    } else if y >= TEXT.bottom() {
        (scripter.top + LINES, place.1)
    } else {
        place
    };
    scripter.buffer.move_to(place, true);
    scripter.follow(fonts);
    true
}

pub fn pointer_up(scripter: &mut Scripter) {
    scripter.dragging = false;
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// Draws the editor. The message, the question and the key bar are drawn by
/// the caller, as for any screen.
pub fn draw<A: Api>(
    session: &Session<A>,
    scripter: &Scripter,
    fonts: &Fonts,
    frame: &mut Frame,
    clock: &str,
    blink: bool,
) {
    let Some(scripting) = session.view().scripting.as_ref() else { return };
    let report = scripting.report.as_ref();
    let wrong = report.and_then(|report| report.line).map(|line| (line as usize).saturating_sub(1));

    chrome::draw_desktop(frame);
    chrome::draw_menu_bar(frame, fonts, Some(&session.hello().instance), clock);
    draw_window(frame, fonts, WINDOW, &scripting.title, WindowStyle::STACK);

    // The source: line numbers, then the text in its colours.
    frame.fill(CODE, slot::NIGHT);
    frame.fill(Rect::new(CODE.x, CODE.y, GUTTER, CODE.h), slot::INK);
    frame.outline(CODE.inset(-1), slot::INK);
    let buffer = &scripter.buffer;
    let selection = buffer.selection();
    let mut open = Open::Nothing;
    for line in buffer.lines.iter().take(scripter.top) {
        colour(line, &mut open);
    }
    for (row, (number, line)) in
        buffer.lines.iter().enumerate().skip(scripter.top).take(LINES).enumerate()
    {
        let y = CODE.y + row as i32 * LINE;
        let baseline = y + 9;
        let current = number == buffer.cursor.0;
        if current {
            frame.fill(Rect::new(TEXT.x - 4, y, CODE.right() - TEXT.x + 4, LINE), slot::NAVY);
        }
        if wrong == Some(number) {
            frame.fill(Rect::new(CODE.x, y, GUTTER, LINE), slot::RED);
        }
        let label = (number + 1).to_string();
        let ink = if current || wrong == Some(number) { slot::WHITE } else { slot::VIOLET };
        let label_x = CODE.x + GUTTER - 4 - fonts.regular.width(&label);
        fonts.regular.draw(frame, label_x, baseline, &label, ink);

        let clip = frame.clip_to(Rect::new(TEXT.x - 2, y, TEXT.w + 2, LINE));
        let x_of = |column: usize| {
            let before: String = line.chars().take(column).collect();
            TEXT.x + fonts.regular.width(&before) - scripter.left
        };
        if let Some(((first, from), (last, to))) = selection {
            if (first..=last).contains(&number) {
                let start = if number == first { x_of(from) } else { TEXT.x - 2 };
                let end = if number == last { x_of(to) } else { TEXT.right() };
                frame.fill(Rect::new(start, y, end - start, LINE), slot::BLUE);
            }
        }
        let mut pen = TEXT.x - scripter.left;
        let mut written = line.chars();
        for (count, ink) in colour(line, &mut open) {
            let piece: String = written.by_ref().take(count).collect();
            pen = fonts.regular.draw(frame, pen, baseline, &piece, ink.slot());
        }
        if current && blink {
            frame.fill(Rect::new(x_of(buffer.cursor.1), y + 1, 2, LINE - 2), slot::FERN);
        }
        frame.set_clip(clip);
    }

    // Under the source: what the last test said, and where the cursor is.
    let place = format!("line {}, column {}", buffer.cursor.0 + 1, buffer.cursor.1 + 1);
    let place_x = CODE.right() - fonts.regular.width(&place);
    fonts.regular.draw(frame, place_x, STATUS_BASELINE, &place, slot::COCOA);
    let (mark, status) = match report {
        Some(report) if report.ok => (slot::FERN, "Tested: no error"),
        Some(_) => (slot::RED, "Tested: there is an error"),
        None if scripter.changed() => (slot::AMBER, "Changed"),
        None => (slot::GREY, "Saved"),
    };
    let square = Rect::new(CODE.x, STATUS_BASELINE - 7, 7, 7);
    frame.fill(square, mark);
    frame.outline(square, slot::INK);
    fonts.regular.draw(frame, CODE.x + 12, STATUS_BASELINE, status, slot::INK);

    for (label, area) in buttons(fonts) {
        let look = ButtonLook { primary: label == "Test", ..ButtonLook::default() };
        chrome::draw_button(frame, fonts, area, label, look);
    }

    // The events: those this script may handle, ticked when it does, and
    // those that will come later.
    draw_window(frame, fonts, EVENTS, "Events", WindowStyle::SYSTEM);
    let handled = events_of(scripting);
    for (index, event) in handled.into_iter().enumerate() {
        let row = event_row(index);
        if index == scripter.event.min(1) {
            frame.fill(row, slot::YELLOW);
        }
        let tick = Rect::new(row.x + 6, row.y + 3, 7, 7);
        frame.fill(tick, if scripter.handles(event) { slot::FERN } else { slot::WHITE });
        frame.outline(tick, slot::INK);
        fonts.regular.draw(frame, row.x + 19, row.y + 10, event.label(), slot::INK);
    }
    let caption = event_row(handled.len());
    frame.hline(caption.x + 5, caption.y + 3, caption.w - 10, slot::SILVER);
    fonts.regular.draw(frame, caption.x + 6, caption.y + 13, "Later:", slot::GREY);
    for (index, label) in LATER.into_iter().enumerate() {
        let row = event_row(handled.len() + 1 + index).moved(0, 2);
        let tick = Rect::new(row.x + 6, row.y + 3, 7, 7);
        frame.outline(tick, slot::GREY);
        fonts.regular.draw(frame, row.x + 19, row.y + 10, label, slot::GREY);
    }

    // What the last test did, in words.
    draw_window(frame, fonts, REPORT, "Test", WindowStyle::SYSTEM);
    let inside = Rect::new(REPORT.x + 7, REPORT.y + 18, REPORT.w - 14, REPORT.h - 24);
    let lines: Vec<String> = match report {
        Some(report) => report.lines.clone(),
        None => vec![
            "F5 tries the chosen event on the card that was on screen.".to_owned(),
            "Nothing it writes is kept.".to_owned(),
        ],
    };
    let clip = frame.clip_to(inside);
    let mut baseline = inside.y + 8;
    'report: for line in &lines {
        for part in chrome::wrap(fonts, line, inside.w) {
            if baseline > inside.bottom() {
                break 'report;
            }
            fonts.regular.draw(frame, inside.x, baseline, &part, slot::INK);
            baseline += LINE;
        }
        baseline += 3;
    }
    frame.set_clip(clip);
}

/// Test access to what is otherwise private to the editor.
#[cfg(test)]
pub mod probe {
    use super::*;

    /// A point of the screen on a place of the text.
    pub fn point_of(scripter: &Scripter, fonts: &Fonts, place: Place) -> (i32, i32) {
        let before: String = scripter.buffer.lines[place.0].chars().take(place.1).collect();
        (
            TEXT.x + fonts.regular.width(&before) - scripter.left,
            TEXT.y + (place.0 - scripter.top) as i32 * LINE + LINE / 2,
        )
    }

    pub fn event_row(index: usize) -> Rect {
        super::event_row(index)
    }

    pub fn button(fonts: &Fonts, label: &str) -> Rect {
        buttons(fonts).into_iter().find(|(name, _)| *name == label).expect("a button").1
    }

    pub fn window() -> Rect {
        WINDOW
    }

    pub fn top(scripter: &Scripter) -> usize {
        scripter.top
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(buffer: &mut Buffer, text: &str) {
        for c in text.chars() {
            if c == '\n' {
                buffer.enter();
            } else {
                buffer.typed(c);
            }
        }
    }

    #[test]
    fn text_is_kept_as_lines_without_tabulations() {
        let buffer = Buffer::of("function on_click()\r\n\tmessage('Hi')\r\nend");
        assert_eq!(buffer.lines(), ["function on_click()", "  message('Hi')", "end"]);
        assert_eq!(buffer.text(), "function on_click()\n  message('Hi')\nend");
        assert_eq!(Buffer::of("").lines(), [""]);
    }

    #[test]
    fn a_new_line_is_indented_like_the_one_before() {
        let mut buffer = Buffer::default();
        typed(&mut buffer, "function on_click()\nif card then\nsave(card)\n");
        assert_eq!(
            buffer.lines(),
            ["function on_click()", "  if card then", "    save(card)", "    "]
        );
        // Backspace takes an indentation back a step at a time.
        buffer.backspace();
        assert_eq!(buffer.lines()[3], "  ");
        // A word that closes a block lines up with what it closes.
        typed(&mut buffer, "  end\nend");
        assert_eq!(buffer.lines()[3..], ["  end", "end"]);
        // Once only: a name that starts like one stays where it is.
        let mut buffer = Buffer::of("if x then");
        buffer.move_to((0, usize::MAX), false);
        typed(&mut buffer, "\nendless = true\nelseif y then");
        assert_eq!(buffer.lines(), ["if x then", "endless = true", "elseif y then"]);
    }

    #[test]
    fn typing_replaces_the_selection() {
        let mut buffer = Buffer::of("local city = \"Paris\"\nmessage(city)");
        buffer.move_to((0, 14), false);
        buffer.move_to((0, 19), true);
        assert_eq!(buffer.selected_text().as_deref(), Some("Paris"));
        buffer.insert("London");
        assert_eq!(buffer.lines()[0], "local city = \"London\"");
        assert_eq!(buffer.selection(), None);

        // Over several lines, and whichever way it was selected.
        buffer.move_to((1, 7), false);
        buffer.move_to((0, 6), true);
        assert_eq!(buffer.selected_text().as_deref(), Some("city = \"London\"\nmessage"));
        buffer.backspace();
        assert_eq!(buffer.text(), "local (city)");
        assert_eq!(buffer.cursor, (0, 6));
    }

    #[test]
    fn pasting_text_of_several_lines_keeps_what_follows_the_cursor() {
        let mut buffer = Buffer::of("save()");
        buffer.move_to((0, 5), false);
        buffer.insert("card\n  -- twice\n  card");
        assert_eq!(buffer.lines(), ["save(card", "  -- twice", "  card)"]);
        assert_eq!(buffer.cursor, (2, 6));
    }

    #[test]
    fn lines_are_joined_from_both_sides() {
        let mut buffer = Buffer::of("one\ntwo\nthree");
        buffer.move_to((1, 0), false);
        buffer.backspace();
        assert_eq!(buffer.lines(), ["onetwo", "three"]);
        assert_eq!(buffer.cursor, (0, 3));
        buffer.move_to((0, usize::MAX), false);
        buffer.delete();
        assert_eq!(buffer.lines(), ["onetwothree"]);
    }

    #[test]
    fn the_arrows_move_and_the_shift_key_selects() {
        let mut buffer = Buffer::of("  first\nsecond line\nx");
        buffer.step(Key::Down);
        buffer.step(Key::End);
        assert_eq!(buffer.cursor, (1, 11));
        // A shorter line stops the cursor at its end.
        buffer.step(Key::Down);
        assert_eq!(buffer.cursor, (2, 1));
        buffer.step(Key::Right);
        assert_eq!(buffer.cursor, (2, 1), "the end of the text");
        buffer.step(Key::Up);
        buffer.step(Key::Up);
        buffer.step(Key::Home);
        assert_eq!(buffer.cursor, (0, 2), "where the line starts being written");
        buffer.step(Key::Home);
        assert_eq!(buffer.cursor, (0, 0));
        buffer.step(Key::Left);
        assert_eq!(buffer.cursor, (0, 0), "the start of the text");

        buffer.step(Key::ShiftDown);
        buffer.step(Key::ShiftRight);
        assert_eq!(buffer.selected_text().as_deref(), Some("  first\ns"));
        // An arrow alone lets go of the selection, on the side it points to.
        buffer.step(Key::Left);
        assert_eq!((buffer.cursor, buffer.selection()), ((0, 0), None));

        buffer.select_all();
        assert_eq!(buffer.selected_text().as_deref(), Some("  first\nsecond line\nx"));
    }

    #[test]
    fn the_selected_lines_are_indented_together() {
        let mut buffer = Buffer::of("if x then\nsave(card)\nmessage('ok')\nend");
        buffer.move_to((1, 0), false);
        buffer.move_to((3, 0), true);
        buffer.shift(true);
        assert_eq!(buffer.lines(), ["if x then", "  save(card)", "  message('ok')", "end"]);
        buffer.shift(false);
        buffer.shift(false);
        assert_eq!(buffer.lines()[1..3], ["save(card)", "message('ok')"]);
    }

    fn inks(line: &str, open: &mut Open) -> Vec<(String, Ink)> {
        let mut written = line.chars();
        colour(line, open)
            .into_iter()
            .map(|(count, ink)| (written.by_ref().take(count).collect::<String>(), ink))
            .filter(|(text, _)| !text.trim().is_empty())
            .collect()
    }

    #[test]
    fn lua_is_coloured_by_what_each_word_is() {
        let mut open = Open::Nothing;
        assert_eq!(
            inks(r#"local n = query("SELECT 'x'", 42) -- rows"#, &mut open),
            [
                ("local".to_owned(), Ink::Keyword),
                (" n = ".to_owned(), Ink::Plain),
                ("query".to_owned(), Ink::Known),
                ("(".to_owned(), Ink::Plain),
                (r#""SELECT 'x'""#.to_owned(), Ink::Text),
                (", ".to_owned(), Ink::Plain),
                ("42".to_owned(), Ink::Number),
                (") ".to_owned(), Ink::Plain),
                ("-- rows".to_owned(), Ink::Comment),
            ]
        );
        assert_eq!(open, Open::Nothing);
        // An escaped quote does not end a text; one never closed runs to the
        // end of the line and no further.
        assert_eq!(inks(r#"'it\'s' "open"#, &mut open).len(), 2);
        assert_eq!(open, Open::Nothing);
        // Every character is in one piece and one only.
        let line = "x = card.city .. [[a]] --[[b]] y";
        assert_eq!(colour(line, &mut open).iter().map(|(n, _)| n).sum::<usize>(), line.len());
    }

    #[test]
    fn a_long_comment_or_text_goes_on_over_the_lines() {
        let mut open = Open::Nothing;
        assert_eq!(inks("x = 1 --[[ not", &mut open).last().unwrap().1, Ink::Comment);
        assert_eq!(open, Open::Comment(0));
        assert_eq!(
            inks("function on_click()", &mut open),
            [("function on_click()".to_owned(), Ink::Comment)]
        );
        assert_eq!(
            inks("yet ]] save(card)", &mut open),
            [
                ("yet ]]".to_owned(), Ink::Comment),
                ("save".to_owned(), Ink::Known),
                ("(".to_owned(), Ink::Plain),
                ("card".to_owned(), Ink::Known),
                (")".to_owned(), Ink::Plain)
            ]
        );
        assert_eq!(open, Open::Nothing);

        inks("sql = [==[SELECT ]] still", &mut open);
        assert_eq!(open, Open::Text(2));
        assert_eq!(inks("text ]==] end", &mut open)[1], ("end".to_owned(), Ink::Keyword));
    }

    #[test]
    fn changes_are_undone_a_word_at_a_time() {
        let mut scripter = Scripter::default();
        for c in "save card".chars() {
            scripter.remember(c != ' ');
            scripter.buffer.insert(&c.to_string());
        }
        assert!(scripter.undo());
        assert_eq!(scripter.buffer.text(), "save ");
        assert!(scripter.undo());
        assert_eq!(scripter.buffer.text(), "save");
        assert!(scripter.undo());
        assert_eq!(scripter.buffer.text(), "");
        assert!(!scripter.undo());
        assert!(scripter.redo() && scripter.redo() && scripter.redo());
        assert_eq!(scripter.buffer.text(), "save card");
        assert!(!scripter.redo());
        // A new change makes what was undone impossible to do again.
        scripter.undo();
        scripter.remember(false);
        scripter.buffer.insert("!");
        assert!(!scripter.redo());
    }

    #[test]
    fn a_function_in_a_comment_does_not_handle_an_event() {
        let script = |text: &str| Scripter { buffer: Buffer::of(text), ..Scripter::default() };
        assert!(script("  function on_click()\nend").handles(Event::Click));
        assert!(script("on_key = function(key) end").handles(Event::Key));
        assert!(!script("-- function on_click()").handles(Event::Click));
        assert!(!script("--[[\nfunction on_open()\nend ]]").handles(Event::Open));
        assert!(!script("function on_clicked() end").handles(Event::Click));
        assert!(!script("on_click == nil").handles(Event::Click));
    }

    // The editor on screen ----------------------------------------------------

    use crate::design;
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
            let key = if c == '\n' { Key::Enter } else { Key::Char(c) };
            screen::handle_key(session, ui, &fonts(), key);
        }
    }

    fn click(session: &mut Session<Fake>, ui: &mut Ui, x: i32, y: i32) {
        screen::pointer_down(session, ui, &fonts(), x, y);
        screen::pointer_up(session, ui, &fonts(), x, y);
    }

    /// The script editor, open on the script of the button "next".
    fn script_session() -> (Session<Fake>, Ui) {
        let (mut session, mut ui) = editor_session();
        let button = design::probe::area_of(session.view(), &ui.designer, &fonts(), "next");
        click(&mut session, &mut ui, button.x + button.w / 2, button.y + button.h / 2);
        assert_eq!(ui.designer.selected.as_deref(), Some("next"));
        press(&mut session, &mut ui, &[Key::F(8)]);
        assert!(session.view().scripting.is_some(), "{:?}", session.message());
        (session, ui)
    }

    #[test]
    fn the_script_of_a_button_is_written_and_saved_on_leaving() {
        let (mut session, mut ui) = script_session();
        assert_eq!(session.api().last().1["widget"], "next");
        type_text(&mut session, &mut ui, "function on_click()\nmessage(\"Hello\")\nend");
        // The lines were indented as they were typed.
        assert_eq!(ui.scripter.buffer.text(), "function on_click()\n  message(\"Hello\")\nend");

        let frame = render(&session, &ui);
        snapshot(&frame, "script");
        // The window carries the name of the button; the source is written
        // light on dark, with its words in their colours.
        assert!(has_slot(&frame, Rect::new(WINDOW.x, WINDOW.y, WINDOW.w, 13), slot::AMBER));
        assert!(count_slot(&frame, CODE, slot::NIGHT) > 10_000);
        let first = Rect::new(TEXT.x, TEXT.y, TEXT.w, LINE);
        assert!(has_slot(&frame, first, Ink::Keyword.slot()), "function");
        assert!(has_slot(&frame, first, Ink::Known.slot()), "on_click");
        let second = Rect::new(TEXT.x, TEXT.y + LINE, TEXT.w, LINE);
        assert!(has_slot(&frame, second, Ink::Text.slot()), "the text of the message");
        // The events of a button; the script handles the click.
        let tick = |index: usize| {
            let row = probe::event_row(index);
            frame.pixel(row.x + 8, row.y + 5)
        };
        assert_eq!(tick(0), Some(slot::FERN), "click is handled");
        assert_eq!(tick(1), Some(slot::WHITE), "key is not");

        // Leaving saves, and goes back to the visual editor.
        press(&mut session, &mut ui, &[Key::F(12)]);
        assert_eq!(session.view().scripting, None);
        assert!(session.view().designing);
        assert!(!ui.scripter.is_open());
        assert_eq!(
            session.api().scripts.get(&Some("next".to_owned())).map(String::as_str),
            Some("function on_click()\n  message(\"Hello\")\nend")
        );
        assert_eq!(session.message(), Some("Script saved."));

        // It opens again on what was saved; leaving it untouched saves nothing.
        press(&mut session, &mut ui, &[Key::F(8)]);
        assert_eq!(ui.scripter.buffer.lines().len(), 3);
        press(&mut session, &mut ui, &[Key::Escape]);
        assert_eq!(session.api().count("save_script"), 1);
        assert_eq!(session.view().scripting, None);
    }

    #[test]
    fn with_nothing_selected_it_is_the_script_of_the_screen() {
        let (mut session, mut ui) = editor_session();
        press(&mut session, &mut ui, &[Key::F(8)]);
        let scripting = session.view().scripting.clone().expect("the editor is open");
        assert_eq!(scripting.widget, None);
        assert_eq!(scripting.title, "Script: screen of “Address book”");

        // The screen is told about keys and about cards that come on screen.
        // Choosing an event it does not handle writes its empty function,
        // with the cursor inside.
        let row = probe::event_row(1);
        click(&mut session, &mut ui, row.x + 30, row.y + 5);
        assert_eq!(ui.scripter.buffer.text(), "function on_open()\n  \nend");
        assert_eq!(ui.scripter.buffer.cursor, (1, 2));
        type_text(&mut session, &mut ui, "message(card.city)");
        // Chosen again, it is only the event to test.
        click(&mut session, &mut ui, row.x + 30, row.y + 5);
        assert_eq!(ui.scripter.buffer.lines().len(), 3);
        press(&mut session, &mut ui, &[Key::F(5)]);
        assert_eq!(session.api().last().1["event"]["on"], "open");
    }

    #[test]
    fn a_test_says_what_the_script_would_do_and_where_it_is_wrong() {
        let (mut session, mut ui) = script_session();
        type_text(&mut session, &mut ui, "function on_click()\nboom()\nend");
        press(&mut session, &mut ui, &[Key::F(5)]);

        let (action, input) = session.api().last();
        assert_eq!(action, "test_script");
        assert_eq!(input["source"], "function on_click()\n  boom()\nend");
        assert_eq!(input["event"]["on"], "click");
        let report = session.view().scripting.as_ref().unwrap().report.clone().unwrap();
        assert_eq!((report.ok, report.line), (false, Some(2)));
        // The line is marked in the margin, and the report is in its window.
        let frame = render(&session, &ui);
        snapshot(&frame, "script-error");
        assert!(has_slot(&frame, Rect::new(CODE.x, CODE.y + LINE, GUTTER, LINE), slot::RED));
        assert!(!has_slot(&frame, Rect::new(CODE.x, CODE.y, GUTTER, LINE), slot::RED));
        assert!(has_slot(&frame, REPORT.inset(8), slot::INK));

        // Changing the script makes the report out of date: it goes away.
        press(&mut session, &mut ui, &[Key::Up, Key::Home, Key::ShiftRight, Key::ShiftRight]);
        press(&mut session, &mut ui, &[Key::ShiftRight, Key::ShiftRight]);
        type_text(&mut session, &mut ui, "print");
        assert_eq!(ui.scripter.buffer.lines()[1], "  print()");
        assert_eq!(session.view().scripting.as_ref().unwrap().report, None);
        // The Test button does what F5 does.
        let test = probe::button(&fonts(), "Test");
        click(&mut session, &mut ui, test.x + 5, test.y + 5);
        let report = session.view().scripting.as_ref().unwrap().report.clone().unwrap();
        assert!(report.ok);
        assert_eq!(session.api().count("test_script"), 2);
        // Nothing was saved by testing.
        assert_eq!(session.api().count("save_script"), 0);
    }

    #[test]
    fn text_is_selected_copied_cut_and_pasted() {
        let (mut session, mut ui) = script_session();
        type_text(&mut session, &mut ui, "local city = \"Paris\"\nmessage(city)");

        // The pointer puts the cursor, and drags a selection.
        let from = probe::point_of(&ui.scripter, &fonts(), (0, 14));
        let to = probe::point_of(&ui.scripter, &fonts(), (0, 19));
        screen::pointer_down(&mut session, &mut ui, &fonts(), from.0, from.1);
        assert_eq!(ui.scripter.buffer.cursor, (0, 14));
        assert!(screen::pointer_move(&mut session, &mut ui, &fonts(), to.0, to.1));
        screen::pointer_up(&mut session, &mut ui, &fonts(), to.0, to.1);
        assert_eq!(ui.scripter.buffer.selected_text().as_deref(), Some("Paris"));
        // Once released, the pointer no longer selects.
        assert!(!screen::pointer_move(&mut session, &mut ui, &fonts(), from.0, from.1));

        // Copying leaves the text as it is; cutting takes it out.
        assert_eq!(screen::copy(&mut session, &mut ui, &fonts(), false).as_deref(), Some("Paris"));
        assert_eq!(ui.scripter.buffer.lines()[0], "local city = \"Paris\"");
        assert_eq!(screen::copy(&mut session, &mut ui, &fonts(), true).as_deref(), Some("Paris"));
        assert_eq!(ui.scripter.buffer.lines()[0], "local city = \"\"");
        assert_eq!(screen::copy(&mut session, &mut ui, &fonts(), false), None, "nothing selected");

        // Pasting writes at the cursor, over several lines if need be.
        screen::paste(&mut session, &mut ui, &fonts(), "Lon\tdon");
        assert_eq!(ui.scripter.buffer.lines()[0], "local city = \"Lon  don\"");
        press(&mut session, &mut ui, &[Key::SelectAll]);
        screen::paste(&mut session, &mut ui, &fonts(), "one\r\ntwo");
        assert_eq!(ui.scripter.buffer.text(), "one\ntwo");

        // Each of these is undone on its own, and done again.
        press(&mut session, &mut ui, &[Key::Undo]);
        assert_eq!(ui.scripter.buffer.lines()[0], "local city = \"Lon  don\"");
        press(&mut session, &mut ui, &[Key::Undo, Key::Undo]);
        assert_eq!(ui.scripter.buffer.lines()[0], "local city = \"Paris\"");
        press(&mut session, &mut ui, &[Key::Redo, Key::Redo, Key::Redo]);
        assert_eq!(ui.scripter.buffer.text(), "one\ntwo");
    }

    #[test]
    fn a_long_script_scrolls_with_the_cursor() {
        let (mut session, mut ui) = script_session();
        let long: String = (1..=40).map(|n| format!("-- line {n}\n")).collect();
        screen::paste(&mut session, &mut ui, &fonts(), &long);
        assert_eq!(ui.scripter.buffer.cursor, (40, 0));
        assert_eq!(probe::top(&ui.scripter), 41 - LINES);
        press(&mut session, &mut ui, &[Key::PageUp, Key::PageUp, Key::PageUp]);
        assert_eq!(ui.scripter.buffer.cursor.0, 0);
        assert_eq!(probe::top(&ui.scripter), 0);
        // A line longer than the window scrolls sideways.
        type_text(&mut session, &mut ui, &"x = x + 1 ".repeat(12));
        let cursor = probe::point_of(&ui.scripter, &fonts(), ui.scripter.buffer.cursor);
        assert!(TEXT.contains(cursor.0, cursor.1), "{cursor:?}");
        press(&mut session, &mut ui, &[Key::Home, Key::Home]);
        assert_eq!(probe::point_of(&ui.scripter, &fonts(), (0, 0)).0, TEXT.x);
    }

    #[test]
    fn a_script_the_server_refuses_stays_in_the_editor() {
        let (mut session, mut ui) = script_session();
        type_text(&mut session, &mut ui, "-- mine");
        session.api_mut().refuses_scripts = true;
        press(&mut session, &mut ui, &[Key::F(12)]);
        assert!(session.view().scripting.is_some(), "the work is not lost");
        assert_eq!(ui.scripter.buffer.text(), "-- mine");
        assert!(session.message().unwrap().contains("belongs to bob"));

        // The close box and the Close button leave the same way.
        session.api_mut().refuses_scripts = false;
        let close = probe::button(&fonts(), "Close");
        click(&mut session, &mut ui, close.x + 5, close.y + 5);
        assert_eq!(session.view().scripting, None);
        assert_eq!(session.api().scripts[&Some("next".to_owned())], "-- mine");
    }

    #[test]
    fn the_keys_of_the_editor_are_its_own() {
        let (mut session, mut ui) = script_session();
        // A key of the stack does nothing here, and says so.
        press(&mut session, &mut ui, &[Key::F(8)]);
        assert_eq!(session.message(), Some("This key is not active in the script editor."));
        assert_eq!(session.api().count("next_card"), 0);
        // The key bar is clicked like anywhere else.
        let keys = chrome::key_slots(&fonts(), KEYS.into_iter());
        let (_, _, test) = keys.iter().find(|(name, _, _)| *name == "F5").unwrap();
        click(&mut session, &mut ui, test.x + 3, test.y + 3);
        assert_eq!(session.api().count("test_script"), 1);
        // Escape lets go of a selection first, then leaves.
        type_text(&mut session, &mut ui, "x");
        press(&mut session, &mut ui, &[Key::SelectAll, Key::Escape]);
        assert!(session.view().scripting.is_some());
        assert_eq!(ui.scripter.buffer.selection(), None);
        press(&mut session, &mut ui, &[Key::Escape]);
        assert_eq!(session.view().scripting, None);
    }
}
