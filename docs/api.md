# pgXila SQL API

This document describes the contract between the pgXila extension and its
clients, as implemented for phase 1. API version: 1.

A pgXila client is an ordinary PostgreSQL client. It connects over TLS 1.3 with
SCRAM-SHA-256, calls SQL functions of the `xila` schema, and listens on
notification channels. It holds no business logic: it draws what the functions
return and sends back what the user does.

## Design decisions

| Decision | Choice |
|----------|--------|
| Where a screen definition is stored | As rows (one per widget, action, key), not as one document |
| What the client receives | One JSON document per screen, assembled by a function |
| Where the cards live | In the stack's own table; nothing is copied into `xila` |
| Card identity | The single-column primary key of that table, carried as text |
| Navigation state | Kept by the client and sent with each call; the server is stateless |
| What a button or a key does | It names an action; the server runs it and answers with effects |
| Concurrent edits | The last save wins; history keeps every version |
| Identifiers | `uuid` (UUIDv7), ready for several instances |
| Text grid | 80 columns by 24 rows |
| Graphic screen | 480 by 300 pixels, one byte per pixel, each a palette slot |
| Language | English, for identifiers and for everything the user reads |

## 1. Roles

| Role | Login | Purpose |
|------|-------|---------|
| `xila_user` | no | Group role. May call the API. Every person is a member |
| `xila_author` | no | Group role, member of `xila_user`. May write stack definitions |
| one role per person | yes | Created by an administrator, member of `xila_user` |

A role that is not a member of `xila_user` cannot use the `xila` schema at all.

Access to the cards of a stack is decided by ordinary PostgreSQL privileges on
the stack's table. The API functions run with the privileges of the caller, so
`GRANT`, `REVOKE` and row-level security apply unchanged. A stack is listed for
a user when that user may `SELECT` from its table.

Known limit: a member of `xila_author` may currently write the definition of any
stack, including the system stacks. Ownership rules come with author mode.

The role the agent tools run under (`xila_agent`) comes with phase 3.

## 2. Data model

System objects live in the `xila` schema. A stack keeps its data in a table of
its own, usually in a schema of its own, for example `addressbook.contacts`.

| Table | One row is | Main columns |
|-------|-----------|--------------|
| `xila.palette` | a system colour | `slot` (0 to 31), `name`, `rgb` (`#rrggbb`) |
| `xila.stack_colour` | an extra colour of a stack | `stack_id`, `slot` (32 to 255), `name`, `rgb` |
| `xila.image` | an indexed image | `id`, `width`, `height`, `pixels` (one byte per pixel) |
| `xila.stack` | a stack | `id`, `name` (identifier), `title`, `kind` (`system` or `user`), `icon_id`, `owner` |
| `xila.background` | the screen shared by the cards of a stack | `id`, `stack_id`, `source_schema`, `source_table`, `key_column`, `order_columns` |
| `xila.action` | something a button or a key triggers | `id`, `background_id`, `name`, `label`, `handler`, `args` |
| `xila.widget` | a widget | `id`, `background_id`, `layer`, `card_key`, `name`, `kind`, `ord`, `grp`, `label`, `bound_column`, `action_id`, `props` |
| `xila.layout` | the placement of a widget for one renderer | `widget_id`, `renderer` (`text` or `graphic`), `x`, `y`, `w`, `h`, `style` |
| `xila.key_binding` | a key bound to an action | `background_id`, `key`, `action_id` |
| `xila.history` | one recorded change | see section 7 |

Building a stack means inserting rows in these tables. `examples/addressbook.sql`
builds a complete one.

- **Cards.** A card is a row of the table named by its background. pgXila keeps
  no copy. A background without a source table is a static screen with a single
  card (the desktop, for example). The only card-specific data kept in `xila` are
  the widgets placed on one card (`layer = 'card'`, `card_key`).
- **Source table.** It must have a primary key made of a single column. The
  cards are ordered by `order_columns`, then by the key. Inserting or changing a
  background row checks the table and installs on it the trigger that records
  history and notifies clients; the role doing so must own the table.
