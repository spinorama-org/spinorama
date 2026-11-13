#!/bin/bash
# Start the Rust spinorama API server (replaces gunicorn_start.sh).
#
# Single process, thread-per-connection, no worker pool: the API
# serves a few requests per second and the goal is a small resident
# set on the 2 GB VPS, not throughput.
set -eu

DIR=${SPINORAMA_API_DIR:-/home/spin/run/spin-api}
# Supervisor does not set $HOME, so default from the service home.
SERVICE_HOME=${HOME:-/home/spin}
BIN=${SPINORAMA_API_BIN:-$SERVICE_HOME/spinorama-bin/spinorama-api}
# Private data read by this binary only; never below an nginx root.
DATA_DIR=${SPINORAMA_DATA_DIR:-$SERVICE_HOME/spinorama-data/api}

export SPINORAMA_API_FILES="${SPINORAMA_API_FILES:-$DATA_DIR}"
export SPINORAMA_SPINFILES="${SPINORAMA_SPINFILES:-$SPINORAMA_API_FILES/speakers}"
export SPINORAMA_API_KEY_DB="${SPINORAMA_API_KEY_DB:-$DIR/accounts/keys.sqlite3}"
export SPINORAMA_ORIGINS_TOML="${SPINORAMA_ORIGINS_TOML:-$DIR/datas/origins.toml}"
# Nginx upstream stays on the historical socket path.
export SPINORAMA_API_BIND="${SPINORAMA_API_BIND:-unix:/home/spin/run/gunicorn.sock}"
# On-the-fly /compare.webp pictures are cached here (content-hashed);
# the API renders at most SPINORAMA_COMPARE_MAX_RENDERS_PER_MIN per
# minute (default 20) and answers 503 + Retry-After beyond that.
export SPINORAMA_COMPARE_CACHE="${SPINORAMA_COMPARE_CACHE:-$DIR/cache/compare}"
# SPINORAMA_FINGERPRINT_SECRET must be set in the service environment
# (supervisor/systemd) to enable per-key fingerprinting.

cd "$DIR"
exec "$BIN" serve
