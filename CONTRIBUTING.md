# Contributing

## Requirements

You need [rustup](https://rustup.rs/), Rust 1.96 or later, and [just](https://github.com/casey/just).
`just install-dev-tools` installs the cargo tools that the other recipes use.

## Checks

```sh
just check
```

`check` runs the formatter, clippy, the build, the tests, and rustdoc, and CI runs the same recipe.
Before you open a pull request, also run `just check-code-quality` and `just check-package`.
The package check runs a publish dry run, so it needs network access and a committed tree.
Run `just` with no arguments to list every recipe.

## Conventions

- Every public item has a doc comment, and CI treats every warning as an error.
- `unsafe` code is forbidden, and `unwrap` and `expect` are denied outside tests.
  Each file under `tests/` starts with `#![allow(clippy::unwrap_used, clippy::expect_used)]`, because clippy does not treat the helper functions of an integration test as test code.
- A source file contains at most 600 lines of application code.
  A file over 300 lines, or a test module over 600 lines, is a candidate for a split.
- A module with submodules is a `name.rs` file next to a `name/` directory, with no `mod.rs` files.
  The exception is `tests/common/mod.rs`, which Cargo requires.
- Each test follows the Arrange, Act, Assert pattern, with each section marked by a comment and a single statement in Act:

```rust
#[test]
fn a_message_at_the_log_bound_reaches_the_sink_whole() {
    // Arrange
    let sink = Arc::new(RecordingSink::default());
    let mut instance = bounded(&sink, 5);

    // Act
    let result = log_hello(&mut instance);

    // Assert
    assert_eq!(result, Status::Ok);
    assert_eq!(sink.entries(), vec![(LogLevel::Info, b"hello".to_vec())]);
}
```

## Test guests

The tests run real Proxy-Wasm guests, committed as compiled modules under `crates/proxy-wasm-host/tests/fixtures`.
Most of them are built from the crates in `crates/test-guests` with `just build-guests`, which installs the pinned toolchain and the `wasm32-wasip1` target on first use.
A rebuild on another platform can produce different bytes for the same source, so commit a rebuilt fixture only when you changed its guest.

To add a guest, create its crate under `crates/test-guests`, add it to the `members` list of `crates/test-guests/Cargo.toml`, and run `just build-guests`.
If you copied the guest from another project, credit it in `NOTICE`.

## Documentation site

The guide under `docs/` is an [mdBook](https://rust-lang.github.io/mdBook/) site, and `just docs` serves it locally.
Open an issue before you add or rewrite a page.

## License

By contributing, you agree that your contribution is licensed under the Apache License, Version 2.0.
