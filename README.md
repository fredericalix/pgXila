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

Early development. What exists today:

- a Docker image with PostgreSQL 19, the `pgxila` extension and
  [pgvector](https://github.com/pgvector/pgvector) built from source;
- mandatory encryption: the server only accepts TLS 1.3 connections, and the
  extension refuses unencrypted connections by itself;
- the connection code shared by the future clients, with the same policy on
  the client side;
- a proposal for the SQL API between the extension and the clients, in
  [docs/api.md](docs/api.md). It is not implemented yet.

There is no client to run yet, and no stack to open.

## Requirements

- Docker with Docker Compose, for everything on the server side;
- a stable Rust toolchain, for the clients and the integration tests.

PostgreSQL, pgrx and the Rust toolchain needed by the extension live inside the
Docker image. Nothing has to be installed on the host for the server.

## Run the server

```sh
docker compose up --build
```

The first build compiles the extension and pgvector and takes several minutes.
The server then listens on `localhost:5419`. Set `PGXILA_PORT` to use another
port.

On first start the container generates a development certificate authority and
a server certificate. Private keys stay inside a Docker volume. The public
certificate of the authority is written to `docker/certs/ca.crt`, which clients
need to verify the server. That directory is ignored by git.

To open an administration session, use the Unix socket inside the container:

```sh
docker compose exec -u postgres db psql -d pgxila
```

To create a role that can connect from the host:

```sql
CREATE ROLE ada LOGIN PASSWORD 'choose a password';
```

and connect with any PostgreSQL client that verifies the certificate:

```sh
psql "host=localhost port=5419 dbname=pgxila user=ada sslmode=verify-full sslrootcert=docker/certs/ca.crt"
```

To start again from an empty database, remove the volumes:

```sh
docker compose down --volumes
```

### Agent settings

The agent (a later phase) calls a model through an OpenAI-compatible API. Its
endpoint and token are read from a `.env.local` file at the root of the
repository. Copy `.env.example` to `.env.local` and fill it in. The file is
ignored by git and is optional: the server runs without it.

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
# Client code: unit tests
cargo test --workspace

# Extension: pgrx tests, inside Docker
docker compose --profile test run --rm --build extension-test

# Integration tests against real servers in Docker
scripts/test-integration.sh
```

The integration tests prove, among other things, that an unencrypted connection
is refused, both by the configuration and by the extension alone, that TLS 1.2
is refused, and that the client rejects an untrusted certificate or a wrong
host name.

## Repository layout

| Path | Content |
|------|---------|
| `extension/` | The `pgxila` PostgreSQL extension (Rust, pgrx) |
| `crates/xila-core/` | Code shared by the clients |
| `docker/` | Server image: Dockerfile, configuration, entrypoint |
| `docs/api.md` | SQL API between the extension and the clients (proposal) |
| `scripts/` | Development scripts |

The extension is not part of the Cargo workspace: it only builds against the
PostgreSQL headers, inside the Docker image.

## Licence

pgXila is released under the [PostgreSQL License](LICENSE).
