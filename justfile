# muster task runner. Run `just` to list recipes.

# Path to the installed nightly rustfmt (unstable import options require nightly).
nightly_rustfmt := `ls ~/.rustup/toolchains/nightly*/bin/rustfmt 2>/dev/null | head -1`

# List available recipes.
default:
    @just --list

# Type-check the crate.
check:
    cargo check

# Build the crate.
build:
    cargo build

# Run the TUI. Pass args after `--`, e.g. `just run -- --config muster.yml`.
run *ARGS:
    cargo run -- {{ARGS}}

# Run the test suite.
test:
    cargo test

# Format with nightly rustfmt (honors rustfmt.toml; no global config change).
fmt:
    RUSTFMT="{{nightly_rustfmt}}" cargo fmt

# Verify formatting without writing changes.
fmt-check:
    RUSTFMT="{{nightly_rustfmt}}" cargo fmt --check

# Lint; warnings are errors.
lint:
    cargo clippy --all-targets -- -D warnings

# Everything CI enforces.
ci: fmt-check lint test
