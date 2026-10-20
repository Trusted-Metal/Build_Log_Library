## What this changes and why

<!-- One or two sentences. The pull request title is a Conventional Commit: `type(scope): subject` (see commitlint.config.cjs). -->

## Requirements and tests

- **Requirement ids this covers** (for example `BL-AR-02`):
- **Named tests that show it** (the test names, not "tests pass"):

## Boundaries

- [ ] **No MCF.** Nothing here loads a machine configuration or depends on an MCF crate or package (boundary rules M1 to M6).
- [ ] **Layering holds.** The Model has no HDF5; Processing does not depend on IO; nothing in the workspace depends on the `build-log` facade.
- [ ] **No feature changes the on-disk schema.**

## Fixtures

- [ ] **V18 is provisional.** If this depends on a fact about V18, the test that asserts it is guarded by the pinned hash, and the impact of V18 changing is written here:
- [ ] **Miniatures in `testdata/`** are untouched, or changed with the owner's approval and a new line in `testdata/SHA256SUMS` (a miniature is never rebuilt).
- [ ] **No original or large fixture** is read by Rust code or added to the repository.

## Checks

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
