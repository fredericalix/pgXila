# pgXila SQL API

This document describes the contract between the pgXila extension and its
clients, as implemented for phase 1, for phase 2 (author mode) and for steps
3a and 3b of the agent. API version: 5.

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
| Who may change a stack | Its owner role and the members of that role, enforced by row-level security |
| Where the author screens live | In the database, as system stacks; the clients have no author code |
| Tables pgXila alters | Only those it created (`managed`); never a table that existed before |
| Pictures | An image never changes: a drawing is saved as a new image, and what showed the old one points to the new one |
| Scripts | Lua 5.4 inside the extension, one script per button or per screen, run with the rights of whoever triggers it |
| The agent | In a background worker, under a role of its own. It reads for the person it talks with, and never more than that person may read. It writes nothing: it proposes, and the person applies |
| Identifiers | `uuid` (UUIDv7), ready for several instances |
| Text grid | 80 columns by 24 rows |
| Graphic screen | 480 by 300 pixels, one byte per pixel, each a palette slot |
| Language | English, for identifiers and for everything the user reads |

## 1. Roles

| Role | Login | Purpose |
|------|-------|---------|
| `xila_user` | no | Group role. May call the API. Every person is a member |
| `xila_author` | no | Group role, member of `xila_user`. May write stack definitions |
| `xila_admin` | no | Group role, member of `xila_author`. May set up the agent and read its whole journal |
| `xila_agent` | no | The role of the agent worker; see section 11.2. Nobody is a member of it |
| one role per person | yes | Created by an administrator, member of `xila_user` |

A role that is not a member of `xila_user` cannot use the `xila` schema at all.

Access to the cards of a stack is decided by ordinary PostgreSQL privileges on
the stack's table. The API functions run with the privileges of the caller, so
`GRANT`, `REVOKE` and row-level security apply unchanged. A stack is listed for
a user when that user may `SELECT` from its table.

The definition of a stack may only be changed by the role that owns the stack
and by the members of that role; see section 10.1.

## 2. Data model

System objects live in the `xila` schema. A stack keeps its data in a table of
its own, usually in a schema of its own, for example `addressbook.contacts`.

| Table | One row is | Main columns |
|-------|-----------|--------------|
| `xila.palette` | a system colour | `slot` (0 to 31), `name`, `rgb` (`#rrggbb`) |
| `xila.stack_colour` | an extra colour of a stack | `stack_id`, `slot` (32 to 254), `name`, `rgb` |
| `xila.image` | an indexed image, written once | `id`, `width`, `height`, `pixels` (one byte per pixel; 255 is transparent) |
| `xila.stack` | a stack | `id`, `name` (identifier), `title`, `kind` (`system` or `user`), `icon_id`, `owner`, `agent_reads` (section 11.2) |
| `xila.background` | the screen shared by the cards of a stack | `id`, `stack_id`, `source_schema`, `source_table`, `key_column`, `order_columns`, `managed` |
| `xila.action` | something a button or a key triggers | `id`, `background_id`, `name`, `label`, `handler`, `args` |
| `xila.widget` | a widget | `id`, `background_id`, `layer`, `card_key`, `name`, `kind`, `ord`, `grp`, `label`, `bound_column`, `action_id`, `props` |
| `xila.layout` | the placement of a widget for one renderer | `widget_id`, `renderer` (`text` or `graphic`), `x`, `y`, `w`, `h`, `style` |
| `xila.key_binding` | a key bound to an action | `background_id`, `key`, `action_id` |
| `xila.script` | the script of a button, or of a screen | `background_id`, `widget_id`, `language`, `source`, `events`, `keys`; see section 9 |
| `xila.history` | one recorded change | see section 7 |
| `xila.agent_setting`, `xila.agent_tool`, `xila.conversation`, `xila.agent_message`, `xila.agent_proposal`, `xila.agent_log` | the agent | see sections 11.4 and 11.10 |

Building a stack means inserting rows in these tables. `examples/addressbook.sql`
builds a complete one in SQL; author mode (section 10) does the same from the
client.

- **Cards.** A card is a row of the table named by its background. pgXila keeps
  no copy. A background without a source table is a static screen with a single
  card (the desktop, for example). The only card-specific data kept in `xila` are
  the widgets placed on one card (`layer = 'card'`, `card_key`).
- **Source table.** It must have a primary key made of a single column. The
  cards are ordered by `order_columns`, then by the key. Inserting or changing a
  background row checks the table and installs on it the trigger that records
  history and notifies clients; the role doing so must own the table.
- **Read-only source.** The source may be a view instead of a table. Its cards
  are shown and browsed, never saved, and no history is kept for them. Asking a
  view for a card that no longer exists is an error (`XL002`), not the first
  card: the author dialogs are cards of views, and must not show another subject
  than the one asked for.
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
  type of the bound column: `text`, `number`, `date` or `boolean`. A `boolean`
  field is a check box in the window, and a choice between Yes and No in the
  terminal.
- **Choices.** `{"choices": [{"value": "date", "label": "Date"}]}` in the
  `props` of a field limits it to those values; the client shows the labels and
  goes from one to the next. `{"choices_from": {"schema": "xila", "table":
  "own_table", "value": "name"}}` takes more of them from a table or a view; the
  server reads them, and the client only ever receives `choices`.
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
  pixel, row by row; each byte is a palette slot. Slot 255 is transparent: such
  a pixel shows what is behind the picture. Images are never changed nor
  deleted; see section 10.8.
- **Grid.** A `grid` widget lists the rows of a table or view named in `props`:
  `{"schema": "xila", "table": "visible_stack", "key": "name", "columns":
  [{"column": "title", "label": "Stack"}]}`. Choosing a row runs the widget's
  action with the key of that row. Two more properties are optional:
  `"where": {"stack": "name"}` keeps the rows whose column `stack` equals the
  column `name` of the card on screen, so that a grid lists what belongs to that
  card; `"order": ["ord", "name"]` sorts by these columns instead of the key,
  from last to first with `"descending": true`. `{"view": "icons", "icon":
  "icon"}` asks for the rows to be shown as icons, as on a desktop, and names
  the column that holds the image of each row; the graphical client then draws
  each row as its icon, twice its size, over the text of the first column and
  that of the second ("12 cards"). The text client shows the rows.
  `{"view": "talk", "who": "kind"}` asks for the rows to be shown as a
  conversation; see section 11.7.
- **Actions and keys.** An action or a key binding without a background is
  available on every screen. A key bound on a screen wins over the same key
  bound everywhere. Keys are `F1` to `F24` and `Enter`. `F3`, `F9`, `F11` and
  `F12` mean the same on every screen: leave pgXila, call the agent, author
  mode, go back. A stack cannot bind them. `{"for": "author"}` in the `args`
  of an action keeps it, and its keys, for the members of `xila_author`;
  `{"for": "editor"}` for those who may change the stack on screen; `{"for":
  "admin"}` for the members of `xila_admin`. `{"except": ["agent"]}` keeps an
  action shared by every screen off some of them: the key that leads to a
  screen is not listed on that screen.

## 3. Screen definition

The storage format is the set of rows above. The client never joins them itself:
it calls `xila.open_stack` and receives one JSON document.

```json
{
  "stack": { "name": "addressbook", "title": "Address book", "kind": "user",
             "may_edit": false },
  "source": { "has_cards": true, "count": 12, "managed": true },
  "widgets": [
    { "name": "title", "kind": "text", "layer": "background", "ord": 10,
      "props": { "template": "{last_name}, {first_name}", "style": "title" },
      "layout": { "text": { "x": 0, "y": 0, "w": 0, "h": 0, "style": { "hidden": true } } } },
    { "name": "last_name", "kind": "field", "layer": "background", "ord": 20,
      "label": "Last name", "column": "last_name", "type": "text", "required": true,
      "props": {}, "layout": {} },
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
  "colours": [],
  "events": []
}
```

A button whose script handles the click has `script` as its action, and the
keys a script asks for are listed with the others, bound to `script` too.
`events` holds `open` when the script of the screen wants to be told that a
card came on screen (section 9.4).

