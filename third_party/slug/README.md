# Slug reference code

Source: https://github.com/EricLengyel/Slug/blob/main/SlugPixelShader.hlsl

Retrieved 2026-09-08. SHA-256 of the preserved source:
`6eb2d77216aa6fd2bf92e0141a7fd54012340e6b3fdb4fff2c3a033d01719cf0`

`native/slug.wgsl` adapts the root eligibility, dual-ray coverage and weighted
combination from Eric Lengyel's reference implementation under the MIT License.
The original copyright notice and MIT license are retained here.

The adaptation uses f32 storage buffers and u32 indices instead of the reference
texture packing. It supports nonzero winding and axis-aligned 2D glyph instances.
The vertex stage expands glyph rectangles by one physical pixel for AA.
The almost-linear root branch additionally checks for a nonzero linear coefficient.
It does not implement the complete commercial Slug Library or its layout engine.
