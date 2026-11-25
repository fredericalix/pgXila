-- pgXila actions: what a button or a key does. See docs/api.md, section 5.
--
-- This file comes after author.sql, whose functions and views the author
-- handlers use.

-- Runs an action and answers with the effects the client must apply.
-- context is the state of the client: {"card": key, "dirty": {col: value},
-- "row": key of the selected grid row, "confirmed": true once the user has
-- accepted the question of a "confirm" effect}.
--
-- On the author screens, which are cards of views, the key of the card is the
-- subject of the dialog: the name of a stack, the identifier of a field.
CREATE FUNCTION xila.act(stack text, action text, context jsonb DEFAULT '{}') RETURNS jsonb
LANGUAGE plpgsql AS $$
DECLARE
    bg      xila.background := xila.background_of(stack);
    source  regclass := xila.source_of(bg);
    info    xila.stack;
    act     xila.action;
    card    text := context->>'card';
    dirty   jsonb := coalesce(context->'dirty', '{}');
    picked  text := context->>'row';
    agreed  boolean := coalesce((context->>'confirmed')::boolean, false);
    target  text;
    saved   jsonb;
    field   xila.field;
    edited  xila.stack_info;
    entry   xila.history;
    scope   text;
    shown   text;
    n       bigint;
BEGIN
    SELECT s.* INTO info FROM xila.stack s WHERE s.id = bg.stack_id;

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
        target := coalesce(act.args->>'stack', picked);
        IF target IS NULL THEN
            RAISE EXCEPTION 'No stack is selected.' USING ERRCODE = 'XL002';
        END IF;
        PERFORM xila.background_of(target);
        RETURN jsonb_build_array(xila.fx_show(target));

    WHEN 'new_card' THEN
        RETURN jsonb_build_array(jsonb_build_object('do', 'new'));

    WHEN 'save_card' THEN
        IF card IS NULL OR dirty <> '{}' THEN
            saved := xila.save_card(stack, card, dirty);
            RETURN jsonb_build_array(
                xila.fx_show(stack, saved->>'key'), xila.fx_message('Card saved.'));
        END IF;
        RETURN jsonb_build_array(xila.fx_message('Nothing to save.'));

    WHEN 'next_card', 'previous_card', 'first_card', 'last_card' THEN
        -- Leaving a card saves what was typed on it.
        IF dirty <> '{}' THEN
            saved := xila.save_card(stack, card, dirty);
            card := saved->>'key';
        END IF;
        target := xila.card(stack, card, split_part(act.handler, '_', 1))->>'key';
        RETURN jsonb_build_array(xila.fx_show(stack, target));

    WHEN 'delete_card' THEN
        IF source IS NULL THEN
            RAISE EXCEPTION 'This stack has no cards.' USING ERRCODE = 'XL003';
        END IF;
        IF card IS NULL THEN
            -- A card that was never saved: there is nothing to delete.
            RETURN jsonb_build_array(xila.fx_show(stack));
        END IF;
        IF NOT agreed THEN
            RETURN xila.fx_confirm('Delete this card?', 'Delete');
        END IF;
        -- The card to show afterwards: the next one, or else the one before.
        target := xila.card(stack, card, 'next')->>'key';
        IF target = card THEN
            target := xila.card(stack, card, 'previous')->>'key';
        END IF;
        BEGIN
            EXECUTE format('DELETE FROM %s WHERE %I::text = $1', source, bg.key_column)
            USING card;
        EXCEPTION
            WHEN insufficient_privilege THEN
                RAISE EXCEPTION 'You are not allowed to change this stack.'
                    USING ERRCODE = 'XL004';
            WHEN foreign_key_violation THEN
                RAISE EXCEPTION 'This card is used elsewhere and cannot be deleted.'
                    USING ERRCODE = 'XL003';
        END;
        RETURN jsonb_build_array(
            xila.fx_show(stack, nullif(target, card)), xila.fx_message('Card deleted.'));

    -- Author mode ---------------------------------------------------------

    WHEN 'author' THEN
        -- On a system screen, the selected row names the stack.
        target := CASE WHEN info.kind = 'system' THEN picked ELSE stack END;
        IF target IS NULL THEN
            RAISE EXCEPTION 'No stack is selected.' USING ERRCODE = 'XL002';
        END IF;
        PERFORM xila.editable(target);
        RETURN jsonb_build_array(xila.fx_show('author', target));

    WHEN 'browse' THEN
        IF card IS NULL THEN
            RAISE EXCEPTION 'No stack is selected.' USING ERRCODE = 'XL002';
        END IF;
        RETURN jsonb_build_array(xila.fx_show(card));

    WHEN 'new_stack' THEN
        IF NOT pg_has_role('xila_author', 'USAGE') THEN
            RAISE EXCEPTION 'Only authors can create stacks.' USING ERRCODE = 'XL004';
        END IF;
        RETURN jsonb_build_array(
            xila.fx_show('new_stack'), jsonb_build_object('do', 'new'));

    WHEN 'create_stack' THEN
        IF nullif(dirty->>'source', '') IS NOT NULL
           AND to_regclass(dirty->>'source') IS NULL THEN
            RAISE EXCEPTION 'The table “%” does not exist.', dirty->>'source'
                USING ERRCODE = 'XL002', COLUMN = 'source';
        END IF;
        target := xila.create_stack(dirty->>'title', dirty->>'name',
                                    to_regclass(nullif(dirty->>'source', '')));
        RETURN jsonb_build_array(
            jsonb_build_object('do', 'back'),
            xila.fx_show('author', target),
            xila.fx_message('Stack created. F6 adds a field.'));

    WHEN 'save_stack' THEN
        IF dirty = '{}' THEN
            RETURN jsonb_build_array(xila.fx_message('Nothing to save.'));
        END IF;
        PERFORM xila.set_stack(card, dirty);
        RETURN jsonb_build_array(xila.fx_show(stack, card), xila.fx_message('Saved.'));

    WHEN 'delete_stack' THEN
        PERFORM xila.editable(card);
        SELECT i.* INTO edited FROM xila.stack_info i WHERE i.name = card;
        IF NOT agreed THEN
            RETURN xila.fx_confirm(format(
                'Delete the stack “%s”? Its cards are kept, and the stack can be restored.',
                edited.title), 'Delete');
        END IF;
        PERFORM xila.delete_stack(card);
        RETURN jsonb_build_array(
            jsonb_build_object('do', 'back'),
            xila.fx_message(format(
                'Stack “%s” deleted. F10 in “My stacks” lists the deleted stacks.',
                edited.title)));

    WHEN 'new_field' THEN
        PERFORM xila.editable(card);
        RETURN jsonb_build_array(
            xila.fx_show('field'),
            jsonb_build_object('do', 'new',
                               'values', jsonb_build_object('stack', card, 'type', 'text')));

    WHEN 'edit_field' THEN
        IF picked IS NULL THEN
            RAISE EXCEPTION 'No field is selected.' USING ERRCODE = 'XL002';
        END IF;
        RETURN jsonb_build_array(xila.fx_show('field', picked));

    WHEN 'save_field' THEN
        IF card IS NULL THEN
            target := xila.add_field(dirty->>'stack', dirty->>'label', dirty->>'type',
                                     dirty->>'name');
            IF coalesce((nullif(dirty->>'required', ''))::boolean, false) THEN
                PERFORM xila.set_field(dirty->>'stack', target, '{"required": true}');
            END IF;
            RETURN jsonb_build_array(
                jsonb_build_object('do', 'back'),
                xila.fx_message(format('Field “%s” added.', btrim(dirty->>'label'))));
        END IF;
        SELECT f.* INTO field FROM xila.field f WHERE f.id::text = card;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'This field no longer exists.' USING ERRCODE = 'XL002';
        END IF;
        IF dirty ? 'type' AND dirty->>'type' <> field.type THEN
            RAISE EXCEPTION 'The type of a field cannot be changed. Add another field instead.'
                USING ERRCODE = 'XL003', COLUMN = 'type';
        END IF;
        IF dirty - 'type' - 'stack' = '{}' THEN
            RETURN jsonb_build_array(jsonb_build_object('do', 'back'));
        END IF;
        PERFORM xila.set_field(field.stack, field.name, dirty - 'type' - 'stack');
        RETURN jsonb_build_array(
            jsonb_build_object('do', 'back'), xila.fx_message('Field saved.'));

    WHEN 'remove_field' THEN
        SELECT f.* INTO field FROM xila.field f WHERE f.id::text = card;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'This field is not saved yet.' USING ERRCODE = 'XL002';
        END IF;
        bg := xila.editable(field.stack);
        IF NOT agreed THEN
            IF NOT bg.managed THEN
                RETURN xila.fx_confirm(format(
                    'Take the field “%s” off the screen? The table is not changed.',
                    field.label), 'Remove');
            END IF;
            EXECUTE format('SELECT count(*) FROM %s WHERE %I IS NOT NULL',
                           xila.source_of(bg), field.name)
            INTO n;
            RETURN xila.fx_confirm(format(
                'Remove the field “%s”? %s. History can bring it back.', field.label,
                CASE n WHEN 0 THEN 'No card has a value for it'
                       WHEN 1 THEN '1 card has a value for it'
                       ELSE n || ' cards have a value for it' END), 'Remove');
        END IF;
        PERFORM xila.remove_field(field.stack, field.name);
        RETURN jsonb_build_array(
            jsonb_build_object('do', 'back'),
            xila.fx_message(format('Field “%s” removed.', field.label)));

    WHEN 'move_field' THEN
        SELECT f.* INTO field FROM xila.field f WHERE f.id::text = picked;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'No field is selected.' USING ERRCODE = 'XL002';
        END IF;
        PERFORM xila.move_field(field.stack, field.name, (act.args->>'by')::integer);
        RETURN jsonb_build_array(
            jsonb_build_object('do', 'reload'),
            jsonb_build_object('do', 'select', 'row', picked));

    WHEN 'history' THEN
        RETURN jsonb_build_array(CASE act.args->>'of'
            WHEN 'deleted' THEN xila.fx_show('deleted_stacks')
            WHEN 'cards' THEN xila.fx_show('card_history', stack)
            ELSE xila.fx_show('history', card)
        END);

    WHEN 'rewind' THEN
        scope := act.args->>'scope';
        IF picked IS NULL OR picked !~ '^[0-9]+$' THEN
            RAISE EXCEPTION 'Nothing is selected.' USING ERRCODE = 'XL002';
        END IF;

        IF scope = 'deleted' THEN
            SELECT d.name, d.title INTO target, shown
            FROM xila.deleted_stack d
            WHERE d.id = picked::bigint;
            IF NOT FOUND THEN
                RAISE EXCEPTION 'This stack is no longer in the list.' USING ERRCODE = 'XL002';
            END IF;
            IF NOT agreed THEN
                RETURN xila.fx_confirm(format('Bring back the stack “%s”?', shown),
                                       'Restore');
            END IF;
            PERFORM xila.rewind(target, 'stack', picked::bigint);
            RETURN jsonb_build_array(
                jsonb_build_object('do', 'reload'),
                xila.fx_message(format('Stack “%s” restored.', shown)));
        END IF;

        -- The history dialogs are about one stack: the card on screen.
        SELECT i.* INTO edited FROM xila.stack_info i WHERE i.name = card;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'This stack no longer exists.' USING ERRCODE = 'XL002';
        END IF;
        SELECT h.* INTO entry
        FROM xila.history h JOIN xila.stack s ON s.id = h.stack_id
        WHERE h.id = picked::bigint AND s.name = card;
        IF NOT FOUND OR (scope = 'card' AND entry.object_kind <> 'card') THEN
            RAISE EXCEPTION 'This change is no longer in the list.' USING ERRCODE = 'XL002';
        END IF;

        SELECT count(*) INTO n
        FROM xila.rewind_entries(entry.stack_id, scope, entry.id,
                                 CASE WHEN scope = 'card' THEN entry.object_key END);
        IF n = 0 THEN
            RETURN jsonb_build_array(xila.fx_message(
                'There is nothing to undo from there: only cards were changed since.'));
        END IF;
        IF NOT agreed THEN
            RETURN xila.fx_confirm(CASE scope
                WHEN 'card' THEN format(
                    'Undo %s to this card, back to before %s?',
                    xila.counted(n, 'change', 'changes'),
                    to_char(entry.at, 'YYYY-MM-DD HH24:MI'))
                WHEN 'screen' THEN format(
                    'Undo %s to the screen of “%s”? The cards keep their other values.',
                    xila.counted(n, 'change', 'changes'), edited.title)
                ELSE format(
                    'Undo %s to “%s”, screen and cards, back to before %s?',
                    xila.counted(n, 'change', 'changes'), edited.title,
                    to_char(entry.at, 'YYYY-MM-DD HH24:MI'))
            END, 'Rewind');
        END IF;
        saved := xila.rewind(card, scope, entry.id,
                             CASE WHEN scope = 'card' THEN entry.object_key END);
        RETURN jsonb_build_array(
            jsonb_build_object('do', 'back'),
            xila.fx_message(xila.counted((saved->>'undone')::bigint,
                                         'change undone.', 'changes undone.')));
    END CASE;
END
$$;
