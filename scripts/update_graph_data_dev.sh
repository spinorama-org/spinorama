#!/bin/sh
# Keep dynamic graph inputs private and in sync with the public dev pictures.
set -eu

REPO_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
HOST=${DEV_API_HOST:-web01.spin}
DATA_DIR=${SPINORAMA_DATA_DIR:-/home/spin/spinorama-data/api}
RUN_DIR=/home/spin/run/spin-api

case "$DATA_DIR" in
    /var/www|/var/www/*|*[!a-zA-Z0-9_./-]*|'')
        echo "Private graph data requires a path outside nginx roots: $DATA_DIR" >&2
        exit 2 ;;
    /*) ;;
    *) echo "SPINORAMA_DATA_DIR must be absolute" >&2; exit 2 ;;
esac
if [ ! -d "$REPO_ROOT/dist-private/speakers" ]; then
    echo "Missing private graph bundles; run make graphs before deployment." >&2
    exit 1
fi

ssh "$HOST" "mkdir -p '$DATA_DIR/speakers' '$DATA_DIR/assets' '$RUN_DIR'"
# The dynamic renderer reads trace bundles, not the larger Plotly payloads.
rsync -az --prune-empty-dirs --include='*/' --include='**/traces/***' --exclude='*' \
    "$REPO_ROOT/dist-private/speakers/" "$HOST:$DATA_DIR/speakers/"
rsync -az --include='metadata*.json*' --include='headphone*.json*' --exclude='*' \
    "$REPO_ROOT/dist/json/" "$HOST:$DATA_DIR/assets/"
rsync -az "$REPO_ROOT/scripts/spinorama-api-start.sh" "$HOST:$RUN_DIR/"
echo "Updated private dev graph inputs at $HOST:$DATA_DIR"
