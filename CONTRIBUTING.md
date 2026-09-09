# Contributing

Rust 1.92.0 and the platform GPU development tools are required.

Before submitting a change, run:

```sh
cargo fmt --all -- --check
cargo test --features text
cargo clippy --all-targets --features portable,native -- -D warnings
cargo package --allow-dirty --no-verify
```

Public API changes follow Semantic Versioning. Add user-visible changes to
`CHANGELOG.md`, preserve painter's order, and return capacity errors atomically.
Platform-specific functionality belongs behind the contracts in `platform`.
