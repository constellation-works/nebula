#!/usr/bin/env bash
# scripts/check-file-size.sh — STD-02 §R18: a source file is split along its
# responsibilities before it grows past 800 lines. Every `.rs` file under the
# workspace crates and the desktop shell is at most 800 lines, or says why it
# is one cohesive item with a `// size: <reason>` line in its first five
# lines. Lists every file over the limit without one and exits 1.
set -euo pipefail
script_path=${BASH_SOURCE[0]}
script_dir=${script_path%/*}
[[ "$script_dir" != "$script_path" ]] || script_dir=.
cd "$script_dir/.."

for tool in find sort wc; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "file-size: required inspection tool $tool is unavailable" >&2
    exit 1
  fi
done

limit=800

fail=0
if files=$(find crates apps/desktop/src-tauri -name target -prune -o -name '*.rs' -type f -print | sort); then
  :
else
  status=$?
  echo "file-size: failed to enumerate Rust source files with find/sort (exit $status)" >&2
  exit 1
fi

while IFS= read -r file; do
  [[ -n "$file" ]] || continue
  if lines=$(wc -l <"$file"); then
    :
  else
    status=$?
    echo "file-size: failed to count lines in $file with wc (exit $status)" >&2
    fail=1
    continue
  fi
  has_size_reason=0
  line_number=0
  while IFS= read -r line || [[ -n "$line" ]]; do
    line_number=$((line_number + 1))
    if [[ "$line" =~ ^[[:space:]]*//\ size:\ [^[:space:]] ]]; then
      has_size_reason=1
      break
    fi
    if ((line_number == 5)); then
      break
    fi
  done <"$file"
  if ((lines > limit && has_size_reason == 0)); then
    echo "file-size: $file has $lines lines, over $limit, and no \`// size: <reason>\` in its first five lines" >&2
    fail=1
  fi
done <<<"$files"

if [[ "$fail" -eq 0 ]]; then
  echo "file-size: ok"
fi
exit "$fail"
