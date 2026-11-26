//! The state of a client session, independent of how it is drawn.
//!
//! A session is a pile of views. Each view is one stack on screen: its
//! definition, the card being shown and what the user has typed but not saved.
//! The session holds no business rule: it sends the user's intent to the
//! server as an action, and applies the effects the server answers with.

use std::collections::BTreeMap;

use crate::api::{
    Api, ApiError, Card, Context, Effect, Event, GridPage, Hello, Image, Move, Notification,
    PaletteColour, Screen, Widget, WidgetKind,
};

/// The version of the API this client speaks; see `docs/api.md`.
pub const API_VERSION: u32 = 4;

/// The rows of a grid widget and the selected one.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GridState {
    pub page: GridPage,
    pub selected: usize,
}

impl GridState {
    pub fn selected_key(&self) -> Option<&str> {
        self.page.rows.get(self.selected).map(|row| row.key.as_str())
    }
}

/// One stack on screen.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub screen: Screen,
    pub card: Card,
    /// True while the user fills in a card that is not saved yet.
    pub is_new: bool,
    /// True while the stack is open in the visual editor rather than browsed.
    pub designing: bool,
    /// The picture open in the drawing editor, if one is.
    pub drawing: Option<Drawing>,
    /// The script open in the script editor, if one is.
    pub scripting: Option<Scripting>,
    /// What the user typed, by column.
    pub edits: BTreeMap<String, String>,
    pub grids: BTreeMap<String, GridState>,
    /// Name of the widget that has the focus. The renderer decides the order.
    pub focus: Option<String>,
}

impl View {
    pub fn stack(&self) -> &str {
        &self.screen.stack.name
    }

    /// The text to show in a field: what was typed, or else the saved value.
    pub fn value(&self, column: &str) -> &str {
        match self.edits.get(column) {
            Some(typed) => typed,
            None => self.saved(column),
        }
    }

    fn saved(&self, column: &str) -> &str {
        self.card.values.get(column).and_then(|v| v.as_deref()).unwrap_or("")
    }

    /// The edits that actually change something.
    pub fn dirty(&self) -> BTreeMap<String, String> {
        self.edits
            .iter()
            .filter(|(column, typed)| typed.as_str() != self.saved(column))
            .map(|(column, typed)| (column.clone(), typed.clone()))
            .collect()
    }

    /// The identifier of the image an image widget shows on this card: a
    /// fixed one from its properties, or else the value of its column.
    pub fn image_id<'a>(&'a self, widget: &'a Widget) -> Option<&'a str> {
        let fixed = widget.props.get("image").and_then(|v| v.as_str());
        let bound = || widget.column.as_deref().map(|column| self.value(column));
        fixed.or_else(bound).filter(|id| !id.is_empty())
    }

    /// The widgets on screen: those of the background, then those of the card.
    pub fn widgets(&self) -> impl Iterator<Item = &Widget> {
        self.screen.widgets.iter().chain(self.card.widgets.iter())
    }

    /// What a text widget shows. The server computes it for the card on
    /// screen. A card that is not saved yet has no such text: a text that
    /// depends on nothing is shown as it is written, and any other is empty.
    pub fn text(&self, name: &str) -> &str {
        if let Some(text) = self.card.texts.get(name) {
            return text;
        }
        let widget = self.widgets().find(|w| w.name == name);
        let template = widget.and_then(|w| w.props.get("template")).and_then(|v| v.as_str());
        match template {
            Some(fixed) if !fixed.contains('{') => fixed,
            Some(_) => "",
            None => widget.and_then(|w| w.label.as_deref()).unwrap_or(""),
        }
    }

    /// The key of the card on screen, unless it is not saved yet.
    fn card_key(&self) -> Option<&str> {
        if self.is_new {
            None
        } else {
            self.card.key.as_deref()
        }
    }

    /// What the server needs to know about this view to run an action.
    fn context(
        &self,
        confirmed: bool,
        renderer: &str,
        input: Option<serde_json::Value>,
        event: Option<Event>,
    ) -> Context {
        // The focused grid decides the selected row; a single grid needs no focus.
        let grid = self.focus.as_ref().and_then(|name| self.grids.get(name)).or_else(|| {
            if self.grids.len() == 1 {
                self.grids.values().next()
            } else {
                None
            }
        });
        Context {
            card: self.card_key().map(str::to_owned),
            dirty: self.dirty(),
            row: grid.and_then(|g| g.selected_key()).map(str::to_owned),
            confirmed,
            renderer: renderer.to_owned(),
            input,
            event,
        }
    }

    /// What tells one card on screen from another: the stack, and the card
    /// or the fact that it is a new one.
    fn shown(&self) -> (String, bool, Option<String>) {
        (self.stack().to_owned(), self.is_new, self.card_key().map(str::to_owned))
    }
}

/// A script open in the script editor of the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scripting {
    /// The button the script belongs to; None for the script of the screen.
    pub widget: Option<String>,
    pub title: String,
    /// The script as it is saved: what the editor starts from.
    pub source: String,
    /// What the last test did, if one was run.
    pub report: Option<Report>,
}

/// What the test of a script did, in words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub ok: bool,
    /// The line of the error, if there is one and it has a line.
    pub line: Option<u32>,
    pub lines: Vec<String>,
}

/// A picture open in the drawing editor of the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drawing {
    /// What the picture is: `icon`, `card` or `widget`.
    pub target: String,
    /// The widget that shows it, unless it is the icon of the stack.
    pub widget: Option<String>,
    /// The image it shows now, if any: what the drawing starts from.
    pub image: Option<String>,
    pub width: u16,
    pub height: u16,
}

/// A question an action asks before it goes on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    pub text: String,
    /// What accepting does, in a word: "Delete".
    pub label: String,
    /// The action to run again once the user has accepted, with what the
    /// tool that ran it had to say and what triggered it.
    action: String,
    input: Option<serde_json::Value>,
    event: Option<Event>,
}

pub struct Session<A: Api> {
    api: A,
    hello: Hello,
    palette: Vec<PaletteColour>,
    views: Vec<View>,
    /// The images read so far, by identifier. None is an image that does
    /// not exist, so that it is not asked for again.
    images: BTreeMap<String, Option<Image>>,
    message: Option<String>,
    question: Option<Question>,
    /// Which client this is, for the server: `text` unless said otherwise.
    renderer: &'static str,
    quit: bool,
}

impl<A: Api> Session<A> {
    /// Greets the server and opens the home stack.
    pub fn start(mut api: A) -> Result<Self, ApiError> {
        let hello = api.hello()?;
        if hello.api != API_VERSION {
            return Err(ApiError::Version { client: API_VERSION, server: hello.api });
        }
        let palette = api.palette()?;
        let home = open(&mut api, &hello.home, None)?;
        let mut session = Session {
            api,
            hello,
            palette,
            views: vec![home],
            images: BTreeMap::new(),
            message: None,
            question: None,
            renderer: "text",
            quit: false,
        };
        session.fetch_images();
        Ok(session)
    }

