# pgXila SQL API

> **Status: proposal, not approved, not implemented.**
> This document describes the contract between the pgXila extension and its
> clients. Nothing in it is final until the decisions listed below are approved.

A pgXila client is an ordinary PostgreSQL client. It connects over TLS 1.3 with
SCRAM-SHA-256, calls SQL functions of the `xila` schema, and listens on
notification channels. It never reads or writes a table directly and it holds no
business logic: it draws what the functions return and sends back what the user
does.

## Decisions to approve

| # | Decision | Proposal |
|---|----------|----------|
| 1 | Where a screen definition is stored | As rows (one per widget, action, key), not as one document |
| 2 | What the client receives | One JSON document per screen, assembled by a function |
| 3 | Where the cards live | In the stack's own table; nothing is copied into `xila` |
| 4 | Card identity | The single-column primary key of that table, carried as text |
| 5 | Navigation state | Kept by the client and sent with each call; the server is stateless |
| 6 | Action result | A list of display effects from a small fixed vocabulary |
| 7 | Concurrent edits | Optimistic: a save fails if the card changed since it was read |
| 8 | Identifiers | `uuid` (UUIDv7) everywhere, ready for several instances |
| 9 | Text grid | 80 columns by 24 rows |
| 10 | Scripting language, in the meantime | Lua 5.4, run by the extension. To decide in phase 2 |

## 1. Roles

| Role | Login | Purpose |
|------|-------|---------|
| `xila_user` | no | Group role. May call the browsing API. Every person is a member |
| `xila_author` | no | Group role, member of `xila_user`. May create and edit stacks (phase 2) |
| `xila_agent` | no | Restricted role the agent tools run under (phase 3) |
| one role per person | yes | Created by an administrator, member of `xila_user` |

Access to the cards of a stack is decided by ordinary PostgreSQL privileges on
the stack's table. The API functions that read and write cards run with the
privileges of the caller, so `GRANT`, `REVOKE` and row-level security apply
unchanged. A stack is visible to a user when that user may `SELECT` from its
table.

## 2. Data model

System objects live in the `xila` schema. Each stack gets a PostgreSQL schema of
its own for its data, for example `carnet.contacts`.

| Table | One row is | Main columns |
|-------|-----------|--------------|
| `xila.palette` | a system colour | `slot` (0 to 31), `name`, `rgb` (`#rrggbb`) |
| `xila.stack_colour` | an extra colour of a stack | `stack_id`, `slot` (32 to 255), `name`, `rgb` |
| `xila.stack` | a stack | `id`, `name` (identifier), `title`, `kind` (`system` or `user`), `icon`, `owner` |
| `xila.background` | the screen shared by the cards of a stack | `id`, `stack_id`, `source_schema`, `source_table`, `key_column`, `order_by` |
| `xila.widget` | a widget | `id`, `background_id`, `layer`, `card_key`, `name`, `kind`, `ord`, `grp`, `label`, `bound_column`, `action_id`, `props` |
| `xila.layout` | the placement of a widget for one renderer | `widget_id`, `renderer` (`text` or `graphic`), `x`, `y`, `w`, `h`, `style` |
| `xila.action` | something a button or a key triggers | `id`, `background_id`, `name`, `label`, `handler`, `args` |
| `xila.key_binding` | a function key | `background_id` (null means everywhere), `key`, `action_id` |
| `xila.image` | an indexed image | `id`, `width`, `height`, `pixels` (one byte per pixel) |
| `xila.history` | one recorded change | see section 7 |

Notes:

- **Cards.** A card is a row of the table named by its background. pgXila does
  not keep a copy. A background without a source table is a static screen with a
  single card (the desktop, for example). This differs from the wording of the
  brief, which lists cards in the system schema: the only card-specific data kept
  in `xila` are the widgets placed on one card (`layer = 'card'`, `card_key`).
- **One background per stack** for now. It is a separate table so that several
  backgrounds per stack can be allowed later without moving columns.
