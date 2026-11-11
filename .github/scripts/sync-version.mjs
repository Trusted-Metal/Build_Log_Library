#!/usr/bin/env node
/**
 * Sets the library's version in every place it is written (IP-26 phase 26I, decision D26-4).
 *
 *   node .github/scripts/sync-version.mjs <version> [--root <directory>]
 *
 * Called by semantic-release (`prepareCmd` in .releaserc.json) with the version it has just worked out, before the
 * release commit and the tag are made, so the tag always equals `CARGO_PKG_VERSION`. The places are:
 *
 *   - `[workspace.package] version` in the root Cargo.toml;
 *   - the version of each workspace member that `[workspace.dependencies]` lists by path (`cargo package --workspace`
 *     refuses a path dependency without one, so the two must always agree);
 *   - the version of each workspace member in Cargo.lock.
 *
 * The members are read from the workspace itself (`members` in the root Cargo.toml), so a crate added later needs no
 * change here. The script then reads everything back and fails if any place differs, because the release commit it
 * leads to is pushed with [skip ci] and a wrong Cargo.lock would break the next run of every pull request (`--locked`).
 *
 * When GITHUB_OUTPUT is set (a GitHub Actions step), it writes `version=<version>` there, which the back-merge step reads.
 */
import { appendFileSync, existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const VERSION_SHAPE = /^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/;

function fail(message) {
  console.error(`sync-version: ${message}`);
  process.exit(1);
}

/** Runs `edit` on each line inside the table `[header]`; a line it returns a string for is a target (replaced), null is skipped. */
function editTable(text, header, edit) {
  const lines = text.split("\n");
  let inside = false;
  let targets = 0;
  for (let i = 0; i < lines.length; i += 1) {
    const trimmed = lines[i].trim();
    if (trimmed.startsWith("[")) {
      inside = trimmed === `[${header}]`;
      continue;
    }
    if (!inside) continue;
    const next = edit(lines[i]);
    if (next !== null) {
      lines[i] = next;
      targets += 1;
    }
  }
  return { text: lines.join("\n"), targets };
}

/** The names of the workspace members, from `members` in the root Cargo.toml (`dir/*` globs and plain directories). */
function memberNames(root) {
  const manifest = readFileSync(join(root, "Cargo.toml"), "utf8");
  const listed = manifest.match(/members\s*=\s*\[([^\]]*)\]/);
  if (!listed) fail("no `members` list in the root Cargo.toml");
  const names = new Set();
  for (const entry of [...listed[1].matchAll(/"([^"]+)"/g)].map((m) => m[1])) {
    const directories = entry.endsWith("/*")
      ? readdirSync(join(root, entry.slice(0, -2)), { withFileTypes: true })
          .filter((d) => d.isDirectory())
          .map((d) => join(root, entry.slice(0, -2), d.name))
      : [join(root, entry)];
    for (const directory of directories) {
      const path = join(directory, "Cargo.toml");
      if (!existsSync(path)) continue;
      editTable(readFileSync(path, "utf8"), "package", (line) => {
        const m = line.match(/^name\s*=\s*"([^"]+)"/);
        if (m) names.add(m[1]);
        return null;
      });
    }
  }
  if (names.size === 0) fail("no workspace member found");
  return names;
}

const setWorkspaceVersion = (text, version) =>
  editTable(text, "workspace.package", (line) =>
    /^version\s*=\s*"[^"]*"/.test(line) ? line.replace(/^(version\s*=\s*)"[^"]*"/, `$1"${version}"`) : null,
  );

/** A workspace member listed by path must carry a version: replace it. (The facade is not listed, and is skipped.) */
const setDependencyVersions = (text, members, version) =>
  editTable(text, "workspace.dependencies", (line) => {
    const entry = line.match(/^([A-Za-z0-9_-]+)\s*=\s*\{(.*)\}/);
    if (!entry || !members.has(entry[1]) || !/\bpath\s*=/.test(entry[2])) return null;
    if (!/\bversion\s*=\s*"[^"]*"/.test(entry[2])) {
      fail(`workspace dependency ${entry[1]} is listed by path without a version (cargo package refuses that)`);
    }
    return line.replace(/(\bversion\s*=\s*)"[^"]*"/, `$1"${version}"`);
  });

