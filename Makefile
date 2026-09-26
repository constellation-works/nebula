.PHONY: help build release run dev check test doctest hostile-env-test types types-check fmt fmt-check release-check standards-check terminal-guard dependency-direction clippy ci-lint audit tree ci ci-fast install uninstall skill-link clean corpus-check watch desktop-deps desktop-dev desktop desktop-check

# ------------------------------------------------------------
# Config
# ------------------------------------------------------------
CARGO ?= cargo
BINARY := neb
INSTALL_PROFILE ?= release
INSTALL_BIN_DIR ?= $(HOME)/.cargo/bin
# Every cargo call that resolves dependencies uses Cargo.lock as written and
# fails if it is stale, rather than rewriting it (STD-05 §R23, STD-04 §R11).
# `fmt`, `clean`, `deny` and `watch` resolve nothing and go without.
LOCKED := --locked
# The Tauri app. pnpm owns its frontend; cargo builds its shell as a
# workspace member, so `make build`/`clippy`/`test` cover it too.
DESKTOP := apps/desktop
PNPM ?= pnpm

# Detect profile
PROFILE ?= debug
ifeq ($(PROFILE),release)
	CARGO_PROFILE := --release
	TARGET_DIR := target/release
else
	CARGO_PROFILE :=
	TARGET_DIR := target/debug
endif

ifeq ($(INSTALL_PROFILE),release)
	INSTALL_CARGO_PROFILE := --release
	INSTALL_TARGET_DIR := target/release
else
	INSTALL_CARGO_PROFILE :=
	INSTALL_TARGET_DIR := target/debug
endif

# ------------------------------------------------------------
# Help
# ------------------------------------------------------------
help:
	@echo "Nebula Make Targets"
	@echo ""
	@echo "  make build         Build (PROFILE=release optional)"
	@echo "  make release       Build optimized release binary"
	@echo "  make run ARGS=...  Run the CLI through cargo"
	@echo "  make dev ARGS=...  Run the built binary directly"
	@echo "  make check         Type-check every crate"
	@echo "  make test          Run all tests, every crate"
	@echo "  make hostile-env-test  Run the suites under a hostile HOME, TMPDIR and GIT_DIR"
	@echo "  make doctest       Run the documentation examples, every crate"
	@echo "  make types         Regenerate apps/desktop/src/types from nebula-core"
	@echo "  make types-check   Fail if apps/desktop/src/types differs from a fresh export"
	@echo "  make fmt           Format code"
	@echo "  make fmt-check     Check formatting"
	@echo "  make release-check Verify Cargo/CHANGELOG version lockstep"
	@echo "  make standards-check Verify the vendored constellation standards are unedited"
	@echo "  make terminal-guard Verify only the CLI's output layer names stdout/stderr"
	@echo "  make dependency-direction Check crate dependency direction (manifests only)"
	@echo "  make clippy        Lint with clippy (deny warnings)"
	@echo "  make ci-lint       Run the clippy CI gate"
	@echo "  make audit         Supply-chain audit (cargo-deny; the desktop's pnpm pin and"
	@echo "                     pnpm audit)"
	@echo "  make tree          Print dependency tree"
	@echo "  make ci            Full CI pass (ci-fast, tests, doctest, types-check,"
	@echo "                     audit, desktop-check)"
	@echo "  make ci-fast       Pre-handoff gate (fmt-check, release-check, standards-check,"
	@echo "                     terminal-guard, dependency-direction, clippy)"
	@echo "  make corpus-check  Run the invariant checker over your corpus (ROOT=/path"
	@echo "                     optional; else as neb resolves it: \$$NEBULA_ROOT, the nearest"
	@echo "                     corpus at or above the current directory,"
	@echo "                     ~/.config/nebula/root, ~/.nebula)"
	@echo "  make desktop-dev   Run the desktop app with live reload (pnpm tauri dev)"
	@echo "  make desktop       Build the unsigned desktop .app and .dmg on macOS"
	@echo "  make desktop-check Type-check and unit-test the desktop frontend"
	@echo "  make install       Install the binary (INSTALL_PROFILE=debug optional)"
	@echo "  make uninstall     Remove the installed binary"
	@echo "  make skill-link    Symlink skills/nebula into ~/.claude/skills/nebula"
	@echo "  make clean         Clean build artifacts"
	@echo "  make watch         Continuous check + test"

# ------------------------------------------------------------
# Build
# ------------------------------------------------------------
build:
	$(CARGO) build $(LOCKED) --workspace $(CARGO_PROFILE)

release:
	$(CARGO) build $(LOCKED) --bin $(BINARY) --release

# ------------------------------------------------------------
# Run
# ------------------------------------------------------------
run:
	$(CARGO) run $(LOCKED) --bin $(BINARY) -- $(ARGS)

# Direct execution (after build)
dev: build
	$(TARGET_DIR)/$(BINARY) $(ARGS)

# ------------------------------------------------------------
# Quality
# ------------------------------------------------------------
check:
	$(CARGO) check $(LOCKED) --workspace --all-targets --all-features

test:
	$(CARGO) test $(LOCKED) --workspace --all-targets