- **Widget kinds**: `field`, `text`, `button`, `image`, `grid`, `slider`,
  `canvas`. Phase 1 implements the first five.
- **Logical model first.** `xila.widget` says what the screen contains: name,
  kind, label, bound column, logical order (`ord`) and group (`grp`). It says
  nothing about where things are drawn. `xila.layout` adds one optional placement
  per renderer: character cells for `text`, pixels for `graphic`. When a layout
  row is missing, the client builds a default placement from `ord` and `grp`.
- **Field type.** The editing behaviour of a field comes from the PostgreSQL
  type of the bound column (text, number, date, boolean). `props` holds the rest:
  `required`, `read_only`, `max_length`.
- **Computed text.** A `text` widget may carry a template such as
  `{nom}, {prenom}` or `Fiche {position} sur {count}`. The server fills it in.

## 3. Screen definition

The storage format is the set of rows above. The client never joins them itself:
it calls `xila.open_stack` and receives one JSON document.

```json
{
  "stack": { "name": "carnet", "title": "Carnet d'adresses", "kind": "user" },
  "source": { "has_cards": true, "count": 42 },
  "widgets": [
    { "name": "titre", "kind": "text", "layer": "background", "ord": 10,
      "props": { "template": "{nom}, {prenom}", "style": "title" },
      "layout": { "graphic": { "x": 16, "y": 8, "w": 300, "h": 20 } } },
    { "name": "nom", "kind": "field", "layer": "background", "ord": 20,
      "label": "Nom", "column": "nom", "type": "text",
      "props": { "required": true },
      "layout": { "text":    { "x": 20, "y": 4, "w": 30, "h": 1 },
                  "graphic": { "x": 170, "y": 60, "w": 215, "h": 14 } } },
    { "name": "suivante", "kind": "button", "ord": 90,
      "label": "Suiv.", "action": "next_card", "layout": {} }
  ],
  "actions": [
    { "name": "next_card", "label": "Suiv." },
    { "name": "previous_card", "label": "Préc." }
  ],
  "keys": [
    { "key": "F3", "label": "Quitter", "action": "quit" },
    { "key": "F7", "label": "Préc.", "action": "previous_card" },
    { "key": "F8", "label": "Suiv.", "action": "next_card" },
    { "key": "F12", "label": "Retour", "action": "back" }
  ],
  "colours": [ { "slot": 32, "rgb": "#aabbcc" } ]
}
```

Coordinates: `text` layouts are in character cells on an 80 by 24 grid, the last
row being the function key bar. `graphic` layouts are in pixels, relative to the
content area of the stack window, on the 480 by 300 screen.

## 4. Functions

All functions live in the `xila` schema and return `jsonb`, except where noted.

| Function | Returns |
|----------|---------|
| `xila.version()` | `text`: version of the extension. The only function that exists today |
| `xila.hello()` | API version, instance name, current role, group memberships, name of the home stack |
| `xila.palette()` | set of `(slot, name, rgb)`: the 32 system colours |
| `xila.open_stack(stack text)` | the screen definition of section 3 |
| `xila.card(stack text, key text, move text)` | one card (see below) |
| `xila.save_card(stack text, key text, changes jsonb, version text)` | the saved card |
| `xila.grid_rows(stack text, widget text, first int, count int)` | a page of rows for a `grid` widget |
| `xila.image(id uuid)` | `(width, height, pixels bytea)` |
| `xila.act(stack text, action text, context jsonb)` | a list of effects (section 5) |

**`xila.card`.** `move` is one of `first`, `last`, `next`, `previous`, `at`. With
`next` and `previous`, `key` is the card the user is on. The result is:

```json
{
  "key": "0193a5e2-…",
  "position": 3,
  "count": 42,
  "version": "8c1f…",
  "values": { "nom": "Lovelace", "prenom": "Ada", "ville": "Londres" },
  "texts":  { "titre": "Lovelace, Ada", "compteur": "Fiche 3 sur 42" },
  "widgets": []
}
```

