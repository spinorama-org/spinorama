#!/bin/bash
set -e

# Resolve paths from this script's location so it can be invoked from anywhere.
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
cd "${ROOT_DIR}"

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

# Install only into the checkout and the current user's Rust directories.
INSTALL_ONLY=false
case "${1:-}" in
    --install-only) INSTALL_ONLY=true ;;
    "") ;;
    *) echo "Usage: $0 [--install-only]" >&2; exit 2 ;;
esac

for tool in npm git; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "ERROR: Install $tool before running setup." >&2
        exit 1
    fi
done

export PYTHONPATH="${ROOT_DIR}:${ROOT_DIR}/src:${ROOT_DIR}/src/website:${ROOT_DIR}/scripts"
PYTHON="${SPINORAMA_PYTHON:-python3.12}"
if [ -e .venv ] || [ -L .venv ]; then
    PYTHON="${ROOT_DIR}/.venv/bin/python"
fi
if ! "$PYTHON" -c 'import sys; sys.exit(sys.version_info[:2] != (3, 12))'; then
    echo "ERROR: Python 3.12 is required. Set SPINORAMA_PYTHON to its executable." >&2
    echo "If .venv is incompatible or broken, move it aside before rerunning setup." >&2
    exit 1
fi
if [ ! -d .venv ]; then
    "$PYTHON" -m venv .venv
fi
PYTHON="${ROOT_DIR}/.venv/bin/python"
"$PYTHON" -m pip install -r "${ROOT_DIR}/requirements.txt"

export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
export PATH="$CARGO_HOME/bin:$PATH"
if [ ! -x "$CARGO_HOME/bin/rustup" ]; then
    command -v curl >/dev/null 2>&1 || { echo "ERROR: curl is needed to install rustup." >&2; exit 1; }
    RUSTUP_INSTALLER="$(mktemp)"
    trap 'rm -f "$RUSTUP_INSTALLER"' EXIT
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o "$RUSTUP_INSTALLER"
    sh "$RUSTUP_INSTALLER" -y --profile minimal --default-toolchain stable --no-modify-path
elif ! "$CARGO_HOME/bin/rustup" show active-toolchain >/dev/null 2>&1; then
    "$CARGO_HOME/bin/rustup" toolchain install stable --profile minimal
    "$CARGO_HOME/bin/rustup" default stable
fi

npm ci
mkdir -p build dist
if [ "$INSTALL_ONLY" = false ]; then
    ./scripts/update_3rdparties.sh
    "$CARGO_HOME/bin/cargo" build --locked --release --workspace
fi
printf 'Dependencies installed. Run source "%s/env.sh" to activate them.\n' "$ROOT_DIR"
