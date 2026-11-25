-- pgXila client API. See docs/api.md.
--
-- Every function runs with the privileges of the caller: what a user may read
-- or write in a stack is decided by ordinary PostgreSQL privileges on the
-- stack's table. Identifiers that end up in dynamic SQL always come from the
-- catalog or go through format('%I').
--
-- Errors meant for the user use SQLSTATE class XL and a message written for
-- the user, which the clients show as it is:
--   XL002  unknown stack, card, widget or action
--   XL003  a value was rejected
--   XL004  not allowed for this role

-- ---------------------------------------------------------------------------
-- Helpers
-- ---------------------------------------------------------------------------

-- The background of a stack, or an XL002 error.
CREATE FUNCTION xila.background_of(stack text) RETURNS xila.background
LANGUAGE plpgsql STABLE AS $$
DECLARE
    result xila.background;
BEGIN
    SELECT b.* INTO result
    FROM xila.stack s JOIN xila.background b ON b.stack_id = s.id
    WHERE s.name = stack;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'The stack “%” does not exist.', stack USING ERRCODE = 'XL002';
    END IF;
    RETURN result;
END
$$;

-- The source table of a background, or NULL for a static screen.
CREATE FUNCTION xila.source_of(bg xila.background) RETURNS regclass
LANGUAGE sql STABLE AS $$
    SELECT CASE WHEN bg.source_table IS NOT NULL
                THEN to_regclass(format('%I.%I', bg.source_schema, bg.source_table)) END
$$;

-- "ORDER BY" expression of the cards of a background: the order columns,
-- then the key so that the order is total.
CREATE FUNCTION xila.card_order(bg xila.background) RETURNS text
LANGUAGE sql STABLE AS $$
    SELECT string_agg(format('%I', c), ', ' ORDER BY n)
    FROM unnest(bg.order_columns || bg.key_column) WITH ORDINALITY AS o(c, n)
$$;

-- How a client should edit a column, from its PostgreSQL type.
CREATE FUNCTION xila.field_type(source regclass, col name) RETURNS text
LANGUAGE sql STABLE AS $$
    SELECT CASE
        WHEN t.typcategory = 'N' THEN 'number'
        WHEN t.typcategory = 'B' THEN 'boolean'
        WHEN t.typcategory = 'D' THEN 'date'
        ELSE 'text'
    END
    FROM pg_attribute a JOIN pg_type t ON t.oid = a.atttypid
    WHERE a.attrelid = source AND a.attname = col AND a.attnum > 0 AND NOT a.attisdropped
$$;

-- Replaces {name} placeholders by the matching entry of vals.
CREATE FUNCTION xila.fill_template(template text, vals jsonb) RETURNS text
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    result text := template;
    entry  record;
BEGIN
    FOR entry IN SELECT key, value FROM jsonb_each_text(vals) LOOP
        result := replace(result, '{' || entry.key || '}', coalesce(entry.value, ''));
    END LOOP;
    RETURN result;
END
$$;

-- The widgets of a background as JSON: those of the background layer when
-- card is NULL, those of one card otherwise.
CREATE FUNCTION xila.widgets_json(bg xila.background, card text) RETURNS jsonb
LANGUAGE sql STABLE AS $$
    SELECT coalesce(jsonb_agg(jsonb_strip_nulls(jsonb_build_object(
               'name', w.name,
               'kind', w.kind,
               'layer', w.layer,
               'ord', w.ord,
               'group', w.grp,
               'label', w.label,
               'column', w.bound_column,
               'type', CASE WHEN w.bound_column IS NOT NULL
                            THEN xila.field_type(xila.source_of(bg), w.bound_column) END,
               'action', a.name))
               || jsonb_build_object(
               'props', w.props,
               'layout', coalesce((
                   SELECT jsonb_object_agg(l.renderer, jsonb_build_object(
                       'x', l.x, 'y', l.y, 'w', l.w, 'h', l.h, 'style', l.style))
                   FROM xila.layout l WHERE l.widget_id = w.id), '{}'))
               ORDER BY w.ord, w.name), '[]')
    FROM xila.widget w LEFT JOIN xila.action a ON a.id = w.action_id
    WHERE w.background_id = bg.id AND w.card_key IS NOT DISTINCT FROM card
