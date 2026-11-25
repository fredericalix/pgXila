#!/usr/bin/env bash
#
# Runs the integration tests against real pgXila servers in Docker.
#
# Two servers are used:
#
#   db          the development server, with the configuration of the image.
#               Only connection tests run against it: they create a temporary
#               role and touch no data.
#   db-scratch  a throwaway server whose data is not kept. The example stack
#               is loaded into it and the API tests read and write there. Its
#               pg_hba.conf also accepts unencrypted connections, to prove
#               that the extension refuses them by itself.
#
# Roles get a random password, and are removed when the tests end.
set -Eeuo pipefail
cd "$(dirname "$0")/.."

PORT="${PGXILA_PORT:-5419}"
SCRATCH_PORT="${PGXILA_SCRATCH_PORT:-5420}"
ROLE="xila_integration_test"
OUTSIDER="xila_integration_outsider"
PASSWORD="$(openssl rand -hex 24)"
COMPOSE=(docker compose --profile test)

psql_in() { # service, then psql arguments; SQL on stdin
    local service="$1"; shift
    "${COMPOSE[@]}" exec -T -u postgres -e PGOPTIONS='-c client_min_messages=warning' "$service" \
        psql -X -q -v ON_ERROR_STOP=1 -d pgxila "$@"
}

cleanup() {
    psql_in db -c "DROP ROLE IF EXISTS $ROLE" >/dev/null 2>&1 || true
    "${COMPOSE[@]}" rm -f -s -v db-scratch >/dev/null 2>&1 || true
    rm -rf "$SCRATCH_CERTS"
}
SCRATCH_CERTS="$(mktemp -d)"
trap cleanup EXIT
export PGXILA_SCRATCH_CERTS="$SCRATCH_CERTS"

"${COMPOSE[@]}" up -d --build --wait db db-scratch

# A plain login role on the development server: enough to test connections.
psql_in db -v role="$ROLE" -v password="$PASSWORD" <<'SQL'
DROP ROLE IF EXISTS :"role";
CREATE ROLE :"role" LOGIN PASSWORD :'password';
SQL

# On the scratch server: a pgXila user, a role that is not one, and a stack.
psql_in db-scratch -v role="$ROLE" -v outsider="$OUTSIDER" -v password="$PASSWORD" <<'SQL'
CREATE ROLE :"role" LOGIN PASSWORD :'password' IN ROLE xila_user;
CREATE ROLE :"outsider" LOGIN PASSWORD :'password';
SQL
PGXILA_SERVICE=db-scratch scripts/load-example.sh carnet

export PGXILA_TEST_HOST=localhost
export PGXILA_TEST_PORT="$PORT"
export PGXILA_TEST_DATABASE=pgxila
export PGXILA_TEST_USER="$ROLE"
export PGXILA_TEST_OUTSIDER="$OUTSIDER"
export PGXILA_TEST_PASSWORD="$PASSWORD"
export PGXILA_TEST_CA="$PWD/docker/certs/ca.crt"
export PGXILA_TEST_SCRATCH_PORT="$SCRATCH_PORT"
export PGXILA_TEST_SCRATCH_CA="$SCRATCH_CERTS/ca.crt"

# The API tests share one stack: run them one at a time.
cargo test --workspace -- --ignored --test-threads=1 "$@"
