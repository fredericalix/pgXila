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
two clients.

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
  a field and write it back. Both draw the same screens from the same rows.

Not there yet: author mode, the agent and multiple instances. Until author mode
exists, a stack is built with SQL, as
[examples/addressbook.sql](examples/addressbook.sql) shows. The interface is in
English.

## Requirements

- Docker with Docker Compose, for everything on the server side;
- a stable Rust toolchain, for the clients and the integration tests.

PostgreSQL, pgrx and the Rust toolchain needed by the extension live inside the
Docker image. Nothing has to be installed on the host for the server.

## Quick start

```sh
# 1. Build and start the server (the first build takes several minutes)
docker compose up --build --detach --wait

# 2. Create a user; the password is asked for
scripts/create-user.sh ada

# 3. Load the example stack, an address book
scripts/load-example.sh addressbook

# 4. Run a client: in a window...
cargo run -p xila-gui -- --user ada --ca docker/certs/ca.crt

#    ...or in the terminal
cargo run -p xila-term -- --user ada --ca docker/certs/ca.crt
```

In either client, type the password and press Enter. Open the list of stacks
(Enter in the window, `1` then Enter in the terminal), and press Enter again to
open the address book. `F7` and `F8` browse the cards, `Tab` moves between
fields, Enter saves, `F6` starts a new card, `F12` goes back and `F3` quits.
Moving to another card also saves what was typed. A change made by someone else
appears on screen by itself.

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
creates one; add `--author` for a member of `xila_author`, who may also write
stack definitions. Any PostgreSQL client that verifies the certificate can
connect with such a role:

```sh
psql "host=localhost port=5419 dbname=pgxila user=ada sslmode=verify-full sslrootcert=docker/certs/ca.crt"
```

To start again from an empty database, remove the volumes:

```sh
docker compose down --volumes
```

Backups are those of PostgreSQL: `pg_dump` saves the stacks, their cards and
the history, and `pg_restore` brings them back. The system stacks are not part
of a dump; the extension creates them.

The extension has no upgrade script yet: after pulling a new version, rebuild
the image and start again from an empty database.

### Agent settings

The agent (a later phase) calls a model through an OpenAI-compatible API. Its
endpoint and token are read from a `.env.local` file at the root of the
repository. Copy `.env.example` to `.env.local` and fill it in. The file is
ignored by git and is optional: the server runs without it.

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
picture, and check what is written. They run against a throwaway server and do
not touch the data of the development server.

The same three suites run in GitHub Actions on every push, with the clients
built and tested on Linux, macOS and Windows.

## Repository layout

| Path | Content |
|------|---------|
| `extension/` | The `pgxila` PostgreSQL extension (Rust with pgrx, and SQL) |
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
