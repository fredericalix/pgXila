-- pgXila system schema: roles, tables, history and notifications.
--
-- Everything pgXila knows about a stack is a row in one of these tables.
-- The cards themselves are not here: a card is a row of the stack's own table.

CREATE SCHEMA xila;

-- ---------------------------------------------------------------------------
-- Roles
-- ---------------------------------------------------------------------------
-- Group roles. Every person who uses pgXila is a member of xila_user.
-- Roles belong to the cluster, not to the database, so they may already exist.
DO $$
BEGIN
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'xila_user') THEN
        CREATE ROLE xila_user NOLOGIN;
    END IF;
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'xila_author') THEN
        CREATE ROLE xila_author NOLOGIN IN ROLE xila_user;
    END IF;
    -- Administrators set up the agent. The agent itself is a role that owns
    -- nothing and cannot log in: its worker connects as that role.
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'xila_admin') THEN
        CREATE ROLE xila_admin NOLOGIN IN ROLE xila_author;
    END IF;
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'xila_agent') THEN
        CREATE ROLE xila_agent NOLOGIN;
    END IF;
END
$$;

-- ---------------------------------------------------------------------------
-- Palette
-- ---------------------------------------------------------------------------
-- The 32 system colours. Images are indexed, one byte per pixel: slots 0 to 31
-- are these colours, slots 32 to 255 are free for each stack.
CREATE TABLE xila.palette (
    slot smallint PRIMARY KEY CHECK (slot BETWEEN 0 AND 31),
    name text NOT NULL,
    rgb  text NOT NULL CHECK (rgb ~ '^#[0-9a-f]{6}$')
);

-- Restoring a dump loads the saved palette on top of the default one written
-- below. This trigger turns such an insert into an update, so that a changed
-- theme survives a dump and restore.
CREATE FUNCTION xila.palette_upsert() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    UPDATE xila.palette SET name = NEW.name, rgb = NEW.rgb WHERE slot = NEW.slot;
    IF FOUND THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER palette_upsert BEFORE INSERT ON xila.palette
    FOR EACH ROW EXECUTE FUNCTION xila.palette_upsert();

INSERT INTO xila.palette (slot, name, rgb) VALUES
    ( 0, 'black',     '#14121f'), ( 1, 'night',     '#232a4d'),
    ( 2, 'navy',      '#2f4a8a'), ( 3, 'blue',      '#3f73c4'),
    ( 4, 'azure',     '#6fb0e8'), ( 5, 'ice',       '#bfe3f5'),
    ( 6, 'cream',     '#f6ecd2'), ( 7, 'white',     '#fffdf5'),
    ( 8, 'sand',      '#e3cf9f'), ( 9, 'ochre',     '#c2a06a'),
    (10, 'brown',     '#8a6240'), (11, 'chocolate', '#573a2e'),
    (12, 'slate',     '#3d3f52'), (13, 'grey',      '#6c7086'),
    (14, 'silver',    '#a4a8b8'), (15, 'pearl',     '#d6d8e0'),
    (16, 'fir',       '#1f5a48'), (17, 'green',     '#2f9a5c'),
    (18, 'meadow',    '#7fd06a'), (19, 'lime',      '#c9f0a8'),
    (20, 'teal',      '#1f8a8f'), (21, 'turquoise', '#4fd0c2'),
    (22, 'yellow',    '#f7d046'), (23, 'amber',     '#f0a030'),
    (24, 'orange',    '#e8703a'), (25, 'red',       '#d0414a'),
    (26, 'burgundy',  '#8a2846'), (27, 'pink',      '#f08aa0'),
    (28, 'peach',     '#f8c8a8'), (29, 'violet',    '#6a4aa8'),
    (30, 'lavender',  '#a88ae0'), (31, 'plum',      '#3c2452');

-- ---------------------------------------------------------------------------
-- Images
-- ---------------------------------------------------------------------------
-- An indexed image: one byte per pixel, row by row, each byte a palette slot.
-- Slot 255 is transparent.
--
-- An image never changes. Saving a drawing adds an image, and what showed the
-- old one now points to the new one: the old image is still there for the
-- history of whatever pointed to it, and nobody can overwrite the picture of
-- someone else.
CREATE TABLE xila.image (
    id     uuid PRIMARY KEY DEFAULT uuidv7(),
    width  smallint NOT NULL CHECK (width BETWEEN 1 AND 480),
    height smallint NOT NULL CHECK (height BETWEEN 1 AND 300),
    pixels bytea NOT NULL,
    CHECK (octet_length(pixels) = width::integer * height::integer)
);

-- To find an image that is already there instead of adding it again.
CREATE INDEX image_content ON xila.image (md5(pixels));

