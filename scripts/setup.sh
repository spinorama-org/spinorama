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

## package check
## ----------------------------------------------------------------------

OS=$(uname)
PYVERSION=3.12
PYTHON="python${PYVERSION}"

if test "$OS" = "Linux"; then
  # ------------ PYTHON
  if [ -x /usr/bin/apt ]; then
    sudo /usr/bin/apt install -y "python${PYVERSION}" "python${PYVERSION}-venv" imagemagick keychain npm wget
  fi
  # ------------ LOCALE
  # add locale if they don't exist possibly C.utf8 would work
  if command -v locale-gen >/dev/null 2>&1; then
    sudo locale-gen en_US.UTF-8
  elif command -v localedef >/dev/null 2>&1; then
    sudo localedef -f UTF-8 -i en_US en_US.UTF-8
  fi
elif test "$OS" = "Darwin"; then
    brew install npm hdf5 c-blosc2 lzo bzip2 python@${PYVERSION} freetype imagemagick gawk gsed redis
    HDF5_DIR="$(brew --prefix hdf5)"
    export HDF5_DIR
    # Homebrew's default python3 may point to a different Python release.
    PYTHON="$(brew --prefix "python@${PYVERSION}")/bin/python${PYVERSION}"
fi

export PYTHONPATH="${ROOT_DIR}/src:${ROOT_DIR}/src/website"

# python section
if ! command -v "${PYTHON}" >/dev/null 2>&1; then
    echo "ERROR: Python ${PYVERSION} is required. Install python${PYVERSION} and rerun scripts/setup.sh." >&2
    exit 1
fi
if ! "${PYTHON}" -c 'import sys; sys.exit("%d.%d" % sys.version_info[:2] != sys.argv[1])' "${PYVERSION}"; then
    echo "ERROR: ${PYTHON} must run Python ${PYVERSION}." >&2
    exit 1
fi

# Reusing a venv from another release can leave incompatible packages and
# native extensions behind. Keep it intact and explain how to start cleanly.
if [ -e .venv ] || [ -L .venv ]; then
    if ! .venv/bin/python -c 'import sys; sys.exit("%d.%d" % sys.version_info[:2] != sys.argv[1])' "${PYVERSION}"; then
        echo "ERROR: .venv is broken or does not use Python ${PYVERSION}." >&2
        echo "Move .venv aside (e.g. mv .venv .venv-backup), then rerun scripts/setup.sh." >&2
        exit 1
    fi
fi
"${PYTHON}" -m venv .venv
# shellcheck source=/dev/null
source ./.venv/bin/activate
PYTHON="${ROOT_DIR}/.venv/bin/python"
echo "Using $("${PYTHON}" --version) at ${PYTHON}"

# setup.py is invoked directly below, so PEP 517 build-system requirements are
# not installed in an isolated build environment for us. Python 3.12 venvs do
# not include setuptools by default.
"${PYTHON}" -m pip install -U pip setuptools wheel

ARCH=$(uname -a | awk '{print $NF}')
if test "$OS" = "Darwin"  -a "$ARCH" = "arm64" ; then
    # ack to install tables on arm
    echo pip install git+https://github.com/PyTables/PyTables.git@master#egg=tables
fi
"${PYTHON}" -m pip install -U -r requirements.txt
"${PYTHON}" -m pip install -U -r requirements-test.txt
"${PYTHON}" -m pip install -U -r requirements-dev.txt
"${PYTHON}" -m pip install -U -r requirements-api.txt
"${PYTHON}" -m pip install -U -r requirements-meta.txt
"${PYTHON}" -m pip install -U -r requirements-scrape.txt

# node section
npm install .

# lint
"${PYTHON}" -m flake8 . --count --select=E9,F63,F7,F82 --show-source --statistics --exclude .venv

# install 3rd parties
./scripts/update_3rdparties.sh

# create some directories
mkdir -p build dist

# compile cython
cd "${ROOT_DIR}/src/spinorama/compute_scores_cython" && \
    PYTHONPATH=../../.. "${PYTHON}" setup.py build_ext --inplace && \
    rm -f compute_scores_cython.so && \
    SO_FILE="compute_scores_cython$("${PYTHON}" -c 'import sysconfig; print(sysconfig.get_config_var("EXT_SUFFIX"))')" && \
    ln -s "$SO_FILE" compute_scores_cython.so && \
    cd ../../..

# Compile and install Rust extensions into the active virtual environment.
"${PYTHON}" -m maturin develop --release \
    --manifest-path "${ROOT_DIR}/src/spinorama/compute_scores_rust/Cargo.toml"
"${PYTHON}" -m maturin develop --release \
    --manifest-path "${ROOT_DIR}/src/spinorama/annotations_rust/Cargo.toml"

# Do not silently fall back to the Python implementations after setup.
"${PYTHON}" - <<'PY'
import annotations_rust
import compute_scores_rust
from spinorama.plot import annotations

assert annotations._c_place_annotations is not None
print("Rust extensions installed successfully")
PY
