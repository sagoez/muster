# muster task runner. Run `just` to list recipes.

# Path to the installed nightly rustfmt (unstable import options require nightly).
nightly_rustfmt := `ls ~/.rustup/toolchains/nightly*/bin/rustfmt 2>/dev/null | head -1`

# cargo-audit release `just audit` runs through mise; CI pins the same one.
cargo_audit := "0.23.0"

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

# Check Cargo.lock against the RustSec advisory database; .cargo/audit.toml holds its policy and exemptions.
audit:
    mise exec cargo:cargo-audit@{{cargo_audit}} -- cargo audit

# Re-vendor the page's libraries, font and icons into muster-web/assets from pinned npm tarballs, each checked against its registry integrity. Needs curl, tar, openssl and sed, no Node.
vendor-web:
    #!/usr/bin/env bash
    set -euo pipefail
    assets="{{justfile_directory()}}/muster-web/assets"
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT

    # fetch <package> <version> <integrity>: unpack the tarball into $work/<package>, refusing one the registry did not publish.
    fetch() {
        local name=$1 version=$2 integrity=$3 dir="$work/$1"
        mkdir -p "$dir"
        curl -sfL -o "$dir.tgz" "https://registry.npmjs.org/$name/-/${name##*/}-$version.tgz"
        local actual="sha512-$(openssl dgst -sha512 -binary "$dir.tgz" | openssl base64 -A)"
        [ "$actual" = "$integrity" ] || { echo "integrity mismatch for $name@$version: $actual" >&2; exit 1; }
        tar xzf "$dir.tgz" -C "$dir" --strip-components=1
    }

    # module <package> <source> <target> <file>: copy an ES module without its source map comment, its licence beside it.
    module() {
        local dir="$assets/js/vendor/$3"
        mkdir -p "$dir"
        sed -E 's#//\# sourceMappingURL=[^ ]*$##' "$work/$1/$2" > "$dir/$4"
        cp "$work/$1/LICENSE" "$dir/LICENSE.txt"
    }

    fetch preact 11.2.0 sha512-/HfHI/7N84e6GrWBJ3J280z3mLaRJiDcSvj/A+y/4AbLnND/1UlGHe1sR6oYuTGwId0nP7dqDU9OnLxMPZ7xBg==
    fetch htm 3.1.1 sha512-983Vyg8NwUE7JkZ6NmOqpCZ+sh1bKv2iYTlUkzlWmA5JD2acKoxd4KVxbMmxX/85mtfdnDmTFoNKcg5DGAvxNQ==
    fetch @preact/signals-core 1.14.5 sha512-vcRv7Ityz5JVmeiVqIluLFzT/PDS/AlhHUpw3/aUWqcVUs6bDGIPXE/6RSC4evBNWYvnniozg5Syfj33al3gkg==
    fetch @preact/signals 2.11.4 sha512-Rb+5RPo9gWSaHtzJeauhHxiC8KP/IF2DEPkHPEOGOnuy2wnJzRqyeLRankUCYd6ADqtxS6GnWGv6Ie343d8aYw==
    fetch @fontsource-variable/inter 5.3.0 sha512-OupL48va4JNofb97w6NYeF9S7W/kHNKM0Er8Dem5nqi4jeOLrVJDoE8tZEpnMJmtkvNbB1EIPPwHcdkF6b1oUA==
    fetch simple-icons 16.37.0 sha512-8mCJIhpMYOL9g13GW59EiL3sVXJnTEKGCTZFaekl9UQ3Qo0SwVjnLaJ82QiaH8BKBdouEDzrHMXIWh8cBIy48A==
    fetch lucide-static 1.73.0 sha512-/FfzdRThAPnDVMk643pzENqYUnxudEON/fyOdh31ZiPiw/seE9VoYeIJPRDTCLDNtnF+5pK6CNwf6IkQMfrJ5w==

    rm -rf "$assets/js/vendor" "$assets/fonts" "$assets/icons"
    # An import map is an inline script, which the CSP forbids: bare specifiers become relative paths.
    module preact dist/preact.mjs preact preact.mjs
    module preact hooks/dist/hooks.mjs preact hooks.mjs
    sed -i -E 's#from"preact"#from"./preact.mjs"#g' "$assets/js/vendor/preact/hooks.mjs"
    module htm dist/htm.mjs htm htm.mjs
    module @preact/signals-core dist/signals-core.mjs signals-core signals-core.mjs
    module @preact/signals dist/signals.mjs signals signals.mjs
    sed -i -E 's#from"preact"#from"../preact/preact.mjs"#g; s#from"preact/hooks"#from"../preact/hooks.mjs"#g; s#from"@preact/signals-core"#from"../signals-core/signals-core.mjs"#g' "$assets/js/vendor/signals/signals.mjs"

    mkdir -p "$assets/fonts"
    cp "$work/@fontsource-variable/inter/files/inter-latin-wght-normal.woff2" "$assets/fonts/"
    cp "$work/@fontsource-variable/inter/LICENSE" "$assets/fonts/LICENSE.txt"

    mkdir -p "$assets/icons/brands" "$assets/icons/lucide"
    for slug in rust docker nodedotjs python go vite nextdotjs postgresql redis mysql mongodb nginx deno bun ruby rubyonrails django php elixir dotnet vuedotjs svelte astro nuxt rabbitmq claude; do
        cp "$work/simple-icons/icons/$slug.svg" "$assets/icons/brands/$slug.svg"
    done
    cp "$work/simple-icons/LICENSE.md" "$assets/icons/brands/LICENSE.txt"
    for icon in folder-open sticky-note check refresh-cw triangle-alert circle-x loader-circle circle-check circle-pause x square-terminal bot user pause eye-off; do
        cp "$work/lucide-static/icons/$icon.svg" "$assets/icons/lucide/$icon.svg"
    done
    cp "$work/lucide-static/icons/square-terminal.svg" "$assets/icons/brands/terminal.svg"
    cp "$work/lucide-static/LICENSE" "$assets/icons/lucide/LICENSE.txt"

    # Every vendored module imports relative paths only.
    if grep -rEq '(from|import) ?"[^./]' "$assets/js/vendor"; then
        echo "a vendored module still has a bare import" >&2
        exit 1
    fi

