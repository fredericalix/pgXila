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
    ( 0, 'noir',      '#14121f'), ( 1, 'nuit',      '#232a4d'),
    ( 2, 'marine',    '#2f4a8a'), ( 3, 'bleu',      '#3f73c4'),
    ( 4, 'azur',      '#6fb0e8'), ( 5, 'glacier',   '#bfe3f5'),
    ( 6, 'crème',     '#f6ecd2'), ( 7, 'blanc',     '#fffdf5'),
    ( 8, 'sable',     '#e3cf9f'), ( 9, 'ocre',      '#c2a06a'),
    (10, 'brun',      '#8a6240'), (11, 'chocolat',  '#573a2e'),
    (12, 'ardoise',   '#3d3f52'), (13, 'gris',      '#6c7086'),
    (14, 'argent',    '#a4a8b8'), (15, 'perle',     '#d6d8e0'),
    (16, 'sapin',     '#1f5a48'), (17, 'vert',      '#2f9a5c'),
    (18, 'prairie',   '#7fd06a'), (19, 'tilleul',   '#c9f0a8'),
    (20, 'canard',    '#1f8a8f'), (21, 'turquoise', '#4fd0c2'),
    (22, 'jaune',     '#f7d046'), (23, 'ambre',     '#f0a030'),
    (24, 'orange',    '#e8703a'), (25, 'rouge',     '#d0414a'),
    (26, 'bordeaux',  '#8a2846'), (27, 'rose',      '#f08aa0'),
    (28, 'pêche',     '#f8c8a8'), (29, 'violet',    '#6a4aa8'),
    (30, 'lavande',   '#a88ae0'), (31, 'prune',     '#3c2452');

-- ---------------------------------------------------------------------------
-- Images
-- ---------------------------------------------------------------------------
-- An indexed image: one byte per pixel, row by row, each byte a palette slot.
CREATE TABLE xila.image (
    id     uuid PRIMARY KEY DEFAULT uuidv7(),
    width  smallint NOT NULL CHECK (width BETWEEN 1 AND 480),
    height smallint NOT NULL CHECK (height BETWEEN 1 AND 300),
    pixels bytea NOT NULL,
    CHECK (octet_length(pixels) = width::integer * height::integer)
);

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
    created_at timestamptz NOT NULL DEFAULT now()
);

-- Extra colours of a stack, beyond the 32 system colours.
CREATE TABLE xila.stack_colour (
    stack_id uuid NOT NULL REFERENCES xila.stack ON DELETE CASCADE,
    slot     smallint NOT NULL CHECK (slot BETWEEN 32 AND 255),
    name     text,
    rgb      text NOT NULL CHECK (rgb ~ '^#[0-9a-f]{6}$'),
    PRIMARY KEY (stack_id, slot)
);

-- The screen shared by all the cards of a stack. The cards are the rows of
-- the source table. Without a source table, the background is a static screen
-- with a single card.
CREATE TABLE xila.background (
    id            uuid PRIMARY KEY DEFAULT uuidv7(),
    stack_id      uuid NOT NULL UNIQUE REFERENCES xila.stack ON DELETE CASCADE,
    source_schema name,
    source_table  name,
    key_column    name,
    order_columns name[] NOT NULL DEFAULT '{}',
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
                      'last_card', 'save_card', 'new_card', 'back', 'quit')),
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

-- Finds the stack a definition row belongs to.
CREATE FUNCTION xila.stack_of(kind text, r jsonb) RETURNS uuid
LANGUAGE sql STABLE AS $$
    SELECT CASE kind
        WHEN 'stack' THEN (r->>'id')::uuid
        WHEN 'background' THEN (r->>'stack_id')::uuid
        WHEN 'stack_colour' THEN (r->>'stack_id')::uuid
        WHEN 'layout' THEN (
            SELECT b.stack_id
            FROM xila.widget w JOIN xila.background b ON b.id = w.background_id
            WHERE w.id = (r->>'widget_id')::uuid)
        ELSE (SELECT b.stack_id FROM xila.background b WHERE b.id = (r->>'background_id')::uuid)
    END
$$;

-- Records a change to a definition table and tells the clients about it.
-- SECURITY DEFINER: the history table is not writable by the roles that make
-- the changes.
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

