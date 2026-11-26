-- Scripts: what the Lua engine of the extension (xila.check_script and
-- xila.run_script) needs from SQL, and how a script is saved and tested.

-- ---------------------------------------------------------------------------
-- What a running script asks for
-- ---------------------------------------------------------------------------
-- Clients carry the values of a card as text. A script gets them typed: a
-- number is a number, a boolean a boolean, nothing is nil.

-- Values given as text, with the type of their column. A value that is not
-- valid for its column stays the text that was typed.
CREATE FUNCTION xila.typed_values(source regclass, vals jsonb) RETURNS jsonb
LANGUAGE plpgsql STABLE AS $$
DECLARE
    result   jsonb := '{}';
    v        record;
    col_type regtype;
    typed    jsonb;
BEGIN
    FOR v IN SELECT e.key AS col, e.value #>> '{}' AS val FROM jsonb_each(vals) e LOOP
        SELECT a.atttypid::regtype INTO col_type
        FROM pg_attribute a
        WHERE a.attrelid = source AND a.attname = v.col AND a.attnum > 0 AND NOT a.attisdropped;
        IF NOT FOUND THEN
            result := result || jsonb_build_object(v.col, v.val);
            CONTINUE;
        END IF;
        -- An empty input means "no value" for anything that is not text.
        IF v.val = '' AND col_type NOT IN ('text'::regtype, 'character varying'::regtype) THEN
            v.val := NULL;
        END IF;
        BEGIN
            EXECUTE format('SELECT to_jsonb(CAST($1 AS %s))', col_type) INTO typed USING v.val;
        EXCEPTION WHEN data_exception THEN
            typed := to_jsonb(v.val);
        END;
        result := result || jsonb_build_object(v.col, typed);
    END LOOP;
    RETURN result;
END
$$;

-- One card as a script sees it: its key and its typed values. NULL when the
-- card does not exist.
CREATE FUNCTION xila.script_card(bg xila.background, key text) RETURNS jsonb
LANGUAGE plpgsql STABLE AS $$
DECLARE
    source regclass := xila.source_of(bg);
    found  jsonb;
BEGIN
    EXECUTE format('SELECT to_jsonb(t) FROM %s AS t WHERE t.%I::text = $1', source, bg.key_column)
    INTO found USING key;
    RETURN CASE WHEN found IS NOT NULL
                THEN jsonb_build_object('key', key, 'values', found) END;
END
$$;

-- What a script starts from: who runs it, where, and the card on screen: its
-- saved values, what the user has typed on it since ("typed") and the fields
-- the user has emptied ("cleared"). A card that is not saved yet has no key.
CREATE FUNCTION xila.script_scene(stack text, context jsonb) RETURNS jsonb
LANGUAGE plpgsql STABLE AS $$
DECLARE
    bg     xila.background := xila.background_of(stack);
    source regclass := xila.source_of(bg);
    shown  jsonb;
    typed  jsonb;
BEGIN
    IF source IS NOT NULL THEN
        IF NOT xila.may_read(source) THEN
            RAISE EXCEPTION 'You do not have access to the stack “%”.', stack
                USING ERRCODE = 'XL004';
        END IF;
        shown := coalesce(
            CASE WHEN context->>'card' IS NOT NULL
                 THEN xila.script_card(bg, context->>'card') END,
            jsonb_build_object('key', NULL, 'values', '{}'::jsonb));
        typed := xila.typed_values(source, coalesce(context->'dirty', '{}'));
        shown := shown || jsonb_build_object(
            'typed', jsonb_strip_nulls(typed),
            'cleared', (SELECT coalesce(jsonb_agg(e.key), '[]')
                        FROM jsonb_each(typed) e WHERE e.value = 'null'));
    END IF;
    RETURN jsonb_build_object(
        'user', current_user,
        'instance', xila.hello()->>'instance',
        'title', (SELECT s.title FROM xila.stack s WHERE s.id = bg.stack_id),
        'card', shown);
END
$$;

