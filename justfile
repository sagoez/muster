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

# Everything CI enforces.
ci: fmt-check lint test
