#!/usr/bin/env bash
#
# Loads an example stack into the development server.
#
#   scripts/load-example.sh carnet
#
# Examples are not part of pgXila: they show what a user builds with it.
set -Eeuo pipefail
cd "$(dirname "$0")/.."

name="${1:?usage: scripts/load-example.sh <example>   (see the examples directory)}"
file="examples/$name.sql"
[ -f "$file" ] || { echo "no such example: $file" >&2; exit 1; }

docker compose exec -T -u postgres db \
    psql -X -q -v ON_ERROR_STOP=1 --single-transaction -d pgxila < "$file"
echo "Example '$name' loaded."
