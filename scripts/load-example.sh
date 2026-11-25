#!/usr/bin/env bash
#
# Loads an example stack into the development server.
#
#   scripts/load-example.sh addressbook                the stack belongs to the administrator
#   scripts/load-example.sh addressbook --owner ada    the stack belongs to ada, an author,
#                                                      who can then change it in author mode
#
# Examples are not part of pgXila: they show what a user builds with it.
# PGXILA_SERVICE selects another Compose service than the development server.
set -Eeuo pipefail
cd "$(dirname "$0")/.."

usage="usage: scripts/load-example.sh <example> [--owner <role>]   (see the examples directory)"
name="${1:?$usage}"
owner=""
if [ "${2:-}" = "--owner" ]; then
    owner="${3:?$usage}"
fi
file="examples/$name.sql"
[ -f "$file" ] || { echo "no such example: $file" >&2; exit 1; }

run_sql() {
    docker compose --profile test exec -T -u postgres \
        -e PGOPTIONS='-c client_min_messages=warning' "${PGXILA_SERVICE:-db}" \
        psql -X -q -v ON_ERROR_STOP=1 --single-transaction -d pgxila "$@"
}

run_sql < "$file"

# Handing the stack over: its schema, its table and its definition. The stack
# of an example has the name of the example.
if [ -n "$owner" ]; then
    run_sql -v stack="$name" -v owner="$owner" <<'SQL'
SELECT format('ALTER SCHEMA %I OWNER TO %I', b.source_schema, :'owner'),
       format('ALTER TABLE %I.%I OWNER TO %I', b.source_schema, b.source_table, :'owner')
FROM xila.background b JOIN xila.stack s ON s.id = b.stack_id
WHERE s.name = :'stack' \gexec
UPDATE xila.stack SET owner = :'owner' WHERE name = :'stack';
SQL
    echo "Example '$name' loaded. It belongs to $owner."
else
    echo "Example '$name' loaded."
fi
