-- Example stack: an address book.
--
-- This is NOT part of pgXila. pgXila ships the tool to build stacks, not
-- ready-made applications. This file builds the kind of stack a user would
-- build in author mode, with nothing but ordinary SQL: a table, then rows in
-- the xila tables. It is used as a test fixture and as a demonstration.
--
-- Load it with scripts/load-example.sh addressbook. Loading it again replaces
-- it.
-- All names and values are fictitious.

DELETE FROM xila.stack WHERE name = 'addressbook';
-- The portraits of a previous load live in xila.image.
DO $$
BEGIN
    IF to_regclass('addressbook.contacts') IS NOT NULL THEN
        DELETE FROM xila.image WHERE id IN (SELECT photo FROM addressbook.contacts);
    END IF;
END
$$;
DROP SCHEMA IF EXISTS addressbook CASCADE;

-- The data: one row per card.
CREATE SCHEMA addressbook;

CREATE TABLE addressbook.contacts (
    id         uuid PRIMARY KEY DEFAULT uuidv7(),
    last_name  text NOT NULL,
    first_name text,
    company    text,
    phone      text,
    email      text,
    city       text,
    birthday   date,
    -- A picture is a row of xila.image: one byte per pixel, palette slots.
    photo      uuid REFERENCES xila.image ON DELETE SET NULL
);

INSERT INTO addressbook.contacts (last_name, first_name, company, phone, email, city) VALUES
    ('Babbage',   'Charles',  'Difference Works',     '01 23 45 67 01', 'charles@example.org',  'London'),
    ('Hopper',    'Grace',    'United Compilers',     '01 23 45 67 02', 'grace@example.org',    'New York'),
    ('Lovelace',  'Ada',      'Analytical Engines',   '01 23 45 67 89', 'ada@example.org',      'London'),
    ('Turing',    'Alan',     'Bletchley Consulting', '01 23 45 67 03', 'alan@example.org',     'Manchester'),
    ('Hamilton',  'Margaret', 'Apollo Software',      '01 23 45 67 04', 'margaret@example.org', 'Boston'),
    ('Pascal',    'Blaise',   'La Pascaline',         '01 23 45 67 05', 'blaise@example.org',   'Clermont-Ferrand'),
    ('Jacquard',  'Joseph',   'Jacquard Looms',       '01 23 45 67 06', 'joseph@example.org',   'Lyon'),
    ('Shannon',   'Claude',   'Signal and Noise',     '01 23 45 67 07', 'claude@example.org',   'Gaylord'),
    ('Lamarr',    'Hedy',     'Frequency Hopping',    '01 23 45 67 08', 'hedy@example.org',     'Vienna'),
    ('Dijkstra',  'Edsger',   'Shortest Path',        '01 23 45 67 09', 'edsger@example.org',   'Rotterdam'),
    ('Liskov',    'Barbara',  'Substitution Ltd',     '01 23 45 67 10', 'barbara@example.org',  'Los Angeles'),
    ('Pouzin',    'Louis',    'Cyclades',             '01 23 45 67 11', 'louis@example.org',    'Chantenay-Saint-Imbert');

-- The portraits. A user would draw them in author mode; here they are
-- computed: the same face for everyone, with its own hair, skin and clothes.
-- The result is an indexed image of 68 by 80 pixels. The arguments are
-- palette slots.
CREATE FUNCTION addressbook.portrait(hair integer, skin integer, clothes integer,
                                bun boolean, long_hair boolean)
