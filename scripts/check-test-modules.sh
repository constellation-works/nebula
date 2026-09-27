#!/usr/bin/env bash
# scripts/check-test-modules.sh — STD-02 §R19: every unit-test file under a
# `src/**/tests/` directory is declared in that directory's `mod.rs`, and
# every such directory is declared as `mod tests;` by the module it tests
# (`<module>/mod.rs`, or `lib.rs`/`main.rs` at a crate root).
# The same holds for a crate-root integration suite split across a
# directory: `tests/<suite>/main.rs` is the test target, and every other file
# under `tests/<suite>/` is a module it or a module below it declares.
# An undeclared file or directory compiles to nothing, so its tests silently
# never run. Scans the workspace crates and the desktop shell.
set -euo pipefail
script_path=${BASH_SOURCE[0]}
script_dir=${script_path%/*}
[[ "$script_dir" != "$script_path" ]] || script_dir=.
cd "$script_dir/.."

for tool in find sort dirname basename grep; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "test-modules: required inspection tool $tool is unavailable" >&2
    exit 1
  fi
done

declares() { # declares <file> <module name>
  local status
  if grep -qE "^[[:space:]]*(#\[cfg\([^]]+\)\][[:space:]]*)?mod[[:space:]]+${2}[[:space:]]*;" "$1"; then
    return 0
  else
    status=$?
  fi
  if ((status == 1)); then
    return 1
  fi
  echo "test-modules: grep failed while inspecting declarations in $1 (exit $status)" >&2
  fail=1
  return 2
}

fail=0
if files=$(find crates apps/desktop/src-tauri -path '*/src/*' -path '*/tests/*' -name '*.rs' | sort); then
  :
else
  status=$?
  echo "test-modules: failed to enumerate source test modules with find/sort (exit $status)" >&2
  exit 1
fi

while IFS= read -r file; do
  [[ -n "$file" ]] || continue
  dir=$(dirname "$file")
  name=$(basename "$file" .rs)
  if [[ "$name" == "mod" ]]; then
    parent=$(dirname "$dir")
    module=$(basename "$dir")
    declared=0
    declaration_error=0
    for owner in "$parent/mod.rs" "$parent/lib.rs" "$parent/main.rs" "$parent.rs"; do
      if [[ -f "$owner" ]]; then
        if declares "$owner" "$module"; then
          declared=1
        else
          status=$?
          ((status == 1)) || declaration_error=1
        fi
      fi
    done
    if [[ "$declared" -eq 0 && "$declaration_error" -eq 0 ]]; then
      echo "test-modules: $dir is not declared in $parent (add \`#[cfg(test)] mod $module;\`)" >&2
      fail=1
    fi
    continue
  fi
  if [[ ! -f "$dir/mod.rs" ]]; then
    echo "test-modules: $file is not declared in $dir/mod.rs (add \`mod $name;\`)" >&2
    fail=1
  elif declares "$dir/mod.rs" "$name"; then
    :
  else
    status=$?
    if ((status == 1)); then
      echo "test-modules: $file is not declared in $dir/mod.rs (add \`mod $name;\`)" >&2
      fail=1
    fi
  fi
done <<<"$files"

# Integration suites: `<dir>/<name>.rs` is declared by `<dir>/main.rs` at the
# suite's root, or below it by `<dir>/mod.rs` or `<dir>.rs`; `<dir>/mod.rs`
# is declared by the directory above it the same way.
for main in crates/*/tests/*/main.rs apps/desktop/src-tauri/tests/*/main.rs; do
  [[ -f "$main" ]] || continue
  if files=$(find "$(dirname "$main")" -name '*.rs' | sort); then
    :
  else
    status=$?
    echo "test-modules: failed to enumerate integration suite under $(dirname "$main") with find/sort (exit $status)" >&2
    exit 1
  fi
  while IFS= read -r file; do
    [[ -n "$file" ]] || continue
    [[ "$file" == "$main" ]] && continue
    dir=$(dirname "$file")
    name=$(basename "$file" .rs)
    if [[ "$name" == "mod" ]]; then
      name=$(basename "$dir")
      dir=$(dirname "$dir")
    fi
    declared=0
    declaration_error=0
    for owner in "$dir/main.rs" "$dir/mod.rs" "$dir.rs"; do
      if [[ -f "$owner" ]]; then
        if declares "$owner" "$name"; then
          declared=1
        else
          status=$?
          ((status == 1)) || declaration_error=1
        fi
      fi
    done
    if [[ "$declared" -eq 0 && "$declaration_error" -eq 0 ]]; then
      echo "test-modules: $file is not declared by its suite $main (add \`mod $name;\` to the module that owns $dir)" >&2
      fail=1
    fi
  done <<<"$files"
done

if [[ "$fail" -eq 0 ]]; then
  echo "test-modules: ok"
fi
exit "$fail"