-- ---------------------------------------------------------------------------
-- Stacks and their screens
-- ---------------------------------------------------------------------------
CREATE TABLE xila.stack (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    name       text NOT NULL UNIQUE CHECK (name ~ '^[a-z][a-z0-9_]{0,30}$'),
    title      text NOT NULL,
    kind       text NOT NULL DEFAULT 'user' CHECK (kind IN ('system', 'user')),
    icon_id    uuid REFERENCES xila.image,
    owner      name NOT NULL DEFAULT current_user,
    created_at timestamptz NOT NULL DEFAULT now(),
    -- Whether the agent may read the cards. Kept in step with a privilege on
    -- the table of the stack; see xila.sync_agent_grant.
    agent_reads boolean NOT NULL DEFAULT true
);

-- Extra colours of a stack, beyond the 32 system colours.
CREATE TABLE xila.stack_colour (
    stack_id uuid NOT NULL REFERENCES xila.stack ON DELETE CASCADE,
    slot     smallint NOT NULL CHECK (slot BETWEEN 32 AND 254),
    name     text,
    rgb      text NOT NULL CHECK (rgb ~ '^#[0-9a-f]{6}$'),
    PRIMARY KEY (stack_id, slot)
);

-- The screen shared by all the cards of a stack. The cards are the rows of
-- the source table. Without a source table, the background is a static screen
-- with a single card. The source may also be a view: its cards are then shown
-- but never saved.
CREATE TABLE xila.background (
    id            uuid PRIMARY KEY DEFAULT uuidv7(),
    stack_id      uuid NOT NULL UNIQUE REFERENCES xila.stack ON DELETE CASCADE,
    source_schema name,
    source_table  name,
    key_column    name,
    order_columns name[] NOT NULL DEFAULT '{}',
    -- True when pgXila created the table and may therefore alter it.
    managed       boolean NOT NULL DEFAULT false,
    CHECK ((source_schema IS NULL) = (source_table IS NULL)),
    CHECK ((source_table IS NULL) = (key_column IS NULL))
);

-- Something a button or a key triggers. An action without a background is
-- available on every screen.
CREATE TABLE xila.action (
    id            uuid PRIMARY KEY DEFAULT uuidv7(),
    background_id uuid REFERENCES xila.background ON DELETE CASCADE,
    name          text NOT NULL,
    label         text NOT NULL,
    handler       text NOT NULL CHECK (handler IN (
                      'open_stack', 'next_card', 'previous_card', 'first_card',
                      'last_card', 'save_card', 'new_card', 'delete_card', 'back', 'quit',
                      'author', 'browse', 'new_stack', 'create_stack', 'save_stack',
                      'delete_stack', 'new_field', 'edit_field', 'save_field',
                      'remove_field', 'move_field', 'history', 'rewind',
                      'place_widgets', 'reset_layout', 'add_widget', 'set_widget',
                      'remove_widget', 'undo', 'draw', 'save_drawing', 'set_colour',
                      'remove_colour', 'script', 'edit_script', 'save_script',
                      'test_script', 'agent', 'ask_agent', 'stop_agent',
                      'new_conversation', 'conversations', 'open_conversation',
                      'agent_journal', 'agent_settings', 'save_agent_settings')),
    args          jsonb NOT NULL DEFAULT '{}',
    UNIQUE NULLS NOT DISTINCT (background_id, name)
);

-- A widget says what the screen contains, never where it is drawn.
CREATE TABLE xila.widget (
    id            uuid PRIMARY KEY DEFAULT uuidv7(),
    background_id uuid NOT NULL REFERENCES xila.background ON DELETE CASCADE,
    layer         text NOT NULL DEFAULT 'background' CHECK (layer IN ('background', 'card')),
    card_key      text,
    name          text NOT NULL,
    kind          text NOT NULL CHECK (kind IN (
                      'field', 'text', 'button', 'image', 'grid', 'slider', 'canvas')),
    ord           integer NOT NULL DEFAULT 0,
    grp           text,
    label         text,
    bound_column  name,
    action_id     uuid REFERENCES xila.action ON DELETE SET NULL,
    props         jsonb NOT NULL DEFAULT '{}',
    CHECK ((layer = 'card') = (card_key IS NOT NULL)),
    UNIQUE NULLS NOT DISTINCT (background_id, card_key, name)
);

-- Where a widget is drawn, for one renderer. Optional: without it, the client
-- derives a placement from the logical model.
CREATE TABLE xila.layout (
    widget_id uuid NOT NULL REFERENCES xila.widget ON DELETE CASCADE,
    renderer  text NOT NULL CHECK (renderer IN ('text', 'graphic')),
    x         integer NOT NULL DEFAULT 0,
    y         integer NOT NULL DEFAULT 0,
    w         integer NOT NULL DEFAULT 0,
    h         integer NOT NULL DEFAULT 0,
    style     jsonb NOT NULL DEFAULT '{}',
    PRIMARY KEY (widget_id, renderer)
);

-- A key bound to an action. Without a background, the key works everywhere.
CREATE TABLE xila.key_binding (
    id            uuid PRIMARY KEY DEFAULT uuidv7(),
    background_id uuid REFERENCES xila.background ON DELETE CASCADE,
    key           text NOT NULL CHECK (key ~ '^(F([1-9]|1[0-9]|2[0-4])|Enter)$'),
    action_id     uuid NOT NULL REFERENCES xila.action ON DELETE CASCADE,
    UNIQUE NULLS NOT DISTINCT (background_id, key)
);

