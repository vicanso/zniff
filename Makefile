# zniff desktop capture app. Run from the repository root.
# The crate lives in app/. app/Cargo.toml is the single source of truth
# for the version number; app/nsh.json mirrors it (see sync-version).

CARGO ?= cargo
MANIFEST := app/Cargo.toml
NSH_JSON := app/nsh.json
CARGO_FLAGS := --manifest-path $(MANIFEST)
VERSION := $(shell sed -n 's/^version = "\([^"]*\)"/\1/p' $(MANIFEST) | head -1)

.PHONY: all help build dev dev-app dev-cargo debug run web web-ui fmt lint test test-app bench check check-wasm \
	build-app build-linux-ebpf package-macos package-macos-universal package-macos-signed release release-cli bundle bundle-signed \
	clean bloat udeps version sync-version version-patch version-minor version-major tag nightly changelog

# --- develop ---------------------------------------------------------------

all: build

help:
	@printf '%s\n' \
		'build                  compile the debug binary' \
		'dev-app | dev | run    run desktop app via nsh $(NSH_VERSION) (`ZNIFF_DEV=1`)' \
		'dev-cargo              bacon: rebuild and open the window directly via cargo' \
		'debug                  dev with RUST_LOG=debug' \
		'web                    shared UI at http://127.0.0.1:8080, capture stays closed' \
		'web-ui                 rebuild the shared page with nsh $(NSH_VERSION)' \
		'fmt                    cargo fmt' \
		'lint                   clippy, warnings denied' \
		'check                  type-check every target (plus wasm32 when installed)' \
		'check-wasm             type-check the shared wasm32 UI library' \
		'test                   run the library and binary tests' \
		'test-app               run fmt/lint/test + nsh check && nsh test in app/' \
		'bench                  run large-session performance benchmarks (10k / 100k flows)' \
		'build-app              build desktop nsh runtime artifacts' \
		'build-linux-ebpf       build Linux aya eBPF helper binary (Linux + nightly + bpf-linker)' \
		'package-macos          package zniff.app via nsh package --target macos' \
		'package-macos-universal  package universal arm64+x86_64 zniff.app via nsh' \
		'package-macos-signed   package zniff.app via nsh and embed/sign system extension' \
		'release                compile the release binary' \
		'release-cli            compile the headless CLI binary (--no-default-features)' \
		'clean                  remove this package from the cargo target directory' \
		'bloat                  release binary size by crate' \
		'udeps                  unused dependencies (nightly)' \
		'' \
		'version-patch | version-minor | version-major' \
		'                       bump app/Cargo.toml, sync app/nsh.json, then prepend CHANGELOG.md' \
		'sync-version           copy the app/Cargo.toml version into app/nsh.json' \
		'tag                    annotate v$(VERSION) (push it yourself)' \
		'nightly                move and push the nightly tag; CI ships pre-release and vicanso/zniff-web:nightly' \
		'changelog              regenerate CHANGELOG.md from conventional commits' \
		'' \
		'Extra arguments go in ARGS, for example: make test ARGS=--offline'

# Standard NSH desktop run in dev mode (`config.dev.toml` / port 8878).
dev-app:
	@test -n "$(NSH_BIN)" || { echo "需要 nsh $(NSH_VERSION) 才能运行桌面应用"; exit 1; }
	@case "$$($(NSH_BIN) --version 2>/dev/null)" in \
		*$(NSH_VERSION)*) ;; \
		*) echo "nsh 必须是 $(NSH_VERSION)，当前是 $$($(NSH_BIN) --version 2>/dev/null || printf '<none>')"; exit 1 ;; \
	esac
	cd app && ZNIFF_DEV=1 $(NSH_BIN) run $(ARGS)

dev: dev-app

# Direct Cargo debug build + launch with bacon keeping console attached.
dev-cargo:
	ZNIFF_DEV=1 bacon --project app run $(if $(ARGS),-- $(ARGS))

debug:
	RUST_LOG=debug $(MAKE) dev

run: dev-app

