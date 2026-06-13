# Releasing chiacheck to npm

`chiacheck` ships to npm as a thin launcher package plus one prebuilt-binary package per platform,
using the optionalDependencies pattern (like Biome/esbuild). The
[`release.yml`](.github/workflows/release.yml) workflow builds all platforms and publishes them.

## Packages

| Package               | Contents                                   |
|-----------------------|--------------------------------------------|
| `chiacheck`               | JS launcher (`bin/chiacheck.js`) + optionalDeps |
| `@chiacheck/darwin-arm64` | macOS arm64 binary                          |
| `@chiacheck/darwin-x64`   | macOS x64 binary                            |
| `@chiacheck/linux-x64`    | Linux x64 (glibc) binary                    |
| `@chiacheck/linux-arm64`  | Linux arm64 (glibc) binary                  |
| `@chiacheck/win32-x64`    | Windows x64 binary                          |

The set of targets lives in one place: the `TARGETS` table in
[`scripts/assemble-npm-packages.mjs`](scripts/assemble-npm-packages.mjs). Keep the
`PLATFORMS` map in [`npm/chiacheck/bin/chiacheck.js`](npm/chiacheck/bin/chiacheck.js) and the build matrix in
`release.yml` in sync with it.

## How a release works

1. The launcher version, every `optionalDependencies` entry, and every platform package version
   are all pinned to the **same exact version** (no `^`/`~`). The single source of truth is
   `chiacheck/Cargo.toml`'s `[package].version`.
2. `release.yml` builds a binary per target, then `assemble-npm-packages.mjs` asserts the tag
   matches `Cargo.toml` and writes `npm/dist/`.
3. `publish-npm.sh` publishes every `@chiacheck/*` platform package first, then the `chiacheck` launcher
   last, so the launcher's optionalDependencies already exist when it resolves. It skips any
   version already on the registry, so a partially-failed release is safe to re-run.

## Cutting a release

```bash
# 1. Bump the version in chiacheck/Cargo.toml, then refresh Cargo.lock and commit.
cargo build            # updates Cargo.lock to the new version
git commit -am "release: vX.Y.Z"

# 2. Tag and push — the tag triggers the release workflow.
git tag vX.Y.Z
git push origin main --tags
```

A `workflow_dispatch` run with `publish=false` (the default) is a **build-only dry run** — it
compiles all five targets without publishing. Use it to validate cross-compilation (especially
`aarch64-unknown-linux-gnu` with bundled SQLite) before tagging.

## Authentication — npm Trusted Publishing (OIDC)

The workflow publishes with `id-token: write` + `npm publish --provenance`. With a configured
npm **trusted publisher**, no stored token is needed and provenance is attached automatically.

One-time setup, **for each of the six package names** (`chiacheck` and all five `@chiacheck/*`), on
npmjs.com → package → Settings → Trusted Publishing:

- Provider: GitHub Actions
- Repository: `joshcartme/chiacheck`
- Workflow: `release.yml`

Requirements (already handled by the workflow): `permissions: id-token: write` on the publish
job and `npm install -g npm@latest` (OIDC trusted publishing needs npm ≥ 11.5).

### Bootstrap (first release only)

A trusted publisher can't be configured for a package name that doesn't exist yet, so the very
first publish of each name needs a token. Two options:

- **Recommended — local manual first publish:** `npm login`, then build once and run
  `node scripts/assemble-npm-packages.mjs <version> --allow-missing` plus
  `bash scripts/publish-npm.sh` from a machine that has the binaries, or simply publish the
  packages by hand. Then configure trusted publishers and use the workflow for all later releases.
- **CI token:** add a granular automation `NPM_TOKEN` repo secret and temporarily uncomment the
  `NODE_AUTH_TOKEN` env block in `release.yml`'s publish step. After the first successful
  publish, configure trusted publishers, re-comment the block, and delete the secret.

## Scope

The `@chiacheck` npm scope must exist and you must own it (or publish under a scope you control).
Create it on npmjs.com before the first scoped publish.

## Adding a platform later (e.g. musl)

1. Add the target to the `build` matrix in `release.yml` (runner + any cross toolchain setup).
2. Add it to `TARGETS` in `assemble-npm-packages.mjs` (with `libc` for musl).
3. Add it to `PLATFORMS` in `npm/chiacheck/bin/chiacheck.js` and to `optionalDependencies` in
   `npm/chiacheck/package.json`.
4. Configure a trusted publisher for the new `@chiacheck/<platform>` package name.
