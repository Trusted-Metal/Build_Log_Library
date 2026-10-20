# Build Log Library

A versioned library for the per-build records of laser powder bed fusion (LPBF) builds, stored as HDF5. It reads every version of the Build Log file format and writes the current ones, through one common data model. Rust first.

**Status: early.** The workspace, its layers and its conventions are in place; the file adapters, the writers and the processing are not written yet. This README is completed when the repository scaffold is.

## Layout

| Path | What |
|---|---|
| [`rust/model`](rust/model) | `build-log-model`: the Model, plain data types with no HDF5. Builds for wasm32. |
| [`rust/io`](rust/io) | `build-log-io`: the IO layer, HDF5 files to and from the Model. A skeleton. |
| [`rust/processing`](rust/processing) | `build-log-processing`: derived computation over Model data. A skeleton. |
| [`rust/build-log`](rust/build-log) | `build-log`: the public entry point; the one crate consumers depend on. Features choose the layers. |
| [`rust/testkit`](rust/testkit) | `build-log-testkit`: where tests find their data. Dev-only. |
| [`schema/`](schema) | The language-neutral specification (not written yet); outside every language folder. |
| [`testdata/`](testdata) | The committed, redacted miniatures that tests run on, with their checksums. |
| [`docs/`](docs) | Plans, decisions, layouts, migrations and the model derivation. |

## Build and test

```text
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

The IO layer does not link HDF5 yet. When it does, building it needs the HDF5 library on the machine, or `--features static-hdf5` to compile it from source; this README will say how.

## Conventions

- **Commits** follow Conventional Commits (`type(scope): subject`), checked by commitlint on commit; the types and scopes are in [`commitlint.config.cjs`](commitlint.config.cjs).
- **Tests** are named for the claim they prove and live where [`docs/test-conventions.md`](docs/test-conventions.md) says.
- **Pull requests** use [the template](.github/pull_request_template.md).
- **Test data**: the miniatures in [`testdata/`](testdata/README.md) are never rebuilt; the originals they were cut from are not in this repository.
- **The Model** is documented against the file it mirrors in [`docs/models/derivation.md`](docs/models/derivation.md).
