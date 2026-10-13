# Changelog

All notable changes are documented here. The project follows Semantic Versioning.

## Unreleased

- Port Graphy's shared arc/cubic corners, dynamic UI/visual/vector storage,
  accurate cubic subdivision, context menus and macOS chrome fixes upstream.
- Reuse a generation-tagged ID-count index across frames. Warm UI registration
  is allocation-free, with duplicate-ID detection and rollback preserved.
- Add growable `text_edit::TextEditor`, sharing Unicode-grapheme editing with
  bounded `TextEdit`. Undo stores changed text; failed IME commits are atomic.
- Add the Unicode 16 official grapheme-boundary conformance fixture.
- Add `text::paragraph::Paragraph` for cached wrapping, bidi, shared-font-system
  layout, byte-position hit testing and mixed-direction selection geometry.
- Grow text, vector, basic-quad and emoji instance storage automatically. Emoji
  atlas growth preserves stable layer addresses and existing pixels; no truncation.
- Grow alloc-enabled panels, tabs and overlays. Split identities no longer refer
  to array positions and survive docking relocation.
- Correct disabled pointer activation, hover-release change reporting, and modal
  keyboard/pointer scope. Release removed-frame dynamic semantic strings.
- **API migration:** semantic labels/values use `SemanticText` (`"Save".into()`
  or `Arc<str>`). Semantic export borrows `&Semantics`; `WidgetSpec` is Clone.
  `TextRenderer::sync_font` and emoji cache/prepare now return Results.
  Set modal bounds before registering the modal and its children.
- The no_std core now uses the no_std unicode-segmentation dependency; it remains
  allocation-free without alloc. Explicit bounded storage is still supported.
- Add release-only validation, strict library Clippy, machine-readable performance
  reports and allocator regression checks. No global fastest claim is made.

- Expose reusable shader sources through `lagui::gpu::shaders`, so downstream
  applications do not copy Lagui-owned WGSL assets.
- Provide portable system-font loading and color-emoji rendering as Lagui APIs
  instead of demo support code.

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
