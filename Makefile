.PHONY: build release test lint fmt check conformance clean install release-patch release-minor release-major update-deps

build:
	cargo build --locked

release:
	cargo build --locked --release

test:
	cargo test --locked

lint:
	cargo fmt -- --check
	cargo clippy --locked --all-targets -- -D warnings

fmt:
	cargo fmt

check: lint test

# Score the binary against The CLI Spec (clispec.dev). Requires `clispec`.
conformance: release
	clispec score ./target/release/simplenote-cli auth status

clean:
	cargo clean

install: release
	mkdir -p ~/.local/bin
	cp target/release/simplenote-cli ~/.local/bin/simplenote-cli

update-deps:
	upd --apply --max-bump minor --lang rust,actions

release-patch:
	vership bump patch --prepare

release-minor:
	vership bump minor --prepare

release-major:
	vership bump major --prepare