CREATE TRIGGER history AFTER INSERT OR UPDATE OR DELETE ON xila.palette
    FOR EACH ROW EXECUTE FUNCTION xila.record_definition_change();
CREATE TRIGGER history AFTER INSERT OR UPDATE OR DELETE ON xila.stack
    FOR EACH ROW EXECUTE FUNCTION xila.record_definition_change();
CREATE TRIGGER history AFTER INSERT OR UPDATE OR DELETE ON xila.stack_colour
    FOR EACH ROW EXECUTE FUNCTION xila.record_definition_change();
CREATE TRIGGER history AFTER INSERT OR UPDATE OR DELETE ON xila.background
    FOR EACH ROW EXECUTE FUNCTION xila.record_definition_change();
CREATE TRIGGER history AFTER INSERT OR UPDATE OR DELETE ON xila.action
    FOR EACH ROW EXECUTE FUNCTION xila.record_definition_change();
CREATE TRIGGER history AFTER INSERT OR UPDATE OR DELETE ON xila.widget
    FOR EACH ROW EXECUTE FUNCTION xila.record_definition_change();
CREATE TRIGGER history AFTER INSERT OR UPDATE OR DELETE ON xila.layout
    FOR EACH ROW EXECUTE FUNCTION xila.record_definition_change();
CREATE TRIGGER history AFTER INSERT OR UPDATE OR DELETE ON xila.key_binding
    FOR EACH ROW EXECUTE FUNCTION xila.record_definition_change();

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

-- Checks the source table of a background and installs the card trigger on
-- it. Runs with the privileges of the caller, who must own the table.
CREATE FUNCTION xila.attach_source() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    source   regclass;
    key_cols name[];
    missing  name;
BEGIN
    IF NEW.source_table IS NULL THEN
        RETURN NEW;
    END IF;

    source := to_regclass(format('%I.%I', NEW.source_schema, NEW.source_table));
    IF source IS NULL THEN
        RAISE EXCEPTION 'La table %.% n''existe pas.', NEW.source_schema, NEW.source_table
            USING ERRCODE = 'XL002';
    END IF;

    SELECT array_agg(a.attname) INTO key_cols
    FROM pg_index i
    JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY (i.indkey)
    WHERE i.indrelid = source AND i.indisprimary;

    IF key_cols IS DISTINCT FROM ARRAY[NEW.key_column] THEN
        RAISE EXCEPTION 'La colonne « % » doit être, à elle seule, la clé primaire de la table %.',
            NEW.key_column, source
            USING ERRCODE = 'XL003';
    END IF;

    SELECT c INTO missing
    FROM unnest(NEW.order_columns) AS c
    WHERE NOT EXISTS (
        SELECT FROM pg_attribute a
        WHERE a.attrelid = source AND a.attname = c AND a.attnum > 0 AND NOT a.attisdropped)
    LIMIT 1;
    IF missing IS NOT NULL THEN
        RAISE EXCEPTION 'La colonne de tri « % » n''existe pas dans la table %.', missing, source
            USING ERRCODE = 'XL003';
    END IF;

    EXECUTE format('DROP TRIGGER IF EXISTS xila_card ON %s', source);
    EXECUTE format(
        'CREATE TRIGGER xila_card AFTER INSERT OR UPDATE OR DELETE ON %s
             FOR EACH ROW EXECUTE FUNCTION xila.record_card_change(%L)',
        source, NEW.key_column);
    RETURN NEW;
END
$$;

CREATE TRIGGER attach_source
    BEFORE INSERT OR UPDATE OF source_schema, source_table, key_column, order_columns
    ON xila.background
    FOR EACH ROW EXECUTE FUNCTION xila.attach_source();

-- ---------------------------------------------------------------------------
-- Privileges
-- ---------------------------------------------------------------------------
-- Users read the definitions through the API functions, which run with their
-- own privileges. Authors write them. Nobody writes history directly.
GRANT USAGE ON SCHEMA xila TO xila_user;
GRANT SELECT ON xila.palette, xila.image, xila.stack, xila.stack_colour, xila.background,
    xila.action, xila.widget, xila.layout, xila.key_binding TO xila_user;
GRANT INSERT, UPDATE, DELETE ON xila.image, xila.stack, xila.stack_colour, xila.background,
    xila.action, xila.widget, xila.layout, xila.key_binding TO xila_author;
GRANT UPDATE ON xila.palette TO xila_author;

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
