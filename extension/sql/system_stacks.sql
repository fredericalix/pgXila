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
    "view": "icons", "icon": "icon",
    "columns": [
        {"column": "title", "label": "Stack"},
        {"column": "cards", "label": "Cards", "align": "right"}
    ]}'
FROM a;

-- ---------------------------------------------------------------------------
-- Author mode
-- ---------------------------------------------------------------------------
-- The author screens are stacks too. Each one is a card of a view: the key of
-- the card says what the dialog is about (the name of a stack, the identifier
-- of a field), and what is typed on it goes to an action, never to the view.

-- F11 enters author mode from any stack the current role may change.
INSERT INTO xila.action (name, label, handler, args)
VALUES ('author', 'Author', 'author', '{"for": "editor"}');
INSERT INTO xila.key_binding (key, action_id) VALUES ('F11', xila.action_id(NULL, 'author'));

-- What the visual editor of the graphical client asks for, on any stack the
-- current role may change. "author_form" reaches the forms from the editor.
INSERT INTO xila.action (name, label, handler, args) VALUES
    ('author_form',   'Stack',        'author',        '{"for": "editor", "form": true}'),
    ('new_field',     'New field',    'new_field',     '{"for": "editor"}'),
    ('stack_history', 'History',      'history',       '{"for": "editor", "of": "stack"}'),
    ('place_widgets', 'Move',         'place_widgets', '{"for": "editor"}'),
    ('reset_layout',  'Reset layout', 'reset_layout',  '{"for": "editor"}'),
    ('add_widget',    'New widget',   'add_widget',    '{"for": "editor"}'),
    ('set_widget',    'Change',       'set_widget',    '{"for": "editor"}'),
    ('remove_widget', 'Remove',       'remove_widget', '{"for": "editor"}'),
    ('undo',          'Undo',         'undo',          '{"for": "editor"}'),
    ('set_colour',    'Colour',       'set_colour',    '{"for": "editor"}'),
    ('remove_colour', 'No colour',    'remove_colour', '{"for": "editor"}');

-- What the drawing editor of the graphical client asks for, on any screen:
-- the picture of a card is drawn by whoever may write the card. They are
-- tools, not something a button of a stack would do.
INSERT INTO xila.action (name, label, handler, args) VALUES
    ('draw',         'Draw',         'draw',         '{"for": "tool"}'),
    ('save_drawing', 'Save picture', 'save_drawing', '{"for": "tool"}');

-- Scripts. "script" runs the script of the button, of the key or of the
-- screen that triggered it; it is not something to choose for a button. The
-- others are what the script editor of the graphical client asks for.
INSERT INTO xila.action (name, label, handler, args) VALUES
    ('script',      'Script',      'script',      '{"for": "tool"}'),
    ('edit_script', 'Script',      'edit_script', '{"for": "editor"}'),
    ('test_script', 'Test',        'test_script', '{"for": "editor"}'),
    ('save_script', 'Save script', 'save_script', '{"for": "editor"}');

-- What authors find in the list of stacks.
INSERT INTO xila.action (background_id, name, label, handler, args) VALUES
    (xila.background_id('stacks'), 'new_stack', 'New stack', 'new_stack', '{"for": "author"}'),
    (xila.background_id('stacks'), 'deleted',   'Deleted',   'history',
     '{"for": "author", "of": "deleted"}'),
    (xila.background_id('stacks'), 'author',    'Author',    'author',    '{"for": "author"}');
INSERT INTO xila.key_binding (background_id, key, action_id) VALUES
    (xila.background_id('stacks'), 'F6',  xila.action_id('stacks', 'new_stack')),
    (xila.background_id('stacks'), 'F10', xila.action_id('stacks', 'deleted')),
    (xila.background_id('stacks'), 'F11', xila.action_id('stacks', 'author'));

