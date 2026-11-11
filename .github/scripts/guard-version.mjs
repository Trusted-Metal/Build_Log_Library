#!/usr/bin/env node
/**
 * Refuses a candidate version that would jump the series (IP-26 phase 26I, decision D26-13).
 *
 *   node .github/scripts/guard-version.mjs <version>
 *
 * Called by semantic-release (`verifyReleaseCmd` in .releaserc.json) with the version it has worked out, BEFORE any commit or tag exists. It fails the
 * run when the version is a candidate (it has a prerelease part) of a major number above 0 while no stable version has been released yet.
 *
 * Why: semantic-release does not know a hand-made tag as a release unless a git note says so (docs/release-process.md, "How the series was seeded").
 * Without the note it finds no earlier release and numbers the next candidate 1.0.0-rc.1, which the plan rules out; and the tag ruleset (no update, no
 * deletion) would make that tag permanent. A stable version, and every candidate after the first stable version, passes.
 */
import { execFileSync } from "node:child_process";

const version = process.argv[2];
if (!version || !/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) {
  console.error(`guard-version: give a version such as 0.1.0-rc.2 (got ${JSON.stringify(version)})`);
  process.exit(1);
}

const tags = execFileSync("git", ["tag", "--list", "v[0-9]*"], { encoding: "utf8" }).split("\n").map((t) => t.trim()).filter(Boolean);
const stable = tags.filter((tag) => !tag.includes("-"));
const isCandidate = version.includes("-");
const major = Number(version.split(".")[0]);

if (isCandidate && major !== 0 && stable.length === 0) {
  console.error(
    `guard-version: the next candidate would be ${version}, but until the first stable version the series is 0.y.z-rc.N (decision D26-13). ` +
      `Most likely the note that tells semantic-release about the hand-made tag is missing on the remote; see "How the series was seeded" in docs/release-process.md. ` +
      `Nothing was created. Tags seen: ${tags.join(", ") || "none"}.`,
  );
  process.exit(1);
}
console.log(`guard-version: ${version} follows the series (${stable.length} stable tag(s), ${tags.length} tag(s) in all)`);
