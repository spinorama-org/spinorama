#!/bin/sh
# Install the repository's dev nginx policy using the administrator account.
set -eu

DEV_NGINX_HOST=${DEV_NGINX_HOST:-web01.local}
REPO_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
stage=$(ssh -o BatchMode=yes -o ConnectTimeout=10 "$DEV_NGINX_HOST" 'mktemp -d /tmp/spinorama-nginx-dev.XXXXXX')
trap 'ssh -o BatchMode=yes -o ConnectTimeout=10 "$DEV_NGINX_HOST" "rm -rf -- $stage"' EXIT

scp -o BatchMode=yes -o ConnectTimeout=10 "$REPO_ROOT/conf/etc/nginx/sites-available/spinorama-dev" \
    "$REPO_ROOT/conf/etc/nginx/snippets/spinorama-dev-cache.conf" \
    "$REPO_ROOT/conf/etc/nginx/snippets/bot-protection-server.conf" \
    "$REPO_ROOT/conf/etc/nginx/snippets/deny-repository-metadata.conf" \
    "$REPO_ROOT/conf/etc/nginx/conf.d/bot-protection.conf" \
    "$DEV_NGINX_HOST:$stage/"

ssh -o BatchMode=yes -o ConnectTimeout=10 "$DEV_NGINX_HOST" sh -s -- "$stage" <<'REMOTE'
set -eu
stage=$1
configs="sites-available/spinorama-dev snippets/spinorama-dev-cache.conf snippets/bot-protection-server.conf snippets/deny-repository-metadata.conf conf.d/bot-protection.conf"
for relative in $configs; do
    destination=/etc/nginx/$relative
    name=${relative##*/}
    if sudo -n test -e "$destination"; then
        sudo -n cp -a "$destination" "$stage/$name.backup"
    fi
done
restore() {
    for relative in $configs; do
        destination=/etc/nginx/$relative
        name=${relative##*/}
        if [ -e "$stage/$name.backup" ]; then
            sudo -n cp -a "$stage/$name.backup" "$destination"
        else
            sudo -n rm -f "$destination"
        fi
    done
}
trap restore EXIT
for relative in $configs; do
    sudo -n install -m 644 "$stage/${relative##*/}" "/etc/nginx/$relative"
done
# The live ban list is deliberately preserved.
sudo -n nginx -t
sudo -n systemctl reload nginx
trap - EXIT
echo 'Installed dev nginx configuration and reloaded nginx.'
REMOTE