-- New stack.
INSERT INTO xila.stack (name, title, kind) VALUES ('new_stack', 'New stack', 'system');
INSERT INTO xila.background (stack_id, source_schema, source_table, key_column)
SELECT id, 'xila', 'new_stack_form', 'name' FROM xila.stack WHERE name = 'new_stack';
INSERT INTO xila.action (background_id, name, label, handler)
VALUES (xila.background_id('new_stack'), 'create', 'Create', 'create_stack');
INSERT INTO xila.key_binding (background_id, key, action_id)
VALUES (xila.background_id('new_stack'), 'Enter', xila.action_id('new_stack', 'create'));
INSERT INTO xila.widget (background_id, name, kind, ord, label, bound_column, action_id, props)
VALUES
    (xila.background_id('new_stack'), 'heading', 'text', 10, NULL, NULL, NULL,
     '{"template": "New stack", "style": "title"}'),
    (xila.background_id('new_stack'), 'hint', 'text', 20, NULL, NULL, NULL,
     '{"template": "The name may be left empty.", "style": "subtitle"}'),
    (xila.background_id('new_stack'), 'title', 'field', 30, 'Title', 'title', NULL, '{}'),
    (xila.background_id('new_stack'), 'name', 'field', 40, 'Name', 'name', NULL, '{}'),
    (xila.background_id('new_stack'), 'source', 'field', 50, 'Start from', 'source', NULL,
     '{"choices": [{"value": "", "label": "An empty stack"}],
       "choices_from": {"schema": "xila", "table": "own_table", "value": "name"}}'),
    (xila.background_id('new_stack'), 'create', 'button', 60, 'Create', NULL,
     xila.action_id('new_stack', 'create'), '{}'),
    (xila.background_id('new_stack'), 'cancel', 'button', 70, 'Cancel', NULL,
     xila.action_id(NULL, 'back'), '{}');

-- Author mode: one stack, its title and its fields.
INSERT INTO xila.stack (name, title, kind) VALUES ('author', 'Author mode', 'system');
INSERT INTO xila.background (stack_id, source_schema, source_table, key_column)
SELECT id, 'xila', 'stack_info', 'name' FROM xila.stack WHERE name = 'author';
INSERT INTO xila.action (background_id, name, label, handler, args) VALUES
    (xila.background_id('author'), 'save',         'Save',         'save_stack',   '{}'),
    (xila.background_id('author'), 'new_field',    'New field',    'new_field',    '{}'),
    (xila.background_id('author'), 'edit_field',   'Edit',         'edit_field',   '{}'),
    (xila.background_id('author'), 'up',           'Up',           'move_field',   '{"by": -1}'),
    (xila.background_id('author'), 'down',         'Down',         'move_field',   '{"by": 1}'),
    (xila.background_id('author'), 'history',      'History',      'history',      '{"of": "stack"}'),
    (xila.background_id('author'), 'browse',       'Browse',       'browse',       '{}'),
    (xila.background_id('author'), 'delete_stack', 'Delete stack', 'delete_stack', '{}');
INSERT INTO xila.key_binding (background_id, key, action_id)
SELECT xila.background_id('author'), k.key, xila.action_id('author', k.action)
FROM (VALUES ('Enter', 'save'), ('F6', 'new_field'), ('F7', 'up'), ('F8', 'down'),
             ('F10', 'history'), ('F11', 'browse')) AS k(key, action);
INSERT INTO xila.widget (background_id, name, kind, ord, label, bound_column, action_id, props)
VALUES
    (xila.background_id('author'), 'heading', 'text', 10, NULL, NULL, NULL,
     '{"template": "{title}", "style": "title"}'),
    (xila.background_id('author'), 'mode', 'text', 20, NULL, NULL, NULL,
     '{"template": "Author mode", "style": "subtitle"}'),
    (xila.background_id('author'), 'title', 'field', 30, 'Title', 'title', NULL, '{}'),
    (xila.background_id('author'), 'card_title', 'field', 40, 'Card title', 'card_title', NULL, '{}'),
    (xila.background_id('author'), 'fields', 'grid', 50, NULL, NULL,
     xila.action_id('author', 'edit_field'),
     '{"schema": "xila", "table": "field", "key": "id",
       "where": {"stack": "name"}, "order": ["ord", "name"],
       "columns": [
           {"column": "label", "label": "Field"},
           {"column": "name", "label": "Name"},
           {"column": "type_label", "label": "Type"},
           {"column": "required_label", "label": "Required"}
       ]}'),
    (xila.background_id('author'), 'new_field', 'button', 60, 'New field', NULL,
     xila.action_id('author', 'new_field'), '{}'),
    (xila.background_id('author'), 'history', 'button', 70, 'History', NULL,
     xila.action_id('author', 'history'), '{}'),
    (xila.background_id('author'), 'delete_stack', 'button', 80, 'Delete stack', NULL,
     xila.action_id('author', 'delete_stack'), '{}');