# Headless operator console. No window, no system-proxy change, and the
# capture port stays closed until the page starts it. The server captures
# only clients that set this process as their HTTP proxy. Open
# http://127.0.0.1:8080. The page is the shared App in app/.nsh/web.
# `make web-ui` rebuilds that directory with nsh $(NSH_VERSION). Extra flags
# go in ARGS, for example make web ARGS="--web-port 8090".
NSH_VERSION := 0.7.0
WEB_DIR := $(CURDIR)/app/.nsh/web
NSH_BIN ?= $(shell \
	if [ -x "$(HOME)/.nsh/v$(NSH_VERSION)/nsh/bin/nsh" ]; then printf '%s' "$(HOME)/.nsh/v$(NSH_VERSION)/nsh/bin/nsh"; \
	elif [ -x "/tmp/nsh-$(NSH_VERSION)/nsh/bin/nsh" ]; then printf '%s' "/tmp/nsh-$(NSH_VERSION)/nsh/bin/nsh"; \
	elif command -v nsh >/dev/null 2>&1; then command -v nsh; \
	fi)

web-ui:
	@test -n "$(NSH_BIN)" || { echo "需要 nsh $(NSH_VERSION) 才能打包共用界面"; exit 1; }
	@case "$$($(NSH_BIN) --version 2>/dev/null)" in \
		*$(NSH_VERSION)*) ;; \
		*) echo "nsh 必须是 $(NSH_VERSION)，当前是 $$($(NSH_BIN) --version 2>/dev/null || printf '<none>')"; exit 1 ;; \
	esac
	@llvm=""; if [ -d /opt/homebrew/opt/llvm/bin ]; then llvm="/opt/homebrew/opt/llvm/bin:"; fi; \
		PATH="$$llvm$$PATH" $(NSH_BIN) package --target web $(CURDIR)/app -o $(CURDIR)/app/.nsh/web-pkg
	rm -rf $(WEB_DIR)
	mv $(CURDIR)/app/.nsh/web-pkg/zniff $(WEB_DIR)
	rm -rf $(CURDIR)/app/.nsh/web-pkg
	sh $(CURDIR)/scripts/postprocess-web.sh $(WEB_DIR) $(CURDIR)/app/assets/fonts

web:
	@if [ ! -f $(WEB_DIR)/index.html ] || find $(CURDIR)/app/src $(CURDIR)/app/assets $(CURDIR)/app/build.rs $(CURDIR)/app/Cargo.toml $(CURDIR)/app/nsh.json -newer $(WEB_DIR)/index.html -print -quit 2>/dev/null | grep -q .; then \
		$(MAKE) web-ui; \
	fi
	ZNIFF_DEV=1 ZNIFF_WEB_ROOT=$(WEB_DIR) $(CARGO) run $(CARGO_FLAGS) -- --web $(ARGS)

build:
	$(CARGO) build $(CARGO_FLAGS) $(ARGS)

# --- quality ---------------------------------------------------------------

fmt:
	$(CARGO) fmt $(CARGO_FLAGS) $(ARGS)

lint:
	$(CARGO) clippy $(CARGO_FLAGS) --all-targets $(ARGS) -- --deny=warnings

check: check-wasm
	$(CARGO) check $(CARGO_FLAGS) --all-targets $(ARGS)

check-wasm:
	@if [ -d "$$(rustc --print target-libdir --target wasm32-unknown-unknown 2>/dev/null)" ]; then \
		RUSTFLAGS="-D warnings" $(CARGO) check $(CARGO_FLAGS) --lib --no-default-features --target wasm32-unknown-unknown $(ARGS); \
	fi

test:
	$(CARGO) test $(CARGO_FLAGS) --workspace $(ARGS)

test-app: fmt lint test
	@test -n "$(NSH_BIN)" || { echo "需要 nsh $(NSH_VERSION) 才能运行 nsh check / nsh test"; exit 1; }
	@case "$$($(NSH_BIN) --version 2>/dev/null)" in \
		*$(NSH_VERSION)*) ;; \
		*) echo "nsh 必须是 $(NSH_VERSION)，当前是 $$($(NSH_BIN) --version 2>/dev/null || printf '<none>')"; exit 1 ;; \
	esac
	cd app && $(NSH_BIN) check && $(NSH_BIN) test

bench:
	$(CARGO) bench $(CARGO_FLAGS) --bench large_session $(ARGS)

# --- release ---------------------------------------------------------------

build-app:
	@test -n "$(NSH_BIN)" || { echo "需要 nsh $(NSH_VERSION) 才能运行 nsh build"; exit 1; }
	@case "$$($(NSH_BIN) --version 2>/dev/null)" in \
		*$(NSH_VERSION)*) ;; \
		*) echo "nsh 必须是 $(NSH_VERSION)，当前是 $$($(NSH_BIN) --version 2>/dev/null || printf '<none>')"; exit 1 ;; \
	esac
	cd app && $(NSH_BIN) build $(ARGS)

build-linux-ebpf:
	scripts/build-linux-ebpf.sh