-- The script of a button, or of the screen itself when widget_id is null:
-- Lua source with one function per event. events and keys say what it
-- handles; xila.set_script works them out from the source when it saves it.
-- The keys to leave a screen, and the key of the agent, can never be taken.
CREATE TABLE xila.script (
    id            uuid PRIMARY KEY DEFAULT uuidv7(),
    background_id uuid NOT NULL REFERENCES xila.background ON DELETE CASCADE,
    widget_id     uuid REFERENCES xila.widget ON DELETE CASCADE,
    language      text NOT NULL DEFAULT 'lua' CHECK (language = 'lua'),
    source        text NOT NULL,
    events        text[] NOT NULL DEFAULT '{}'
                  CHECK (events <@ ARRAY['click', 'key', 'open']),
    keys          jsonb NOT NULL DEFAULT '{}'
                  CHECK (jsonb_typeof(keys) = 'object'
                         AND NOT keys ?| ARRAY['F3', 'F9', 'F11', 'F12']),
    UNIQUE NULLS NOT DISTINCT (background_id, widget_id)
);

-- ---------------------------------------------------------------------------
-- History
-- ---------------------------------------------------------------------------
-- Every change to a stack, a screen or a card. There is deliberately no
-- foreign key: history outlives what it describes.
CREATE TABLE xila.history (
    id          bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    at          timestamptz NOT NULL DEFAULT clock_timestamp(),
    txid        xid8 NOT NULL DEFAULT pg_current_xact_id(),
    actor       name NOT NULL DEFAULT session_user,
    origin      text NOT NULL DEFAULT 'user' CHECK (origin IN ('user', 'agent', 'system')),
    stack_id    uuid,
    object_kind text NOT NULL,
    object_key  text NOT NULL,
    op          text NOT NULL CHECK (op IN ('insert', 'update', 'delete')),
    before      jsonb,
    after       jsonb
);

CREATE INDEX history_object ON xila.history (stack_id, object_kind, object_key, id);

-- True while a backup is being restored.
--
-- A dump carries the history and the triggers of the database it was taken
-- from. Reloading it must not record the reload as new changes, nor install
-- again what the dump is about to create. pg_dump output always runs with an
-- empty search_path, which is how a restore is recognised; a superuser can
-- also say so with "SET xila.restoring = on" before a bulk load.
--
-- Only superusers are believed: for anyone else this is always false, so
-- nobody can escape history by changing a setting.
CREATE FUNCTION xila.restoring() RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT (SELECT r.rolsuper FROM pg_catalog.pg_roles r WHERE r.rolname = session_user)
       AND (current_setting('search_path') IN ('', '""')
            OR coalesce(current_setting('xila.restoring', true), '') = 'on')
$$;

-- Finds the stack a definition row belongs to. When a stack is deleted, its
-- rows go with it, parents first: the parent of a row may be gone by the time
-- the deletion of the row is recorded. History still knows where it was.
CREATE FUNCTION xila.stack_of(kind text, r jsonb) RETURNS uuid
LANGUAGE plpgsql STABLE AS $$
DECLARE
    result      uuid;
    parent_kind text;
    parent_key  text;
BEGIN
    IF kind = 'stack' THEN
        RETURN (r->>'id')::uuid;
    ELSIF kind IN ('background', 'stack_colour') THEN
        RETURN (r->>'stack_id')::uuid;
    ELSIF kind = 'palette' THEN
        RETURN NULL;
    ELSIF kind = 'layout' THEN
        parent_kind := 'widget';
        parent_key := r->>'widget_id';
        SELECT b.stack_id INTO result
        FROM xila.widget w JOIN xila.background b ON b.id = w.background_id
        WHERE w.id = parent_key::uuid;
    ELSE
        parent_kind := 'background';
        parent_key := r->>'background_id';
        SELECT b.stack_id INTO result FROM xila.background b WHERE b.id = parent_key::uuid;
    END IF;

    IF result IS NULL AND parent_key IS NOT NULL THEN
        SELECT h.stack_id INTO result
        FROM xila.history h
        WHERE h.object_kind = parent_kind AND h.object_key = parent_key
        ORDER BY h.id DESC
        LIMIT 1;
    END IF;
    RETURN result;
END
$$;

-- Records a change to a definition table and tells the clients about it.
-- SECURITY DEFINER: the history table is not writable by the roles that make
-- the changes. The triggers that call it are created in triggers.sql, once
-- the extension has written its own rows.
CREATE FUNCTION xila.record_definition_change() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    old_row jsonb := CASE WHEN TG_OP <> 'INSERT' THEN to_jsonb(OLD) END;
    new_row jsonb := CASE WHEN TG_OP <> 'DELETE' THEN to_jsonb(NEW) END;
    cur_row jsonb := coalesce(new_row, old_row);
    stack   uuid;
