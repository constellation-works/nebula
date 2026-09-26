#!/usr/bin/env bash
# scripts/check-pnpm-pin.sh — STD-05 §R23 for the desktop's package manager.
#
# apps/desktop/package.json pins pnpm as `pnpm@X.Y.Z+sha512.<hex>`. Neither
# pnpm/action-setup nor pnpm itself checks that hash; they only check the
# downloaded tarball against the integrity the npm registry records for X.Y.Z.
# This closes the chain: it fails unless
#
#   1. the pin has that exact form,
#   2. the registry's recorded sha512 for pnpm@X.Y.Z equals the pinned one, and
#   3. the pnpm that runs here (`$PNPM`, default `pnpm`) reports version X.Y.Z.
#
# It needs node and npm, and the network. CI runs it after installing pnpm;
# `make audit` runs it with the Makefile's PNPM.
set -euo pipefail
cd "$(dirname "$0")/.."

PACKAGE_JSON=apps/desktop/package.json
PNPM=${PNPM:-pnpm}

fail() { echo "pnpm-pin: $*" >&2; exit 1; }

pin=$(node -p "require('./$PACKAGE_JSON').packageManager ?? ''")
[[ $pin =~ ^pnpm@([0-9]+\.[0-9]+\.[0-9]+)\+sha512\.([0-9a-f]{128})$ ]] ||
  fail "$PACKAGE_JSON: packageManager is '$pin', not pnpm@X.Y.Z+sha512.<128 hex>"
version=${BASH_REMATCH[1]}
pinned=${BASH_REMATCH[2]}

integrity=$(npm view "pnpm@$version" dist.integrity) ||
  fail "could not read the registry's integrity for pnpm@$version"
[[ $integrity == sha512-* ]] ||
  fail "registry integrity for pnpm@$version is '$integrity', not sha512"
recorded=$(node -p "Buffer.from(process.argv[1], 'base64').toString('hex')" "${integrity#sha512-}")
[[ $recorded == "$pinned" ]] ||
  fail "pnpm@$version: registry sha512 $recorded does not match the pin $pinned"

# Word splitting is wanted: PNPM may be a command line such as `npx -y pnpm@X`.
# shellcheck disable=SC2086
running=$($PNPM --version) || fail "could not run '$PNPM --version'"
[[ $running == "$version" ]] ||
  fail "'$PNPM' is pnpm $running; $PACKAGE_JSON pins $version"

echo "pnpm-pin: pnpm@$version matches its sha512 pin"
