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

set -eu

# Resolve the sibling checkout independently of the caller's working directory.
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
pictures_root="$(dirname "$root")/spinorama-pictures"
sourcedir="$pictures_root/web"
targetdir=dist

# Install the manual's images without changing their resolution.
mkdir -p "${targetdir}/docs/images" "${targetdir}/pictures"
cp -pR manual/images/. "${targetdir}/docs/images/"

if [ ! -d "$sourcedir" ]; then
    cat >&2 <<EOF
WARNING: Website pictures are unavailable: $sourcedir
Clone the picture repository alongside this checkout:
  git clone https://github.com/spinorama-org/spinorama-pictures.git "$pictures_root"
Then rerun scripts/update_pictures.sh. Continuing without updating product pictures.
EOF
else
    # web/ already contains the website variants; preserve their names and bytes.
    find "$sourcedir" -type f \( -name '*.png' -o -name '*.jpg' -o -name '*.jpeg' -o -name '*.webp' -o -name '*.svg' -o -name '*.ico' \) -exec sh -eu -c '
        source_root=$1
        destination_root=$2
        shift 2
        for picture do
            relative=${picture#"$source_root"/}
            target="$destination_root/$relative"
            if [ -d "$target" ]; then
                echo "Cannot copy picture: destination is a directory: $target" >&2
                exit 1
            fi
            mkdir -p "$(dirname "$target")"
            if [ ! -f "$target" ] || ! cmp -s "$picture" "$target"; then
                cp -p "$picture" "$target"
            fi
        done
    ' sh "$sourcedir" "${targetdir}/pictures" {} +
fi
# Copy only browser image assets. Notebooks and archives must not enter dist.
for icon in datas/icons/*; do
    target="dist/pictures/$(basename "$icon")"
    case "$icon" in
        *.png|*.jpg|*.jpeg|*.webp|*.svg|*.ico) ;;
        *)
            # Remove copies left by older runs of this script.
            rm -f -- "$target"
            continue
            ;;
    esac
    [ -f "$icon" ] || continue
    if test ! -f "$target" || ! cmp -s "$icon" "$target"; then
        cp -p "$icon" "$target"
    fi
done
