# Changelog

All notable changes are documented here. The project follows Semantic Versioning.

## Unreleased

- Add 32-byte frame-local GPU transform and clip tables shared by rounded visuals and Windfoil.
  Instances keep local coordinates; transform-only changes damage the old and
  new clipped bounds without uploading instances. Snap text baselines in the
  vertex shader. The visual and outline instance formats are now 80 bytes.
- Add `TextSection::cull` (`Clip`, persistent local `Guard`, `Off`), cached run
  ink bounds, and section ranges returned by prepare. Callers must use these
  ranges rather than assuming every shaped glyph is emitted.
- Add CPU-only outline preparation and `gpu::outlines::OutlineBatch`, sharing
  one outline atlas and painter-ordered instance stream across fonts and paths.
- Add `gpu::RetainedTarget` with physical damage scissors, replacement clears
  for translucent replay, a presentation blit, and backdrop damage closure.
- Add `PresentPolicy`, the platform `PointerProbe` contract, and the AppKit
  pointer sampling bridge. Portable examples accept `LAGUI_BACKEND`.
- Add upload counters, `FrameStats`, nonblocking pass timestamp rings and the
  `bench_rendering` offscreen benchmark with JSON Lines percentile reports.
- Add bidirectional band integration with a median split and indirect left
  curve runs; retain direct right runs and the minification profile path.
  `Pipeline::with_band_direction` permits same-device reference comparisons.
- Add an experimental unified vertex-pulling pipeline for same-scene comparison.
  It is not selected by default; final validation and adoption measurements are deferred.

- `VisualList::growable()` and `VisualList::with_capacity(n)` (`alloc`) own
  their storage and grow instead of returning `CapacityExceeded`. `clear`
  keeps the allocation. Borrowed lists from `VisualList::new` are unchanged;
  `is_growable()` tells them apart and `capacity()` reports `usize::MAX` for
  growable lists.
- `Font::metrics()` returns `FontMetrics` (ascender, descender, line gap, cap
  height, x-height in em) with `line_height()` and `centered_baseline()`.
- `TextRun::carets()` lists caret stops (byte offset and x in em) at every
  grapheme boundary, splitting ligature clusters evenly. `caret_x` and
  `hit_test` map between byte offsets and positions.
- Antialiasing quality:
  - Text and Windfoil vectors use perceptual edge coverage: linear blending is
    kept, but coverage is remapped so edges match gamma-space blending over a
    contrasting background (`windfoil::PERCEPTUAL_COVERAGE_GAMMA` = 2.2).
    Dark-on-light text no longer looks thin, light-on-dark no longer bold.
    `set_coverage_gamma(1.0)` on `TextRenderer`/`windfoil::Renderer` restores
    linear coverage.
  - Drop shadows are the shape convolved with a Gaussian: exact separable erf
    for sharp boxes, Evan Wallace's erf/sampled method for rounded corners.
    Edges are half opaque on the spread outline instead of fully opaque, and
    spread corners follow the CSS/Figma radius rule. `blur` stays the standard
    deviation; `DropShadow::from_blur_radius` takes CSS/Figma blur radii.
  - Smoothed-corner distances are Euclidean (Newton refinement plus the arc
    candidate), and outline interiors are the outer shape offset by the stroke
    width, so strokes keep a constant width through smoothed corners.
  - Text baselines snap to physical pixel rows; horizontal placement keeps
    subpixel precision.
- Retain prepared GPU instances. `VisualRenderer`, `TextRenderer` and
  `windfoil::Renderer` compare each prepare with the resident buffer and upload
  only changed instance runs; `damage()` reports the changed logical region
  (`gpu::Damage::{None, Region, Full}`) for hosts that redraw partially.
- Text and vector shapes now share one Windfoil GPU path. `windfoil::Pipeline`
  compiles the shader once per format; `TextRenderer::with_pipeline` and
  `windfoil::Renderer::with_pipeline` reuse it, so new fonts never recompile and
  consecutive draws keep the bound pipeline. `new` remains for a private pipeline.
- Smaller instances: visuals 128 → 76 bytes (half-float colors/shadow, draw bounds
  derived on the GPU), Windfoil glyphs/shapes 96 → 72 bytes (affine placement).
  Scale factor moved to the viewport uniform.
- Clip quads in the vertex stage instead of discarding fragments (same pixel-center
  rule), skip the shadow distance field for shadowless visuals, and expand quads by
  the shadow reach only when the shadow color is non-zero.

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