BEGIN
    IF old_row IS NOT DISTINCT FROM new_row THEN
        RETURN NULL;
    END IF;
    stack := xila.stack_of(TG_TABLE_NAME, cur_row);

    INSERT INTO xila.history (origin, stack_id, object_kind, object_key, op, before, after)
    VALUES (
        coalesce(nullif(current_setting('xila.origin', true), ''), 'user'),
        stack,
        TG_TABLE_NAME,
        CASE TG_TABLE_NAME
            WHEN 'palette' THEN cur_row->>'slot'
            WHEN 'stack_colour' THEN cur_row->>'slot'
            WHEN 'layout' THEN (cur_row->>'widget_id') || '/' || (cur_row->>'renderer')
            ELSE cur_row->>'id'
        END,
        lower(TG_OP), old_row, new_row);

    PERFORM pg_notify('xila_screen', jsonb_build_object(
        'stack', (SELECT s.name FROM xila.stack s WHERE s.id = stack))::text);
    RETURN NULL;
END
$$;

-- Records a change to a card and tells the clients about it. Installed on the
-- source table of each stack; the key column is the trigger argument.
CREATE FUNCTION xila.record_card_change() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
DECLARE
    old_row    jsonb := CASE WHEN TG_OP <> 'INSERT' THEN to_jsonb(OLD) END;
    new_row    jsonb := CASE WHEN TG_OP <> 'DELETE' THEN to_jsonb(NEW) END;
    card_key   text := coalesce(new_row, old_row)->>TG_ARGV[0];
    stack_id   uuid;
    stack_name text;
BEGIN
    IF old_row IS NOT DISTINCT FROM new_row THEN
        RETURN NULL;
    END IF;

    SELECT s.id, s.name INTO stack_id, stack_name
    FROM xila.background b JOIN xila.stack s ON s.id = b.stack_id
    WHERE b.source_schema = TG_TABLE_SCHEMA AND b.source_table = TG_TABLE_NAME;

    INSERT INTO xila.history (origin, stack_id, object_kind, object_key, op, before, after)
    VALUES (
        coalesce(nullif(current_setting('xila.origin', true), ''), 'user'),
        stack_id, 'card', card_key, lower(TG_OP), old_row, new_row);

    -- Identifiers only: any connected role may listen to this channel.
    PERFORM pg_notify('xila_card', jsonb_build_object(
        'stack', stack_name, 'key', card_key, 'op', lower(TG_OP))::text);
    RETURN NULL;
END
$$;

-- Checks the source of a background and, for a table, installs the card
-- trigger on it. Runs with the privileges of the caller, who must own the
-- table. A view is accepted as it is: its cards are read-only.
CREATE FUNCTION xila.attach_source() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    source   regclass;
    kind     "char";
    key_cols name[];
    missing  name;
BEGIN
    -- During a restore the dump brings the table's key and trigger itself.
    IF NEW.source_table IS NULL OR xila.restoring() THEN
        RETURN NEW;
    END IF;

    source := to_regclass(format('%I.%I', NEW.source_schema, NEW.source_table));
    IF source IS NULL THEN
        RAISE EXCEPTION 'The table %.% does not exist.', NEW.source_schema, NEW.source_table
            USING ERRCODE = 'XL002';
    END IF;
    SELECT c.relkind INTO kind FROM pg_class c WHERE c.oid = source;

    SELECT c INTO missing
    FROM unnest(NEW.order_columns || NEW.key_column) AS c
    WHERE NOT EXISTS (
        SELECT FROM pg_attribute a
        WHERE a.attrelid = source AND a.attname = c AND a.attnum > 0 AND NOT a.attisdropped)
    LIMIT 1;
    IF missing IS NOT NULL THEN
        RAISE EXCEPTION 'The column “%” does not exist in the table %.', missing, source
            USING ERRCODE = 'XL003';
    END IF;

    IF kind IN ('v', 'm') THEN
        IF NEW.managed THEN
            RAISE EXCEPTION 'A view cannot be managed by pgXila.' USING ERRCODE = 'XL003';
        END IF;
        RETURN NEW;
    END IF;

    SELECT array_agg(a.attname) INTO key_cols
    FROM pg_index i
    JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY (i.indkey)
    WHERE i.indrelid = source AND i.indisprimary;

    IF key_cols IS DISTINCT FROM ARRAY[NEW.key_column] THEN
        RAISE EXCEPTION 'The column “%” must be, on its own, the primary key of the table %.',
            NEW.key_column, source
            USING ERRCODE = 'XL003';
    END IF;

    EXECUTE format('DROP TRIGGER IF EXISTS xila_card ON %s', source);
    EXECUTE format(
        'CREATE TRIGGER xila_card AFTER INSERT OR UPDATE OR DELETE ON %s
             FOR EACH ROW WHEN (NOT xila.restoring())
             EXECUTE FUNCTION xila.record_card_change(%L)',
        source, NEW.key_column);
    RETURN NEW;
END
$$;

