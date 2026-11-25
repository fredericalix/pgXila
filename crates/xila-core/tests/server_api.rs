//! Integration tests of the client API against a real pgXila server.
//!
//! They run against the throwaway "scratch" server, where the example stack
//! `addressbook` is loaded, and are ignored by default. Run them with
//! `scripts/test-integration.sh`.
//!
//! The tests share one stack and each one puts back what it changes, so they
//! must run one at a time (the script does that).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use xila_core::api::{Api, ApiError, Context, Effect, Move, Notification, WidgetKind};
use xila_core::connection::ConnectParams;
use xila_core::server::Server;
use xila_core::session::Session;

fn var(name: &str) -> String {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name} is not set: use scripts/test-integration.sh"))
}

fn params(user_variable: &str) -> ConnectParams {
    ConnectParams {
        host: var("PGXILA_TEST_HOST"),
        port: var("PGXILA_TEST_SCRATCH_PORT").parse().expect("PGXILA_TEST_SCRATCH_PORT"),
        database: var("PGXILA_TEST_DATABASE"),
        user: var(user_variable),
        password: var("PGXILA_TEST_PASSWORD"),
        ca_file: Some(PathBuf::from(var("PGXILA_TEST_SCRATCH_CA"))),
    }
}

fn server() -> Server {
    Server::connect(&params("PGXILA_TEST_USER")).expect("connection to the scratch server")
}

/// A session taken from the desktop to the address book, the way a user
/// gets there.
fn session_on_addressbook() -> Session<Server> {
    let mut session = Session::start(server()).expect("session");
    assert_eq!(session.view().stack(), "desktop");
    session.activate("stacks");
    assert_eq!(session.view().stack(), "stacks");
    session.activate("list");
    assert_eq!(session.view().stack(), "addressbook", "{:?}", session.message());
    session
}

/// Runs one SQL statement as the test user, outside the pgXila API.
fn execute(statement: &str, parameter: &str) {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let (client, connection) =
            xila_core::connection::connect(&params("PGXILA_TEST_USER")).await.unwrap();
        tokio::spawn(connection);
        client.execute(statement, &[&parameter]).await.unwrap();
    });
}