Coordinates: `text` layouts are in character cells on an 80 by 24 grid. The
client keeps rows 0 to 2 for the header and rows 21 to 23 for messages and
function keys. `graphic` layouts are in pixels, relative to the content area of
the stack window, on the 480 by 300 screen.

### The graphic renderer

`may_edit` says whether the current role may change the definition of the
stack, and `managed` whether pgXila created its table and may alter it. A field
says whether it is `required`. An action kept for some users says for whom
(`"for": "editor"`): such an action serves the author screens, not the buttons
of a stack.

The graphical client shows one stack at a time, in a window whose size follows
its content. Without any `graphic` layout row, it places the widgets itself:

- **A stack with cards** is a sheet of paper. The `title` text is written large
  at the top, with the position in the stack on its right. Under it, the first
  `image` widget is on the left, and the fields form a column with their labels,
  in `ord` order. The buttons are in a row under the sheet.
- **A static screen** starts with a banner made of its `title` and `subtitle`
  texts. The other texts follow, one per line, then the fields, the grid and a
  row of buttons.
- **A system stack** is drawn as a static screen even when it shows a card: it
  is a dialog. The window is wider when its grid has more than two columns.

A `graphic` layout row changes that for one widget. Without a size (`w` and
`h` at zero), the widget keeps its automatic place and only `style` is used.
With a size, the widget is drawn exactly in that rectangle, and the layout of
the card is said to be made by hand:

- the sheet of paper is drawn behind everything that is not a button, and goes
  down to under the lowest of those widgets;
- the header of the sheet is where the `title` text is, and a ruled line runs
  under each field and each line of text;
- a widget that has no rectangle goes under those that have one.

The visual editor (section 10.7) writes the rectangle of every widget at once,
where the automatic placement had put them, so that a layout looks the same
when it stops being automatic.

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
| `xila.hello()` | `jsonb`: API version, extension version, instance name, current role, whether it is an author, name of the home stack. A client refuses a server whose API version is not its own |
| `xila.palette()` | set of `(slot, name, rgb)`: the 32 system colours |
| `xila.open_stack(stack text)` | `jsonb`: the screen definition of section 3 |
| `xila.card(stack text, key text, move text)` | `jsonb`: one card (see below) |
| `xila.save_card(stack text, key text, changes jsonb)` | `jsonb`: the saved card |
| `xila.grid_rows(stack text, widget text, first int, count int, card text)` | `jsonb`: `{"total": n, "rows": [{"key": …, "cells": {column: text}}]}`. `card` is the key of the card on screen, for a grid with a `where` property |
| `xila.image(image_id uuid)` | `(width, height, pixels bytea)` |
| `xila.act(stack text, action text, context jsonb)` | `jsonb`: a list of effects (section 5) |
| `xila.delete_card(stack text, key text)` | `jsonb`: `{"next": key}`, the card to show in its place, if there is one left |

`xila.visible_stack` is a view of the user stacks the current role may open,
with their number of cards. The stack list is a grid over it.

**`xila.card`.** `move` is one of `first`, `last`, `at`, `next`, `previous`; the
last three are relative to `key`. `next` and `previous` stop at the ends. When
`key` no longer exists, the first card is returned, except for a view, where it
is an error. The result is:

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
a null `key` and a `count` of 0. A card that is not saved yet has no computed
texts: the client shows a template without placeholders as it is written, and
leaves the others empty.

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
| `XL005` | A script failed. The message names the button and the line |

## 5. Actions

Buttons and function keys do not carry code. They name an action, and the client
asks the server to run it:

```sql
SELECT xila.act('addressbook', 'next_card',
                '{"card": "0193a5e2-…", "dirty": {"city": "Paris"}}');
```

`context` is the state of the client: `card` is the key of the card on screen
(null for a card not saved yet), `dirty` holds what the user typed and has not
saved, `row` is the key of the selected grid row, and `confirmed` is true once
the user has accepted the question the action asked. `renderer` says which
client this is, `text` or `graphic`, and `input` holds what a tool of the
client, such as the visual editor or the drawing editor, has to say. `event`
says what triggered the action: `{"on": "click", "widget": "new"}` for a button
or a list, `{"on": "key", "key": "F6"}` for a key, `{"on": "open"}` when a card
came on screen; the scripts of a screen answer to it. The server looks up the
action, runs its handler, and answers with effects that the client applies in
order:

```json
[ { "do": "show", "stack": "addressbook", "card": "0193a5e3-…" } ]
```

| Effect | Meaning |
|--------|---------|
| `show` | Display a stack, at a given card if `card` is present. A stack that is already open underneath comes back to the front instead of being opened twice |
| `back` | Return to the previous screen. A screen that no longer exists is skipped |
| `reload` | Read the current screen again: its definition, its card and the rows of its grids |
| `focus` | Put the cursor in a widget; in the visual editor, select it |
| `message` | Show a message to the user |
| `new` | Display an empty card, to be saved as a new one. `values`, if present, fills some of its fields in; they are sent back like what the user types |
| `confirm` | Ask the question in `text`. If the user accepts (`label` says what accepting does), the client runs the same action again with `"confirmed": true` in its context |
| `select` | Select the row of the grid whose key is `row` |
| `design` | Open the visual editor on the stack named by `stack`. Only sent to a graphical client |
| `draw` | Open the drawing editor on a picture: `target` is `icon`, `card` or `widget`, `widget` names the widget that shows it, `image` is the image it shows now, if any, and `width` and `height` the size of the drawing. Only sent to a graphical client |
| `set` | Put `values` in the fields of the card on screen, as if the user had typed them; a null value empties the field. Nothing is saved |
| `script` | Open the script editor on the script of the button named by `widget`, or of the screen without one: `title` and `source`. Only sent to a graphical client |
| `tested` | What the test of a script did, for the script editor: `ok`, the `line` of the error if there is one, and `lines` to read |
| `quit` | Leave the client |

The handlers:

| Handler | What it does |
|---------|--------------|
| `open_stack` | Shows the stack named in `args`, or else the selected grid row |
| `next_card`, `previous_card`, `first_card`, `last_card` | Saves what was typed, then shows the other card |
| `save_card` | Saves what was typed; creates the card if it is new |
| `new_card` | Shows an empty card |
| `delete_card` | Asks, then deletes the card on screen and shows the next one |
| `history` | Shows the history of the cards of the stack (`{"of": "cards"}`), of the stack edited in author mode, or the deleted stacks (`{"of": "deleted"}`) |
| `rewind` | Asks, then rewinds from the selected line of a history; `args` gives the scope (section 10.5) |
| `back`, `quit` | Ask the client to go back, or to leave |
| `author`, `browse` | Enter author mode on the current stack or on the selected one (the visual editor for a graphical client, the author dialog for a text client, or for anyone with `{"form": true}`); open the stack being edited |
| `new_stack`, `create_stack`, `save_stack`, `delete_stack` | The dialogs of a stack in author mode |
| `new_field`, `edit_field`, `save_field`, `remove_field`, `move_field` | The dialogs of a field in author mode |
| `place_widgets`, `reset_layout`, `add_widget`, `set_widget`, `remove_widget`, `undo` | What the visual editor asks for, with its request in `input`; see section 10.7 |
| `draw`, `save_drawing`, `set_colour`, `remove_colour` | What the drawing editor asks for; see section 10.8 |
| `script` | Runs the script of what triggered the action; see section 9.4 |
| `edit_script`, `test_script`, `save_script` | What the script editor asks for; see section 9.6 |
| `agent`, `ask_agent`, `stop_agent`, `new_conversation`, `conversations`, `open_conversation`, `agent_journal`, `agent_settings`, `save_agent_settings` | The agent and its screens; see section 11.7 |
| `apply_proposal`, `refuse_proposal`, `view_proposal` | What the person does with a change the agent proposes; see section 11.10 |

The author handlers read the context and call the functions of section 10.3.
On the author screens the key of the card is the subject of the dialog: the
name of a stack, the identifier of a field.

What a stack author wants a button or a key to do beyond these is written as
a script (section 9). A script is one more handler behind the same `xila.act`
call.

## 6. Notifications

| Channel | Payload | Sent when |
|---------|---------|-----------|
| `xila_card` | `{"stack": "addressbook", "key": "…", "op": "update"}` | a card is inserted, updated or deleted |
| `xila_screen` | `{"stack": "addressbook"}` | a screen definition changes; `stack` is null when it is the palette |