RETURNS bytea
LANGUAGE sql IMMUTABLE AS $$
    WITH grid AS (
        SELECT x, y, x - 33.5 AS dx
        FROM generate_series(0, 79) AS y, generate_series(0, 67) AS x),
    parts AS (
        -- What each pixel belongs to: 0 background, 1 bun, 2 hair, 3 skin,
        -- 4 clothes, 5 collar.
        SELECT x, y, CASE
            WHEN y BETWEEN 62 AND 68 AND abs(dx) <= (68 - y) * 0.9 THEN 5
            WHEN power(dx / 26, 2) + power((y - 86) / 27.0, 2) <= 1 THEN 4
            WHEN abs(dx) <= 5 AND y BETWEEN 50 AND 60 THEN 3
            WHEN abs(dx) <= 11.5 AND y BETWEEN 30 AND 43 THEN 3
            WHEN y > 43 AND power(dx / 11.5, 2) + power((y - 43) / 8.5, 2) <= 1 THEN 3
            WHEN power(dx / 16.5, 2) + power((y - 32) / 17.5, 2) <= 1
                 AND (long_hair OR y < 37) THEN 2
            WHEN bun AND power(dx / 7.5, 2) + power((y - 10) / 6.5, 2) <= 1 THEN 1
            ELSE 0
        END AS part
        FROM grid),
    outlined AS (
        -- A pixel next to another part is on an outline.
        SELECT x, y, part,
               part <> coalesce(lag(part)  OVER across, part)
            OR part <> coalesce(lead(part) OVER across, part)
            OR part <> coalesce(lag(part)  OVER down, part)
            OR part <> coalesce(lead(part) OVER down, part) AS outline
        FROM parts
        WINDOW across AS (PARTITION BY y ORDER BY x), down AS (PARTITION BY x ORDER BY y)),
    coloured AS (
        SELECT x, y, CASE
            -- Eyes with a glint, cheeks, mouth.
            WHEN y = 35 AND x IN (28, 37) THEN 7
            WHEN y BETWEEN 35 AND 37 AND (x BETWEEN 28 AND 30 OR x BETWEEN 37 AND 39) THEN 0
            WHEN y BETWEEN 40 AND 41 AND (x BETWEEN 25 AND 27 OR x BETWEEN 40 AND 42) THEN 27
            WHEN y = 44 AND x BETWEEN 31 AND 36 THEN 26
            WHEN outline THEN 0
            WHEN part = 5 THEN 7
            WHEN part = 4 THEN clothes
            WHEN part = 3 THEN skin
            WHEN part IN (1, 2) THEN hair
            -- The background: sky, dithered with a lighter blue further down.
            WHEN y >= 43 AND (y - 43) % 2 = 0 AND x % 2 = 1 THEN 5
            WHEN y >= 43 AND (y - 43) % 4 = 1 AND x % 4 = 0 THEN 5
            WHEN y >= 43 AND (y - 43) % 4 = 3 AND x % 4 = 2 THEN 5
            ELSE 4
        END AS slot
        FROM outlined)
    SELECT decode(string_agg(lpad(to_hex(slot), 2, '0'), '' ORDER BY y, x), 'hex')
    FROM coloured
$$;

WITH looks (last_name, hair, skin, clothes, bun, long_hair) AS (VALUES
    ('Babbage',  13, 28, 12, false, false),
    ('Hopper',   10, 28,  2, true,  true),
    ('Lovelace', 11, 28, 29, true,  true),
    ('Turing',   11, 28, 16, false, false),
    ('Hamilton', 10, 28, 20, false, true),
    ('Pascal',   31, 28, 26, false, true),
    ('Jacquard', 14, 28, 10, false, false),
    ('Shannon',  12, 28,  3, false, false),
    ('Lamarr',    0, 28, 25, false, true),
    ('Dijkstra', 13, 28, 17, false, false),
    ('Liskov',    9, 28, 24, true,  true),
    ('Pouzin',   15, 28, 13, false, false)),
chosen AS MATERIALIZED (
    SELECT c.id AS contact_id, uuidv7() AS image_id, l.*
    FROM addressbook.contacts c JOIN looks l USING (last_name)),
drawn AS (
    INSERT INTO xila.image (id, width, height, pixels)
    SELECT image_id, 68, 80, addressbook.portrait(hair, skin, clothes, bun, long_hair)
    FROM chosen)
