#!/usr/bin/env bash
# scripts/check-terminal-guard.sh — STD-02 §R15, the stream grep guard (and
# STD-01 §R13, which the output layer exists to keep).
#
# Only the CLI's output layer may write to stdout or stderr or name either
# stream. Anything else doing so bypasses the layer that keeps a closed
# stdout from panicking or losing a write's commit. Clippy's print lints do
# not see `writeln!(io::stdout(), …)`, so the stream names are banned too.
#
# The same module is the one colour and terminal gate (STD-01 §R17), so
# asking whether a stream is a terminal, or reading `NO_COLOR`,
# `CLICOLOR[_FORCE]`, `TERM`, `COLUMNS` or the terminal size, is banned
# everywhere else too.
#
# `crates/` only. The desktop shell under apps/ logs through `tracing`, whose
# one subscriber (`run()` in apps/desktop/src-tauri/src/lib.rs) names stderr.
# Integration tests (`tests/`) are exempt, as is a `//` comment line.
set -euo pipefail
script_path=${BASH_SOURCE[0]}
script_dir=${script_path%/*}
[[ "$script_dir" != "$script_path" ]] || script_dir=.
cd "$script_dir/.."

if ! command -v grep >/dev/null 2>&1; then
  echo "terminal-guard: required inspection tool grep is unavailable" >&2
  exit 1
fi

# The module, as `output.rs` or split into `output/`.
OUTPUT=crates/neb/src/output
STREAMS='io::stdout|io::stderr|println!|eprintln!|print!|eprint!|dbg!'
TERMINAL='is_terminal|IsTerminal|NO_COLOR|CLICOLOR|"TERM"|COLUMNS|terminal_size'
PATTERN="$STREAMS|$TERMINAL"

grep_capture() {
  local destination="$1" context="$2" captured status
  shift 2
  if captured=$(grep "$@"); then
    :
  else
    status=$?
    if ((status == 1)); then
      captured=""
    else
      echo "terminal-guard: grep failed while $context (exit $status)" >&2
      return 2
    fi
  fi
  printf -v "$destination" '%s' "$captured"
}

if ! grep_capture hits "scanning Rust sources under crates" -rnE --include='*.rs' --exclude-dir=tests "$PATTERN" crates; then
  exit 1
fi
if [[ -n "$hits" ]]; then
  if ! grep_capture hits "filtering the output module" -vE "^$OUTPUT(\.rs:|/)" <<<"$hits"; then
    exit 1
  fi
fi
if [[ -n "$hits" ]]; then
  if ! grep_capture hits "filtering comment lines" -vE '^[^:]+:[0-9]+:[[:space:]]*//' <<<"$hits"; then
    exit 1
  fi
fi

if [[ -n "$hits" ]]; then
  echo "$hits" >&2
  echo "terminal-guard: std streams and terminal state belong to $OUTPUT.rs only; write through its out!/outln!/errln!, and ask it about colour and terminals" >&2
  exit 1
fi
echo "terminal-guard: ok"