Payloads carry identifiers only, never card contents, because any connected role
may listen. A client that shows the affected card or stack reads it again.

A conversation with the agent is the card of the stack `agent`: each new line
of it is announced on `xila_card` like a change to any other card, with the
identifier of the conversation as the key.

## 7. History

Every change to a stack, a screen or a card is recorded in `xila.history`:

| Column | Content |
|--------|---------|
| `id`, `at`, `txid` | order, time and transaction of the change |
| `actor` | the role that made it |
| `origin` | `user`, `agent` or `system`. `agent` is a change the agent proposed and `actor` applied (section 11.10) |
| `stack_id`, `object_kind`, `object_key` | what changed: a `card`, or a row of `stack`, `background`, `widget`, `layout`, `action`, `key_binding`, `stack_colour`, `palette` |
| `op` | `insert`, `update` or `delete` |
| `before`, `after` | the row before and after, as `jsonb` |

History starts empty in a new database: the rows the extension creates itself
(palette, system stacks) are the baseline. Restoring a backup reloads the
history it contains and does not record the reload as new changes; see
`xila.restoring()` in the extension for how a restore is recognised.

Besides the rows of the tables, two kinds of entries are written by the author
functions: `column`, for what they do to the columns of a table, and `rewind`,
the mark a rewind leaves. Rewinding, and who may read history, are in section
10.5.

Not recorded: images (they get a history with the drawing editor), and changes
made to a table by hand with `ALTER TABLE`.

## 8. System stacks

The desktop and the stack list are stacks shipped by the extension, with
`kind = 'system'`, described with the same rows as any other stack:

- `desktop`: a static screen. Its buttons run `open_stack`; its texts say
  which instance this is and who is connected. In text mode the buttons are a
  numbered menu and the texts are hidden, because the header of the screen
  already says the same. In the window they form the welcome banner.
- `stacks`: a static screen with a `grid` widget over `xila.visible_stack`.
  Choosing a row runs `open_stack`.

The author screens are system stacks too: `new_stack`, `author`, `field`,
`history`, `card_history` and `deleted_stacks`; see section 10.6. So are the
screens of the agent: `agent`, `conversations`, `agent_journal` and
`agent_settings`; see section 11.7.

The connection screen is the only screen built into the clients, because it is
shown before any connection exists. The window in which a `confirm` effect asks
its question is drawn by the clients as well.

## 9. Scripts

A script is written in **Lua 5.4** and runs inside the extension. The language
was chosen after building Lua and Rhai into the extension on PostgreSQL 19 and
running the same checks on both. The language of pgXila itself may come later,
behind the same rows.

### 9.1 What a script is

One script belongs to one object: a **button**, or the **screen** of a stack
(its background). It holds one function per event it handles.

```lua
-- script of the button "New card"
keys = { F6 = "New card" }

function on_click()
  local new = stack.new_card()
  new.city = "London"
  go_to(new)
  focus("name")
end

function on_key(key)
  if key == "F6" then on_click() end
end
```

| Event | Function | Where | When |
|-------|----------|-------|------|
| click | `on_click()` | a button | the button is pressed |
| key | `on_key(key)` | a button, the screen | one of the keys the script asks for is pressed |
| open card | `on_open()` | the screen | a card comes on screen |

- **Keys.** A script asks for keys with the table `keys`: the key, and the
  label shown in the key bar. On one screen a key belongs to one script only.
  A key of a script wins over the same key bound to an action. `F3`, `F9`,
  `F11` and `F12` can never be taken: a script cannot keep anyone inside a
  stack, nor away from the agent.
- **Top level.** Outside a function, a script only defines things. Calling the
  database or the screen there is an error, so that checking a script never
  does anything.
- **No memory.** Every event runs in a new Lua state. What must last goes in
  the database.
- Timer, database change and received message are listed in the editor and
  marked "later". Scripts on a field, on one card or on the stack, and events
  passed from an object to the one that contains it, are not in this step.

The row: `xila.script(id, background_id, widget_id, language, source, events,
keys)`. `widget_id` is null for the script of the screen. `language` is `lua`.
`events` and `keys` are filled in when the script is saved, from what it
defines. Everyone who may open the stack may read its scripts, since they run
with the rights of whoever triggers them; only those who may change the stack
may write them. A script is part of the screen: its changes are recorded in
the history, can be rewound and undone, and notify `xila_screen`.

### 9.2 What a script can do

| In a script | Meaning |
|-------------|---------|
| `card` | The card on screen, with what the user typed; `nil` on a screen without cards. `card.city` reads a field. `card.city = "London"` writes it on screen, as if typed: nothing is saved yet |
| `stack.name`, `stack.title` | The stack on screen |
| `stack.new_card()` | A new empty card, not saved |
| `stack.cards{city = "London"}` | The cards whose fields equal these values, in the order of the stack, 1000 at most; all of them without a filter |
| `save(card)` | Saves the card, new or not, as `xila.save_card` does: same checks, same history. Gives the saved card |
| `delete(card)` | Deletes the card |
| `go_to(card)` | Shows that card. A card not saved yet is shown empty with its values, to be completed and saved by the user |
| `go_to("next")` | Also `"previous"`, `"first"`, `"last"` |
| `open("addressbook")` | Opens another stack |
| `focus("name")` | Puts the cursor in a widget |
| `message("Saved.")` | Shows a message |
| `confirm("Delete it?", "Delete")` | Asks first. Nothing the script did before is kept; once the user accepts, the script runs again from the start and `confirm` gives `true` |
| `act("save_card")` | Runs an action of the screen and adds what it answers |
| `query(sql, ...)` | Runs a `SELECT` and gives its rows, 1000 at most, each a table by column name. `$1`, `$2`… take the values that follow |
| `execute(sql, ...)` | Runs an `INSERT`, `UPDATE`, `DELETE` or `MERGE` and gives the number of rows |
| `today()`, `now()` | The date and the time of the server, as text |
| `user`, `instance` | The current role, the name of the instance |
| `print(...)` | A line for the test report of the editor; ignored otherwise |

From Lua itself: `string`, `table`, `math`, `utf8`, `pairs`, `ipairs`, `next`,
`select`, `type`, `tostring`, `tonumber`, `pcall`, `error`, `assert`. Nothing
else: no file, no operating system, no module, no loading of code.

The library of Lua is written in C, where the count that stops a script does
not see. The functions that could run for very long are therefore pgXila's
own, counted like the rest, and answering as Lua's do: `string.find`,
`string.match`, `string.gmatch`, `string.gsub`, `string.rep`, `table.insert`
and `table.remove`. `table.move`, `string.pack`, `string.unpack`,
`string.packsize` and `string.dump` are not there.

### 9.3 Rights and limits

- **Rights.** A script runs with the rights of the role that triggers it, like
  an ordinary PostgreSQL function: privileges and row-level security apply to
  everything it does. `query` and `execute` take one statement that reads or
  writes rows; anything else (`CREATE`, `GRANT`, `SET ROLE`…) is refused.
- **Trust.** The script of an author therefore acts with the rights of the
  user. Open only the stacks of authors you trust. For that reason **a script
  never runs for a superuser**: the action is refused with a message.
- **All or nothing.** An event is one transaction. A script that fails leaves
  nothing behind, and the user sees its message with the line (`XL005`).
  A script may catch the error of a statement with `pcall` and go on: only
  that statement is undone.
- **It cannot block a backend.** A script is stopped after 50 million Lua
  instructions or 5 seconds, whichever comes first, and it holds 16 MB at
  most. The search of a pattern counts as instructions. `pcall` cannot catch
  these limits, nor a cancelled query.
- **Within reason.** A query gives a script 1000 rows and 4 MB at most. A
  value handed to the database may be nested 32 tables deep. One script runs
  at a time in a session: a script cannot run another one through SQL.

### 9.4 How the clients trigger it

Nothing changes in how a client asks: `xila.act`. Three additions.

- The context says what triggered the action, in `event`:
  `{"on": "click", "widget": "new"}`, `{"on": "key", "key": "F6"}` or
  `{"on": "open"}`.
