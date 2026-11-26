#!/usr/bin/env bash
#
# Tells the agent of the development server which model to call.
#
#   scripts/set-agent.sh <address> <model> [token-variable]
#
#   scripts/set-agent.sh https://openrouter.ai/api/v1 some/model OPENROUTER_API_KEY
#   scripts/set-agent.sh http://host.docker.internal:1234/v1 local-model
#
# The address is the base address of an API that speaks the OpenAI chat API.
# The token is never given here: token-variable is the NAME of the environment
# variable of the server that holds it. With Docker Compose, put the variable
# in .env.local and restart the server. Leave it out for a model that needs no
# token, such as one that runs on this machine.
#
# An administrator can do the same from the client: F9, F10, then F4.
# PGXILA_SERVICE selects another Compose service than the development server.
set -Eeuo pipefail
cd "$(dirname "$0")/.."

usage="usage: scripts/set-agent.sh <address> <model> [token-variable]"
address="${1:?$usage}"
model="${2:?$usage}"
variable="${3:-}"

docker compose --profile test exec -T -u postgres "${PGXILA_SERVICE:-db}" \
    psql -X -q -v ON_ERROR_STOP=1 -d pgxila \
    -v address="$address" -v model="$model" -v variable="$variable" <<'SQL'
SELECT xila.set_agent(jsonb_build_object(
    'address', :'address', 'model', :'model', 'token_variable', :'variable'));
SQL

echo "The agent calls '$model' at $address."
if [ -n "$variable" ]; then
    # Says whether the variable is set, never what it holds.
    if docker compose --profile test exec -T "${PGXILA_SERVICE:-db}" \
           sh -c 'test -n "$(printenv "$1")"' sh "$variable"; then
        echo "Its token is read from the variable $variable of the server, which is set."
    else
        echo "Its token is read from the variable $variable of the server, which is NOT set:"
        echo "put it in .env.local and run 'docker compose up -d' again."
    fi
fi
