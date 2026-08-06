# wabhoa CLI — task runner: build/test/lint/fmt/clean/install, a cheap `smoke`
# check, and an aggregate `verify` gate. Thin wrappers over cargo so a green
# local run predicts a green CI run.

BIN := wabhoa
CARGO := cargo

.PHONY: all build release test lint fmt fmt-check clean install deps smoke verify dev

all: verify

build:
	$(CARGO) build

release:
	$(CARGO) build --release

test:
	$(CARGO) test --all

lint:
	$(CARGO) clippy --all-targets -- -D warnings

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all -- --check

clean:
	$(CARGO) clean

install:
	$(CARGO) install --path . --force

deps:
	$(CARGO) fetch

# Debug build re-signed with a stable identity so macOS keychain "Always
# Allow" grants survive rebuilds during development.
dev:
	cargo build
	@if [ -x "$$HOME/Dev/cli-common/scripts/dev-sign.sh" ]; then \
		"$$HOME/Dev/cli-common/scripts/dev-sign.sh" target/debug/$(BIN); \
	else echo "cli-common/scripts/dev-sign.sh not found — binary left ad-hoc signed"; fi

# Cheap sanity checks needing no config or network: version, help, and the two
# commands that answer without a session.
smoke: release
	./target/release/$(BIN) --version
	./target/release/$(BIN) --help > /dev/null
	./target/release/$(BIN) info > /dev/null
	./target/release/$(BIN) writes > /dev/null

verify: fmt-check lint test smoke
