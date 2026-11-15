# Graph annotation solver

The API and graph generators use this shared Rust library to place labels and
leader lines around plotted curves. It is a member of the root Cargo workspace
and builds without Python by default.

Run its native tests from the repository root:

```sh
cargo test --locked -p annotations_rust
```

The optional `python` feature exposes the same solver through PyO3. Install it
into the project virtual environment with `make rust-python`, or include it with
the other Python reference extensions using `make python-reference`.

The Python import remains `spinorama.annotations_rust`. The crate-local
`pyproject.toml` keeps the extension's packaging separate from the website's
Python package.
