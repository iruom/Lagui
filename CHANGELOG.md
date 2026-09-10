# Changelog

All notable changes are documented here. The project follows Semantic Versioning.

## Unreleased

- Expose reusable shader sources through `lagui::gpu::shaders`, so downstream
  applications do not copy Lagui-owned WGSL assets.

- Made `gpu::PrepareError` a standard error so downstream applications can
  propagate renderer setup and preparation failures normally.
- Added strict, duplicate-detecting widget registration and a reusable widget
  registration helper that derives visual state from interaction and semantics.
- Added atomic standard button and switch builders plus a downstream public-API test.
- Added `WidgetContext` transactions so application-defined widgets can atomically
  compose semantic regions and visual primitives; standard widgets use the same API.
- Completed single-axis Flex sizing with shrink and min/max redistribution.
- Added displayable public capacity and registration errors.
- Added `GpuCanvas` viewport/scissor integration, an embedded depth-tested 3D preview,
  and a separately managed live preview window in Studio.
- Fixed raster fallback/emoji text bypassing painter's order and made degenerate
  node-connection geometry non-fatal at extreme zoom levels.
- Added reusable platform adapter contracts.
- Added multiline and word-wise text navigation helpers.
- Added grapheme/run-level system-font, color-emoji, and BiDi fallback rendering to Studio;
  surrounding outline text remains on the Windfoil path.
- Added cross-platform CI and release metadata.

## 0.1.0 - 2026-09-09

- Initial allocation-free UI, layout, panels, overlays, text editing, wgpu rendering,
  Windfoil vector rendering, and native studio demo.
