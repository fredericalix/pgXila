-- pgXila author functions: building and changing a stack, and rewinding.
-- See docs/api.md, section 10.
--
-- Like the client API, these functions run with the privileges of the caller.
-- What an author may change is decided by the ownership policies of the
-- definition tables and by PostgreSQL's own rules on the stack's table; the
-- checks made here only exist to answer with a clear message.
--
-- pgXila only alters a table it has created (xila.background.managed).

-- ---------------------------------------------------------------------------
-- Names and labels
-- ---------------------------------------------------------------------------

-- An identifier made from what a person typed: "Last name" gives last_name.
CREATE FUNCTION xila.identifier_from(label text) RETURNS text
LANGUAGE sql IMMUTABLE AS $$
    SELECT left(
        regexp_replace(
            btrim(regexp_replace(
                lower(translate(label,
                    'ÀÁÂÃÄÅàáâãäåÇçÈÉÊËèéêëÌÍÎÏìíîïÑñÒÓÔÕÖòóôõöÙÚÛÜùúûüÝýÿ',
                    'AAAAAAaaaaaaCcEEEEeeeeIIIIiiiiNnOOOOOoooooUUUUuuuuYyy')),
                '[^a-z0-9]+', '_', 'g'), '_'),
            '^([0-9])', 'n\1'),
        31)
$$;

-- A label made from the name of a column: last_name gives "Last name".
CREATE FUNCTION xila.label_from(col text) RETURNS text
LANGUAGE sql IMMUTABLE AS $$
    SELECT upper(left(col, 1)) || substr(replace(col, '_', ' '), 2)
$$;

-- Raises an error unless the text can name a stack or a field.
CREATE FUNCTION xila.check_identifier(ident text) RETURNS void
LANGUAGE plpgsql IMMUTABLE AS $$
BEGIN
    IF ident IS NULL OR ident !~ '^[a-z][a-z0-9_]{0,30}$' THEN
        RAISE EXCEPTION
            'The name “%” cannot be used: a name starts with a letter and holds only lowercase letters, digits and “_”.',
            coalesce(ident, '')
            USING ERRCODE = 'XL003';
    END IF;
END
$$;

-- The background of a stack the current role may change, or an XL004 error.
CREATE FUNCTION xila.editable(stack text) RETURNS xila.background
LANGUAGE plpgsql STABLE AS $$
DECLARE
    bg   xila.background := xila.background_of(stack);
    info xila.stack;
BEGIN
    SELECT * INTO info FROM xila.stack WHERE id = bg.stack_id;
    IF info.kind <> 'user' THEN
        RAISE EXCEPTION 'The stack “%” is part of pgXila and cannot be changed.', info.title
            USING ERRCODE = 'XL004';
    END IF;
    IF NOT xila.is_mine(info.owner) THEN
        RAISE EXCEPTION 'The stack “%” belongs to %: you cannot change it.', info.title, info.owner
            USING ERRCODE = 'XL004';
    END IF;
    RETURN bg;
END
$$;

-- The template written at the top of each card of a background.
CREATE FUNCTION xila.card_title_template(background uuid) RETURNS text
LANGUAGE sql STABLE AS $$
    SELECT w.props->>'template'
    FROM xila.widget w
    WHERE w.background_id = background AND w.layer = 'background'
      AND w.kind = 'text' AND w.props->>'style' = 'title'
    ORDER BY w.ord, w.name
    LIMIT 1
$$;

-- ---------------------------------------------------------------------------
-- Stacks
-- ---------------------------------------------------------------------------

-- Creates the schema a new stack keeps its cards in, owned by the caller.
-- SECURITY DEFINER, so that authors need no right to create schemas in the
-- database. It does this one thing, for authors only.
CREATE FUNCTION xila.make_schema(name text) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
BEGIN
    IF NOT pg_has_role(session_user, 'xila_author', 'USAGE') THEN
        RAISE EXCEPTION 'Only authors can create stacks.' USING ERRCODE = 'XL004';
    END IF;
    IF name IS NULL OR name !~ '^[a-z][a-z0-9_]{0,30}$' OR name ~ '^pg_'
       OR name IN ('xila', 'public', 'information_schema') THEN
        RAISE EXCEPTION 'The name “%” cannot be used for a stack.', coalesce(name, '')
            USING ERRCODE = 'XL003';
    END IF;
    IF EXISTS (SELECT FROM pg_namespace n WHERE n.nspname = name) THEN
        RAISE EXCEPTION 'The name “%” is already used in this database.', name
            USING ERRCODE = 'XL003';
    END IF;
    EXECUTE format('CREATE SCHEMA %I AUTHORIZATION %I', name, session_user);
END
$$;

