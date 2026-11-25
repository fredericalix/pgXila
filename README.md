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

Early development. This branch does not contain anything to build or run yet.
The first working slice is being built on work branches and will be merged
here once it works end to end.

## Licence

pgXila is released under the [PostgreSQL License](LICENSE).