release:
	@cargo_home="$${CARGO_HOME:-$$HOME/.cargo}"; \
	rustup_home="$${RUSTUP_HOME:-$$HOME/.rustup}"; \
	RUSTFLAGS="--remap-path-prefix=$(CURDIR)=. --remap-path-prefix=$$cargo_home=/cargo --remap-path-prefix=$$rustup_home=/rustup $${RUSTFLAGS:-}" \
	$(CARGO) build $(CARGO_FLAGS) --release $(ARGS)

release-cli:
	@cargo_home="$${CARGO_HOME:-$$HOME/.cargo}"; \
	rustup_home="$${RUSTUP_HOME:-$$HOME/.rustup}"; \
	RUSTFLAGS="--remap-path-prefix=$(CURDIR)=. --remap-path-prefix=$$cargo_home=/cargo --remap-path-prefix=$$rustup_home=/rustup $${RUSTFLAGS:-}" \
	$(CARGO) build $(CARGO_FLAGS) --release --no-default-features $(ARGS)

# Standard NSH macOS packaging (.app under app/.nsh/out/zniff.app).
package-macos:
	@case "$$(uname -s)" in \
	  Darwin) ;; \
	  *) echo "package-macos builds a macOS .app; this machine is $$(uname -s)"; exit 1 ;; \
	esac
	@test -n "$(NSH_BIN)" || { echo "需要 nsh $(NSH_VERSION) 才能打包 macOS 应用"; exit 1; }
	@case "$$($(NSH_BIN) --version 2>/dev/null)" in \
		*$(NSH_VERSION)*) ;; \
		*) echo "nsh 必须是 $(NSH_VERSION)，当前是 $$($(NSH_BIN) --version 2>/dev/null || printf '<none>')"; exit 1 ;; \
	esac
	cd app && $(NSH_BIN) package --target macos $(ARGS)

package-macos-universal:
	@case "$$(uname -s)" in \
	  Darwin) ;; \
	  *) echo "package-macos-universal builds a macOS .app; this machine is $$(uname -s)"; exit 1 ;; \
	esac
	@test -n "$(NSH_BIN)" || { echo "需要 nsh $(NSH_VERSION) 才能打包 macOS 应用"; exit 1; }
	@case "$$($(NSH_BIN) --version 2>/dev/null)" in \
		*$(NSH_VERSION)*) ;; \
		*) echo "nsh 必须是 $(NSH_VERSION)，当前是 $$($(NSH_BIN) --version 2>/dev/null || printf '<none>')"; exit 1 ;; \
	esac
	cd app && $(NSH_BIN) package --target macos --arch universal $(ARGS)

# Signed macOS .app with embedded NETransparentProxyProvider System Extension.
# Pass ARCH=universal to package arm64+x86_64 via nsh before embedding sysext.
package-macos-signed:
	@case "$$(uname -s)" in \
	  Darwin) ;; \
	  *) echo "package-macos-signed builds a macOS .app; this machine is $$(uname -s)"; exit 1 ;; \
	esac
	@if [ "$(ARCH)" = "universal" ]; then \
	  $(MAKE) package-macos-universal; \
	else \
	  $(MAKE) package-macos; \
	fi
	@app_prof="$${ZNIFF_APP_PROFILE:-$$HOME/Downloads/zniff_Developer_ID.provisionprofile}"; \
	sysext_prof="$${ZNIFF_SYSEXT_PROFILE:-$$HOME/Downloads/zniff_Transparent_Proxy___Developer_ID.provisionprofile}"; \
	test -f "$$app_prof" || { echo "Missing ZNIFF_APP_PROFILE: $$app_prof" >&2; exit 1; }; \
	test -f "$$sysext_prof" || { echo "Missing ZNIFF_SYSEXT_PROFILE: $$sysext_prof" >&2; exit 1; }; \
	identity="$${ZNIFF_SIGN_IDENTITY:-$$(python3 -c 'import plistlib,subprocess,hashlib,sys; pl=plistlib.loads(subprocess.check_output(["security","cms","-D","-i",sys.argv[1]], stderr=subprocess.DEVNULL)); certs=pl.get("DeveloperCertificates", []); print(hashlib.sha1(certs[0]).hexdigest().upper() if certs else "")' "$$app_prof")}"; \
	test -n "$$identity" || { echo "Missing ZNIFF_SIGN_IDENTITY and failed to extract certificate SHA-1 from $$app_prof" >&2; exit 1; }; \
	APP_PATH="$(CURDIR)/app/.nsh/out/zniff.app" \
	ZNIFF_APP_PROFILE="$$app_prof" \
	ZNIFF_SYSEXT_PROFILE="$$sysext_prof" \
	ZNIFF_SIGN_IDENTITY="$$identity" \
	.github/workflows/sign_and_package_macos.sh --embed-sysext

