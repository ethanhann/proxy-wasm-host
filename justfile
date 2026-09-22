# List recipes.
_list:
    @just --list

# Install the development tooling that the quality checks use.
install-dev-tools:
    cargo install --locked cargo-machete cargo-deny cargo-audit

# Build every workspace target.
build:
    cargo build --workspace --all-targets --locked

# Run unit, integration, and doc tests.
test:
    cargo test --workspace --all-targets --locked
    cargo test --workspace --doc --locked

# Run tests and show coverage report
test-with-coverage:
    cargo llvm-cov nextest --workspace --all-features --summary-only --ignore-filename-regex 'tests/|examples/'

# Run clippy with warnings denied.
lint:
    cargo clippy --workspace --all-targets --locked -- -D warnings

# Format the workspace.
fmt:
    cargo fmt --all

# Check formatting without writing.
fmt-check:
    cargo fmt --all -- --check

# Build rustdoc with warnings denied.
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked

# Run every check that CI runs.
check: fmt-check lint build test doc

# Run the code quality checks.
check-code-quality: lint
    cargo machete
    cargo audit
    cargo deny check

# Build the test guests for wasm32-wasip1 and copy them into the fixtures directory.
build-guests:
    #!/usr/bin/env bash
    set -euo pipefail
    root="$(pwd)"
    guests_dir="$root/crates/test-guests"
    fixtures_dir="$root/crates/proxy-wasm-host/tests/fixtures"
    target="wasm32-wasip1"
    shopt -s nullglob
    members=()
    for guest in "$guests_dir"/*/; do
        [ -f "$guest/Cargo.toml" ] && members+=("$guest")
    done
    if [ "${#members[@]}" -eq 0 ]; then
        echo "no guests under crates/test-guests, nothing to build"
        exit 0
    fi
    cd "$guests_dir"
    rustup toolchain install --no-self-update
    pinned="$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)"
    running="$(rustc -vV | sed -n 's/^release: //p')"
    if [ "$running" != "$pinned" ]; then
        echo "FAIL rustc $running is running but rust-toolchain.toml pins $pinned"
        exit 1
    fi
    if ! rustup target list --installed | grep -qx "$target"; then
        echo "FAIL the $target target is not installed, run: rustup target add $target --toolchain $pinned"
        exit 1
    fi
    cargo_home="${CARGO_HOME:-$HOME/.cargo}"
    sysroot="$(rustc --print sysroot)"
    commit="$(rustc -vV | sed -n 's/^commit-hash: //p')"
    export RUSTFLAGS="--remap-path-prefix=$cargo_home/registry/src=/cargo/registry --remap-path-prefix=$guests_dir=/guest --remap-path-prefix=$sysroot/lib/rustlib/src/rust=/rustc/$commit"
    echo "RUSTFLAGS=$RUSTFLAGS"
    cargo build --release --locked --workspace --target "$target"
    mkdir -p "$fixtures_dir"
    for guest in "${members[@]}"; do
        name="$(basename "$guest")"
        artifact="$guests_dir/target/$target/release/${name//-/_}.wasm"
        if [ ! -f "$artifact" ]; then
            echo "FAIL expected artifact $artifact after building $name"
            exit 1
        fi
        cp "$artifact" "$fixtures_dir/$name.wasm"
        chmod 644 "$fixtures_dir/$name.wasm"
        echo "built crates/proxy-wasm-host/tests/fixtures/$name.wasm"
    done

# Compare the copied guest sources with the release of the SDK they came from.
# Run it when a new SDK version appears, and before a release.
# It needs the network, so it is not part of `check`.
check-guest-sources:
    #!/usr/bin/env bash
    set -euo pipefail
    tag="$(sed -n 's/^The copies come from tag \([^,]*\),.*/\1/p' NOTICE)"
    if [ -z "$tag" ]; then
        echo "FAIL NOTICE does not name the tag the copies came from"
        exit 1
    fi
    work="$(mktemp -d)"
    trap 'rm -rf "$work"' EXIT
    echo "fetching proxy-wasm-rust-sdk $tag"
    git -c advice.detachedHead=false clone --quiet --depth 1 --branch "$tag" \
        https://github.com/proxy-wasm/proxy-wasm-rust-sdk.git "$work/sdk"
    drift=0
    for guest in crates/test-guests/sdk-*/; do
        member="$(basename "$guest")"
        upstream="$work/sdk/examples/${member#sdk-}"
        upstream="${upstream//-/_}"
        if [ ! -d "$upstream/src" ]; then
            echo "FAIL $member has no example named ${upstream##*/} at $tag"
            drift=1
            continue
        fi
        if ! diff -ru "$upstream/src" "$guest/src"; then
            echo "FAIL the sources of $member differ from $tag"
            drift=1
        fi
        expected="$work/$member.toml"
        awk -v member="$member" '
            /^\[profile\.release\]$/ { stop = 1 }
            stop { next }
            !named && /^name = / { print "name = \"" member "\""; named = 1; next }
            { print }
        ' "$upstream/Cargo.toml" \
            | sed 's|proxy-wasm = { path = "../../" }|proxy-wasm = "=0.2.5"|' \
            | awk 'NF { blank = 0; print; next } { blank++ } END {}' > "$expected"
        if ! diff -u "$expected" <(awk 'NF' "$guest/Cargo.toml"); then
            echo "FAIL the manifest of $member differs from $tag by more than the three changes"
            drift=1
        fi
    done
    if [ "$drift" -ne 0 ]; then
        echo "the copied guest sources differ from $tag"
        exit 1
    fi
    echo "every copied guest source matches $tag"

# Run the benchmarks.
bench:
    cargo bench --workspace --locked

# Run docs site locally
docs:
    cd docs && mdbook serve
