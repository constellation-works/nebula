#!/usr/bin/env bash
# scripts/check-dependency-direction.sh — STD-02 §R1–R7, §R16, §R17 for this
# workspace.
#
# From source files alone (no build, no network, no jq), so it runs in
# `make ci-fast` as well as CI:
#
#   1. Crate edges. Every member listed in the root Cargo.toml's
#      `[workspace] members` needs a policy below. A crate may depend only on
#      the workspace crates in its allowlist, and never on the external crates
#      in its banlist. Dev-dependencies are exempt. A member without a policy
#      fails the check, so a new member forces a decision.
#   2. Grep bans over non-test Rust sources.
#
# The layering is written down in docs/design/v0.2/2_architecture.md
# ("Layering, enforced"); change it and this script in the same commit
# (STD-02 §R6).
set -euo pipefail
cd "$(dirname "$0")/.."

fail=0
err() { echo "dependency-direction: $*" >&2; fail=1; }

# --- 1. crate policies -------------------------------------------------------
# policy <crate> sets ALLOWED (workspace crates) and BANNED (external crates,
# shell patterns allowed); it fails for a crate with no policy.
policy() {
  case "$1" in
    nebula-core)
      ALLOWED=""
      # Core is pure with respect to presentation: it never parses arguments,
      # draws on a terminal, installs a log subscriber, or knows about the
      # desktop shell or its file watcher. Those belong to the surfaces.
      BANNED="clap clap_complete shlex tauri* notify tracing-subscriber anstream anstyle crossterm termcolor"
      ;;
    neb)
      ALLOWED="nebula-core"
      BANNED=""
      ;;
    nebula-desktop)
      ALLOWED="nebula-core"
      BANNED=""
      ;;
    *) return 1 ;;
  esac
}

# The member paths in the root Cargo.toml's `[workspace] members` array.
workspace_members() {
  awk '
    /^\[/ { in_ws = ($0 ~ /^\[workspace\][[:space:]]*$/); in_members = 0 }
    in_ws && /^[[:space:]]*members[[:space:]]*=/ { in_members = 1 }
    in_members {
      line = $0
      sub(/#.*/, "", line)
      while (match(line, /"[^"]*"/)) {
        print substr(line, RSTART + 1, RLENGTH - 2)
        line = substr(line, RSTART + RLENGTH)
      }
      if ($0 ~ /\]/) in_members = 0
    }
  ' Cargo.toml
}

package_name() {
  awk -F'"' '
    /^\[/ { in_pkg = ($0 ~ /^\[package\][[:space:]]*$/); next }
    in_pkg && /^name[[:space:]]*=/ { print $2; exit }
  ' "$1"
}

# The dependency names declared in a Cargo.toml's non-dev tables:
# [dependencies], [build-dependencies] and their target-specific forms, plus
# the [dependencies.<name>] table form. A `package = "…"` rename reports the
# real package.
declared_deps() {
  awk '
    /^\[/ {
      in_deps = ($0 ~ /^\[(target\..*\.)?(build-)?dependencies\][[:space:]]*$/)
      if (match($0, /^\[(target\..*\.)?(build-)?dependencies\.[A-Za-z0-9_-]+\]/)) {
        name = substr($0, RSTART, RLENGTH - 1)
        sub(/.*\./, "", name)
        print name
      }
      next
    }
    in_deps && /^[A-Za-z0-9_-]+[[:space:]]*(\.workspace)?[[:space:]]*=/ {
      name = $1; sub(/\.workspace$/, "", name); sub(/=.*/, "", name)
      gsub(/[[:space:]]/, "", name)
      if (match($0, /package[[:space:]]*=[[:space:]]*"[^"]+"/)) {
        name = substr($0, RSTART, RLENGTH)
        sub(/^[^"]*"/, "", name); sub(/"$/, "", name)
      }
      print name
    }
  ' "$1"
}

# matches <word> <space-separated patterns>
matches() {
  local word="$1" pattern
  for pattern in $2; do
    # shellcheck disable=SC2053 # the pattern is a glob on purpose
    [[ "$word" == $pattern ]] && return 0
  done
  return 1
}

members=""
internal=""
while IFS= read -r member; do
  manifest="$member/Cargo.toml"
  if [[ ! -f "$manifest" ]]; then
    err "workspace member '$member' has no $manifest"
    continue
  fi
  members="$members $manifest"
  internal="$internal $(package_name "$manifest")"
done < <(workspace_members)

if [[ -z "$members" ]]; then
  err "found no workspace members in Cargo.toml"
fi

for manifest in $members; do
  crate=$(package_name "$manifest")
  if ! policy "$crate"; then
    err "crate '$crate' ($manifest) has no policy; add one to scripts/check-dependency-direction.sh and to the layering in docs/design/v0.2/2_architecture.md"
    continue
  fi
  while IFS= read -r dep; do
    if matches "$dep" "$internal"; then
      matches "$dep" "$ALLOWED" || err "$crate must not depend on workspace crate $dep ($manifest)"
    elif matches "$dep" "$BANNED"; then
      err "$crate must not depend on $dep ($manifest): a surface crate in the core"
    fi
  done < <(declared_deps "$manifest")
done

# --- 2. grep bans ------------------------------------------------------------
# ban <path…> -- <extended-regex> <message>: fail if the pattern appears in a
# non-test, non-comment line of the Rust sources under the paths.
ban() {
  local paths=() hits
  while [[ "$1" != "--" ]]; do
    # A missing path would make grep find nothing and the ban pass silently.
    [[ -e "$1" ]] || err "ban path $1 does not exist"
    paths+=("$1")
    shift
  done
  shift
  hits=$(grep -rnHE --include='*.rs' --exclude-dir=tests "$1" "${paths[@]}" 2>/dev/null \
    | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' || true)
  if [[ -n "$hits" ]]; then
    echo "$hits" >&2
    err "$2"
  fi
}

# Persisted shapes never fabricate a timestamp on read (STD-02 §R16).
ban crates apps/desktop/src-tauri/src -- 'serde\(default *= *"[A-Za-z_:]*(now|now_utc)"' \
  "a persisted timestamp must not default to the current time"

# The core-reads-no-environment ban (STD-02 §R3: `std::env::` and
# `current_dir` in crates/nebula-core/src) joins this list with the change
# that moves root discovery out of store.rs.

# Retired paths stay retired (STD-02 §R17). List a path here when you delete a
# module, a script or a crate.
for retired in; do
  [[ -e "$retired" ]] && err "retired path exists again: $retired"
done

if [[ "$fail" -eq 0 ]]; then
  echo "dependency-direction: ok"
fi
exit "$fail"
