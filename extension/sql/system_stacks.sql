-- System stacks: the screens pgXila itself is made of. They are ordinary
-- stacks, described with the same rows as the stacks users build.

-- Actions and keys available on every screen.
INSERT INTO xila.action (name, label, handler) VALUES
    ('quit', 'Quitter', 'quit'),
    ('back', 'Retour', 'back');

INSERT INTO xila.key_binding (key, action_id)
SELECT k.key, a.id
FROM (VALUES ('F3', 'quit'), ('F12', 'back')) AS k(key, action)
JOIN xila.action a ON a.name = k.action AND a.background_id IS NULL;

-- The desktop: where a session starts.
WITH s AS (
    INSERT INTO xila.stack (name, title, kind) VALUES ('desktop', 'Menu principal', 'system')
    RETURNING id),
b AS (
    INSERT INTO xila.background (stack_id) SELECT id FROM s RETURNING id),
a AS (
    INSERT INTO xila.action (background_id, name, label, handler, args)
    SELECT id, 'open_stacks', 'Mes piles', 'open_stack', '{"stack": "stacks"}' FROM b
    RETURNING id, background_id)
INSERT INTO xila.widget (background_id, name, kind, ord, label, action_id)
SELECT background_id, 'stacks', 'button', 10, 'Mes piles', id FROM a;

-- The list of the stacks the user may open.
WITH s AS (
    INSERT INTO xila.stack (name, title, kind) VALUES ('stacks', 'Mes piles', 'system')
    RETURNING id),
b AS (
    INSERT INTO xila.background (stack_id) SELECT id FROM s RETURNING id),
a AS (
    INSERT INTO xila.action (background_id, name, label, handler)
    SELECT id, 'open', 'Ouvrir', 'open_stack' FROM b
    RETURNING id, background_id)
INSERT INTO xila.widget (background_id, name, kind, ord, action_id, props)
SELECT background_id, 'list', 'grid', 10, id, '{
    "schema": "xila", "table": "visible_stack", "key": "name",
    "columns": [
        {"column": "title", "label": "Pile"},
        {"column": "cards", "label": "Cartes", "align": "right"}
    ]}'
FROM a;
