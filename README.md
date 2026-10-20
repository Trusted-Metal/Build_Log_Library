# Build Log Library

A versioned library for the per-build records of laser powder bed fusion (LPBF) builds, stored as HDF5. It reads every version of the Build Log file format and writes the current ones, through one common data model. Rust first.

**Status: early.** The repository scaffold is in place: the workspace and its layers, the guards that keep them apart, the test data, the CI and the commit conventions. The file adapters, the writers and the processing are not written yet.

## Layout

| Path | What |
|---|---|
| [`rust/model`](rust/model) | `build-log-model`: the Model, plain data types with no HDF5. Builds for wasm32. |
| [`rust/io`](rust/io) | `build-log-io`: the IO layer, HDF5 files to and from the Model. So far the HDF5 binding and a check of which libhdf5 is linked; the file adapters are not written. |
| [`rust/processing`](rust/processing) | `build-log-processing`: derived computation over Model data. A skeleton. |
| [`rust/build-log`](rust/build-log) | `build-log`: the public entry point; the one crate consumers depend on. Features choose the layers. Its tests are the architecture guards. |
| [`rust/testkit`](rust/testkit) | `build-log-testkit`: where tests find their data. Dev-only. |
| [`schema/`](schema) | The language-neutral specification (not written yet); outside every language folder. |
| [`testdata/`](testdata/README.md) | The committed, redacted miniatures that tests run on, with their checksums. |
| [`docs/`](docs) | The test conventions, the model derivation, and the per-version layout and migration documents (which arrive with the adapters). |
| [`.github/`](.github) | The CI workflow and its helper script, and the pull request template. |

## The layers

Four layers, each allowed to depend only on those below it: **Model** (plain data types, no HDF5), **IO** (HDF5 files to and from the Model) and **Processing** (computation over Model values), side by side, and the **facade** `build-log` above them, which is what consumers depend on (by git; nothing is published). The rules that follow:

- **The Model has no HDF5 dependency**, and an advisory CI job builds it for `wasm32`.
- **Processing does not depend on IO.**
- **Nothing in the workspace depends on the facade.**
- **No feature changes what is in a file on disk.** This is a rule written down; its test arrives with the writers.
- **The library never loads a machine configuration (MCF)** and has no dependency on the MCF library.

Tests enforce the others: `layering_is_one_way`, `model_has_no_hdf5_dependency`, `processing_does_not_depend_on_io` and `no_mcf_dependency` in [`rust/build-log/tests`](rust/build-log/tests), each shown to fail on a violation made for it. The allowed dependencies are data, in [`allowed_dependencies.json`](rust/build-log/tests/data/allowed_dependencies.json).

## Build and test

The IO layer links the HDF5 library (libhdf5), so building needs one. **Minimum supported Rust version (measured): 1.85.1**, the oldest compiler that builds and tests the workspace. It is set in every crate, and a test checks it against what the dependencies declare.

**With libhdf5 installed**, the usual commands work:

```text
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

To install libhdf5 as CI does: on Debian or Ubuntu, `sudo apt-get install libhdf5-dev pkg-config`. On Windows, `vcpkg install hdf5:x64-windows`, then set `HDF5_DIR` to `<vcpkg root>\installed\x64-windows` and put its `bin` folder on `PATH` so the DLLs are found.

**On a machine without libhdf5**, compile it from source with the `static-hdf5` feature. It needs CMake and a C compiler (on Windows, the Visual Studio C++ tools). The first build takes about three minutes and leaves libhdf5 in `target/`, so later builds are fast. Add `--features build-log/static-hdf5` to each command that builds the workspace:

```text
cargo build --workspace --features build-log/static-hdf5
cargo test --workspace --features build-log/static-hdf5
cargo clippy --workspace --all-targets --features build-log/static-hdf5 -- -D warnings
cargo fmt --all -- --check
```

**On Windows, keep the path short.** CMake fails to detect the compiler when a path passes 260 characters, and this build's own folders add about 125 to the path of `target/`. Keep the checkout path under about 130 characters, or set `CARGO_TARGET_DIR` to a short folder.

The from-source build compiles the libhdf5 that the binding bundles (2.x at the time of writing). Consumers run other versions, which is why CI tests several.

## Continuous integration

[`.github/workflows/rust.yml`](.github/workflows/rust.yml) runs on pull requests, on pushes to `main` and by hand.

| Check | What it does |
|---|---|
| **`Test`** | **The required check.** Passes only if every job in this table except the two advisory ones succeeded; a job that was skipped or cancelled counts as a failure. It is the one name the branch ruleset on `main` requires, from GitHub Actions. |
| `Lint` | `cargo fmt --check` and `clippy -D warnings`. |
| `Test (ubuntu-latest)`, `Test (windows-latest)`, `Test (debian-bookworm)` | The whole test suite on each, with libhdf5 from apt (Ubuntu, and Debian 12 in a container), or from vcpkg (Windows). |
| `Feature matrix` | The facade builds with each combination of its features. |
| `Commit messages` | commitlint over the commits of the change. |
| `WebAssembly check (advisory)` | The Model builds for `wasm32-unknown-unknown`. |
| `Static libhdf5 (advisory)` | The IO crate builds and passes its tests with libhdf5 compiled from source. |

The advisory jobs show on a pull request but never block it.

**Which libhdf5 each leg uses.** Every test leg runs [`report-libhdf5.sh`](.github/scripts/report-libhdf5.sh) before its tests. It reports the version the library itself says it is linked against, in the job summary (section "libhdf5 on ...") and as the artifact `libhdf5-<leg>`, and fails the leg if it is older than 1.10.8, the candidate minimum. On 2026-10-20 the legs reported:

| Leg | libhdf5 |
|---|---|
| `Test (debian-bookworm)` | 1.10.8 (the leg fails unless it is exactly this) |
| `Test (ubuntu-latest)` | 1.10.10 |
| `Test (windows-latest)` | 2.2.0 (vcpkg's current package) |

**Changing the workflow.** Keep `Test` as the name of the aggregator job, and name matrix legs `Test (<leg>)`, so that no other job has the required name. A new job that should gate merging must be added to the aggregator's `needs` and to the results it checks. On Windows only vcpkg's *archives* are cached, never its `installed/` folder, which went stale when the runner's compiler changed in a sister project; if a run fails with "Unable to locate HDF5 root directory and/or headers" (the binding) or "Could NOT find HDF5" (CMake) after a runner image update, look at the vcpkg and `HDF5_DIR` steps first. The `workflow_dispatch` input `simulate` makes the lint job fail or be skipped, to show that `Test` then fails.

## Conventions

- **Commits** follow Conventional Commits (`type(scope): subject`), checked by commitlint on commit and in CI; the types and scopes are in [`commitlint.config.cjs`](commitlint.config.cjs). The type decides the release a change would make (`feat` a minor one, `fix`, `perf`, `refactor` and `revert` a patch, the rest none); release automation is not built yet.
- **Tests** are named for the claim they prove and live where [`docs/test-conventions.md`](docs/test-conventions.md) says.
- **Pull requests** use [the template](.github/pull_request_template.md).
- **Test data**: the miniatures in [`testdata/`](testdata/README.md) are never rebuilt; the originals they were cut from are not in this repository.
- **The Model** is documented against the file it mirrors in [`docs/models/derivation.md`](docs/models/derivation.md).

## License

Internal; no license file yet.
