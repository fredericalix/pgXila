#!/usr/bin/env bash
#
# Runs the integration tests against real pgXila servers in Docker.
#
# It starts the development server and a throwaway "permissive" server (see
# docker/test/pg_hba.permissive.conf), creates a temporary role with a random
# password on both, runs the ignored tests of the workspace, then removes the
# role and the throwaway server.
set -Eeuo pipefail
cd "$(dirname "$0")/.."

PORT="${PGXILA_PORT:-5419}"
PERMISSIVE_PORT="${PGXILA_PERMISSIVE_PORT:-5420}"
ROLE="xila_integration_test"
PASSWORD="$(openssl rand -hex 24)"
COMPOSE=(docker compose --profile test)

psql_in() { # service, then psql arguments; SQL on stdin
    local service="$1"; shift
    "${COMPOSE[@]}" exec -T -u postgres "$service" psql -X -q -v ON_ERROR_STOP=1 -d pgxila "$@"
}

cleanup() {
    psql_in db -c "DROP ROLE IF EXISTS $ROLE" >/dev/null 2>&1 || true
    "${COMPOSE[@]}" rm -f -s -v db-permissive >/dev/null 2>&1 || true
    rm -rf "$PERMISSIVE_CERTS"
}
PERMISSIVE_CERTS="$(mktemp -d)"
trap cleanup EXIT
export PGXILA_PERMISSIVE_CERTS="$PERMISSIVE_CERTS"

"${COMPOSE[@]}" up -d --build --wait db db-permissive

for service in db db-permissive; do
    psql_in "$service" -v role="$ROLE" -v password="$PASSWORD" <<'SQL'
DROP ROLE IF EXISTS :"role";
CREATE ROLE :"role" LOGIN PASSWORD :'password';
SQL
done

export PGXILA_TEST_HOST=localhost
export PGXILA_TEST_PORT="$PORT"
export PGXILA_TEST_DATABASE=pgxila
export PGXILA_TEST_USER="$ROLE"
export PGXILA_TEST_PASSWORD="$PASSWORD"
export PGXILA_TEST_CA="$PWD/docker/certs/ca.crt"
export PGXILA_TEST_PERMISSIVE_PORT="$PERMISSIVE_PORT"
export PGXILA_TEST_PERMISSIVE_CA="$PERMISSIVE_CERTS/ca.crt"

cargo test --workspace -- --ignored "$@"
