#!/usr/bin/env node
// Resolve and validate the release version from in-repo sources.
//
// Reads chiacheck/Cargo.toml's [package].version and npm/chiacheck/package.json's
// version and fails if they disagree, so the two can never drift apart. On success
// it prints the agreed version to stdout (no trailing noise) for the release
// workflow to capture.
//
// Usage:
//   node scripts/release-version.mjs

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "..");

function fail(msg) {
  console.error(`release-version: ${msg}`);
  process.exit(1);
}

function cargoVersion() {
  const cargo = readFileSync(join(repoRoot, "chiacheck/Cargo.toml"), "utf8");
  // Match the first `version = "x"` after the [package] header.
  const pkg = cargo.split(/\[[^\]]+\]/)[1] ?? cargo; // section after first header ([package])
  const m = pkg.match(/^\s*version\s*=\s*"([^"]+)"/m);
  if (!m) fail("could not find [package].version in chiacheck/Cargo.toml");
  return m[1];
}

function npmVersion() {
  const pj = JSON.parse(readFileSync(join(repoRoot, "npm/chiacheck/package.json"), "utf8"));
  if (!pj.version) fail("npm/chiacheck/package.json has no version");
  return pj.version;
}

const cargo = cargoVersion();
const npm = npmVersion();
if (cargo !== npm) {
  fail(
    `version mismatch: chiacheck/Cargo.toml is "${cargo}" but npm/chiacheck/package.json is "${npm}". ` +
      `Bump both to the same version.`
  );
}
process.stdout.write(cargo + "\n");
