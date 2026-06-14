#!/usr/bin/env node
// Assemble publishable npm packages under npm/dist/ from prebuilt binaries.
//
// For each target it writes a platform package (@chiacheck/<platform>) containing the
// raw binary + a package.json gated by os/cpu(/libc), and it writes the launcher
// package (chiacheck) with `version` and every `optionalDependencies` entry pinned to
// the release version (exact, no ranges).
//
// Usage:
//   node scripts/assemble-npm-packages.mjs [version] [options]
//
//   version            Release version, e.g. 0.1.0. If omitted, derived from
//                      $GITHUB_REF_NAME (strips a leading "v") or read from Cargo.toml.
//   --artifacts <dir>  Directory holding downloaded build artifacts, one subdir per
//                      target named "bin-<triple>" containing the binary (default: artifacts).
//   --out <dir>        Output directory for assembled packages (default: npm/dist).
//   --allow-missing    Skip targets whose artifact is absent (for local single-platform
//                      testing). Default: every target must be present.
//
// The version is always asserted to equal chiacheck/Cargo.toml's [package].version so a
// mistagged release fails fast instead of publishing a mismatch.

import { existsSync, readFileSync, rmSync, mkdirSync, copyFileSync, writeFileSync, chmodSync } from "node:fs";
import { dirname, isAbsolute, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(__dirname, "..");

// Single source of truth for the build/publish matrix.
// Keep in sync with npm/chiacheck/bin/chiacheck.js PLATFORMS and .github/workflows/release.yml.
const TARGETS = [
  { triple: "aarch64-apple-darwin", pkg: "@chiacheck/darwin-arm64", os: "darwin", cpu: "arm64", bin: "chiacheck" },
  { triple: "x86_64-apple-darwin", pkg: "@chiacheck/darwin-x64", os: "darwin", cpu: "x64", bin: "chiacheck" },
  { triple: "x86_64-unknown-linux-gnu", pkg: "@chiacheck/linux-x64", os: "linux", cpu: "x64", libc: "glibc", bin: "chiacheck" },
  { triple: "aarch64-unknown-linux-gnu", pkg: "@chiacheck/linux-arm64", os: "linux", cpu: "arm64", libc: "glibc", bin: "chiacheck" },
  { triple: "x86_64-pc-windows-msvc", pkg: "@chiacheck/win32-x64", os: "win32", cpu: "x64", bin: "chiacheck.exe" },
];

function parseArgs(argv) {
  const opts = { artifacts: "artifacts", out: "npm/dist", allowMissing: false, version: undefined };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--allow-missing") opts.allowMissing = true;
    else if (a === "--artifacts") opts.artifacts = argv[++i];
    else if (a === "--out") opts.out = argv[++i];
    else if (a.startsWith("--")) fail(`Unknown option: ${a}`);
    else opts.version = a;
  }
  return opts;
}

function fail(msg) {
  console.error(`assemble-npm-packages: ${msg}`);
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

function resolveVersion(arg) {
  if (arg) return arg;
  const ref = process.env.GITHUB_REF_NAME;
  if (ref) return ref.replace(/^v/, "");
  return cargoVersion();
}

function writeJson(path, obj) {
  writeFileSync(path, JSON.stringify(obj, null, 2) + "\n");
}

function main() {
  const opts = parseArgs(process.argv.slice(2));
  const version = resolveVersion(opts.version);
  const expected = cargoVersion();
  if (version !== expected) {
    fail(`version mismatch: requested "${version}" but chiacheck/Cargo.toml is "${expected}". ` +
      `Bump Cargo.toml and the tag together.`);
  }

  const artifactsDir = resolve(repoRoot, opts.artifacts);
  const outDir = resolve(repoRoot, opts.out);
  const licenseSrc = join(repoRoot, "LICENSE");
  // Safety: we recursively delete outDir, so refuse anything that isn't strictly
  // inside the repo. `relative` is "" when equal, starts with ".." when outside,
  // and is absolute across drives on Windows — reject all three.
  const outRel = relative(repoRoot, outDir);
  if (outRel === "" || outRel.startsWith("..") || isAbsolute(outRel)) {
    fail(`refusing to use --out "${opts.out}": resolves to ${outDir}, which is not inside the repository (${repoRoot})`);
  }
  rmSync(outDir, { recursive: true, force: true });
  mkdirSync(outDir, { recursive: true });

  const assembled = [];
  for (const t of TARGETS) {
    const srcBin = join(artifactsDir, `bin-${t.triple}`, t.bin);
    if (!existsSync(srcBin)) {
      if (opts.allowMissing) {
        console.warn(`  skip ${t.pkg} (missing artifact: ${srcBin})`);
        continue;
      }
      fail(`missing artifact for ${t.triple}: ${srcBin} (use --allow-missing for local testing)`);
    }
    const pkgDir = join(outDir, t.pkg);
    mkdirSync(pkgDir, { recursive: true });
    copyFileSync(srcBin, join(pkgDir, t.bin));
    if (t.os !== "win32") chmodSync(join(pkgDir, t.bin), 0o755);

    const pj = {
      name: t.pkg,
      version,
      description: `chiacheck binary for ${t.os}-${t.cpu}`,
      repository: { type: "git", url: "git+https://github.com/joshcartme/chiacheck.git" },
      license: "MIT",
      os: [t.os],
      cpu: [t.cpu],
      ...(t.libc ? { libc: [t.libc] } : {}),
      files: [t.bin, "LICENSE"],
    };
    writeJson(join(pkgDir, "package.json"), pj);
    copyFileSync(licenseSrc, join(pkgDir, "LICENSE"));
    writeFileSync(
      join(pkgDir, "README.md"),
      `# ${t.pkg}\n\nThe ${t.os}-${t.cpu} binary for [chiacheck](https://www.npmjs.com/package/chiacheck). ` +
        `Installed automatically as an optional dependency of \`chiacheck\`; do not depend on it directly.\n`
    );
    assembled.push(t.pkg);
    console.log(`  ok   ${t.pkg}`);
  }

  if (assembled.length === 0) fail("no platform packages assembled (no artifacts found)");

  // Launcher package: copy committed source, pin version + every optionalDependency.
  const launcherSrc = join(repoRoot, "npm/chiacheck");
  const launcherOut = join(outDir, "chiacheck");
  mkdirSync(join(launcherOut, "bin"), { recursive: true });
  copyFileSync(join(launcherSrc, "bin/chiacheck.js"), join(launcherOut, "bin/chiacheck.js"));
  copyFileSync(join(launcherSrc, "README.md"), join(launcherOut, "README.md"));
  copyFileSync(licenseSrc, join(launcherOut, "LICENSE"));

  const launcher = JSON.parse(readFileSync(join(launcherSrc, "package.json"), "utf8"));
  launcher.version = version;
  for (const t of TARGETS) {
    if (launcher.optionalDependencies && t.pkg in launcher.optionalDependencies) {
      launcher.optionalDependencies[t.pkg] = version;
    }
  }
  writeJson(join(launcherOut, "package.json"), launcher);
  console.log(`  ok   chiacheck (launcher) -> ${launcherOut}`);

  console.log(`\nAssembled ${assembled.length} platform package(s) + launcher at version ${version} in ${outDir}`);
}

main();
