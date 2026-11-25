-- Example stack: an address book ("Carnet d'adresses").
--
-- This is NOT part of pgXila. pgXila ships the tool to build stacks, not
-- ready-made applications. This file builds the kind of stack a user would
-- build in author mode, with nothing but ordinary SQL: a table, then rows in
-- the xila tables. It is used as a test fixture and as a demonstration.
--
-- Load it with scripts/load-example.sh carnet. Loading it again replaces it.
-- All names and values are fictitious.

DELETE FROM xila.stack WHERE name = 'carnet';
DROP SCHEMA IF EXISTS carnet CASCADE;

-- The data: one row per card.
CREATE SCHEMA carnet;

CREATE TABLE carnet.contacts (
    id        uuid PRIMARY KEY DEFAULT uuidv7(),
    nom       text NOT NULL,
    prenom    text,
    societe   text,
    telephone text,
    courriel  text,
    ville     text,
    anniversaire date
);

INSERT INTO carnet.contacts (nom, prenom, societe, telephone, courriel, ville) VALUES
    ('Babbage',   'Charles',  'Difference Works',     '01 23 45 67 01', 'charles@example.org',  'Londres'),
    ('Hopper',    'Grace',    'Compilateurs Réunis',  '01 23 45 67 02', 'grace@example.org',    'New York'),
    ('Lovelace',  'Ada',      'Analytical Engines',   '01 23 45 67 89', 'ada@example.org',      'Londres'),
    ('Turing',    'Alan',     'Bletchley Conseil',    '01 23 45 67 03', 'alan@example.org',     'Manchester'),
    ('Hamilton',  'Margaret', 'Apollo Logiciels',     '01 23 45 67 04', 'margaret@example.org', 'Boston'),
    ('Pascal',    'Blaise',   'La Pascaline',         '01 23 45 67 05', 'blaise@example.org',   'Clermont-Ferrand'),
    ('Jacquard',  'Joseph',   'Métiers à tisser',     '01 23 45 67 06', 'joseph@example.org',   'Lyon'),
    ('Shannon',   'Claude',   'Signal et Bruit',      '01 23 45 67 07', 'claude@example.org',   'Gaylord'),
    ('Lamarr',    'Hedy',     'Sauts de fréquence',   '01 23 45 67 08', 'hedy@example.org',     'Vienne'),
    ('Dijkstra',  'Edsger',   'Plus Court Chemin',    '01 23 45 67 09', 'edsger@example.org',   'Rotterdam'),
    ('Liskov',    'Barbara',  'Substitution SARL',    '01 23 45 67 10', 'barbara@example.org',  'Los Angeles'),
    ('Pouzin',    'Louis',    'Cyclades',             '01 23 45 67 11', 'louis@example.org',    'Chantenay-Saint-Imbert');

-- Who may use the stack is decided by ordinary PostgreSQL privileges.
GRANT USAGE ON SCHEMA carnet TO xila_user;
GRANT SELECT, INSERT, UPDATE, DELETE ON carnet.contacts TO xila_user;

-- The stack, its background, its actions, its keys and its widgets.
WITH s AS (
    INSERT INTO xila.stack (name, title) VALUES ('carnet', 'Carnet d''adresses')
    RETURNING id),
b AS (
    INSERT INTO xila.background (stack_id, source_schema, source_table, key_column, order_columns)
    SELECT id, 'carnet', 'contacts', 'id', '{nom,prenom}' FROM s
    RETURNING id),
a AS (
    INSERT INTO xila.action (background_id, name, label, handler)
    SELECT b.id, v.name, v.label, v.name
    FROM b, (VALUES
        ('previous_card', 'Préc.'),
        ('next_card',     'Suiv.'),
        ('new_card',      'Nouvelle'),
        ('save_card',     'Enregistrer')) AS v(name, label)
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
        ('titre',     'text',   10, NULL,           NULL,        NULL,            '{"template": "{nom}, {prenom}", "style": "title"}'),
        ('nom',       'field',  20, 'Nom',          'nom',       NULL,            '{"required": true}'),
        ('prenom',    'field',  30, 'Prénom',       'prenom',    NULL,            '{}'),
        ('societe',   'field',  40, 'Société',      'societe',   NULL,            '{}'),
        ('telephone', 'field',  50, 'Téléphone',    'telephone', NULL,            '{}'),
        ('courriel',  'field',  60, 'Courriel',     'courriel',  NULL,            '{}'),
        ('ville',     'field',  70, 'Ville',        'ville',     NULL,            '{}'),
        ('anniversaire', 'field', 80, 'Anniversaire', 'anniversaire', NULL,       '{}'),
        ('precedente','button', 90, 'Préc.',        NULL,        'previous_card', '{}'),
        ('suivante',  'button', 100, 'Suiv.',       NULL,        'next_card',     '{}'),
        ('nouvelle',  'button', 110, 'Nouvelle fiche', NULL,     'new_card',      '{}')
    ) AS v(name, kind, ord, label, col, action, props)
    RETURNING id, name)
-- One presentation per renderer, when the default one is not wanted: the big
-- title belongs to the graphical card, not to the text screen.
INSERT INTO xila.layout (widget_id, renderer, style)
SELECT id, 'text', '{"hidden": true}' FROM w WHERE name = 'titre';