-- One field of a stack.
INSERT INTO xila.stack (name, title, kind) VALUES ('field', 'Field', 'system');
INSERT INTO xila.background (stack_id, source_schema, source_table, key_column)
SELECT id, 'xila', 'field', 'id' FROM xila.stack WHERE name = 'field';
INSERT INTO xila.action (background_id, name, label, handler) VALUES
    (xila.background_id('field'), 'save',   'Save',   'save_field'),
    (xila.background_id('field'), 'remove', 'Remove', 'remove_field');
INSERT INTO xila.key_binding (background_id, key, action_id)
VALUES (xila.background_id('field'), 'Enter', xila.action_id('field', 'save'));
INSERT INTO xila.widget (background_id, name, kind, ord, label, bound_column, action_id, props)
VALUES
    (xila.background_id('field'), 'heading', 'text', 10, NULL, NULL, NULL,
     '{"template": "Field", "style": "title"}'),
    (xila.background_id('field'), 'hint', 'text', 20, NULL, NULL, NULL,
     '{"template": "The name may be left empty.", "style": "subtitle"}'),
    (xila.background_id('field'), 'label', 'field', 30, 'Label', 'label', NULL, '{}'),
    (xila.background_id('field'), 'name', 'field', 40, 'Name', 'name', NULL, '{}'),
    (xila.background_id('field'), 'type', 'field', 50, 'Type', 'type', NULL,
     '{"choices": [
           {"value": "text", "label": "Text"},
           {"value": "number", "label": "Number"},
           {"value": "date", "label": "Date"},
           {"value": "boolean", "label": "Yes or no"},
           {"value": "picture", "label": "Picture"}
       ]}'),
    (xila.background_id('field'), 'required', 'field', 60, 'Required', 'required', NULL, '{}'),
    (xila.background_id('field'), 'save', 'button', 70, 'Save', NULL,
     xila.action_id('field', 'save'), '{}'),
    (xila.background_id('field'), 'remove', 'button', 80, 'Remove', NULL,
     xila.action_id('field', 'remove'), '{}'),
    (xila.background_id('field'), 'cancel', 'button', 90, 'Cancel', NULL,
     xila.action_id(NULL, 'back'), '{}');

-- The history of a stack: every change, newest first.
INSERT INTO xila.stack (name, title, kind) VALUES ('history', 'History', 'system');
INSERT INTO xila.background (stack_id, source_schema, source_table, key_column)
SELECT id, 'xila', 'stack_info', 'name' FROM xila.stack WHERE name = 'history';
INSERT INTO xila.action (background_id, name, label, handler, args) VALUES
    (xila.background_id('history'), 'rewind_screen', 'Rewind screen', 'rewind', '{"scope": "screen"}'),
    (xila.background_id('history'), 'rewind_all',    'Rewind all',    'rewind', '{"scope": "stack"}');
INSERT INTO xila.key_binding (background_id, key, action_id) VALUES
    (xila.background_id('history'), 'F6', xila.action_id('history', 'rewind_screen')),
    (xila.background_id('history'), 'F7', xila.action_id('history', 'rewind_all'));
INSERT INTO xila.widget (background_id, name, kind, ord, label, bound_column, action_id, props)
VALUES
    (xila.background_id('history'), 'heading', 'text', 10, NULL, NULL, NULL,
     '{"template": "{title}", "style": "title"}'),
    (xila.background_id('history'), 'hint', 'text', 20, NULL, NULL, NULL,
     '{"template": "A rewind undoes the selected change and all later ones.", "style": "subtitle"}'),
    (xila.background_id('history'), 'changes', 'grid', 30, NULL, NULL,
     xila.action_id('history', 'rewind_all'),
     '{"schema": "xila", "table": "history_line", "key": "id",
       "where": {"stack": "name"}, "order": ["id"], "descending": true,
       "columns": [
           {"column": "at", "label": "When"},
           {"column": "actor", "label": "Who"},
           {"column": "summary", "label": "What"}
       ]}'),
    (xila.background_id('history'), 'rewind_screen', 'button', 40, 'Rewind screen', NULL,
     xila.action_id('history', 'rewind_screen'), '{}'),
    (xila.background_id('history'), 'rewind_all', 'button', 50, 'Rewind all', NULL,
     xila.action_id('history', 'rewind_all'), '{}');