-- Saves a card for a script, and gives it back as the script sees it.
-- (Two statements: the card is read once it has been written.)
CREATE FUNCTION xila.script_save(stack text, key text, changes jsonb) RETURNS jsonb
LANGUAGE plpgsql AS $$
DECLARE
    saved text;
BEGIN
    saved := xila.save_card(stack, key, changes)->>'key';
    RETURN xila.script_card(xila.background_of(stack), saved);
END
$$;

-- Leaves the card on screen for another one, as the keys of a stack do: what
-- was typed is saved first. Answers with the key of the card to show.
CREATE FUNCTION xila.script_move(stack text, key text, typed jsonb, move text) RETURNS text
LANGUAGE plpgsql AS $$
DECLARE
    card text := key;
BEGIN
    IF move NOT IN ('first', 'last', 'next', 'previous') THEN
        RAISE EXCEPTION 'Unknown move: %.', move USING ERRCODE = 'XL002';
    END IF;
    IF coalesce(typed, '{}') <> '{}' THEN
        card := xila.save_card(stack, card, typed)->>'key';
    END IF;
    RETURN xila.card(stack, card, move)->>'key';
END
$$;

-- The cards whose fields equal the values of filter, in the order of the
-- stack; all of them when filter is empty.
CREATE FUNCTION xila.script_cards(stack text, filter jsonb) RETURNS jsonb
LANGUAGE plpgsql STABLE AS $$
DECLARE
    bg      xila.background := xila.background_of(stack);
    source  regclass := xila.source_of(bg);
    missing text;
    wanted  text;
    found   jsonb;
BEGIN
    IF source IS NULL THEN
        RAISE EXCEPTION 'This stack has no cards.' USING ERRCODE = 'XL003';
    END IF;
    IF NOT xila.may_read(source) THEN
        RAISE EXCEPTION 'You do not have access to the stack “%”.', stack USING ERRCODE = 'XL004';
    END IF;

    SELECT k INTO missing
    FROM jsonb_object_keys(filter) k
    WHERE NOT EXISTS (
        SELECT FROM pg_attribute a
        WHERE a.attrelid = source AND a.attname = k AND a.attnum > 0 AND NOT a.attisdropped)
    LIMIT 1;
    IF missing IS NOT NULL THEN
        RAISE EXCEPTION 'The field “%” does not exist in this stack.', missing
            USING ERRCODE = 'XL003';
    END IF;

    -- The values of the filter take the types of their columns.
    SELECT coalesce(string_agg(format(
               't.%1$I IS NOT DISTINCT FROM (SELECT f.%1$I FROM jsonb_populate_record(NULL::%2$s, $1) AS f)',
               k, source), ' AND '), 'true')
    INTO wanted
    FROM jsonb_object_keys(filter) k;

    EXECUTE format($q$
        SELECT coalesce(jsonb_agg(jsonb_build_object('key', c.key, 'values', c.vals)
                                  ORDER BY c.n), '[]')
        FROM (
            SELECT t.%1$I::text AS key, to_jsonb(t) AS vals,
                   row_number() OVER (ORDER BY %2$s) AS n
            FROM %3$s AS t
            WHERE %4$s
            ORDER BY n
            LIMIT 1001) c
        $q$, bg.key_column, xila.card_order(bg), source, wanted)
    INTO found USING filter;

    IF jsonb_array_length(found) > 1000 THEN
        RAISE EXCEPTION 'More than 1000 cards match: narrow the search.' USING ERRCODE = 'XL003';
    END IF;
    RETURN found;
END
$$;

-- ---------------------------------------------------------------------------
-- Saving a script
-- ---------------------------------------------------------------------------

-- How a script is named to its author: by the button it belongs to.
CREATE FUNCTION xila.script_owner(w xila.widget) RETURNS text
LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE WHEN w.id IS NULL THEN 'the screen'
                ELSE '“' || coalesce(nullif(w.label, ''), w.name) || '”' END
$$;