/** Cargo.lock: the `version` line under each `name = "<member>"`. Returns the new text and how many times each member was found. */
function setLockVersions(text, members, version) {
  const lines = text.split("\n");
  const found = new Map();
  for (let i = 0; i < lines.length - 1; i += 1) {
    const name = lines[i].match(/^name = "([^"]+)"/);
    if (!name || !members.has(name[1]) || !/^version = "/.test(lines[i + 1])) continue;
    lines[i + 1] = lines[i + 1].replace(/^(version = )"[^"]*"/, `$1"${version}"`);
    found.set(name[1], (found.get(name[1]) ?? 0) + 1);
  }
  return { text: lines.join("\n"), found };
}

/** Reads every place back, for the check. */
function readBack(root, members) {
  const manifest = readFileSync(join(root, "Cargo.toml"), "utf8");
  const lock = readFileSync(join(root, "Cargo.lock"), "utf8");
  const places = new Map();
  editTable(manifest, "workspace.package", (line) => {
    const m = line.match(/^version\s*=\s*"([^"]*)"/);
    if (m) places.set("Cargo.toml [workspace.package]", m[1]);
    return null;
  });
  editTable(manifest, "workspace.dependencies", (line) => {
    const entry = line.match(/^([A-Za-z0-9_-]+)\s*=\s*\{(.*)\}/);
    if (!entry || !members.has(entry[1]) || !/\bpath\s*=/.test(entry[2])) return null;
    places.set(`Cargo.toml [workspace.dependencies] ${entry[1]}`, entry[2].match(/\bversion\s*=\s*"([^"]*)"/)?.[1] ?? "(none)");
    return null;
  });
  const lines = lock.split("\n");
  for (let i = 0; i < lines.length - 1; i += 1) {
    const name = lines[i].match(/^name = "([^"]+)"/);
    const version = lines[i + 1].match(/^version = "([^"]*)"/);
    if (name && members.has(name[1]) && version) places.set(`Cargo.lock ${name[1]}`, version[1]);
  }
  return places;
}

function main() {
  const args = process.argv.slice(2);
  const rootFlag = args.indexOf("--root");
  const root =
    rootFlag >= 0
      ? resolve(args.splice(rootFlag, 2)[1] ?? fail("--root needs a directory"))
      : join(dirname(fileURLToPath(import.meta.url)), "..", "..");
  const version = args[0];
  if (!version || !VERSION_SHAPE.test(version)) fail(`give a version such as 0.2.0-rc.1 or 1.0.0 (got ${JSON.stringify(version)})`);

  const members = memberNames(root);
  console.log(`sync-version: ${version} for ${[...members].sort().join(", ")}`);

  const manifestPath = join(root, "Cargo.toml");
  const lockPath = join(root, "Cargo.lock");

  const first = setWorkspaceVersion(readFileSync(manifestPath, "utf8"), version);
  if (first.targets !== 1) fail(`expected one version line in [workspace.package], found ${first.targets}`);
  const second = setDependencyVersions(first.text, members, version);
  writeFileSync(manifestPath, second.text, "utf8");

  const lock = setLockVersions(readFileSync(lockPath, "utf8"), members, version);
  for (const name of members) {
    if (lock.found.get(name) !== 1) fail(`Cargo.lock has ${lock.found.get(name) ?? 0} entries for ${name}, expected exactly one`);
  }
  writeFileSync(lockPath, lock.text, "utf8");

  const wrong = [...readBack(root, members)].filter(([, found]) => found !== version);
  if (wrong.length > 0) fail(`after the edit these still differ: ${wrong.map(([place, found]) => `${place} = ${found}`).join("; ")}`);
  console.log(`sync-version: the workspace version, ${second.targets} workspace dependencies and ${members.size} Cargo.lock entries are ${version}`);

  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `version=${version}\n`);
}

main();
