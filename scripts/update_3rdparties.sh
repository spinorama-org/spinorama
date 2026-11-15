#!/bin/sh
set -eu

# warning this section is read by a python script (generate_html) to grab the versions

HANDLEBARS=4.7.10
BULMA=1.0.4
FUSE=7.5.0
# end section

ASSETS=./dist
ASSETS_JS=${ASSETS}/js3rd
ASSETS_CSS=${ASSETS}/css
ASSETS_JSON=${ASSETS}/json

mkdir -p ${ASSETS} ${ASSETS_JS} ${ASSETS_CSS} ${ASSETS_JSON}

# Dependencies are installed by make install / npm ci.
cp node_modules/handlebars/dist/handlebars.min.js "${ASSETS_JS}/handlebars-${HANDLEBARS}.min.js"

# BULMA: compile from SCSS to include both light and dark themes
npx --no-install sass --load-path=node_modules src/website/bulma4spin.scss dist/css/bulma-${BULMA}.min.css --style=compressed --no-source-map

# FUSE.JS
cp node_modules/fuse.js/dist/fuse.min.mjs ${ASSETS_JS}/fuse-${FUSE}.min.mjs