bundle: package-macos

bundle-signed: package-macos-signed

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
# app/Cargo.toml is where the number is set; app/nsh.json only mirrors it
# and is rewritten by `sync-version` on every bump.
#
# The tag is not optional bookkeeping: `git cliff --unreleased` bounds its
# range by the latest tag, so a release that never gets tagged makes the
# next changelog repeat everything since the last one under the new number.
# The reverse is guarded too: once v$(VERSION) exists, `version` refuses,
# because re-running it would prepend a second section under the same number.

version:
	@git describe --tags --match 'v[0-9]*' --abbrev=0 >/dev/null 2>&1 || { \
		echo "no release tag exists — git cliff would treat the ENTIRE history as"; \
		echo "unreleased and repeat it under v$(VERSION). Tag the previous release"; \
		echo "first: git tag -a vX.Y.Z <commit>"; exit 1; }
	@! git rev-parse -q --verify "refs/tags/v$(VERSION)" >/dev/null || { \
		echo "v$(VERSION) is already tagged — this would add a second $(VERSION)"; \
		echo "section to CHANGELOG.md. Bump first: make version-patch"; exit 1; }
	git cliff --tag-pattern '^v[0-9]+' --unreleased --tag v$(VERSION) --prepend CHANGELOG.md
	@n=$$(git cliff --tag-pattern '^v[0-9]+' --unreleased --tag v$(VERSION) 2>&1 >/dev/null \
		| grep -o '[0-9]* commit(s) were skipped' | grep -o '^[0-9]*'); \
	if [ -n "$$n" ]; then \
		echo ""; \
		echo "WARNING: $$n commit(s) are missing from the v$(VERSION) section — not"; \
		echo "conventional-commit syntax. This names them:"; \
		echo "  git cliff --tag-pattern '^v[0-9]+' --unreleased -vv 2>&1 | grep 'did not match'"; \
	fi

# Copy the Cargo.toml version into the top-level "version" of app/nsh.json.
# `-i.bak` + rm keeps the in-place edit portable across BSD and GNU sed.
sync-version:
	@test -n "$(VERSION)" || { echo "could not read version from $(MANIFEST)"; exit 1; }
	@sed -i.bak -E 's/^(  "version": *")[^"]*(",?)$$/\1$(VERSION)\2/' $(NSH_JSON) && rm -f $(NSH_JSON).bak
	@grep -q '^  "version": "$(VERSION)"' $(NSH_JSON) || { echo "failed to set version in $(NSH_JSON)"; exit 1; }
	@echo "$(NSH_JSON) version -> $(VERSION)"

# Bump app/Cargo.toml (+ app/Cargo.lock) via cargo-edit, then run
# `sync-version` and `version` in a fresh make invocation — VERSION is
# expanded at parse time, so the recursive $(MAKE) is what picks up the
# just-bumped number.
version-patch:
	$(CARGO) set-version --manifest-path $(MANIFEST) --bump patch
	$(MAKE) sync-version version

version-minor:
	$(CARGO) set-version --manifest-path $(MANIFEST) --bump minor
	$(MAKE) sync-version version

version-major:
	$(CARGO) set-version --manifest-path $(MANIFEST) --bump major
	$(MAKE) sync-version version

tag:
	git tag -a v$(VERSION) -m "version $(VERSION)"
	@echo "created v$(VERSION) — push it with: git push origin v$(VERSION)"

nightly:
	@if [ -n "$$(git status --porcelain)" ]; then \
		echo "working tree is dirty — commit or stash changes before make nightly" >&2; \
		exit 1; \
	fi
	git tag -f nightly HEAD
	@remote_sha=$$(git ls-remote --tags origin refs/tags/nightly | awk '{print $$1}'); \
	head_sha=$$(git rev-parse HEAD); \
	if [ "$$remote_sha" = "$$head_sha" ]; then \
		git push origin :refs/tags/nightly; \
	fi
	git push -f origin refs/tags/nightly
	@echo "triggered nightly build for $$(git rev-parse --short=7 HEAD)"

# Regenerate CHANGELOG.md from conventional commits (git-cliff).
# Run it in the version-bump commit, before tagging.
changelog:
	git cliff --tag-pattern '^v[0-9]+' -o CHANGELOG.md