- One action for all scripts, `script`, on every screen. In the screen
  definition, a button whose script has `on_click` names it as its action, and
  the keys of the scripts are listed with the others, bound to it. The
  definition also says `"events": ["open"]` when the screen has `on_open`; the
  client then runs `script` with `{"on": "open"}` each time it shows a card,
  except a card shown by `on_open` itself.
- One more effect: `{"do": "set", "values": {"city": "London"}}` puts values
  in the fields of the card on screen, as if the user had typed them.

Scripts run the same in the text client and in the window. `xila.hello`
answers `api: 3`.

### 9.5 Functions

| Function | Does |
|----------|------|
| `xila.set_script(stack, widget, source)` | Checks and saves the script of a button, or of the screen when `widget` is null. An empty source removes it. Returns the events and keys found |
| `xila.check_script(source)` | Checks only: `{"ok": true, "events": ["click", "key"], "keys": {"F6": "New card"}}`, or `{"ok": false, "line": 4, "message": "…"}` |
| `xila.test_script(stack, source, event, context)` | Runs one event of a source, saved or not, undoes everything it wrote, and answers with what it would have done, in words |
| `xila.run_script(stack, source, event, context, keep)` | The engine: runs one event of a script with the rights of the current role and answers with `{"effects": […], "printed": […], "wrote": {…}}` or `{"error": {"line": 4, "message": "…"}}`. With `keep` false, everything it wrote is undone. Used by `xila.act` and by the test |

### 9.6 The script editor

A tool of the graphical client, as the visual editor and the drawing editor
are. The text client runs scripts but does not edit them.

- **Entering it.** In the visual editor, `F8` opens the script of the selected
  button, or of the screen when nothing is selected; so does the Script button
  of the Properties window, next to what the button does. The server answers
  the action `edit_script` with the effect `{"do": "script", "widget": "new",
  "title": "Script: button “New card”", "source": "…"}`.
- **The window.** The source with line numbers and colours for comments,
  texts, numbers, keywords and the names of 9.2; the line and column of the
  cursor; whether the script was changed, and what its last test said.
- **Typing.** A new line is indented like the one before, one step further
  after a line that opens a block, and `end` lines up with what it closes.
  `Tab` and `Shift-Tab` move the selected lines. `Shift` with the arrows, or
  the pointer, selects; Control or Command with `C`, `X`, `V`, `A` copies,
  cuts, pastes and selects everything, through the clipboard of the system;
  with `Z` it undoes, a word at a time, and with `Shift-Z` or `Y` it does again.
- **Events.** A list on the right: click and key for a button, key and open
  card for the screen, ticked when the script handles them. Choosing one that
  is missing adds its empty function. Timer, database change and received
  message are listed under "Later".
- **Test** (`F5`). Runs the event chosen in the list on the card that was on
  screen, with the source as typed, saved or not; a key event is tried with
  the first key the script asks for. Everything it writes is undone. The
  editor then shows what it would have done: what it asked, what it printed,
  what it wrote in the database and its effects, in words; or the error, with
  its line marked in the margin.
- **Leaving** (`F12`, or Close) saves the script through the action
  `save_script`. A script with an error is saved too, with a warning, so that
  no work is lost; it handles no event until it is fixed.
- Keys: `F3` Quit, `F5` Test, `F12` Back. The Agent panel and the "Ask the
  agent" button of the mockup come with phase 3.

## 10. Author mode

Phase 2 was delivered in four steps: 2a, building a stack with forms,
ownership and rewinding, in both clients; 2b, the visual editor of a
background; 2c, the drawing editor; 2d, scripts. This section describes steps
2a, 2b and 2c; scripts are in section 9.

### 10.1 Ownership

A stack belongs to the role that created it (`xila.stack.owner`). Its definition
may be changed by that role and by the members of that role, and by nobody else:
sharing the right to edit a stack is a `GRANT` of the owner role. The system
stacks, and the actions and keys available on every screen, may only be changed
by a superuser.

This is enforced by row-level security on the definition tables (`stack`,
`background`, `action`, `widget`, `layout`, `key_binding`, `stack_colour`), not
by the functions below: it also holds for someone who writes the rows in SQL.
Everyone may still read every definition.

One consequence for those who write the rows in SQL as an author: a row and the
rows that depend on it cannot be inserted by one statement (a `WITH` that
inserts a widget and its layout, for example), because the policy of the second
table does not see the first row yet. Use two statements. A superuser is not
concerned.

Who may read or write the *cards* of a stack stays a matter of privileges on its
table: a stack created in author mode is visible to its author alone until
someone grants `USAGE` on its schema and privileges on its table.

Any author may still change the system palette. Images need no owner: they are
never changed (section 10.8).

### 10.2 Where a new stack keeps its cards

A stack created in author mode gets a schema of its own, named like the stack,
and a table `cards` in it, both owned by the author:

```sql
CREATE TABLE books.cards (id uuid PRIMARY KEY DEFAULT uuidv7());
```

Authors do not get the right to create schemas in the database. One narrow
`SECURITY DEFINER` function, `xila.make_schema`, creates the schema for them,
after checking the name and that the caller is an author.

pgXila only alters tables it has created. A background says so with its column
`managed`:

- **managed** (a stack created empty): a field is a widget and its column.
  Adding, renaming or removing the field alters the table.
- **not managed** (a stack created from an existing table): the table is never
  altered. The author chooses which columns are shown, their labels and their
  order. Removing a field only takes it off the screen; a field can only be
  added for a column that exists.

### 10.3 Author functions

They run with the privileges of the caller. They are what the author screens
call, what can be called directly in SQL, and what the agent will be given as
tools in phase 3.

| Function | What it does |
|----------|--------------|
| `xila.create_stack(title text, name text DEFAULT NULL, source regclass DEFAULT NULL)` | Creates a stack and returns its name, made from the title when not given. Without `source`: the schema, the table, an empty screen with its browsing buttons and keys. With `source`: a screen with one field per column of that table, which the caller must own |
| `xila.set_stack(stack text, changes jsonb)` | Changes `title`, `card_title`, the template written at the top of each card, such as `{last_name}, {first_name}`, `icon`, the identifier of an image drawn with the system colours, or `agent`, whether the agent may read the cards (section 11.2) |
| `xila.delete_stack(stack text)` | Removes the stack and its screens. The table and the cards are left in place; rewinding brings the stack back |
| `xila.add_field(stack text, label text, type text DEFAULT 'text', name text DEFAULT NULL)` | Adds a field at the end and returns its name. `type` is `text`, `number`, `date`, `boolean` or `picture` (a column that refers to an image, shown by an `image` widget). Without `name`, an identifier is made from the label. On a stack that is not managed, the name is an existing column to show. The first field of a stack becomes the title of its cards |
| `xila.set_field(stack text, field text, changes jsonb)` | Changes `label`, `name` (renames the column too) or `required` (`NOT NULL`; refused, with the number of cards concerned, while some cards have no value) |
| `xila.move_field(stack text, field text, by integer)` | Moves a field up (`-1`) or down (`1`) in the logical order |
| `xila.remove_field(stack text, field text)` | Removes a field. On a managed stack the column is dropped; its values are saved in history, so that rewinding restores them |
| `xila.place_widgets(stack text, renderer text, placements jsonb, card text DEFAULT NULL)` | Sets the rectangle of some widgets for one renderer: `{"city": {"x": 128, "y": 94, "w": 100, "h": 14}}`. Styles are kept. A widget of a single card is looked for on `card` |
| `xila.reset_layout(stack text, renderer text)` | Forgets every rectangle of that renderer: the placement is automatic again. Styles are kept |
| `xila.add_widget(stack text, kind text, props jsonb DEFAULT '{}', card text DEFAULT NULL)` | Adds a `text`, a `button` or an `image` (a picture of the background) and returns its name (`text1`, `button1`…). `props` may hold `label`, `text` (a template), `action` (the name of an action of the stack), `image` and `placement`. With `card`, the widget is on that card only |
| `xila.set_widget(stack text, widget text, changes jsonb, card text DEFAULT NULL)` | Changes `label`, `text`, `action`, `main` (the main button of the screen), `required` (for a field), `image` (for a picture of the background) or `layer` (`background` or `card`, to move a text, a button or a picture from one to the other) |
| `xila.new_image(width int, height int, pixels bytea)` | Adds an image and returns its identifier. An image identical to one that is already there is not added twice. Open to every member of `xila_user` |
| `xila.set_colour(stack text, slot int, rgb text, name text DEFAULT NULL)` | Adds an extra colour to a stack, or changes one, and returns its slot. With a null slot, the first free one is taken |
| `xila.remove_colour(stack text, slot int)` | Removes an extra colour |
| `xila.remove_widget(stack text, widget text, card text DEFAULT NULL)` | Removes a widget. A field is removed as by `xila.remove_field` |
| `xila.undo(stack text)` | Undoes the last change made to the screen of the stack; see 10.7. Returns `{"undone": n, "summary": "…"}` |
| `xila.rewind(stack text, scope text, since bigint, card text DEFAULT NULL)` | Undoes the recorded change `since` and all those after it; see 10.5. Returns `{"undone": n}` |
| `xila.rewind_to(stack text, scope text, moment timestamptz, card text DEFAULT NULL)` | The same, for everything recorded after a moment in time |