REVOKE ALL ON FUNCTION xila.make_schema(text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION xila.make_schema(text) TO xila_author;

-- Creates a stack and answers with its name.
--
-- Without a source, the stack gets a schema and a table of its own, which
-- pgXila manages, and an empty screen. With a source, the stack shows a table
-- that already exists and that the caller owns: one field per column. pgXila
-- never alters such a table.
CREATE FUNCTION xila.create_stack(title text, name text DEFAULT NULL,
                                  source regclass DEFAULT NULL)
RETURNS text
LANGUAGE plpgsql AS $$
DECLARE
    shown      text := btrim(coalesce(title, ''));
    stack_name text := coalesce(nullif(btrim(name), ''), xila.identifier_from(title));
    src_schema name;
    src_table  name;
    key_col    name;
    new_stack  uuid;
    bg_id      uuid;
    col        record;
    place      integer := 0;
    heading    text := '';
    is_picture boolean;
BEGIN
    IF NOT pg_has_role('xila_author', 'USAGE') THEN
        RAISE EXCEPTION 'Only authors can create stacks.' USING ERRCODE = 'XL004';
    END IF;
    IF shown = '' THEN
        RAISE EXCEPTION 'A stack needs a title.' USING ERRCODE = 'XL003', COLUMN = 'title';
    END IF;
    PERFORM xila.check_identifier(stack_name);
    IF EXISTS (SELECT FROM xila.stack s WHERE s.name = stack_name) THEN
        RAISE EXCEPTION 'A stack named “%” already exists.', stack_name USING ERRCODE = 'XL003';
    END IF;

    IF source IS NULL THEN
        PERFORM xila.make_schema(stack_name);
        EXECUTE format('CREATE TABLE %I.cards (id uuid PRIMARY KEY DEFAULT uuidv7())', stack_name);
        src_schema := stack_name;
        src_table := 'cards';
        key_col := 'id';
    ELSE
        SELECT n.nspname, c.relname INTO src_schema, src_table
        FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE c.oid = source AND c.relkind IN ('r', 'p');
        IF NOT FOUND THEN
            RAISE EXCEPTION '% is not a table.', source USING ERRCODE = 'XL003';
        END IF;
        IF NOT pg_has_role((SELECT c.relowner FROM pg_class c WHERE c.oid = source), 'USAGE') THEN
            RAISE EXCEPTION 'The table % is not yours.', source USING ERRCODE = 'XL004';
        END IF;
        IF EXISTS (SELECT FROM xila.background b
                   WHERE b.source_schema = src_schema AND b.source_table = src_table) THEN
            RAISE EXCEPTION 'The table % already belongs to a stack.', source
                USING ERRCODE = 'XL003';
        END IF;
        SELECT a.attname INTO key_col
        FROM pg_index i
        JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = i.indkey[0]
        WHERE i.indrelid = source AND i.indisprimary AND i.indnkeyatts = 1;
        IF key_col IS NULL THEN
            RAISE EXCEPTION 'The table % needs a primary key made of a single column.', source
                USING ERRCODE = 'XL003';
        END IF;
    END IF;

    INSERT INTO xila.stack (name, title) VALUES (stack_name, shown) RETURNING id INTO new_stack;
    INSERT INTO xila.background (stack_id, source_schema, source_table, key_column, managed)
    VALUES (new_stack, src_schema, src_table, key_col, source IS NULL)
    RETURNING id INTO bg_id;

    INSERT INTO xila.action (background_id, name, label, handler, args) VALUES
        (bg_id, 'previous', 'Prev.',   'previous_card', '{}'),
        (bg_id, 'next',     'Next',    'next_card',     '{}'),
        (bg_id, 'new',      'New',     'new_card',      '{}'),
        (bg_id, 'save',     'Save',    'save_card',     '{}'),
        (bg_id, 'delete',   'Delete',  'delete_card',   '{}'),
        (bg_id, 'history',  'History', 'history',       '{"of": "cards"}');

    INSERT INTO xila.key_binding (background_id, key, action_id)
    SELECT bg_id, k.key, a.id
    FROM (VALUES ('F4', 'delete'), ('F6', 'new'), ('F7', 'previous'), ('F8', 'next'),
                 ('F10', 'history'), ('Enter', 'save')) AS k(key, action)
    JOIN xila.action a ON a.background_id = bg_id AND a.name = k.action;

    -- One field per column of an existing table. A column that points to a
    -- picture becomes a picture.
    IF source IS NOT NULL THEN
        FOR col IN
            SELECT a.attname, a.attnum
            FROM pg_attribute a
            WHERE a.attrelid = source AND a.attnum > 0 AND NOT a.attisdropped
              AND a.attname <> key_col AND a.attgenerated = '' AND a.attidentity = ''
            ORDER BY a.attnum
        LOOP
            place := place + 10;
            is_picture := EXISTS (
                SELECT FROM pg_constraint c
                WHERE c.conrelid = source AND c.contype = 'f'
                  AND c.confrelid = 'xila.image'::regclass AND c.conkey = ARRAY[col.attnum]);
            INSERT INTO xila.widget (background_id, name, kind, ord, label, bound_column)
            VALUES (bg_id, col.attname,
                    CASE WHEN is_picture THEN 'image' ELSE 'field' END, place,
                    CASE WHEN NOT is_picture THEN xila.label_from(col.attname) END, col.attname);
            IF heading = '' AND NOT is_picture THEN
                heading := '{' || col.attname || '}';
            END IF;
        END LOOP;
    END IF;

    -- The title of the card and the browsing buttons.
    INSERT INTO xila.widget (background_id, name, kind, ord, label, action_id, props)
    SELECT bg_id, v.name, v.kind, v.ord, v.label,
           (SELECT a.id FROM xila.action a
            WHERE a.background_id = bg_id AND a.name = v.action),
           v.props
    FROM (VALUES
        ('heading',  'text',   0,    NULL,     NULL,
         jsonb_build_object('template', heading, 'style', 'title')),
        ('previous', 'button', 9010, 'Prev.',  'previous', '{}'::jsonb),
        ('next',     'button', 9020, 'Next',   'next',     '{}'::jsonb),
        ('new',      'button', 9030, 'New',    'new',      '{}'::jsonb),
        ('delete',   'button', 9040, 'Delete', 'delete',   '{}'::jsonb)
    ) AS v(name, kind, ord, label, action, props)
    WHERE NOT EXISTS (SELECT FROM xila.widget x
                      WHERE x.background_id = bg_id AND x.name = v.name);

    -- How they look. In text mode the header of the screen already carries the
    -- title. This is a statement of its own: the policy of xila.layout must
    -- see the widgets, which it would not within the statement that adds them.
    INSERT INTO xila.layout (widget_id, renderer, style)
    SELECT w.id, s.renderer, s.style
    FROM xila.widget w JOIN (VALUES
        ('heading',  'text',   'text',    '{"hidden": true}'::jsonb),
        ('previous', 'button', 'graphic', '{"arrow": "left"}'::jsonb),
        ('next',     'button', 'graphic', '{"arrow": "right"}'::jsonb),
        ('new',      'button', 'graphic', '{"variant": "primary"}'::jsonb)
    ) AS s(name, kind, renderer, style) ON s.name = w.name AND s.kind = w.kind
    WHERE w.background_id = bg_id;

    RETURN stack_name;
END
$$;

-- Changes the title of a stack, or the template written at the top of each
-- card: {"title": "…", "card_title": "{last_name}, {first_name}"}.
CREATE FUNCTION xila.set_stack(stack text, changes jsonb) RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    bg      xila.background := xila.editable(stack);
    unknown text;
    heading uuid;
BEGIN
    SELECT k INTO unknown FROM jsonb_object_keys(changes) k
    WHERE k NOT IN ('title', 'card_title') LIMIT 1;
    IF unknown IS NOT NULL THEN
        RAISE EXCEPTION 'A stack has no property “%”.', unknown USING ERRCODE = 'XL003';
    END IF;

    IF changes ? 'title' THEN
        IF btrim(coalesce(changes->>'title', '')) = '' THEN
            RAISE EXCEPTION 'A stack needs a title.' USING ERRCODE = 'XL003', COLUMN = 'title';
        END IF;
        UPDATE xila.stack SET title = btrim(changes->>'title') WHERE id = bg.stack_id;
    END IF;

    IF changes ? 'card_title' THEN
        UPDATE xila.widget w
        SET props = w.props || jsonb_build_object('template', coalesce(changes->>'card_title', ''))
        WHERE w.id = (
            SELECT x.id FROM xila.widget x
            WHERE x.background_id = bg.id AND x.layer = 'background'
              AND x.kind = 'text' AND x.props->>'style' = 'title'
            ORDER BY x.ord, x.name LIMIT 1);
        IF NOT FOUND THEN
            INSERT INTO xila.widget (background_id, name, kind, ord, props)
            VALUES (bg.id, 'heading', 'text', 0,
                    jsonb_build_object('template', coalesce(changes->>'card_title', ''),
                                       'style', 'title'))
            RETURNING id INTO heading;
            INSERT INTO xila.layout (widget_id, renderer, style)
            VALUES (heading, 'text', '{"hidden": true}');
        END IF;
    END IF;
END
$$;

-- Removes a stack and its screens. The table and its cards are left in place;
-- rewinding brings the stack back.
CREATE FUNCTION xila.delete_stack(stack text) RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    bg xila.background := xila.editable(stack);
BEGIN
    DELETE FROM xila.stack WHERE id = bg.stack_id;
END
$$;

-- ---------------------------------------------------------------------------
-- Columns of a managed table
-- ---------------------------------------------------------------------------
-- Each of these alters the table and records what it did as a history row of
-- kind 'column', so that it can be rewound. They are the only place where
-- pgXila alters a table.

CREATE FUNCTION xila.managed_source(bg xila.background) RETURNS regclass
LANGUAGE plpgsql STABLE AS $$
DECLARE
    source regclass := xila.source_of(bg);
BEGIN
    IF source IS NULL OR NOT bg.managed THEN
        RAISE EXCEPTION
            'The table of this stack was not created by pgXila, which never alters it. Change its columns in SQL.'
            USING ERRCODE = 'XL003';
    END IF;
    RETURN source;
END
$$;

CREATE FUNCTION xila.note_column(bg xila.background, col text, change text,
                                 was jsonb, becomes jsonb)
RETURNS void
LANGUAGE sql AS $$
    INSERT INTO xila.history (origin, stack_id, object_kind, object_key, op, before, after)
    VALUES (coalesce(nullif(current_setting('xila.origin', true), ''), 'user'),
            (bg).stack_id, 'column', col, change, was, becomes)
$$;

-- Adds a column. vals, if given, holds the value of the column for some
-- cards, by key: this is how a dropped column comes back.
CREATE FUNCTION xila.column_add(bg xila.background, col text, sql_type text,
                                required boolean DEFAULT false, vals jsonb DEFAULT NULL)
RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    source  regclass := xila.managed_source(bg);
    checked regtype := to_regtype(sql_type);
BEGIN
    PERFORM xila.check_identifier(col);
    IF checked IS NULL THEN
        RAISE EXCEPTION 'Unknown type: %.', sql_type USING ERRCODE = 'XL003';
    END IF;
    EXECUTE format('ALTER TABLE %s ADD COLUMN %I %s', source, col, checked);
    IF vals IS NOT NULL AND vals <> '{}' THEN
        EXECUTE format(
            'UPDATE %1$s AS t SET %2$I = CAST(v.value AS %3$s)
             FROM jsonb_each_text($1) AS v WHERE t.%4$I::text = v.key',
            source, col, checked, bg.key_column)
        USING vals;
    END IF;
    IF required THEN
        EXECUTE format('ALTER TABLE %s ALTER COLUMN %I SET NOT NULL', source, col);
    END IF;
    PERFORM xila.note_column(bg, col, 'insert', NULL,
        jsonb_build_object('name', col, 'type', checked::text, 'required', required));
END
$$;

-- Drops a column. Its values go to history, so that a rewind restores them.
CREATE FUNCTION xila.column_drop(bg xila.background, col text) RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    source   regclass := xila.managed_source(bg);
    sql_type text;
    required boolean;
    vals     jsonb;
BEGIN
    SELECT format_type(a.atttypid, a.atttypmod), a.attnotnull INTO sql_type, required
    FROM pg_attribute a
    WHERE a.attrelid = source AND a.attname = col AND a.attnum > 0 AND NOT a.attisdropped;
    IF NOT FOUND OR col = bg.key_column THEN
        RAISE EXCEPTION 'The column “%” cannot be removed.', col USING ERRCODE = 'XL003';
    END IF;
    EXECUTE format(
        'SELECT coalesce(jsonb_object_agg(t.%1$I::text, t.%2$I::text), ''{}'')
         FROM %3$s AS t WHERE t.%2$I IS NOT NULL',
        bg.key_column, col, source)
    INTO vals;
    EXECUTE format('ALTER TABLE %s DROP COLUMN %I', source, col);
    PERFORM xila.note_column(bg, col, 'delete',
        jsonb_build_object('name', col, 'type', sql_type, 'required', required, 'values', vals),
        NULL);
END
$$;

CREATE FUNCTION xila.column_rename(bg xila.background, col text, new_name text) RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    source regclass := xila.managed_source(bg);
BEGIN
    PERFORM xila.check_identifier(new_name);
    IF col = bg.key_column THEN
        RAISE EXCEPTION 'The column “%” cannot be renamed.', col USING ERRCODE = 'XL003';
    END IF;
    EXECUTE format('ALTER TABLE %s RENAME COLUMN %I TO %I', source, col, new_name);
    PERFORM xila.note_column(bg, new_name, 'update',
        jsonb_build_object('name', col), jsonb_build_object('name', new_name));
END
$$;

CREATE FUNCTION xila.column_required(bg xila.background, col text, required boolean)
RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    source regclass := xila.managed_source(bg);
    was    boolean;
    empty  bigint;
BEGIN
    SELECT a.attnotnull INTO was
    FROM pg_attribute a
    WHERE a.attrelid = source AND a.attname = col AND a.attnum > 0 AND NOT a.attisdropped;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'The column “%” does not exist.', col USING ERRCODE = 'XL002';
    END IF;
    IF was = required THEN
        RETURN;
    END IF;
    IF required THEN
        EXECUTE format('SELECT count(*) FROM %s WHERE %I IS NULL', source, col) INTO empty;
        IF empty > 0 THEN
            RAISE EXCEPTION
                'The field “%” cannot be required yet: % no value for it.', col,
                CASE WHEN empty = 1 THEN '1 card has' ELSE empty || ' cards have' END
                USING ERRCODE = 'XL003', COLUMN = 'required';
        END IF;
        EXECUTE format('ALTER TABLE %s ALTER COLUMN %I SET NOT NULL', source, col);
    ELSE
        EXECUTE format('ALTER TABLE %s ALTER COLUMN %I DROP NOT NULL', source, col);
    END IF;
    PERFORM xila.note_column(bg, col, 'update',
        jsonb_build_object('name', col, 'required', was),
        jsonb_build_object('name', col, 'required', required));
END
$$;

-- ---------------------------------------------------------------------------
-- Fields
-- ---------------------------------------------------------------------------
-- A field is a widget bound to a column. On a managed stack the two live and
-- die together.

-- The PostgreSQL type behind a type of field.
CREATE FUNCTION xila.column_type(field_type text) RETURNS text
LANGUAGE plpgsql IMMUTABLE AS $$
BEGIN
    RETURN CASE coalesce(nullif(field_type, ''), 'text')
        WHEN 'text' THEN 'text'
        WHEN 'number' THEN 'numeric'
        WHEN 'date' THEN 'date'
        WHEN 'boolean' THEN 'boolean'
    END;
END
$$;

CREATE FUNCTION xila.field_widget(bg xila.background, field text) RETURNS xila.widget
LANGUAGE plpgsql STABLE AS $$
DECLARE
    result xila.widget;
BEGIN
    SELECT w.* INTO result
    FROM xila.widget w
    WHERE w.background_id = bg.id AND w.layer = 'background'
      AND w.name = field AND w.bound_column IS NOT NULL;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'This stack has no field named “%”.', field USING ERRCODE = 'XL002';
    END IF;
    RETURN result;
END
$$;

-- True when the widgets of a background have been placed by hand for a
-- renderer: at least one of them has a rectangle.
CREATE FUNCTION xila.has_free_layout(background uuid, wanted text) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT FROM xila.layout l JOIN xila.widget w ON w.id = l.widget_id
        WHERE w.background_id = background AND l.renderer = wanted AND l.w > 0 AND l.h > 0)