# `--all-targets` leaves doctests out, so the examples in doc comments run here.
doctest:
	$(CARGO) test $(LOCKED) --workspace --doc

# The suites under a hostile HOME, TMPDIR inside a git repository, and
# GIT_DIR exported, checking that no test touched the host.
hostile-env-test:
	CARGO="$(CARGO)" ./scripts/hostile-env-test.sh

# The TypeScript bindings are generated, never edited: this is the only way
# they change. The script exports into a fresh directory and replaces the
# contents of apps/desktop/src/types, deleting stale files.
types:
	UPDATE=1 CARGO=$(CARGO) ./scripts/check-types.sh

# The same export, compared with the committed directory: an added, removed
# or changed file fails, and so does an export that wrote nothing.
types-check:
	CARGO=$(CARGO) ./scripts/check-types.sh

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all -- --check

release-check:
	./scripts/release-check.sh

# The vendored constellation standards are read-only; this fails on any edit.
standards-check:
	sh docs/standards/check.sh

# Only crates/neb/src/output.rs writes to or names stdout/stderr (STD-02 §R15).
terminal-guard:
	./scripts/check-terminal-guard.sh

# Crate edges and grep bans from the manifests and sources alone; no build.
dependency-direction:
	./scripts/check-dependency-direction.sh

clippy:
	$(CARGO) clippy $(LOCKED) --workspace --all-targets --all-features -- -D warnings

ci-lint: clippy

# Supply-chain audit (STD-05 §R23, §R24): advisories, licenses and sources
# via cargo-deny; the desktop's pnpm against its sha512 pin; advisories in its
# npm tree, with the dated ignores in apps/desktop/pnpm-workspace.yaml. The
# same checks as CI's `deny` and `desktop` jobs.
audit:
	@command -v cargo-deny >/dev/null 2>&1 || { echo "Install cargo-deny via: cargo install cargo-deny --locked"; exit 1; }
	$(CARGO) deny check
	PNPM="$(PNPM)" ./scripts/check-pnpm-pin.sh
	$(PNPM) --dir $(DESKTOP) audit --audit-level moderate

# Dependency tree inspection
tree:
	$(CARGO) tree $(LOCKED) -e features

# Full CI pass: every check .github/workflows/ci.yml runs, across its jobs.
# Keep the two aligned. `audit` fetches the advisory database, so it needs the
# network, and it fails when cargo-deny or pnpm is not installed.
ci: ci-fast test doctest types-check audit desktop-check

# Pre-handoff gate: every cheap check CI runs, plus clippy (STD-02 §R22).
# `test`, `doctest` and `types-check` each need a full build of their own (the
# last with nebula-core's `ts` feature), so they stay in `ci` and CI
# (STD-04@1 §R12).
ci-fast: fmt-check release-check standards-check terminal-guard dependency-direction clippy

# ------------------------------------------------------------
# Corpus
# ------------------------------------------------------------
# The corpus never lives in this repository. ROOT is passed as `--root`;
# without it, `neb` resolves the corpus as usual: $NEBULA_ROOT, else the
# nearest corpus at or above the current directory, else
# ~/.config/nebula/root, else ~/.nebula.
corpus-check: build
	$(TARGET_DIR)/$(BINARY) $(if $(ROOT),--root $(ROOT),) check

# ------------------------------------------------------------
# Desktop
# ------------------------------------------------------------
# The lockfile is the contract: a fresh clone gets exactly what CI tests.
desktop-deps:
	$(PNPM) --dir $(DESKTOP) install --frozen-lockfile

desktop-dev: desktop-deps
	$(PNPM) --dir $(DESKTOP) tauri dev --config src-tauri/tauri.conf.dev.json

# Local .app and .dmg bundles under target/release/bundle/macos; not signed or notarised.
desktop: desktop-deps
	$(PNPM) --dir $(DESKTOP) tauri build

# `exec`, because `pnpm --dir <path> <bin>` only resolves scripts, not bins.
desktop-check: desktop-deps
	$(PNPM) --dir $(DESKTOP) exec tsc --noEmit
	$(PNPM) --dir $(DESKTOP) exec vitest run

# ------------------------------------------------------------
# Install
# ------------------------------------------------------------
install:
	$(CARGO) build $(LOCKED) --bin $(BINARY) $(INSTALL_CARGO_PROFILE)
	install -d $(INSTALL_BIN_DIR)
	install -m 755 $(INSTALL_TARGET_DIR)/$(BINARY) $(INSTALL_BIN_DIR)/$(BINARY)

uninstall:
	rm -f $(INSTALL_BIN_DIR)/$(BINARY)

# The agent skill, linked rather than copied so edits here are live.
SKILL_DIR ?= $(HOME)/.claude/skills
skill-link:
	install -d $(SKILL_DIR)
	ln -sfn $(CURDIR)/skills/nebula $(SKILL_DIR)/nebula
	@echo "linked $(SKILL_DIR)/nebula -> $(CURDIR)/skills/nebula"

# ------------------------------------------------------------
# Clean
# ------------------------------------------------------------
clean:
	$(CARGO) clean

# ------------------------------------------------------------
# Dev Loop
# ------------------------------------------------------------
watch:
	$(CARGO) watch -x "check" -x "test"
