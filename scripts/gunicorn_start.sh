#!/bin/bash

NAME=spinorama-api
DIR=/home/spin/run/spin-api
USER=spin
GROUP=spin
WORKERS=${SPINORAMA_API_WORKERS:-2}
WORKER_CLASS=uvicorn.workers.UvicornWorker
VENV=$DIR/.venv/bin/activate
BIND=unix:/home/spin/run/gunicorn.sock
LOG_LEVEL=info

set -eu
cd "$DIR"
source "$VENV"
# SQLite is outside the served data tree and preserved across deployments.
export SPINORAMA_API_KEY_DB="${SPINORAMA_API_KEY_DB:-$DIR/accounts/keys.sqlite3}"

export PYTHONPATH=$DIR

exec gunicorn api.main:app \
  --name $NAME \
  --workers $WORKERS \
  --worker-class $WORKER_CLASS \
  --user=$USER \
  --group=$GROUP \
  --bind=$BIND \
  --log-level=$LOG_LEVEL \
  --log-file=/home/spin/log/api.log
