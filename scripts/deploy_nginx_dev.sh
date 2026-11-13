#!/bin/sh
# Install the repository's dev nginx policy using the administrator account.
set -eu

DEV_NGINX_HOST=${DEV_NGINX_HOST:-web01.local}
REPO_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
stage=$(ssh -o BatchMode=yes -o ConnectTimeout=10 "$DEV_NGINX_HOST" 'mktemp -d /tmp/spinorama-nginx-dev.XXXXXX')
trap 'ssh -o BatchMode=yes -o ConnectTimeout=10 "$DEV_NGINX_HOST" "rm -rf -- $stage"' EXIT

scp -o BatchMode=yes -o ConnectTimeout=10 "$REPO_ROOT/conf/etc/nginx/sites-available/spinorama-dev" \
    "$REPO_ROOT/conf/etc/nginx/snippets/spinorama-dev-cache.conf" \
    "$DEV_NGINX_HOST:$stage/"

ssh -o BatchMode=yes -o ConnectTimeout=10 "$DEV_NGINX_HOST" sh -s -- "$stage" <<'REMOTE'
set -eu
stage=$1
site=/etc/nginx/sites-available/spinorama-dev
snippet=/etc/nginx/snippets/spinorama-dev-cache.conf
sudo -n cp -a "$site" "$stage/site.backup"
if sudo -n test -e "$snippet"; then
    sudo -n cp -a "$snippet" "$stage/snippet.backup"
fi
restore() {
    sudo -n cp -a "$stage/site.backup" "$site"
    if [ -e "$stage/snippet.backup" ]; then
        sudo -n cp -a "$stage/snippet.backup" "$snippet"
    else
        sudo -n rm -f "$snippet"
    fi
}
trap restore EXIT
sudo -n install -m 644 "$stage/spinorama-dev-cache.conf" "$snippet"
sudo -n install -m 644 "$stage/spinorama-dev" "$site"
sudo -n nginx -t
sudo -n systemctl reload nginx
trap - EXIT
echo 'Installed dev nginx configuration and reloaded nginx.'
REMOTE