# Node release `just page-test` runs through mise; CI pins the same one.
node_version := "24.22.1"

# Check the muster view page in Chromium against a fake stream. Needs mise; Node and Playwright stay inside muster-web/page-tests.
page-test:
    cd muster-web/page-tests && mise exec node@{{node_version}} -- npm ci --ignore-scripts
    cd muster-web/page-tests && mise exec node@{{node_version}} -- npx playwright install chromium
    cd muster-web/page-tests && mise exec node@{{node_version}} -- npx playwright test

# Record the README's muster view demo from a real sandboxed stack and edit it into docs/muster-view.gif, the one the README shows,
# and docs/view-demo/out/muster-view.mp4, which is not committed: upload it where a video should play.
# Needs mise, ffmpeg and the system python3; Node, Playwright and Remotion stay inside docs/view-demo.
demo-view:
    cargo build --bin muster
    NODE_VERSION={{node_version}} docs/view-demo/record.sh
    NODE_VERSION={{node_version}} docs/view-demo/render.sh

# Set the release version of every crate and its sibling pins.
bump version:
    sed -i -E '/^\[workspace\.package\]/,/^\[/ s/^version = ".*"/version = "{{version}}"/' Cargo.toml
    sed -i -E 's/^(muster-[a-z]+ = \{ path = "[^"]+", version = )"=[^"]+"/\1"={{version}}"/' Cargo.toml
    cargo update --workspace

# Everything CI enforces, in its order.
ci: fmt-check lint lint-crates test package audit
