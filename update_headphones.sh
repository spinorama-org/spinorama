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
cd -- "$(dirname -- "$0")"

usage() {
    cat <<'USAGE'
Usage: ./update_headphones.sh [dev|prod] [--force] [--headphone NAME] [--jobs N]
                             [--skip-build] [--skip-deploy]
                             [--skip-catalogue-refresh] [--skip-check-html]

Build Rust generators and the GPUI renderer, refresh the ASR catalogue,
compute EQs, generate metadata and pictures, build HTML, and deploy to dev.
--skip-catalogue-refresh reuses the local headphone catalogue without ASR access.
USAGE
}

SITE_TARGET=dev
SKIP_BUILD=0
SKIP_DEPLOY=0
SKIP_CATALOGUE_REFRESH=0
SKIP_CHECK_HTML=0
HEADPHONE=""
JOBS=${JOBS:-1}
FORCE_ARGS=()
while [ "$#" -gt 0 ]; do
    case "$1" in
        dev|prod) SITE_TARGET=$1 ;;
        --force) FORCE_ARGS=(--force) ;;
        --headphone|--jobs)
            if [ "$#" -lt 2 ] || [ -z "$2" ] || [[ "$2" == --* ]]; then
                echo "$1 requires a value" >&2
                exit 2
            fi
            if [ "$1" = --headphone ]; then HEADPHONE=$2; else JOBS=$2; fi
            shift ;;
        --skip-build) SKIP_BUILD=1 ;;
        --skip-deploy) SKIP_DEPLOY=1 ;;
        --skip-catalogue-refresh) SKIP_CATALOGUE_REFRESH=1 ;;
        --skip-check-html) SKIP_CHECK_HTML=1 ;;
        --help|-h) usage; exit 0 ;;
        *) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
    shift
done
case "$JOBS" in ''|*[!0-9]*) echo "--jobs requires a non-negative integer" >&2; exit 2 ;; esac

mkdir -p .cache build/website
# Both website updates write the same metadata, HTML, and JavaScript assets.
LOCK_DIR=.cache/update_website.lock
if ! mkdir "$LOCK_DIR" 2>/dev/null; then
    echo "Another website update holds $LOCK_DIR; refusing to overlap it." >&2
    exit 1
fi
printf '%s\n' "$$" > "$LOCK_DIR/pid"
release_lock() {
    rm -f "$LOCK_DIR/pid"
    rmdir "$LOCK_DIR"
}
trap release_lock EXIT
trap 'exit 130' HUP INT TERM
trap 'echo "Headphone update failed at line $LINENO" >&2' ERR

export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-"$PWD/target"}
export GPUI_TOOLKIT_DIR=${GPUI_TOOLKIT_DIR:-"$(cd ../.. && pwd)/gpui-toolkit"}
export RENDER_TARGET_DIR=${RENDER_TARGET_DIR:-"$GPUI_TOOLKIT_DIR/target"}
export SPINORAMA_RENDER_IMAGES=${SPINORAMA_RENDER_IMAGES:-"$RENDER_TARGET_DIR/release/spinorama-render-images"}
if [ -z "${THEPYTHON:-}" ]; then
    if [ -x .venv312/bin/python ]; then THEPYTHON=.venv312/bin/python
    elif [ -x .venv/bin/python ]; then THEPYTHON=.venv/bin/python
    else THEPYTHON=python3
    fi
fi
export THEPYTHON
export PYTHONPATH=src:src/website:src/spinorama:.

if [ "$SKIP_BUILD" -eq 0 ]; then
    make generate-build renderer-build
fi
GENERATE_META="$CARGO_TARGET_DIR/release/generate-headphone-meta"
GENERATE_GRAPH="$CARGO_TARGET_DIR/release/generate-headphone-graph"
GENERATE_PEQ="$CARGO_TARGET_DIR/release/generate-peq-headphone"
for binary in "$GENERATE_META" "$GENERATE_GRAPH" "$GENERATE_PEQ" "$SPINORAMA_RENDER_IMAGES"; do
    if [ ! -x "$binary" ]; then
        echo "Missing executable: $binary (run without --skip-build)" >&2
        exit 1
    fi
done

echo "Updating headphones for $SITE_TARGET"
if [ "$SKIP_CATALOGUE_REFRESH" -eq 0 ]; then
    "$THEPYTHON" ./scripts/generate_headphone_datas.py
fi

EQ_SELECTION=()
GRAPH_SELECTION=()
if [ -n "$HEADPHONE" ]; then
    EQ_SELECTION=(--only "$HEADPHONE")
    GRAPH_SELECTION=(--speaker "$HEADPHONE")
fi

# Publish metadata and plots only after the new EQ files and scores exist.
"$GENERATE_PEQ" --batch --jobs "$JOBS" ${EQ_SELECTION[@]+"${EQ_SELECTION[@]}"} ${FORCE_ARGS[@]+"${FORCE_ARGS[@]}"}
"$GENERATE_META" --datas-dir ./datas --output-dir dist/json
"$GENERATE_GRAPH" --headphones --data-dir . --jobs "$JOBS" \
    --render-images-bin "$SPINORAMA_RENDER_IMAGES" --render-webp \
    ${GRAPH_SELECTION[@]+"${GRAPH_SELECTION[@]}"} ${FORCE_ARGS[@]+"${FORCE_ARGS[@]}"}

"$THEPYTHON" ./scripts/svg2symbols.py > build/website/symbols.html
if [ "$SITE_TARGET" = prod ]; then
    "$THEPYTHON" ./scripts/generate_html.py --optim --skip-speakers
else
    "$THEPYTHON" ./scripts/generate_html.py --dev --optim --sitedev=https://dev.spinorama.org --skip-speakers
fi
if [ "$SKIP_CHECK_HTML" -eq 0 ]; then
    ./scripts/check_html.sh
fi
"$THEPYTHON" ./scripts/check_public_dist.py ./dist
if [ "$SKIP_DEPLOY" -eq 1 ]; then
    echo "Skipping deployment to $SITE_TARGET"
elif [ "$SITE_TARGET" = prod ]; then
    ./scripts/update_prod_ovh.sh
else
    ./scripts/update_dev.sh
fi
echo "Headphone update complete"