    /// The server this session talks to.
    pub fn api(&self) -> &A {
        &self.api
    }

    /// The same, to change it: a test makes its server behave otherwise.
    pub fn api_mut(&mut self) -> &mut A {
        &mut self.api
    }

    pub fn hello(&self) -> &Hello {
        &self.hello
    }

    /// The system colours. Both renderers draw with these and nothing else.
    pub fn palette(&self) -> &[PaletteColour] {
        &self.palette
    }

    /// An image of the current screen, once read from the server.
    pub fn image(&self, id: &str) -> Option<&Image> {
        self.images.get(id).and_then(Option::as_ref)
    }

    /// Reads the images the current screen shows and that are not known yet.
    /// An image that cannot be read is simply not drawn.
    fn fetch_images(&mut self) {
        let view = self.views.last().expect("a session always has a view");
        let pictures = view
            .widgets()
            .filter(|widget| widget.kind == WidgetKind::Image)
            .filter_map(|widget| view.image_id(widget));
        // A list shown as icons has one image for each of its rows.
        let icons =
            view.screen.widgets.iter().filter(|w| w.kind == WidgetKind::Grid).flat_map(|widget| {
                let column = widget.props.get("icon").and_then(|v| v.as_str());
                let rows = view.grids.get(&widget.name).map(|grid| grid.page.rows.as_slice());
                rows.unwrap_or_default()
                    .iter()
                    .filter_map(move |row| row.cells.get(column?).and_then(|cell| cell.as_deref()))
            });
        let mut wanted: Vec<String> = Vec::new();
        for id in pictures.chain(icons) {
            if !id.is_empty() && !self.images.contains_key(id) && !wanted.iter().any(|w| w == id) {
                wanted.push(id.to_owned());
            }
        }
        for id in wanted {
            let image = self.api.image(&id).ok().flatten();
            self.images.insert(id, image);
        }
    }

    /// Shows a message to the user. The next action replaces it.
    pub fn say(&mut self, text: impl Into<String>) {
        self.message = Some(text.into());
    }

    pub fn view(&self) -> &View {
        self.views.last().expect("a session always has a view")
    }

    fn view_mut(&mut self) -> &mut View {
        self.views.last_mut().expect("a session always has a view")
    }

    /// The message to show the user, if any.
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub fn should_quit(&self) -> bool {
        self.quit
    }

    /// The question waiting for an answer, if any. While there is one, the
    /// client asks it and does nothing else.
    pub fn question(&self) -> Option<&Question> {
        self.question.as_ref()
    }

    /// Answers the pending question: accepting runs its action for good.
    pub fn answer(&mut self, accepted: bool) {
        if let Some(question) = self.question.take() {
            if accepted {
                self.perform(&question.action, true, question.input, question.event);
            }
        }
    }

    /// Says that this client draws in a window. The server then answers
    /// author mode with the visual editor instead of the forms.
    pub fn be_graphic(&mut self) {
        self.renderer = "graphic";
    }

    /// Leaves the visual editor: the stack is browsed again.
    pub fn stop_designing(&mut self) {
        self.view_mut().designing = false;
    }

    /// Leaves the drawing editor.
    pub fn stop_drawing(&mut self) {
        self.view_mut().drawing = None;
    }

    /// Leaves the script editor.
    pub fn stop_scripting(&mut self) {
        self.view_mut().scripting = None;
    }

    /// Forgets what the last test of the script said: the script has changed.
    pub fn forget_report(&mut self) {
        if let Some(scripting) = &mut self.view_mut().scripting {
            scripting.report = None;
        }
    }

    /// Changes a field that only takes some values to the next of them, or
    /// to the previous one. Returns false when the field takes any value.
    pub fn cycle(&mut self, widget: &str, forward: bool) -> bool {
        let view = self.view();
        let next = view.widgets().find(|w| w.name == widget).and_then(|w| {
            Some((w.column.clone()?, w.next_choice(view.value(w.column.as_deref()?), forward)?))
        });
        match next {
            Some((column, value)) => {
                self.edit(&column, value);
                true
            }
            None => false,
        }
    }

    /// Records what the user typed in a field.
    pub fn edit(&mut self, column: &str, text: String) {
        self.view_mut().edits.insert(column.to_owned(), text);
    }

    pub fn set_focus(&mut self, widget: Option<String>) {
        self.view_mut().focus = widget;
    }

    /// Moves the selection of a grid up or down.
    pub fn select_row(&mut self, widget: &str, delta: isize) {
        if let Some(grid) = self.view_mut().grids.get_mut(widget) {
            let last = grid.page.rows.len().saturating_sub(1);
            grid.selected = grid.selected.saturating_add_signed(delta).min(last);
        }
    }

    /// Selects a row of a grid by its position, for a client with a pointer.
    pub fn select(&mut self, widget: &str, row: usize) {
        if let Some(grid) = self.view_mut().grids.get_mut(widget) {
            grid.selected = row.min(grid.page.rows.len().saturating_sub(1));
        }
    }

    /// Runs the action bound to a key ("F8", "Enter"). Returns false when the
    /// key is not bound on this screen.
    pub fn press(&mut self, key: &str) -> bool {
        let bound = self.view().screen.keys.iter().find(|k| k.key == key).map(|k| k.action.clone());
        match bound {
            Some(action) => {
                self.perform(&action, false, None, Some(Event::key(key)));
                true
            }
            None => false,
        }
    }

    /// Runs the action of a widget: a button, or a grid for its selected row.
    pub fn activate(&mut self, widget: &str) {
        let action =
            self.view().widgets().find(|w| w.name == widget).and_then(|w| w.action.clone());
        if let Some(action) = action {
            self.perform(&action, false, None, Some(Event::click(widget)));
        }
    }

    /// Asks the server to run an action and applies the effects it answers.
    pub fn run(&mut self, action: &str) {
        self.perform(action, false, None, None);
    }

    /// Runs an action for a tool of the client, which says what it wants in
    /// `input`. Returns false when the server refused: the message says why.
    pub fn run_with(&mut self, action: &str, input: serde_json::Value) -> bool {
        self.perform(action, false, Some(input), None)
    }

