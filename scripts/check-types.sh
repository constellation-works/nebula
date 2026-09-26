#!/usr/bin/env bash
# scripts/check-types.sh — apps/desktop/src/types is exactly what nebula-core
# exports (STD-04 §R10, §R11).
#
# Check mode (default): regenerate the bindings into a fresh temporary
# directory and compare the whole directory with the committed one, so an
# added, a removed and a changed file all fail. (`git diff --exit-code` sees
# only tracked files: a stale extra `.ts`, or a new type whose `.ts` was never
# committed, passed it.) An export that wrote nothing fails too, rather than
# comparing empty with empty.
#
# Fix mode (UPDATE=1): replace the committed directory's contents with the
# fresh export, deleting stale files. `make types` and `pnpm gen-types` run it.
#
# The generator is the one `ts_export` test in nebula-core's lib, selected by
# exact name: every root type is listed there, and `export_all` follows their
# dependencies. The per-type `#[ts(export)]` tests are left out on purpose, so
# that an emptied or renamed generator shows up as "generated nothing" instead
# of as a partial set.
set -euo pipefail
cd "$(dirname "$0")/.."
export LC_ALL=C

TYPES_DIR=apps/desktop/src/types
GENERATOR=ts_export::writes_the_typescript_bindings
CARGO=${CARGO:-cargo}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
out="$tmp/out"
mkdir "$out"

# An exported TS_RS_EXPORT_DIR overrides the `.cargo/config.toml` default.
TS_RS_EXPORT_DIR="$out" "$CARGO" test --locked -p nebula-core --features ts --lib \
  -- --exact "$GENERATOR"

# Relative paths of every file under a directory, sorted.
files_under() { (cd "$1" && find . -type f | sed 's|^\./||' | sort); }

files_under "$out" >"$tmp/generated"
if ! grep -q '\.ts$' "$tmp/generated"; then
  echo "types: the export generated nothing into $out; expected the bindings from" >&2
  echo "  nebula-core's $GENERATOR test (cargo test -p nebula-core --features ts --lib)" >&2
  exit 1
fi

if [[ -d "$TYPES_DIR" ]]; then
  files_under "$TYPES_DIR" >"$tmp/committed"
else
  : >"$tmp/committed"
fi
comm -23 "$tmp/committed" "$tmp/generated" >"$tmp/stale"
comm -13 "$tmp/committed" "$tmp/generated" >"$tmp/missing"
: >"$tmp/changed"
while IFS= read -r f; do
  cmp -s "$out/$f" "$TYPES_DIR/$f" || echo "$f" >>"$tmp/changed"
done < <(comm -12 "$tmp/committed" "$tmp/generated")

report() { # report <list file> <label>
  local f
  while IFS= read -r f; do echo "  $2: $TYPES_DIR/$f"; done <"$1"
}

if [[ "${UPDATE:-}" == 1 ]]; then
  mkdir -p "$TYPES_DIR"
  find "$TYPES_DIR" -mindepth 1 -delete
  cp -R "$out/." "$TYPES_DIR/"
  if [[ -s "$tmp/stale" || -s "$tmp/missing" || -s "$tmp/changed" ]]; then
    echo "types: updated $TYPES_DIR"
    report "$tmp/stale" removed
    report "$tmp/missing" added
    report "$tmp/changed" changed
  else
    echo "types: $TYPES_DIR already up to date ($(wc -l <"$tmp/generated" | tr -d ' ') files)"
  fi
  exit 0
fi

if [[ -s "$tmp/stale" || -s "$tmp/missing" || -s "$tmp/changed" ]]; then
  {
    echo "types: $TYPES_DIR does not match what nebula-core exports"
    report "$tmp/stale" "stale (not generated)"
    report "$tmp/missing" "missing (generated, not committed)"
    report "$tmp/changed" changed
    while IFS= read -r f; do
      diff -u "$TYPES_DIR/$f" "$out/$f" || true
    done <"$tmp/changed"
    echo "Regenerate with \`make types\` and commit the result."
  } >&2
  exit 1
fi

echo "types: $TYPES_DIR matches the export ($(wc -l <"$tmp/generated" | tr -d ' ') files)"
