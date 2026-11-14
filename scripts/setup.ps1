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

$ErrorActionPreference = "Stop"
$RootDir = Split-Path -Parent $PSScriptRoot
Set-Location $RootDir

$PYVERSION = "3.12"

# ------------ UTF-8
# Windows does not default to UTF-8 for Python; enable it via PYTHONUTF8.
# See https://docs.python.org/3/using/windows.html#win-utf8-mode
#     https://docs.python.org/3/using/cmdline.html#envvar-PYTHONUTF8
$env:PYTHONUTF8 = "1"
[System.Environment]::SetEnvironmentVariable("PYTHONUTF8", "1", "User")
Write-Host "Set PYTHONUTF8=1 (session + user environment)"

# ------------ CHECK PREREQUISITES
$missing = @()
if (-not (Get-Command "python" -ErrorAction SilentlyContinue)) { $missing += "python $PYVERSION (https://www.python.org/downloads/)" }
if (-not (Get-Command "npm"    -ErrorAction SilentlyContinue)) { $missing += "Node.js / npm (https://nodejs.org/)" }
if (-not (Get-Command "git"    -ErrorAction SilentlyContinue)) { $missing += "git (https://git-scm.com/)" }
if (-not (Get-Command "cargo"  -ErrorAction SilentlyContinue)) { $missing += "Rust / cargo" }

if ($missing.Count -gt 0) {
    Write-Host "`nThe following tools are required but not found in PATH:" -ForegroundColor Red
    $missing | ForEach-Object { Write-Host "  - $_" -ForegroundColor Yellow }
    Write-Host "`nInstall them and re-run this script.`n" -ForegroundColor Red
    exit 1
}

# Verify Python version
$pyVer = python --version 2>&1
Write-Host "Detected: $pyVer"
python -c 'import sys; sys.exit(sys.version_info[:2] != (3, 12))'
if ($LASTEXITCODE -ne 0) { throw "Python $PYVERSION is required" }

# ------------ PYTHONPATH
$env:PYTHONPATH = "$RootDir;$RootDir\src;$RootDir\src\website;$RootDir\scripts"

# ------------ PYTHON VENV
Write-Host "`n--- Creating Python virtual environment ---"
if (Test-Path .venv) {
    & .\.venv\Scripts\python.exe -c 'import sys; sys.exit(sys.version_info[:2] != (3, 12))'
    if ($LASTEXITCODE -ne 0) { throw "Move the incompatible .venv aside, then rerun setup.ps1" }
}
python -m venv .venv
if ($LASTEXITCODE -ne 0) { throw "Failed to create .venv" }
& .\.venv\Scripts\Activate.ps1
$ProjectPython = Join-Path $RootDir ".venv\Scripts\python.exe"

# ------------ PIP PACKAGES
Write-Host "`n--- Installing Python dependencies ---"
& $ProjectPython -m pip install -U pip
if ($LASTEXITCODE -ne 0) { throw "Failed to update pip" }
& $ProjectPython -m pip install -U -r (Join-Path $RootDir "requirements.txt")
if ($LASTEXITCODE -ne 0) { throw "Failed to install Python dependencies" }

# ------------ NODE PACKAGES
Write-Host "`n--- Installing Node dependencies ---"
npm install .

# ------------ LINT
Write-Host "`n--- Running flake8 lint check ---"
flake8 . --count --select=E9,F63,F7,F82 --show-source --statistics --exclude .venv

# ------------ 3RD PARTIES
Write-Host "`n--- Installing 3rd-party assets ---"

# Versions must match scripts/update_3rdparties.sh
$PLOTLY      = "3.4.0"
$HANDLEBARS  = "4.7.8"
$BULMA       = "1.0.4"
$FUSE        = "7.0.0"

$ASSETS      = ".\dist"
$ASSETS_JS   = "$ASSETS\js3rd"
$ASSETS_CSS  = "$ASSETS\css"
$ASSETS_JSON = "$ASSETS\json"

New-Item -ItemType Directory -Force -Path $ASSETS, $ASSETS_JS, $ASSETS_CSS, $ASSETS_JSON | Out-Null

# Handlebars
$hbsFile = "$ASSETS_JS\handlebars-$HANDLEBARS.min.js"
if (-not (Test-Path $hbsFile)) {
    Write-Host "Downloading Handlebars $HANDLEBARS ..."
    Invoke-WebRequest -Uri "https://cdn.jsdelivr.net/npm/handlebars@$HANDLEBARS/dist/handlebars.min.js" -OutFile $hbsFile
}

# Bulma (compile from SCSS)
npm install bulma
npx sass --load-path=node_modules src/website/bulma4spin.scss "dist\css\bulma-$BULMA.min.css" --style=compressed --no-source-map

# Fuse.js
npm install fuse.js
Copy-Item "node_modules\fuse.js\dist\fuse.min.mjs" "$ASSETS_JS\fuse-$FUSE.min.mjs" -Force

# Plotly
npm install plotly.js-dist-min
Copy-Item "node_modules\plotly.js-dist-min\plotly.min.js" "$ASSETS_JS\plotly-$PLOTLY.min.mjs" -Force

# ------------ CREATE DIRECTORIES
Write-Host "`n--- Creating build directories ---"
New-Item -ItemType Directory -Force -Path "build", "dist" | Out-Null

# ------------ BUILD NATIVE BINARIES
Write-Host "`n--- Building native generators and API ---"
cargo build --locked --release --workspace
if ($LASTEXITCODE -ne 0) {
    throw "Failed to build the Rust workspace"
}

Write-Host "`n--- Setup complete ---" -ForegroundColor Green