    fn perform(
        &mut self,
        action: &str,
        confirmed: bool,
        input: Option<serde_json::Value>,
        event: Option<Event>,
    ) -> bool {
        self.message = None;
        self.question = None;
        let before = self.view().shown();
        let stack = self.view().stack().to_owned();
        let context = self.view().context(confirmed, self.renderer, input.clone(), event.clone());
        let outcome = self.api.act(&stack, action, &context).and_then(|effects| {
            effects.into_iter().try_for_each(|effect| self.apply(effect, action, &input, &event))
        });
        let done = outcome.is_ok();
        if let Err(error) = outcome {
            self.message = Some(describe(&error));
        }
        // A card shown by the script of the screen is not announced to it
        // again: it would never end.
        let by_the_screen = event.as_ref().is_some_and(|event| event.on == "open");
        if done && !by_the_screen && self.question.is_none() {
            self.announce_card(before);
        }
        self.fetch_images();
        done
    }

    /// Tells the script of the screen that a card came on screen, when it
    /// asked to be told and the card is not the one that was there.
    fn announce_card(&mut self, before: (String, bool, Option<String>)) {
        let view = self.view();
        let wanted = view.screen.events.iter().any(|event| event == "open");
        let editing = view.designing || view.drawing.is_some() || view.scripting.is_some();
        if !wanted || editing || view.shown() == before {
            return;
        }
        // What the action said stays, unless the script has something to say.
        let said = self.message.take();
        self.perform("script", false, None, Some(Event::open()));
        if self.message.is_none() {
            self.message = said;
        }
    }

    /// Applies one effect of an action.
    fn apply(
        &mut self,
        effect: Effect,
        action: &str,
        input: &Option<serde_json::Value>,
        event: &Option<Event>,
    ) -> Result<(), ApiError> {
        match effect {
            Effect::Show { stack, card } => {
                if stack == self.view().stack() {
                    self.load_card(card.as_deref(), Move::At)?;
                } else {
                    self.bring_up(&stack, card.as_deref())?;
                }
            }
            Effect::Design { stack } => {
                if self.renderer != "graphic" {
                    self.message = Some("The visual editor needs the graphical client.".into());
                    return Ok(());
                }
                // The stack being browsed is edited where it is, on the card
                // it shows.
                if stack != self.view().stack() {
                    self.bring_up(&stack, None)?;
                }
                let view = self.view_mut();
                view.designing = true;
                view.edits.clear();
                view.focus = None;
            }
            Effect::Back => self.back()?,
            Effect::Draw { target, widget, image, width, height } => {
                if self.renderer != "graphic" {
                    self.message = Some("Drawing needs the graphical client.".into());
                    return Ok(());
                }
                // The drawing starts from the image the picture shows now.
                if let Some(id) = &image {
                    if !self.images.contains_key(id) {
                        let read = self.api.image(id)?;
                        self.images.insert(id.clone(), read);
                    }
                }
                self.view_mut().drawing = Some(Drawing { target, widget, image, width, height });
            }
            Effect::Reload => self.reopen()?,
            Effect::Focus { widget } => self.view_mut().focus = Some(widget),
            Effect::Message { text } => self.message = Some(text),
            Effect::New { values } => {
                let view = self.view_mut();
                view.card = Card { count: view.card.count, ..Card::default() };
                view.is_new = true;
                view.edits = values;
                view.focus = None;
                // A grid that lists what belongs to the card has nothing to
                // list for a card that does not exist yet.
                self.load_grids()?;
            }
            Effect::Set { values } => {
                let edits = &mut self.view_mut().edits;
                for (column, value) in values {
                    edits.insert(column, value.unwrap_or_default());
                }
            }
            Effect::Script { widget, title, source } => {
                if self.renderer != "graphic" {
                    self.message = Some("Scripts are written in the graphical client.".into());
                    return Ok(());
                }
                self.view_mut().scripting = Some(Scripting { widget, title, source, report: None });
            }
            Effect::Tested { ok, line, lines } => {
                if let Some(scripting) = &mut self.view_mut().scripting {
                    scripting.report = Some(Report { ok, line, lines });
                }
            }
            Effect::Confirm { text, label } => {
                self.question = Some(Question {
                    text,
                    label,
                    action: action.to_owned(),
                    input: input.clone(),
                    event: event.clone(),
                });
            }
            Effect::Select { row } => {
                for grid in self.view_mut().grids.values_mut() {
                    if let Some(at) = grid.page.rows.iter().position(|r| r.key == row) {
                        grid.selected = at;
                    }
                }
            }
            Effect::Quit => self.quit = true,
        }
        Ok(())
    }

    /// Puts a stack other than the current one on screen: back to it when it
    /// is already open underneath, rather than opened a second time.
    fn bring_up(&mut self, stack: &str, card: Option<&str>) -> Result<(), ApiError> {
        if let Some(below) = self.views.iter().rposition(|v| v.stack() == stack) {
            self.views.truncate(below + 1);
            // Its definition may have changed while it was covered.
            self.reopen()?;
            if let Some(key) = card {
                self.load_card(Some(key), Move::At)?;
            }
        } else {
            let view = open(&mut self.api, stack, card)?;
            self.views.push(view);
        }
        Ok(())
    }

    /// Returns to the previous screen. A screen that no longer exists, such
    /// as a stack that was just deleted, is skipped.
    fn back(&mut self) -> Result<(), ApiError> {
        while self.views.len() > 1 {
            self.views.pop();
            // What is underneath may have changed in the meantime.
            match self.reopen() {
                Err(ApiError::User { code, .. }) if code == "XL002" && self.views.len() > 1 => {}
                outcome => return outcome,
            }
        }
        Ok(())
    }

    /// Reads a card of the current stack and shows it, dropping the edits.
    fn load_card(&mut self, key: Option<&str>, to: Move) -> Result<(), ApiError> {
        let stack = self.view().stack().to_owned();
        let to = if key.is_none() && to == Move::At { Move::First } else { to };
        let card = self.api.card(&stack, key, to)?;
        let view = self.view_mut();
        view.card = card;
        view.is_new = false;
        view.edits.clear();
        self.load_grids()
    }

    /// Reads again what the current view shows, keeping the user's edits.
    fn refresh(&mut self) -> Result<(), ApiError> {
        let stack = self.view().stack().to_owned();
        if !self.view().is_new {
            let key = self.view().card.key.clone();
            let to = if key.is_some() { Move::At } else { Move::First };
            let card = self.api.card(&stack, key.as_deref(), to)?;
            self.view_mut().card = card;
        }
        self.load_grids()
    }

    /// Reads the rows of the grids of the current view, keeping the selected
    /// row where it was.
    fn load_grids(&mut self) -> Result<(), ApiError> {
        let stack = self.view().stack().to_owned();
        let card = self.view().card_key().map(str::to_owned);
        let names: Vec<String> = self.view().grids.keys().cloned().collect();
        for name in names {
            let page = self.api.grid_rows(&stack, &name, card.as_deref())?;
            let grid = self.view_mut().grids.get_mut(&name).expect("the grid was just listed");
            let selected = grid.selected_key().map(str::to_owned);
            // The selection follows its row when the rows move around it.
            grid.selected = selected
                .and_then(|key| page.rows.iter().position(|r| r.key == key))
                .unwrap_or(grid.selected)
                .min(page.rows.len().saturating_sub(1));
            grid.page = page;
        }
        Ok(())
    }

