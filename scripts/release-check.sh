#!/usr/bin/env bash
set -euo pipefail

repo_root="${BASH_SOURCE[0]%scripts/release-check.sh}"
cargo_version="$(sed -n '/^\[package\]/,/^\[/ { s/^version = "\([^"]*\)".*/\1/p; }' "$repo_root/Cargo.toml")"
changelog_version="$(sed -n '/^## / { s/^## \([^ ]*\).*/\1/; p; q; }' "$repo_root/CHANGELOG.md")"

if [[ -z "$cargo_version" || -z "$changelog_version" ]]; then
    echo "release-check: could not read Cargo.toml version ($cargo_version) or CHANGELOG.md version ($changelog_version)" >&2
    exit 1
fi

if [[ "$cargo_version" != "$changelog_version" ]]; then
    echo "release-check: Cargo.toml version is $cargo_version but CHANGELOG.md version is $changelog_version" >&2
    exit 1
fi

echo "release-check: Cargo.toml and CHANGELOG.md are both $cargo_version"
