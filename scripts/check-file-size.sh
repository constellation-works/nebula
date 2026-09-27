#!/usr/bin/env bash
# scripts/check-file-size.sh — STD-02 §R18: a source file is split along its
# responsibilities before it grows past 800 lines. Every `.rs` file under the
# workspace crates and the desktop shell is at most 800 lines, or says why it
# is one cohesive item with a `// size: <reason>` line in its first five
# lines. Lists every file over the limit without one and exits 1.
set -euo pipefail
cd "$(dirname "$0")/.."

limit=800

fail=0
while IFS= read -r file; do
  lines=$(wc -l <"$file")
  if ((lines > limit)) && ! head -n 5 "$file" | grep -qE '^[[:space:]]*// size: [^[:space:]]'; then
    echo "file-size: $file has $lines lines, over $limit, and no \`// size: <reason>\` in its first five lines" >&2
    fail=1
  fi
done < <(find crates apps/desktop/src-tauri -name target -prune -o -name '*.rs' -type f -print | sort)

if [[ "$fail" -eq 0 ]]; then
  echo "file-size: ok"
fi
exit "$fail"
