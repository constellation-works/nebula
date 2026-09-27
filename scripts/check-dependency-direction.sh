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
# the [dependencies.<name>] table form. Resolve package renames, including
# inherited workspace dependencies, before checking the crate policy.
declared_deps() {
  awk '
    function without_comment(value, i, char, quoted, escaped) {
      for (i = 1; i <= length(value); i++) {
        char = substr(value, i, 1)
        if (escaped) { escaped = 0; continue }
        if (char == "\\" && quoted) { escaped = 1; continue }
        if (char == "\"") quoted = !quoted
        if (char == "#" && !quoted) return substr(value, 1, i - 1)
      }
      return value
    }
    function package_from(value, name) {
      if (!match(value, /(^|[,[:space:]])package[[:space:]]*=[[:space:]]*"[^"]+"/)) return ""
      name = substr(value, RSTART, RLENGTH)
      sub(/^[^"]*"/, "", name)
      sub(/"$/, "", name)
      return name
    }
    function inherited(value) {
      return value ~ /(^|[,{[:space:]])workspace[[:space:]]*=[[:space:]]*true/
    }
    function add_entry(alias, value, name) {
      name = package_from(value)
      if (source == "workspace") {
        workspace_package[alias] = name == "" ? alias : name
      } else {
        deps[++dep_count] = alias
        dep_package[dep_count] = name
        dep_inherited[dep_count] = inherited(value)
      }
    }
    function finish_table() {
      if (table_alias != "") add_entry(table_alias, table_value)
      table_alias = ""
      table_value = ""
    }
    FNR == 1 {
      finish_table()
      source = FILENAME == ARGV[1] ? "workspace" : "crate"
      section = ""
    }
    /^\[/ {
      finish_table()
      header = $0
      sub(/[[:space:]]*#.*/, "", header)
      section = ""
      if (source == "workspace") {
        if (header ~ /^\[workspace\.dependencies\][[:space:]]*$/) section = "deps"
        if (header ~ /^\[workspace\.dependencies\.[A-Za-z0-9_-]+\][[:space:]]*$/) {
          table_alias = header
          sub(/.*\./, "", table_alias)
          sub(/\].*/, "", table_alias)
        }
      } else {
        if (header ~ /^\[(target\..*\.)?(build-)?dependencies\][[:space:]]*$/) section = "deps"
        if (header ~ /^\[(target\..*\.)?(build-)?dependencies\.[A-Za-z0-9_-]+\][[:space:]]*$/) {
          table_alias = header
          sub(/.*\./, "", table_alias)
          sub(/\].*/, "", table_alias)
        }
      }
      next
    }
    table_alias != "" { table_value = table_value " " without_comment($0); next }
    inline_alias != "" {
      inline_value = inline_value " " without_comment($0)
      if ($0 ~ /}/) {
        add_entry(inline_alias, inline_value)
        inline_alias = ""
        inline_value = ""
      }
      next
    }
    section == "deps" && /^[[:space:]]*[A-Za-z0-9_-]+(\.workspace)?[[:space:]]*=/ {
      alias = $0
      sub(/^[[:space:]]*/, "", alias)
      sub(/[[:space:]]*=.*/, "", alias)
      value = without_comment($0)
      sub(/^[^=]*=/, "", value)
      if (alias ~ /\.workspace$/) {
        sub(/\.workspace$/, "", alias)
        value = "workspace = true " value
      }
      if (value ~ /\{/ && value !~ /}/) {
        inline_alias = alias
        inline_value = value
      } else {
        add_entry(alias, value)
      }
    }
    END {
      finish_table()
      for (i = 1; i <= dep_count; i++) {
        alias = deps[i]
        if (dep_inherited[i]) {
          if (alias in workspace_package) print workspace_package[alias]
          else print "!unresolved-workspace-dependency:" alias
        } else if (dep_package[i] != "") {
          print dep_package[i]
        } else {
          print alias
        }
      }
    }
  ' Cargo.toml "$1"
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
    if [[ "$dep" == !unresolved-workspace-dependency:* ]]; then
      err "$crate inherits an undeclared workspace dependency ${dep#*:} ($manifest)"
    elif matches "$dep" "$internal"; then
      matches "$dep" "$ALLOWED" || err "$crate must not depend on workspace crate $dep ($manifest)"
    elif matches "$dep" "$BANNED"; then
      err "$crate must not depend on $dep ($manifest): a surface crate in the core"
    fi
  done < <(declared_deps "$manifest")
done

# --- 2. grep bans ------------------------------------------------------------
# ban [--everywhere] <path…> -- <extended-regex> <message>: fail if the
# pattern appears in a non-test, non-comment line of the Rust sources under
# the paths. With --everywhere, test modules and comments count too.
ban() {
  local paths=() hits everywhere=0
  if [[ "$1" == "--everywhere" ]]; then
    everywhere=1
    shift
  fi
  while [[ "$1" != "--" ]]; do
    # A missing path would make grep find nothing and the ban pass silently.
    [[ -e "$1" ]] || err "ban path $1 does not exist"
    paths+=("$1")
    shift
  done
  shift
  if [[ "$everywhere" -eq 1 ]]; then
    hits=$(grep -rnHE --include='*.rs' "$1" "${paths[@]}" 2>/dev/null || true)
  else
    hits=$(grep -rnHE --include='*.rs' --exclude-dir=tests "$1" "${paths[@]}" 2>/dev/null \
      | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' || true)
  fi
  if [[ -n "$hits" ]]; then
    echo "$hits" >&2
    err "$2"
  fi
}

# Persisted shapes never fabricate a timestamp on read (STD-02 §R16).
ban crates apps/desktop/src-tauri/src -- 'serde\(default *= *"[A-Za-z_:]*(now|now_utc)"' \
  "a persisted timestamp must not default to the current time"

# Core reads no environment (STD-02 §R3): each surface reads its own once,
# into `Locations`, and hands it down. No exemption, not even a unit test or
# a comment, so the spelling never creeps back as an example.
ban --everywhere crates/nebula-core/src -- 'std::env::|env::var|current_dir|home_dir' \
  "nebula-core must not read the environment or the working directory; take them from Locations"

# Retired paths stay retired (STD-02 §R17). List a path here when you delete a
# module, a script or a crate.
for retired in; do
  [[ -e "$retired" ]] && err "retired path exists again: $retired"
done

if [[ "$fail" -eq 0 ]]; then
  echo "dependency-direction: ok"
fi
exit "$fail"
