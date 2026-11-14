#!/bin/bash
# Copyright (C) 2020-2025 Pierre Aubert pierre(at)spinorama(dot)org
# SPDX-License-Identifier: GPL-3.0-or-later
# Source this file from Bash or Zsh to activate the project environment.

if [ -n "${BASH_VERSION:-}" ]; then
    SPIN="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
elif [ -n "${ZSH_VERSION:-}" ]; then
    SPIN="$(cd -- "$(dirname -- "${(%):-%x}")" && pwd)"
else
    echo "Source env.sh from Bash or Zsh." >&2
    return 1 2>/dev/null || exit 1
fi
export SPIN

_spinorama_env() {
    if [ ! -e "$SPIN/.venv" ]; then
        if ! command -v python3.12 >/dev/null 2>&1; then
            echo "Python 3.12 is required; run scripts/setup.sh first." >&2
            return 1
        fi
        python3.12 -m venv "$SPIN/.venv" || return
    fi
    if ! "$SPIN/.venv/bin/python" -c 'import sys; sys.exit(sys.version_info[:2] != (3, 12))'; then
        echo "$SPIN/.venv must use Python 3.12; move it aside and run scripts/setup.sh." >&2
        return 1
    fi
    # shellcheck source=/dev/null
    source "$SPIN/.venv/bin/activate" || return
    "$SPIN/.venv/bin/python" -m pip install --quiet -r "$SPIN/requirements.txt" || return
    export PYTHONPATH="$SPIN:$SPIN/src:$SPIN/src/website:$SPIN/scripts"

    if [ ! -d "$SPIN/node_modules" ]; then
        npm --prefix "$SPIN" install || return
    fi
    case ":$PATH:" in
        *":$SPIN/node_modules/.bin:"*) ;;
        *) export PATH="$SPIN/node_modules/.bin:$PATH" ;;
    esac

    if [ -d /opt/homebrew/opt/imagemagick ]; then
        export MAGICK_HOME=/opt/homebrew/opt/imagemagick
    elif [ -d /opt/homebrew/opt/imagemagick@7 ]; then
        export MAGICK_HOME=/opt/homebrew/opt/imagemagick@7
    fi
    mkdir -p "$SPIN/build" || return
    printf 'Spinorama: %s\nPython: %s\n' "$SPIN" "$(python --version)"
}

if _spinorama_env; then
    unset -f _spinorama_env
else
    unset -f _spinorama_env
    return 1 2>/dev/null || exit 1
fi
