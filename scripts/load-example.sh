#!/usr/bin/env bash
#
# Loads an example stack into the development server.
#
#   scripts/load-example.sh addressbook
#
# Examples are not part of pgXila: they show what a user builds with it.
# PGXILA_SERVICE selects another Compose service than the development server.
set -Eeuo pipefail
cd "$(dirname "$0")/.."

name="${1:?usage: scripts/load-example.sh <example>   (see the examples directory)}"
file="examples/$name.sql"
[ -f "$file" ] || { echo "no such example: $file" >&2; exit 1; }

docker compose --profile test exec -T -u postgres -e PGOPTIONS='-c client_min_messages=warning' \
    "${PGXILA_SERVICE:-db}" \
    psql -X -q -v ON_ERROR_STOP=1 --single-transaction -d pgxila < "$file"
echo "Example '$name' loaded."