/// Waits for a notification, which arrives asynchronously.
fn wait_for(server: &mut Server, wanted: impl Fn(&Notification) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if server.notifications().iter().any(&wanted) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
#[ignore = "needs running pgXila servers: use scripts/test-integration.sh"]
fn the_server_introduces_itself() {
    let mut server = server();
    let hello = server.hello().unwrap();
    assert_eq!(hello.api, 1);
    assert_eq!(hello.role, var("PGXILA_TEST_USER"));
    assert_eq!(hello.home, "desktop");
    assert!(!hello.author);

    let palette = server.palette().unwrap();
    assert_eq!(palette.len(), 32);
    assert_eq!(palette[24].name, "orange");
    assert_eq!(palette[24].components(), Some((0xe8, 0x70, 0x3a)));
}

#[test]
#[ignore = "needs running pgXila servers: use scripts/test-integration.sh"]
fn a_role_outside_xila_user_cannot_use_the_api() {
    let mut outsider = Server::connect(&params("PGXILA_TEST_OUTSIDER")).expect("connection");
    let error = outsider.hello().unwrap_err();
    assert!(
        matches!(&error, ApiError::Other(text) if text.contains("permission denied")),
        "{error:?}"
    );
}

#[test]
#[ignore = "needs running pgXila servers: use scripts/test-integration.sh"]
fn a_stack_is_described_by_the_server() {
    let mut server = server();
    let screen = server.open_stack("addressbook").unwrap();
    assert_eq!(screen.stack.title, "Address book");
    assert!(screen.source.has_cards);

    let fields: Vec<&str> = screen
        .widgets
        .iter()
        .filter(|w| w.kind == WidgetKind::Field)
        .map(|w| w.name.as_str())
        .collect();
    assert_eq!(
        fields,
        ["last_name", "first_name", "company", "phone", "email", "city", "birthday"]
    );

    // Keys of the stack and keys available everywhere, together.
    let keys: Vec<&str> = screen.keys.iter().map(|k| k.key.as_str()).collect();
    assert_eq!(keys, ["F3", "F6", "F7", "F8", "F12", "Enter"]);

    let error = server.open_stack("nowhere").unwrap_err();
    assert_eq!(
        error,
        ApiError::User {
            code: "XL002".into(),
            message: "The stack “nowhere” does not exist.".into()
        }
    );
}

#[test]
#[ignore = "needs running pgXila servers: use scripts/test-integration.sh"]
fn a_user_browses_from_the_desktop_to_a_card() {
    let mut session = session_on_addressbook();
    let view = session.view();
    assert_eq!(view.card.position, 1);
    assert_eq!(view.value("last_name"), "Babbage");
    assert_eq!(view.card.texts["title"], "Babbage, Charles");

    session.press("F8");
    assert_eq!(session.view().card.position, 2);
    session.press("F7");
    assert_eq!(session.view().value("last_name"), "Babbage");

    session.press("F12");
    assert_eq!(session.view().stack(), "stacks");
}

#[test]
#[ignore = "needs running pgXila servers: use scripts/test-integration.sh"]
fn an_edited_field_is_written_back_when_the_user_moves_on() {
    let mut session = session_on_addressbook();
    let key = session.view().card.key.clone().unwrap();

    session.edit("city", "Cambridge".into());
    session.press("F8");
    assert_eq!(session.message(), None);

    // Another connection reads what was saved.
    let mut other = server();
    let saved = other.card("addressbook", Some(&key), Move::At).unwrap();
    assert_eq!(saved.values["city"].as_deref(), Some("Cambridge"));

    // Put the card back, through the API as well.
    let context = Context {
        card: Some(key),
        dirty: [("city".to_string(), "London".to_string())].into(),
        row: None,
    };
    let effects = other.act("addressbook", "save_card", &context).unwrap();
    assert!(effects.contains(&Effect::Message { text: "Card saved.".into() }));
}

#[test]
#[ignore = "needs running pgXila servers: use scripts/test-integration.sh"]
fn a_rejected_value_is_explained_and_nothing_is_lost() {
    let mut session = session_on_addressbook();
    session.edit("birthday", "someday".into());
    session.press("Enter");

    assert_eq!(
        session.message(),
        Some("The value “someday” is not valid for the field “birthday”.")
    );
    assert_eq!(session.view().value("birthday"), "someday", "what was typed stays on screen");
}

#[test]
#[ignore = "needs running pgXila servers: use scripts/test-integration.sh"]
fn a_new_card_is_created_then_found_in_its_place() {
    let mut session = session_on_addressbook();
    let before = session.view().card.count;

    session.press("F6");
    assert!(session.view().is_new);
    session.edit("last_name", "Aaron".into());
    session.edit("first_name", "Test".into());
    session.press("Enter");

    let view = session.view();
    assert_eq!(session.message(), Some("Card saved."));
    assert!(!view.is_new);
    assert_eq!(view.card.count, before + 1);
    assert_eq!(view.card.position, 1, "cards are ordered by name");
    assert_eq!(view.value("last_name"), "Aaron");

    // Cards are ordinary rows: remove the new one with plain SQL.
    let key = view.card.key.clone().unwrap();
    execute("DELETE FROM addressbook.contacts WHERE id::text = $1", &key);
    assert_eq!(server().card("addressbook", None, Move::First).unwrap().count, before);
}

#[test]
#[ignore = "needs running pgXila servers: use scripts/test-integration.sh"]
fn a_change_made_by_someone_else_is_pushed_to_the_client() {
    let mut watcher = server();
    let mut writer = server();

    let card = writer.card("addressbook", None, Move::Last).unwrap();
    let key = card.key.clone().unwrap();
    let original = card.values["company"].clone().unwrap_or_default();

    let save = |writer: &mut Server, value: &str| {
        let context = Context {
            card: Some(key.clone()),
            dirty: [("company".to_string(), value.to_string())].into(),
            row: None,
        };
        writer.act("addressbook", "save_card", &context).unwrap();
    };

    save(&mut writer, "Notifications SA");
    let notified = wait_for(&mut watcher, |n| {
        *n == Notification::Card { stack: Some("addressbook".into()), key: Some(key.clone()) }
    });
    save(&mut writer, &original);
    assert!(notified, "the watcher was not told about the change");
}