-- Checks and saves the script of a button, or of the screen when widget is
-- null. An empty source removes it. Answers as xila.check_script does.
--
-- A script that has an error is saved too, so that no work is lost; it
-- handles nothing until it is fixed.
CREATE FUNCTION xila.set_script(stack text, widget text, source text,
                                card text DEFAULT NULL)
RETURNS jsonb
LANGUAGE plpgsql AS $$
DECLARE
    bg      xila.background := xila.editable(stack);
    w       xila.widget;
    checked jsonb;
    problem text;
    taken   record;
BEGIN
    IF widget IS NOT NULL THEN
        w := xila.widget_named(bg, widget, card);
        IF w.kind <> 'button' THEN
            RAISE EXCEPTION 'Only a button, or the screen, has a script.' USING ERRCODE = 'XL003';
        END IF;
    END IF;

    IF btrim(coalesce(set_script.source, ''), E' \t\r\n') = '' THEN
        DELETE FROM xila.script s
        WHERE s.background_id = bg.id AND s.widget_id IS NOT DISTINCT FROM w.id;
        RETURN jsonb_build_object('ok', true, 'removed', true,
                                  'events', '[]'::jsonb, 'keys', '{}'::jsonb);
    END IF;

    checked := xila.check_script(set_script.source);
    IF (checked->>'ok')::boolean THEN
        -- Each event has its place, and a key belongs to one script only.
        SELECT e.key, o.id AS other INTO taken
        FROM jsonb_each(checked->'keys') e
        JOIN xila.script s ON s.background_id = bg.id AND s.keys ? e.key
                          AND s.widget_id IS DISTINCT FROM w.id
        LEFT JOIN xila.widget o ON o.id = s.widget_id
        LIMIT 1;
        problem := CASE
            WHEN w.id IS NULL AND checked->'events' ? 'click'
                THEN 'on_click belongs to the script of a button.'
            WHEN w.id IS NOT NULL AND checked->'events' ? 'open'
                THEN 'on_open belongs to the script of the screen.'
            WHEN w.card_key IS NOT NULL AND checked->'keys' <> '{}'
                THEN 'A button that belongs to one card cannot take keys.'
            WHEN taken.key IS NOT NULL
                THEN format('%s already belongs to the script of %s.', taken.key,
                            xila.script_owner((SELECT o FROM xila.widget o WHERE o.id = taken.other)))
        END;
        IF problem IS NOT NULL THEN
            checked := jsonb_build_object('ok', false, 'line', NULL, 'message', problem);
        END IF;
    END IF;

    UPDATE xila.script s
    SET source = set_script.source,
        events = CASE WHEN (checked->>'ok')::boolean
                      THEN ARRAY(SELECT jsonb_array_elements_text(checked->'events'))
                      ELSE '{}' END,
        keys = CASE WHEN (checked->>'ok')::boolean THEN checked->'keys' ELSE '{}' END
    WHERE s.background_id = bg.id AND s.widget_id IS NOT DISTINCT FROM w.id;
    IF NOT FOUND THEN
        INSERT INTO xila.script (background_id, widget_id, source, events, keys)
        VALUES (bg.id, w.id, set_script.source,
                CASE WHEN (checked->>'ok')::boolean
                     THEN ARRAY(SELECT jsonb_array_elements_text(checked->'events'))
                     ELSE '{}' END,
                CASE WHEN (checked->>'ok')::boolean THEN checked->'keys' ELSE '{}' END);
    END IF;
    RETURN checked;
END
$$;

-- ---------------------------------------------------------------------------
-- Running and testing
-- ---------------------------------------------------------------------------

-- The message a user reads when a script fails.
CREATE FUNCTION xila.script_failure(owner text, problem jsonb) RETURNS text
LANGUAGE sql IMMUTABLE AS $$
    SELECT format('The script of %s failed%s: %s', owner,
                  coalesce(' at line ' || (problem->>'line'), ''),
                  problem->>'message')
$$;

