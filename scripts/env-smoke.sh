#!/usr/bin/env bash
# Exercise the CLI when git cannot start and when the corpus cannot be written.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
echo 'env-smoke: running build neb'
if ! cargo build --locked -p neb --bin neb; then
    echo 'env-smoke: FAIL: build neb' >&2
    exit 1
fi
neb="${CARGO_TARGET_DIR:-$repo_root/target}/debug/neb"

scratch_base="${ORBIT_SCRATCH_DIR:-${TMPDIR:-/tmp}}"
scratch="$(mktemp -d "$scratch_base/nebula-env-smoke.XXXXXX")"
failed=0
cleanup() {
    chmod -R u+w "$scratch"
    if (( failed == 0 )); then
        rm -r -- "$scratch"
    else
        echo "env-smoke: fixtures and logs kept in $scratch" >&2
    fi
}
trap cleanup EXIT

export HOME="$scratch/home" XDG_CONFIG_HOME="$scratch/home/.config"
export TMPDIR="$scratch"
export LC_ALL=C LANG=C
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
export GIT_AUTHOR_NAME=env-smoke GIT_AUTHOR_EMAIL=env-smoke@example.invalid
export GIT_COMMITTER_NAME=env-smoke GIT_COMMITTER_EMAIL=env-smoke@example.invalid
export GIT_CEILING_DIRECTORIES="$scratch"
unset NEBULA_ROOT OBSERVATORY_ROOT NEBULA_READ_ONLY VISUAL EDITOR
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE
mkdir -p "$HOME" "$scratch/empty-path"

fail() {
    echo "env-smoke: FAIL: $*" >&2
    failed=1
}

run_neb() {
    "$neb" --root "$root" "$@"
}

check_ok() {
    local label="$1"
    shift
    echo "env-smoke: running $label"
    if "$@" >"$scratch/$label.out" 2>&1; then
        echo "env-smoke: PASS: $label"
    else
        fail "$label (exit $?): $(cat "$scratch/$label.out")"
    fi
}

expect_refusal() {
    local label="$1" code="$2" path="$3" status=0
    shift 3
    echo "env-smoke: running $label"
    env PATH="${NEB_SMOKE_PATH:-$PATH}" "$neb" --json --root "$root" "$@" >"$scratch/$label.out" 2>"$scratch/$label.err" || status=$?
    if (( status != 1 )); then
        fail "$label: expected exit 1, got $status; $(cat "$scratch/$label.out" "$scratch/$label.err")"
    elif ! grep -Fq "\"code\":\"$code\"" "$scratch/$label.err"; then
        fail "$label: expected typed $code refusal; $(cat "$scratch/$label.err")"
    elif ! grep -Fq "$path" "$scratch/$label.err"; then
        fail "$label: refusal did not name $path; $(cat "$scratch/$label.err")"
    else
        echo "env-smoke: PASS: $label (exit 1, $code, path named)"
    fi
}

expect_prose_refusal() {
    local label="$1" path="$2" status=0
    shift 2
    echo "env-smoke: running $label"
    run_neb "$@" >"$scratch/$label.out" 2>"$scratch/$label.err" || status=$?
    if (( status != 1 )); then
        fail "$label: expected exit 1, got $status; $(cat "$scratch/$label.out" "$scratch/$label.err")"
    elif ! grep -Fq "$path" "$scratch/$label.err" || ! grep -Fq 'Permission denied' "$scratch/$label.err"; then
        fail "$label: expected a permission refusal naming $path; $(cat "$scratch/$label.err")"
    else
        echo "env-smoke: PASS: $label (exit 1, path named)"
    fi
}

# A private corpus with commits off continues to work without the git binary.
root="$scratch/no-git-corpus"
check_ok no-git-init run_neb init
check_ok no-git-commit-off run_neb config commit off
echo 'env-smoke: running no-git capture, promote, check and show (commit off)'
echo 'env-smoke: running no-git-capture'
entry="$(PATH="$scratch/empty-path" run_neb capture --quiet 'no git capture')" || fail 'no-git capture with commit off'
if [[ -n "$entry" ]]; then
    echo 'env-smoke: running no-git-promote'
    node="$(PATH="$scratch/empty-path" run_neb promote "$entry" --quiet --by env-smoke)" || fail 'no-git promote with commit off'
    check_ok no-git-check env PATH="$scratch/empty-path" "$neb" --json --root "$root" check
    if [[ -n "$node" ]]; then
        check_ok no-git-show env PATH="$scratch/empty-path" "$neb" --json --root "$root" show "$node"
        if ! grep -Fq 'no git capture' "$scratch/no-git-show.out"; then
            fail 'no-git-show: promoted node text was absent'
        fi
    fi
