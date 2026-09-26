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
# `crates/` only: the desktop shell under apps/ has its own stderr lines.
# Integration tests (`tests/`) are exempt, as is a `//` comment line.
set -euo pipefail
cd "$(dirname "$0")/.."

# The module, as `output.rs` or split into `output/`.
OUTPUT=crates/neb/src/output
STREAMS='io::stdout|io::stderr|println!|eprintln!|print!|eprint!|dbg!'
TERMINAL='is_terminal|IsTerminal|NO_COLOR|CLICOLOR|"TERM"|COLUMNS|terminal_size'
PATTERN="$STREAMS|$TERMINAL"

hits=$(grep -rnE --include='*.rs' --exclude-dir=tests "$PATTERN" crates \
  | grep -vE "^$OUTPUT(\.rs:|/)" \
  | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' || true)

if [[ -n "$hits" ]]; then
  echo "$hits" >&2
  echo "terminal-guard: std streams and terminal state belong to $OUTPUT.rs only; write through its out!/outln!/errln!, and ask it about colour and terminals" >&2
  exit 1
fi
echo "terminal-guard: ok"
