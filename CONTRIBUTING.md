# Contributing

Rust 1.92.0 and the platform GPU development tools are required.

Before submitting a change, run:

```sh
bash scripts/check.sh
bash scripts/check.sh gpu
bash scripts/check.sh smoke # requires a logged-in desktop
```

Public API changes follow Semantic Versioning. Add user-visible changes to
`CHANGELOG.md`, preserve painter's order, and return capacity errors atomically.
Platform-specific functionality belongs behind the contracts in `platform`.

Use release builds only. Keep rendering, platform services, text layout and input
storage separate. Application documents and committed undo belong to the host.
Include regressions for failure atomicity, stable identity and resource growth.
GPU skips must be visible; `LAGUI_REQUIRE_GPU=1` makes adapter absence a failure.
Keep the build environment with benchmark reports. CPU command construction is
not GPU time or input-to-photon latency. Performance regressions require repeated
measurements on a fixed machine, not thresholds on an arbitrary hosted runner.
