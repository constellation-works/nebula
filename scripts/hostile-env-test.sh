#!/usr/bin/env bash
# Run the test suites under the environments a developer's shell or a git hook
# can hand them, and prove no test read or wrote the host's state
# (STD-03 §R20, STD-04 §R7):
#
#   home     a hostile HOME: a relative `observatory-root`, XDG_CONFIG_HOME
#            inside it, and a git config whose `core.hooksPath` hook writes a
#            marker whenever any git run reads that config;
#   tmpdir   TMPDIR inside a scratch git repository, so a fixture that
#            discovers the repository above its own temporary root stages
#            into it;
#   gitdir   GIT_DIR and GIT_WORK_TREE exported to a sentinel repository, as
#            inside a git hook or `git rebase --exec`.
#
# Each condition runs `cargo test --locked` over neb and nebula-core, and the
# desktop crate where its system libraries exist, then checks the host side:
# the marker is absent and the settings unchanged, the scratch index is empty,
# the sentinel's HEAD, index and log are as they were. Exits 0 only if every
# suite passed and every check held.
#
# HOSTILE_ENV_DESKTOP=1 or 0 forces the desktop crate in or out.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

CARGO="${CARGO:-cargo}"

# This script's own git calls must not act on whatever repository the caller
# was inside either.
# shellcheck disable=SC2046
unset $(git rev-parse --local-env-vars)

# Pinned before any condition moves HOME, so cargo and rustup still find the
# toolchain and the registry.
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"

packages=(-p neb -p nebula-core)
desktop="${HOSTILE_ENV_DESKTOP:-}"
if [[ -z "$desktop" ]]; then
    case "$(uname -s)" in
        Darwin) desktop=1 ;;
        *) if pkg-config --exists webkit2gtk-4.1 2>/dev/null; then desktop=1; else desktop=0; fi ;;
    esac
fi
if [[ "$desktop" == 1 ]]; then
    packages+=(-p nebula-desktop)
else
    echo "hostile-env: skipping nebula-desktop: webkit2gtk-4.1 is not installed"
fi

scratch="$(mktemp -d "${TMPDIR:-/tmp}/nebula-hostile.XXXXXX")"
failed=0
# Kept when anything failed, so the logs can be read.
trap '[[ "$failed" -ne 0 ]] || rm -rf "$scratch"' EXIT

# A git that ignores the caller's configuration and hooks, for the fixtures
# this script builds.
setup_git() {
    GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null git \
        -c user.name=hostile-env -c user.email=hostile-env@example.invalid \
        -c commit.gpgsign=false -c init.defaultBranch=main "$@"
}

fail() {
    echo "hostile-env: FAIL: $*" >&2
    failed=1
}

# Build once in the caller's environment, so each condition below only runs.
"$CARGO" test --locked "${packages[@]}" --no-run

# run_suites LABEL [VAR=VALUE ...]: run every suite with the given environment
# and print the pass and fail counts summed over all test binaries.
run_suites() {
    local label="$1"
    shift
    local log="$scratch/$label.log"
    local status=0
    env "$@" "$CARGO" test --locked --no-fail-fast "${packages[@]}" >"$log" 2>&1 || status=$?
    local counts
    counts="$(awk '/^test result:/ { p += $4; f += $6 } END { printf "%d passed, %d failed", p, f }' "$log")"
    echo "hostile-env: $label: $counts"
    if [[ "$status" -ne 0 ]]; then
        grep -E '^test .* FAILED$' "$log" >&2 || tail -n 40 "$log" >&2
        echo "hostile-env: $label: full log in $log" >&2
        fail "$label: cargo test exited $status"
    fi
}

# ---------------------------------------------------------------- home --
home="$scratch/home"
marker="$scratch/hook-ran"
mkdir -p "$home/.config/nebula" "$home/.config/git" "$scratch/hooks"
printf 'relative/observatory\n' >"$home/.config/nebula/observatory-root"
for hook in pre-commit prepare-commit-msg commit-msg post-commit post-checkout reference-transaction; do
    printf '#!/bin/sh\necho "%s $PWD" >>"%s"\n' "$hook" "$marker" >"$scratch/hooks/$hook"
    chmod +x "$scratch/hooks/$hook"
done
for config in "$home/.gitconfig" "$home/.config/git/config"; do
    printf '[core]\n\thooksPath = %s\n' "$scratch/hooks" >"$config"
done
settings_before="$(cd "$home/.config/nebula" && ls -A && cat observatory-root)"

run_suites home HOME="$home" XDG_CONFIG_HOME="$home/.config"

if [[ -e "$marker" ]]; then
    fail "home: a test ran git with the host's hooks ($(wc -l <"$marker") runs):"
    sort "$marker" | uniq -c | sort -rn | awk 'NR <= 20' >&2
fi
settings_after="$(cd "$home/.config/nebula" && ls -A && cat observatory-root)"
[[ "$settings_before" == "$settings_after" ]] || fail "home: a test wrote the host's nebula settings"

# -------------------------------------------------------------- tmpdir --
outer="$scratch/outer"
setup_git init -q "$outer"
mkdir "$outer/tmp"

run_suites tmpdir TMPDIR="$outer/tmp"

staged="$(setup_git -C "$outer" ls-files --stage)"
[[ -z "$staged" ]] || fail "tmpdir: a test staged into the repository above its temp root:
$staged"
if setup_git -C "$outer" rev-parse -q --verify HEAD >/dev/null; then
    fail "tmpdir: a test committed into the repository above its temp root"
fi

# -------------------------------------------------------------- gitdir --
sentinel="$scratch/sentinel"
setup_git init -q "$sentinel"
printf 'sentinel\n' >"$sentinel/README"
setup_git -C "$sentinel" add README
setup_git -C "$sentinel" commit -q -m sentinel
sentinel_state() {
    setup_git -C "$sentinel" rev-parse HEAD
    setup_git -C "$sentinel" ls-files --stage
    setup_git -C "$sentinel" log --all --format='%H %s'
    setup_git -C "$sentinel" status --porcelain
}
sentinel_before="$(sentinel_state)"

run_suites gitdir GIT_DIR="$sentinel/.git" GIT_WORK_TREE="$sentinel"

[[ "$sentinel_before" == "$(sentinel_state)" ]] || fail "gitdir: a test changed the sentinel repository"

if [[ "$failed" -ne 0 ]]; then
    echo "hostile-env: failed; logs and fixtures kept in $scratch" >&2
    exit 1
fi
echo "hostile-env: all suites passed and the host was untouched"