-- The history of the cards of a stack, for whoever may write them.
INSERT INTO xila.stack (name, title, kind) VALUES ('card_history', 'History of the cards', 'system');
INSERT INTO xila.background (stack_id, source_schema, source_table, key_column)
SELECT id, 'xila', 'stack_info', 'name' FROM xila.stack WHERE name = 'card_history';
INSERT INTO xila.action (background_id, name, label, handler, args) VALUES
    (xila.background_id('card_history'), 'rewind', 'Rewind card', 'rewind', '{"scope": "card"}');
INSERT INTO xila.key_binding (background_id, key, action_id) VALUES
    (xila.background_id('card_history'), 'F6', xila.action_id('card_history', 'rewind'));
INSERT INTO xila.widget (background_id, name, kind, ord, label, bound_column, action_id, props)
VALUES
    (xila.background_id('card_history'), 'heading', 'text', 10, NULL, NULL, NULL,
     '{"template": "{title}", "style": "title"}'),
    (xila.background_id('card_history'), 'hint', 'text', 20, NULL, NULL, NULL,
     '{"template": "Undoes the selected change and later ones on that card.", "style": "subtitle"}'),
    (xila.background_id('card_history'), 'changes', 'grid', 30, NULL, NULL,
     xila.action_id('card_history', 'rewind'),
     '{"schema": "xila", "table": "card_history_line", "key": "id",
       "where": {"stack": "name"}, "order": ["id"], "descending": true,
       "columns": [
           {"column": "at", "label": "When"},
           {"column": "actor", "label": "Who"},
           {"column": "summary", "label": "What"}
       ]}'),
    (xila.background_id('card_history'), 'rewind', 'button', 40, 'Rewind card', NULL,
     xila.action_id('card_history', 'rewind'), '{}');

-- The stacks that were deleted and can be brought back.
INSERT INTO xila.stack (name, title, kind) VALUES ('deleted_stacks', 'Deleted stacks', 'system');
INSERT INTO xila.background (stack_id) SELECT id FROM xila.stack WHERE name = 'deleted_stacks';
INSERT INTO xila.action (background_id, name, label, handler, args) VALUES
    (xila.background_id('deleted_stacks'), 'restore', 'Restore', 'rewind', '{"scope": "deleted"}');
INSERT INTO xila.key_binding (background_id, key, action_id) VALUES
    (xila.background_id('deleted_stacks'), 'F6', xila.action_id('deleted_stacks', 'restore'));
INSERT INTO xila.widget (background_id, name, kind, ord, label, bound_column, action_id, props)
VALUES
    (xila.background_id('deleted_stacks'), 'list', 'grid', 10, NULL, NULL,
     xila.action_id('deleted_stacks', 'restore'),
     '{"schema": "xila", "table": "deleted_stack", "key": "id",
       "order": ["id"], "descending": true,
       "columns": [
           {"column": "title", "label": "Stack"},
           {"column": "at", "label": "Deleted"},
           {"column": "actor", "label": "By"}
       ]}'),
    (xila.background_id('deleted_stacks'), 'restore', 'button', 20, 'Restore', NULL,
     xila.action_id('deleted_stacks', 'restore'), '{}');

-- ---------------------------------------------------------------------------
-- The agent
-- ---------------------------------------------------------------------------
-- See docs/api.md, section 11. F9 calls the agent from any screen; like the
-- keys to leave a screen, it means the same everywhere.
INSERT INTO xila.action (name, label, handler) VALUES ('agent', 'Agent', 'agent');
INSERT INTO xila.key_binding (key, action_id) VALUES ('F9', xila.action_id(NULL, 'agent'));

INSERT INTO xila.widget (background_id, name, kind, ord, label, action_id)
VALUES (xila.background_id('desktop'), 'agent', 'button', 60, 'Agent',
        xila.action_id(NULL, 'agent'));