- **One background per stack** for now. It is a separate table so that several
  backgrounds per stack can be allowed later without moving columns.
- **Widget kinds**: `field`, `text`, `button`, `image`, `grid`, `slider`,
  `canvas`. The clients of phase 1 draw `field`, `text`, `button` and `grid`;
  the graphical client also draws `image`.
- **Logical model first.** `xila.widget` says what the screen contains: name,
  kind, label, bound column, logical order (`ord`) and group (`grp`). It says
  nothing about where things are drawn. `xila.layout` adds one optional placement
  per renderer: character cells for `text`, pixels for `graphic`. When a layout
  row is missing, the client builds a default placement from the logical model.
  A layout whose `style` contains `"hidden": true` removes the widget from that
  renderer.
- **Field type.** The editing behaviour of a field comes from the PostgreSQL
  type of the bound column: `text`, `number`, `date` or `boolean`.
- **Computed text.** A `text` widget carries a template in `props`, such as
  `{"template": "{last_name}, {first_name}"}`. Besides the columns of the card,
  `{position}`, `{count}`, `{instance}` and `{user}` are available; a column
  with one of these names wins. The server fills the template in.
- **Kinds of text.** `"style": "title"` in the `props` of a `text` widget makes
  it the title of the screen, and `"style": "subtitle"` the line under that
  title. A `text` widget that has a `label` is a line of a summary: the label,
  then the text. These say what a text is, not how it is drawn.
- **Image.** An `image` widget shows a row of `xila.image`: the one whose
  identifier is in its bound column, of type `uuid`, or a fixed one named in
  `props` as `{"image": "0193…"}`. An image is `width * height` bytes, one per
  pixel, row by row; each byte is a palette slot.
- **Grid.** A `grid` widget lists the rows of a table or view named in `props`:
  `{"schema": "xila", "table": "visible_stack", "key": "name", "columns":
  [{"column": "title", "label": "Stack"}]}`. Choosing a row runs the widget's
  action with the key of that row.
- **Actions and keys.** An action or a key binding without a background is
  available on every screen. A key bound on a screen wins over the same key
  bound everywhere. Keys are `F1` to `F24` and `Enter`.

## 3. Screen definition

The storage format is the set of rows above. The client never joins them itself:
it calls `xila.open_stack` and receives one JSON document.

```json
{
  "stack": { "name": "addressbook", "title": "Address book", "kind": "user" },
  "source": { "has_cards": true, "count": 12 },
  "widgets": [
    { "name": "title", "kind": "text", "layer": "background", "ord": 10,
      "props": { "template": "{last_name}, {first_name}", "style": "title" },
      "layout": { "text": { "x": 0, "y": 0, "w": 0, "h": 0, "style": { "hidden": true } } } },
    { "name": "last_name", "kind": "field", "layer": "background", "ord": 20,
      "label": "Last name", "column": "last_name", "type": "text",
      "props": { "required": true }, "layout": {} },
    { "name": "next", "kind": "button", "layer": "background", "ord": 100,
      "label": "Next", "action": "next_card", "props": {}, "layout": {} }
  ],
  "actions": [
    { "name": "next_card", "label": "Next" },
    { "name": "quit", "label": "Quit" }
  ],
  "keys": [
    { "key": "F3", "label": "Quit", "action": "quit" },
    { "key": "F8", "label": "Next", "action": "next_card" },
    { "key": "F12", "label": "Back", "action": "back" },
    { "key": "Enter", "label": "Save", "action": "save_card" }
  ],
  "colours": []
}
```

Coordinates: `text` layouts are in character cells on an 80 by 24 grid. The
client keeps rows 0 to 2 for the header and rows 21 to 23 for messages and
function keys. `graphic` layouts are in pixels, relative to the content area of
the stack window, on the 480 by 300 screen.

### The graphic renderer

The graphical client shows one stack at a time, in a window whose size follows
its content. Without any `graphic` layout row, it places the widgets itself:

