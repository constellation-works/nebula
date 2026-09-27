#!/usr/bin/env bash
# scripts/bench.sh — how long `neb` takes on a large corpus.
#
#   scripts/bench.sh [--runs N] <neb> <corpus>
#       Time each of COMMANDS on <corpus> and print one JSON document of
#       command -> median milliseconds.
#   scripts/bench.sh [--runs N] --pair <base-neb> <head-neb> <corpus> <base.json> <head.json>
#       The same for two binaries, interleaved run by run so both see the
#       same machine, each result written to its own file.
#   scripts/bench.sh --compare <base.json> <head.json>
#       Print a table of the two, and fail when any command is both more
#       than 25% and more than 50 ms slower at head.
#
# <corpus> is one the generator wrote (crates/nebula-core/examples/
# synthetic_corpus.rs); its synthetic.json names the nodes `near`, `trace`
# and `impact` are asked about. The corpus is only read: the write, a
# `capture` and then a `promote` of that capture, runs on a fresh copy each
# time, and only the two neb calls are timed. Each command runs once
# untimed, then N times (default 5, at least 5); the median is reported.
#
# Times are wall clock from bash's $EPOCHREALTIME, so bash 5 or later. jq
# reads the manifest and the results. Nothing here builds anything: the
# weekly workflow (.github/workflows/bench.yml) builds the binaries.
set -euo pipefail
export LC_ALL=C

MAX_PERCENT=25
MAX_MS=50
RUNS=5
COMMANDS=(check list review near trace impact graph capture promote)

die() {
  echo "bench: $*" >&2
  exit 1
}

usage() {
  sed -n '3,13p' "$0" | sed 's/^# \{0,1\}//' >&2
  exit 2
}

need_tools() {
  [[ -n "${EPOCHREALTIME:-}" ]] || die "needs bash 5 or later for \$EPOCHREALTIME (this is $BASH_VERSION)"
  command -v jq >/dev/null || die "needs jq"
}

# Nothing from the caller's environment decides what neb reads or whether
# it may write: no root but --root, no Orbit run context, no read-only
# switch, and a HOME of its own so no machine setting applies.
isolate() {
  unset NEBULA_ROOT OBSERVATORY_ROOT NEBULA_READ_ONLY ORBIT_RUN_ID ORBIT_TASK_ID \
    XDG_STATE_HOME VISUAL EDITOR
  export HOME="$WORK/home"
  mkdir -p "$HOME"
}

# manifest_id <corpus> <key>
manifest_id() {
  local id
  id=$(jq -er --arg key "$2" '.[$key]' "$1/synthetic.json") \
    || die "$1/synthetic.json names no \`$2\` node; is this a generated corpus?"
  printf '%s\n' "$id"
}

# run <neb> <corpus> <command>: the one neb call a sample times. The
# promote promotes the entry id the capture before it printed.
run() {
  local neb="$1" corpus="$2"
  case "$3" in
    check) "$neb" --root "$corpus" check ;;
    list) "$neb" --root "$corpus" list --json ;;
    review) "$neb" --root "$corpus" review ;;
    near) "$neb" --root "$corpus" near "$NEAR" ;;
    trace) "$neb" --root "$corpus" trace "$DEEP" ;;
    impact) "$neb" --root "$corpus" impact "$ROOT_ID" ;;
    graph) "$neb" --root "$corpus" graph --json ;;
    capture) "$neb" --root "$corpus" capture --quiet "$CAPTURE" ;;
    promote) "$neb" --root "$corpus" promote --quiet "$(<"$WORK/entry")" ;;
    *) die "no such command: $3" ;;
  esac
}

# sample <neb> <command>: run it once, fail loudly if neb does, and leave
# the wall time in microseconds in $ELAPSED.
sample() {
  local neb="$1" cmd="$2" corpus="$CORPUS" start end status=0
  # The write gets a fresh copy of the corpus, made before the clock starts:
  # a capture before a promote, the promote on the capture just made.
  case "$cmd" in
    capture)
      rm -rf "$WORK/copy"
      cp -R "$CORPUS" "$WORK/copy"
      # The write fsyncs; without this it would also wait out the copy's
      # dirty pages.
      sync
      corpus="$WORK/copy"
      ;;
    promote) corpus="$WORK/copy" ;;
  esac
  start=${EPOCHREALTIME/./}
  run "$neb" "$corpus" "$cmd" >"$WORK/out" 2>"$WORK/err" || status=$?
  end=${EPOCHREALTIME/./}
  if [[ "$status" -ne 0 ]]; then
    cat "$WORK/err" >&2
    die "\`neb $cmd\` failed with exit $status using $neb"
  fi
  if [[ "$cmd" == capture ]]; then
    tr -d '[:space:]' <"$WORK/out" >"$WORK/entry"
    [[ -s "$WORK/entry" ]] || die "\`neb capture --quiet\` printed no entry id using $neb"
  fi
  ELAPSED=$((end - start))
}

# median_ms: the median of whitespace-separated microsecond samples on
# stdin, in milliseconds to one decimal.
median_ms() {
  tr ' ' '\n' | sed '/^$/d' | sort -n | awk '{ a[++n] = $1 }
    END {
      if (n == 0) exit 1
      m = (n % 2) ? a[(n + 1) / 2] : (a[n / 2] + a[n / 2 + 1]) / 2
      printf "%.1f", m / 1000
    }'
}

