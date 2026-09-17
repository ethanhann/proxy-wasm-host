# List recipes.
_list:
    @just --list

# Build every workspace target.
build:
    cargo build --workspace --all-targets --locked

# Run unit, integration, and doc tests.
test:
    cargo test --workspace --all-targets --locked
    cargo test --workspace --doc --locked

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

# Build the test guests for wasm32-wasip1 and copy them into the fixtures directory.
build-guests:
    #!/usr/bin/env bash
    set -euo pipefail
    guests_dir="crates/test-guests"
    fixtures_dir="crates/proxy-wasm-host/tests/fixtures"
    target="wasm32-wasip1"
    target_dir="$guests_dir/target"
    shopt -s nullglob
    guests=("$guests_dir"/*/)
    if [ "${#guests[@]}" -eq 0 ]; then
        echo "no guests under $guests_dir, nothing to build"
        exit 0
    fi
    if ! rustup target list --installed | grep -qx "$target"; then
        echo "FAIL the $target target is not installed, run: rustup target add $target"
        exit 1
    fi
    mkdir -p "$fixtures_dir"
    for guest in "${guests[@]}"; do
        name=$(basename "$guest")
        artifact="$target_dir/$target/release/${name//-/_}.wasm"
        cargo build --release --locked --target "$target" --target-dir "$target_dir" --manifest-path "$guest/Cargo.toml"
        if [ ! -f "$artifact" ]; then
            echo "FAIL expected artifact $artifact after building $name"
            exit 1
        fi
        cp "$artifact" "$fixtures_dir/$name.wasm"
        echo "built $fixtures_dir/$name.wasm"
    done

# Run the benchmarks.
bench:
    cargo bench --workspace --locked