$$;

-- Gives a new field its place in the window when the layout is made by hand:
-- under the lowest field, with the same left edge and width.
CREATE FUNCTION xila.place_new_field(bg xila.background, field text) RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    lowest xila.layout;
BEGIN
    IF NOT xila.has_free_layout(bg.id, 'graphic') THEN
        RETURN;
    END IF;
    SELECT l.* INTO lowest
    FROM xila.layout l JOIN xila.widget w ON w.id = l.widget_id
    WHERE w.background_id = bg.id AND w.layer = 'background' AND w.kind = 'field'
      AND l.renderer = 'graphic' AND l.w > 0 AND l.h > 0
    ORDER BY l.y DESC, l.x
    LIMIT 1;
    INSERT INTO xila.layout (widget_id, renderer, x, y, w, h)
    SELECT w.id, 'graphic', coalesce(lowest.x, 100), coalesce(lowest.y + 20, 44),
           coalesce(lowest.w, 200), 14
    FROM xila.widget w
    WHERE w.background_id = bg.id AND w.layer = 'background' AND w.name = field;
END
$$;

-- Adds a field at the end and answers with its name. On a managed stack this
-- adds a column; name is made from the label when it is not given. On a stack
-- that shows an existing table, name is a column of that table.
CREATE FUNCTION xila.add_field(stack text, label text, type text DEFAULT 'text',
                               name text DEFAULT NULL)
RETURNS text
LANGUAGE plpgsql AS $$
DECLARE
    bg     xila.background := xila.editable(stack);
    source regclass := xila.source_of(bg);
    shown  text := nullif(btrim(coalesce(label, '')), '');
    col    text := coalesce(nullif(btrim(name), ''), xila.identifier_from(label));
    exists_in_table boolean;
