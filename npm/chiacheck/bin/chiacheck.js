#!/usr/bin/env node
"use strict";

const path = require("node:path");
const { spawnSync } = require("node:child_process");

// Single source of truth: node `${platform}-${arch}` -> { pkg, bin }.
// Keep in sync with the build matrix and scripts/assemble-npm-packages.mjs.
const PLATFORMS = {
  "darwin-arm64": { pkg: "@chiacheck/darwin-arm64", bin: "chiacheck" },
  "darwin-x64": { pkg: "@chiacheck/darwin-x64", bin: "chiacheck" },
  "linux-x64": { pkg: "@chiacheck/linux-x64", bin: "chiacheck" },
  "linux-arm64": { pkg: "@chiacheck/linux-arm64", bin: "chiacheck" },
  "win32-x64": { pkg: "@chiacheck/win32-x64", bin: "chiacheck.exe" },
};

function resolveBinary() {
  const key = `${process.platform}-${process.arch}`;
  const entry = PLATFORMS[key];
  if (!entry) {
    const supported = Object.keys(PLATFORMS).join(", ");
    throw new Error(
      `chiacheck: unsupported platform "${key}".\n` +
        `Supported platforms: ${supported}.\n` +
        `If you need this platform, please open an issue: ` +
        `https://github.com/joshcartme/chiacheck/issues`
    );
  }
  try {
    // require.resolve finds the installed optionalDependency regardless of
    // hoisting; the binary lives next to its package.json.
    const pkgJson = require.resolve(`${entry.pkg}/package.json`);
    return path.join(path.dirname(pkgJson), entry.bin);
  } catch (_e) {
    throw new Error(
      `chiacheck: the platform package "${entry.pkg}" is not installed.\n` +
        `This usually means optionalDependencies were skipped during install ` +
        `(e.g. --no-optional / --omit=optional, or an npm bug).\n` +
        `Try reinstalling and allowing optional dependencies:\n` +
        `  npm install chiacheck\n`
    );
  }
}

function main() {
  let binary;
  try {
    binary = resolveBinary();
  } catch (err) {
    process.stderr.write(`${err.message}\n`);
    process.exit(1);
  }

  const result = spawnSync(binary, process.argv.slice(2), {
    stdio: "inherit",
    windowsHide: true,
  });

  if (result.error) {
    process.stderr.write(
      `chiacheck: failed to launch binary at ${binary}: ${result.error.message}\n`
    );
    process.exit(1);
  }

  // Propagate a terminating signal by re-raising it; otherwise pass the code.
  if (result.signal) {
    process.kill(process.pid, result.signal);
    return;
  }
  process.exit(result.status === null ? 1 : result.status);
}

module.exports = { PLATFORMS };

if (require.main === module) {
  main();
}
