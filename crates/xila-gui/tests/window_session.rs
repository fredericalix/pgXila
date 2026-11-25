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

use xila_core::input::Key;
use xila_core::login::open_session;
use xila_core::server::Server;
use xila_gui::app::{App, Options};
use xila_gui::chrome::{content_of, window_around};
use xila_gui::font::Fonts;
use xila_gui::frame::{Frame, Rect, HEIGHT, WIDTH};
use xila_gui::layout::layout;
use xila_gui::palette::{slot, Palette};

/// Palette slot of the skin of the portraits of the example.
const SKIN: u8 = 28;

fn var(name: &str) -> String {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name} is not set: use scripts/test-integration.sh"))
}

fn app() -> App<Server> {
    let options = Options {
        host: var("PGXILA_TEST_HOST"),
        port: var("PGXILA_TEST_SCRATCH_PORT").parse().expect("PGXILA_TEST_SCRATCH_PORT"),
        database: var("PGXILA_TEST_DATABASE"),
        user: Some(var("PGXILA_TEST_USER")),
        ca: Some(PathBuf::from(var("PGXILA_TEST_SCRATCH_CA"))),
    };
    App::new(options, Box::new(open_session))
}

fn connected() -> App<Server> {
    let mut app = app();
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
    assert!(count(&list, area_of(&app, "list"), slot::BLUE) > 1000, "a row is selected");

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

    // The first row is selected: a click on it opens it.
    let list = area_of(&app, "list");
    click(&mut app, Rect::new(list.x, list.y + 14, list.w, 12));
    assert_eq!(stack(&app), "addressbook");

    let next = area_of(&app, "next");
    click(&mut app, next);
    assert_eq!(app.session().unwrap().view().card.position, 2);
    let previous = area_of(&app, "previous");
    click(&mut app, previous);
    assert_eq!(app.session().unwrap().view().card.position, 1);
}
