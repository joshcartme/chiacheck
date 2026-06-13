#!/usr/bin/env bash
# Publish assembled npm packages from npm/dist: every @chiacheck/* platform package
# first, then the chiacheck launcher last (so its optionalDependencies already exist
# on the registry). Idempotent: a version already on the registry is skipped, so
# a partially-failed release can be re-run safely.
#
# Auth: with `id-token: write` + a configured npm trusted publisher this uses
# OIDC automatically. To bootstrap (first publish, before trusted publishers can
# be configured), set NODE_AUTH_TOKEN via the NPM_TOKEN secret in the workflow.
set -euo pipefail

DIST="${1:-npm/dist}"

publish_pkg() {
  local dir="$1"
  local name version
  name=$(node -p "require('$dir/package.json').name")
  version=$(node -p "require('$dir/package.json').version")
  if npm view "$name@$version" version >/dev/null 2>&1; then
    echo "skip   $name@$version (already published)"
    return 0
  fi
  echo "publish $name@$version"
  ( cd "$dir" && npm publish --provenance --access public )
}

shopt -s nullglob
for dir in "$DIST"/@chiacheck/*; do
  [ -d "$dir" ] && publish_pkg "$dir"
done

publish_pkg "$DIST/chiacheck"

echo "Done."
