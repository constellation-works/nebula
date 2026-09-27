#!/usr/bin/env bash
# The structural checkers must fail closed when a scan tool or traversal fails.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)
scratch_root="$repo_root/.orbit/tmp"
mkdir -p "$scratch_root"
work="$scratch_root/structural-checks-$$"
mkdir "$work"

cleanup() {
  while IFS= read -r -d '' path; do
    if [[ -d "$path" && ! -L "$path" ]]; then
      rmdir "$path"
    else
      unlink "$path"
    fi
  done < <(find "$work" -depth -mindepth 1 -print0)
  rmdir "$work"
}
trap cleanup EXIT

dependency_fixture="$work/dependency-fixture"
mkdir -p "$dependency_fixture/scripts" "$dependency_fixture/crates/nebula-core/src" \
  "$dependency_fixture/apps/desktop/src-tauri/src"
cp "$repo_root/scripts/check-dependency-direction.sh" "$dependency_fixture/scripts/"

check_dependency_fixture() {
  local label="$1" workspace_dep="$2" crate_dep="$3" expected="$4" output status
  cat >"$dependency_fixture/Cargo.toml" <<EOF
[workspace]
members = ["crates/nebula-core"]
[workspace.dependencies]
$workspace_dep
EOF
  cat >"$dependency_fixture/crates/nebula-core/Cargo.toml" <<EOF
[package]
name = "nebula-core"
[dependencies]
$crate_dep
EOF
  if output=$(/bin/bash "$dependency_fixture/scripts/check-dependency-direction.sh" 2>&1); then
    status=0
  else
    status=$?
  fi
  if [[ "$expected" == success && "$status" -eq 0 ]]; then
    return
  fi
  if [[ "$expected" == banned && "$status" -ne 0 && "$output" == *"must not depend on clap"* ]]; then
    return
  fi
  echo "structural-checks: $label: expected $expected, got exit $status:" >&2
  echo "$output" >&2
  return 1
}

# The real checker must resolve both inline spellings and inherited aliases.
# The compact direct case passed before ORB-13404, bypassing the core ban.
check_dependency_fixture "compact direct alias" "" \
  'alias = {package = "clap", version = "4"}' banned
check_dependency_fixture "spaced direct alias" "" \
  'alias = { package = "clap", version = "4" }' banned
check_dependency_fixture "compact allowed alias" "" \
  'alias = {package = "serde", version = "1"}' success
check_dependency_fixture "spaced allowed alias" "" \
  'alias = { package = "serde", version = "1" }' success
check_dependency_fixture "compact inherited alias" \
  'alias = {package = "clap", version = "4"}' \
  'alias = {workspace=true}' banned
check_dependency_fixture "spaced inherited alias" \
  'alias = { package = "clap", version = "4" }' \
  'alias = { workspace = true }' banned
check_dependency_fixture "compact inherited allowed alias" \
  'alias = {package = "serde", version = "1"}' \
  'alias = {workspace=true}' success

tools=(dirname awk basename find sort wc head sed grep python3 cargo)

make_tool_path() {
  local path="$1" tool source
  mkdir "$path"
  for tool in "${tools[@]}"; do
    source=$(command -v "$tool") || {
      echo "structural-checks: required host tool $tool is unavailable" >&2
      return 1
    }
    ln -s "$source" "$path/$tool"
  done
}

run_success() {
  local label="$1" path="$2" script="$3" output status
  if output=$(PATH="$path" /bin/bash "$repo_root/scripts/$script" 2>&1); then
    return 0
  else
    status=$?
  fi
  echo "structural-checks: $label unexpectedly failed (exit $status):" >&2
  echo "$output" >&2
  return 1
}

run_failure() {
  local label="$1" path="$2" script="$3" expected="$4" output status
  if output=$(PATH="$path" /bin/bash "$repo_root/scripts/$script" 2>&1); then
    echo "structural-checks: $label unexpectedly passed:" >&2
    echo "$output" >&2
    return 1
  else
    status=$?
  fi
  if [[ "$output" != *"$expected"* ]]; then
    echo "structural-checks: $label failed without identifying $expected (exit $status):" >&2
    echo "$output" >&2
    return 1
  fi
}

all_tools="$work/all-tools"
make_tool_path "$all_tools"

