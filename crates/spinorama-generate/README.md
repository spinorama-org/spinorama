# spinorama-generate

Native graph, metadata, and EQ generators for Spinorama. The binaries load
measurement files directly and use the gpui-toolkit `spinorama-render-images`
binary to rasterize plots. They do not require Python extensions.

## Build and generate

Run these commands from the repository root:

```sh
make generate-build
make graphs SPEAKER="KEF R3"
make images SPEAKER="KEF R3"
```

Omit `SPEAKER` to process the catalogue. Set `VERSION` to select one measurement
version. `make images` also builds the renderer from `GPUI_TOOLKIT_DIR`.
Graph payloads go into `dist-private/speakers/`; public images go into
`dist/speakers/`.

The catalogue uses `datas/measurements/<speaker>/meta.toml`, with origins in
`datas/origins.toml`. The supported loaders are `klippel`, `spl_hv_txt`,
`gll_hv_txt`, `princeton`, `rew_text_dump`, `webplotdigitizer`, and `csv_freq_spl`.

Validate metadata or parse the corresponding measurement files:

```sh
make check-meta
make check-meta-files SPEAKER="KEF R3"
```

Use `make metaedit SPEAKER="KEF R3"` to edit a speaker's TOML through the Qt UI.
The editor uses `generate-graph --check-meta` before saving.

## Binaries

| Binary | Output |
| --- | --- |
| `generate-graph` | Speaker graph payloads and rendered images |
| `generate-meta` | Speaker metadata and EQ JSON for the website |
| `generate-radar` | Radar figures |
| `generate-eq-compare` | EQ comparison figures |
| `generate-peq` | Speaker EQ filters and diagnostic figures |
| `generate-headphone-graph` | Headphone graphs |
| `generate-headphone-meta` | Headphone metadata and EQ JSON |
| `generate-peq-headphone` | Headphone EQ filters |

For each binary's arguments, run `target/release/<binary> --help`.
The former Python generation and compare-rendering entrypoints have been removed.
Python loaders, plotting code, and optional extensions remain for ingestion,
analysis, and reference tests.

## Compute speaker EQ

Use the Python orchestration scripts to run the Rust optimiser for individual
speakers or a batch:

```sh
PYTHONPATH=src:scripts:. .venv/bin/python scripts/eqs_compute.py --help
PYTHONPATH=src:scripts:. .venv/bin/python scripts/eqs_process.py --help
```

These scripts default to CMA-ES. The native `generate-peq --help` describes
algorithm, iteration, filter, and fitness options for direct invocation.

## Dependencies and checks

AutoEQ and gpui-toolkit resolve from Git dependencies locked in `Cargo.lock`.
The renderer is built separately from the sibling gpui-toolkit checkout.

```sh
make generate-check generate-fmt generate-clippy generate-test
```

Native integration tests cover TOML validation, measurement loading, graph
outputs, missing-image regeneration, and headphone graphs and EQ selection.
The Python reference extensions are optional. Install them with
`make python-reference` when running their parity tests.
