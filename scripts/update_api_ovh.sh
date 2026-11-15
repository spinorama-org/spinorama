#!/bin/bash
# A library to display spinorama charts
#
# Copyright (C) 2020-2025 Pierre Aubert pierre(at)spinorama(dot)org
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <https://www.gnu.org/licenses/>.

set -euo pipefail
shopt -s nullglob

echo "Update starts"
export PYTHONPATH=src:src/website:src/spinorama:scripts:.

# Private API data lives outside every nginx document root. The API
# and image-rendering binaries read it through SPINORAMA_API_FILES /
# SPINORAMA_SPINFILES; nginx only proxies their sockets.
DATA_DIR=${SPINORAMA_DATA_DIR:-/home/spin/spinorama-data/api}
case "$DATA_DIR" in
    /var/www|/var/www/*|[^/]*|'')
        echo "KO: SPINORAMA_DATA_DIR must stay outside nginx roots: $DATA_DIR" >&2
        exit 2
        ;;
esac
# Paths are embedded in remote shell commands and must be canonical absolute paths.
if [[ "$DATA_DIR" =~ [^a-zA-Z0-9_./-] ]] || [[ "$DATA_DIR/" == *"/../"* ]] || [[ "$DATA_DIR/" == *"/./"* ]]; then
    echo "KO: invalid SPINORAMA_DATA_DIR" >&2
    exit 2
fi
RUNDIR=/home/spin/run/spin-api
MANIFEST=$(mktemp)
trap 'rm -f "$MANIFEST"' EXIT
python3 "$(dirname "$0")/api_staging_manifest.py" "$DATA_DIR" "$RUNDIR" > "$MANIFEST"

DATA_TARGET=ovhspin:$DATA_DIR
RUN_TARGET=ovhspin:$RUNDIR

# Older runs could leave $RUNDIR/etc as a regular file (rsync created
# it when the directory did not exist yet); move it aside.
ssh ovhspin "mkdir -p '$DATA_DIR/assets/headphones' '$DATA_DIR/speakers' \
    '$DATA_DIR/headphones-graphs' '$RUNDIR/datas' && \
    if [ -f '$RUNDIR/etc' ]; then mv '$RUNDIR/etc' '$RUNDIR/etc.conf.bak'; fi && \
    mkdir -p '$RUNDIR/etc'"

rsync -arv ./scripts/spinorama-api-start.sh $RUN_TARGET
rsync -arv ./datas/origins.toml $RUN_TARGET/datas/
rsync -arv ./dist/json/metadata*.json* ./dist/json/headphone*.json* \
    ./dist/json/eqdata.json* ./dist/json/headphone_eqdata.json* $DATA_TARGET/assets/
rsync -arv ./datas/headphones/ $DATA_TARGET/assets/headphones/
# Pristine curve JSON and trace bundles (private; the API and the compare
# renderer read them, the public website never does).
rsync -arv ./dist-private/speakers/ $DATA_TARGET/speakers/
if [ -d ./dist-private/headphones ]; then
    rsync -arv ./dist-private/headphones/ "$DATA_TARGET/headphones-graphs/"
fi
rsync ./conf/etc/supervisor/conf.d/spinorama-app.conf $RUN_TARGET/etc/
rsync ./conf/etc/nginx/sites-available/spinorama-api "$RUN_TARGET/etc/spinorama-api.nginx"
rsync ./conf/etc/nginx/sites-available/spinorama-prod "$RUN_TARGET/etc/spinorama-prod.nginx"
rsync ./conf/etc/nginx/conf.d/bot-protection.conf "$RUN_TARGET/etc/"
rsync ./conf/etc/nginx/snippets/bot-protection-server.conf "$RUN_TARGET/etc/"
rsync ./conf/etc/nginx/snippets/spinorama-banned.conf "$RUN_TARGET/etc/"
rsync ./conf/etc/nginx/snippets/deny-repository-metadata.conf "$RUN_TARGET/etc/"

# Verify the destination bytes, including private trees, before giving activation instructions.
rsync "$MANIFEST" "$RUN_TARGET/staging.sha256"
ssh ovhspin "sha256sum -c '$RUNDIR/staging.sha256'"
echo "OK: staged content verified against SHA-256 manifest"

echo "1. Deploy the Rust API binary (built on the dev host): ./scripts/deploy_binaries.sh prod"
echo "   The binary lands in ~/spinorama-bin/spinorama-api; no Python venv is needed anymore."

echo "2. Initialize the key database once: ~/spinorama-bin/spinorama-api keys init"
echo "The updated API refuses all requests until authentication is configured."
echo "3. Install the proxy configuration and restart as the ubuntu admin user"
echo "   (spin has no sudo; Supervisor itself is controlled as spin):"
echo "sudo cp $RUNDIR/etc/spinorama-app.conf /etc/supervisor/conf.d/"
echo "sudo install -m 644 $RUNDIR/etc/deny-repository-metadata.conf /etc/nginx/snippets/deny-repository-metadata.conf"
echo "sudo install -m 644 $RUNDIR/etc/bot-protection.conf /etc/nginx/conf.d/bot-protection.conf"
echo "sudo install -m 644 $RUNDIR/etc/bot-protection-server.conf /etc/nginx/snippets/bot-protection-server.conf"
echo "sudo test -e /etc/nginx/snippets/spinorama-banned.conf || sudo install -m 644 $RUNDIR/etc/spinorama-banned.conf /etc/nginx/snippets/spinorama-banned.conf"
echo "sudo cp $RUNDIR/etc/spinorama-api.nginx /etc/nginx/sites-available/spinorama-api"
echo "sudo cp $RUNDIR/etc/spinorama-prod.nginx /etc/nginx/sites-available/spinorama-prod"
echo "sudo groupadd -f spinorama-proxy"
echo "sudo usermod -aG spinorama-proxy spin"
echo "sudo usermod -aG spinorama-proxy www-data"
echo "sudo install -d -o spin -g spinorama-proxy -m 2750 /home/spin/run/api-proxy"
echo "sudo install -d -o spin -g spin -m 700 $RUNDIR/accounts /home/spin/log"
echo "sudo chmod 600 $RUNDIR/accounts/keys.sqlite3* /home/spin/log/api.log*"
echo "sudo nginx -t"
echo "sudo supervisorctl restart spinorama-api"
echo "sudo systemctl restart nginx # refresh proxy group membership"
echo "4. Set SPINORAMA_FINGERPRINT_SECRET in the API service environment (keep it stable and private), then restart the API."

exit 0;
