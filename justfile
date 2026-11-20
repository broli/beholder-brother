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

# Install rustup targets needed for multi-arch musl builds
setup-targets:
    rustup target add x86_64-unknown-linux-musl i686-unknown-linux-musl aarch64-unknown-linux-musl armv7-unknown-linux-musleabihf

# Sync system Zig lib dir to a user-writable path (needed for armv7 cargo-zigbuild)
sync-zig-lib:
    mkdir -p "$HOME/.local/share/zig-lib"
    cp -au /usr/lib/zig/. "$HOME/.local/share/zig-lib/"

# Build static release binaries for a specific target (requires zig + cargo-zigbuild)
release-target target: sync-zig-lib
    ZIG_LIB_DIR="$HOME/.local/share/zig-lib" cargo zigbuild --release --target {{target}}
    @ls -lh target/{{target}}/release/beholder target/{{target}}/release/brother

# Build static musl release binaries for x86_64, i686, aarch64, and armv7
release-all:
    just release-target x86_64-unknown-linux-musl
    just release-target i686-unknown-linux-musl
    just release-target aarch64-unknown-linux-musl
    just release-target armv7-unknown-linux-musleabihf

# Build and package .tar.gz, .deb, and .rpm artifacts for all 4 targets into dist/
package-all: release-all
    #!/usr/bin/env bash
    set -euo pipefail
    ver=$(cargo metadata --no-deps --format-version 1 | python3 -c "import sys, json; print(json.load(sys.stdin)['packages'][0]['version'])")
    rpm_ver="${ver//-/\~}"
    rm -rf dist && mkdir -p dist
    for t in x86_64-unknown-linux-musl i686-unknown-linux-musl aarch64-unknown-linux-musl armv7-unknown-linux-musleabihf; do
        tar -C "target/$t/release" -czf "dist/beholder-brother-v${ver}-${t}.tar.gz" beholder brother
        cargo deb --no-build --no-strip --target "$t" --output dist/
        cargo generate-rpm --target "$t" --auto-req disabled -s "version = \"${rpm_ver}\"" --output dist/
    done
    (cd dist && sha256sum *.tar.gz *.deb *.rpm > SHA256SUMS)
    ls -lh dist/

# Clean cargo artifacts and /tmp/test/
clean:
    cargo clean
    rm -rf /tmp/test/