    /// Handles the notifications pushed by the server. Returns true when the
    /// screen must be drawn again.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        for notification in self.api.notifications() {
            let outcome = match notification {
                // Any card change can alter the current card, its position,
                // or the counts shown by a grid.
                Notification::Card { stack, .. } => {
                    let concerned = stack.as_deref() == Some(self.view().stack())
                        || !self.view().grids.is_empty();
                    if concerned {
                        changed = true;
                        self.refresh()
                    } else {
                        Ok(())
                    }
                }
                // Without a stack, it is the palette that changed.
                Notification::Screen { stack: None } => {
                    changed = true;
                    self.api.palette().map(|palette| self.palette = palette)
                }
                Notification::Screen { stack: Some(stack) } => {
                    if stack == self.view().stack() {
                        changed = true;
                        self.reopen()
                    } else if !self.view().grids.is_empty() {
                        // A list may show that stack: its fields, its title.
                        changed = true;
                        self.refresh()
                    } else {
                        Ok(())
                    }
                }
            };
            if let Err(error) = outcome {
                self.message = Some(describe(&error));
            }
        }
        if changed {
            self.fetch_images();
        }
        changed
    }

    /// Reads the definition of the current screen again, keeping the card
    /// and the user's edits.
    fn reopen(&mut self) -> Result<(), ApiError> {
        let stack = self.view().stack().to_owned();
        let screen = self.api.open_stack(&stack)?;
        let view = self.view_mut();
        // The grids follow the definition: new ones appear, old ones go.
        let mut grids = BTreeMap::new();
        for widget in screen.widgets.iter().filter(|w| w.kind == WidgetKind::Grid) {
            let state = view.grids.remove(&widget.name).unwrap_or_default();
            grids.insert(widget.name.clone(), state);
        }
        view.grids = grids;
        view.screen = screen;
        self.refresh()
    }
}

/// Opens a stack as a new view, at a given card or at the first one.
fn open<A: Api>(api: &mut A, stack: &str, card: Option<&str>) -> Result<View, ApiError> {
    let screen = api.open_stack(stack)?;
    let to = if card.is_some() { Move::At } else { Move::First };
    let card = api.card(stack, card, to)?;
    let mut grids = BTreeMap::new();
    for widget in screen.widgets.iter().filter(|w| w.kind == WidgetKind::Grid) {
        let page = api.grid_rows(stack, &widget.name, card.key.as_deref())?;
        grids.insert(widget.name.clone(), GridState { page, selected: 0 });
    }
    Ok(View {
        screen,
        card,
        is_new: false,
        designing: false,
        drawing: None,
        scripting: None,
        edits: BTreeMap::new(),
        grids,
        focus: None,
    })
}

