# muster task runner. Run `just` to list recipes.

# Path to the installed nightly rustfmt (unstable import options require nightly).
nightly_rustfmt := `ls ~/.rustup/toolchains/nightly*/bin/rustfmt 2>/dev/null | head -1`

# List available recipes.
default:
    @just --list

# Type-check every crate.
check:
    cargo check --workspace --all-targets

# Build every crate.
build:
    cargo build --workspace

# Run the CLI. Pass args after `--`, e.g. `just run -- ps` or `just run -- up`.
run *ARGS:
    cargo run --bin muster -- {{ARGS}}

# Run the test suite.
test:
    cargo test --workspace --all-features

# Format with nightly rustfmt (honors rustfmt.toml; no global config change).
fmt:
    RUSTFMT="{{nightly_rustfmt}}" cargo fmt --all

# Verify formatting without writing changes.
fmt-check:
    RUSTFMT="{{nightly_rustfmt}}" cargo fmt --all --check

# Lint; warnings are errors.
lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Lint each crate alone, so none builds only through workspace feature unification.
lint-crates:
    #!/usr/bin/env bash
    set -euo pipefail
    for crate in $(cargo metadata --no-deps --format-version 1 | jq -r '.packages[].name'); do
        cargo clippy -p "$crate" --all-targets -- -D warnings
    done

# Package every crate as crates.io would and build the packages.
package:
    cargo package --workspace --locked

# Set the release version of every crate and its sibling pins.
bump version:
    sed -i -E '/^\[workspace\.package\]/,/^\[/ s/^version = ".*"/version = "{{version}}"/' Cargo.toml
    sed -i -E 's/^(muster-[a-z]+ = \{ path = "[^"]+", version = )"=[^"]+"/\1"={{version}}"/' Cargo.toml
    cargo update --workspace

# Everything CI enforces, in its order.
ci: fmt-check lint lint-crates test package