A stack created by `xila.create_stack` has the keys `F4` Delete, `F6` New, `F7`
Previous, `F8` Next, `F10` History and `Enter` Save.

The type of an existing field cannot be changed.

Views, for the author screens and for anyone:

| View | One row is |
|------|-----------|
| `xila.stack_info` | a user stack: `name`, `title`, `owner`, `managed`, `card_title`, `agent` |
| `xila.field` | a field of a user stack: `id`, `stack`, `name`, `label`, `ord`, `type`, `required` |
| `xila.own_table` | a table of the current role that a stack could be made from |
| `xila.history_line` | a line of the history of a stack: `id`, `stack`, `at`, `actor`, `summary` |
| `xila.card_history_line` | a change to a card: `id`, `stack`, `card`, `at`, `actor`, `summary` |
| `xila.deleted_stack` | a deleted stack that can be brought back: `id`, `name`, `title`, `at`, `actor` |

Times are written in the time zone of the session; the clients set it to the
local one when they connect.

### 10.4 What the author screens are made of

The author screens are system stacks, drawn by both clients like any other
screen. They rely on parts of the model that every stack can use, described in
sections 2 and 5: a view as a read-only source, a grid of the card on screen
(`where`), choices, check boxes, the `new` effect with values, the `confirm` and
`select` effects, actions kept for authors (`for`), and `may_edit`.

Each author screen is a card of a view. The key of the card is the subject of
the dialog, and what is typed on it goes to an action, never to the view:
`author` shows the card `books` of `xila.stack_info`, and `Enter` runs
`save_stack` with the card key and what was typed.

### 10.5 History and rewinding

- **Structure.** The author functions record what they do to a table as history
  rows of kind `column`: added, renamed, made required, dropped. A dropped
  column is recorded with the value it had on each card.
- **One gesture, one line.** The changes of one transaction are one line for
  the user, named after the most telling of them: "Field “year” added", "Card
  “Lovelace, Ada” changed: city". Each call of a client is a transaction.
- **Scopes.** `xila.rewind` undoes, newest first, the changes from `since` on,
  within a scope:

  | Scope | What is undone |
  |-------|----------------|
  | `card` | the values of one card, named by `card`; a deleted card comes back |
  | `screen` | the definition: widgets, layouts, actions, keys, title, and the columns that go with the fields. Cards keep their other values |
  | `stack` | the definition and all the cards |

- **Nothing is lost.** A rewind is recorded as new changes, followed by a mark
  of kind `rewind`. Rewinding a rewind brings back what it had undone, including
  the values of a column.
- **All or nothing.** A rewind is one transaction. If a step cannot be undone,
  for example because the table was altered by hand in the meantime, nothing is
  changed and the message says which step failed.
- **Order.** When a row goes away with those that depend on it, PostgreSQL
  records the parent first. Undoing newest first would bring a child back before
  its parent: such a step waits until the row it needs is back.
- **A deleted stack** is rewound by its name; `xila.deleted_stack` lists those
  the current role owned, with the change to rewind to.
- **Who may rewind.** `card`: whoever may write the card. `screen` and `stack`:
  whoever may edit the stack.
- **Who may read history.** The definition changes of a stack: whoever may open
  it. The card changes: whoever may read the whole table; when the table uses
  row-level security, only those who may edit the stack, so that history never
  shows a row that the table hides. This is enforced by row-level security on
  `xila.history`. Nobody writes history directly, except for the two kinds of
  entries the author functions add.

### 10.6 Screens

| Screen | Reached by | Content and keys |
|--------|-----------|------------------|
| My stacks | | For authors: `F6` New stack, `F10` Deleted stacks, `F11` Author mode on the selected stack |
| New stack | `F6` on My stacks | Title, name (made from the title when left empty), start from an empty stack or from one of the caller's tables. `Enter` creates it and opens author mode: the visual editor in the window, the author dialog in the terminal |
| Author mode | `F11` on a stack one may change, or on My stacks, in the text client; `F4` in the visual editor | Title, card title, whether the agent may read the cards, the list of fields with label, name, type and required. `Enter` saves, or opens the selected field. `F6` New field, `F7` Move up, `F8` Move down, `F10` History, `F11` Browse the stack, button Delete stack |
| Field | `F6` or `Enter` in author mode | Label, name, type (text, number, date, yes or no, picture), required. `Enter` saves. Button Remove, with a confirmation that says how many cards have a value |
| History | `F10` in author mode | The changes of the stack, newest first: when, who, what. `F6` Rewind the screen from the selected line on, `F7` or `Enter` Rewind the whole stack |
| History of the cards | `F10` on a stack | The changes made to its cards. `F6` or `Enter` rewinds the card of the selected line, from that change on |
| Deleted stacks | `F10` on My stacks | The stacks the author deleted. `F6` or `Enter` brings the selected one back |

`Escape` leaves a screen like `F12`, and gives up a question.

Left for later steps: sharing the cards of a stack from the client. A stack
can also be made from a description, with the agent (section 11.10).

### 10.7 The visual editor

The visual editor is a tool of the graphical client: the background of a stack
is shown with its widgets, which are selected, moved and resized with the
pointer or the keyboard. It is the author mode of the window; the text client
has the dialogs of 10.6.

**Entering it.** The `author` handler answers a graphical client with the
effect `{"do": "design", "stack": "books"}`, and so does `create_stack`. From
the editor, `F4` opens the author dialog (title, list of fields, deletion),
`F10` the history of the stack, and `F11` goes back to browsing.

**What the editor sends.** Like everything else, an action, with what the tool
has to say in the `input` of the context. These actions are available on every
stack one may change; each calls the function of the same name (10.3).

| Action | `input` |
|--------|---------|
| `place_widgets` | `{"placements": {"city": {"x": 128, "y": 94, "w": 100, "h": 14}}}` |
| `add_widget` | `{"kind": "text", "layer": "background", "props": {"placement": {…}}}`; with `"layer": "card"`, the widget goes on the card of the context |
| `set_widget` | `{"widget": "city", "changes": {"label": "Town"}}` |
| `remove_widget` | `{"widget": "city"}`; asks first |
| `reset_layout` | nothing; asks first |
| `undo` | nothing |

`author_form`, `new_field` and `stack_history` open the author dialog, the Field
dialog and the history of the stack from the editor. Fields are created by
`xila.add_field`: the field tool opens the Field dialog.

**Automatic or free.** A stack is laid out automatically until one of its
widgets is given a place. At that moment the editor sends the rectangle of
*every* widget, where the automatic placement had put it: from then on the
layout is made by hand, and what is on screen is what is in the database (see
section 3 for how such a layout is drawn). `xila.reset_layout` goes back. A
field added later goes under the lowest field, with the same left edge and
width. The text renderer is not concerned.

**The card layer.** A text or a button may belong to one card only (`layer =
'card'`): a note on the card of Ada. The editor creates widgets on the layer
chosen in its tool palette, and shows those of the card on screen with the
others, with a violet dotted line around them. Fields always belong to the
background.

