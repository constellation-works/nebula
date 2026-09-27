#!/usr/bin/env bash
# scripts/check-test-modules.sh — STD-02 §R19: every unit-test file under a
# `src/**/tests/` directory is declared in that directory's `mod.rs`.
# An undeclared file compiles to nothing, so its tests silently never run.
# Scans the workspace crates and the desktop shell.
set -euo pipefail
cd "$(dirname "$0")/.."

fail=0
while IFS= read -r file; do
  dir=$(dirname "$file")
  name=$(basename "$file" .rs)
  [[ "$name" == "mod" ]] && continue
  if [[ ! -f "$dir/mod.rs" ]] || ! grep -qE "^[[:space:]]*(#\[cfg\([^]]+\)\][[:space:]]*)?mod[[:space:]]+${name}[[:space:]]*;" "$dir/mod.rs"; then
    echo "test-modules: $file is not declared in $dir/mod.rs (add \`mod $name;\`)" >&2
    fail=1
  fi
done < <(find crates apps/desktop/src-tauri -path '*/src/*' -path '*/tests/*' -name '*.rs' | sort)

if [[ "$fail" -eq 0 ]]; then
  echo "test-modules: ok"
fi
exit "$fail"
