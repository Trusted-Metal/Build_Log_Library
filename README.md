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

The IO layer links the HDF5 library (libhdf5), so building needs one. **Minimum supported Rust version (measured): 1.85.1**, the oldest compiler that builds and tests the workspace. It is set in every crate, and a test checks it against what the dependencies declare.

**On a machine without libhdf5**, compile it from source with the `static-hdf5` feature. It needs CMake and a C compiler (on Windows, the Visual Studio C++ tools). The first build takes about three minutes and leaves libhdf5 in `target/`, so later builds are fast:

```text
cargo build --workspace --features build-log/static-hdf5
cargo test --workspace --features build-log/static-hdf5
cargo clippy --workspace --all-targets --features build-log/static-hdf5 -- -D warnings
cargo fmt --all -- --check
```

**With libhdf5 installed**, drop the `--features` flag. CI does this (apt on Linux, vcpkg on Windows); the exact setup is written down with the CI workflow.

**On Windows, keep the path short.** CMake fails to detect the compiler when a path passes 260 characters, and this build's own folders add about 125 to the path of `target/`. Keep the checkout path under about 130 characters, or set `CARGO_TARGET_DIR` to a short folder.

The from-source build compiles the libhdf5 that the binding bundles (2.x at the time of writing), not the 1.x versions some consumers run; the CI legs test those.

## Conventions

- **Commits** follow Conventional Commits (`type(scope): subject`), checked by commitlint on commit; the types and scopes are in [`commitlint.config.cjs`](commitlint.config.cjs).
- **Tests** are named for the claim they prove and live where [`docs/test-conventions.md`](docs/test-conventions.md) says.
- **Pull requests** use [the template](.github/pull_request_template.md).
- **Test data**: the miniatures in [`testdata/`](testdata/README.md) are never rebuilt; the originals they were cut from are not in this repository.
- **The Model** is documented against the file it mirrors in [`docs/models/derivation.md`](docs/models/derivation.md).