-- The conversation with the agent. The card is one conversation of the
-- current role; its lines are a list shown as a conversation.
INSERT INTO xila.stack (name, title, kind) VALUES ('agent', 'Agent', 'system');
INSERT INTO xila.background (stack_id, source_schema, source_table, key_column)
SELECT id, 'xila', 'agent_talk', 'id' FROM xila.stack WHERE name = 'agent';
INSERT INTO xila.action (background_id, name, label, handler, args) VALUES
    (xila.background_id('agent'), 'send',     'Send',          'ask_agent',        '{}'),
    (xila.background_id('agent'), 'stop',     'Stop',          'stop_agent',       '{}'),
    (xila.background_id('agent'), 'new',      'New',           'new_conversation', '{}'),
    (xila.background_id('agent'), 'list',     'Conversations', 'conversations',    '{}'),
    (xila.background_id('agent'), 'journal',  'Journal',       'agent_journal',    '{}'),
    (xila.background_id('agent'), 'settings', 'Settings',      'agent_settings',   '{"for": "admin"}');
INSERT INTO xila.key_binding (background_id, key, action_id)
SELECT xila.background_id('agent'), k.key, xila.action_id('agent', k.action)
FROM (VALUES ('Enter', 'send'), ('F4', 'settings'), ('F5', 'stop'), ('F6', 'new'),
             ('F7', 'journal'), ('F10', 'list')) AS k(key, action);
INSERT INTO xila.widget (background_id, name, kind, ord, label, bound_column, action_id, props)
VALUES
    (xila.background_id('agent'), 'heading', 'text', 10, NULL, NULL, NULL,
     '{"template": "{title}", "style": "title"}'),
    (xila.background_id('agent'), 'about', 'text', 20, NULL, NULL, NULL,
     '{"template": "{about}", "style": "subtitle"}'),
    (xila.background_id('agent'), 'talk', 'grid', 30, NULL, NULL, NULL,
     '{"schema": "xila", "table": "agent_line", "key": "id",
       "view": "talk", "who": "kind",
       "where": {"conversation": "id"}, "order": ["at", "id"], "descending": true,
       "columns": [
           {"column": "kind", "label": "Who"},
           {"column": "text", "label": "Said"}
       ]}'),
    (xila.background_id('agent'), 'ask', 'field', 40, 'Ask', 'ask', NULL, '{}'),
    (xila.background_id('agent'), 'send', 'button', 50, 'Send', NULL,
     xila.action_id('agent', 'send'), '{}'),
    (xila.background_id('agent'), 'model', 'text', 60, 'Model', NULL, NULL,
     '{"template": "{model}"}'),
    (xila.background_id('agent'), 'host', 'text', 70, 'Where', NULL, NULL,
     '{"template": "{host}"}'),
    (xila.background_id('agent'), 'tools', 'text', 80, 'Tools', NULL, NULL,
     '{"template": "{tools}"}'),
    (xila.background_id('agent'), 'guard', 'text', 90, 'Guard rails', NULL, NULL,
     '{"template": "Reads only, restricted role, journal"}'),
    (xila.background_id('agent'), 'today', 'text', 100, 'Today', NULL, NULL,
     '{"template": "{today}"}');

-- The conversations of the current role.
INSERT INTO xila.stack (name, title, kind) VALUES ('conversations', 'Conversations', 'system');
INSERT INTO xila.background (stack_id) SELECT id FROM xila.stack WHERE name = 'conversations';
INSERT INTO xila.action (background_id, name, label, handler) VALUES
    (xila.background_id('conversations'), 'open', 'Open', 'open_conversation'),
    (xila.background_id('conversations'), 'new',  'New',  'new_conversation');
INSERT INTO xila.key_binding (background_id, key, action_id) VALUES
    (xila.background_id('conversations'), 'F6', xila.action_id('conversations', 'new'));
INSERT INTO xila.widget (background_id, name, kind, ord, label, bound_column, action_id, props)
VALUES
    (xila.background_id('conversations'), 'list', 'grid', 10, NULL, NULL,
     xila.action_id('conversations', 'open'),
     '{"schema": "xila", "table": "conversation_line", "key": "id",
       "order": ["changed_at", "id"], "descending": true,
       "columns": [
           {"column": "title", "label": "Conversation"},
           {"column": "at", "label": "Last line"},
           {"column": "lines", "label": "Lines", "align": "right"}
       ]}'),
    (xila.background_id('conversations'), 'open', 'button', 20, 'Open', NULL,
     xila.action_id('conversations', 'open'), '{}');