**Undo.** `xila.undo` rewinds the most recent line of the history of the
*screen* that is not itself an undo. Called again, it goes one line further
back; any new change starts over from the most recent line. It does not undo
the creation of the stack, and it only undoes changes made by the current role:
when the last change is someone else's, it refuses and points to the history of
the stack. An undo is a rewind like any other, recorded and marked as an undo,
and can be rewound from the History screen.

**In the editor**, drawn by the client:

- a tool palette in the left margin: pointer, field, text, button, picture;
  the layer for new widgets (background or card); grid and guides on or off;
- the background with a dotted grid. Fields show the name of their column, in
  grey, instead of the values of a card, and the title shows its template;
- selection handles; moving and resizing snap to the edges and centres of the
  other widgets, shown by a red guide line, and otherwise to a grid of 4 pixels;
- a floating Properties window: label or text, action, required, main button,
  on every card or on this one, position and size as numbers. It sits in the
  corner that leaves the selected widget in sight, can be moved, and is hidden
  with a key. With nothing selected, it says whether the layout is automatic
  and offers to reset it;
- with the keyboard: `Tab` selects the next widget, the arrows move it by one
  pixel and `Shift` with the arrows resize it (several presses are saved as one
  change), `Enter` goes to its properties, `Delete` removes it after a
  confirmation, `Escape` lets go of the tool, then of the selection, then
  leaves the editor;
- keys: `F2` Undo (also Control-Z, or Command-Z on a Mac), `F4` Stack, `F5` Icon
  (the drawing editor, on the icon of the stack), `F6` New widget, `F7`
  Properties, `F10` History, `F11` Browse, `F12` Back.

The script of a button, and of the screen, is written in the script editor
(section 9.6): `F8`, or the Script button of the Properties window.

Left for later: the Agent button (phase 3), the slider, grid and chart tools,
renaming a widget from the editor (a field is renamed in its dialog).

### 10.8 Drawing

The drawing editor is a tool of the graphical client, like the visual editor:
a picture is drawn pixel by pixel, with the colours of the palette.

**Images never change.** A row of `xila.image` is written once. Saving a
drawing adds an image, and what showed the old one now points to the new one:
the column of a card, the icon of a stack, a widget of the background.
Rewinding therefore needs nothing more: the history of the card, of the stack
or of the widget holds the old reference, and the old image is still there.
Nobody can overwrite the picture of someone else. Every member of `xila_user`
may add images (`xila.new_image`); nobody but a superuser may change or delete
one. Old versions stay in the database; a way to purge those that nothing
refers to any more comes later.

**Transparency.** Palette slot 255 is transparent. The extra colours of a stack
are therefore slots 32 to 254.

**What is drawn.**

| Picture | Where it is kept | Who may draw it | Colours |
|---------|------------------|-----------------|---------|
| The icon of a stack | `xila.stack.icon_id` | whoever may edit the stack | the 32 system colours |
| The picture of a card | a column of the card, of field type `picture` | whoever may write the card | the 32 system colours and those of the stack |
| A picture of the background | the `image` property of an `image` widget | whoever may edit the stack | the same |

An icon is limited to the system colours because the list of stacks shows the
icons of several stacks at once, each of which may have its own extra colours.
The server refuses an icon that uses another colour.

**Actions.** On every screen:

- `draw`, with `{"target": "card" | "icon" | "widget", "widget": "photo"}` in
  its input, checks that the current role may draw that picture and answers
  with the effect `draw`, which opens the drawing editor on the image the
  picture shows now, or on an empty drawing. The size of an empty drawing is
  that of the frame of the widget, or 68 by 80 for the picture of a card, and 28
  by 26 for an icon. The card must be saved before its picture is drawn;
- `save_drawing`, with the same target and `width`, `height` and `pixels` (two
  hexadecimal digits per pixel), adds the image and points the card, the stack
  or the widget to it, in one transaction. The picture of a card is saved like
  any other value of the card, by `xila.save_card`;
- `set_colour`, with `{"slot": 32, "rgb": "#ab12cd"}` (or no slot, for a new
  colour), and `remove_colour`, with `{"slot": 32}`, for those who may edit the
  stack.

**The list of stacks in icons.** `xila.visible_stack` has a column `icon`, and
the grid of "My stacks" asks to be shown as icons (section 2). A stack without
an icon gets a generic one, drawn by the client.

**Reaching the editor.** A click on the picture of a card, or `Enter` when it
has the focus; `F5` in the visual editor for the icon of the stack; the picture
tool of the visual editor, then "Draw" in Properties, for a picture of the
background. Leaving the editor saves the drawing if it changed, as leaving a
card saves what was typed on it; if the server refuses it, the editor stays
open.

**In the editor**, drawn by the client, as in the mockup:

- the picture enlarged, with a grid, transparent pixels shown as a checkerboard;
- tools: pencil, eraser (transparent), fill, line, rectangle (outlined or
  filled), pipette, selection. A selected area is moved by dragging it or with
  the arrows, duplicated (`D`), erased (`Delete`) and put down (`Enter`);
- the whole picture is flipped (`H`, `V`), turned a quarter (`T`), or given
  another size (`F6`), which keeps what is already drawn at the top left;
- a Colours window: the 32 system colours, the extra colours of the stack with
  `+` and `-` to add and remove one, and the name, the number and the value of
  the colour in hand; the value of a colour of the stack is typed there;
- a Preview window: the picture at its real size on the desktop, and enlarged
  on paper;
- `F2` undoes, step by step, what was drawn since the editor was opened; `F4`
  goes back to the picture as it was;
- with the keyboard: the arrows move a cursor over the pixels and the space bar
  is the button of the pointer there (it starts and ends a line, a rectangle or
  a selection); a letter picks each tool (`P`, `E`, `F`, `L`, `R`, `I`, `S`),
  `+` and `-` change the zoom, `G` shows or hides the grid, `X` fills rectangles
  or not.

An image is at most 480 by 300 pixels.

Left for later: importing an image from a file, purging unused images, the
Agent button.

## 11. The agent

Phase 3 is delivered in three steps: 3a, talking with the agent from a screen
and letting it read; 3b, the agent proposes changes that the user applies or
refuses; 3c, memory, with pgvector. This section describes steps 3a and 3b,
which are implemented.

| Decision | Choice |
|----------|--------|
| Where the agent runs | In a background worker, never in the backend of a user |
| What it can do | Read, answer, and propose changes. It changes nothing itself: the person applies a proposal or refuses it (11.10) |
| What it may read | The cards of the stacks left open to it, and never more than the person it talks with may read |
| A stack and the agent | Open by default; whoever may edit the stack closes it. Being open is a `GRANT` to the role `xila_agent` |
| Its tools | SQL functions, listed in a table, run under the role `xila_agent` |
| The model | Any endpoint that speaks the OpenAI chat API (OpenRouter, LM Studio…) |
| Its settings | Address and model in a table, changed by an administrator. The token is never in the database |
| Its key | `F9`, on every screen. A stack cannot take `F9` for itself |

### 11.1 What happens when someone asks

1. The user writes on the agent screen and presses `Enter`. The server stores
   the line in the conversation and marks the conversation as waiting. This is
   all the backend of the user does: it returns at once.
2. The worker is woken when that transaction is committed. It sends the
   conversation to the model, with the list of tools. The call is made on a
   thread of the worker, so that the worker stays able to stop at any moment;
   a call may last 120 seconds.
3. When the model asks for a tool, the worker runs the SQL function of that
   tool, adds the result to the conversation, and calls the model again. A
   question gets at most 8 calls to the model.
4. When the model answers in words, the worker stores the answer, marks the
   conversation as idle and notifies the clients, which read the screen again.

Each step is stored as soon as it is done: the user sees "Looked for “ada” in
Address book: 2 cards." appear while the agent works. The conversation is the
whole state of the work: a conversation that was waiting when the server
stopped is taken up again when it starts.

The worker answers up to 4 conversations at a time; the others wait their
turn. A conversation takes one question at a time, and `F5` gives up the one
being answered: whatever comes back for it afterwards is dropped. What the
worker does in the database for one step is limited to 15 seconds, so that a
tool that reads a huge table does not keep every conversation waiting.