UPDATE addressbook.contacts c SET photo = chosen.image_id
FROM chosen WHERE c.id = chosen.contact_id;

-- Who may use the stack is decided by ordinary PostgreSQL privileges.
GRANT USAGE ON SCHEMA addressbook TO xila_user;
GRANT SELECT, INSERT, UPDATE, DELETE ON addressbook.contacts TO xila_user;

-- The stack, its background, its actions, its keys and its widgets.
WITH s AS (
    INSERT INTO xila.stack (name, title) VALUES ('addressbook', 'Address book')
    RETURNING id),
b AS (
    INSERT INTO xila.background (stack_id, source_schema, source_table, key_column, order_columns)
    SELECT id, 'addressbook', 'contacts', 'id', '{last_name,first_name}' FROM s
    RETURNING id),
a AS (
    INSERT INTO xila.action (background_id, name, label, handler)
    SELECT b.id, v.name, v.label, v.name
    FROM b, (VALUES
        ('previous_card', 'Prev.'),
        ('next_card',     'Next'),
        ('new_card',      'New'),
        ('save_card',     'Save')) AS v(name, label)
    RETURNING id, background_id, name),
k AS (
    INSERT INTO xila.key_binding (background_id, key, action_id)
    SELECT a.background_id, v.key, a.id
    FROM a JOIN (VALUES
        ('F6',    'new_card'),
        ('F7',    'previous_card'),
        ('F8',    'next_card'),
        ('Enter', 'save_card')) AS v(key, action) ON v.action = a.name),
w AS (
    INSERT INTO xila.widget (background_id, name, kind, ord, label, bound_column, action_id, props)
    SELECT b.id, v.name, v.kind, v.ord, v.label, v.col,
           (SELECT a.id FROM a WHERE a.name = v.action), v.props::jsonb
    FROM b, (VALUES
        ('title',      'text',   10,  NULL,         NULL,         NULL,            '{"template": "{last_name}, {first_name}", "style": "title"}'),
        ('photo',      'image',  15,  NULL,         'photo',      NULL,            '{}'),
        ('last_name',  'field',  20,  'Last name',  'last_name',  NULL,            '{"required": true}'),
        ('first_name', 'field',  30,  'First name', 'first_name', NULL,            '{}'),
        ('company',    'field',  40,  'Company',    'company',    NULL,            '{}'),
        ('phone',      'field',  50,  'Phone',      'phone',      NULL,            '{}'),
        ('email',      'field',  60,  'Email',      'email',      NULL,            '{}'),
        ('city',       'field',  70,  'City',       'city',       NULL,            '{}'),
        ('birthday',   'field',  80,  'Birthday',   'birthday',   NULL,            '{}'),
        ('previous',   'button', 90,  'Prev.',      NULL,         'previous_card', '{}'),
        ('next',       'button', 100, 'Next',       NULL,         'next_card',     '{}'),
        ('new',        'button', 110, 'New card',   NULL,         'new_card',      '{}')
    ) AS v(name, kind, ord, label, col, action, props)
    RETURNING id, name)
-- One presentation per renderer, when the default one is not wanted. The big
-- title belongs to the graphical card, not to the text screen. In the window,
-- the browsing buttons get an arrow and "New card" is the main button.
-- These rows carry a style only: without a size, a widget keeps the place the
-- client gives it.
INSERT INTO xila.layout (widget_id, renderer, style)
SELECT w.id, v.renderer, v.style::jsonb
FROM w JOIN (VALUES
    ('title',    'text',    '{"hidden": true}'),
    ('previous', 'graphic', '{"arrow": "left"}'),
    ('next',     'graphic', '{"arrow": "right"}'),
    ('new',      'graphic', '{"variant": "primary"}')) AS v(name, renderer, style)
  ON v.name = w.name;