/// What to tell the user about an error.
fn describe(error: &ApiError) -> String {
    match error {
        ApiError::User { message, .. } => message.clone(),
        ApiError::Other(message) => format!("Error: {message}"),
        ApiError::Version { .. } => error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{GridRow, KeyBinding};

    /// A tiny server in memory: a desktop, a stack list and a three-card stack.
    #[derive(Default)]
    struct FakeServer {
        calls: Vec<String>,
        contexts: Vec<Context>,
        effects: BTreeMap<String, Result<Vec<Effect>, ApiError>>,
        pending: Vec<Notification>,
        /// The images asked for, in order.
        images: Vec<String>,
        /// The card each call for the rows of a grid was made for.
        grid_cards: Vec<Option<String>>,
        /// Stacks that no longer exist.
        gone: Vec<String>,
        /// The version of the API the server claims, when not the current one.
        api: Option<u32>,
        /// True when the list of stacks is shown as icons.
        icons: bool,
        /// True when the script of a stack wants to know that a card came on
        /// screen.
        opening: bool,
    }

    impl FakeServer {
        fn answering(mut self, action: &str, effects: Vec<Effect>) -> Self {
            self.effects.insert(action.to_owned(), Ok(effects));
            self
        }

        /// What an action answers once the user has accepted its question.
        fn answering_once_confirmed(self, action: &str, effects: Vec<Effect>) -> Self {
            self.answering(&format!("{action} (confirmed)"), effects)
        }

        fn gone_if_missing(&self, stack: &str) -> Result<(), ApiError> {
            if self.gone.iter().any(|gone| gone == stack) {
                return Err(ApiError::User {
                    code: "XL002".into(),
                    message: format!("The stack “{stack}” does not exist."),
                });
            }
            Ok(())
        }

        fn failing(mut self, action: &str, message: &str) -> Self {
            self.effects.insert(
                action.to_owned(),
                Err(ApiError::User { code: "XL003".into(), message: message.into() }),
            );
            self
        }
    }

    fn screen(name: &str, grid: bool) -> Screen {
        let widgets = if grid {
            serde_json::json!([{ "name": "list", "kind": "grid", "ord": 10, "action": "open" }])
        } else {
            serde_json::json!([
                { "name": "city", "kind": "field", "ord": 10, "label": "City", "column": "city" },
                { "name": "photo", "kind": "image", "ord": 20, "column": "photo" },
                { "name": "logo", "kind": "image", "ord": 30, "props": { "image": "logo" } }
            ])
        };
        let mut screen: Screen = serde_json::from_value(serde_json::json!({
            "stack": { "name": name, "title": name, "kind": "user" },
            "source": { "has_cards": !grid, "count": 3 },
            "widgets": widgets, "actions": [], "keys": [],
        }))
        .unwrap();
        screen.keys.push(KeyBinding {
            key: "F8".into(),
            label: "Next".into(),
            action: "next_card".into(),
        });
        screen
    }

    impl Api for FakeServer {
        fn hello(&mut self) -> Result<Hello, ApiError> {
            Ok(Hello {
                api: self.api.unwrap_or(API_VERSION),
                version: "0".into(),
                instance: "test".into(),
                role: "ada".into(),
                author: false,
                home: "stacks".into(),
            })
        }

        fn palette(&mut self) -> Result<Vec<PaletteColour>, ApiError> {
            self.calls.push("palette".into());
            Ok(vec![PaletteColour { slot: 0, name: "black".into(), rgb: "#14121f".into() }])
        }

        fn image(&mut self, id: &str) -> Result<Option<Image>, ApiError> {
            self.images.push(id.to_owned());
            match id {
                "photo-missing" => Ok(None),
                "photo-broken" => Err(ApiError::Other("no such image".into())),
                _ => Ok(Some(Image { width: 1, height: 1, pixels: vec![id.len() as u8] })),
            }
        }

        fn open_stack(&mut self, stack: &str) -> Result<Screen, ApiError> {
            self.calls.push(format!("open_stack {stack}"));
            self.gone_if_missing(stack)?;
            let mut screen = screen(stack, stack == "stacks");
            if self.icons {
                for widget in screen.widgets.iter_mut().filter(|w| w.kind == WidgetKind::Grid) {
                    widget.props.insert("view".into(), "icons".into());
                    widget.props.insert("icon".into(), "icon".into());
                }
            }
            if self.opening && stack != "stacks" {
                screen.events.push("open".into());
            }
            Ok(screen)
        }

        fn card(&mut self, stack: &str, key: Option<&str>, to: Move) -> Result<Card, ApiError> {
            self.calls.push(format!("card {stack} {key:?} {}", to.as_str()));
            self.gone_if_missing(stack)?;
            let key = key.unwrap_or("1").to_owned();
            Ok(Card {
                values: BTreeMap::from([
                    ("city".to_string(), Some(format!("Ville {key}"))),
                    ("photo".to_string(), Some(format!("photo-{key}"))),
                ]),
                key: Some(key),
                position: 1,
                count: 3,
                ..Card::default()
            })
        }

        fn grid_rows(
            &mut self,
            stack: &str,
            widget: &str,
            card: Option<&str>,
        ) -> Result<GridPage, ApiError> {
            self.calls.push(format!("grid_rows {stack} {widget}"));
            self.grid_cards.push(card.map(str::to_owned));
            let row = |key: &str| GridRow { key: key.into(), cells: BTreeMap::new() };
            let mut rows = vec![row("addressbook"), row("recipes")];
            if self.icons {
                // One stack has an icon, the other has none.
                rows[0].cells.insert("icon".into(), None);
                rows[1].cells.insert("icon".into(), Some("icon-of-recipes".into()));
            }
            Ok(GridPage { total: 2, rows })
        }

        fn act(
            &mut self,
            stack: &str,
            action: &str,
            context: &Context,
        ) -> Result<Vec<Effect>, ApiError> {
            self.calls.push(format!("act {stack} {action}"));
            self.contexts.push(context.clone());
            let confirmed = format!("{action} (confirmed)");
            let name = if context.confirmed { confirmed.as_str() } else { action };
            self.effects.get(name).cloned().unwrap_or(Ok(vec![]))
        }

        fn notifications(&mut self) -> Vec<Notification> {
            std::mem::take(&mut self.pending)
        }
    }

    /// A session showing the "addressbook" stack on top of the stack list.
    fn on_addressbook(server: FakeServer) -> Session<FakeServer> {
        let server = server
            .answering("open", vec![Effect::Show { stack: "addressbook".into(), card: None }]);
        let mut session = Session::start(server).unwrap();
        session.activate("list");
        assert_eq!(session.view().stack(), "addressbook");
        session.api.calls.clear();
        session.api.contexts.clear();
        session
    }

    #[test]
    fn a_session_starts_on_the_home_stack() {
        let session = Session::start(FakeServer::default()).unwrap();
        assert_eq!(session.view().stack(), "stacks");
        assert_eq!(session.hello().role, "ada");
        assert_eq!(
            session.api.calls,
            ["palette", "open_stack stacks", "card stacks None first", "grid_rows stacks list"]
        );
    }

    #[test]
    fn a_server_of_another_version_is_refused() {
        let server = FakeServer { api: Some(1), ..FakeServer::default() };
        let error = Session::start(server).err().expect("a refusal");
        assert_eq!(error, ApiError::Version { client: API_VERSION, server: 1 });
        assert_eq!(
            error.to_string(),
            format!(
                "This client speaks version {API_VERSION} of the pgXila API and the server \
                 version 1: update the older of the two."
            )
        );
    }

    #[test]
    fn activating_a_grid_sends_its_selected_row() {
        let server = FakeServer::default()
            .answering("open", vec![Effect::Show { stack: "recipes".into(), card: None }]);
        let mut session = Session::start(server).unwrap();
        session.select_row("list", 1);
        session.select_row("list", 5);
        session.activate("list");

        assert_eq!(session.api.contexts[0].row.as_deref(), Some("recipes"));
        assert_eq!(session.view().stack(), "recipes");
    }

    #[test]
    fn a_key_runs_its_action_with_the_card_and_the_unsaved_edits() {
        let mut session = on_addressbook(FakeServer::default());
        session.edit("city", "Paris".into());

        assert!(session.press("F8"));
        assert!(!session.press("F5"), "F5 is not bound");

        assert_eq!(session.api.calls, ["act addressbook next_card"]);
        assert_eq!(
            session.api.contexts[0],
            Context {
                card: Some("1".into()),
                dirty: BTreeMap::from([("city".to_string(), "Paris".to_string())]),
                renderer: "text".into(),
                event: Some(Event::key("F8")),
                ..Context::default()
            }
        );
    }

    #[test]
    fn typing_the_saved_value_back_is_not_a_change() {
        let mut session = on_addressbook(FakeServer::default());
        session.edit("city", "Paris".into());
        session.edit("city", "Ville 1".into());
        assert!(session.view().dirty().is_empty());
        assert_eq!(session.view().value("city"), "Ville 1");
    }

    #[test]
    fn showing_another_card_of_the_same_stack_replaces_the_card_and_drops_the_edits() {
        let server = FakeServer::default().answering(
            "next_card",
            vec![Effect::Show { stack: "addressbook".into(), card: Some("2".into()) }],
        );
        let mut session = on_addressbook(server);
        session.edit("city", "Paris".into());
        session.run("next_card");

        assert_eq!(session.view().card.key.as_deref(), Some("2"));
        assert_eq!(session.view().value("city"), "Ville 2");
        assert!(session.view().edits.is_empty());
        assert_eq!(
            session.api.calls,
            ["act addressbook next_card", "card addressbook Some(\"2\") at"]
        );
    }

    #[test]
    fn back_returns_to_the_previous_screen_and_refreshes_it() {
        let mut session =
            on_addressbook(FakeServer::default().answering("back", vec![Effect::Back]));
        session.run("back");
        assert_eq!(session.view().stack(), "stacks");
        assert_eq!(
            session.api.calls,
            [
                "act addressbook back",
                "open_stack stacks",
                "card stacks Some(\"1\") at",
                "grid_rows stacks list"
            ]
        );

        // There is nothing behind the first screen.
        session.run("back");
        assert_eq!(session.view().stack(), "stacks");
    }

    #[test]
    fn an_error_for_the_user_is_shown_and_the_edits_are_kept() {
        let server =
            FakeServer::default().failing("save_card", "The field “last_name” is required.");
        let mut session = on_addressbook(server);
        session.edit("city", "Paris".into());
        session.run("save_card");

        assert_eq!(session.message(), Some("The field “last_name” is required."));
        assert_eq!(session.view().value("city"), "Paris");
    }

    #[test]
    fn a_message_lasts_until_the_next_action() {
        let server = FakeServer::default()
            .answering("save_card", vec![Effect::Message { text: "Card saved.".into() }]);
        let mut session = on_addressbook(server);
        session.run("save_card");
        assert_eq!(session.message(), Some("Card saved."));
        session.run("next_card");
        assert_eq!(session.message(), None);
    }

    #[test]
    fn a_new_card_is_empty_and_is_sent_without_a_key() {
        let mut session = on_addressbook(
            FakeServer::default()
                .answering("new_card", vec![Effect::New { values: BTreeMap::new() }]),
        );
        session.run("new_card");
        assert!(session.view().is_new);
        assert_eq!(session.view().value("city"), "");
        assert_eq!(session.view().card.count, 3);

        session.edit("city", "Lyon".into());
        session.run("save_card");
        let sent = session.api.contexts.last().unwrap();
        assert_eq!(sent.card, None);
        assert_eq!(sent.dirty["city"], "Lyon");
    }

    #[test]
    fn a_new_card_keeps_the_texts_that_depend_on_nothing() {
        let mut session = on_addressbook(
            FakeServer::default()
                .answering("new_card", vec![Effect::New { values: BTreeMap::new() }]),
        );
        let widgets: Vec<Widget> = serde_json::from_value(serde_json::json!([
            { "name": "heading", "kind": "text", "ord": 1, "props": { "template": "New stack" } },
            { "name": "title", "kind": "text", "ord": 2, "props": { "template": "{city}" } },
            { "name": "note", "kind": "text", "ord": 3, "label": "A note" }
        ]))
        .unwrap();
        session.view_mut().screen.widgets.extend(widgets);
        session.view_mut().card.texts.insert("title".into(), "Ville 1".into());
        assert_eq!(session.view().text("title"), "Ville 1");

        session.run("new_card");
        assert_eq!(session.view().text("heading"), "New stack");
        assert_eq!(session.view().text("title"), "", "there is no city yet");
        assert_eq!(session.view().text("note"), "A note");
        assert_eq!(session.view().text("nothing"), "");
    }

    #[test]
    fn a_new_card_may_come_with_some_fields_filled_in() {
        let values = BTreeMap::from([("city".to_string(), "London".to_string())]);
        let mut session = on_addressbook(
            FakeServer::default().answering("new_card", vec![Effect::New { values }]),
        );
        session.run("new_card");
        assert_eq!(session.view().value("city"), "London");

        session.run("save_card");
        let sent = session.api.contexts.last().unwrap();
        assert_eq!(sent.card, None);
        assert_eq!(sent.dirty["city"], "London", "what was filled in is sent like what is typed");
    }

    #[test]
    fn an_action_may_ask_before_it_goes_on() {
        let ask =
            vec![Effect::Confirm { text: "Delete this card?".into(), label: "Delete".into() }];
        let server = FakeServer::default().answering("delete", ask).answering_once_confirmed(
            "delete",
            vec![Effect::Message { text: "Card deleted.".into() }],
        );
        let mut session = on_addressbook(server);

        session.run("delete");
        let question = session.question().expect("a question is pending");
        assert_eq!(
            (question.text.as_str(), question.label.as_str()),
            ("Delete this card?", "Delete")
        );
        assert_eq!(session.message(), None);

        // Refusing does nothing at all.
        session.answer(false);
        assert_eq!(session.question(), None);
        assert_eq!(session.api.calls, ["act addressbook delete"]);

        // Accepting runs the same action again, and says so.
        session.run("delete");
        session.answer(true);
        assert_eq!(session.question(), None);
        assert_eq!(session.message(), Some("Card deleted."));
        let sent: Vec<bool> = session.api.contexts.iter().map(|c| c.confirmed).collect();
        assert_eq!(sent, [false, false, true]);
    }

    #[test]
    fn another_action_drops_a_pending_question() {
        let ask = vec![Effect::Confirm { text: "Sure?".into(), label: "Yes".into() }];
        let mut session = on_addressbook(FakeServer::default().answering("delete", ask));
        session.run("delete");
        session.run("next_card");
        assert_eq!(session.question(), None);
    }

    #[test]
    fn showing_a_stack_that_is_open_underneath_goes_back_to_it() {
        let server = FakeServer::default()
            .answering("author", vec![Effect::Show { stack: "author".into(), card: None }])
            .answering("browse", vec![Effect::Show { stack: "addressbook".into(), card: None }]);
        let mut session = on_addressbook(server);
        session.run("author");
        assert_eq!(session.view().stack(), "author");

        session.run("browse");
        assert_eq!(session.view().stack(), "addressbook");
        // The pile did not grow: going back leads to the list of stacks.
        let mut session = {
            session.api.effects.insert("back".into(), Ok(vec![Effect::Back]));
            session
        };
        session.run("back");
        assert_eq!(session.view().stack(), "stacks");
    }

    #[test]
    fn going_back_skips_a_screen_that_no_longer_exists() {
        let server = FakeServer::default()
            .answering("author", vec![Effect::Show { stack: "author".into(), card: None }])
            .answering("delete_stack", vec![Effect::Back]);
        let mut session = on_addressbook(server);
        session.run("author");

        // The stack underneath the dialog was deleted by the dialog.
        session.api.gone.push("addressbook".into());
        session.run("delete_stack");
        assert_eq!(session.view().stack(), "stacks");
        assert_eq!(session.message(), None);
    }

    #[test]
    fn a_grid_lists_what_belongs_to_the_card_on_screen() {
        let server = FakeServer::default().answering(
            "author",
            vec![Effect::Show { stack: "stacks".into(), card: Some("7".into()) }],
        );
        let mut session = Session::start(server).unwrap();
        assert_eq!(session.api.grid_cards, [Some("1".to_string())]);

        // Another card: the rows are read again for it.
        session.run("author");
        assert_eq!(session.api.grid_cards.last(), Some(&Some("7".to_string())));

        // A card that does not exist yet has no rows of its own.
        session.api.effects.insert("new".into(), Ok(vec![Effect::New { values: BTreeMap::new() }]));
        session.run("new");
        assert_eq!(session.api.grid_cards.last(), Some(&None));
    }

    #[test]
    fn a_row_can_be_selected_by_the_server() {
        let server = FakeServer::default()
            .answering("down", vec![Effect::Reload, Effect::Select { row: "recipes".into() }]);
        let mut session = Session::start(server).unwrap();
        session.run("down");
        assert_eq!(session.view().grids["list"].selected_key(), Some("recipes"));
        // Reading the rows again keeps the selection on its row.
        session.api.effects.insert("again".into(), Ok(vec![Effect::Reload]));
        session.run("again");
        assert_eq!(session.view().grids["list"].selected_key(), Some("recipes"));
    }

    #[test]
    fn a_change_to_another_stack_refreshes_a_list_that_may_show_it() {
        let mut session = Session::start(FakeServer::default()).unwrap();
        session.api.calls.clear();
        session.api.pending.push(Notification::Screen { stack: Some("recipes".into()) });
        assert!(session.poll());
        assert_eq!(session.api.calls, ["card stacks Some(\"1\") at", "grid_rows stacks list"]);
    }

    #[test]
    fn the_visual_editor_opens_on_the_stack_in_a_graphical_client() {
        let design = vec![Effect::Design { stack: "addressbook".into() }];
        let mut session = on_addressbook(FakeServer::default().answering("author", design.clone()));

        // A client that draws text has no such editor, and says so.
        session.run("author");
        assert!(!session.view().designing);
        assert_eq!(session.message(), Some("The visual editor needs the graphical client."));
        assert_eq!(session.api.contexts.last().unwrap().renderer, "text");

        // A graphical client says what it is, and edits the stack where it
        // is, on the card it shows.
        session.be_graphic();
        session.edit("city", "Paris".into());
        session.run("author");
        assert_eq!(session.api.contexts.last().unwrap().renderer, "graphic");
        assert!(session.view().designing);
        assert_eq!(session.view().stack(), "addressbook");
        assert_eq!(session.view().card.key.as_deref(), Some("1"));
        assert!(session.view().edits.is_empty(), "nothing is typed in the editor");

        session.stop_designing();
        assert!(!session.view().designing);

        // From the list of stacks, the stack is opened to be edited.
        let mut session =
            Session::start(FakeServer::default().answering("author", design)).unwrap();
        session.be_graphic();
        session.run("author");
        assert_eq!(session.view().stack(), "addressbook");
        assert!(session.view().designing);
    }

    #[test]
    fn a_tool_says_what_it_wants_and_is_asked_again_with_it() {
        let ask = vec![Effect::Confirm { text: "Take it off?".into(), label: "Remove".into() }];
        let server = FakeServer::default()
            .answering("remove_widget", ask)
            .answering_once_confirmed("remove_widget", vec![Effect::Reload]);
        let mut session = on_addressbook(server);
        let input = serde_json::json!({ "widget": "city" });

        session.run_with("remove_widget", input.clone());
        assert_eq!(session.api.contexts[0].input, Some(input.clone()));
        assert!(session.question().is_some());

        // Accepting sends the same request again, this time for good.
        session.api.calls.clear();
        session.answer(true);
        let sent = session.api.contexts.last().unwrap();
        assert!(sent.confirmed);
        assert_eq!(sent.input, Some(input));
        // Reloading reads the definition of the screen again, not only the card.
        assert_eq!(
            session.api.calls,
            [
                "act addressbook remove_widget",
                "open_stack addressbook",
                "card addressbook Some(\"1\") at"
            ]
        );
    }

    #[test]
    fn the_drawing_editor_opens_on_a_picture_with_its_image() {
        let draw = vec![Effect::Draw {
            target: "card".into(),
            widget: Some("photo".into()),
            image: Some("portrait".into()),
            width: 68,
            height: 80,
        }];
        let mut session = on_addressbook(FakeServer::default().answering("draw", draw));
        session.api.images.clear();

        // A terminal has no drawing editor, and says so.
        session.run("draw");
        assert_eq!(session.view().drawing, None);
        assert_eq!(session.message(), Some("Drawing needs the graphical client."));

        session.be_graphic();
        session.run("draw");
        let drawing = session.view().drawing.clone().expect("the editor is open");
        assert_eq!((drawing.target.as_str(), drawing.width, drawing.height), ("card", 68, 80));
        // The image it starts from was read.
        assert!(session.api.images.contains(&"portrait".to_string()));
        assert!(session.image("portrait").is_some());

        session.stop_drawing();
        assert_eq!(session.view().drawing, None);
    }

    #[test]
    fn the_icons_of_a_list_are_read_with_its_rows() {
        let server = FakeServer { icons: true, ..FakeServer::default() };
        let session = Session::start(server).unwrap();
        // One of the two rows has an icon; it is asked for once.
        assert_eq!(session.api.images, ["icon-of-recipes"]);
        assert!(session.image("icon-of-recipes").is_some());
    }

    #[test]
    fn quit_ends_the_session() {
        let mut session =
            on_addressbook(FakeServer::default().answering("quit", vec![Effect::Quit]));
        assert!(!session.should_quit());
        session.run("quit");
        assert!(session.should_quit());
    }

    #[test]
    fn a_change_made_elsewhere_refreshes_the_card_but_keeps_what_is_being_typed() {
        let mut session = on_addressbook(FakeServer::default());
        session.edit("city", "Paris".into());
        session
            .api
            .pending
            .push(Notification::Card { stack: Some("addressbook".into()), key: Some("1".into()) });

        assert!(session.poll());
        assert_eq!(session.api.calls, ["card addressbook Some(\"1\") at"]);
        assert_eq!(session.view().value("city"), "Paris");
    }

    #[test]
    fn a_change_in_another_stack_is_ignored() {
        let mut session = on_addressbook(FakeServer::default());
        session.api.pending.push(Notification::Card { stack: Some("recipes".into()), key: None });
        assert!(!session.poll());
        assert!(session.api.calls.is_empty());
    }

    #[test]
    fn a_changed_palette_is_read_again() {
        let mut session = on_addressbook(FakeServer::default());
        session.api.pending.push(Notification::Screen { stack: None });
        assert!(session.poll());
        assert_eq!(session.api.calls, ["palette"]);
        assert_eq!(session.palette().len(), 1);
    }

    #[test]
    fn a_changed_screen_definition_is_read_again() {
        let mut session = on_addressbook(FakeServer::default());
        session.api.pending.push(Notification::Screen { stack: Some("addressbook".into()) });
        assert!(session.poll());
        assert_eq!(
            session.api.calls,
            ["open_stack addressbook", "card addressbook Some(\"1\") at"]
        );
    }

    #[test]
    fn a_row_is_selected_by_its_position() {
        let mut session = Session::start(FakeServer::default()).unwrap();
        session.select("list", 1);
        assert_eq!(session.view().grids["list"].selected_key(), Some("recipes"));
        session.select("list", 99);
        assert_eq!(session.view().grids["list"].selected, 1, "there are two rows");
        session.select("nothing", 0);
    }

    #[test]
    fn the_images_of_a_screen_are_read_once() {
        let server = FakeServer::default().answering(
            "open",
            vec![Effect::Show { stack: "addressbook".into(), card: Some("2".into()) }],
        );
        let mut session = Session::start(server).unwrap();
        assert!(session.api().images.is_empty(), "the stack list shows no image");

        session.run("open");
        assert_eq!(session.api().images, ["photo-2", "logo"]);
        assert_eq!(session.image("photo-2").map(|image| image.pixels.clone()), Some(vec![7]));
        assert!(session.image("logo").is_some());

        // Showing the same card again asks for nothing more.
        session.run("open");
        assert_eq!(session.api().images.len(), 2);
    }

    #[test]
    fn an_image_that_cannot_be_read_is_not_asked_for_again() {
        let server = FakeServer::default()
            .answering(
                "missing",
                vec![Effect::Show { stack: "addressbook".into(), card: Some("missing".into()) }],
            )
            .answering(
                "broken",
                vec![Effect::Show { stack: "addressbook".into(), card: Some("broken".into()) }],
            );
        let mut session = Session::start(server).unwrap();
        session.run("missing");
        session.run("broken");
        session.run("missing");
        session.run("broken");

        assert_eq!(session.api().images, ["photo-missing", "logo", "photo-broken"]);
        assert!(session.image("photo-missing").is_none());
        assert!(session.image("photo-broken").is_none());
        assert_eq!(session.message(), None, "a missing image is not an error for the user");
    }

    // Scripts ----------------------------------------------------------------

    #[test]
    fn the_server_is_told_what_triggered_an_action() {
        let mut session = Session::start(FakeServer::default()).unwrap();
        session.activate("list");
        session.run("refresh");
        let events: Vec<_> = session.api.contexts.iter().map(|c| c.event.clone()).collect();
        assert_eq!(events, [Some(Event::click("list")), None]);
    }

    #[test]
    fn a_script_writes_on_the_card_as_if_the_user_had_typed() {
        let values = BTreeMap::from([
            ("city".to_string(), Some("London".to_string())),
            ("photo".to_string(), None),
        ]);
        let server = FakeServer::default().answering("script", vec![Effect::Set { values }]);
        let mut session = on_addressbook(server);
        session.run("script");

        assert_eq!(session.view().value("city"), "London");
        assert_eq!(session.view().value("photo"), "");
        // Nothing is saved: it is sent back like what the user types.
        assert_eq!(
            session.view().dirty(),
            BTreeMap::from([
                ("city".to_string(), "London".to_string()),
                ("photo".to_string(), String::new()),
            ])
        );
    }

    #[test]
    fn the_script_of_the_screen_is_told_when_a_card_comes_on_screen() {
        let server = FakeServer { opening: true, ..FakeServer::default() }
            .answering(
                "next_card",
                vec![Effect::Show { stack: "addressbook".into(), card: Some("2".into()) }],
            )
            .answering(
                "save_card",
                vec![
                    Effect::Show { stack: "addressbook".into(), card: Some("2".into()) },
                    Effect::Message { text: "Card saved.".into() },
                ],
            );
        let mut session = on_addressbook(server);

        // Another card: the script is told, with that card in the context.
        session.press("F8");
        assert_eq!(session.api.calls.last().unwrap(), "act addressbook script");
        let told = session.api.contexts.last().unwrap();
        assert_eq!(told.event, Some(Event::open()));
        assert_eq!(told.card.as_deref(), Some("2"));

        // The same card again is not news, and what the action said stays.
        session.api.calls.clear();
        session.run("save_card");
        assert!(!session.api.calls.iter().any(|call| call == "act addressbook script"));
        assert_eq!(session.message(), Some("Card saved."));
    }

    #[test]
    fn opening_a_stack_announces_its_first_card() {
        let server = FakeServer { opening: true, ..FakeServer::default() }
            .answering("open", vec![Effect::Show { stack: "addressbook".into(), card: None }])
            .answering("script", vec![Effect::Message { text: "Welcome.".into() }]);
        let mut session = Session::start(server).unwrap();
        session.activate("list");
        assert_eq!(session.api.calls.last().unwrap(), "act addressbook script");
        assert_eq!(session.message(), Some("Welcome."));
    }

    #[test]
    fn a_card_shown_by_the_script_of_the_screen_is_not_announced_again() {
        // A script that sends every card to the next one would never end.
        let server = FakeServer { opening: true, ..FakeServer::default() }
            .answering(
                "next_card",
                vec![Effect::Show { stack: "addressbook".into(), card: Some("2".into()) }],
            )
            .answering(
                "script",
                vec![Effect::Show { stack: "addressbook".into(), card: Some("3".into()) }],
            );
        let mut session = on_addressbook(server);
        session.press("F8");

        let scripts = session.api.calls.iter().filter(|call| *call == "act addressbook script");
        assert_eq!(scripts.count(), 1);
        assert_eq!(session.view().card.key.as_deref(), Some("3"));
    }

    #[test]
    fn a_question_of_a_script_runs_the_same_event_again() {
        let server = FakeServer::default()
            .answering(
                "script",
                vec![Effect::Confirm { text: "Sure?".into(), label: "Yes".into() }],
            )
            .answering_once_confirmed("script", vec![Effect::Message { text: "Done.".into() }]);
        let mut session = on_addressbook(server);
        session.press("F8");
        session.api.contexts.clear();
        session.perform("script", false, None, Some(Event::key("F6")));
        assert_eq!(session.question().map(|q| q.text.as_str()), Some("Sure?"));

        session.answer(true);
        let again = session.api.contexts.last().unwrap();
        assert!(again.confirmed);
        assert_eq!(again.event, Some(Event::key("F6")));
        assert_eq!(session.message(), Some("Done."));
    }

    #[test]
    fn the_script_editor_is_a_tool_of_the_window() {
        let open = Effect::Script {
            widget: Some("stamp".into()),
            title: "Script: button “Stamp”".into(),
            source: "function on_click() end".into(),
        };
        let tested = Effect::Tested { ok: false, line: Some(2), lines: vec!["Line 2: no".into()] };
        let server = FakeServer::default()
            .answering("edit_script", vec![open])
            .answering("test_script", vec![tested]);

        // A text client is told where scripts are written.
        let mut session = on_addressbook(server);
        session.run("edit_script");
        assert_eq!(session.view().scripting, None);
        assert_eq!(session.message(), Some("Scripts are written in the graphical client."));

        session.be_graphic();
        session.run("edit_script");
        let scripting = session.view().scripting.clone().expect("the editor is open");
        assert_eq!(scripting.widget.as_deref(), Some("stamp"));
        assert_eq!(scripting.source, "function on_click() end");
        assert_eq!(scripting.report, None);

        // What a test says is kept for the editor to show, until the script
        // changes.
        session.run("test_script");
        let report = session.view().scripting.as_ref().unwrap().report.clone().unwrap();
        assert_eq!((report.ok, report.line), (false, Some(2)));
        session.forget_report();
        assert_eq!(session.view().scripting.as_ref().unwrap().report, None);

        session.stop_scripting();
        assert_eq!(session.view().scripting, None);
    }
}