-- The journal of the agent: one's own entries, or all of them for an
-- administrator.
INSERT INTO xila.stack (name, title, kind) VALUES ('agent_journal', 'Journal of the agent', 'system');
INSERT INTO xila.background (stack_id) SELECT id FROM xila.stack WHERE name = 'agent_journal';
INSERT INTO xila.widget (background_id, name, kind, ord, label, bound_column, action_id, props)
VALUES
    (xila.background_id('agent_journal'), 'list', 'grid', 10, NULL, NULL, NULL,
     '{"schema": "xila", "table": "agent_journal_line", "key": "id",
       "order": ["moment", "id"], "descending": true,
       "columns": [
           {"column": "at", "label": "When"},
           {"column": "role", "label": "For"},
           {"column": "summary", "label": "What"}
       ]}');

-- The settings of the agent, for administrators.
INSERT INTO xila.stack (name, title, kind) VALUES ('agent_settings', 'Agent settings', 'system');
INSERT INTO xila.background (stack_id, source_schema, source_table, key_column)
SELECT id, 'xila', 'agent_setting_form', 'id' FROM xila.stack WHERE name = 'agent_settings';
INSERT INTO xila.action (background_id, name, label, handler)
VALUES (xila.background_id('agent_settings'), 'save', 'Save', 'save_agent_settings');
INSERT INTO xila.key_binding (background_id, key, action_id)
VALUES (xila.background_id('agent_settings'), 'Enter', xila.action_id('agent_settings', 'save'));
INSERT INTO xila.widget (background_id, name, kind, ord, label, bound_column, action_id, props)
VALUES
    (xila.background_id('agent_settings'), 'heading', 'text', 10, NULL, NULL, NULL,
     '{"template": "Agent settings", "style": "title"}'),
    (xila.background_id('agent_settings'), 'hint', 'text', 20, NULL, NULL, NULL,
     '{"template": "The token stays on the server: only the name of its variable is kept here.", "style": "subtitle"}'),
    (xila.background_id('agent_settings'), 'address', 'field', 30, 'Address', 'address', NULL, '{}'),
    (xila.background_id('agent_settings'), 'model', 'field', 40, 'Model', 'model', NULL, '{}'),
    (xila.background_id('agent_settings'), 'token_variable', 'field', 50, 'Token variable',
     'token_variable', NULL, '{}'),
    (xila.background_id('agent_settings'), 'instructions', 'field', 60, 'Instructions',
     'instructions', NULL, '{}'),
    (xila.background_id('agent_settings'), 'save', 'button', 70, 'Save', NULL,
     xila.action_id('agent_settings', 'save'), '{}'),
    (xila.background_id('agent_settings'), 'cancel', 'button', 80, 'Cancel', NULL,
     xila.action_id(NULL, 'back'), '{}');

-- In author mode: whether the agent may read the cards of the stack.
INSERT INTO xila.widget (background_id, name, kind, ord, label, bound_column, props)
VALUES (xila.background_id('author'), 'agent', 'field', 45, 'The agent may read the cards',
        'agent', '{}');

-- In text mode the header of the screen already carries its title.
INSERT INTO xila.layout (widget_id, renderer, style)
SELECT w.id, 'text', '{"hidden": true}'
FROM xila.widget w
WHERE w.kind = 'text' AND w.props->>'style' IN ('title', 'subtitle')
  AND NOT EXISTS (SELECT FROM xila.layout l WHERE l.widget_id = w.id AND l.renderer = 'text');

-- The main button of each dialog.
INSERT INTO xila.layout (widget_id, renderer, style)
SELECT w.id, 'graphic', '{"variant": "primary"}'
FROM xila.widget w
JOIN xila.background b ON b.id = w.background_id
JOIN xila.stack s ON s.id = b.stack_id
WHERE (s.name, w.name) IN (('new_stack', 'create'), ('author', 'new_field'), ('field', 'save'),
                           ('card_history', 'rewind'), ('deleted_stacks', 'restore'),
                           ('agent', 'send'), ('conversations', 'open'),
                           ('agent_settings', 'save'));
