param([switch]$InstallOnly)

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
Write-Host "Set PYTHONUTF8=1 (current session)"

# ------------ CHECK PREREQUISITES
$missing = @()
if (-not (Get-Command "python" -ErrorAction SilentlyContinue)) { $missing += "python $PYVERSION (https://www.python.org/downloads/)" }
if (-not (Get-Command "npm"    -ErrorAction SilentlyContinue)) { $missing += "Node.js / npm (https://nodejs.org/)" }
if (-not (Get-Command "git"    -ErrorAction SilentlyContinue)) { $missing += "git (https://git-scm.com/)" }

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
& $ProjectPython -m pip install -r (Join-Path $RootDir "requirements.txt")
if ($LASTEXITCODE -ne 0) { throw "Failed to install Python dependencies" }

# ------------ USER-LOCAL RUST
if (-not $env:CARGO_HOME) { $env:CARGO_HOME = Join-Path $env:USERPROFILE ".cargo" }
if (-not $env:RUSTUP_HOME) { $env:RUSTUP_HOME = Join-Path $env:USERPROFILE ".rustup" }
$env:PATH = "$(Join-Path $env:CARGO_HOME 'bin');$env:PATH"
$Rustup = Join-Path $env:CARGO_HOME "bin\rustup.exe"
if (-not (Test-Path $Rustup)) {
    $RustTarget = if ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -eq "Arm64") {
        "aarch64-pc-windows-msvc"
    } else {
        "x86_64-pc-windows-msvc"
    }
    $Installer = Join-Path ([System.IO.Path]::GetTempPath()) ("rustup-" + [guid]::NewGuid() + ".exe")
    try {
        Invoke-WebRequest -Uri "https://static.rust-lang.org/rustup/dist/$RustTarget/rustup-init.exe" -OutFile $Installer
        & $Installer -y --profile minimal --default-toolchain stable --no-modify-path
        if ($LASTEXITCODE -ne 0) { throw "Failed to install rustup" }
    } finally {
        Remove-Item $Installer -ErrorAction SilentlyContinue
    }
} else {
    & $Rustup show active-toolchain
    if ($LASTEXITCODE -ne 0) {
        & $Rustup toolchain install stable --profile minimal
        if ($LASTEXITCODE -ne 0) { throw "Failed to install Rust stable" }
        & $Rustup default stable
        if ($LASTEXITCODE -ne 0) { throw "Failed to select Rust stable" }
    }
}

# ------------ LOCKED NODE PACKAGES
npm ci
if ($LASTEXITCODE -ne 0) { throw "Failed to install Node dependencies" }
New-Item -ItemType Directory -Force -Path "build", "dist" | Out-Null
if ($InstallOnly) { return }

# ------------ WEBSITE ASSETS
# Read the same asset versions as the Unix script and HTML generator.
$Versions = @{}
Get-Content (Join-Path $PSScriptRoot "update_3rdparties.sh") | ForEach-Object {
    if ($_ -match '^(HANDLEBARS|BULMA|FUSE)=(.+)$') { $Versions[$Matches[1]] = $Matches[2] }
}
New-Item -ItemType Directory -Force -Path "dist/js3rd", "dist/css", "dist/json" | Out-Null
Copy-Item "node_modules/handlebars/dist/handlebars.min.js" "dist/js3rd/handlebars-$($Versions['HANDLEBARS']).min.js" -Force
Copy-Item "node_modules/fuse.js/dist/fuse.min.mjs" "dist/js3rd/fuse-$($Versions['FUSE']).min.mjs" -Force
npx --no-install sass --load-path=node_modules src/website/bulma4spin.scss "dist/css/bulma-$($Versions['BULMA']).min.css" --style=compressed --no-source-map
if ($LASTEXITCODE -ne 0) { throw "Failed to compile website CSS" }

# ------------ BUILD NATIVE BINARIES
& (Join-Path $env:CARGO_HOME "bin/cargo.exe") build --locked --release --workspace
if ($LASTEXITCODE -ne 0) { throw "Failed to build the Rust workspace" }
Write-Host "`n--- Setup complete ---" -ForegroundColor Green
