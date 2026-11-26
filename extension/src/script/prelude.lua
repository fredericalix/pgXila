-- What a script of pgXila sees, written on top of what the extension gives
-- (`host`). Nothing here can be reached from a script except through `env`,
-- the table its globals live in: there is no file, no operating system, no
-- module and no loading of code in it.
local host = ...

local pcall, error, type, tostring, tonumber, pairs, ipairs, next, select, rawequal, setmetatable =
    pcall, error, type, tostring, tonumber, pairs, ipairs, next, select, rawequal, setmetatable
local math_type, to_integer = math.type, math.tointeger
local pack, unpack, concat, insert = table.pack, table.unpack, table.concat, table.insert

local NULL = host.null
local stack_name = host.stack
local context = host.context

-- ---------------------------------------------------------------------------
-- The library of Lua, where it could not be stopped
-- ---------------------------------------------------------------------------
-- The functions of the library are written in C: the count of instructions
-- that stops a script does not see inside them. Those that can run for very
-- long are replaced here by ones that are counted, and give the same answers.
-- They are changed in the tables of the library themselves, so that
-- ("text"):find(…) is the counted one too.
local sub, raw_rep, raw_find = string.sub, string.rep, string.find

-- Bytecode cannot be loaded, so there is no use in dumping it; nothing here
-- reads or writes binary data; and table.move can be asked to move nothing,
-- forever.
string.dump, string.pack, string.unpack, string.packsize = nil, nil, nil, nil
table.move = nil

local function text_argument(value, position, name)
    local kind = type(value)
    if kind == "string" then
        return value
    elseif kind == "number" then
        return tostring(value)
    end
    error("bad argument #" .. position .. " to '" .. name .. "' (string expected, got "
        .. (value == nil and "no value" or kind) .. ")", 3)
end

-- Where a search starts: counted from the end when negative, as in Lua.
local function start_of(text, init)
    if init == nil then
        return 1
    end
    init = to_integer(init)
    if init == nil then
        error("bad argument #3 (number has no integer representation)", 3)
    elseif init < 0 then
        init = #text + init + 1
        if init < 1 then init = 1 end
    elseif init == 0 then
        init = 1
    end
    return init
end

function string.find(text, pattern, init, plain)
    text, pattern = text_argument(text, 1, "find"), text_argument(pattern, 2, "find")
    init = start_of(text, init)
    if init > #text + 1 then return nil end
    local found = pack(host.match(text, pattern, init, plain and "plain" or "find"))
    if found[1] == false then error(found[2], 2) end
    if found[1] == nil then return nil end
    return unpack(found, 1, found.n)
end

-- What a match gives: its captures, or all of it when it has none.
local function given(text, found)
    if found.n == 2 then
        return sub(text, found[1], found[2])
    end
    return unpack(found, 3, found.n)
end

function string.match(text, pattern, init)
    text, pattern = text_argument(text, 1, "match"), text_argument(pattern, 2, "match")
    init = start_of(text, init)
    if init > #text + 1 then return nil end
    local found = pack(host.match(text, pattern, init, "match"))
    if found[1] == false then error(found[2], 2) end
    if found[1] == nil then return nil end
    return given(text, found)
end

function string.gmatch(text, pattern, init)
    text, pattern = text_argument(text, 1, "gmatch"), text_argument(pattern, 2, "gmatch")
    local at, previous = start_of(text, init), nil
    return function()
        if at > #text + 1 then return nil end
        local found = pack(host.match(text, pattern, at, "next", previous))
        if found[1] == false then error(found[2], 2) end
        if found[1] == nil then
            at = #text + 2
            return nil
        end
        at = found[2] + 1
        previous = at
        return given(text, found)
    end
end

