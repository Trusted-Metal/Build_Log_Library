# Releases

How the Build Log Library is released, how an application pins a release, and what a repository administrator sets up once. Nothing is published to a registry: the library is used as a git dependency, so **a tag and a GitHub release are the delivery**.

## In short

- **A push to `main` makes a release candidate** (`v0.1.0-rc.2`, `v0.1.0-rc.3`, ...), a GitHub pre-release. **A push to `release` makes a stable version** (`v1.0.0`, ...), a GitHub release.
- **Both happen only after the required check `Test` succeeded on that commit**, and only if a commit since the last release says so (below). Nobody tags by hand.
- **An application pins a tag, never a commit.** `build-log = { git = "https://github.com/Trusted-Metal/Build_Log_Library", tag = "v0.1.0-rc.2" }`.
- **Candidates only exist so that a consumer can pin something until the first stable version.** The first stable version, `1.0.0`, is a stability promise and is made deliberately, at the freeze of the read and write surfaces.

## How a candidate arises

The commits in a pull request are Conventional Commits (`type(scope): subject`; the pull request title is one too, and commitlint checks them). When a pull request is merged into `main`, the `Release` job of the Rust workflow reads the commits since the last release:

| Commit type | Release |
|---|---|
| `feat` | a candidate |
| `fix`, `perf`, `refactor`, `revert` | a candidate |
| a breaking change (a `BREAKING CHANGE:` footer, or `!` after the type or scope) | a candidate (see the numbering below) |
| `docs`, `style`, `test`, `build`, `ci`, `chore` | **nothing**: the job runs and ends with "no relevant changes" |

If there is something to release, the job, in this order:

1. works out the next version from the tags and the commit types;
2. checks that it follows the series (the guard, below);
3. sets the version in the root `Cargo.toml` (the workspace version and the six workspace dependency versions) and in `Cargo.lock`, and checks that `cargo tree --locked` accepts the result;
4. commits that as `chore(release): bump version to X [skip ci]` on `main` (the `[skip ci]` starts no run), tags the commit `vX`, and creates the GitHub release with notes generated from the commits.

**The tag always equals the version in `Cargo.toml`**, so `build_log::library_versions()` reports the version of the tag a consumer pinned.

### The numbering

