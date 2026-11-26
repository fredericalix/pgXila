# pgXila

pgXila is a fantasy OS that lives inside PostgreSQL, in the spirit of PICO-8
for consoles and HyperCard for applications.

Everything is stored in the database: stacks, cards, screens, widgets,
documents, the colour palette, configuration and history. A separate client
connects to the database and renders what it describes. The client holds no
business logic.

pgXila is made of two parts:

- a PostgreSQL extension, written in Rust with [pgrx](https://github.com/pgcentralfoundation/pgrx),
  that provides the system schema, SQL functions and background workers;
- clients, written in Rust, that draw the screens: a text client for the
  terminal and a pixel art graphical client for Windows, Linux and macOS.

## Status

Early development. Phase 1 is done: one vertical slice, from the database to
two clients. Phase 2, author mode, is done too: a stack is built from the
client, without SQL, laid out with a visual editor, illustrated with a drawing
editor, and given scripts. Phase 3, the agent, has its first step: one talks
with the agent from a screen, and it reads the stacks to answer.

- a Docker image with PostgreSQL 19, the `pgxila` extension and
  [pgvector](https://github.com/pgvector/pgvector) built from source;
- mandatory encryption: the server only accepts TLS 1.3 connections, and the
  extension refuses unencrypted connections by itself;
- the system schema `xila`: stacks, backgrounds, widgets, layouts, actions,
  keys, images and the 32-colour palette, with a history of every change;
- the SQL API the clients call, described in [docs/api.md](docs/api.md);
- the desktop and the stack list, which are themselves stacks stored in the
  database;
- **the text client**, in the manner of an AS/400 display, and **the
  graphical client**, in pixel art: connect, browse the cards of a stack, edit
  a field and write it back. Both draw the same screens from the same rows;
- **author mode**, in both clients: create a stack, add, rename, order and
  remove its fields, and delete it. A stack belongs to its author;
- **the visual editor**, in the graphical client: move and resize the widgets
  of a card with the pointer or the keyboard, add texts, buttons and pictures,
  on every card or on one only, and undo;
- **the drawing editor**, in the graphical client: draw the icon of a stack,
  the picture of a card or a picture of the background, pixel by pixel, with
  the 32 colours of pgXila and those a stack adds;
- **scripts**, in Lua: a button or a screen does what its author wrote, with
  the rights of whoever uses it and within limits it cannot escape. They are
  written and tested in a script editor, in the graphical client, and run in
  both clients;
- **rewinding**: a card, the screen of a stack or a whole stack goes back to
  before any recorded change, a deleted stack or a removed field included;
- **the agent**, in both clients: `F9` on any screen opens a conversation with
  a language model, which reads the stacks to answer. It runs in a background
  worker of PostgreSQL, under a role of its own, and never reads more than the
  person it talks with may read.

Not there yet: an agent that changes things, its memory, and multiple
instances. A stack can also be
built with SQL, as [examples/addressbook.sql](examples/addressbook.sql) shows.
The interface is in English.

## Requirements

- Docker with Docker Compose, for everything on the server side;
- a stable Rust toolchain, for the clients and the integration tests.

PostgreSQL, pgrx and the Rust toolchain needed by the extension live inside the
Docker image. Nothing has to be installed on the host for the server.

## Quick start

```sh
# 1. Build and start the server (the first build takes several minutes)
docker compose up --build --detach --wait

# 2. Create a user who may build stacks; the password is asked for
scripts/create-user.sh ada --author

# 3. Load the example stack, an address book, and hand it to that user
scripts/load-example.sh addressbook --owner ada

# 4. Run a client: in a window...
cargo run -p xila-gui -- --user ada --ca docker/certs/ca.crt

#    ...or in the terminal
cargo run -p xila-term -- --user ada --ca docker/certs/ca.crt
```

The agent needs a model to call: see [The agent](#the-agent).

In either client, type the password and press Enter. Open the list of stacks
(Enter in the window, `1` then Enter in the terminal), and press Enter again to
open the address book. `F7` and `F8` browse the cards, `Tab` moves between
fields, Enter saves, `F6` starts a new card, `F4` deletes the card, `F12` goes
back and `F3` quits. Moving to another card also saves what was typed. A change
made by someone else appears on screen by itself.

To build a stack of your own, press `F6` in the list of stacks: see
[Author mode](#author-mode).

## The server

```sh
docker compose up --build
```

The server listens on `localhost:5419`. Set `PGXILA_PORT` to use another port.

On first start the container generates a development certificate authority and
a server certificate. Private keys stay inside a Docker volume. The public
certificate of the authority is written to `docker/certs/ca.crt`, which clients
need to verify the server. That directory is ignored by git.

To open an administration session, use the Unix socket inside the container:

```sh
docker compose exec -u postgres db psql -d pgxila
```

Users are PostgreSQL roles, members of `xila_user`. `scripts/create-user.sh`
creates one; add `--author` for a member of `xila_author`, who may also build
stacks, or `--admin` for a member of `xila_admin`, an author who may also set
up the agent. Any PostgreSQL client that verifies the certificate can connect with
such a role:

```sh
psql "host=localhost port=5419 dbname=pgxila user=ada sslmode=verify-full sslrootcert=docker/certs/ca.crt"
```

To start again from an empty database, remove the volumes:

```sh
docker compose down --volumes
```

Backups are those of PostgreSQL: `pg_dump` saves the stacks, their cards, the
history, and the conversations with the agent, and `pg_restore` brings them
back. The system stacks are not part
of a dump; the extension creates them.

The extension has no upgrade script yet: after pulling a new version, rebuild
the image and start again from an empty database.

## The clients

Both clients take the same options:

```sh
cargo run -p xila-gui -- --help
cargo run -p xila-term -- --help
```

| Option | Environment | Default |
|--------|-------------|---------|
| `--host` | `PGXILA_HOST` | `localhost` |
| `--port` | `PGXILA_PORT` | `5419` |
| `--database` | `PGXILA_DATABASE` | `pgxila` |
| `--user` | `PGXILA_USER` | asked on the connection screen |
| `--ca` | `PGXILA_CA` | the certificate authorities of the operating system |

The password is typed on the connection screen. For scripts, it can come from
the `PGXILA_PASSWORD` environment variable; there is deliberately no command
line option for it.

### The graphical client

`xila-gui` draws its own pixels: a screen of 480 by 300 pixels, one byte per
pixel, with the colours of `xila.palette` and its own bitmap fonts. The window
enlarges that screen by a whole factor, so pixels stay square and sharp. It
opens at 3 times the size when the display is large enough, or else less;
`--scale` (or `PGXILA_SCALE`) chooses the factor, and the window can be resized
or made full screen.

Everything works with the keyboard, as in the text client. The pointer works
too: click a button, a field, a row of a list (a second click opens it), a key
of the function key bar, or the close box of the window to go back.

On a Mac, the function keys of a laptop keyboard need the `fn` key, unless
"Use F1, F2, etc. keys as standard function keys" is turned on in the keyboard
settings. Clicking the keys of the bar at the bottom of the screen does the
same.

Rendering uses [winit](https://github.com/rust-windowing/winit) for the window
and the keyboard, and [softbuffer](https://github.com/rust-windowing/softbuffer)
to show the pixels, without a graphics card.

### The text client

`xila-term` needs a terminal of at least 80 columns by 24 rows. It draws with
the colours of `xila.palette`, as 24-bit colours: use a terminal that supports
them.

## Author mode

An author builds a stack from either client. In the terminal, author mode is a
set of forms. In the window it is a visual editor, from which the same forms
are one key away (`F4`).

### The forms

| Where | Key | What it does |
|-------|-----|--------------|
| My stacks | `F6` | New stack: a title is enough. The stack gets a table of its own |
| My stacks | `F11` | Author mode on the selected stack |
| A stack | `F11` | Author mode on that stack, for those who may change it |
| The visual editor | `F4` | The forms of the stack being edited |
| Author mode | `F6` | New field: a label and a type (text, number, date, yes or no) |
| Author mode | Enter | Saves the title, or opens the selected field |
| Author mode | `F7`, `F8` | Moves the selected field up or down |
| Author mode | `F10` | History of the stack, to rewind its screen or all of it |
| Author mode | `F11` | Shows the stack as its users see it |
| A stack | `F10` | History of its cards, to bring one back to before a change |
| My stacks | `F10` | The stacks you deleted, to bring one back |

Removing a field drops its column, and deleting a stack removes its screens;
both ask first, and both can be undone from the history. Nothing is ever lost: a
rewind is itself recorded and can be rewound.

### The visual editor

In the graphical client, `F11` on a stack shows its background with its widgets
as things to arrange. Fields show the name of their column instead of a value.

- **Select** a widget with a click, or with `Tab`. **Move** it by dragging it,
  or with the arrows; **resize** it by a handle, or with `Shift` and the arrows.
  It snaps to its neighbours, with a red guide line, and to a grid.
- **The tool palette**, on the left: the pointer, then a field, a text and a
  button to add; then whether new widgets go on every card or on the card on
  screen only; then the grid and the guides, on or off. `F6` adds a widget
  without the pointer.
- **Properties**, in a small window that can be moved and hidden (`F7`): the
  label of a field or a button, the words of a text, what a button does, its
  script, the position and the size as numbers.
- `F2`, Control-Z or Command-Z **undoes** the last change, then the one before.
  `Delete` removes the selected widget, after a confirmation.
- `F11` goes back to browsing, `F10` opens the history of the stack.

The layout of a stack is automatic until a widget is moved. From then on every
widget stays where it was put; Properties, with nothing selected, offers to go
back to the automatic layout.

### The drawing editor

Pictures are drawn in the graphical client, pixel by pixel:

| What | How to get there | Who may |
|------|------------------|---------|
| The picture of a card | Click it on the card, or press Enter when it has the focus. A stack gets such pictures with a field of type Picture | Whoever may write the card |
| The icon of a stack, shown in "My stacks" | `F5` in the visual editor | Whoever may change the stack |
| A picture of the background | The picture tool of the visual editor, then Draw in Properties | Whoever may change the stack |

The tools are on the left: pencil, eraser, fill, line, rectangle, pipette and
selection; each also has a letter (`P`, `E`, `F`, `L`, `R`, `I`, `S`). The
arrows move a cursor over the pixels and the space bar draws there. `+` and `-`
change the zoom; `H`, `V` and `T` flip and turn the picture; `F6` gives it
another size. The Colours window holds the 32 colours of pgXila and, for the
pictures of a stack, the colours that stack adds (`+`). `F2` undoes, `F4` goes
back to the picture as it was.

Leaving the editor (`F12`) saves the drawing. An image is never changed: each
drawing is kept as a new image, so the history of a card can bring back the
picture it had before, and nobody can overwrite the picture of someone else.

### Scripts

A button does one of the things pgXila knows: next card, save, delete. For
anything else its author writes a script, in [Lua 5.4](https://www.lua.org/).
The screen of a stack has a script too, for its keys and for what happens when
a card comes on screen.

```lua
-- script of the button "New card"
keys = { F2 = "New in London" }

function on_click()
  local new = stack.new_card()
  new.city = "London"
  go_to(new)
  focus("last_name")
end

function on_key(key)
  on_click()
end
```

In the visual editor, select a button and press `F8`, or use the Script button
of Properties; with nothing selected, it is the script of the screen. The
script editor colours the source and indents it, selects with `Shift` and the
arrows or with the pointer, copies and pastes through the clipboard of the
system, and undoes with Control-Z or Command-Z.

- **Test** (`F5`) tries the event chosen in the list on the right, on the card
  that was on screen. Everything the script writes is undone: the editor only
  says what it would have done, or which line is wrong.
- **Leaving** (`F12`) saves the script. A script with a mistake is kept too,
  and does nothing until it is fixed.

A script reads and writes the card on screen (`card.city`), finds, saves and
deletes cards (`stack.cards{city = "London"}`, `save`, `delete`), moves
(`go_to`, `open`, `focus`), talks to the user (`message`, `confirm`) and
reaches the database with `query` and `execute`. The whole list is in
[docs/api.md](docs/api.md), section 9.

What a script cannot do matters as much:

- it runs **with the rights of whoever presses the button**, never more:
  privileges and row-level security apply to everything it does, and it only
  reads and writes rows. So open the stacks of authors you trust; and no script
  ever runs for a superuser;
- it has **no file, no network, no operating system**, and keeps nothing in
  memory between two events;
- it **cannot block the server**: it is stopped after five seconds or too much
  work, cannot hold more than 16 MB, and cannot catch these limits;
- it is **all or nothing**: a script that fails leaves no trace in the
  database, and the user is told which line failed;
- `F3`, `F11` and `F12` can never be taken by a script: one can always leave.

### Who may change a stack

A stack belongs to the role that created it. Only that role and its members may
change it, and PostgreSQL enforces this on the tables themselves. A new stack is
visible to its author alone; to let others use its cards, grant them privileges
on its table, as for any table:

```sql
GRANT USAGE ON SCHEMA books TO xila_user;
GRANT SELECT, INSERT, UPDATE, DELETE ON books.cards TO xila_user;
```

A stack can also be made from a table that already exists and that you own:
choose it in the New stack dialog. pgXila then shows the table and never alters
it; its columns are added and removed in SQL.

## The agent

`F9`, on any screen, opens a conversation with the agent. Write a question and
press Enter: the agent answers in the conversation, and says on the way what
it looked at.

```text
 You: Who lives in London?
   - Looked for “London” in Address book: 2 cards.
 Agent: Two people live in London: Charles Babbage and Ada Lovelace.
```

`F5` gives up a question, `F6` starts a new conversation and `F10` lists the
conversations; from that list, `F7` shows the journal of what the agent did.
Called from a card, the agent knows which stack and which card "this" is.

In this first step the agent reads and answers. It changes nothing: it has no
tool to. Its three tools list the stacks, find cards and read a card.

### Setting it up

The agent calls a language model through an API that speaks the OpenAI chat
API, such as [OpenRouter](https://openrouter.ai) or a local
[LM Studio](https://lmstudio.ai). It needs an address, the name of a model
and, for most endpoints, a token.

```sh
# 1. The token goes in .env.local, under a name of your choice
cp .env.example .env.local        # then fill in OPENROUTER_API_KEY
docker compose up --detach --wait

# 2. The address, the model, and the NAME of the variable that holds the token
scripts/set-agent.sh https://openrouter.ai/api/v1 some/model OPENROUTER_API_KEY

# A model on this machine needs no token
scripts/set-agent.sh http://host.docker.internal:1234/v1 some-local-model
```

`.env.local` is ignored by git. The token never enters the database: the
settings hold the name of its variable, and the worker reads the value from
its environment when it calls the model. An administrator
(`scripts/create-user.sh ada --admin`) can change the settings from the
client: `F9`, `F10`, then `F4`.

The address must be `https://`. Plain `http://` is accepted for the machine
itself only (`localhost`, `host.docker.internal`): nothing a person asks, and
no token, crosses a network in clear.

### What it may read

What the agent reads is sent to the model, so it matters what it may read:

- **A stack is open to the agent until someone closes it.** In author mode,
  the check box "The agent may read the cards" does it. This is a real
  PostgreSQL privilege on the table of the stack, granted to the role of the
  agent, not a flag. Of a closed stack the agent knows the name and the
  title, so that it can say that it is closed.
- **Never more than the person.** The agent reads for the person who asks,
  and refuses what this person could not open.
- **Everything is written down.** The journal has a line for every call to
  the model and every tool the agent ran, for whom. It does not copy the
  cards.

## Security model

- **TLS 1.3 only.** `ssl_min_protocol_version` is `TLSv1.3` and `pg_hba.conf`
  only contains `hostssl` lines.
- **Enforced by the extension.** `pgxila` must be loaded through
  `shared_preload_libraries`. It installs a guard in PostgreSQL's client
  authentication hook that ends any TCP connection that is not encrypted with
  TLS 1.3, even if `pg_hba.conf` is edited to allow it. There is no setting to
  turn the guard off. Connections through the Unix socket, which never leave
  the machine, are the only exception.
- **SCRAM-SHA-256** authentication with PostgreSQL roles.
- **Clients verify the server.** The client code always requires TLS 1.3,
  verifies the certificate chain and the host name (the equivalent of
  `sslmode=verify-full`), and requires SCRAM channel binding.
- **Ownership is enforced by PostgreSQL.** Row-level security on the tables of
  the `xila` schema lets only the owner of a stack change its definition, and
  only a superuser change the system stacks. The functions of pgXila run with
  the privileges of the caller, so `GRANT`, `REVOKE` and row-level security on
  the tables of the stacks apply unchanged. History follows the same rule: it
  shows the old versions of a card only to those who may read the card.
- **Scripts are confined.** A script written by the author of a stack runs
  inside the extension, in a Lua state that has no file, network or system
  access, only loads source text, and is stopped when it exceeds its time or
  its memory. It acts with the privileges of the role that triggers it, through
  statements that read and write rows only, and never for a superuser.
- **The agent is confined.** It runs in a background worker connected as
  `xila_agent`, a role that cannot log in, owns nothing, and may only write
  its own conversations and journal. It reads the cards of a stack through a
  privilege that the owner of the stack can take away, and never for someone
  who could not read them. The model does not write SQL: it can only ask for
  one of the tools, which are fixed SQL functions. The token of the model
  stays in the environment of the server, is sent to the configured address
  only, over HTTPS, and is never stored or logged; only an administrator can
  change that address.

## Tests

```sh
# Clients: unit tests of the API types, the session, the layouts and the
# screens of both clients, drawn without a terminal or a window
cargo test --workspace

# Extension: pgrx tests of the schema and the SQL API, inside Docker
docker compose --profile test run --rm --build extension-test

# Integration tests against real servers in Docker
scripts/test-integration.sh
```

The integration tests prove, among other things, that an unencrypted connection
is refused, both by the configuration and by the extension alone, that TLS 1.2
is refused, and that the client rejects an untrusted certificate or a wrong
host name. They also drive both clients from the desktop to a card, with its
picture, and check what is written; an author builds a stack from the client,
rewinds it, deletes it and brings it back; an author writes a script in the
window, tests it, and a user runs it; and a user asks the agent, whose worker
calls a fake model run by the tests, which check what is sent to it. No test
calls a real model. They run against a throwaway server and do not touch the
data of the development server.

The same three suites run in GitHub Actions on every push, with the clients
built and tested on Linux, macOS and Windows.

## Repository layout

| Path | Content |
|------|---------|
| `extension/` | The `pgxila` PostgreSQL extension (Rust with pgrx, Lua for the scripts, and SQL: schema, client API, author functions, scripts, agent, actions, system stacks). `src/workers/` holds the background workers |
| `crates/xila-core/` | Code shared by the clients: connection, API, session, connection form |
| `crates/xila-gui/` | The graphical client, with its fonts |
| `crates/xila-term/` | The text client |
| `docker/` | Server image: Dockerfile, configuration, entrypoint |
| `docs/api.md` | SQL API between the extension and the clients |
| `examples/` | Example stacks. They are not part of pgXila |
| `scripts/` | Development scripts |

The extension is not part of the Cargo workspace: it only builds against the
PostgreSQL headers, inside the Docker image.

## Licence

pgXila is released under the [PostgreSQL License](LICENSE).

The two bitmap fonts of the graphical client are a conversion of Pixeloid Sans
by GGBotNet and stay under the SIL Open Font License 1.1; see
[crates/xila-gui/assets/fonts](crates/xila-gui/assets/fonts/README.md).
