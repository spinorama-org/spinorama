# spinorama-generate

Rust replacements for `generate_graphs.py` / `generate_peqs.py`, built
in slices. Both binaries never spawn Python; PNG rasterization drives
the gpui-toolkit `spinorama-render-images` binary.

## `generate-graph`

Loads one measurement version (or a whole catalogue with
`--metadata-json`), computes CEA2034 and derived curves, and writes the
private figure payloads (Plotly JSON + v2 trace bundles for scatter
figures). The comparable contract against the Python pipeline is
trace-bundle numeric equality (1024-point log grid, 0.01 dB), not
Plotly JSON or pixel bytes.

Loaders: `klippel`, `spl_hv_txt`, `gll_hv_txt`, `princeton` (MATLAB v4
IR + FFT), `rew_text_dump`, `webplotdigitizer`, and `csv_freq_spl`
(headphones: Frequency Response / Compensated / Target Deviation
against the Harman targets in `--targets-dir`). Figures for HV data:
the 5 common views, group delay (when phase exists), 3 reflection
views, 4 SPL views, 8 contour / surface views, 2 radar views, and 2
directivity views (24–25 total). Curve loaders emit the 5 common
views. Regression bands and valid-range `N/A` traces match the Python
bundles. When `datas/eq/<speaker>/iir.txt` exists, the same figure set
is recomputed with that EQ applied into `<version>_eq/` (`--no-eq`
skips it, `--eq-dir` relocates the EQ root).

```
cargo run --release --manifest-path crates/spinorama-generate/Cargo.toml \
  --bin generate-graph -- \
  --speaker-dir "datas/measurements/KEF R3/asr" \
  --speaker "KEF R3" --origin ASR --mversion asr \
  --data-dir /Volumes/home_tmp/tmp/spinorama-generate-out --force --render-png
```

Batch mode consumes the per-speaker TOML catalogue directly
(`datas/measurements/<Speaker>/meta.toml`, validated strictly by
`src/metadata.rs`); `--metadata-json` remains for the legacy export
from `scripts/export_speaker_metadata_json.py`:

```
cargo run --release --manifest-path crates/spinorama-generate/Cargo.toml \
  --bin generate-graph -- \
  --metadata-dir datas/measurements \
  --data-dir /Volumes/home_tmp/tmp/spinorama-generate-out --force
cargo run --release --manifest-path crates/spinorama-generate/Cargo.toml \
  --bin generate-graph -- --metadata-dir datas/measurements --check-meta
```

With `--render-png --render-webp`, rendered PNGs are hardened to
public WebP in Rust (seeded texture + diagonal attribution watermark
with the measurement source, matching the Python layers). Encoding is
libwebp via the `webp` crate (bundled C sources, NEON on Apple
Silicon), lossy `--webp-quality` (default 82, method 4,
multithreaded) or `--webp-lossless`; conversion runs in parallel
across CPU cores. On KEF R3 at 2400x1600 q82 averages ~173 KB/image
vs ~2.1 MB lossless, and 20 images render+harden in ~1.4 s. The Python HDF5 cache
is only a batch freshness skip in `generate_graphs.py`; the Rust
loaders run in milliseconds and use output existence instead, so no
HDF5 reader is ported. Named gaps: contour gridline traces, per-origin
mean-band parameters, headphone EQ figure variants, and the installed
`spinorama-render-images` binary only renders CEA2034 and 2D contour
stems today (rebuilding it is the gpui-toolkit owner's step).

## `generate-meta`

Metadata JSON generation (`dist/json/metadata.json`, `eqdata.json`,
head/year chunks + zip/bz2 companions), replacing Python
`scripts/generate_meta.py`. It loads every measurement directly
with the crate loaders — the Python version read the HDF5 cache
written by `generate_graphs.py`, so under the Rust pipeline its
output silently went stale (it missed ~170 measurements' estimates
and ~140 preference ratings). Full catalogue (1140 speakers) runs in
~2 s. Parity against the last Python output: median and p99 absolute
`pref_score` difference 0.000 over 1110 common measurements; one
known outlier (PMC PMC10-4, LFX from the stale cache). Headphone
metadata stays Python (`scripts/generate_headphone_meta.py`).

## `generate-peq`

PEQ computation on Rust AutoEQ (`autoeq-workflow`). Inputs: a
measurement version directory (`--speaker-dir`, any graph loader
format), a `generate-graph` CEA2034 payload (`--cea2034-json`), a
single freq/SPL CSV (`--curve`), or a catalogue (`--metadata-dir`
or legacy `--metadata-json`, default measurement per speaker,
`--output-dir` is then the EQ root).
Fitness: `Flat`, `LeastSquare`, `Score`, `Combine`, `FlatPir`.
Optimisation: `global` (joint DE), `greedy` (one filter at a time
against the residual, best-score prefix kept), or `--graphic-eq NAME`
(fixed-band devices from `datas/grapheq.py`; `--graphic-eq-list`
lists them). `--peq-model` selects shelf-capable AutoEQ models
(`ls-pk-hs`, …); `--algo` selects the AutoEQ algorithm. Python
defaults override AutoEQ defaults (7 filters, Q 0.25–3, ±3 dB, 48 kHz,
seed 42). Outputs `iir-autoeq.txt` (or `iir-autoeq-<device>.txt` for
graphic EQ; previous-score gating unless `--force`),
`conf-autoeq.json` (create-if-absent), and — with `--data-dir` — the
16 separate `filters_*` site figures (per-filter EQ, EQ vs target,
spin before/after, ON/LW/PIR regression views, and error histograms).
With `--generate-images-only` the figures are rebuilt from the
existing APO file without optimizing.

Acceptance is score/RMS-based (Python DE is unseeded; bit-identical
PEQs are never claimed). Named gaps: the combined 8×2 `filters_eq`
subplot figure, multi-curve weighted optimisation, exact Python
greedy/grapheq loss details, the 0.75 dB minimum |gain| bound (no
signed-bound equivalent in AutoEQ), an unexplained ~0.1 absolute
Olive-score offset versus Python, WebP emission, and
`results_iter.csv`.

## Dependencies

AutoEQ is a pure git dependency
(`https://github.com/pierreaubert/autoeq`, locked in `Cargo.lock`);
its math crates resolve from `pierreaubert/math-audio` since upstream
commit `a0782641` ("point to git directories not filesystem"). No
`[patch]` or local checkout is required to build this crate.

## Checks

```
make generate-check generate-fmt generate-clippy generate-test generate-build
```
