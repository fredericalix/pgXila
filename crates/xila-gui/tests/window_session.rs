//! Integration tests of the graphical client against a real pgXila server.
//!
//! The client is driven without its window: keys go in, a frame comes out.
//! The tests run against the throwaway "scratch" server, where the example
//! stack `addressbook` is loaded, and are ignored by default. Run them with
//! `scripts/test-integration.sh`.
//!
//! Set `XILA_SNAPSHOT_DIR` to a directory to get what the screens look like,
//! as PPM pictures.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use xila_core::api::WidgetKind;
use xila_core::input::Key;
use xila_core::login::open_session;
use xila_core::server::Server;
use xila_gui::app::{App, Options};
use xila_gui::chrome::{content_of, window_around};
use xila_gui::design::{palette_cells, Cell, Tool};
use xila_gui::font::Fonts;
use xila_gui::frame::{Frame, Rect, HEIGHT, WIDTH};
use xila_gui::layout::layout;
use xila_gui::palette::{slot, Palette};
use xila_gui::screen::tile_area;

/// Palette slot of the skin of the portraits of the example.
const SKIN: u8 = 28;
/// Palette slot of the cover of the notebook that is the icon of the example.
const ORANGE: u8 = 24;

fn var(name: &str) -> String {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name} is not set: use scripts/test-integration.sh"))
}

fn app() -> App<Server> {
    app_of("PGXILA_TEST_USER")
}

/// The client of the user named by an environment variable.
fn app_of(user_variable: &str) -> App<Server> {
    let options = Options {
        host: var("PGXILA_TEST_HOST"),
        port: var("PGXILA_TEST_SCRATCH_PORT").parse().expect("PGXILA_TEST_SCRATCH_PORT"),
        database: var("PGXILA_TEST_DATABASE"),
        user: Some(var(user_variable)),
        ca: Some(PathBuf::from(var("PGXILA_TEST_SCRATCH_CA"))),
    };
    App::new(options, Box::new(open_session))
}

fn connected() -> App<Server> {
    connected_as("PGXILA_TEST_USER")
}

fn connected_as(user_variable: &str) -> App<Server> {
    let mut app = app_of(user_variable);
    app.connect_with(&var("PGXILA_TEST_PASSWORD"));
    assert!(app.session().is_some(), "{:?}", app.login_message());
    app
}

