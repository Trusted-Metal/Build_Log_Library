#!/usr/bin/env bash
# Loads the secret RELEASE_DEPLOY_KEY and makes git push over SSH with it (IP-26 phase 26I; the same recipe as the Machine Config Library's).
#
# Why: the release job pushes a version commit to `main` (or `release`) and a tag, and those branches are protected by rulesets that a
# pull request has to satisfy. A deploy key with write access and "Deploy keys / Always allow" in the ruleset's bypass list is the one
# way past that which is limited to this repository. The key's fingerprint is printed so that it can be compared with the one GitHub
# shows under Settings, Deploy keys. A key can belong to one repository only: make a new pair for each (docs/release-process.md).
set -euo pipefail

if [ -z "${RELEASE_DEPLOY_KEY:-}" ]; then
  echo "::error::the secret RELEASE_DEPLOY_KEY is not set (Settings, Secrets and variables, Actions): see docs/release-process.md"
  exit 1
fi

mkdir -p ~/.ssh
echo "$RELEASE_DEPLOY_KEY" > ~/.ssh/deploy_key
chmod 600 ~/.ssh/deploy_key

# Validate the key and print its fingerprint for the log.
FINGERPRINT=$(ssh-keygen -y -f ~/.ssh/deploy_key | ssh-keygen -E sha256 -lf -)
echo "Deploy key fingerprint: ${FINGERPRINT}"

# Start an SSH agent and load the key.
eval "$(ssh-agent -s)"
ssh-add ~/.ssh/deploy_key

# Trust github.com.
ssh-keyscan -t ed25519 github.com >> ~/.ssh/known_hosts 2>/dev/null

export GIT_SSH_COMMAND="ssh -i ~/.ssh/deploy_key -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes"

# Switch the remote to SSH and hand the URL to semantic-release (--repository-url).
REPO_URL="git@github.com:${GITHUB_REPOSITORY}.git"
git remote set-url origin "${REPO_URL}"
echo "SEMANTIC_RELEASE_REPOSITORY_URL=${REPO_URL}" >> "${GITHUB_ENV}"
echo "GIT_SSH_COMMAND=${GIT_SSH_COMMAND}" >> "${GITHUB_ENV}"

git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
echo "Deploy key configured: pushes from this job use SSH as the deploy key"