- **A stack with cards** is a sheet of paper. The `title` text is written large
  at the top, with the position in the stack on its right. Under it, the first
  `image` widget is on the left, and the fields form a column with their labels,
  in `ord` order. The buttons are in a row under the sheet.
- **A static screen** starts with a banner made of its `title` and `subtitle`
  texts. The other texts follow, one per line, then the fields, the grid and a
  row of buttons.

A `graphic` layout row changes that for one widget:

- with a size (`w` and `h` greater than zero), the widget is drawn exactly in
  that rectangle and takes no part in the automatic placement;
- without a size, the widget keeps its automatic place and only `style` is
  used.

| Style key | Applies to | Meaning |
|-----------|-----------|---------|
| `hidden` | any widget | `true` removes the widget from this renderer |
| `variant` | button | `"primary"` makes it the main button of the screen |
| `arrow` | button | `"left"` or `"right"` adds an arrow, for browsing buttons |

The function key bar lists the keys of the screen; a click on a key is the key.
The close box of the window is the `F12` key.

## 4. Functions

All functions live in the `xila` schema.

| Function | Returns |
|----------|---------|
| `xila.version()` | `text`: version of the extension |
| `xila.hello()` | `jsonb`: API version, extension version, instance name, current role, whether it is an author, name of the home stack |
| `xila.palette()` | set of `(slot, name, rgb)`: the 32 system colours |
| `xila.open_stack(stack text)` | `jsonb`: the screen definition of section 3 |
| `xila.card(stack text, key text, move text)` | `jsonb`: one card (see below) |
| `xila.save_card(stack text, key text, changes jsonb)` | `jsonb`: the saved card |
| `xila.grid_rows(stack text, widget text, first int, count int)` | `jsonb`: `{"total": n, "rows": [{"key": …, "cells": {column: text}}]}` |
| `xila.image(image_id uuid)` | `(width, height, pixels bytea)` |
| `xila.act(stack text, action text, context jsonb)` | `jsonb`: a list of effects (section 5) |

`xila.visible_stack` is a view of the user stacks the current role may open,
with their number of cards. The stack list is a grid over it.

**`xila.card`.** `move` is one of `first`, `last`, `at`, `next`, `previous`; the
last three are relative to `key`. `next` and `previous` stop at the ends. When
`key` no longer exists, the first card is returned. The result is:

```json
{
  "key": "0193a5e2-…",
  "position": 3,
  "count": 12,
  "values": { "last_name": "Lovelace", "first_name": "Ada", "city": "London" },
  "texts":  { "title": "Lovelace, Ada" },
  "widgets": []
}
```

Values travel as text, in the PostgreSQL text form of each type. `widgets` lists
the widgets that belong to this card only. A stack without any card answers with
a null `key` and a `count` of 0.

**`xila.save_card`.** `changes` holds only the modified columns, as text. With a
null `key`, a new card is created. An empty text clears a column that is not of
a text type. If two people edit the same card, the last save wins; both versions
are in the history. The clients do not call this function directly: they run the
`save_card` action, or simply move to another card.

**Errors.** pgXila errors use SQLSTATE class `XL`. Their message is written for
the user, in English, and the client shows it as it is.

| SQLSTATE | Meaning |
|----------|---------|
| `XL002` | Unknown stack, card, widget or action |
| `XL003` | A value was rejected. The message names the field |
| `XL004` | Not allowed for this role |

## 5. Actions

Buttons and function keys do not carry code. They name an action, and the client
asks the server to run it:

```sql
SELECT xila.act('addressbook', 'next_card',
                '{"card": "0193a5e2-…", "dirty": {"city": "Paris"}}');
```

`context` is the state of the client: `card` is the key of the card on screen
(null for a card not saved yet), `dirty` holds what the user typed and has not
saved, `row` is the key of the selected grid row. The server looks up the action,
runs its handler, and answers with effects that the client applies in order:

```json
[ { "do": "show", "stack": "addressbook", "card": "0193a5e3-…" } ]
```