$$;

-- ---------------------------------------------------------------------------
-- Session
-- ---------------------------------------------------------------------------

CREATE FUNCTION xila.hello() RETURNS jsonb
LANGUAGE sql STABLE AS $$
    SELECT jsonb_build_object(
        'api', 1,
        'version', xila.version(),
        'instance', coalesce(nullif(current_setting('cluster_name'), ''), 'pgxila'),
        'role', session_user,
        'author', pg_has_role(session_user, 'xila_author', 'MEMBER'),
        'home', 'desktop')
$$;

CREATE FUNCTION xila.palette() RETURNS TABLE (slot smallint, name text, rgb text)
LANGUAGE sql STABLE AS $$
    SELECT p.slot, p.name, p.rgb FROM xila.palette p ORDER BY p.slot
$$;

CREATE FUNCTION xila.image(image_id uuid)
RETURNS TABLE (width smallint, height smallint, pixels bytea)
LANGUAGE sql STABLE AS $$
    SELECT i.width, i.height, i.pixels FROM xila.image i WHERE i.id = image_id
$$;

-- ---------------------------------------------------------------------------
-- Stacks
-- ---------------------------------------------------------------------------

-- Number of cards of a stack: its rows, or 1 for a static screen.
CREATE FUNCTION xila.card_count(stack text) RETURNS bigint
LANGUAGE plpgsql STABLE AS $$
DECLARE
    bg     xila.background := xila.background_of(stack);
    source regclass := xila.source_of(bg);
    result bigint;
BEGIN
    IF source IS NULL THEN
        RETURN 1;
    END IF;
    EXECUTE format('SELECT count(*) FROM %s', source) INTO result;
    RETURN result;
END
$$;

-- The user stacks the current role may open: those whose table it may read.
CREATE VIEW xila.visible_stack AS
    SELECT s.name, s.title, xila.card_count(s.name) AS cards
    FROM xila.stack s JOIN xila.background b ON b.stack_id = s.id
    WHERE s.kind = 'user'
      AND CASE WHEN b.source_table IS NULL THEN true
               ELSE has_table_privilege(
                        to_regclass(format('%I.%I', b.source_schema, b.source_table))::oid,
                        'SELECT')
          END;

CREATE FUNCTION xila.open_stack(stack text) RETURNS jsonb
LANGUAGE plpgsql STABLE AS $$
DECLARE
    bg     xila.background := xila.background_of(stack);
    source regclass := xila.source_of(bg);
    info   xila.stack;
BEGIN
    SELECT * INTO info FROM xila.stack WHERE id = bg.stack_id;

    IF source IS NOT NULL AND NOT has_table_privilege(source, 'SELECT') THEN
        RAISE EXCEPTION 'You do not have access to the stack “%”.', stack USING ERRCODE = 'XL004';
    END IF;

    RETURN jsonb_build_object(
        'stack', jsonb_build_object('name', info.name, 'title', info.title, 'kind', info.kind),
        'source', jsonb_build_object(
            'has_cards', source IS NOT NULL,
            'count', xila.card_count(stack)),
        'widgets', xila.widgets_json(bg, NULL),
        'actions', (
            SELECT coalesce(jsonb_agg(jsonb_build_object('name', a.name, 'label', a.label)
                                      ORDER BY a.name), '[]')
            FROM xila.action a
            WHERE a.background_id = bg.id OR a.background_id IS NULL),
        -- A key bound on this screen wins over the same key bound everywhere.
        'keys', (
            SELECT coalesce(jsonb_agg(jsonb_build_object(
                       'key', k.key, 'label', k.label, 'action', k.action)
                       ORDER BY length(k.key), k.key), '[]')
            FROM (
                SELECT DISTINCT ON (kb.key) kb.key, a.label, a.name AS action
                FROM xila.key_binding kb JOIN xila.action a ON a.id = kb.action_id
                WHERE kb.background_id = bg.id OR kb.background_id IS NULL
                ORDER BY kb.key, kb.background_id NULLS LAST) k),
        'colours', (
            SELECT coalesce(jsonb_agg(jsonb_build_object('slot', c.slot, 'rgb', c.rgb)
                                      ORDER BY c.slot), '[]')
            FROM xila.stack_colour c WHERE c.stack_id = bg.stack_id));