-- One effect, as a line of the test report.
CREATE FUNCTION xila.describe_effect(effect jsonb) RETURNS text
LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE effect->>'do'
        WHEN 'show' THEN CASE
            WHEN effect->>'card' IS NOT NULL THEN 'It shows a card.'
            ELSE format('It opens the stack “%s”.', effect->>'stack') END
        WHEN 'new' THEN 'It shows a new card' || coalesce((
            SELECT ' with ' || string_agg(format('%s = %s', v.key, v.value), ', ' ORDER BY v.key)
            FROM jsonb_each_text(effect->'values') v), '') || '.'
        WHEN 'set' THEN 'It writes ' || coalesce((
            SELECT string_agg(format('%s = %s', v.key, coalesce(v.value, 'nothing')), ', '
                              ORDER BY v.key)
            FROM jsonb_each_text(effect->'values') v), 'nothing') || ' on the card.'
        WHEN 'focus' THEN format('It puts the cursor in “%s”.', effect->>'widget')
        WHEN 'message' THEN format('It says: %s', effect->>'text')
        WHEN 'back' THEN 'It goes back to the previous screen.'
        WHEN 'reload' THEN 'It reads the screen again.'
        WHEN 'quit' THEN 'It leaves pgXila.'
        ELSE 'It opens a tool of the client.'
    END
$$;

-- Runs one event of a script as it is typed, saved or not, and undoes
-- everything it wrote. Answers with what it would have done, in words:
-- {"ok": true, "lines": ["It shows a new card with city = London.", …]}, or
-- {"ok": false, "line": 4, "lines": ["attempt to …"]}.
CREATE FUNCTION xila.test_script(stack text, source text, event jsonb, context jsonb)
RETURNS jsonb
LANGUAGE plpgsql AS $$
DECLARE
    lines   text[] := '{}';
    played  jsonb;
    wrote   text;
BEGIN
    played := xila.run_script(stack, test_script.source, event,
                              context || '{"confirmed": false}'::jsonb, false);
    IF coalesce((played->>'asked')::boolean, false) THEN
        lines := lines || format('It asks: %s (%s)', played->'effects'->0->>'text',
                                 played->'effects'->0->>'label')
                       || 'Once accepted:'::text;
        played := xila.run_script(stack, test_script.source, event,
                                  context || '{"confirmed": true}'::jsonb, false);
    END IF;

    lines := lines || ARRAY(
        SELECT 'print: ' || p FROM jsonb_array_elements_text(played->'printed') WITH ORDINALITY AS t(p, n)
        ORDER BY n);

    IF played ? 'error' THEN
        RETURN jsonb_build_object(
            'ok', false,
            'line', played->'error'->'line',
            'lines', to_jsonb(lines || (
                coalesce('Line ' || (played->'error'->>'line') || ': ', '')
                || (played->'error'->>'message'))));
    END IF;

    SELECT string_agg(part, ', ') INTO wrote
    FROM (VALUES
        (CASE WHEN (played->'wrote'->>'saved')::bigint > 0
              THEN xila.counted((played->'wrote'->>'saved')::bigint, 'card saved', 'cards saved') END),
        (CASE WHEN (played->'wrote'->>'deleted')::bigint > 0
              THEN xila.counted((played->'wrote'->>'deleted')::bigint,
                                'card deleted', 'cards deleted') END),
        (CASE WHEN (played->'wrote'->>'rows')::bigint > 0
              THEN xila.counted((played->'wrote'->>'rows')::bigint,
                                'row written', 'rows written') END)) AS w(part);
    IF wrote IS NOT NULL THEN
        lines := lines || ('In the database: ' || wrote || '.');
    END IF;

    lines := lines || ARRAY(
        SELECT xila.describe_effect(e)
        FROM jsonb_array_elements(played->'effects') WITH ORDINALITY AS t(e, n)
        ORDER BY n);

    IF wrote IS NULL AND jsonb_array_length(played->'effects') = 0 THEN
        lines := lines || 'It does nothing.'::text;
    ELSIF wrote IS NOT NULL THEN
        lines := lines || 'This was a test: nothing was kept.'::text;
    END IF;
    RETURN jsonb_build_object('ok', true, 'line', NULL, 'lines', to_jsonb(lines));
END
$$;