fi

# Give this corpus a real repository so commit-on must try to start git.
check_ok no-git-repo git -C "$root" init -q
check_ok no-git-commit-on run_neb config commit on
NEB_SMOKE_PATH="$scratch/empty-path" expect_refusal no-git-commit git "$root" new 'Survives missing git' --id survives-missing-git --by env-smoke
if [[ ! -f "$root/nodes/survives-missing-git.md" ]] || ! grep -Fq 'Survives missing git' "$root/nodes/survives-missing-git.md"; then
    fail 'no-git-commit: the new node was lost after git failed'
fi

# Create valid targets for all corpus-changing verbs before making the corpus
# read-only. The open entry is intentionally used by both promote and drop.
root="$scratch/read-only-corpus"
check_ok read-only-setup run_neb init
check_ok read-only-new-alpha run_neb new 'Smoke alpha' --id smoke-alpha --by env-smoke
check_ok read-only-new-beta run_neb new 'Smoke beta' --id smoke-beta --by env-smoke
entry="$(run_neb capture --quiet 'read only inbox entry')" || fail 'read-only fixture capture'
snapshot() {
    (cd "$root" && find . -type f -print0 | sort -z | xargs -0 sha256sum)
}
before="$(snapshot)"
chmod -R a-w "$root"
check_ok read-only-check run_neb check
check_ok read-only-show run_neb show smoke-alpha
check_ok read-only-inbox run_neb inbox
check_ok read-only-list run_neb list

expect_refusal read-only-init io_at "$root" init
expect_refusal read-only-migrate io_at "$root" migrate
expect_refusal read-only-capture io_at "$root" capture 'write denied'
expect_refusal read-only-promote io_at "$root" promote "$entry" --by env-smoke
expect_refusal read-only-drop io_at "$root" drop "$entry"
expect_refusal read-only-new io_at "$root" new 'Write denied' --id write-denied --by env-smoke
cat >"$scratch/edit.sh" <<'EDITOR_SCRIPT'
#!/bin/sh
printf '\nmodified by env-smoke\n' >> "$1"
EDITOR_SCRIPT
chmod +x "$scratch/edit.sh"
EDITOR="$scratch/edit.sh" expect_refusal read-only-edit io_at "$root" edit smoke-alpha --by env-smoke
expect_refusal read-only-sharpen io_at "$root" sharpen smoke-alpha --kill 'a falsifier' --by env-smoke
expect_refusal read-only-status io_at "$root" status smoke-alpha abandoned --why 'a reason'
expect_refusal read-only-link io_at "$root" link smoke-alpha derives-from smoke-beta --by env-smoke
expect_refusal read-only-tag io_at "$root" tag smoke-alpha --add smoke
expect_refusal read-only-note io_at "$root" note smoke-alpha 'a note' --by env-smoke
expect_refusal read-only-cite io_at "$root" cite smoke-alpha --kind paper --uri https://example.invalid/paper --note 'a source' --by env-smoke
expect_refusal read-only-handoff io_at "$root" handoff smoke-alpha Q123 --note 'a handoff' --by env-smoke
expect_refusal read-only-config io_at "$root" config commit on
printf 'p\n' >"$scratch/triage-input"
# Triage deliberately has no --json form; its underlying I/O refusal is prose.
expect_prose_refusal read-only-triage "$root" triage --by env-smoke <"$scratch/triage-input"

after="$(snapshot)"
if [[ "$before" != "$after" ]]; then
    fail 'read-only corpus changed: checksum listing before/after differs'
else
    echo 'env-smoke: PASS: read-only corpus checksum listing unchanged'
fi

if (( failed != 0 )); then
    exit 1
fi
echo 'env-smoke: all degraded-host checks passed'
