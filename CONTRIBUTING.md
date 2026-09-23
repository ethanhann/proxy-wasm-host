# Contributing

Thanks for your interest in improving proxy-wasm-host.

## What you need

You need [rustup](https://rustup.rs/), Rust 1.96 or later, and [just](https://github.com/casey/just).
The quality checks use a few extra cargo tools, which you can install in one step:

```sh
just install-dev-tools
```

The test guests are built with their own pinned toolchain and the `wasm32-wasip1` target.
You do not need to install either by hand, because `just build-guests` installs both on first use.

## Checking a change

Run the main checks before you commit:

```sh
just check
```

`check` runs the format check, clippy, the build, the tests, and rustdoc.
Before you open a pull request, also run the quality checks and the package check:

```sh
just check-code-quality
just check-package
```

`check-package` needs the network, since it runs a publish dry run against the crates.io index.
It also refuses to run while a file the package holds, the README, or a manifest has uncommitted changes, so commit your work first.
Run `just` with no arguments to see every recipe.

## What the lints ask of you

The workspace turns on a strict set of lints, and CI treats every warning as an error.
Every public item needs a doc comment.
The `clippy::all` and `clippy::pedantic` groups are denied.
The workspace forbids `unsafe` code.

`unwrap` and `expect` are denied outside tests, so in application code you return the error instead.
Clippy allows them in a `#[test]` function and a `#[cfg(test)]` module, but not in a helper function of an integration test file.
That is why each file under `tests/` starts with `#![allow(clippy::unwrap_used, clippy::expect_used)]`, and a new test file should do the same.

## The size of a file

A source file holds at most 600 lines of application code, and a file with more than 300 lines of it is a candidate for a split.
The tests in a file's `#[cfg(test)]` module are counted separately, against the same limits.
Integration tests, examples, and benches follow the same limits.

## Writing tests

Each test follows the Arrange, Act, and Assert pattern, with each section marked by a comment.
Keep the Act section to a single statement, so it is clear what the test exercises:

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

Prefer assertions that check the whole result.
In the example above, the test compares every entry the sink received, not only the number of entries, so a wrong level or a wrong message fails it too.

## The test guests

Many tests run real Proxy-Wasm guests, which live as compiled modules under `crates/proxy-wasm-host/tests/fixtures`.
The modules are committed, so the tests run without a guest toolchain.
Most of them are built from the guest crates in `crates/test-guests`, and you can rebuild those with:

```sh
just build-guests
```

The TinyGo module is the exception.
It is a copy from proxy-wasm-go-host, as `NOTICE` explains, and its source is not in this repository.

A rebuild on another platform can produce different bytes for the same source.
Only commit a rebuilt fixture when you changed its guest, and commit the one you tested against.

Some guests are unchanged copies of the example plugins of the Rust SDK, and `just check-guest-sources` compares them with their upstream release.

To add a guest, create its crate under `crates/test-guests`, add it to the `members` list of `crates/test-guests/Cargo.toml`, and run `just build-guests`.
If the guest is copied from another project, add a paragraph that credits it to the `NOTICE` file at the repository root.

## The documentation site

The guide under `docs/` is an [mdBook](https://rust-lang.github.io/mdBook/) site.
Install mdBook with `cargo install mdbook`, and then serve the site locally with:

```sh
just docs
```

Please open an issue before you add or rewrite a page, so the change can be discussed first.

## License

The project is licensed under the Apache License, Version 2.0.
By contributing, you agree that your contribution is licensed under the same terms.

## Reporting a vulnerability

Please do not open a public issue for a security problem.
[SECURITY.md](https://github.com/ethanhann/proxy-wasm-host/blob/main/SECURITY.md) explains how to report one privately.
