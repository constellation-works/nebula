.PHONY: help build release run dev check test types fmt fmt-check release-check clippy audit tree ci ci-fast install uninstall skill-link clean corpus-check watch desktop-deps desktop-dev desktop desktop-check

# ------------------------------------------------------------
# Config
# ------------------------------------------------------------
CARGO ?= cargo
BINARY := neb
# The library crate; the binary is `neb` and lives beside it under crates/.
CORE := nebula-core
INSTALL_PROFILE ?= release
INSTALL_BIN_DIR ?= $(HOME)/.cargo/bin
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
	@echo "  make types         Regenerate apps/desktop/src/types from nebula-core"
	@echo "  make fmt           Format code"
	@echo "  make fmt-check     Check formatting"
	@echo "  make release-check Verify Cargo/CHANGELOG version lockstep"
	@echo "  make clippy        Lint with clippy (deny warnings)"
	@echo "  make audit         Supply-chain audit (cargo-deny)"
	@echo "  make tree          Print dependency tree"
	@echo "  make ci            Full CI pass (fmt-check + clippy + tests)"
	@echo "  make ci-fast       Pre-handoff gate (fmt-check only; no compile)"
	@echo "  make corpus-check  Run the invariant checker over your corpus"
	@echo "                     (ROOT=/path optional; defaults to \$$NEBULA_ROOT or ~/.nebula)"
	@echo "  make desktop-dev   Run the desktop app with live reload (pnpm tauri dev)"
	@echo "  make desktop       Build the desktop .app, unsigned (pnpm tauri build)"
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
	$(CARGO) build --workspace $(CARGO_PROFILE)

release:
	$(CARGO) build --bin $(BINARY) --release

# ------------------------------------------------------------
# Run
# ------------------------------------------------------------
run:
	$(CARGO) run --bin $(BINARY) -- $(ARGS)

# Direct execution (after build)
dev: build
	$(TARGET_DIR)/$(BINARY) $(ARGS)

# ------------------------------------------------------------
# Quality
# ------------------------------------------------------------
check:
	$(CARGO) check --workspace --all-targets --all-features

test:
	$(CARGO) test --workspace --all-targets

# The TypeScript bindings are generated, never edited: this is the only way
# they change. Destination is TS_RS_EXPORT_DIR in .cargo/config.toml.
types:
	$(CARGO) test -p $(CORE) --features ts --lib

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all -- --check

release-check:
	./scripts/release-check.sh

clippy:
	$(CARGO) clippy --workspace --all-targets --all-features -- -D warnings

# Supply-chain audit: advisories + licenses via cargo-deny.
audit:
	@command -v cargo-deny >/dev/null 2>&1 || { echo "Install cargo-deny via: cargo install cargo-deny --locked"; exit 1; }
	$(CARGO) deny check

# Dependency tree inspection
tree:
	$(CARGO) tree -e features

# Full CI pass. Keep aligned with .github/workflows/ci.yml.
ci: fmt-check release-check clippy test types desktop-check

# Pre-handoff gate for agents: no compile.
ci-fast: fmt-check

# ------------------------------------------------------------
# Corpus
# ------------------------------------------------------------
# The corpus never lives in this repository. ROOT overrides the usual
# $NEBULA_ROOT / ~/.nebula resolution.
corpus-check: build
	$(TARGET_DIR)/$(BINARY) $(if $(ROOT),--root $(ROOT),) check

# ------------------------------------------------------------
# Desktop
# ------------------------------------------------------------
# The lockfile is the contract: a fresh clone gets exactly what CI tests.
desktop-deps:
	$(PNPM) --dir $(DESKTOP) install --frozen-lockfile

desktop-dev: desktop-deps
	$(PNPM) --dir $(DESKTOP) tauri dev

# A local .app under target/release/bundle/macos; not signed or notarised.
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
	$(CARGO) build --bin $(BINARY) $(INSTALL_CARGO_PROFILE)
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
