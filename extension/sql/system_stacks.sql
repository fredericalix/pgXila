-- System stacks: the screens pgXila itself is made of. They are ordinary
-- stacks, described with the same rows as the stacks users build.

-- Actions and keys available on every screen.
INSERT INTO xila.action (name, label, handler) VALUES
    ('quit', 'Quit', 'quit'),
    ('back', 'Back', 'back');

INSERT INTO xila.key_binding (key, action_id)
SELECT k.key, a.id
FROM (VALUES ('F3', 'quit'), ('F12', 'back')) AS k(key, action)
JOIN xila.action a ON a.name = k.action AND a.background_id IS NULL;

-- The desktop: where a session starts.
WITH s AS (
    INSERT INTO xila.stack (name, title, kind) VALUES ('desktop', 'Main menu', 'system')
    RETURNING id),
b AS (
    INSERT INTO xila.background (stack_id) SELECT id FROM s RETURNING id),
a AS (
    INSERT INTO xila.action (background_id, name, label, handler, args)
    SELECT id, 'open_stacks', 'My stacks', 'open_stack', '{"stack": "stacks"}' FROM b
    RETURNING id, background_id),
w AS (
    INSERT INTO xila.widget (background_id, name, kind, ord, label, action_id, props)
    SELECT a.background_id, v.name, v.kind, v.ord, v.label,
           CASE WHEN v.kind = 'button' THEN a.id END, v.props::jsonb
    FROM a, (VALUES
        ('banner',   'text',   10, NULL,          '{"template": "pgXila", "style": "title"}'),
        ('slogan',   'text',   20, NULL,          '{"template": "A fantasy OS inside PostgreSQL", "style": "subtitle"}'),
        ('instance', 'text',   30, 'Instance',    '{"template": "{instance}"}'),
        ('user',     'text',   40, 'User', '{"template": "{user}"}'),
        ('stacks',   'button', 50, 'My stacks',   '{}')
    ) AS v(name, kind, ord, label, props)
    RETURNING id, name, kind)
-- One presentation per renderer. The text screen is a plain numbered menu: its
-- header already says where and who we are. The window makes "My stacks" its
-- main button.
INSERT INTO xila.layout (widget_id, renderer, style)
SELECT id, 'text', '{"hidden": true}'::jsonb FROM w WHERE kind = 'text'
UNION ALL
SELECT id, 'graphic', '{"variant": "primary"}'::jsonb FROM w WHERE name = 'stacks';

-- The list of the stacks the user may open.
WITH s AS (
    INSERT INTO xila.stack (name, title, kind) VALUES ('stacks', 'My stacks', 'system')
    RETURNING id),
b AS (
    INSERT INTO xila.background (stack_id) SELECT id FROM s RETURNING id),
a AS (
    INSERT INTO xila.action (background_id, name, label, handler)
    SELECT id, 'open', 'Open', 'open_stack' FROM b
    RETURNING id, background_id)
INSERT INTO xila.widget (background_id, name, kind, ord, action_id, props)
SELECT background_id, 'list', 'grid', 10, id, '{
    "schema": "xila", "table": "visible_stack", "key": "name",
    "columns": [
        {"column": "title", "label": "Stack"},
        {"column": "cards", "label": "Cards", "align": "right"}
    ]}'
FROM a;
