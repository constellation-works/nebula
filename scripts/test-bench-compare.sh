#!/usr/bin/env bash
# Exercise the public comparison entry point with complete and corrupt results.
set -euo pipefail

work=$(mktemp -d "${ORBIT_SCRATCH_DIR:-${TMPDIR:-/tmp}}/neb-bench-compare.XXXXXX")
trap 'rm -r "$work"' EXIT
bench=$(cd "$(dirname "$0")" && pwd)/bench.sh

jq -n '{check: 100, list: 100, review: 100, near: 100, trace: 100,
  impact: 100, graph: 100, capture: 100, promote: 100}' >"$work/valid.json"

reject() {
  local base="$1" head="$2" invalid="$3" reason="$4"
  if bash "$bench" --compare "$base" "$head" >"$work/out" 2>"$work/err"; then
    echo "accepted invalid benchmark input: $invalid ($reason)" >&2
    exit 1
  fi
  if ! grep -Fq -- "$invalid" "$work/err" || ! grep -Fq -- "$reason" "$work/err"; then
    echo "comparison did not identify $invalid ($reason):" >&2
    cat "$work/err" >&2
    exit 1
  fi
}

accept() {
  bash "$bench" --compare "$1" "$2" >"$work/out" 2>"$work/err"
  grep -Fq 'no command is more than' "$work/out"
  [[ $(grep -c '  ok$' "$work/out") -eq 9 ]]
}

regress() {
  if bash "$bench" --compare "$1" "$2" >"$work/out" 2>"$work/err"; then
    echo "accepted a benchmark regression in $2" >&2
    exit 1
  fi
  grep -Fq 'regression over 25% and 50 ms at head: check' "$work/err"
}

: >"$work/empty.json"
reject "$work/empty.json" "$work/valid.json" "$work/empty.json" 'JSON'
reject "$work/valid.json" "$work/empty.json" "$work/empty.json" 'JSON'
printf '{}\n' >"$work/object.json"
reject "$work/object.json" "$work/object.json" "$work/object.json" 'no comparable'
printf '{"check": 1}\n' >"$work/only-check.json"
printf '{"list": 1}\n' >"$work/only-list.json"
reject "$work/only-check.json" "$work/only-list.json" "$work/only-check.json" 'no comparable'

jq 'del(.check)' "$work/valid.json" >"$work/missing.json"
reject "$work/missing.json" "$work/valid.json" "$work/missing.json" 'check'
reject "$work/valid.json" "$work/missing.json" "$work/missing.json" 'check'
jq '.check = "fast"' "$work/valid.json" >"$work/text.json"
reject "$work/text.json" "$work/valid.json" "$work/text.json" 'check'
reject "$work/valid.json" "$work/text.json" "$work/text.json" 'check'
jq '.check = -1' "$work/valid.json" >"$work/negative.json"
reject "$work/negative.json" "$work/valid.json" "$work/negative.json" 'check'
reject "$work/valid.json" "$work/negative.json" "$work/negative.json" 'check'

accept "$work/valid.json" "$work/valid.json"
jq '.check = 150' "$work/valid.json" >"$work/threshold.json"
accept "$work/valid.json" "$work/threshold.json"
jq '.check = 151' "$work/valid.json" >"$work/regression.json"
regress "$work/valid.json" "$work/regression.json"
jq '.check = 1000' "$work/valid.json" >"$work/large-base.json"
jq '.check = 1060' "$work/valid.json" >"$work/small-percent.json"
accept "$work/large-base.json" "$work/small-percent.json"
jq '.check = 204' "$work/valid.json" >"$work/percent-base.json"
jq '.check = 255.1' "$work/valid.json" >"$work/near-percent.json"
regress "$work/percent-base.json" "$work/near-percent.json"

jq '.check = 0' "$work/valid.json" >"$work/zero.json"
accept "$work/zero.json" "$work/zero.json"
jq '.check = 50' "$work/valid.json" >"$work/from-zero-ok.json"
accept "$work/zero.json" "$work/from-zero-ok.json"
jq '.check = 51' "$work/valid.json" >"$work/from-zero-regression.json"
regress "$work/zero.json" "$work/from-zero-regression.json"

echo 'bench comparison tests passed'
