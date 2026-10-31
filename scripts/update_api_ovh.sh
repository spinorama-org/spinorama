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

echo "Update starts"
export PYTHONPATH=src:src/website:src/spinorama:scripts:.

DISTDIR=/var/www/html/spinorama-api
RUNDIR=/home/spin/run/spin-api

DIST_TARGET=ovhspin:$DISTDIR
RUN_TARGET=ovhspin:$RUNDIR

rsync -arv ./scripts/gunicorn_start.sh requirements-api.txt requirements-api.lock $RUN_TARGET
rsync -arv ./datas/*.py $RUN_TARGET/datas
rsync -arv ./dist/json/metadata.json* ./dist/json/headphone.json* $DIST_TARGET/assets
rsync -arv ./datas/headphones/ $DIST_TARGET/assets/headphones
rsync -arv --exclude '__pycache__/' ./src/api "$RUN_TARGET/"
rsync ./conf/etc/supervisor/conf.d/spinorama-app.conf $RUN_TARGET/etc
rsync ./conf/etc/nginx/sites-available/spinorama-api "$RUN_TARGET/etc/spinorama-api.nginx"
rsync ./conf/etc/nginx/snippets/deny-repository-metadata.conf "$RUN_TARGET/etc/"

echo "1. as spin user"
echo "cd $RUNDIR && python3 -m venv .venv && source .venv/bin/activate && pip install -U 'pip>=26.2' && pip install -U -r requirements-api.lock"

echo "2. Initialize the key database once: cd $RUNDIR && .venv/bin/python -m api.keys init"
echo "The updated API refuses all requests until authentication is configured."
echo "3. Install the proxy configuration and restart (see api/README.md for key administration):"
echo "sudo cp $RUNDIR/etc/spinorama-app.conf /etc/supervisor/conf.d/"
echo "sudo install -m 644 $RUNDIR/etc/deny-repository-metadata.conf /etc/nginx/snippets/deny-repository-metadata.conf"
echo "sudo cp $RUNDIR/etc/spinorama-api.nginx /etc/nginx/sites-available/spinorama-api"
echo "sudo nginx -t"
echo "sudo supervisorctl restart spinorama-api"
echo "sudo nginx -s reload"

exit 0;
