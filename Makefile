# zniff desktop capture app. Run from the repository root.
# The crate lives in app/. app/Cargo.toml is the single source of truth
# for the version number.

CARGO ?= cargo
MANIFEST := app/Cargo.toml
CARGO_FLAGS := --manifest-path $(MANIFEST)
VERSION := $(shell sed -n 's/^version = "\([^"]*\)"/\1/p' $(MANIFEST) | head -1)

.PHONY: all help build dev debug run fmt lint test check release bundle clean \
	bloat udeps version version-patch version-minor version-major tag changelog

# --- develop ---------------------------------------------------------------

all: build

help:
	@printf '%s\n' \
		'build    compile the debug binary' \
		'dev      bacon: rebuild and open the window, console stays attached' \
		'debug    dev with RUST_LOG=debug' \
		'run      same as dev' \
		'fmt      cargo fmt' \
		'lint     clippy, warnings denied' \
		'check    type-check every target' \
		'test     run the library and binary tests' \
		'release  compile the release binary' \
		'bundle   release binary wrapped as zniff.app with icons/zniff.icns' \
		'clean    remove this package from the cargo target directory' \
		'bloat    release binary size by crate' \
		'udeps    unused dependencies (nightly)' \
		'' \
		'version-patch | version-minor | version-major' \
		'         bump app/Cargo.toml, then prepend CHANGELOG.md' \
		'tag      annotate v$(VERSION) (push it yourself)' \
		'changelog  regenerate CHANGELOG.md from conventional commits' \
		'' \
		'Extra cargo arguments go in ARGS, for example: make test ARGS=--offline'

# Debug build + launch. bacon keeps the console attached, so tracing and
# panics land in the terminal (a release build detaches on Windows).
dev:
	ZNIFF_DEV=1 bacon --project app run $(if $(ARGS),-- $(ARGS))

debug:
	RUST_LOG=debug $(MAKE) dev

run: dev

build:
	$(CARGO) build $(CARGO_FLAGS) $(ARGS)

# --- quality ---------------------------------------------------------------

fmt:
	$(CARGO) fmt $(CARGO_FLAGS) $(ARGS)

lint:
	$(CARGO) clippy $(CARGO_FLAGS) --all-targets $(ARGS) -- --deny=warnings

check:
	$(CARGO) check $(CARGO_FLAGS) --all-targets $(ARGS)

test:
	$(CARGO) test $(CARGO_FLAGS) --workspace $(ARGS)

# --- release ---------------------------------------------------------------

release:
	$(CARGO) build $(CARGO_FLAGS) --release $(ARGS)

# macOS .app. The Dock and Finder icon is icons/zniff.icns.
bundle: release
	@case "$$(uname -s)" in \
	  Darwin) ;; \
	  *) echo "bundle builds a macOS .app; this machine is $$(uname -s)"; exit 1 ;; \
	esac
	@target_dir=$$($(CARGO) metadata $(CARGO_FLAGS) --format-version 1 --no-deps \
	  | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p'); \
	scripts/macos-app.sh "$$target_dir/release/zniff" "$$target_dir/release/zniff.app"

# Where the release binary's size goes, by crate.
bloat:
	$(CARGO) bloat $(CARGO_FLAGS) --release --crates --bin zniff

# Unused dependencies (nightly-only).
udeps:
	$(CARGO) +nightly udeps $(CARGO_FLAGS)

clean:
	$(CARGO) clean $(CARGO_FLAGS) $(ARGS)

# --- version ---------------------------------------------------------------

# Release flow: `make version-patch` (bump + changelog), review the diff,
# commit it as `chore: version X.Y.Z`, then `make tag` and
# `git push origin vX.Y.Z`. Nothing here commits or pushes.
# app/Cargo.toml is the only place the number lives.
#
# The tag is not optional bookkeeping: `git cliff --unreleased` bounds its
# range by the latest tag, so a release that never gets tagged makes the
# next changelog repeat everything since the last one under the new number.
# The reverse is guarded too: once v$(VERSION) exists, `version` refuses,
# because re-running it would prepend a second section under the same number.

version:
	@git describe --tags --abbrev=0 >/dev/null 2>&1 || { \
		echo "no release tag exists — git cliff would treat the ENTIRE history as"; \
		echo "unreleased and repeat it under v$(VERSION). Tag the previous release"; \
		echo "first: git tag -a vX.Y.Z <commit>"; exit 1; }
	@! git rev-parse -q --verify "refs/tags/v$(VERSION)" >/dev/null || { \
		echo "v$(VERSION) is already tagged — this would add a second $(VERSION)"; \
		echo "section to CHANGELOG.md. Bump first: make version-patch"; exit 1; }
	git cliff --unreleased --tag v$(VERSION) --prepend CHANGELOG.md
	@n=$$(git cliff --unreleased --tag v$(VERSION) 2>&1 >/dev/null \
		| grep -o '[0-9]* commit(s) were skipped' | grep -o '^[0-9]*'); \
	if [ -n "$$n" ]; then \
		echo ""; \
		echo "WARNING: $$n commit(s) are missing from the v$(VERSION) section — not"; \
		echo "conventional-commit syntax. This names them:"; \
		echo "  git cliff --unreleased -vv 2>&1 | grep 'did not match'"; \
	fi

# Bump app/Cargo.toml (+ app/Cargo.lock) via cargo-edit, then run `version`
# in a fresh make invocation — VERSION is expanded at parse time, so the
# recursive $(MAKE) is what picks up the just-bumped number.
version-patch:
	$(CARGO) set-version --manifest-path $(MANIFEST) --bump patch
	$(MAKE) version

version-minor:
	$(CARGO) set-version --manifest-path $(MANIFEST) --bump minor
	$(MAKE) version

version-major:
	$(CARGO) set-version --manifest-path $(MANIFEST) --bump major
	$(MAKE) version

tag:
	git tag -a v$(VERSION) -m "version $(VERSION)"
	@echo "created v$(VERSION) — push it with: git push origin v$(VERSION)"

# Regenerate CHANGELOG.md from conventional commits (git-cliff).
# Run it in the version-bump commit, before tagging.
changelog:
	git cliff -o CHANGELOG.md