Values travel as text, in the PostgreSQL text form of each type. `widgets` lists
the widgets that belong to this card only.

**`xila.save_card`.** `changes` holds only the modified columns. `version` is the
value received with the card. If the row changed in the meantime the function
raises `XL001` and nothing is written. If a value is rejected (wrong type,
constraint, required field), it raises `XL003` with the column name in the error
detail. With `key` null, a new card is created.

**Errors.** pgXila errors use SQLSTATE class `XL`. Their message is written for
the user, in French, and the client shows it as is.

| SQLSTATE | Meaning |
|----------|---------|
| `XL001` | The card was changed by someone else |
| `XL002` | Unknown stack, card, widget or action |
| `XL003` | A value was rejected |
| `XL004` | Not allowed for this role |

## 5. Actions

Buttons and function keys do not carry code. They name an action, and the client
asks the server to run it:

```sql
SELECT xila.act('carnet', 'next_card', '{"card": "0193a5e2-…", "dirty": {}}');
```

`context` is the state of the client: the current card, the unsaved field values,
the selected grid row. The server looks up the action, runs its handler, and
answers with effects that the client applies in order:

```json
[ { "do": "show", "stack": "carnet", "card": "0193a5e3-…" },
  { "do": "focus", "widget": "nom" } ]
```

| Effect | Meaning |
|--------|---------|
| `show` | Display a stack, optionally at a given card |
| `back` | Return to the previous screen |
| `reload` | Read the current card again |
| `focus` | Put the cursor in a widget |
| `message` | Show a message to the user |
| `quit` | Leave the client |

A handler is a SQL function of the extension with the signature
`(context jsonb, args jsonb) returns jsonb`. Phase 1 ships these handlers:
`open_stack`, `next_card`, `previous_card`, `first_card`, `last_card`,
`save_card`, `new_card`, `back`, `quit`.

Handlers written by stack authors, in SQL or in a script, arrive with author mode
and scripting. They will be additional kinds of handler behind the same
`xila.act` call, so the client does not change.

## 6. Notifications

| Channel | Payload | Sent when |
|---------|---------|-----------|
| `xila_card` | `{"stack": "carnet", "key": "…", "op": "update"}` | a card is inserted, updated or deleted |
| `xila_screen` | `{"stack": "carnet"}` | a screen definition or the palette changes |

Payloads carry identifiers only, never card contents, because any connected role
may listen. A client that shows the affected card or stack reads it again.

The notifications are sent by triggers. Attaching a table to a stack installs
them, together with the history trigger of the next section.

## 7. History

Every change to a stack, a screen or a card is recorded in `xila.history`:

| Column | Content |
|--------|---------|
| `id`, `at`, `txid` | order, time and transaction of the change |
| `actor` | the role that made it |
| `origin` | `user`, `agent` or `system` |
| `stack_id`, `object_kind`, `object_key` | what changed: a card, a widget, a layout, an action, a stack |
| `op` | `insert`, `update` or `delete` |
| `before`, `after` | the row before and after, as `jsonb` |

Recording starts in phase 1. Rewinding (re-applying `before` images in reverse
order, for one card, one screen or a whole stack) comes with phase 2, together
with its API. Changes to the structure of a stack's table, such as adding or
dropping a column, are made through pgXila functions in phase 2 so that they can
be recorded and rewound as well. How a dropped column is restored is still open.

## 8. System stacks

The desktop and the stack list are stacks shipped by the extension, with
`kind = 'system'`:

- `desktop`: a static screen. Its widgets are buttons whose action is
  `open_stack`. In text mode the default layout of a screen made only of buttons
  is a numbered menu.
- `stacks`: a static screen with a `grid` widget fed by the stacks visible to the
  current user. Choosing a row runs `open_stack`.

The connection screen is the only screen built into the clients, because it is
shown before any connection exists.

## 9. Scripts (proposal for phase 2)

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
