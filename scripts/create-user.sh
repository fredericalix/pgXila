#!/usr/bin/env bash
#
# Creates a pgXila user on the development server: a PostgreSQL login role,
# member of xila_user. The password is asked for, never passed on the command
# line and never stored.
#
#   scripts/create-user.sh ada            a user
#   scripts/create-user.sh ada --author   a user who may also build stacks
#   scripts/create-user.sh ada --admin    an author who may also set up the agent
set -Eeuo pipefail
cd "$(dirname "$0")/.."

name="${1:?usage: scripts/create-user.sh <name> [--author | --admin]}"
group="xila_user"
[ "${2:-}" = "--author" ] && group="xila_author"
[ "${2:-}" = "--admin" ] && group="xila_admin"

read -r -s -p "Password for $name: " password; echo
read -r -s -p "Again: " again; echo
[ "$password" = "$again" ] || { echo "The passwords differ." >&2; exit 1; }
[ -n "$password" ] || { echo "The password is empty." >&2; exit 1; }

docker compose exec -T -u postgres db \
    psql -X -q -v ON_ERROR_STOP=1 -d pgxila -v name="$name" -v group="$group" -v password="$password" <<'SQL'
CREATE ROLE :"name" LOGIN PASSWORD :'password' IN ROLE :"group";
SQL
echo "User '$name' created (member of $group)."
