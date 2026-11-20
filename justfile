# Default: list available recipes
default:
    @just --list

# Fast compiler check
check:
    cargo check

# Run daemon (optionally pass CLI flags: `just run -i 1000`)
run *args:
    cargo run -- {{args}}

# Run quick test sampling / and /home into /tmp/test/
run-test-param:
    cargo run -- -m / -m /home/ -i 60 -l /tmp/test/

# Run fmt check, standard clippy, and unit/integration tests
test:
    cargo fmt --check
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test

# Run deep pedantic & nursery clippy scrutiny
pedantic-test:
    cargo clippy --all-targets --all-features -- -W clippy::pedantic -W clippy::nursery

# Auto-format all Rust source files
fmt:
    cargo fmt

# Build optimized release binaries and show sizes
release:
    cargo build --release
    @ls -lh target/release/beholder target/release/brother

# Clean cargo artifacts and /tmp/test/
clean:
    cargo clean
    rm -rf /tmp/test/