/// Draws the screen, and keeps a picture of it when asked to.
fn render(app: &App<Server>, name: &str) -> Frame {
    let mut frame = Frame::new();
    app.draw(&mut frame, "16:54", true);
    if let Ok(directory) = std::env::var("XILA_SNAPSHOT_DIR") {
        let palette = app.palette();
        let mut data = format!("P6\n{WIDTH} {HEIGHT}\n255\n").into_bytes();
        for slot in frame.pixels() {
            let rgb = palette.rgb(*slot);
            data.extend([(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8]);
        }
        std::fs::write(PathBuf::from(directory).join(format!("{name}.ppm")), data).unwrap();
    }
    frame
}

/// The rectangle of a widget of the current screen.
fn area_of(app: &App<Server>, widget: &str) -> Rect {
    let view = app.session().unwrap().view();
    let layout = layout(&view.screen, &view.card.widgets, &Fonts::builtin());
    let content = content_of(window_around(layout.width, layout.height));
    let item = layout.item(widget).unwrap_or_else(|| panic!("no widget {widget}"));
    item.area.moved(content.x, content.y)
}

fn count(frame: &Frame, area: Rect, slot: u8) -> usize {
    (area.y..area.bottom())
        .flat_map(|y| (area.x..area.right()).map(move |x| (x, y)))
        .filter(|(x, y)| frame.pixel(*x, *y) == Some(slot))
        .count()
}

fn type_text(app: &mut App<Server>, text: &str) {
    for c in text.chars() {
        app.key(Key::Char(c));
    }
}

fn click(app: &mut App<Server>, area: Rect) {
    let (x, y) = (area.x + area.w / 2, area.y + area.h / 2);
    app.pointer_down(x, y);
    app.pointer_up(x, y);
}

fn stack(app: &App<Server>) -> String {
    app.session().unwrap().view().stack().to_owned()
}

fn value(app: &App<Server>, column: &str) -> String {
    app.session().unwrap().view().value(column).to_owned()
}

#[test]
#[ignore = "needs the scratch server: use scripts/test-integration.sh"]
fn the_built_in_palette_is_the_one_of_the_server() {
    let app = connected();
    assert_eq!(app.palette(), Palette::boot());
}

#[test]
#[ignore = "needs the scratch server: use scripts/test-integration.sh"]
fn a_wrong_password_is_explained_on_the_connection_screen() {
    let mut app = app();
    render(&app, "login");
    type_text(&mut app, "certainly not the password");
    app.key(Key::Enter);
    assert!(app.session().is_none());
    assert_eq!(app.login_message(), Some("Wrong user name or password."));
    let frame = render(&app, "login-refused");
    assert!(count(&frame, Rect::new(0, 272, WIDTH, 13), slot::YELLOW) > 4000);
}

#[test]
#[ignore = "needs the scratch server: use scripts/test-integration.sh"]
fn the_keyboard_goes_from_the_desktop_to_a_card_with_its_picture() {
    let mut app = connected();
    assert_eq!(stack(&app), "desktop");
    let desktop = render(&app, "desktop");
    // The welcome window says who is connected, with the main button below.
    assert!(count(&desktop, area_of(&app, "user"), slot::INK) > 20);
    assert!(count(&desktop, area_of(&app, "stacks"), slot::YELLOW) > 500);

    app.key(Key::Enter);
    assert_eq!(stack(&app), "stacks");
    let list = render(&app, "stacks");
    // The stacks are shown as icons; the first one is selected, and has the
    // notebook of the example as its icon.
    let stacks = area_of(&app, "list");
    assert!(count(&list, stacks, slot::BLUE) > 300, "a stack is selected");
    assert!(count(&list, tile_area(stacks, 0), ORANGE) > 1000, "the icon, twice its size");

    app.key(Key::Enter);
    assert_eq!(stack(&app), "addressbook");
    let card = render(&app, "addressbook");
    assert_eq!(value(&app, "last_name"), "Babbage");

    // The picture came from xila.image and is drawn in its frame.
    let photo = value(&app, "photo");
    let image = app.session().unwrap().image(&photo).expect("the picture of the card");
    assert_eq!((image.width, image.height), (68, 80));
    let frame = area_of(&app, "photo");
    assert_eq!((frame.w, frame.h), (70, 82));
    assert!(count(&card, frame, SKIN) > 300);

    // Another card, another picture.
    app.key(Key::F(8));
    assert_eq!(value(&app, "last_name"), "Dijkstra");
    let next = value(&app, "photo");
    assert_ne!(next, photo);
    assert!(app.session().unwrap().image(&next).is_some());
    render(&app, "addressbook-next");

    app.key(Key::F(12));
    assert_eq!(stack(&app), "stacks");
    app.key(Key::F(3));
    assert!(app.should_quit());
}

#[test]
#[ignore = "needs the scratch server: use scripts/test-integration.sh"]
fn typing_in_a_field_and_enter_writes_the_card() {
    let mut app = connected();
    app.key(Key::Enter);
    app.key(Key::Enter);
    assert_eq!(stack(&app), "addressbook");

    // Down to the city, which is the sixth field.
    for _ in 0..5 {
        app.key(Key::Tab);
    }
    assert_eq!(app.session().unwrap().view().focus.as_deref(), Some("city"));
    type_text(&mut app, " (été)");
    app.key(Key::Enter);
    assert_eq!(value(&app, "city"), "London (été)");
    let session = app.session().unwrap();
    assert!(session.view().dirty().is_empty(), "{:?}", session.message());
    render(&app, "addressbook-saved");

    // A second client sees what was written.
    let mut other = connected();
    other.key(Key::Enter);
    other.key(Key::Enter);
    assert_eq!(value(&other, "city"), "London (été)");

    // Put the card back, and see the other client follow by itself.
    for _ in 0..6 {
        app.key(Key::Backspace);
    }
    app.key(Key::Enter);
    assert_eq!(value(&app, "city"), "London");
    let deadline = Instant::now() + Duration::from_secs(5);
    while value(&other, "city") != "London" {
        assert!(Instant::now() < deadline, "the change was not pushed");
        other.tick();
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
#[ignore = "needs the scratch server: use scripts/test-integration.sh"]
fn the_pointer_opens_a_stack_and_browses_it() {
    let mut app = connected();
    let button = area_of(&app, "stacks");
    click(&mut app, button);
    assert_eq!(stack(&app), "stacks");

    // The first stack is selected: a click on its icon opens it.
    let list = area_of(&app, "list");
    click(&mut app, tile_area(list, 0));
    assert_eq!(stack(&app), "addressbook");

    let next = area_of(&app, "next");
    click(&mut app, next);
    assert_eq!(app.session().unwrap().view().card.position, 2);
    let previous = area_of(&app, "previous");
    click(&mut app, previous);
    assert_eq!(app.session().unwrap().view().card.position, 1);
}

fn designing(app: &App<Server>) -> bool {
    app.session().unwrap().view().designing
}

/// The rectangle the server holds for a widget in the window, if it has one.
fn place_of(app: &App<Server>, widget: &str) -> Option<(i32, i32, i32, i32)> {
    let view = app.session().unwrap().view();
    let widget = view.widgets().find(|w| w.name == widget)?;
    let place = widget.layout.get("graphic").filter(|place| place.w > 0)?;
    Some((place.x, place.y, place.w, place.h))
}

/// Where a widget is in the content of its window, whatever the size of the
/// window and so wherever the window is on the screen.
fn content_place(app: &App<Server>, widget: &str) -> Rect {
    let view = app.session().unwrap().view();
    let layout = layout(&view.screen, &view.card.widgets, &Fonts::builtin());
    layout.item(widget).unwrap_or_else(|| panic!("no widget {widget}")).area
}

fn field_names(app: &App<Server>) -> Vec<String> {
    let view = app.session().unwrap().view();
    let fields = view.screen.widgets.iter().filter(|w| w.kind == WidgetKind::Field);
    fields.map(|w| w.name.clone()).collect()
}

/// Adds a field from the editor: F6, Field, then the dialog.
fn add_field(app: &mut App<Server>, label: &str, spaces_on_type: usize) {
    app.key(Key::F(6));
    app.key(Key::Enter);
    assert_eq!(stack(app), "field", "{:?}", app.session().unwrap().message());
    type_text(app, label);
    app.key(Key::Tab);
    app.key(Key::Tab);
    for _ in 0..spaces_on_type {
        app.key(Key::Char(' '));
    }
    app.key(Key::Enter);
    assert!(designing(app), "{:?}", app.session().unwrap().message());
}

#[test]
#[ignore = "needs the scratch server: use scripts/test-integration.sh"]
fn an_author_builds_a_stack_in_the_window() {
    // The scratch server is new for each run: the name is free.
    let mut app = connected_as("PGXILA_TEST_AUTHOR");

    // My stacks, then F6: the dialog of a new stack.
    app.key(Key::Enter);
    app.key(Key::F(6));
    assert_eq!(stack(&app), "new_stack");
    type_text(&mut app, "Window test");
    let dialog = render(&app, "new-stack");
    assert!(count(&dialog, area_of(&app, "create"), slot::YELLOW) > 300);
    app.key(Key::Enter);

    // The new stack opens in the visual editor. It has no field yet.
    assert_eq!(stack(&app), "window_test", "{:?}", app.session().unwrap().message());
    assert!(designing(&app));
    render(&app, "editor-empty");

    // Two fields, each through the Field dialog: a date and a text.
    add_field(&mut app, "Opened on", 2);
    add_field(&mut app, "Place", 0);
    assert_eq!(field_names(&app), ["opened_on", "place"]);
    assert_eq!(place_of(&app, "place"), None, "the layout is automatic");
    let editor = render(&app, "editor-two-fields");
    assert!(count(&editor, area_of(&app, "place"), slot::SILVER) > 10, "the name of the column");

    // The second field is dragged 40 pixels down with the pointer.
    let before = content_place(&app, "place");
    let on_screen = area_of(&app, "place");
    let (x, y) = (on_screen.x + on_screen.w / 2, on_screen.y + on_screen.h / 2);
    app.pointer_down(x, y);
    app.pointer_move(x, y + 20);
    render(&app, "editor-dragging");
    app.pointer_move(x, y + 40);
    app.pointer_up(x, y + 40);
    let after = content_place(&app, "place");
    assert!((after.y - (before.y + 40)).abs() <= 2, "{before:?} {after:?}");
    assert_eq!((after.x, after.w, after.h), (before.x, before.w, before.h));
    assert_eq!(place_of(&app, "place"), Some((after.x, after.y, after.w, after.h)));
    // The server now holds the place of every widget, not only of that one.
    for widget in ["heading", "opened_on", "place", "previous", "next", "new", "delete"] {
        assert!(place_of(&app, widget).is_some(), "{widget}");
    }
    render(&app, "editor-moved");

    // F2 undoes it: the layout is automatic again.
    app.key(Key::F(2));
    assert_eq!(app.session().unwrap().message(), Some("Undone: Layout changed."));
    assert_eq!(place_of(&app, "place"), None);
    assert_eq!(content_place(&app, "place"), before);

    // A text, with the tool of the palette, then its words in Properties.
    let (_, text_tool) =
        palette_cells().into_iter().find(|(cell, _)| *cell == Cell::Tool(Tool::Text)).unwrap();
    click(&mut app, text_tool);
    let place = area_of(&app, "place");
    app.pointer_down(place.x, place.bottom() + 30);
    app.pointer_up(place.x, place.bottom() + 30);
    assert!(place_of(&app, "text1").is_some(), "{:?}", app.session().unwrap().message());
    app.key(Key::Enter);
    type_text(&mut app, " here");
    app.key(Key::Enter);
    let view = app.session().unwrap().view();
    let text = view.widgets().find(|w| w.name == "text1").unwrap();
    assert_eq!(text.props["template"], "Text here");
    render(&app, "editor-text");

    // F4: the forms of the stack, with its two fields; Escape comes back.
    app.key(Key::F(4));
    assert_eq!(stack(&app), "author");
    assert_eq!(app.session().unwrap().view().grids["fields"].page.rows.len(), 2);
    app.key(Key::Escape);
    assert!(designing(&app));

    // F11 shows the stack as its users see it, and F11 again edits it.
    app.key(Key::F(11));
    assert_eq!(stack(&app), "window_test");
    assert!(!designing(&app));
    render(&app, "built-stack");
    assert_eq!(app.session().unwrap().view().card.count, 0);
    app.key(Key::F(11));
    assert!(designing(&app));

    // The stack is deleted from its forms; the question is asked in a window.
    app.key(Key::F(4));
    let delete = area_of(&app, "delete_stack");
    click(&mut app, delete);
    assert!(app.session().unwrap().question().is_some());
    render(&app, "delete-question");
    app.key(Key::Escape);
    assert!(app.session().unwrap().question().is_none());
    click(&mut app, delete);
    app.key(Key::Enter);
    assert_eq!(stack(&app), "stacks", "{:?}", app.session().unwrap().message());
}

/// Clicks on a pixel of the picture open in the drawing editor.
fn dab(app: &mut App<Server>, pixel: (i32, i32)) {
    let square = app.ui().unwrap().painter.square_of(pixel);
    click(app, square);
}

#[test]
#[ignore = "needs the scratch server: use scripts/test-integration.sh"]
fn a_user_draws_the_picture_of_a_card_and_history_brings_the_old_one_back() {
    // A plain user: the picture of a card is theirs to draw.
    let mut app = connected();
    app.key(Key::Enter);
    app.key(Key::Enter);
    assert_eq!(stack(&app), "addressbook");
    let original = value(&app, "photo");

    // A click on the photo opens the drawing editor on it.
    let photo = area_of(&app, "photo");
    click(&mut app, photo);
    let drawing = app.session().unwrap().view().drawing.clone();
    let drawing = drawing.unwrap_or_else(|| panic!("{:?}", app.session().unwrap().message()));
    assert_eq!((drawing.width, drawing.height), (68, 80));
    assert_eq!(drawing.image.as_deref(), Some(original.as_str()));
    let editor = render(&app, "draw-portrait");
    assert!(count(&editor, Rect::new(0, 15, 310, 255), SKIN) > 500, "the portrait, enlarged");

    // A red dot on the nose, then out: the drawing is saved as a new image.
    let red = 25;
    let swatch = xila_gui::draw::swatches(&(0..32).collect::<Vec<u8>>())
        .into_iter()
        .find(|(colour, _)| *colour == red)
        .unwrap()
        .1;
    click(&mut app, swatch);
    dab(&mut app, (34, 40));
    app.key(Key::F(12));
    assert!(app.session().unwrap().view().drawing.is_none());
    assert_eq!(app.session().unwrap().message(), Some("Picture saved."));
    let drawn = value(&app, "photo");
    assert_ne!(drawn, original, "an image is never changed: the card has another one");
    let image = app.session().unwrap().image(&drawn).expect("the new picture was read").clone();
    assert_eq!(image.pixel(34, 40), Some(red));
    let card = render(&app, "addressbook-drawn");
    assert!(count(&card, area_of(&app, "photo"), red) >= 1);

    // The history of the cards undoes it: the card shows the old image again.
    app.key(Key::F(10));
    assert_eq!(stack(&app), "card_history");
    app.key(Key::Enter);
    app.key(Key::Enter);
    assert_eq!(stack(&app), "addressbook", "{:?}", app.session().unwrap().message());
    assert_eq!(value(&app, "photo"), original);
}

#[test]
#[ignore = "needs the scratch server: use scripts/test-integration.sh"]
fn an_author_draws_the_icon_of_a_stack() {
    // The scratch server is new for each run: the name is free.
    let mut app = connected_as("PGXILA_TEST_AUTHOR");
    app.key(Key::Enter);
    app.key(Key::F(6));
    type_text(&mut app, "Icon test");
    app.key(Key::Enter);
    assert!(designing(&app), "{:?}", app.session().unwrap().message());

    // F5 in the visual editor: the icon, 28 by 26, empty.
    app.key(Key::F(5));
    let drawing = app.session().unwrap().view().drawing.clone();
    let drawing = drawing.unwrap_or_else(|| panic!("{:?}", app.session().unwrap().message()));
    assert_eq!((drawing.target.as_str(), drawing.width, drawing.height), ("icon", 28, 26));

    // A filled square, with the keyboard and the pointer.
    app.key(Key::Char('x'));
    app.key(Key::Char('r'));
    let from = app.ui().unwrap().painter.square_of((4, 4));
    let to = app.ui().unwrap().painter.square_of((23, 21));
    app.pointer_down(from.x + 2, from.y + 2);
    app.pointer_move(to.x + 2, to.y + 2);
    app.pointer_up(to.x + 2, to.y + 2);
    render(&app, "draw-icon");
    app.key(Key::F(12));
    assert_eq!(app.session().unwrap().message(), Some("Picture saved."));
    assert!(designing(&app), "back in the visual editor");

    // The list of stacks shows it, twice its size.
    app.key(Key::F(12));
    assert_eq!(stack(&app), "stacks");
    let view = app.session().unwrap().view();
    let rows = &view.grids["list"].page.rows;
    let at = rows.iter().position(|row| row.key == "icon_test").expect("the stack is listed");
    let icon = rows[at].cells["icon"].clone().expect("it has an icon");
    let image = app.session().unwrap().image(&icon).expect("the icon was read");
    assert_eq!((image.pixel(4, 4), image.pixel(3, 3)), (Some(0), Some(255)));
    let list = render(&app, "stacks-icons");
    let tile = tile_area(area_of(&app, "list"), at);
    assert!(count(&list, tile, 0) > 20 * 18 * 4 - 100, "a black square of 20 by 18, doubled");

    // The stack is deleted: select it, author mode, its forms, Delete.
    for _ in 0..at {
        app.key(Key::Right);
    }
    app.key(Key::F(11));
    assert!(designing(&app));
    app.key(Key::F(4));
    let delete = area_of(&app, "delete_stack");
    click(&mut app, delete);
    app.key(Key::Enter);
    assert_eq!(stack(&app), "stacks", "{:?}", app.session().unwrap().message());
}

/// Types a text of several lines in the script editor: a line feed is the
/// Enter key.
fn type_lines(app: &mut App<Server>, text: &str) {
    for c in text.chars() {
        app.key(if c == '\n' { Key::Enter } else { Key::Char(c) });
    }
}

#[test]
#[ignore = "needs running pgXila servers: use scripts/test-integration.sh"]
fn an_author_writes_the_script_of_a_button_in_the_window() {
    let mut app = connected_as("PGXILA_TEST_AUTHOR");

    // A new stack with one field; it opens in the visual editor.
    app.key(Key::Enter);
    app.key(Key::F(6));
    type_text(&mut app, "Script test");
    app.key(Key::Enter);
    assert_eq!(stack(&app), "script_test", "{:?}", app.session().unwrap().message());
    add_field(&mut app, "Place", 0);

    // The button "New" is selected, and F8 opens its script: there is none.
    let target = area_of(&app, "new");
    click(&mut app, target);
    app.key(Key::F(8));
    let session = app.session().unwrap();
    let scripting = session.view().scripting.clone().expect("the script editor is open");
    assert_eq!(scripting.title, "Script: button “New”", "{:?}", session.message());
    assert_eq!(scripting.source, "");
    render(&app, "script-empty");

    // The script is typed as one types it: the editor indents.
    type_lines(
        &mut app,
        "function on_click()\nlocal new = stack.new_card()\nnew.place = \"London\"\n\
         go_to(new)\nfocus(\"place\")\nend",
    );
    let written =
        "function on_click()\n  local new = stack.new_card()\n  new.place = \"London\"\n  \
                   go_to(new)\n  focus(\"place\")\nend";
    assert_eq!(app.ui().unwrap().scripter.buffer.text(), written);

    // F5 tries it: the server says what it would do, and keeps nothing.
    app.key(Key::F(5));
    let session = app.session().unwrap();
    let report = session.view().scripting.as_ref().unwrap().report.clone().expect("a report");
    assert!(report.ok, "{report:?}");
    assert_eq!(
        report.lines,
        ["It shows a new card with place = London.", "It puts the cursor in “place”."]
    );
    let tested = render(&app, "script-tested");
    assert!(count(&tested, Rect::new(337, 129, 138, 139), slot::INK) > 200, "the report is shown");

    // A mistake is found by the test, with its line.
    app.key(Key::Up);
    app.key(Key::Enter);
    type_text(&mut app, "nothing()");
    app.key(Key::F(5));
    let session = app.session().unwrap();
    let report = session.view().scripting.as_ref().unwrap().report.clone().expect("a report");
    assert_eq!((report.ok, report.line), (false, Some(6)), "{report:?}");
    render(&app, "script-error");
    app.key(Key::Undo);
    app.key(Key::Undo);
    assert_eq!(app.ui().unwrap().scripter.buffer.text(), written);

    // Leaving saves; the visual editor is back, and the button runs a script.
    app.key(Key::F(12));
    let session = app.session().unwrap();
    assert_eq!(session.message(), Some("Script saved."));
    assert!(session.view().scripting.is_none() && designing(&app));
    let button = session.view().widgets().find(|w| w.name == "new").unwrap();
    assert_eq!(button.action.as_deref(), Some("script"));

    // Browsing the stack: the button does what its script says.
    app.key(Key::F(11));
    assert!(!designing(&app));
    let target = area_of(&app, "new");
    click(&mut app, target);
    let session = app.session().unwrap();
    assert!(session.view().is_new, "{:?}", session.message());
    assert_eq!(value(&app, "place"), "London");
    assert_eq!(session.view().focus.as_deref(), Some("place"));
    render(&app, "script-ran");
    // The card is the user's to complete and save.
    type_text(&mut app, ", Baker Street");
    app.key(Key::Enter);
    let session = app.session().unwrap();
    assert_eq!(session.message(), Some("Card saved."));
    assert_eq!(value(&app, "place"), "London, Baker Street");

    // The stack is deleted, as it was made, from its forms.
    app.key(Key::F(11));
    app.key(Key::F(4));
    let target = area_of(&app, "delete_stack");
    click(&mut app, target);
    app.key(Key::Enter);
    assert_eq!(stack(&app), "stacks", "{:?}", app.session().unwrap().message());
}