CREATE TRIGGER attach_source
    BEFORE INSERT OR UPDATE OF source_schema, source_table, key_column, order_columns
    ON xila.background
    FOR EACH ROW EXECUTE FUNCTION xila.attach_source();

-- When a background goes away, its table stops being a stack: the card
-- trigger is removed. The table and its rows stay.
CREATE FUNCTION xila.detach_source() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    source regclass;
BEGIN
    IF OLD.source_table IS NULL OR xila.restoring() THEN
        RETURN NULL;
    END IF;
    source := to_regclass(format('%I.%I', OLD.source_schema, OLD.source_table));
    IF source IS NOT NULL THEN
        BEGIN
            EXECUTE format('DROP TRIGGER IF EXISTS xila_card ON %s', source);
        EXCEPTION WHEN insufficient_privilege THEN
            NULL;  -- not our table: its owner removes the trigger
        END;
    END IF;
    RETURN NULL;
END
$$;

CREATE TRIGGER detach_source AFTER DELETE ON xila.background
    FOR EACH ROW EXECUTE FUNCTION xila.detach_source();

-- ---------------------------------------------------------------------------
-- Ownership
-- ---------------------------------------------------------------------------
-- A stack belongs to a role. Its definition may be changed by that role and
-- by its members, and by nobody else. System stacks, and the actions and keys
-- shared by every screen, can only be changed by a superuser. Row-level
-- security enforces this on the tables themselves, so it also holds for
-- someone who writes the rows in SQL. Everyone may read every definition.

-- True when the current role is, or is a member of, the given role.
CREATE FUNCTION xila.is_mine(owner name) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT FROM pg_catalog.pg_roles r
        WHERE r.rolname = owner AND pg_has_role(r.oid, 'USAGE'))
$$;

CREATE FUNCTION xila.may_edit(stack uuid) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT FROM xila.stack s
        WHERE s.id = stack AND s.kind = 'user' AND xila.is_mine(s.owner))
$$;

CREATE FUNCTION xila.may_edit_background(background uuid) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT FROM xila.background b WHERE b.id = background AND xila.may_edit(b.stack_id))
$$;

CREATE FUNCTION xila.may_edit_widget(widget uuid) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT FROM xila.widget w
        WHERE w.id = widget AND xila.may_edit_background(w.background_id))
$$;

ALTER TABLE xila.stack ENABLE ROW LEVEL SECURITY;
CREATE POLICY read ON xila.stack FOR SELECT USING (true);
CREATE POLICY edit ON xila.stack
    USING (kind = 'user' AND xila.is_mine(owner))
    WITH CHECK (kind = 'user' AND xila.is_mine(owner));

ALTER TABLE xila.stack_colour ENABLE ROW LEVEL SECURITY;
CREATE POLICY read ON xila.stack_colour FOR SELECT USING (true);
CREATE POLICY edit ON xila.stack_colour
    USING (xila.may_edit(stack_id)) WITH CHECK (xila.may_edit(stack_id));

ALTER TABLE xila.background ENABLE ROW LEVEL SECURITY;
CREATE POLICY read ON xila.background FOR SELECT USING (true);
CREATE POLICY edit ON xila.background
    USING (xila.may_edit(stack_id)) WITH CHECK (xila.may_edit(stack_id));

ALTER TABLE xila.action ENABLE ROW LEVEL SECURITY;
CREATE POLICY read ON xila.action FOR SELECT USING (true);
CREATE POLICY edit ON xila.action
    USING (xila.may_edit_background(background_id))
    WITH CHECK (xila.may_edit_background(background_id));

ALTER TABLE xila.widget ENABLE ROW LEVEL SECURITY;
CREATE POLICY read ON xila.widget FOR SELECT USING (true);
CREATE POLICY edit ON xila.widget
    USING (xila.may_edit_background(background_id))
    WITH CHECK (xila.may_edit_background(background_id));

ALTER TABLE xila.key_binding ENABLE ROW LEVEL SECURITY;
CREATE POLICY read ON xila.key_binding FOR SELECT USING (true);
-- F3, F9, F11 and F12 mean the same on every screen: a stack cannot take them.
CREATE POLICY edit ON xila.key_binding
    USING (xila.may_edit_background(background_id))
    WITH CHECK (xila.may_edit_background(background_id)
                AND key NOT IN ('F3', 'F9', 'F11', 'F12'));

-- A script runs with the rights of whoever triggers it: everyone who may open
-- the stack may read it.
ALTER TABLE xila.script ENABLE ROW LEVEL SECURITY;
CREATE POLICY read ON xila.script FOR SELECT USING (true);
CREATE POLICY edit ON xila.script
    USING (xila.may_edit_background(background_id))
    WITH CHECK (xila.may_edit_background(background_id));

ALTER TABLE xila.layout ENABLE ROW LEVEL SECURITY;
CREATE POLICY read ON xila.layout FOR SELECT USING (true);
CREATE POLICY edit ON xila.layout
    USING (xila.may_edit_widget(widget_id)) WITH CHECK (xila.may_edit_widget(widget_id));

