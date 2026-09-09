# Windfoil attribution

Algorithm and shader structure adapted from:
https://github.com/texel-org/windfoil

Upstream revision used for this port:
`46bb1f170938564b08e358c3be20dac1b4b9e7fb`

Copyright 2026 Matt DesLauriers. Licensed under Apache License 2.0.
The upstream license text is preserved in `LICENSE` in this directory.

Lagui changes the JavaScript atlas builder to Rust, stores padded quadratic
structures rather than a packed `vec2` array, supplies instances through a
vertex buffer, and integrates the shader with its existing clipping and text
shaping pipeline.