| Effect | Meaning |
|--------|---------|
| `show` | Display a stack, at a given card if `card` is present |
| `back` | Return to the previous screen |
| `reload` | Read the current card again |
| `focus` | Put the cursor in a widget |
| `message` | Show a message to the user |
| `new` | Display an empty card, to be saved as a new one |
| `quit` | Leave the client |

The handlers of phase 1:

| Handler | What it does |
|---------|--------------|
| `open_stack` | Shows the stack named in `args`, or else the selected grid row |
| `next_card`, `previous_card`, `first_card`, `last_card` | Saves what was typed, then shows the other card |
| `save_card` | Saves what was typed; creates the card if it is new |
| `new_card` | Shows an empty card |
| `back`, `quit` | Ask the client to go back, or to leave |

Handlers written by stack authors, in SQL or in a script, arrive with author mode
and scripting. They will be additional kinds of handler behind the same
`xila.act` call, so the clients do not change.

## 6. Notifications

| Channel | Payload | Sent when |
|---------|---------|-----------|
| `xila_card` | `{"stack": "addressbook", "key": "…", "op": "update"}` | a card is inserted, updated or deleted |
| `xila_screen` | `{"stack": "addressbook"}` | a screen definition changes; `stack` is null when it is the palette |

Payloads carry identifiers only, never card contents, because any connected role
may listen. A client that shows the affected card or stack reads it again.

## 7. History

Every change to a stack, a screen or a card is recorded in `xila.history`:

| Column | Content |
|--------|---------|
| `id`, `at`, `txid` | order, time and transaction of the change |
| `actor` | the role that made it |
| `origin` | `user`, `agent` or `system` |
| `stack_id`, `object_kind`, `object_key` | what changed: a `card`, or a row of `stack`, `background`, `widget`, `layout`, `action`, `key_binding`, `stack_colour`, `palette` |
| `op` | `insert`, `update` or `delete` |
| `before`, `after` | the row before and after, as `jsonb` |

History starts empty in a new database: the rows the extension creates itself
(palette, system stacks) are the baseline. Restoring a backup reloads the
history it contains and does not record the reload as new changes; see
`xila.restoring()` in the extension for how a restore is recognised.

Only recording exists today. Rewinding (re-applying `before` images in reverse
order, for one card, one screen or a whole stack) comes with phase 2, together
with its API. Changes to the structure of a stack's table, such as adding or
dropping a column, will go through pgXila functions so that they can be recorded
and rewound as well. How a dropped column is restored is still open.

## 8. System stacks

The desktop and the stack list are stacks shipped by the extension, with
`kind = 'system'`, described with the same rows as any other stack:

- `desktop`: a static screen. Its buttons run `open_stack`; its texts say
  which instance this is and who is connected. In text mode the buttons are a
  numbered menu and the texts are hidden, because the header of the screen
  already says the same. In the window they form the welcome banner.
- `stacks`: a static screen with a `grid` widget over `xila.visible_stack`.
  Choosing a row runs `open_stack`.

The connection screen is the only screen built into the clients, because it is
shown before any connection exists.

## 9. Scripts (proposal for phase 2, not decided)

- A script is a row: `xila.script(id, owner_kind, owner_id, event, language, source)`.
  The owner is a stack, a background, a card or a widget.
- Events: `click`, `key`, `open_card`, `timer`, `row_changed`, `message`.
- A script is one more kind of action handler. It runs in the extension, receives
  the same context, reads and writes through SQL under the caller's role, and
  returns the same effects.
- Language in the meantime: **Lua 5.4**, embedded in the extension, with no file
  or network access and a hard limit on the number of instructions so that a
  script can never block a backend. Rhai (pure Rust, easier to sandbox, less
  known) is the alternative. The language of pgXila itself comes later and would
  replace Lua behind the same `xila.script` rows.

## 10. Not covered yet

- Author mode API (creating stacks, editing widgets, drawing images): phase 2.
- Agent API, audit log, validation of proposed actions: phase 3.
- Publishing stacks between instances, messaging: phase 4.
- Documents, global search.