When the model cannot be reached, refuses the token, or answers nonsense, the
conversation gets a line that says so ("The model refused the token (HTTP
401). An administrator can check the agent settings."), and nothing else
happens. The token is never written in a message, in the journal or in the log
of the server.

### 11.2 Roles and rights

| Role | Login | Purpose |
|------|-------|---------|
| `xila_agent` | no | The role of the worker. It owns nothing, cannot create anything, cannot write to a stack, and is not a member of `xila_user` |
| `xila_admin` | no | Group role, member of `xila_author`. May change the agent settings and read the whole journal |

`xila_agent` may read the definitions of the stacks, read and write the tables
of this section, run the tool functions, and read the cards of the stacks that
are open to it. It can write nowhere else.

**A stack is open or closed to the agent.** `xila.stack.agent_reads` is true
by default. Setting it grants or revokes `SELECT` on the table of the stack to
`xila_agent` (and `USAGE` on its schema): the setting is a real privilege, not
a flag that a function is trusted to check. It is changed by whoever may edit
the stack, with `xila.set_stack(stack, '{"agent": false}')` or the check box
"The agent may read the cards" of the author dialog, and it is recorded in the
history like the title. Deleting a stack takes the privilege away from the
table it leaves behind.

**Never more than the person.** The worker reads for someone. Every tool
receives the role of the person who asked, and refuses a stack that this
person could not open: for the agent, such a stack does not exist. When the
table of a stack uses row-level security, the agent reads it only for those
who may edit the stack, as for history (section 10.5), so that it never shows
a row that the table hides.

**What leaves the server.** What the agent reads is sent to the model: the
question, the titles and the fields of the stacks the person may open, and the
cards the tools returned. Of a closed stack, only the name and the title are
sent, so that the agent can say that it is closed. The address must be
`https://`; plain `http://` is accepted for the machine itself only
(`localhost`, `127.0.0.1`, `[::1]`, `host.docker.internal`), which is what a
local LM Studio needs. The worker follows no redirection.

**What a card can make it do.** The model reads cards as text, and a card may
hold text written to steer it. The worst such a card can do is to make an
answer wrong, or a proposal unwanted: the agent has no tool that writes, sends
or fetches anything, and a proposal changes nothing until the person has read
it and applied it.

### 11.3 Settings

`xila.agent_setting` has one row:

| Column | Content |
|--------|---------|
| `address` | Base address of the API, such as `https://openrouter.ai/api/v1`; an address that ends with `/chat/completions` is used as it is |
| `model` | Name of the model, as the endpoint knows it |
| `token_variable` | **Name** of the environment variable of the server that holds the token, such as `OPENROUTER_API_KEY`; empty for an endpoint that needs none. Upper-case letters, digits and `_`; a variable of PostgreSQL itself (`PGPASSWORD`, `PG_COLOR`, `POSTGRES_PASSWORD`…) is refused |
| `options` | JSON object merged into each request, for what an endpoint or a model needs (`{"temperature": 0.2}`). It cannot replace the model, the conversation or the tools |
| `instructions` | Text added to what the agent is told about itself: the house rules of this instance |

`xila.set_agent(changes jsonb)` changes them; only a member of `xila_admin`
may call it. Whoever chooses the address chooses where the token is sent:
this is why the settings are not open to every author.

The token itself is read by the worker from its environment when it calls the
model. With Docker Compose it comes from `.env.local`. It is in no table, so
in no backup, and no SQL function returns it.

`xila.agent_info` is a view for everyone: `model`, `host` (the host of the
address, so that people know where their questions go), `ready`.

`scripts/set-agent.sh <address> <model> [token-variable]` sets the three on
the development server, and `scripts/create-user.sh ada --admin` creates an
administrator.

### 11.4 Data model

| Table | One row is | Main columns |
|-------|-----------|--------------|
| `xila.agent_setting` | the settings | see 11.3 |
| `xila.agent_tool` | a tool | `name`, `label` (what people read), `description` and `parameters` (what the model reads: a text and a JSON schema), `function`, `applies` (11.10), `ord` |
| `xila.conversation` | a conversation | `id`, `owner`, `title` (the first question), `state` (`idle` or `waiting`), `stack`, `card` (what the owner was looking at when calling the agent), `started_at`, `changed_at` |
| `xila.agent_message` | a line of a conversation | `id`, `conversation_id`, `at`, `kind` (`user`, `agent`, `tool`, `error`, `note`), `text` (what people read), `payload` (what the model is sent back: its own message, or the result of the tool). A `note` says something that happened, to the person and to the model alike: a proposal was applied, a question was given up |
| `xila.agent_proposal` | a change the agent proposes | see 11.10 |
| `xila.agent_log` | a journal entry | `id`, `at`, `conversation_id`, `role` (for whom), `event` (`model`, `tool`, `refused`, `failed`), `tool`, `summary`, `detail`, `ms`, `tokens_in`, `tokens_out` |

Row-level security: a conversation and its lines are read by its owner alone
(and by the worker). A user adds lines of kind `user` and `note` to their own
conversations, and nothing else. The journal is written by the worker only;
people read their own entries, administrators all of them. Nobody can change
or delete a journal entry.

The journal has one entry for each call to the model (how long, how many
tokens) and one for each tool run (which, its arguments, what it found in a
sentence, or why it was refused). It does not copy the cards.

The settings, the conversations, the proposals and the journal are part of a
backup.

### 11.5 Tools

A tool is a SQL function `f(as_role name, arguments jsonb) RETURNS jsonb` and a
row of `xila.agent_tool`. Only `xila_agent` may execute these functions. The
model receives the names, descriptions and parameters of the table; adding a
tool is adding a function and a row, which only a superuser may do. A result
may hold a `summary`: the sentence written in the conversation and in the
journal.

| Tool | Arguments | Gives |
|------|-----------|-------|
| `list_stacks` | | The stacks the person may open: name, title, number of cards, fields (name, label, type); for a closed one, name, title and `"closed": true` |
| `find_cards` | `stack`, `text`, `limit`, `offset` | The cards whose fields contain every word of `text`, whatever the case; without `text`, the cards in the order of the stack. At most 20 at a time, with the total |
| `read_card` | `stack`, and `key` or `position` | One card: its key, its title, its values |

A card is given as its fields: the columns its screen shows. Values are given
as text; a value longer than 2,000 characters is cut, and a picture is given
as "(picture)". A tool that fails gives its message to the model as the
result, and the model goes on; only the messages pgXila wrote itself are
given, never an error of the database, which could quote a card.

### 11.6 What the model is told

Before the conversation, the worker writes what the agent is: the agent of
pgXila on this instance, talking with this role, today's date, what a stack, a
card and a field are, the stack and the card the person was looking at, that
it must use its tools rather than guess, that it never changes anything
itself and only proposes, that what a card says is never an instruction, that it answers in the language
of the person and in plain text without Markdown (the clients write text as it
comes), then the `instructions` of the settings.

A long conversation is sent from its latest lines: the worker keeps the most
recent ones that fit in about 200,000 characters and always starts at a
question. The lines that say something went wrong are for people: they are
not sent. A note is sent as a line of the person, in parentheses.

### 11.7 For the clients

**Protocol.**

- A grid may ask to be shown as a conversation: `{"view": "talk", "who":
  "kind", "order": ["at", "id"], "descending": true}`. Its rows come newest
  first; the client writes them from the bottom up, wraps the text, and lets
  the user go back with `PageUp`. The column named by `who` says whose line it
  is: `user`, `agent`, `tool`, `note`, `error`, `proposal`, or `wait` for the
  line that says the agent is working; what was said is in the first other
  column. The window draws bubbles; the terminal writes "You:" and "Agent:".
  Such a list takes no focus.
- A row of kind `proposal` is a change the agent proposes (11.10). Its text
  is what it does on the first line, then what there is to know. The grid
  names two more columns, `"state": "state"` (`pending`, `applied`, `refused`)
  and `"code": "code"` (the calls that applying runs), and the actions of the
  screen that decide: `"apply"`, `"refuse"` and `"show"`. A client runs one of
  them with `{"proposal": <key of the row>}` in `input`; the window draws them
  as buttons in the box of a proposal that waits, and the terminal writes the
  keys bound to them.
