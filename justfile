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
    crates=$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[].name')
    [ -n "$crates" ] || { echo "cargo metadata named no crate" >&2; exit 1; }
    for crate in $crates; do
        cargo clippy -p "$crate" --all-targets -- -D warnings
    done

# Package every crate as crates.io would, uncommitted work included, without building them.
# Verifying resolves siblings from crates.io; the release's `cargo publish` verifies each crate against its freshly published siblings in order.
package:
    cargo package --workspace --locked --no-verify --allow-dirty

# Set the release version of every crate and its sibling pins.
bump version:
    sed -i -E '/^\[workspace\.package\]/,/^\[/ s/^version = ".*"/version = "{{version}}"/' Cargo.toml
    sed -i -E 's/^(muster-[a-z]+ = \{ path = "[^"]+", version = )"=[^"]+"/\1"={{version}}"/' Cargo.toml
    cargo update --workspace

# Everything CI enforces, in its order.
ci: fmt-check lint lint-crates test package