# A clean scan exercises legitimate grep no-match results in every entry point.
for script in check-terminal-guard.sh check-dependency-direction.sh check-file-size.sh check-test-modules.sh; do
  run_success "$script clean scan" "$all_tools" "$script"
done

missing_grep="$work/missing-grep"
make_tool_path "$missing_grep"
unlink "$missing_grep/grep"
run_failure "terminal guard with missing grep" "$missing_grep" check-terminal-guard.sh "required inspection tool grep"
run_failure "dependency guard with missing grep" "$missing_grep" check-dependency-direction.sh "required inspection tool grep"
run_failure "test-modules guard with missing grep" "$missing_grep" check-test-modules.sh "required inspection tool grep"

missing_awk="$work/missing-awk"
make_tool_path "$missing_awk"
unlink "$missing_awk/awk"
run_failure "dependency guard with missing awk" "$missing_awk" check-dependency-direction.sh "required inspection tool awk"

awk_error="$work/awk-error"
make_tool_path "$awk_error"
unlink "$awk_error/awk"
real_awk=$(command -v awk)
cat >"$awk_error/awk" <<EOF
#!/bin/sh
case "\$1" in
  *"function without_comment"*)
    echo "awk: simulated dependency inspection error" >&2
    exit 2
    ;;
  *) exec "$real_awk" "\$@" ;;
esac
EOF
chmod +x "$awk_error/awk"
run_failure "dependency guard with awk producer error" "$awk_error" check-dependency-direction.sh "failed to inspect dependencies"

grep_error="$work/grep-error"
make_tool_path "$grep_error"
unlink "$grep_error/grep"
cat >"$grep_error/grep" <<'EOF'
#!/bin/sh
echo "grep: simulated inspection error" >&2
exit 2
EOF
chmod +x "$grep_error/grep"
run_failure "terminal guard with grep error" "$grep_error" check-terminal-guard.sh "grep failed"
run_failure "dependency guard with grep error" "$grep_error" check-dependency-direction.sh "grep failed"
run_failure "test-modules guard with grep error" "$grep_error" check-test-modules.sh "grep failed"

missing_find="$work/missing-find"
make_tool_path "$missing_find"
unlink "$missing_find/find"
run_failure "file-size guard with missing find" "$missing_find" check-file-size.sh "required inspection tool find"
run_failure "test-modules guard with missing find" "$missing_find" check-test-modules.sh "required inspection tool find"

missing_sort="$work/missing-sort"
make_tool_path "$missing_sort"
unlink "$missing_sort/sort"
run_failure "file-size guard with missing sort" "$missing_sort" check-file-size.sh "required inspection tool sort"
run_failure "test-modules guard with missing sort" "$missing_sort" check-test-modules.sh "required inspection tool sort"

missing_python="$work/missing-python"
make_tool_path "$missing_python"
unlink "$missing_python/python3"
run_failure "test-modules guard with missing Python" "$missing_python" check-test-modules.sh "required inspection tool python3"

missing_cargo="$work/missing-cargo"
make_tool_path "$missing_cargo"
unlink "$missing_cargo/cargo"
run_failure "test-modules guard with missing Cargo" "$missing_cargo" check-test-modules.sh "cargo is required"

find_error="$work/find-error"
make_tool_path "$find_error"
unlink "$find_error/find"
cat >"$find_error/find" <<'EOF'
#!/bin/sh
echo "find: cannot read directory crates/simulated-denied-input" >&2
exit 1
EOF
chmod +x "$find_error/find"
run_failure "file-size guard with traversal error" "$find_error" check-file-size.sh "failed to enumerate Rust source files"
run_failure "test-modules guard with traversal error" "$find_error" check-test-modules.sh "failed to enumerate source test modules"

cargo_error="$work/cargo-error"
make_tool_path "$cargo_error"
unlink "$cargo_error/cargo"
cat >"$cargo_error/cargo" <<'EOF'
#!/bin/sh
echo "cargo: simulated metadata failure" >&2
exit 1
EOF
chmod +x "$cargo_error/cargo"
run_failure "test-modules guard with Cargo metadata error" "$cargo_error" check-test-modules.sh "cargo metadata failed"

echo "structural-checks: ok"