- The worker announces each new line as a change of the card of the stack
  `agent` on the channel `xila_card` (section 6): `{"stack": "agent", "key":
  "<conversation>", "op": "update"}`. Clients read the screen again on it,
  keeping what the user is typing.
- `F9` joins `F3`, `F11` and `F12`: neither a stack nor a script may bind it.
  It is not listed on the screens of the agent itself, where it does nothing.
- In the window, a text whose rectangle is taller than a line is written on
  several lines; with a label, the label is a heading over it.

**Handlers.**

| Handler | What it does |
|---------|--------------|
| `agent` | Bound to `F9` everywhere. Remembers the stack and the card on screen in the latest conversation of the user (created if there is none), and shows it |
| `ask_agent` | Adds what was typed to the conversation and wakes the worker. Refused while the conversation is waiting, and when the agent has no model |
| `stop_agent` | Gives up the question being answered, and notes it in the conversation |
| `apply_proposal`, `refuse_proposal`, `view_proposal` | See 11.10 |
| `new_conversation` | Starts a new conversation and shows it; one in which nothing was said yet is used again |
| `conversations`, `open_conversation` | Lists the conversations of the user; shows the selected one |
| `agent_journal` | Shows the journal |
| `agent_settings`, `save_agent_settings` | The settings dialog, for administrators |

`xila.ask_agent(conversation uuid, question text)` is what `ask_agent` calls.

**Screens.** They are system stacks, drawn by both clients.

| Screen | Reached by | Content and keys |
|--------|-----------|------------------|
| Agent | `F9` anywhere, or Agent on the desktop | The conversation, and a line to write on. Beside it in the window, as in mock-up 03, and on one line in the terminal: the model and where it is, the tools, the guard rails, and what the agent did today for this user ("5 actions, 2 applied, 1 refused": tools run, proposals applied, proposals refused). `Enter` Send, `F2` Apply, `F4` Refuse, `F5` Stop, `F6` New conversation, `F10` List of the conversations, `PageUp` and `PageDown` |
| Conversations | `F10` on Agent | The conversations of the user, newest first: title, date, number of lines. `Enter` opens one. `F6` New, `F7` Journal, and `F4` Settings for an administrator |
| Journal | `F7` on Conversations | When, for whom, what: "Looked for “ada” in Address book: 2 cards.", "Model: 1.1 s, 354 tokens", "Proposed: add the field “Birthday” to Address book.", "Applied: Add the field “Birthday” to Address book" |
| Agent settings | `F4` on Conversations | Address, model, token variable, instructions. `Enter` saves |

### 11.8 Where the code is

Everything the agent decides is decided in SQL, in `extension/sql/agent.sql`:
what to send (`xila.agent_request`), what an answer means and which tool to
run for whom (`xila.agent_answer`). The worker only carries a request to the
model and its answer back; `extension/src/workers/` holds it.

A small launcher is the only worker registered when the server starts: it
waits until the database and the role `xila_agent` exist, starts the agent
worker, and starts it again if it dies. The `link` worker of phase 4 will be
one more worker for the launcher to start. The database is named by the
server parameter `pgxila.database` (`pgxila` by default).

The extension has one dependency for this, `ureq` with `rustls`, for HTTPS.
The tests never call a real model: the integration tests run a small fake
model and check what the worker sends to it.

### 11.9 Not yet

- Memory across conversations and semantic search, with pgvector: step 3c. It
  needs an endpoint that computes embeddings.
- Answers that appear word by word; pictures sent to the model; an Agent line
  on the screen of a stack, as in the text mock-up; waking the agent on a
  database event or on a schedule; a limit on how much each person may ask.
- Opening or closing a stack to the agent from the visual editor: it is done
  from the author dialog, which the editor reaches with `F4`.
- More than the latest 100 lines of a conversation on screen; the older ones
  are still sent to the model.

### 11.10 Proposals

The agent writes nothing. It has tools that *propose* a change; the person
applies the proposal or refuses it, and what is applied is done by the session
of that person, with their rights.

| Decision | Choice |
|----------|--------|
| Who writes | The session of the person who applies a proposal. The role `xila_agent` has no right to write to a stack |
| What the agent may propose | Fields of a stack, cards, and a new stack |
| What it may not | Layout, pictures, scripts, deleting a stack, rights, and anything on a stack that is closed to it |
| How a proposal is applied | One key or one button, and no other question, whatever it does. History can undo it |
| What an applied proposal leaves | An ordinary change in the history, with the origin `agent`: it is undone and rewound like any other |

**What happens.**

1. The model asks for a writing tool, for example `add_field` with `{"stack":
   "addressbook", "label": "Birthday", "type": "date"}`.
2. The worker runs the function of that tool, which changes nothing: it checks
   that the change makes sense and that the person could make it (the stack
   exists and is open to the agent, the person may edit it, the field is not
   already there…), and describes it. The worker stores the description as a
   proposal, and tells the model that the change is proposed and waits for
   the person. The same change is not proposed twice while it waits.
3. The model says in words what it proposed. The conversation shows the
   proposal in a box: what it does, in a sentence and a few lines, and the
   calls that applying it runs.
4. The person applies it or refuses it. Applying runs the author function or
   `xila.save_card` in the session of the person, in one transaction. The
   conversation gets a note that says so, which the model reads with the next
   question.

What the function of a tool refuses is said to the model, which can correct
itself: "“long ago” cannot be the value of “Year”, which is a number field."

A proposal that was checked may still fail when it is applied, because things
changed in between: the message says why, and the proposal stays. A proposal
to change a card is refused when the fields it changes are no longer what the
agent read: nobody's edit is silently overwritten.

**Tools.** Each has a second function, `g(arguments jsonb) RETURNS jsonb`,
which applies it and is named by `xila.agent_tool.applies`. Anyone may run it:
it does what the author functions do, with the rights of the caller.

| Tool | Arguments | Proposes |
|------|-----------|----------|
| `add_field` | `stack`, `label`, `type` (`text`, `number`, `date`, `boolean`) | A new field at the end of the cards. On a stack whose table pgXila did not create, only a column that exists, given as `name` |
| `change_field` | `stack`, `field`, and `label`, `name` or `required` | Another label, another name, required or not |
| `remove_field` | `stack`, `field` | Removing a field; the proposal says how many cards have a value for it |
| `create_stack` | `title`, `fields` (label and type of each, 1 to 30) | A new stack with its fields, for an author |
| `save_card` | `stack`, `values`, and `key` to change a card rather than add one | A new card, or other values on a card. What the card already says is left out |
| `delete_card` | `stack`, `key` | Deleting a card |

**Data model.** `xila.agent_proposal`: `id`, `conversation_id`, `at`, `tool`,
`arguments` (what the function that applies receives), `title` ("Add the field
“Birthday” to Address book"), `detail` (lines such as "Year: 1759 → 1760"),
`code` (the calls, such as `xila.add_field('addressbook', 'Birthday',
'date')`), `stack` and `card` (what it is about), `state` (`pending`,
`applied`, `refused`), `resolved_at`. It is written by the worker; the owner
of the conversation reads it and may only change what became of it.

**Functions and handlers.**

| Function | What it does |
|----------|--------------|
| `xila.apply_proposal(proposal uuid)` | Applies a proposal of the current role and returns `{"summary", "stack", "card", "conversation"}`. What it changes is recorded with the origin `agent` |
| `xila.refuse_proposal(proposal uuid)` | Refuses it: nothing is changed |

`apply_proposal` and `refuse_proposal`, the handlers, act on the proposal
named in `input`, or else on the one that has waited longest in the
conversation on screen. `view_proposal` shows the stack or the card a proposal
is about.

In the history of a stack and of its cards, a change that came from the agent
says so next to who applied it: "ada (agent)".

**Not yet.** Changing the layout, a picture or a script; several changes
applied as one; proposals that expire; the agent acting without validation,
even for small things.

## 12. Not covered yet

- Scripts on a field, on one card or on a stack; timers, and scripts that
  answer to a change in the database or to a message: later steps.
- Memory of the agent: step 3c.
- Publishing stacks between instances, messaging: phase 4.
- Documents, global search.
