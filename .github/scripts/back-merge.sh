#!/usr/bin/env bash
# After a stable release on `release`, brings it back into `main` (IP-26 phase 26I, question Q26-7).
#
#   bash .github/scripts/back-merge.sh <stable version>
#
# Why: the stable tag (v1.0.0, ...) sits on the release commit of `release`. Until `main` has that commit in its history, semantic-release on
# `main` cannot see the stable version and would number the next candidate from the old one. The merge uses the strategy `ours`: `main` keeps
# its own files, because the only content `release` has that `main` lacks is the version commit, and a release pull request takes everything
# else from `main`. A plain merge would conflict on the version lines. The version in the files of `main` is then set to the stable one by the
# same script that sets it for a release (it is rewritten at the next candidate anyway; this only keeps the files honest in between).
# The job pushes with the deploy key (scripts/configure-deploy-key-git.sh) and both commits say [skip ci].
set -euo pipefail

version="${1:?give the stable version that was just released, for example 1.0.0}"

git fetch origin main release --tags
git checkout -B main origin/main
git merge --no-ff -s ours -m "chore(release): merge release back into main [skip ci]" origin/release

node .github/scripts/sync-version.mjs "${version}"
if ! git diff --quiet -- Cargo.toml Cargo.lock; then
  git add Cargo.toml Cargo.lock
  git commit -m "chore(release): set the version to ${version} on main [skip ci]"
fi

git push origin main