-- ---------------------------------------------------------------------------
-- Who may read history
-- ---------------------------------------------------------------------------
-- The owner a stack had when it was deleted. SECURITY DEFINER: it is used by
-- the policy of xila.history, which could not read xila.history itself.
CREATE FUNCTION xila.last_owner(stack uuid) RETURNS name
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
    SELECT (h.before->>'owner')::name
    FROM xila.history h
    WHERE h.stack_id = stack AND h.object_kind = 'stack' AND h.op = 'delete'
    ORDER BY h.id DESC
    LIMIT 1
$$;

-- True when the current role may change the stack, or could when the stack
-- was deleted.
CREATE FUNCTION xila.may_rewind(stack uuid) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT CASE WHEN EXISTS (SELECT FROM xila.stack s WHERE s.id = stack)
                THEN xila.may_edit(stack)
                ELSE coalesce(xila.is_mine(xila.last_owner(stack)), false) END
$$;

-- A table by its names, or NULL. Unlike a lookup by name (to_regclass or a
-- cast to regclass), this never raises an error for a role that may not use
-- the schema: where pgXila asks whether someone has access, the answer to give
-- is "no", not an error.
CREATE FUNCTION xila.relation(in_schema name, named name) RETURNS oid
LANGUAGE sql STABLE AS $$
    SELECT c.oid
    FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = in_schema AND c.relname = named
$$;

-- True when the current role may read a table: use its schema, and select
-- from it.
CREATE FUNCTION xila.may_read(relation oid) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT coalesce((
        SELECT has_schema_privilege(c.relnamespace, 'USAGE')
           AND has_table_privilege(c.oid, 'SELECT')
        FROM pg_catalog.pg_class c
        WHERE c.oid = relation), false)
$$;

-- History holds old versions of cards, so reading it follows the table:
--   * changes to a definition: whoever may open the stack;
--   * changes to cards, and the values of a dropped column: whoever may read
--     the whole table. When the table uses row-level security, only those who
--     may edit the stack, so that history never shows a row the table hides.
CREATE FUNCTION xila.may_read_history(stack uuid, kind text) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT CASE
        WHEN stack IS NULL THEN true
        WHEN xila.may_rewind(stack) THEN true
        ELSE coalesce((
            SELECT CASE
                WHEN b.source_table IS NULL THEN kind NOT IN ('card', 'column')
                WHEN NOT xila.may_read(c.oid) THEN false
                ELSE kind NOT IN ('card', 'column') OR NOT c.relrowsecurity
            END
            FROM xila.background b
            LEFT JOIN pg_catalog.pg_class c
                   ON c.oid = xila.relation(b.source_schema, b.source_table)
            WHERE b.stack_id = stack), false)
    END
$$;

ALTER TABLE xila.history ENABLE ROW LEVEL SECURITY;
CREATE POLICY read ON xila.history FOR SELECT
    USING (xila.may_read_history(stack_id, object_kind));
-- The triggers write history as its owner. The author functions add two kinds
-- of rows themselves: what they did to the columns of a table, and the mark
-- that a rewind leaves.
CREATE POLICY note ON xila.history FOR INSERT
    WITH CHECK (actor = session_user
                AND CASE object_kind
                        WHEN 'column' THEN xila.may_edit(stack_id)
                        WHEN 'rewind' THEN xila.may_read_history(stack_id, 'card')
                        ELSE false
                    END);


-- ---------------------------------------------------------------------------
-- The agent
-- ---------------------------------------------------------------------------
-- See docs/api.md, section 11, and agent.sql for the functions.

-- An address the worker may call: https anywhere; plain http for this machine
-- only, which is what a model that runs locally needs. Nothing a person asks
-- and no token ever crosses a network in clear.
CREATE FUNCTION xila.agent_address_ok(address text) RETURNS boolean
LANGUAGE sql IMMUTABLE AS $$
    SELECT address ~ '^https://[^\s/@]+(/\S*)?$'
        OR address ~ '^http://(localhost|127\.0\.0\.1|\[::1\]|host\.docker\.internal)(:[0-9]+)?(/\S*)?$'
$$;

-- The name of an environment variable that may hold the token: never one of
-- PostgreSQL itself, such as PGPASSWORD, PG_COLOR or POSTGRES_PASSWORD. A
-- name like PGXILA_AGENT_TOKEN is not one of those.
CREATE FUNCTION xila.agent_variable_ok(variable text) RETURNS boolean
LANGUAGE sql IMMUTABLE AS $$
    SELECT variable ~ '^([A-Z][A-Z0-9_]{0,63})?$'
       AND variable !~ '^(PG[A-Z0-9]*|PG_.*|POSTGRES.*)$'
$$;

-- The settings of the agent, in one row. The token of the model is not here:
-- token_variable names the environment variable of the server that holds it.
CREATE TABLE xila.agent_setting (
    only_row       boolean PRIMARY KEY DEFAULT true CHECK (only_row),
    address        text NOT NULL DEFAULT ''
                   CHECK (address = '' OR xila.agent_address_ok(address)),
    model          text NOT NULL DEFAULT '',
    token_variable text NOT NULL DEFAULT '' CHECK (xila.agent_variable_ok(token_variable)),
    options        jsonb NOT NULL DEFAULT '{}' CHECK (jsonb_typeof(options) = 'object'),
    instructions   text NOT NULL DEFAULT '',
    changed_at     timestamptz,
    changed_by     name
);

