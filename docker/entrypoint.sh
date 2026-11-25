#!/usr/bin/env bash
#
# Entrypoint of the pgXila server image.
#
# Before handing over to the stock PostgreSQL entrypoint, it makes sure the
# server has what the mandatory-TLS configuration needs:
#
#   - a TLS certificate and key. If none is present, a development
#     certificate authority and a server certificate are generated. Private
#     keys stay inside the state volume and are never exported.
#   - a superuser password. It is random and nobody needs to know it:
#     administration goes through the Unix socket inside the container.
#
# Only the public CA certificate is copied to /export (when that directory is
# mounted) so that clients on the host can verify the server.
set -Eeuo pipefail

STATE_DIR=/var/lib/pgxila
TLS_DIR="$STATE_DIR/tls"
SECRETS_DIR="$STATE_DIR/secrets"
EXPORT_DIR=/export
TLS_HOSTS="${PGXILA_TLS_HOSTS:-localhost,127.0.0.1,::1}"

generate_certificates() {
    local work san="" host
    work="$(mktemp -d)"

    IFS=',' read -ra hosts <<< "$TLS_HOSTS"
    for host in "${hosts[@]}"; do
        host="${host// /}"
        [ -n "$host" ] || continue
        if [[ "$host" =~ ^[0-9.]+$ || "$host" == *:* ]]; then
            san+="IP:$host,"
        else
            san+="DNS:$host,"
        fi
    done
    san="${san%,}"

    openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
        -keyout "$TLS_DIR/ca.key" -out "$TLS_DIR/ca.crt" -days 3650 \
        -subj "/CN=pgXila development CA" \
        -addext "basicConstraints=critical,CA:TRUE,pathlen:0" \
        -addext "keyUsage=critical,keyCertSign,cRLSign" 2>/dev/null

    openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
        -keyout "$TLS_DIR/server.key" -out "$work/server.csr" \
        -subj "/CN=pgXila server" 2>/dev/null

    cat > "$work/server.ext" <<EXT
basicConstraints=critical,CA:FALSE
keyUsage=critical,digitalSignature
extendedKeyUsage=serverAuth
subjectAltName=$san
EXT

    openssl x509 -req -in "$work/server.csr" -days 825 \
        -CA "$TLS_DIR/ca.crt" -CAkey "$TLS_DIR/ca.key" -CAcreateserial \
        -extfile "$work/server.ext" -out "$TLS_DIR/server.crt" 2>/dev/null

    rm -rf "$work"
    echo "pgxila: generated a development CA and a server certificate for: $san"
}

if [ "$(id -u)" = "0" ]; then
    mkdir -p "$TLS_DIR" "$SECRETS_DIR"

    if [ ! -s "$TLS_DIR/server.crt" ] || [ ! -s "$TLS_DIR/server.key" ]; then
        generate_certificates
    fi

    if [ ! -s "$SECRETS_DIR/postgres-password" ]; then
        openssl rand -base64 32 > "$SECRETS_DIR/postgres-password"
    fi

    chown -R postgres:postgres "$STATE_DIR"
    chmod 700 "$TLS_DIR" "$SECRETS_DIR"
    find "$TLS_DIR" "$SECRETS_DIR" -type f -exec chmod 600 {} +
    chmod 644 "$TLS_DIR"/*.crt

    # Export the public CA certificate only. Never the keys.
    if [ -d "$EXPORT_DIR" ] && [ -s "$TLS_DIR/ca.crt" ]; then
        cp "$TLS_DIR/ca.crt" "$EXPORT_DIR/ca.crt"
        chmod 644 "$EXPORT_DIR/ca.crt"
    fi
fi

export POSTGRES_PASSWORD_FILE="${POSTGRES_PASSWORD_FILE:-$SECRETS_DIR/postgres-password}"

exec docker-entrypoint.sh "$@"
