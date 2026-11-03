# zniff desktop capture app. Run from the repository root.

CARGO ?= cargo
MANIFEST := app/Cargo.toml
CARGO_FLAGS := --manifest-path $(MANIFEST)

.PHONY: all help build dev test check release clean

all: build

help:
	@printf '%s\n' \
		'build    compile the debug binary' \
		'dev      build and open the window' \
		'test     run the library and binary tests' \
		'check    type-check without producing a binary' \
		'release  compile the release binary' \
		'clean    remove this package from the cargo target directory' \
		'' \
		'Extra cargo arguments go in ARGS, for example: make test ARGS=--offline'

build:
	$(CARGO) build $(CARGO_FLAGS) $(ARGS)

dev:
	$(CARGO) run $(CARGO_FLAGS) $(ARGS)

test:
	$(CARGO) test $(CARGO_FLAGS) $(ARGS)

check:
	$(CARGO) check $(CARGO_FLAGS) $(ARGS)

release:
	$(CARGO) build $(CARGO_FLAGS) --release $(ARGS)

clean:
	$(CARGO) clean $(CARGO_FLAGS) $(ARGS)