END
$$;

-- ---------------------------------------------------------------------------
-- Cards
-- ---------------------------------------------------------------------------

-- Reads one card. move is 'first', 'last', 'at', 'next' or 'previous'; the
-- last three are relative to key. When key no longer exists, the first card
-- is returned. 'next' and 'previous' stop at the ends.
CREATE FUNCTION xila.card(stack text, key text DEFAULT NULL, move text DEFAULT 'first')
RETURNS jsonb
LANGUAGE plpgsql STABLE AS $$
DECLARE
    bg     xila.background := xila.background_of(stack);
    source regclass := xila.source_of(bg);
    found  jsonb;
    vals   jsonb;
    extras jsonb;
BEGIN
    IF move NOT IN ('first', 'last', 'at', 'next', 'previous') THEN
        RAISE EXCEPTION 'Unknown move: %.', move USING ERRCODE = 'XL002';
    END IF;

    IF source IS NULL THEN
        found := jsonb_build_object('_pos', 1, '_count', 1);
    ELSE
        EXECUTE format($q$
            WITH ordered AS (
                SELECT t.*, row_number() OVER (ORDER BY %1$s) AS _pos, count(*) OVER () AS _count
                FROM %2$s AS t),
            current AS (
                SELECT _pos FROM ordered WHERE %3$I::text = $1)
            SELECT to_jsonb(o)
            FROM ordered o
            WHERE o._pos = CASE $2
                WHEN 'first' THEN 1
                WHEN 'last' THEN o._count
                WHEN 'at' THEN coalesce((SELECT _pos FROM current), 1)
                WHEN 'next' THEN least(coalesce((SELECT _pos FROM current), 0) + 1, o._count)
                WHEN 'previous' THEN greatest(coalesce((SELECT _pos FROM current), 2) - 1, 1)
            END
            $q$, xila.card_order(bg), source, bg.key_column)
        INTO found USING key, move;
    END IF;

    IF found IS NULL THEN
        -- The stack has no card yet.
        RETURN jsonb_build_object(
            'key', NULL, 'position', 0, 'count', 0,
            'values', '{}'::jsonb, 'texts', '{}'::jsonb, 'widgets', '[]'::jsonb);
    END IF;

    -- Values travel as text, in the PostgreSQL text form of each type.
    SELECT coalesce(jsonb_object_agg(e.key, e.value #>> '{}'), '{}') INTO vals
    FROM jsonb_each(found - '_pos' - '_count') e;

    -- What a template may use: these names, then the columns of the card. A
    -- column wins over a name it shares.
    extras := jsonb_build_object(
        'position', found->'_pos',
        'count', found->'_count',
        'instance', xila.hello()->>'instance',
        'user', session_user) || vals;

    RETURN jsonb_build_object(
        'key', CASE WHEN source IS NOT NULL THEN vals->>bg.key_column END,
        'position', found->'_pos',
        'count', found->'_count',
        'values', vals,
        'texts', (
            SELECT coalesce(jsonb_object_agg(
                       w.name, xila.fill_template(w.props->>'template', extras)), '{}')
            FROM xila.widget w
            WHERE w.background_id = bg.id AND w.kind = 'text' AND w.props ? 'template'),
        'widgets', CASE WHEN source IS NULL THEN '[]'::jsonb
                        ELSE xila.widgets_json(bg, vals->>bg.key_column) END);
END
$$;

-- Saves a card. changes holds the modified columns only, as text. With a
-- NULL key, a new card is created. If two people edit the same card, the
-- last save wins; history keeps both versions.
CREATE FUNCTION xila.save_card(stack text, key text, changes jsonb) RETURNS jsonb
LANGUAGE plpgsql AS $$
DECLARE
    bg       xila.background := xila.background_of(stack);
    source   regclass := xila.source_of(bg);
    cleaned  jsonb := '{}';
    cols     text;
    change   record;
    col_type regtype;
    new_key  text := key;
    probe    text;
    bad_col  text;
BEGIN
    IF source IS NULL THEN
        RAISE EXCEPTION 'The stack “%” has no cards to edit.', stack
            USING ERRCODE = 'XL003';
    END IF;

    -- Check each value on its own first, to name the field that is wrong.
    FOR change IN SELECT e.key AS col, e.value #>> '{}' AS val FROM jsonb_each(changes) e LOOP
        SELECT a.atttypid::regtype INTO col_type
        FROM pg_attribute a
        WHERE a.attrelid = source AND a.attname = change.col
          AND a.attnum > 0 AND NOT a.attisdropped;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'The field “%” does not exist in this stack.', change.col
                USING ERRCODE = 'XL003', COLUMN = change.col;
        END IF;

        -- An empty input means "no value" for anything that is not text.
        IF change.val = '' AND col_type NOT IN ('text'::regtype, 'character varying'::regtype) THEN
            change.val := NULL;
        END IF;

        BEGIN
            EXECUTE format('SELECT CAST($1 AS %s)::text', col_type) INTO probe USING change.val;
        EXCEPTION WHEN data_exception THEN
            RAISE EXCEPTION 'The value “%” is not valid for the field “%”.',
                change.val, change.col
                USING ERRCODE = 'XL003', COLUMN = change.col;
        END;

        cleaned := cleaned || jsonb_build_object(change.col, change.val);
    END LOOP;

    SELECT string_agg(format('%I', k), ', ') INTO cols FROM jsonb_object_keys(cleaned) k;

    BEGIN
        IF key IS NULL THEN
            IF cols IS NULL THEN
                EXECUTE format('INSERT INTO %s DEFAULT VALUES RETURNING %I::text',
                               source, bg.key_column)
                INTO new_key;
            ELSE
                EXECUTE format(
                    'INSERT INTO %1$s (%2$s) SELECT %2$s FROM jsonb_populate_record(NULL::%1$s, $1)
                     RETURNING %3$I::text', source, cols, bg.key_column)
                INTO new_key USING cleaned;
            END IF;
        ELSIF cols IS NOT NULL THEN
            EXECUTE format(
                'UPDATE %1$s SET (%2$s) = (SELECT %2$s FROM jsonb_populate_record(NULL::%1$s, $1))
                 WHERE %3$I::text = $2 RETURNING %3$I::text', source, cols, bg.key_column)
            INTO new_key USING cleaned, key;
            IF new_key IS NULL THEN
                RAISE EXCEPTION 'This card no longer exists.' USING ERRCODE = 'XL002';
            END IF;
        END IF;
    EXCEPTION
        WHEN not_null_violation THEN
            GET STACKED DIAGNOSTICS bad_col = COLUMN_NAME;
            RAISE EXCEPTION 'The field “%” is required.', bad_col
                USING ERRCODE = 'XL003', COLUMN = bad_col;
        WHEN unique_violation THEN
            RAISE EXCEPTION 'Another card already holds this value.' USING ERRCODE = 'XL003';
        WHEN integrity_constraint_violation OR data_exception THEN
            RAISE EXCEPTION 'Value rejected: %', SQLERRM USING ERRCODE = 'XL003';
        WHEN insufficient_privilege THEN
            RAISE EXCEPTION 'You are not allowed to change this stack.'
                USING ERRCODE = 'XL004';
    END;

    RETURN xila.card(stack, new_key, 'at');
END
$$;

-- A page of rows of a grid widget.
CREATE FUNCTION xila.grid_rows(stack text, widget text, first integer DEFAULT 0,
                               count integer DEFAULT 100)
RETURNS jsonb
LANGUAGE plpgsql STABLE AS $$
DECLARE
    bg     xila.background := xila.background_of(stack);
    props  jsonb;
    rel    regclass;
    cols   text;
    result jsonb;
BEGIN
    SELECT w.props INTO props
    FROM xila.widget w
    WHERE w.background_id = bg.id AND w.name = widget AND w.kind = 'grid';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'The grid “%” does not exist.', widget USING ERRCODE = 'XL002';
    END IF;

    rel := to_regclass(format('%I.%I', props->>'schema', props->>'table'));
    IF rel IS NULL THEN
        RAISE EXCEPTION 'The source of the grid “%” does not exist.', widget
            USING ERRCODE = 'XL002';
    END IF;

    SELECT string_agg(format('%1$L, t.%1$I::text', c->>'column'), ', ') INTO cols
    FROM jsonb_array_elements(props->'columns') c;

    EXECUTE format($q$
        SELECT jsonb_build_object(
            'total', (SELECT count(*) FROM %1$s),
            'rows', coalesce((
                SELECT jsonb_agg(r.line ORDER BY r.n)
                FROM (
                    SELECT row_number() OVER (ORDER BY t.%2$I) AS n,
                           jsonb_build_object('key', t.%2$I::text,
                                              'cells', jsonb_build_object(%3$s)) AS line
                    FROM %1$s t
                    ORDER BY t.%2$I
                    OFFSET $1 LIMIT $2) r), '[]'))
        $q$, rel, props->>'key', cols)
    INTO result USING greatest(first, 0), greatest(count, 0);
    RETURN result;
END
$$;

-- ---------------------------------------------------------------------------
-- Actions
-- ---------------------------------------------------------------------------

-- Runs an action and answers with the effects the client must apply.
-- context is the state of the client: {"card": key, "dirty": {col: value},
-- "row": key of the selected grid row}.
CREATE FUNCTION xila.act(stack text, action text, context jsonb DEFAULT '{}') RETURNS jsonb
LANGUAGE plpgsql AS $$
DECLARE
    bg      xila.background := xila.background_of(stack);
    act     xila.action;
    card    text := context->>'card';
    dirty   jsonb := coalesce(context->'dirty', '{}');
    target  text;
    saved   jsonb;
BEGIN
    SELECT a.* INTO act
    FROM xila.action a
    WHERE a.name = action AND (a.background_id = bg.id OR a.background_id IS NULL)
    ORDER BY a.background_id NULLS LAST
    LIMIT 1;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'The action “%” does not exist.', action USING ERRCODE = 'XL002';
    END IF;

    CASE act.handler
    WHEN 'quit' THEN
        RETURN jsonb_build_array(jsonb_build_object('do', 'quit'));

    WHEN 'back' THEN
        RETURN jsonb_build_array(jsonb_build_object('do', 'back'));

    WHEN 'open_stack' THEN
        -- The stack named by the action, or else the selected grid row.
        target := coalesce(act.args->>'stack', context->>'row');
        IF target IS NULL THEN
            RAISE EXCEPTION 'No stack is selected.' USING ERRCODE = 'XL002';
        END IF;
        PERFORM xila.background_of(target);
        RETURN jsonb_build_array(jsonb_build_object('do', 'show', 'stack', target));

    WHEN 'new_card' THEN
        RETURN jsonb_build_array(jsonb_build_object('do', 'new'));

    WHEN 'save_card' THEN
        IF card IS NULL OR dirty <> '{}' THEN
            saved := xila.save_card(stack, card, dirty);
            RETURN jsonb_build_array(
                jsonb_build_object('do', 'show', 'stack', stack, 'card', saved->>'key'),
                jsonb_build_object('do', 'message', 'text', 'Card saved.'));
        END IF;
        RETURN jsonb_build_array(
            jsonb_build_object('do', 'message', 'text', 'Nothing to save.'));

    WHEN 'next_card', 'previous_card', 'first_card', 'last_card' THEN
        -- Leaving a card saves what was typed on it.
        IF dirty <> '{}' THEN
            saved := xila.save_card(stack, card, dirty);
            card := saved->>'key';
        END IF;
        target := xila.card(stack, card, split_part(act.handler, '_', 1))->>'key';
        RETURN jsonb_build_array(
            jsonb_build_object('do', 'show', 'stack', stack, 'card', target));
    END CASE;
END
$$;

GRANT SELECT ON xila.visible_stack TO xila_user;