-- What takes the place of a match: a text with %1 for a capture, the entry
-- of a table, or what a function answers.
local function replacement_for(text, found, replacement, kind)
    local whole = sub(text, found[1], found[2])
    local value
    if kind == "string" then
        local parts, from = {}, 1
        while true do
            local percent = raw_find(replacement, "%", from, true)
            if not percent then break end
            parts[#parts + 1] = sub(replacement, from, percent - 1)
            local what = sub(replacement, percent + 1, percent + 1)
            if what == "%" then
                parts[#parts + 1] = "%"
            elseif what >= "0" and what <= "9" and what ~= "" then
                local index = tonumber(what)
                if index == 0 or (index == 1 and found.n == 2) then
                    parts[#parts + 1] = whole
                elseif found[index + 2] == nil then
                    error("invalid capture index %" .. index .. " in replacement string", 3)
                else
                    parts[#parts + 1] = tostring(found[index + 2])
                end
            else
                error("invalid use of '%' in replacement string", 3)
            end
            from = percent + 2
        end
        parts[#parts + 1] = sub(replacement, from)
        return concat(parts)
    elseif kind == "table" then
        value = replacement[found.n == 2 and whole or found[3]]
    else
        value = replacement(given(text, found))
    end
    if not value then
        return whole
    elseif type(value) == "string" or type(value) == "number" then
        return tostring(value)
    end
    error("invalid replacement value (a " .. type(value) .. ")", 3)
end

function string.gsub(text, pattern, replacement, most)
    text, pattern = text_argument(text, 1, "gsub"), text_argument(pattern, 2, "gsub")
    local kind = type(replacement)
    if kind == "number" then
        replacement, kind = tostring(replacement), "string"
    elseif kind ~= "string" and kind ~= "table" and kind ~= "function" then
        error("bad argument #3 to 'gsub' (string/function/table expected, got "
            .. (replacement == nil and "no value" or kind) .. ")", 2)
    end
    if most == nil then most = #text + 1 end
    -- A pattern that starts with ^ is tried once, where the text starts.
    local anchored = sub(pattern, 1, 1) == "^"
    if anchored then pattern = sub(pattern, 2) end

    local parts, count, at, previous = {}, 0, 1, nil
    while count < most do
        local found = pack(host.match(text, pattern, at, anchored and "at" or "next", previous))
        if found[1] == false then error(found[2], 2) end
        if found[1] == nil then break end
        count = count + 1
        parts[#parts + 1] = sub(text, at, found[1] - 1)
        parts[#parts + 1] = replacement_for(text, found, replacement, kind)
        at = found[2] + 1
        previous = at
        if anchored then break end
    end
    parts[#parts + 1] = sub(text, at)
    return concat(parts), count
end

function string.rep(text, count, separator)
    text = text_argument(text, 1, "rep")
    if separator ~= nil then separator = text_argument(separator, 3, "rep") end
    -- Nothing, repeated, is nothing: the library would count to the end.
    if #text == 0 and (separator == nil or #separator == 0) then
        return ""
    end
    return raw_rep(text, count, separator)
end

-- A list can be made to look immensely long. Shifting its items is done
-- here, where each step is counted.
function table.insert(list, ...)
    if type(list) ~= "table" then
        error("bad argument #1 to 'insert' (table expected, got " .. type(list) .. ")", 2)
    end
    local given_count, size = select("#", ...), #list
    if given_count == 1 then
        list[size + 1] = (...)
    elseif given_count == 2 then
        local position, value = ...
        position = to_integer(position)
        if position == nil or position < 1 or position > size + 1 then
            error("bad argument #2 to 'insert' (position out of bounds)", 2)
        end
        for i = size, position, -1 do
            list[i + 1] = list[i]
        end
        list[position] = value
    else
        error("wrong number of arguments to 'insert'", 2)
    end
end

function table.remove(list, position)
    if type(list) ~= "table" then
        error("bad argument #1 to 'remove' (table expected, got " .. type(list) .. ")", 2)
    end
    local size = #list
    if position == nil then
        position = size
    else
        position = to_integer(position)
        if position == nil or (position ~= size and (position < 1 or position > size + 1)) then
            error("bad argument #2 to 'remove' (position out of bounds)", 2)
        end
    end
    local value = list[position]
    while position < size do
        list[position] = list[position + 1]
        position = position + 1
    end
    list[position] = nil
    return value
end

local env = {
    string = string, table = table, math = math, utf8 = utf8,
    pairs = pairs, ipairs = ipairs, next = next, select = select, type = type,
    tostring = tostring, tonumber = tonumber, error = error, assert = assert,
}

-- A limit of the extension, or a question to the user, stops the script for
-- good: it cannot be caught.
function env.pcall(f, ...)
    local results = pack(pcall(f, ...))
    if not results[1] and host.stopped() then
        error(results[2], 0)
    end
    return unpack(results, 1, results.n)
end

function env.print(...)
    local words = pack(...)
    for i = 1, words.n do
        words[i] = tostring(words[i])
    end
    host.print(concat(words, "\t", 1, words.n))
end

-- ---------------------------------------------------------------------------
-- Values
-- ---------------------------------------------------------------------------

-- The text form of a value, as PostgreSQL reads it; NULL for nothing.
local function text_of(value)
    local kind = type(value)
    if value == nil or rawequal(value, NULL) then
        return NULL
    elseif kind == "string" then
        return value
    elseif kind == "number" then
        -- 10 / 2 is 5.0 in Lua, and "5.0" is not an integer for PostgreSQL.
        if math_type(value) == "float" then
            value = to_integer(value) or value
        end
        return tostring(value)
    elseif kind == "boolean" then
        return value and "true" or "false"
    elseif kind == "table" then
        return host.json(value)
    end
    return nil, "a " .. kind .. " cannot be stored"
end

-- The values of a statement: texts, with nil for NULL.
local function arguments(...)
    local values = pack(...)
    for i = 1, values.n do
        local text, problem = text_of(values[i])
        if problem then
            return nil, "value " .. i .. ": " .. problem
        end
        if rawequal(text, NULL) then
            values[i] = nil
        else
            values[i] = text
        end
    end
    return values
end

-- One value from a function of pgXila.
local function call(sql, ...)
    local rows, problem = host.query(sql, pack(...))
    if problem then
        return nil, problem
    end
    return rows[1] and rows[1].answer
end

function env.query(sql, ...)
    local values, problem = arguments(...)
    if problem then error(problem, 2) end
    local rows
    rows, problem = host.query(sql, values)
    if problem then error(problem, 2) end
    return rows
end

function env.execute(sql, ...)
    local values, problem = arguments(...)
    if problem then error(problem, 2) end
    local count
    count, problem = host.execute(sql, values)
    if problem then error(problem, 2) end
    return count
end

-- ---------------------------------------------------------------------------
-- Cards
-- ---------------------------------------------------------------------------
-- A card is a plain table of its fields. What makes it a card is kept here,
-- out of reach: its key (false until it is saved) and the values it had when
-- it was read, to tell what a script changed.
local key_of = setmetatable({}, { __mode = "k" })
local read_as = setmetatable({}, { __mode = "k" })
-- The card on screen as the user sees it: what is saved, and what was typed.
local shown_as = {}

local function copy(values)
    local same = {}
    for name, value in pairs(values) do
        same[name] = value
    end
    return same
end

local function adopt(card, key)
    local values = {}
    for name, value in pairs(card) do
        values[name] = value
    end
    key_of[card] = key or false
    read_as[card] = values
    return card
end

-- What a card holds that `before` did not, as texts: by default, what is to
-- be saved.
local function changes(card, before)
    local changed, any = {}, false
    before = before or read_as[card]
    for name, value in pairs(card) do
        if before[name] ~= value then
            local text, problem = text_of(value)
            if problem then
                return nil, "field “" .. tostring(name) .. "”: " .. problem
            end
            changed[name] = text
            any = true
        end
    end
    for name in pairs(before) do
        if card[name] == nil then
            changed[name] = NULL
            any = true
        end
    end
    return changed, nil, any
end

-- Puts the values of a row in a card, in place of what it held.
local function refill(card, row, key)
    for name in pairs(card) do
        card[name] = nil
    end
    for name, value in pairs(row) do
        card[name] = value
    end
    return adopt(card, key)
end

-- What the script asks of the client, in order.
local effects = setmetatable({}, host.array)
-- Where the values written on the card on screen go: after the effect that
-- last showed that card again.
local written_after = 0

local function effect(what)
    effects[#effects + 1] = what
end

local function on_screen(card)
    return env.card ~= nil and rawequal(card, env.card)
end

-- What is not saved on the card on screen: what the user typed and what the
-- script wrote. It goes with an action as if all of it had been typed.
local function typed()
    if env.card ~= nil and key_of[env.card] ~= nil then
        return changes(env.card)
    end
    return copy(context.dirty or {})
end

-- The card on screen now shows what it holds.
local function shown()
    shown_as = copy(env.card)
    written_after = #effects
end

function env.save(card)
    local key = key_of[card]
    if key == nil then error("save takes a card", 2) end
    local changed, problem = changes(card)
    if problem then error(problem, 2) end
    local saved
    saved, problem = call(
        "SELECT xila.script_save($1, $2, $3::jsonb) AS answer",
        stack_name, key or nil, host.json(changed))
    if problem then error(problem, 2) end
    refill(card, saved.values, saved.key)
    host.count("saved")
    if on_screen(card) then
        effect({ ["do"] = "show", stack = stack_name, card = saved.key })
        shown()
    end
    return card
end

function env.delete(card)
    local key = key_of[card]
    if key == nil then error("delete takes a card", 2) end
    if not key then return end
    local gone, problem = call(
        "SELECT xila.delete_card($1, $2) AS answer", stack_name, key)
    if problem then error(problem, 2) end
    key_of[card] = false
    read_as[card] = {}
    host.count("deleted")
    if on_screen(card) then
        effect({ ["do"] = "show", stack = stack_name, card = gone.next })
        env.card = nil
    end
end

-- Runs an action of the screen and adds what it answers.
local function run_action(name, level)
    local dirty, problem = typed()
    if problem then error(problem, level) end
    local sent = {}
    for member, value in pairs(context) do
        sent[member] = value
    end
    sent.event = nil
    sent.dirty = dirty
    local answered
    answered, problem = call(
        "SELECT xila.act($1, $2, $3::jsonb) AS answer", stack_name, name, host.json(sent))
    if problem then error(problem, level) end
    local leaves = false
    for _, what in ipairs(answered) do
        if what["do"] == "confirm" and not host.confirmed then
            -- Nothing is kept until the user has answered.
            host.ask(what.text, what.label)
            error("stopped", 0)
        end
        leaves = leaves or what["do"] ~= "message" and what["do"] ~= "focus"
        effect(what)
    end
    if leaves and env.card ~= nil and key_of[env.card] ~= nil then
        -- The action was given what the script wrote: it is no longer pending.
        shown()
    end
end

function env.act(name)
    if type(name) ~= "string" then error("act takes the name of an action", 2) end
    if name == "script" then error("a script cannot run the action “script”", 2) end
    run_action(name, 3)
end

local moves = { next = true, previous = true, first = true, last = true }

function env.go_to(where)
    if type(where) == "string" then
        if not moves[where] then
            error('go_to takes a card, or "next", "previous", "first" or "last"', 2)
        end
        -- Leaving a card saves what was typed on it, and what the script
        -- wrote, as the keys of a stack do.
        local dirty, problem = typed()
        if problem then error(problem, 2) end
        local key = context.card
        if env.card ~= nil and key_of[env.card] ~= nil then
            key = key_of[env.card] or nil
        end
        local target
        target, problem = call(
            "SELECT xila.script_move($1, $2, $3::jsonb, $4) AS answer",
            stack_name, key, host.json(dirty), where)
        if problem then error(problem, 2) end
        effect({ ["do"] = "show", stack = stack_name, card = target })
        env.card = nil
        return
    end
    local key = key_of[where]
    if key == nil then
        error('go_to takes a card, or "next", "previous", "first" or "last"', 2)
    end
    if key then
        effect({ ["do"] = "show", stack = stack_name, card = key })
        if on_screen(where) then
            shown()
        end
    else
        local values, problem = changes(where)
        if problem then error(problem, 2) end
        effect({ ["do"] = "new", values = values })
    end
    if not on_screen(where) then
        env.card = nil
    end
end

function env.open(name)
    if type(name) ~= "string" then error("open takes the name of a stack", 2) end
    effect({ ["do"] = "show", stack = name })
end

function env.focus(name)
    if type(name) ~= "string" then error("focus takes the name of a widget", 2) end
    effect({ ["do"] = "focus", widget = name })
end

function env.message(text)
    effect({ ["do"] = "message", text = tostring(text) })
end

function env.confirm(text, label)
    if host.confirmed then
        return true
    end
    host.ask(tostring(text), label ~= nil and tostring(label) or "OK")
    error("stopped", 0)
end

local function clock(sql)
    local text, problem = call(sql)
    if problem then error(problem, 3) end
    return text
end

function env.today()
    return clock("SELECT current_date::text AS answer")
end

function env.now()
    return clock("SELECT to_char(now(), 'YYYY-MM-DD HH24:MI:SS') AS answer")
end

-- ---------------------------------------------------------------------------
-- What the extension calls
-- ---------------------------------------------------------------------------

local key_names = { Enter = true }
for n = 1, 24 do
    key_names["F" .. n] = true
end
local kept_keys = { F3 = true, F9 = true, F11 = true, F12 = true }

-- What the script handles, read from what it defined.
local function describe()
    local events = setmetatable({}, host.array)
    for _, event in ipairs({ "click", "key", "open" }) do
        if type(env["on_" .. event]) == "function" then
            events[#events + 1] = event
        end
    end
    local found = {}
    local wanted = env.keys
    if wanted ~= nil then
        if type(wanted) ~= "table" then
            return nil, '“keys” is a table, as in keys = { F6 = "New card" }.'
        end
        for key, label in pairs(wanted) do
            if type(key) ~= "string" or not key_names[key] then
                return nil, "“" .. tostring(key) .. "” is not a key. Keys are F1 to F24 and Enter."
            elseif kept_keys[key] then
                return nil, "F3, F9, F11 and F12 cannot be taken by a script."
            elseif type(label) ~= "string" or label == "" then
                return nil, "The key " .. key .. ' needs a label, as in keys = { F6 = "New card" }.'
            end
            found[key] = label
        end
        if next(found) ~= nil and type(env.on_key) ~= "function" then
            return nil, "“keys” asks for keys, but there is no function on_key."
        end
    end
    return { events = events, keys = found }
end

-- Runs one event: sets the scene, calls the function, and gives the effects.
local function run(name, argument)
    local handler = env[name]
    if type(handler) ~= "function" then
        return nil, "The script has no function " .. name .. "."
    end

    local scene, problem = call(
        "SELECT xila.script_scene($1, $2::jsonb) AS answer", stack_name, host.json(context))
    if problem then
        return nil, problem
    end
    env.user = scene.user
    env.instance = scene.instance
    env.stack = {
        name = stack_name,
        title = scene.title,
        new_card = function()
            return adopt({}, false)
        end,
        cards = function(filter)
            if filter ~= nil and type(filter) ~= "table" then
                error('stack.cards takes a table of fields, as in stack.cards{city = "London"}', 2)
            end
            local wanted = {}
            for field, value in pairs(filter or {}) do
                local text, trouble = text_of(value)
                if trouble then error(trouble, 2) end
                wanted[field] = text
            end
            local found, trouble = call(
                "SELECT xila.script_cards($1, $2::jsonb) AS answer",
                stack_name, host.json(wanted))
            if trouble then error(trouble, 2) end
            for _, card in ipairs(found) do
                local key = card.key
                local values = card.values
                for field in pairs(card) do
                    card[field] = nil
                end
                refill(card, values, key)
            end
            return found
        end,
    }
    if scene.card ~= nil then
        local card = adopt(scene.card.values, scene.card.key)
        for name, value in pairs(scene.card.typed) do
            card[name] = value
        end
        for _, name in ipairs(scene.card.cleared) do
            card[name] = nil
        end
        env.card = card
        shown_as = copy(card)
    end

    handler(argument)

    -- What the script wrote on the card on screen and did not save.
    if env.card ~= nil and key_of[env.card] ~= nil then
        local changed, trouble, any = changes(env.card, shown_as)
        if trouble then
            return nil, trouble
        end
        if any then
            insert(effects, written_after + 1, { ["do"] = "set", values = changed })
        end
    end
    return effects
end

return { env = env, describe = describe, run = run }
