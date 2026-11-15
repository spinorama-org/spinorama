#!/bin/sh
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

# TARGET=$HOME/src/spinorama-org/spinorama
# TARGET=/var/www/html/spinorama-prod
# TARGET=/var/www/html/spinorama-dev
# TARGET=pierre@ch.spinorama.org:/var/www/html/spinorama-dev
# TARGET=pierre@es.spinorama.org:/var/www/html/spinorama-dev
#TARGET=pierre@dev.spinorama.org:/var/www/html/spinorama-dev
# TARGET=pierre@web:/var/www/html/spinorama-dev
TARGET=web01.spin:/var/www/html/spinorama-dev
# check
command=$(grep www.spinorama.org dist/*.html | wc -l)
if [ $command -ne 0 ]; then
    echo "KO found prod url in dev site"
    exit 1;
else
    echo "OK checking for prod site in dev"
fi
# the JS bundle embeds the site URL too (meta-*.js); a prod-flavoured
# build deployed to dev makes the frontend fetch prod JSON cross-origin
command=$(grep -l "https://www.spinorama.org" dist/js/meta-*.js dist/js/meta-*.min.js 2>/dev/null | wc -l)
if [ $command -ne 0 ]; then
    echo "KO found prod url in dev JS bundle (dist/js/meta-*)"
    exit 1;
else
    echo "OK checking for prod site in dev JS bundle"
fi
command=$(grep spinorama.internet-box.ch dist/*.html | wc -l)
if [ $command -ne 0 ]; then
    echo "KO found old dev url in prod site"
    exit 1;
else
    echo "OK checking for dev site in prod"
fi
python3 ./scripts/check_public_dist.py ./dist || exit 1
# Refresh the private bundles used to render graph-option changes too.
sh "$(dirname "$0")/update_graph_data_dev.sh" || exit 1
# Install and validate the dev cache policy before uploading site assets.
sh "$(dirname "$0")/deploy_nginx_dev.sh" || exit 1
# copy
echo "Sync"
rsync -arvz --delete ./dist/ "$TARGET/" || exit 1
changes=$(rsync -rcln --delete --itemize-changes ./dist/ "$TARGET/") || exit 1
if [ -n "$changes" ]; then
    echo "KO: destination differs from the checked public tree" >&2
    printf '%s\n' "$changes" >&2
    exit 1
fi
echo "OK: destination contents verified"
