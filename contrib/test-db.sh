#!/usr/bin/env bash
# Starts (or stops) a throwaway PostgreSQL cluster for the database
# integration tests and prints the connection URL to export.
#
#   eval "$(contrib/test-db.sh start)"   # sets TEST_DATABASE_URL
#   cargo test --all-features
#   contrib/test-db.sh stop
#
# Needs initdb, pg_ctl, and psql in PATH (e.g. `nix-shell -p postgresql`).
set -euo pipefail

PORT="${TEST_DB_PORT:-54329}"
# Unix socket paths are limited to ~100 bytes, so keep the directory short.
DIR="${TEST_DB_DIR:-/tmp/mpo-test-db-$(id -u)}"
DATA="$DIR/data"
URL="postgres://postgres@127.0.0.1:$PORT/mpo_test"

case "${1:-}" in
  start)
    mkdir -p "$DIR"
    if [ ! -f "$DATA/PG_VERSION" ]; then
      initdb -D "$DATA" -U postgres --auth=trust >"$DIR/initdb.log" 2>&1
    fi
    if ! pg_ctl -D "$DATA" status >/dev/null 2>&1; then
      pg_ctl -D "$DATA" -l "$DIR/postgres.log" \
        -o "-k $DIR -p $PORT -c listen_addresses=127.0.0.1 -c fsync=off" \
        start >/dev/null
    fi
    for _ in $(seq 1 30); do
      if psql -h 127.0.0.1 -p "$PORT" -U postgres -c 'select 1' >/dev/null 2>&1; then
        break
      fi
      sleep 0.5
    done
    psql -h 127.0.0.1 -p "$PORT" -U postgres -tAc \
      "select 1 from pg_database where datname = 'mpo_test'" | grep -q 1 ||
      psql -h 127.0.0.1 -p "$PORT" -U postgres -c 'create database mpo_test' >/dev/null
    echo "export TEST_DATABASE_URL=$URL"
    ;;
  stop)
    pg_ctl -D "$DATA" stop >/dev/null 2>&1 || true
    ;;
  destroy)
    pg_ctl -D "$DATA" stop >/dev/null 2>&1 || true
    rm -rf "$DIR"
    ;;
  *)
    echo "usage: $0 start|stop|destroy" >&2
    exit 1
    ;;
esac