# bench: time every binary in BINARIES, the i-th one's medians to OUTS[i].
bench() {
  local -A samples=()
  local cmd round i b order
  NEAR=$(manifest_id "$CORPUS" near)
  DEEP=$(manifest_id "$CORPUS" deep)
  ROOT_ID=$(manifest_id "$CORPUS" root)
  CAPTURE="Benchmark capture: sparse memory limits delayed feedback in a quiet archive"
  for cmd in "${COMMANDS[@]}"; do
    [[ "$cmd" == promote ]] && continue # timed beside its capture, below
    echo "bench: $cmd" >&2
    for ((round = 0; round <= RUNS; round++)); do
      # Alternate who goes first, so neither binary always runs on a cache
      # the other just warmed.
      order=("${!BINARIES[@]}")
      if ((round % 2)); then
        order=()
        for ((i = ${#BINARIES[@]} - 1; i >= 0; i--)); do order+=("$i"); done
      fi
      for i in "${order[@]}"; do
        b="${BINARIES[$i]}"
        sample "$b" "$cmd"
        # Round 0 is the warm-up.
        ((round == 0)) || samples["$i|$cmd"]+="$ELAPSED "
        if [[ "$cmd" == capture ]]; then
          sample "$b" promote
          ((round == 0)) || samples["$i|promote"]+="$ELAPSED "
        fi
      done
    done
  done
  for i in "${!BINARIES[@]}"; do
    {
      printf '{'
      local sep=""
      for cmd in "${COMMANDS[@]}"; do
        printf '%s"%s": %s' "$sep" "$cmd" "$(printf '%s' "${samples["$i|$cmd"]}" | median_ms)"
        sep=", "
      done
      printf '}\n'
    } | jq . >"${OUTS[$i]}"
  done
}

# compare <base.json> <head.json>: the table, then the verdict.
compare() {
  local rows failed
  rows=$(jq -nr --slurpfile base "$1" --slurpfile head "$2" \
    --argjson pct "$MAX_PERCENT" --argjson ms "$MAX_MS" '
    $base[0] as $b | $head[0] as $h
    | $h | keys_unsorted[] as $k
    | ($b[$k]) as $old | $h[$k] as $new
    | if $old == null then [$k, "-", $new, "-", "-", "new"]
      else ($new - $old) as $d
        | (if $old > 0 then $d / $old * 100 else 0 end) as $p
        | [$k, $old, $new, ($d * 10 | round / 10), ($p * 10 | round / 10),
           (if $d > $ms and $p > $pct then "REGRESSION" else "ok" end)]
      end
    | @tsv')
  printf '%-10s %10s %10s %10s %8s  %s\n' command "base ms" "head ms" "delta ms" "delta %" verdict
  while IFS=$'\t' read -r k old new d p verdict; do
    printf '%-10s %10s %10s %10s %8s  %s\n' "$k" "$old" "$new" "$d" "$p" "$verdict"
  done <<<"$rows"
  failed=$(awk -F'\t' '$6 == "REGRESSION" { printf "%s%s", sep, $1; sep = ", " }' <<<"$rows")
  if [[ -n "$failed" ]]; then
    die "regression over ${MAX_PERCENT}% and ${MAX_MS} ms at head: $failed"
  fi
  echo "bench: no command is more than ${MAX_PERCENT}% and ${MAX_MS} ms slower at head"
}

MODE=single
while [[ $# -gt 0 ]]; do
  case "$1" in
    --runs)
      [[ $# -ge 2 && "$2" =~ ^[0-9]+$ ]] || usage
      RUNS="$2"
      ((RUNS >= 5)) || die "--runs must be at least 5, not $RUNS"
      shift 2
      ;;
    --pair) MODE=pair; shift ;;
    --compare) MODE=compare; shift ;;
    -h | --help) usage ;;
    --) shift; break ;;
    -*) usage ;;
    *) break ;;
  esac
done

need_tools
case "$MODE" in
  compare)
    [[ $# -eq 2 ]] || usage
    compare "$1" "$2"
    exit 0
    ;;
  single)
    [[ $# -eq 2 ]] || usage
    BINARIES=("$1")
    CORPUS="$2"
    OUTS=(/dev/stdout)
    ;;
  pair)
    [[ $# -eq 5 ]] || usage
    BINARIES=("$1" "$2")
    CORPUS="$3"
    OUTS=("$4" "$5")
    ;;
esac

for b in "${BINARIES[@]}"; do
  [[ -x "$b" ]] || die "not an executable: $b"
done
[[ -f "$CORPUS/synthetic.json" ]] || die "$CORPUS is not a generated corpus (no synthetic.json)"
# Absolute, so a copy made under the work directory and the binaries still
# resolve wherever neb runs.
for i in "${!BINARIES[@]}"; do
  BINARIES[i]=$(cd "$(dirname "${BINARIES[$i]}")" && pwd)/$(basename "${BINARIES[$i]}")
done
CORPUS=$(cd "$CORPUS" && pwd)

WORK=$(mktemp -d "${TMPDIR:-/tmp}/neb-bench.XXXXXX")
trap 'rm -rf "$WORK"' EXIT
isolate
bench
