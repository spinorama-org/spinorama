# Repository checkout

This repository contains code, metadata, EQ files and pictures. Raw measurements
and real-data fixtures are in the private `spinorama-org/measurements` submodule
at `datas/measurements`. Access to that repository is required for data-dependent
builds, tests and tools. Cloning this public code repository alone grants no data access.

```sh
git clone https://github.com/spinorama-org/spinorama.git
cd spinorama
git submodule update --init --recursive
```

For an existing checkout, use `git pull --recurse-submodules` followed by
`git submodule update --init --recursive`. The submodule is pinned to a commit;
update and commit that pointer deliberately when publishing data changes.
All three repositories start with identical `main` and `develop` branches and no old history.

Headphone data, `tests/datas`, extraction fixtures, `experimental` notebooks and
`output.json` use relative links into the private submodule. They become available
after authorized submodule initialization. `.env`, caches, generated site builds,
and local secrets are not included in either repository.

---

# Spinorama : a library to display speaker frequency response and similar graphs

[![GPLv3 license](https://img.shields.io/badge/License-GPLv3-blue.svg)](http://perso.crans.org/besson/LICENSE.html)
[![Maintenance](https://img.shields.io/badge/Maintained%3F-yes-green.svg)](https://github.com/spinorama-org/spinorama/graphs/commit-activity)
[![Website www.spinorama.org](https://img.shields.io/website-up-down-green-red/http/shields.io.svg)](https://www.spinorama.org/)
[![pre-commit](https://img.shields.io/badge/pre--commit-enabled-brightgreen?logo=pre-commit&logoColor=white)](https://github.com/pre-commit/pre-commit)
[![Spinorama Python](https://github.com/spinorama-org/spinorama/actions/workflows/pythonapp.yml/badge.svg?branch=develop)](https://github.com/spinorama-org/spinorama/actions/workflows/pythonapp.yml)
[![Spinorama Javascript](https://github.com/spinorama-org/spinorama/actions/workflows/webapp.yml/badge.svg?branch=develop)](https://github.com/spinorama-org/spinorama/actions/workflows/webapp.yml)

This library provides an easy way to view, compare or analyse speakers data. This can help you take informed decision when buying a speaker instead of relying on commercial information or internet buzz. There are enough measurements now that you can do statistical analysis if you wanted too.

## Jump to [spinorama.org](https://spinorama.org) of all (1000+) speakers measurements.

## Jump to the [documentation](https://spinorama.org/docs) to learn more about what it can do and how to use it.

## Development setup

### Prerequisites

| Tool | Version | Notes |
|------|---------|-------|
| Python | 3.12 | |
| Node.js / npm | LTS | |
| git | any | |
| Rust / cargo | stable | Optional, for the Rust scoring extension |
| maturin | stable | Optional, `pip install maturin` |

### Linux / macOS

```bash
./scripts/setup.sh
```

This installs system packages (via `apt` on Linux or `brew` on macOS), creates a
Python virtual environment, installs all dependencies, downloads third-party
assets, and compiles the Cython and Rust extensions.

### Windows (PowerShell)

Install [Python 3.12](https://www.python.org/downloads/),
[Node.js](https://nodejs.org/) and [git](https://git-scm.com/) manually, then
run:

```powershell
.\scripts\setup.ps1
```

The script automatically sets the `PYTHONUTF8=1` environment variable (both for
the current session and persistently for the user) so that Python reads files as
UTF-8 by default.  This is required on Windows where the system locale is
typically not UTF-8.  See
[Python on Windows UTF-8 mode](https://docs.python.org/3/using/windows.html#win-utf8-mode)
for details.

If you prefer to set it yourself:

```powershell
# Current session only
$env:PYTHONUTF8 = "1"

# Persistent (user-level)
[System.Environment]::SetEnvironmentVariable("PYTHONUTF8", "1", "User")
```

## Private-data CI

Python CI is opt-in on trusted pushes only. Set repository variable
`ENABLE_PRIVATE_DATA_CI=true` and secret `MEASUREMENTS_READ_TOKEN` to a fine-grained
read-only token permitted to read all three repositories. Never expose that token or
private measurements to public pull-request jobs. JavaScript CI runs without the
private submodule.

## Private GraphExtract tools

GraphExtract lives at https://github.com/spinorama-org/graphextract and is mounted
as the private `src/graphextract` submodule. Recursive submodule initialization
requires access to both private repositories. Extraction helpers and legacy
modules are relative links into that submodule; they are not part of the public
Spinorama Python distribution.