BEGIN
    IF source IS NULL THEN
        RAISE EXCEPTION 'This stack has no cards, so it has no fields.' USING ERRCODE = 'XL003';
    END IF;
    IF col IS NULL OR col = '' THEN
        RAISE EXCEPTION 'A field needs a label.' USING ERRCODE = 'XL003', COLUMN = 'label';
    END IF;

    exists_in_table := EXISTS (
        SELECT FROM pg_attribute a
        WHERE a.attrelid = source AND a.attname = col AND a.attnum > 0 AND NOT a.attisdropped);

    IF EXISTS (SELECT FROM xila.widget w
               WHERE w.background_id = bg.id AND w.card_key IS NULL AND w.name = col) THEN
        RAISE EXCEPTION 'The name “%” is already used on this screen.', col
            USING ERRCODE = 'XL003', COLUMN = 'name';
    END IF;

    IF bg.managed THEN
        IF shown IS NULL THEN
            RAISE EXCEPTION 'A field needs a label.' USING ERRCODE = 'XL003', COLUMN = 'label';
        END IF;
        PERFORM xila.check_identifier(col);
        IF exists_in_table THEN
            RAISE EXCEPTION 'The name “%” is already used in this stack.', col
                USING ERRCODE = 'XL003', COLUMN = 'name';
        END IF;
        IF xila.column_type(type) IS NULL THEN
            RAISE EXCEPTION 'Unknown type of field: %.', type
                USING ERRCODE = 'XL003', COLUMN = 'type';
        END IF;
        PERFORM xila.column_add(bg, col, xila.column_type(type));
    ELSIF NOT exists_in_table THEN
        RAISE EXCEPTION
            'The table of this stack has no column “%”. pgXila did not create that table and never alters it: add the column in SQL first.',
            col
            USING ERRCODE = 'XL003', COLUMN = 'name';
    END IF;

    INSERT INTO xila.widget (background_id, name, kind, ord, label, bound_column)
    SELECT bg.id, col, 'field',
           coalesce(max(w.ord) FILTER (WHERE w.bound_column IS NOT NULL), 0) + 10,
           coalesce(shown, xila.label_from(col)), col
    FROM xila.widget w
    WHERE w.background_id = bg.id AND w.layer = 'background';

    -- In a layout made by hand, the new field goes under the lowest one.
    PERFORM xila.place_new_field(bg, col);

    -- The first field of a stack becomes the title of its cards.
    UPDATE xila.widget w
    SET props = w.props || jsonb_build_object('template', '{' || col || '}')
    WHERE w.background_id = bg.id AND w.layer = 'background' AND w.kind = 'text'
      AND w.props->>'style' = 'title' AND coalesce(w.props->>'template', '') = '';

    RETURN col;
END
$$;

-- Changes a field: {"label": "…", "name": "…", "required": true}. Renaming
-- renames the column; required is NOT NULL.
CREATE FUNCTION xila.set_field(stack text, field text, changes jsonb) RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    bg       xila.background := xila.editable(stack);
    w        xila.widget := xila.field_widget(bg, field);
    unknown  text;
    new_name text;
    ordered  boolean;
BEGIN
    SELECT k INTO unknown FROM jsonb_object_keys(changes) k
    WHERE k NOT IN ('label', 'name', 'required') LIMIT 1;
    IF unknown IS NOT NULL THEN
        RAISE EXCEPTION 'A field has no property “%”.', unknown USING ERRCODE = 'XL003';
    END IF;

    IF changes ? 'label' THEN
        IF btrim(coalesce(changes->>'label', '')) = '' THEN
            RAISE EXCEPTION 'A field needs a label.' USING ERRCODE = 'XL003', COLUMN = 'label';
        END IF;
        UPDATE xila.widget SET label = btrim(changes->>'label') WHERE id = w.id;
    END IF;

    IF changes ? 'required' THEN
        PERFORM xila.column_required(bg, w.bound_column,
                                     coalesce((changes->>'required')::boolean, false));
    END IF;

    new_name := btrim(coalesce(changes->>'name', ''));
    IF changes ? 'name' AND new_name <> w.name THEN
        PERFORM xila.managed_source(bg);
        PERFORM xila.check_identifier(new_name);
        IF EXISTS (SELECT FROM xila.widget x
                   WHERE x.background_id = bg.id AND x.card_key IS NULL AND x.name = new_name)
           OR EXISTS (SELECT FROM pg_attribute a
                      WHERE a.attrelid = xila.source_of(bg) AND a.attname = new_name
                        AND a.attnum > 0 AND NOT a.attisdropped) THEN
            RAISE EXCEPTION 'The name “%” is already used in this stack.', new_name
                USING ERRCODE = 'XL003', COLUMN = 'name';
        END IF;

        -- The order of the cards lets go of the column before it is renamed
        -- and takes it back afterwards: every step can then be undone alone.
        ordered := w.bound_column = ANY (bg.order_columns);
        IF ordered THEN
            UPDATE xila.background
            SET order_columns = array_remove(order_columns, w.bound_column) WHERE id = bg.id;
        END IF;
        PERFORM xila.column_rename(bg, w.bound_column, new_name);
        UPDATE xila.widget SET name = new_name, bound_column = new_name WHERE id = w.id;
        IF ordered THEN
            UPDATE xila.background
            SET order_columns = array_replace(bg.order_columns, w.bound_column, new_name::name)
            WHERE id = bg.id;
        END IF;
        UPDATE xila.widget x
        SET props = x.props || jsonb_build_object('template',
                replace(x.props->>'template', '{' || w.bound_column || '}', '{' || new_name || '}'))
        WHERE x.background_id = bg.id AND x.kind = 'text'
          AND x.props->>'template' LIKE '%{' || w.bound_column || '}%';
    END IF;
END
$$;

-- Moves a field up (by < 0) or down (by > 0) in the logical order.
CREATE FUNCTION xila.move_field(stack text, field text, by integer) RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    bg     xila.background := xila.editable(stack);
    w      xila.widget := xila.field_widget(bg, field);
    ids    uuid[];
    at     integer;
    target integer;
BEGIN
    SELECT array_agg(x.id ORDER BY x.ord, x.name) INTO ids
    FROM xila.widget x
    WHERE x.background_id = bg.id AND x.layer = 'background' AND x.bound_column IS NOT NULL;

    at := array_position(ids, w.id);
    target := greatest(1, least(cardinality(ids), at + by));
    IF target <> at THEN
        ids := array_remove(ids, w.id);
        ids := ids[1:target - 1] || w.id || ids[target:];
    END IF;

    UPDATE xila.widget x
    SET ord = o.n * 10
    FROM unnest(ids) WITH ORDINALITY AS o(id, n)
    WHERE x.id = o.id AND x.ord <> o.n * 10;
END
$$;

-- Removes a field. On a managed stack its column is dropped, and the values
-- it held are kept in history.
CREATE FUNCTION xila.remove_field(stack text, field text) RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    bg xila.background := xila.editable(stack);
    w  xila.widget := xila.field_widget(bg, field);
BEGIN
    IF bg.managed AND w.bound_column = ANY (bg.order_columns) THEN
        UPDATE xila.background
        SET order_columns = array_remove(order_columns, w.bound_column) WHERE id = bg.id;
    END IF;
    DELETE FROM xila.widget WHERE id = w.id;
    IF bg.managed THEN
        PERFORM xila.column_drop(bg, w.bound_column);
    END IF;
END
$$;

-- ---------------------------------------------------------------------------
-- Rewinding
-- ---------------------------------------------------------------------------
-- History is a list of changes with the row before and after each one. A
-- rewind applies the opposite of each change, newest first. What it does is
-- recorded like any other change: nothing is lost, and a rewind can itself be
-- rewound.
--
-- The opposite order is not always enough. When a row goes away with its
-- children, the parent is recorded first and the children after it, so that
-- undoing newest first would bring a child back before its parent. A step
-- that cannot be undone yet for that reason waits until the row it needs is
-- back, and so do the later steps on the same object.

-- The changes a rewind would undo, in no particular order.
CREATE FUNCTION xila.rewind_entries(stack uuid, scope text, since bigint, card text)
RETURNS SETOF xila.history
LANGUAGE sql STABLE AS $$
    SELECT h.*
    FROM xila.history h
    WHERE h.stack_id = stack AND h.id >= since
      AND CASE scope
              WHEN 'card' THEN h.object_kind = 'card' AND h.object_key = card
              WHEN 'screen' THEN h.object_kind NOT IN ('card', 'rewind')
              WHEN 'stack' THEN h.object_kind <> 'rewind'
              ELSE false
          END
$$;

-- Applies the opposite of one recorded change.
CREATE FUNCTION xila.undo_change(entry xila.history) RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    bg     xila.background;
    source regclass;
    target text;
    pk     text;
    cols   text;
