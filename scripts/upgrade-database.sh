#!/usr/bin/env bash
#
# Moves the database of the development server to the version of pgXila that
# the server image now has, after "docker compose up --build".
#
#   scripts/upgrade-database.sh
#
# The extension has no upgrade script: a new version is a new extension. This
# does what a backup and a restore do, in place. The database is saved with
# pg_dump, restored into a new database, where the extension is created at
# its new version, and the two are swapped. Stacks, cards, history, pictures,
# scripts and conversations come along; the roles and their passwords belong
# to the server and are not touched.
#
# Nothing is deleted: the database as it was is kept under the name
# pgxila_before_upgrade. Once the new one is checked, remove the old one with
#
#   docker compose exec -u postgres db dropdb pgxila_before_upgrade
#
# PGXILA_SERVICE selects another Compose service than the development server.
set -Eeuo pipefail
cd "$(dirname "$0")/.."

service="${PGXILA_SERVICE:-db}"
in_db() { docker compose --profile test exec -T -u postgres "$service" "$@"; }
sql() { in_db psql -X -At -v ON_ERROR_STOP=1 -d "$1" -c "$2"; }

if [ "$(sql postgres "SELECT count(*) FROM pg_database WHERE datname = 'pgxila_before_upgrade'")" != "0" ]; then
    echo "A database named pgxila_before_upgrade is still there, from an earlier upgrade." >&2
    echo "Check the current one, remove the old one (see the top of this script), and run again." >&2
    exit 1
fi

before="$(sql pgxila "SELECT xila.hello()->>'api'")"
echo "Saving the database (API version $before)..."
in_db sh -c 'pg_dump -d pgxila -Fc -f /tmp/pgxila-upgrade.dump'

echo "Restoring it with the extension of this server image..."
in_db dropdb --if-exists pgxila_upgrading
in_db createdb pgxila_upgrading
in_db pg_restore --exit-on-error -d pgxila_upgrading /tmp/pgxila-upgrade.dump
in_db rm -f /tmp/pgxila-upgrade.dump
after="$(sql pgxila_upgrading "SELECT xila.hello()->>'api'")"

# The swap. Nobody may be connected to a database that is renamed: the
# clients are disconnected, and the agent worker comes back by itself.
echo "Swapping the databases..."
in_db psql -X -q -v ON_ERROR_STOP=1 -d postgres <<'SQL'
ALTER DATABASE pgxila ALLOW_CONNECTIONS false;
DO $$
BEGIN
    PERFORM pg_terminate_backend(pid) FROM pg_stat_activity
    WHERE datname IN ('pgxila', 'pgxila_upgrading') AND pid <> pg_backend_pid();
    PERFORM pg_sleep(0.5);
END
$$;
BEGIN;
ALTER DATABASE pgxila RENAME TO pgxila_before_upgrade;
ALTER DATABASE pgxila_upgrading RENAME TO pgxila;
COMMIT;
ALTER DATABASE pgxila_before_upgrade ALLOW_CONNECTIONS true;
SQL

echo "Done: the database went from API version $before to $after."
echo "The database as it was is kept as pgxila_before_upgrade."
