.PHONY: help check run run-test-param test pedantic-test fmt release clean

help:
	@echo "Available targets:"
	@echo "  make check           - Fast compiler check"
	@echo "  make run             - Run daemon with default flags"
	@echo "  make run-test-param  - Run quick test sampling / and /home into /tmp/test/"
	@echo "  make test            - Run unit tests, fmt check, and standard clippy"
	@echo "  make pedantic-test   - Run deep pedantic & nursery clippy scrutiny"
	@echo "  make fmt             - Auto-format all Rust source files"
	@echo "  make release         - Build optimized release binary and show size"
	@echo "  make clean           - Clean cargo artifacts and /tmp/test/"

check:
	cargo check

run:
	cargo run

run-test-param:
	cargo run -- -m / -m /home/ -i 60 -l /tmp/test/

test:
	cargo fmt --check
	cargo clippy --all-targets --all-features -- -D warnings
	cargo test

pedantic-test:
	cargo clippy --all-targets --all-features -- -W clippy::pedantic -W clippy::nursery

fmt:
	cargo fmt

release:
	cargo build --release
	@ls -lh target/release/beholder-brother

clean:
	cargo clean
	rm -rf /tmp/test/

