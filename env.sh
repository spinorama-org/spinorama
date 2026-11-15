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
    if [ ! -d "$SPIN/.venv" ]; then
        echo "Run make install in $SPIN before sourcing env.sh." >&2
        return 1
    fi
    if ! "$SPIN/.venv/bin/python" -c 'import sys; sys.exit(sys.version_info[:2] != (3, 12))'; then
        echo "$SPIN/.venv must use Python 3.12; move it aside and run scripts/setup.sh." >&2
        return 1
    fi
    # shellcheck source=/dev/null
    source "$SPIN/.venv/bin/activate" || return
    export PYTHONPATH="$SPIN:$SPIN/src:$SPIN/src/website:$SPIN/scripts"

    export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
    export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
    case ":$PATH:" in
        *":$CARGO_HOME/bin:"*) ;;
        *) export PATH="$CARGO_HOME/bin:$PATH" ;;
    esac
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
