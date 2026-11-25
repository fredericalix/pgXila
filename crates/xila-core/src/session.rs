//! The state of a client session, independent of how it is drawn.
//!
//! A session is a pile of views. Each view is one stack on screen: its
//! definition, the card being shown and what the user has typed but not saved.
//! The session holds no business rule: it sends the user's intent to the
//! server as an action, and applies the effects the server answers with.

use std::collections::BTreeMap;

use crate::api::{
    Api, ApiError, Card, Context, Effect, GridPage, Hello, Image, Move, Notification,
    PaletteColour, Screen, Widget, WidgetKind,
};

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

    /// What the server needs to know about this view to run an action.
    fn context(&self) -> Context {
        // The focused grid decides the selected row; a single grid needs no focus.
        let grid = self.focus.as_ref().and_then(|name| self.grids.get(name)).or_else(|| {
            if self.grids.len() == 1 {
                self.grids.values().next()
            } else {
                None
            }
        });
        Context {
            card: if self.is_new { None } else { self.card.key.clone() },
            dirty: self.dirty(),
            row: grid.and_then(|g| g.selected_key()).map(str::to_owned),
        }
    }
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
    quit: bool,
}

impl<A: Api> Session<A> {
    /// Greets the server and opens the home stack.
    pub fn start(mut api: A) -> Result<Self, ApiError> {
        let hello = api.hello()?;
        let palette = api.palette()?;
        let home = open(&mut api, &hello.home, None)?;
        let mut session = Session {
            api,
            hello,
            palette,
            views: vec![home],
            images: BTreeMap::new(),
            message: None,
            quit: false,
        };
        session.fetch_images();
        Ok(session)
    }

    /// The server this session talks to.
    pub fn api(&self) -> &A {
        &self.api
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
        let wanted: Vec<String> = view
            .widgets()
            .filter(|widget| widget.kind == WidgetKind::Image)
            .filter_map(|widget| view.image_id(widget))
            .filter(|id| !self.images.contains_key(*id))
            .map(str::to_owned)
            .collect();
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
                self.run(&action);
                true
            }
            None => false,
        }
    }

    /// Runs the action of a widget: a button, or a grid for its selected row.
    pub fn activate(&mut self, widget: &str) {
        let action = self
            .view()
            .screen
            .widgets
            .iter()
            .find(|w| w.name == widget)
            .and_then(|w| w.action.clone());
        if let Some(action) = action {
            self.run(&action);
        }
    }

    /// Asks the server to run an action and applies the effects it answers.
    pub fn run(&mut self, action: &str) {
        self.message = None;
        let stack = self.view().stack().to_owned();
        let context = self.view().context();
        let outcome = self
            .api
            .act(&stack, action, &context)
            .and_then(|effects| effects.into_iter().try_for_each(|effect| self.apply(effect)));
        if let Err(error) = outcome {
            self.message = Some(describe(&error));
        }
        self.fetch_images();
    }

    fn apply(&mut self, effect: Effect) -> Result<(), ApiError> {
        match effect {
            Effect::Show { stack, card } => {
                if stack == self.view().stack() {
                    self.load_card(card.as_deref(), Move::At)?;
                } else {
                    let view = open(&mut self.api, &stack, card.as_deref())?;
                    self.views.push(view);
                }
            }
            Effect::Back => {
                if self.views.len() > 1 {
                    self.views.pop();
                    // What is underneath may have changed in the meantime.
                    self.refresh()?;
                }
            }
            Effect::Reload => self.refresh()?,
            Effect::Focus { widget } => self.view_mut().focus = Some(widget),
            Effect::Message { text } => self.message = Some(text),
            Effect::New => {
                let view = self.view_mut();
                view.card = Card { count: view.card.count, ..Card::default() };
                view.is_new = true;
                view.edits.clear();
                view.focus = None;
            }
            Effect::Quit => self.quit = true,
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
        Ok(())
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
        let names: Vec<String> = self.view().grids.keys().cloned().collect();
        for name in names {
            let page = self.api.grid_rows(&stack, &name)?;
            let grid = self.view_mut().grids.get_mut(&name).expect("the grid was just listed");
            grid.selected = grid.selected.min(page.rows.len().saturating_sub(1));
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
        self.view_mut().screen = screen;
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
        let page = api.grid_rows(stack, &widget.name)?;
        grids.insert(widget.name.clone(), GridState { page, selected: 0 });
    }
    Ok(View { screen, card, is_new: false, edits: BTreeMap::new(), grids, focus: None })
}

/// What to tell the user about an error.
fn describe(error: &ApiError) -> String {
    match error {
        ApiError::User { message, .. } => message.clone(),
        ApiError::Other(message) => format!("Error: {message}"),
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
    }

    impl FakeServer {
        fn answering(mut self, action: &str, effects: Vec<Effect>) -> Self {
            self.effects.insert(action.to_owned(), Ok(effects));
            self
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
                api: 1,
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
            Ok(screen(stack, stack == "stacks"))
        }

        fn card(&mut self, stack: &str, key: Option<&str>, to: Move) -> Result<Card, ApiError> {
            self.calls.push(format!("card {stack} {key:?} {}", to.as_str()));
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

        fn grid_rows(&mut self, stack: &str, widget: &str) -> Result<GridPage, ApiError> {
            self.calls.push(format!("grid_rows {stack} {widget}"));
            let row = |key: &str| GridRow { key: key.into(), cells: BTreeMap::new() };
            Ok(GridPage { total: 2, rows: vec![row("addressbook"), row("recipes")] })
        }

        fn act(
            &mut self,
            stack: &str,
            action: &str,
            context: &Context,
        ) -> Result<Vec<Effect>, ApiError> {
            self.calls.push(format!("act {stack} {action}"));
            self.contexts.push(context.clone());
            self.effects.get(action).cloned().unwrap_or(Ok(vec![]))
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
                row: None,
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
            ["act addressbook back", "card stacks Some(\"1\") at", "grid_rows stacks list"]
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
        let mut session =
            on_addressbook(FakeServer::default().answering("new_card", vec![Effect::New]));
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
}