BEGIN
    IF entry.object_kind IN ('card', 'column') THEN
        SELECT b.* INTO bg FROM xila.background b WHERE b.stack_id = entry.stack_id;
        source := xila.source_of(bg);
        IF source IS NULL THEN
            RAISE EXCEPTION 'the table of the stack is missing';
        END IF;
    END IF;

    IF entry.object_kind = 'card' THEN
        IF entry.op = 'insert' THEN
            EXECUTE format('DELETE FROM %s WHERE %I::text = $1', source, bg.key_column)
            USING entry.object_key;
            RETURN;
        END IF;

        -- The columns to write back: those of the old version that still
        -- exist and, for a changed card, only those that the change touched.
        SELECT string_agg(format('%I', a.attname), ', ') INTO cols
        FROM pg_attribute a
        WHERE a.attrelid = source AND a.attnum > 0 AND NOT a.attisdropped
          AND a.attgenerated = '' AND entry.before ? a.attname
          AND (entry.op = 'delete'
               OR entry.before->a.attname IS DISTINCT FROM entry.after->a.attname);
        IF cols IS NULL THEN
            RETURN;
        END IF;
        IF entry.op = 'delete' THEN
            EXECUTE format(
                'INSERT INTO %1$s (%2$s) OVERRIDING SYSTEM VALUE
                 SELECT %2$s FROM jsonb_populate_record(NULL::%1$s, $1)', source, cols)
            USING entry.before;
        ELSE
            EXECUTE format(
                'UPDATE %1$s SET (%2$s) = (SELECT %2$s FROM jsonb_populate_record(NULL::%1$s, $1))
                 WHERE %3$I::text = $2', source, cols, bg.key_column)
            USING entry.before, entry.after->>bg.key_column;
        END IF;

    ELSIF entry.object_kind = 'column' THEN
        IF entry.op = 'insert' THEN
            PERFORM xila.column_drop(bg, entry.after->>'name');
        ELSIF entry.op = 'delete' THEN
            PERFORM xila.column_add(bg, entry.before->>'name', entry.before->>'type',
                                    coalesce((entry.before->>'required')::boolean, false),
                                    entry.before->'values');
        ELSE
            IF entry.before->>'name' <> entry.after->>'name' THEN
                PERFORM xila.column_rename(bg, entry.after->>'name', entry.before->>'name');
            END IF;
            IF entry.before ? 'required' THEN
                PERFORM xila.column_required(bg, entry.before->>'name',
                                             (entry.before->>'required')::boolean);
            END IF;
        END IF;

    ELSIF entry.object_kind IN ('stack', 'background', 'action', 'widget', 'layout',
                                'key_binding', 'stack_colour') THEN
        target := format('xila.%I', entry.object_kind);
        pk := CASE entry.object_kind
            WHEN 'layout' THEN 'widget_id = ($1->>''widget_id'')::uuid AND renderer = $1->>''renderer'''
            WHEN 'stack_colour' THEN 'stack_id = ($1->>''stack_id'')::uuid AND slot = ($1->>''slot'')::smallint'
            ELSE 'id = ($1->>''id'')::uuid'
        END;
        IF entry.op = 'insert' THEN
            EXECUTE format('DELETE FROM %s WHERE %s', target, pk) USING entry.after;
        ELSIF entry.op = 'delete' THEN
            EXECUTE format('INSERT INTO %1$s SELECT * FROM jsonb_populate_record(NULL::%1$s, $1)',
                           target)
            USING entry.before;
        ELSE
            SELECT string_agg(format('%I', a.attname), ', ') INTO cols
            FROM pg_attribute a
            WHERE a.attrelid = target::regclass AND a.attnum > 0 AND NOT a.attisdropped;
            EXECUTE format(
                'UPDATE %1$s SET (%2$s) = (SELECT %2$s FROM jsonb_populate_record(NULL::%1$s, $2))
                 WHERE %3$s', target, cols, pk)
            USING entry.after, entry.before;
        END IF;

    ELSE
        RAISE EXCEPTION 'a change of kind “%” cannot be undone', entry.object_kind;
    END IF;
END
$$;

-- Tries to undo one change. Answers false when it has to wait for another
-- row to come back first; raises an error when it cannot be undone at all.
CREATE FUNCTION xila.try_undo(entry xila.history, OUT done boolean, OUT problem text)
LANGUAGE plpgsql AS $$
DECLARE
    detail text;
BEGIN
    BEGIN
        PERFORM xila.undo_change(entry);
        done := true;
    EXCEPTION
        WHEN foreign_key_violation THEN
            done := false;
            problem := 'XL003';
        WHEN insufficient_privilege THEN
            done := false;
            problem := 'XL004';
        WHEN OTHERS THEN
            detail := SQLERRM;
            RAISE EXCEPTION
                'The rewind stopped at the change of % (%): %. Nothing was changed.',
                to_char(entry.at, 'YYYY-MM-DD HH24:MI:SS'), entry.object_kind,
                rtrim(detail, '.')
                USING ERRCODE = 'XL003';
    END;
END
$$;

-- Undoes the recorded change "since" and all those after it, within a scope:
--   card    the values of one card, named by card; a deleted card comes back
--   screen  the definition of the stack, and the columns of its fields
--   stack   the definition and all the cards
-- Answers with the number of changes undone. All or nothing. This is the
-- worker of xila.rewind and of xila.undo, which marks its rewinds as undos.
CREATE FUNCTION xila.do_rewind(stack text, scope text, since bigint, card text,
                               is_undo boolean)
RETURNS jsonb
LANGUAGE plpgsql AS $$
DECLARE
    target   uuid;
    entry    xila.history;
    late     xila.history;
    outcome  record;
    undone   integer := 0;
    waiting  bigint[] := '{}';   -- changes that wait for a row to come back
    held     text[] := '{}';     -- the objects those changes are about
    still    bigint[];
    object   text;
    problem  text;
    moved    boolean;
BEGIN
    IF scope IS NULL OR scope NOT IN ('card', 'screen', 'stack') THEN
        RAISE EXCEPTION 'Unknown scope: %.', scope USING ERRCODE = 'XL002';
    END IF;
    IF scope = 'card' AND card IS NULL THEN
        RAISE EXCEPTION 'No card is named.' USING ERRCODE = 'XL002';
    END IF;

    -- The stack, or the last one of that name to have been deleted.
    SELECT s.id INTO target FROM xila.stack s WHERE s.name = stack;
    IF target IS NULL THEN
        SELECT h.stack_id INTO target
        FROM xila.history h
        WHERE h.object_kind = 'stack' AND h.op = 'delete' AND h.before->>'name' = stack
        ORDER BY h.id DESC
        LIMIT 1;
    END IF;
    IF target IS NULL THEN
        RAISE EXCEPTION 'The stack “%” does not exist.', stack USING ERRCODE = 'XL002';
    END IF;
    IF scope <> 'card' AND NOT xila.may_rewind(target) THEN
        RAISE EXCEPTION 'You are not allowed to rewind the stack “%”.', stack
            USING ERRCODE = 'XL004';
    END IF;

    FOR entry IN
        SELECT * FROM xila.rewind_entries(target, scope, since, card) ORDER BY id DESC
    LOOP
        object := entry.object_kind || ' ' || entry.object_key;
        IF object = ANY (held) THEN
            waiting := waiting || entry.id;
            CONTINUE;
        END IF;
        outcome := xila.try_undo(entry);
        IF NOT outcome.done THEN
            problem := outcome.problem;
            waiting := waiting || entry.id;
            held := held || object;
            CONTINUE;
        END IF;
        undone := undone + 1;

        -- Something came back: what was waiting may now go through.
        WHILE waiting <> '{}' LOOP
            moved := false;
            still := '{}';
            held := '{}';
            FOR late IN
                SELECT * FROM xila.history h WHERE h.id = ANY (waiting) ORDER BY h.id DESC
            LOOP
                object := late.object_kind || ' ' || late.object_key;
                IF object = ANY (held) THEN
                    still := still || late.id;
                    CONTINUE;
                END IF;
                outcome := xila.try_undo(late);
                IF outcome.done THEN
                    undone := undone + 1;
                    moved := true;
                ELSE
                    problem := outcome.problem;
                    still := still || late.id;
                    held := held || object;
                END IF;
            END LOOP;
            waiting := still;
            EXIT WHEN NOT moved;
        END LOOP;
    END LOOP;

    IF waiting <> '{}' THEN
        IF problem = 'XL004' THEN
            RAISE EXCEPTION 'You are not allowed to undo these changes.' USING ERRCODE = 'XL004';
        END IF;
        RAISE EXCEPTION
            'The rewind could not put back something that other things still need. Nothing was changed.'
            USING ERRCODE = 'XL003';
    END IF;

    IF undone > 0 THEN
        INSERT INTO xila.history (origin, stack_id, object_kind, object_key, op, after)
        VALUES (coalesce(nullif(current_setting('xila.origin', true), ''), 'user'),
                target, 'rewind', coalesce(card, scope), 'insert',
                jsonb_build_object('scope', scope, 'since', since, 'card', card,
                                   'undone', undone, 'undo', is_undo));
    END IF;
    RETURN jsonb_build_object('undone', undone);
END
$$;

CREATE FUNCTION xila.rewind(stack text, scope text, since bigint, card text DEFAULT NULL)
RETURNS jsonb
LANGUAGE sql AS $$
    SELECT xila.do_rewind(stack, scope, since, card, false)
$$;

-- The same, to a moment in time: undoes everything recorded after it.
CREATE FUNCTION xila.rewind_to(stack text, scope text, moment timestamptz,
                               card text DEFAULT NULL)
RETURNS jsonb
LANGUAGE sql AS $$
    SELECT xila.rewind(stack, scope,
        coalesce((SELECT min(h.id) FROM xila.history h WHERE h.at > moment),
                 9223372036854775807),
        card)
$$;

-- Undoes the last change made to the screen of a stack: the most recent line
-- of its history that is not itself an undo. Called again, it goes one line
-- further back; any new change starts over from the most recent line.
--
-- An undo is a rewind of the screen from that line on. Rewinding from an older
-- line also takes back the undos made since, and what they had undone: the
-- changes cancel out, and the screen is as it was before that line.
--
-- Only the changes of the current role are undone this way: the History
-- screen shows who did what before anything of someone else is rewound.
CREATE FUNCTION xila.undo(stack text) RETURNS jsonb
LANGUAGE plpgsql AS $$
DECLARE
    bg      xila.background := xila.editable(stack);
    line    record;
    reached bigint;        -- every line from this one on is already undone
    target  record;
    found   boolean := false;
    summary text;
    result  jsonb;
BEGIN
    FOR line IN
        SELECT min(h.id) AS id, min(h.actor::text) AS actor,
               min((h.after->>'since')::bigint) FILTER (
                   WHERE h.object_kind = 'rewind'
                     AND coalesce((h.after->>'undo')::boolean, false)) AS undone_since,
               bool_or(h.object_kind NOT IN ('card', 'rewind')) AS on_screen,
               bool_or(h.object_kind = 'stack' AND h.op = 'insert') AS creation
        FROM xila.history h
        WHERE h.stack_id = bg.stack_id
        GROUP BY h.txid
        ORDER BY 1 DESC
    LOOP
        IF line.undone_since IS NOT NULL THEN
            reached := least(coalesce(reached, line.undone_since), line.undone_since);
        ELSIF line.on_screen AND (reached IS NULL OR line.id < reached) THEN
            target := line;
            found := true;
            EXIT;
        END IF;
    END LOOP;

    -- Creating the stack is not something to undo with a key.
    IF NOT found OR target.creation THEN
        RAISE EXCEPTION 'There is nothing left to undo.' USING ERRCODE = 'XL003';
    END IF;
    IF target.actor <> session_user::text THEN
        RAISE EXCEPTION
            'The last change was made by %: use the history of the stack to rewind it.',
            target.actor
            USING ERRCODE = 'XL004';
    END IF;

    SELECT l.summary INTO summary FROM xila.history_line l WHERE l.id = target.id;
    result := xila.do_rewind(stack, 'screen', target.id, NULL, true);
    RETURN result || jsonb_build_object('summary', summary);
END
$$;

-- ---------------------------------------------------------------------------
-- Widgets and their places
-- ---------------------------------------------------------------------------
-- What the visual editor does. A widget is named within its stack; one that
-- belongs to a single card is looked for on the card given, before the
-- background.

CREATE FUNCTION xila.widget_named(bg xila.background, widget text, card text)
RETURNS xila.widget
LANGUAGE plpgsql STABLE AS $$
DECLARE
    result xila.widget;
BEGIN
    SELECT w.* INTO result
    FROM xila.widget w
    WHERE w.background_id = bg.id AND w.name = widget
      AND (w.card_key IS NULL OR w.card_key = card)
    ORDER BY w.card_key NULLS LAST
    LIMIT 1;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'This screen has nothing named “%”.', widget USING ERRCODE = 'XL002';
    END IF;
    RETURN result;
END
$$;

-- Sets the rectangle of some widgets for one renderer:
-- {"city": {"x": 128, "y": 94, "w": 100, "h": 14}}. Styles are kept.
CREATE FUNCTION xila.place_widgets(stack text, renderer text, placements jsonb,
                                   card text DEFAULT NULL)
RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    bg      xila.background := xila.editable(stack);
    wanted  text := renderer;
    entry   record;
    w       xila.widget;
    box     integer[];
    limits  integer[] := CASE renderer WHEN 'text' THEN ARRAY[80, 24] ELSE ARRAY[480, 300] END;
BEGIN
    IF wanted IS NULL OR wanted NOT IN ('text', 'graphic') THEN
        RAISE EXCEPTION 'Unknown renderer: %.', wanted USING ERRCODE = 'XL003';
    END IF;
    FOR entry IN SELECT e.key, e.value FROM jsonb_each(coalesce(placements, '{}')) e LOOP
        w := xila.widget_named(bg, entry.key, card);
        BEGIN
            box := ARRAY[(entry.value->>'x')::integer, (entry.value->>'y')::integer,
                         (entry.value->>'w')::integer, (entry.value->>'h')::integer];
        EXCEPTION WHEN data_exception THEN
            box := NULL;
        END;
        IF box IS NULL OR array_position(box, NULL) IS NOT NULL
           OR box[1] < 0 OR box[2] < 0 OR box[3] < 1 OR box[4] < 1
           OR box[1] + box[3] > limits[1] OR box[2] + box[4] > limits[2] THEN
            RAISE EXCEPTION 'The place given to “%” does not fit on the screen.', entry.key
                USING ERRCODE = 'XL003';
        END IF;
        INSERT INTO xila.layout AS l (widget_id, renderer, x, y, w, h)
        VALUES (w.id, wanted, box[1], box[2], box[3], box[4])
        ON CONFLICT ON CONSTRAINT layout_pkey
        DO UPDATE SET x = excluded.x, y = excluded.y, w = excluded.w, h = excluded.h;
    END LOOP;
END
$$;

-- Forgets every rectangle of a renderer: the placement is automatic again.
-- Styles are kept.
CREATE FUNCTION xila.reset_layout(stack text, renderer text) RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    bg     xila.background := xila.editable(stack);
    wanted text := renderer;
BEGIN
    DELETE FROM xila.layout l
    USING xila.widget w
    WHERE w.id = l.widget_id AND w.background_id = bg.id
      AND l.renderer = wanted AND l.style = '{}';
    UPDATE xila.layout l
    SET x = 0, y = 0, w = 0, h = 0
    FROM xila.widget w
    WHERE w.id = l.widget_id AND w.background_id = bg.id
      AND l.renderer = wanted AND (l.w <> 0 OR l.h <> 0 OR l.x <> 0 OR l.y <> 0);
END
$$;

-- The action of a stack by its name: one of its own, or one shared by every
-- screen. An empty name is no action.
CREATE FUNCTION xila.action_named(bg xila.background, action text) RETURNS uuid
LANGUAGE plpgsql STABLE AS $$
DECLARE
    result uuid;
BEGIN
    IF coalesce(action, '') = '' THEN
        RETURN NULL;
    END IF;
    SELECT a.id INTO result
    FROM xila.action a
    WHERE a.name = action AND (a.background_id = bg.id OR a.background_id IS NULL)
    ORDER BY a.background_id NULLS LAST
    LIMIT 1;
    IF result IS NULL THEN
        RAISE EXCEPTION 'This stack has no action named “%”.', action
            USING ERRCODE = 'XL003', COLUMN = 'action';
    END IF;
    RETURN result;
END
$$;

-- Adds a text or a button and answers with its name. props may hold "label",
-- "text" (a template, for a text), "action" (the name of an action of the
-- stack, for a button) and "placement" (a rectangle for the window). With
-- card, the widget belongs to that card only.
CREATE FUNCTION xila.add_widget(stack text, kind text, props jsonb DEFAULT '{}',
                                card text DEFAULT NULL)
RETURNS text
LANGUAGE plpgsql AS $$
DECLARE
    bg       xila.background := xila.editable(stack);
    wanted   text := kind;
    given    jsonb := coalesce(props, '{}');
    new_name text;
    new_id   uuid;
    n        integer := 0;
BEGIN
    IF wanted IS NULL OR wanted NOT IN ('text', 'button') THEN
        RAISE EXCEPTION
            'Only texts and buttons are added this way. A field is added with its own dialog.'
            USING ERRCODE = 'XL003';
    END IF;
    IF card IS NOT NULL AND xila.source_of(bg) IS NULL THEN
        RAISE EXCEPTION 'This stack has no cards.' USING ERRCODE = 'XL003';
    END IF;

    -- text1, text2... : the first name that nothing on the screen has.
    LOOP
        n := n + 1;
        new_name := wanted || n;
        EXIT WHEN NOT EXISTS (
            SELECT FROM xila.widget w WHERE w.background_id = bg.id AND w.name = new_name);
    END LOOP;

    INSERT INTO xila.widget (background_id, layer, card_key, name, kind, ord, label, action_id,
                             props)
    SELECT bg.id, CASE WHEN card IS NULL THEN 'background' ELSE 'card' END, card, new_name,
           wanted, coalesce(max(w.ord), 0) + 10,
           CASE WHEN wanted = 'button' THEN coalesce(nullif(btrim(given->>'label'), ''), 'Button')
           END,
           CASE WHEN wanted = 'button' THEN xila.action_named(bg, given->>'action') END,
           CASE WHEN wanted = 'text'
                THEN jsonb_build_object('template', coalesce(given->>'text', 'Text'))
                ELSE '{}'::jsonb END
    FROM xila.widget w
    WHERE w.background_id = bg.id
    RETURNING id INTO new_id;

    IF given ? 'placement' THEN
        PERFORM xila.place_widgets(stack, 'graphic',
                                   jsonb_build_object(new_name, given->'placement'), card);
    END IF;
    RETURN new_name;
END
$$;

-- Changes a widget: {"label": "…", "text": "…", "action": "next", "main":
-- true, "required": true, "layer": "card"}. "layer" moves a text or a button
-- between the background and the card given.
CREATE FUNCTION xila.set_widget(stack text, widget text, changes jsonb,
                                card text DEFAULT NULL)
RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    bg       xila.background := xila.editable(stack);
    w        xila.widget := xila.widget_named(bg, widget, card);
    is_field boolean := w.kind = 'field' AND w.bound_column IS NOT NULL AND w.card_key IS NULL;
    unknown  text;
    to_layer text;
BEGIN
    SELECT k INTO unknown FROM jsonb_object_keys(changes) k
    WHERE k NOT IN ('label', 'text', 'action', 'main', 'required', 'layer') LIMIT 1;
    IF unknown IS NOT NULL THEN
        RAISE EXCEPTION 'A widget has no property “%”.', unknown USING ERRCODE = 'XL003';
    END IF;

    -- What makes a field a field goes through the functions of fields.
    IF is_field AND changes ?| ARRAY['label', 'required'] THEN
        PERFORM xila.set_field(stack, w.name, (
            SELECT jsonb_object_agg(e.key, e.value) FROM jsonb_each(changes) e
            WHERE e.key IN ('label', 'required')));
    ELSIF changes ? 'required' THEN
        RAISE EXCEPTION 'Only a field can be required.' USING ERRCODE = 'XL003';
    ELSIF changes ? 'label' THEN
        UPDATE xila.widget SET label = nullif(btrim(changes->>'label'), '') WHERE id = w.id;
    END IF;

    IF changes ? 'text' THEN
        IF w.kind <> 'text' THEN
            RAISE EXCEPTION 'Only a text has a text.' USING ERRCODE = 'XL003';
        END IF;
        UPDATE xila.widget x
        SET props = x.props || jsonb_build_object('template', coalesce(changes->>'text', ''))
        WHERE x.id = w.id;
    END IF;

    IF changes ? 'action' THEN
        IF w.kind <> 'button' THEN
            RAISE EXCEPTION 'Only a button has an action.' USING ERRCODE = 'XL003';
        END IF;
        UPDATE xila.widget SET action_id = xila.action_named(bg, changes->>'action')
        WHERE id = w.id;
    END IF;

    IF changes ? 'main' THEN
        IF w.kind <> 'button' THEN
            RAISE EXCEPTION 'Only a button can be the main button.' USING ERRCODE = 'XL003';
        END IF;
        IF coalesce((changes->>'main')::boolean, false) THEN
            INSERT INTO xila.layout AS l (widget_id, renderer, style)
            VALUES (w.id, 'graphic', '{"variant": "primary"}')
            ON CONFLICT ON CONSTRAINT layout_pkey
            DO UPDATE SET style = l.style || '{"variant": "primary"}';
        ELSE
            UPDATE xila.layout l SET style = l.style - 'variant'
            WHERE l.widget_id = w.id AND l.renderer = 'graphic';
        END IF;
    END IF;

    IF changes ? 'layer' THEN
        to_layer := changes->>'layer';
        IF w.kind NOT IN ('text', 'button') OR w.bound_column IS NOT NULL THEN
            RAISE EXCEPTION 'Only a text or a button can belong to a single card.'
                USING ERRCODE = 'XL003';
        END IF;
        IF to_layer = 'card' THEN
            IF card IS NULL THEN
                RAISE EXCEPTION 'There is no card to put it on.' USING ERRCODE = 'XL003';
            END IF;
            UPDATE xila.widget SET layer = 'card', card_key = card WHERE id = w.id;
        ELSIF to_layer = 'background' THEN
            UPDATE xila.widget SET layer = 'background', card_key = NULL WHERE id = w.id;
        ELSE
            RAISE EXCEPTION 'Unknown layer: %.', to_layer USING ERRCODE = 'XL003';
        END IF;
    END IF;
END
$$;

-- Removes a widget. A field is removed as by xila.remove_field: on a managed
-- stack its column goes with it.
CREATE FUNCTION xila.remove_widget(stack text, widget text, card text DEFAULT NULL)
RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    bg xila.background := xila.editable(stack);
    w  xila.widget := xila.widget_named(bg, widget, card);
BEGIN
    IF w.bound_column IS NOT NULL AND w.card_key IS NULL THEN
        PERFORM xila.remove_field(stack, w.name);
    ELSE
        DELETE FROM xila.widget WHERE id = w.id;
    END IF;
END
$$;

-- ---------------------------------------------------------------------------
-- What the author screens show
-- ---------------------------------------------------------------------------
-- The author screens are system stacks. These views are their cards and the
-- rows of their grids. They run with the privileges of the reader.

-- One line per user stack.
CREATE VIEW xila.stack_info WITH (security_invoker = true) AS
    SELECT s.name, s.title, s.owner::text AS owner, b.managed,
           coalesce(xila.card_title_template(b.id), '') AS card_title
    FROM xila.stack s JOIN xila.background b ON b.stack_id = s.id
    WHERE s.kind = 'user';

-- One line per field of a user stack.
CREATE VIEW xila.field WITH (security_invoker = true) AS
    SELECT w.id, s.name AS stack, w.name, w.label, w.ord, f.type,
           coalesce(a.attnotnull, false) AS required,
           CASE f.type WHEN 'text' THEN 'Text' WHEN 'number' THEN 'Number'
                       WHEN 'date' THEN 'Date' WHEN 'boolean' THEN 'Yes or no'
                       ELSE 'Picture' END AS type_label,
           CASE WHEN a.attnotnull THEN 'Yes' ELSE '' END AS required_label
    FROM xila.widget w
    JOIN xila.background b ON b.id = w.background_id
    JOIN xila.stack s ON s.id = b.stack_id
    CROSS JOIN LATERAL (SELECT xila.source_of(b) AS source) x
    CROSS JOIN LATERAL (
        SELECT CASE WHEN w.kind = 'image' THEN 'picture'
                    ELSE xila.field_type(x.source, w.bound_column) END AS type) f
    LEFT JOIN pg_attribute a
           ON a.attrelid = x.source AND a.attname = w.bound_column
          AND a.attnum > 0 AND NOT a.attisdropped
    WHERE s.kind = 'user' AND w.layer = 'background' AND w.bound_column IS NOT NULL;

-- The card of the "New stack" dialog: a form with nothing in it.
CREATE VIEW xila.new_stack_form AS
    SELECT ''::text AS title, ''::text AS name, ''::text AS source WHERE false;

-- The tables a new stack can be made from: those of the current role that
-- have a single-column primary key and are not a stack yet.
CREATE VIEW xila.own_table WITH (security_invoker = true) AS
    SELECT format('%I.%I', n.nspname, c.relname) AS name
    FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE c.relkind IN ('r', 'p')
      AND n.nspname NOT IN ('xila', 'information_schema') AND n.nspname !~ '^pg_'
      AND pg_has_role(c.relowner, 'USAGE')
      AND EXISTS (SELECT FROM pg_index i
                  WHERE i.indrelid = c.oid AND i.indisprimary AND i.indnkeyatts = 1)
      AND NOT EXISTS (SELECT FROM xila.background b
                      WHERE b.source_schema = n.nspname AND b.source_table = c.relname);

-- How much a kind of change says about what the user did: the lowest wins
-- when several changes are shown as one line.
CREATE FUNCTION xila.change_rank(kind text) RETURNS integer
LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE kind WHEN 'rewind' THEN 0 WHEN 'stack' THEN 1 WHEN 'column' THEN 2
                     WHEN 'widget' THEN 3 WHEN 'card' THEN 4 WHEN 'layout' THEN 5
                     ELSE 6 END
$$;

-- One recorded change, in words.
CREATE FUNCTION xila.describe_change(kind text, op text, before jsonb, after jsonb,
                                     card_title text DEFAULT NULL)
RETURNS text
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    cur  jsonb := coalesce(after, before);
    card text := CASE WHEN coalesce(card_title, '') = '' THEN 'Card'
                      ELSE 'Card “' || card_title || '”' END;
    cols text;
BEGIN
    CASE kind
    WHEN 'rewind' THEN
        RETURN format('%s rewound (%s undone)',
            CASE after->>'scope' WHEN 'card' THEN 'Card' WHEN 'screen' THEN 'Screen'
                                 ELSE 'Stack' END,
            CASE WHEN after->>'undone' = '1' THEN '1 change'
                 ELSE (after->>'undone') || ' changes' END);
    WHEN 'stack' THEN
        RETURN CASE
            WHEN op = 'insert' THEN 'Stack created'
            WHEN op = 'delete' THEN 'Stack deleted'
            WHEN before->>'title' <> after->>'title'
                THEN format('Title changed to “%s”', after->>'title')
            ELSE 'Stack changed' END;
    WHEN 'column' THEN
        RETURN CASE
            WHEN op = 'insert' THEN format('Field “%s” added', after->>'name')
            WHEN op = 'delete' THEN format('Field “%s” removed', before->>'name')
            WHEN before->>'name' <> after->>'name'
                THEN format('Field “%s” renamed “%s”', before->>'name', after->>'name')
            WHEN (after->>'required')::boolean
                THEN format('Field “%s” made required', after->>'name')
            ELSE format('Field “%s” made optional', after->>'name') END;
    WHEN 'widget' THEN
        RETURN CASE
            WHEN op = 'insert' THEN format('“%s” added to the screen', cur->>'name')
            WHEN op = 'delete' THEN format('“%s” taken off the screen', cur->>'name')
            WHEN before->>'label' IS DISTINCT FROM after->>'label'
                THEN format('Label of “%s” changed to “%s”', after->>'name', after->>'label')
            WHEN before->'props'->>'template' IS DISTINCT FROM after->'props'->>'template'
                THEN CASE WHEN after->'props'->>'style' = 'title' THEN 'Card title changed'
                          ELSE format('Text of “%s” changed', after->>'name') END
            WHEN before->>'ord' <> after->>'ord' THEN format('“%s” moved', after->>'name')
            ELSE format('“%s” changed', after->>'name') END;
    WHEN 'card' THEN
        IF op = 'insert' THEN
            RETURN card || ' added';
        ELSIF op = 'delete' THEN
            RETURN card || ' deleted';
        END IF;
        SELECT string_agg(b.key, ', ' ORDER BY b.key) INTO cols
        FROM jsonb_each(before) b
        WHERE b.value IS DISTINCT FROM after->b.key;
        RETURN card || ' changed' || coalesce(': ' || cols, '');
    WHEN 'layout' THEN
        -- card_title is here the name of the widget, when a single one moved.
        RETURN CASE WHEN coalesce(card_title, '') = '' THEN 'Layout changed'
                    ELSE format('“%s” moved', card_title) END;
    ELSE
        RETURN 'Screen changed';
    END CASE;
END
$$;

-- The title of a card, from one of its recorded versions.
CREATE FUNCTION xila.card_title_of(stack uuid, version jsonb) RETURNS text
LANGUAGE sql STABLE AS $$
    SELECT btrim(xila.fill_template(xila.card_title_template(b.id), version), ' ,;:-/')
    FROM xila.background b
    WHERE b.stack_id = stack
$$;

-- The history of a stack, one line per gesture: the changes of one
-- transaction are shown as the most telling of them. id is the first change
-- of the line: rewinding to it undoes the whole line.
CREATE VIEW xila.history_line WITH (security_invoker = true) AS
    SELECT l.id, s.name AS stack, to_char(l.at, 'YYYY-MM-DD HH24:MI') AS at, l.actor,
           CASE WHEN m.object_kind = 'card' AND l.changes > 1
                THEN l.changes || ' cards changed'
                ELSE xila.describe_change(m.object_kind, m.op, m.before, m.after,
                         CASE WHEN m.object_kind = 'card'
                              THEN xila.card_title_of(m.stack_id, coalesce(m.after, m.before))
                              WHEN m.object_kind = 'layout' AND l.changes = 1
                              THEN (SELECT w.name FROM xila.widget w
                                    WHERE w.id = (coalesce(m.after, m.before)->>'widget_id')::uuid)
                         END)
           END AS summary
    FROM (
        SELECT h.stack_id, h.txid, min(h.id) AS id, min(h.at) AS at,
               min(h.actor::text) AS actor, count(*) AS changes,
               (array_agg(h.id ORDER BY xila.change_rank(h.object_kind), h.id))[1] AS main
        FROM xila.history h
        WHERE h.stack_id IS NOT NULL
        GROUP BY h.stack_id, h.txid) l
    JOIN xila.stack s ON s.id = l.stack_id
    JOIN xila.history m ON m.id = l.main;

-- The changes made to the cards of a stack, one line per change.
CREATE VIEW xila.card_history_line WITH (security_invoker = true) AS
    SELECT h.id, s.name AS stack, h.object_key AS card,
           to_char(h.at, 'YYYY-MM-DD HH24:MI') AS at, h.actor::text AS actor,
           xila.describe_change('card', h.op, h.before, h.after,
               xila.card_title_of(h.stack_id, coalesce(h.after, h.before))) AS summary
    FROM xila.history h JOIN xila.stack s ON s.id = h.stack_id
    WHERE h.object_kind = 'card';

-- The stacks that were deleted and can be brought back. id is the change that
-- deleted the stack: its rows went after it, so rewinding to it undoes the
-- whole deletion.
CREATE VIEW xila.deleted_stack WITH (security_invoker = true) AS
    SELECT DISTINCT ON (h.stack_id)
           h.id, h.before->>'name' AS name, h.before->>'title' AS title,
           to_char(h.at, 'YYYY-MM-DD HH24:MI') AS at, h.actor::text AS actor
    FROM xila.history h
    WHERE h.object_kind = 'stack' AND h.op = 'delete'
      AND NOT EXISTS (SELECT FROM xila.stack s WHERE s.id = h.stack_id)
    ORDER BY h.stack_id, h.id DESC;

GRANT SELECT ON xila.stack_info, xila.field, xila.new_stack_form, xila.own_table,
    xila.history_line, xila.card_history_line, xila.deleted_stack TO xila_user;
