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
# Where the tests run their fake language model, for the agent worker of the
# scratch server to call. Docker Desktop reaches the loopback of the host; on
# Linux the container comes in through the Docker bridge.
MODEL_PORT="${PGXILA_TEST_MODEL_PORT:-5421}"
case "$(uname -s)" in
    Linux) MODEL_BIND="${PGXILA_TEST_MODEL_BIND:-0.0.0.0}" ;;
    *)     MODEL_BIND="${PGXILA_TEST_MODEL_BIND:-127.0.0.1}" ;;
esac
ROLE="xila_integration_test"
OUTSIDER="xila_integration_outsider"
AUTHOR="xila_integration_author"
EDITOR_ROLE="xila_integration_editor"
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

# On the scratch server: a pgXila user, two authors, a role that is neither,
# and a stack.
psql_in db-scratch -v role="$ROLE" -v outsider="$OUTSIDER" -v author="$AUTHOR" \
    -v editor="$EDITOR_ROLE" -v password="$PASSWORD" <<'SQL'
CREATE ROLE :"role" LOGIN PASSWORD :'password' IN ROLE xila_user;
CREATE ROLE :"author" LOGIN PASSWORD :'password' IN ROLE xila_author;
-- A second author, member of the first: both may edit what the first owns.
CREATE ROLE :"editor" LOGIN PASSWORD :'password' IN ROLE xila_author, :"author";
CREATE ROLE :"outsider" LOGIN PASSWORD :'password';
SQL
PGXILA_SERVICE=db-scratch scripts/load-example.sh addressbook

# The agent of the scratch server talks to the fake model of the tests.
psql_in db-scratch -v address="http://host.docker.internal:$MODEL_PORT/v1" <<'SQL'
SELECT xila.set_agent(jsonb_build_object(
    'address', :'address', 'model', 'fake', 'token_variable', 'PGXILA_TEST_MODEL_TOKEN'));
SQL

export PGXILA_TEST_HOST=localhost
export PGXILA_TEST_PORT="$PORT"
export PGXILA_TEST_DATABASE=pgxila
export PGXILA_TEST_USER="$ROLE"
export PGXILA_TEST_OUTSIDER="$OUTSIDER"
export PGXILA_TEST_AUTHOR="$AUTHOR"
export PGXILA_TEST_EDITOR="$EDITOR_ROLE"
export PGXILA_TEST_PASSWORD="$PASSWORD"
export PGXILA_TEST_CA="$PWD/docker/certs/ca.crt"
export PGXILA_TEST_SCRATCH_PORT="$SCRATCH_PORT"
export PGXILA_TEST_SCRATCH_CA="$SCRATCH_CERTS/ca.crt"
export PGXILA_TEST_MODEL_PORT="$MODEL_PORT"
export PGXILA_TEST_MODEL_BIND="$MODEL_BIND"

# The API tests share one stack: run them one at a time.
cargo test --workspace -- --ignored --test-threads=1 "$@"

scratch() { # runs a command inside the scratch server, as the postgres user
    "${COMPOSE[@]}" exec -T -u postgres "$@"
}
history_count() {
    scratch db-scratch psql -X -At -d "${1:-pgxila}" -c "SELECT count(*) FROM xila.history"
}

echo
echo "History cannot be escaped by an ordinary role"
before="$(history_count)"
scratch -e PGPASSWORD="$PASSWORD" db-scratch \
    psql -X -q -v ON_ERROR_STOP=1 "host=localhost dbname=pgxila user=$ROLE sslmode=require" <<'SQL'
SET search_path = '';
SET xila.restoring = on;
UPDATE addressbook.contacts SET city = 'Ailleurs' WHERE last_name = 'Pascal';
UPDATE addressbook.contacts SET city = 'Clermont-Ferrand' WHERE last_name = 'Pascal';
SQL
after="$(history_count)"
[ "$after" -eq "$((before + 2))" ] || {
    echo "FAILED: expected 2 new history rows, got $((after - before))" >&2; exit 1; }
echo "ok"

echo
echo "A backup is restored with its stacks, its cards, its history and its conversations"
# A script, so that the backup has one to carry.
psql_in db-scratch <<'SQL'
SELECT xila.set_script('addressbook', 'new', $lua$
keys = { F2 = "New card" }

function on_click()
  go_to(stack.new_card())
end

function on_key(key)
  on_click()
end
$lua$);
SQL
summary="SELECT (SELECT count(*) FROM xila.stack), (SELECT count(*) FROM xila.background),
                (SELECT count(*) FROM xila.widget), (SELECT count(*) FROM xila.layout),
                (SELECT count(*) FROM xila.action), (SELECT count(*) FROM xila.key_binding),
                (SELECT count(*) FROM xila.palette), (SELECT count(*) FROM xila.history),
                (SELECT count(*) FROM addressbook.contacts),
                (SELECT count(*) FROM xila.image), (SELECT count(photo) FROM addressbook.contacts),
                (SELECT md5(string_agg(i.pixels::text, '' ORDER BY c.last_name))
                   FROM addressbook.contacts c JOIN xila.image i ON i.id = c.photo),
                (SELECT count(*) FROM pg_trigger WHERE tgname = 'xila_card'),
                (SELECT count(*) || ':' || md5(string_agg(source || events::text || keys::text, ''))
                   FROM xila.script),
                xila.open_stack('addressbook')->'keys' @> '[{\"key\": \"F2\", \"action\": \"script\"}]',
                xila.card('addressbook')->'texts',
                (SELECT md5(address || model || token_variable) FROM xila.agent_setting),
                (SELECT count(*) FROM xila.conversation), (SELECT count(*) FROM xila.agent_message),
                (SELECT count(*) FROM xila.agent_log),
                (SELECT count(*) || ':' || count(*) FILTER (WHERE state = 'applied')
                   FROM xila.agent_proposal),
                has_table_privilege('xila_agent', 'addressbook.contacts', 'SELECT')"
scratch db-scratch sh -c '
    set -e
    pg_dump -d pgxila -Fc -f /tmp/pgxila.dump
    createdb restored
    pg_restore --exit-on-error -d restored /tmp/pgxila.dump'
original="$(scratch db-scratch psql -X -At -d pgxila -c "$summary")"
restored="$(scratch db-scratch psql -X -At -d restored -c "$summary")"
[ "$original" = "$restored" ] || {
    echo "FAILED: the restored database differs" >&2
    echo "  original: $original" >&2
    echo "  restored: $restored" >&2
    exit 1; }
echo "ok ($restored)"