Until the freeze every candidate is **`0.1.0-rc.N`**. `N` counts candidates, whatever the commit types: a `feat`, a `fix` and a breaking change each make the next `N`. The rule for a breaking change says *minor* until the freeze, which keeps the series below 1.0.0 (a *major* rule would make the first breaking commit `1.0.0-rc.1`, the way the Machine Config Library's series jumped from `v0.2.0-rc.10` to `v1.0.0-rc.1`); it does not raise the second number, because the candidates are already the prerelease of `0.1.0`. **A change that breaks a consumer is named in the release notes of the candidate that makes it** (under "BREAKING CHANGES"), so a consumer reads the notes before it moves its pin.

After the first stable version, candidates follow it: `1.1.0-rc.1` after a `feat`, and `2.0.0-rc.1` after a breaking change.

### The guard

Before anything is created, `.github/scripts/guard-version.mjs` refuses a candidate of a major number above 0 while no stable version exists. This is what protects against the series restarting at `1.0.0-rc.1` if the seed note (below) is ever missing; the tag ruleset would make such a tag permanent.

## How an application pins and moves a pin

1. Pin the candidate in the application's `Cargo.toml`: `tag = "v0.1.0-rc.2"`. Cargo records the tag and the commit in `Cargo.lock`.
2. To move: change `tag`, run `cargo update -p build-log`, run the application's own tests, and link the release notes of the new candidate in the pull request description.
3. A candidate may break between candidates until the first stable version. The read surface (`open`, `open_at` and the Model types) changes only by addition; anything else is named in the notes of the candidate that makes it. So a pin moves deliberately and never by itself.

The notes of the first candidate, `v0.1.0-rc.1`, which was made by hand, are in [docs/releases/v0.1.0-rc.1.md](releases/v0.1.0-rc.1.md).

## How to make a stable version (at the freeze)

Done once, deliberately, when the read and write surfaces are frozen:

1. **In a pull request into `main`:** change the last rule of the commit-analyzer in `.releaserc.json` (the breaking change) from `"minor"` to `"major"`, and merge it. From then on a breaking change starts the next major candidate.
2. **Open a pull request from `main` into `release`** and merge it (with the required `Test` and a reviewer). The push to `release` makes `1.0.0` as a GitHub release, not a pre-release.
3. **The job then merges `release` back into `main`** (strategy `ours`, so `main` keeps its files) and sets the version in the files of `main` to the stable one. Without that merge the stable tag would not be in the history of `main`, and the next candidate would be numbered from the old version. This step is in `.github/scripts/back-merge.sh`.

Nobody pushes to `release` by hand: its ruleset takes pull requests only, and the first push to it that has releasable commits behind it makes `1.0.0`.

## One-time setup (a repository administrator)

Everything here is done once. The first four are needed before the release job first runs on `main`.

1. **The `release` branch must exist** before the first run. semantic-release refuses to run at all unless at least one stable branch exists on the remote (`ERELEASEBRANCHES`). Create it at the commit of the first tag, **before the workflow that watches it is on `main`**: `git push origin v0.1.0-rc.1^{commit}:refs/heads/release`. Creating it afterwards would be a push to `release` and would make `1.0.0` at once.
2. **The seed note.** semantic-release knows a tag as a release only if a git note says so, and the first candidate was made by hand. Without the note it finds no earlier release and numbers the next candidate `1.0.0-rc.1` (the guard stops that). The note goes on the *commit* of the tag, with the channel `main` (the name of the branch that makes candidates):
   ```
   git notes --ref semantic-release-v0.1.0-rc.1 add -f -m '{"channels":["main"]}' "v0.1.0-rc.1^{commit}"
   git push origin refs/notes/semantic-release-v0.1.0-rc.1
   ```
   With it, the next releasable commit makes `v0.1.0-rc.2`. (Found in the rehearsal: without the note, `1.0.0-rc.1`; the note on the annotated tag object instead of its commit has no effect.)
3. **The deploy key.** The release job pushes a commit and a tag to protected branches, so it pushes with a key that the rulesets let through.
   - Make a new key pair for this repository (a key can belong to one repository only), **in your home directory and never inside the repository**, so that the private key cannot be committed by mistake. In Git Bash (PowerShell mangles the empty passphrase): `ssh-keygen -t ed25519 -C "build-log-library release job" -f ~/build-log-release-key -N ""`. The key has no passphrase because the job cannot type one.
   - Settings, Deploy keys: add the public key (`~/build-log-release-key.pub`) with **Allow write access**.
   - Settings, Secrets and variables, Actions: a repository secret named **`RELEASE_DEPLOY_KEY`** holding the private key file (`~/build-log-release-key`), with its BEGIN and END lines.
   - Delete both key files from the machine, and clear the clipboard if the private key went through it. If the secret is lost, make a new pair.
   - The job prints the key's fingerprint; compare it with the one under Deploy keys.
4. **The rulesets.**
   - `main` (branch): the existing ruleset, with **Deploy keys, Always allow** in its bypass list.
   - `release` (branch): pull requests only, the required check `Test` from GitHub Actions, block deletions and force pushes, and **Deploy keys, Always allow** in the bypass list.
   - `v*` (tag): restrict updates and deletions and block force pushes; leave **creations** unrestricted (the release job creates the tags) and the bypass list empty.
5. **GitHub Actions settings:** the job asks for `contents: write` itself. If the organisation restricts the default workflow permissions, allow the workflow to request it.

`GITHUB_TOKEN` is automatic and creates the GitHub release; only the deploy key is provided by hand.

## Checking that it works (a dry run)

Actions, the Rust workflow, Run workflow, on `main`. **A manual run never releases:** the `Release` job runs `semantic-release --dry-run`, which loads the deploy key (proving the secret and the key), reads the history, and prints "The next release version is ..." or "There are no relevant changes" without creating anything.

To see that a failed check stops a release, run it again with `simulate` set to `fail-lint`: the required check fails and the `Release` job is skipped.

## If a release run fails

- **"the secret RELEASE_DEPLOY_KEY is not set":** the setup step 3.
- **The push is refused** ("protected branch", "rule violations"): the deploy key is not in the bypass list of that ruleset (step 4).
- **`guard-version` refuses:** the seed note is missing from the remote (step 2). Nothing was created; add it and run again.
- **`ERELEASEBRANCHES`:** the `release` branch does not exist on the remote (step 1).
- **The job is skipped:** the required check failed, which is the intent. Fix the check; the release follows from the next push that passes.
- **A run failed after the tag was pushed but before the GitHub release was created:** create the release from the tag by hand (Releases, Draft a new release, choose the tag; tick pre-release for a candidate). Do not delete or move the tag: its ruleset forbids it.

## The files

| File | What it is |
|---|---|
| `.releaserc.json` | the release rules: the branches, the commit types, the breaking-change rule (the one to change at the freeze), the plugins |
| `.github/workflows/rust.yml` | the `Release` job (after `Test`, on a push to `main` or `release`; a manual run is a dry run) |
| `.github/scripts/sync-version.mjs` | sets the version in `Cargo.toml` and `Cargo.lock` and checks every place |
| `.github/scripts/guard-version.mjs` | refuses a candidate that would restart the series at 1.0.0 |
| `.github/scripts/configure-deploy-key-git.sh` | loads the deploy key and makes git push over SSH |
| `.github/scripts/back-merge.sh` | merges `release` back into `main` after a stable version |
| `package.json`, `package-lock.json`, `tooling/` | the tooling (semantic-release and its plugins, commitlint); `tooling/npm-cli-stub` replaces the npm package that a plugin we do not use would pull in with open advisories |