-- Restoring a dump loads the saved settings on top of the empty ones.
CREATE FUNCTION xila.agent_setting_upsert() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    UPDATE xila.agent_setting
    SET address = NEW.address, model = NEW.model, token_variable = NEW.token_variable,
        options = NEW.options, instructions = NEW.instructions,
        changed_at = NEW.changed_at, changed_by = NEW.changed_by;
    IF FOUND THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER agent_setting_upsert BEFORE INSERT ON xila.agent_setting
    FOR EACH ROW EXECUTE FUNCTION xila.agent_setting_upsert();

INSERT INTO xila.agent_setting DEFAULT VALUES;

-- A tool of the agent: a SQL function f(as_role name, arguments jsonb)
-- RETURNS jsonb. The model reads the name, the description and the
-- parameters; people read the label.
CREATE TABLE xila.agent_tool (
    name        text PRIMARY KEY CHECK (name ~ '^[a-z][a-z0-9_]{0,40}$'),
    ord         integer NOT NULL DEFAULT 0,
    label       text NOT NULL,
    description text NOT NULL,
    parameters  jsonb NOT NULL DEFAULT '{"type": "object", "properties": {}}',
    function    regproc NOT NULL
);

-- A conversation between someone and the agent. stack and card say what its
-- owner was looking at when calling the agent.
CREATE TABLE xila.conversation (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    owner      name NOT NULL DEFAULT current_user,
    title      text NOT NULL DEFAULT '',
    state      text NOT NULL DEFAULT 'idle' CHECK (state IN ('idle', 'waiting')),
    stack      text,
    card       text,
    started_at timestamptz NOT NULL DEFAULT now(),
    changed_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX conversation_owner ON xila.conversation (owner, changed_at);
CREATE INDEX conversation_waiting ON xila.conversation (changed_at) WHERE state = 'waiting';

-- A line of a conversation. text is what people read. payload is what the
-- model is sent back: its own message, or the result of a tool.
CREATE TABLE xila.agent_message (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    conversation_id uuid NOT NULL REFERENCES xila.conversation ON DELETE CASCADE,
    at              timestamptz NOT NULL DEFAULT clock_timestamp(),
    kind            text NOT NULL CHECK (kind IN ('user', 'agent', 'tool', 'error')),
    text            text NOT NULL DEFAULT '',
    payload         jsonb
);

CREATE INDEX agent_message_order ON xila.agent_message (conversation_id, at, id);

-- The journal of the agent: every call to the model and every tool it ran,
-- for whom. There is deliberately no foreign key: the journal outlives the
-- conversations. It does not copy what the cards say.
CREATE TABLE xila.agent_log (
    id              uuid PRIMARY KEY DEFAULT uuidv7(),
    at              timestamptz NOT NULL DEFAULT clock_timestamp(),
    conversation_id uuid,
    role            name NOT NULL,
    event           text NOT NULL CHECK (event IN ('model', 'tool', 'refused', 'failed')),
    tool            text,
    summary         text NOT NULL,
    detail          jsonb NOT NULL DEFAULT '{}',
    ms              integer,
    tokens_in       integer,
    tokens_out      integer
);

CREATE INDEX agent_log_role ON xila.agent_log (role, at);

-- A conversation is read by its owner alone, and by the worker. People add
-- their own questions and nothing else; only the worker writes what the
-- agent says and what it did.
ALTER TABLE xila.conversation ENABLE ROW LEVEL SECURITY;
CREATE POLICY own ON xila.conversation TO xila_user
    USING (owner = current_user) WITH CHECK (owner = current_user);
CREATE POLICY worker ON xila.conversation TO xila_agent USING (true) WITH CHECK (true);

ALTER TABLE xila.agent_message ENABLE ROW LEVEL SECURITY;
CREATE POLICY own_read ON xila.agent_message FOR SELECT TO xila_user
    USING (EXISTS (SELECT FROM xila.conversation c WHERE c.id = conversation_id));
CREATE POLICY own_ask ON xila.agent_message FOR INSERT TO xila_user
    WITH CHECK (kind = 'user' AND payload IS NULL
                AND EXISTS (SELECT FROM xila.conversation c WHERE c.id = conversation_id));
CREATE POLICY worker ON xila.agent_message TO xila_agent USING (true) WITH CHECK (true);

ALTER TABLE xila.agent_log ENABLE ROW LEVEL SECURITY;
CREATE POLICY own_read ON xila.agent_log FOR SELECT TO xila_user
    USING (role = current_user OR pg_has_role('xila_admin', 'USAGE'));
CREATE POLICY worker_read ON xila.agent_log FOR SELECT TO xila_agent USING (true);
CREATE POLICY worker_write ON xila.agent_log FOR INSERT TO xila_agent WITH CHECK (true);

-- ---------------------------------------------------------------------------
-- Privileges
-- ---------------------------------------------------------------------------
-- Users read the definitions through the API functions, which run with their
-- own privileges. Authors write them, within the ownership rules above.
-- History is read and written within its own policies.
GRANT USAGE ON SCHEMA xila TO xila_user;
GRANT SELECT ON xila.palette, xila.image, xila.stack, xila.stack_colour, xila.background,
    xila.action, xila.widget, xila.layout, xila.key_binding, xila.script TO xila_user;
GRANT INSERT, UPDATE, DELETE ON xila.stack, xila.stack_colour, xila.background,
    xila.action, xila.widget, xila.layout, xila.key_binding, xila.script TO xila_author;
-- Images are added by anyone who uses pgXila, and changed or deleted by
-- nobody but a superuser. An author may make a column refer to an image.
GRANT INSERT ON xila.image TO xila_user;
GRANT REFERENCES ON xila.image TO xila_author;
GRANT UPDATE ON xila.palette TO xila_author;
GRANT SELECT ON xila.history TO xila_user;
GRANT INSERT (origin, stack_id, object_kind, object_key, op, before, after)
    ON xila.history TO xila_user;

-- The agent. People hold their conversations and ask; they read the journal
-- within its policy and cannot write it. Administrators change the settings.
GRANT SELECT ON xila.agent_tool, xila.conversation, xila.agent_message, xila.agent_log
    TO xila_user;
GRANT INSERT (stack, card) ON xila.conversation TO xila_user;
GRANT UPDATE (title, state, stack, card, changed_at) ON xila.conversation TO xila_user;
GRANT INSERT (conversation_id, kind, text) ON xila.agent_message TO xila_user;
GRANT SELECT, UPDATE ON xila.agent_setting TO xila_admin;

-- The worker reads the definitions of the stacks, holds the conversations and
-- writes the journal. It reads the cards of a stack only once the table of
-- that stack was opened to it. It can write nowhere else.
GRANT USAGE ON SCHEMA xila TO xila_agent;
GRANT SELECT ON xila.stack, xila.background, xila.widget, xila.agent_setting, xila.agent_tool
    TO xila_agent;
GRANT SELECT, UPDATE ON xila.conversation TO xila_agent;
GRANT SELECT, INSERT ON xila.agent_message TO xila_agent;
GRANT SELECT, INSERT ON xila.agent_log TO xila_agent;

-- ---------------------------------------------------------------------------
-- Backups
-- ---------------------------------------------------------------------------
-- These tables hold user data, so pg_dump must save their contents. The
-- system stacks are created by the extension itself and are left out.
SELECT pg_catalog.pg_extension_config_dump('xila.palette', '');
SELECT pg_catalog.pg_extension_config_dump('xila.image', '');
SELECT pg_catalog.pg_extension_config_dump('xila.history', '');
SELECT pg_catalog.pg_extension_config_dump('xila.history_id_seq', '');
SELECT pg_catalog.pg_extension_config_dump('xila.stack', $$WHERE kind = 'user'$$);
SELECT pg_catalog.pg_extension_config_dump('xila.stack_colour', '');
SELECT pg_catalog.pg_extension_config_dump('xila.background',
    $$WHERE stack_id IN (SELECT id FROM xila.stack WHERE kind = 'user')$$);
SELECT pg_catalog.pg_extension_config_dump('xila.action',
    $$WHERE background_id IN (SELECT b.id FROM xila.background b
        JOIN xila.stack s ON s.id = b.stack_id WHERE s.kind = 'user')$$);
SELECT pg_catalog.pg_extension_config_dump('xila.widget',
    $$WHERE background_id IN (SELECT b.id FROM xila.background b
        JOIN xila.stack s ON s.id = b.stack_id WHERE s.kind = 'user')$$);
SELECT pg_catalog.pg_extension_config_dump('xila.layout',
    $$WHERE widget_id IN (SELECT w.id FROM xila.widget w
        JOIN xila.background b ON b.id = w.background_id
        JOIN xila.stack s ON s.id = b.stack_id WHERE s.kind = 'user')$$);
SELECT pg_catalog.pg_extension_config_dump('xila.key_binding',
    $$WHERE background_id IN (SELECT b.id FROM xila.background b
        JOIN xila.stack s ON s.id = b.stack_id WHERE s.kind = 'user')$$);
SELECT pg_catalog.pg_extension_config_dump('xila.agent_setting', '');
SELECT pg_catalog.pg_extension_config_dump('xila.conversation', '');
SELECT pg_catalog.pg_extension_config_dump('xila.agent_message', '');
SELECT pg_catalog.pg_extension_config_dump('xila.agent_log', '');
SELECT pg_catalog.pg_extension_config_dump('xila.script',
    $$WHERE background_id IN (SELECT b.id FROM xila.background b
        JOIN xila.stack s ON s.id = b.stack_id WHERE s.kind = 'user')$$);
