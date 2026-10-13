use lagui::text::{TextRenderer, TextRun, TextSection, load_system_font};
use std::{error::Error, time::Instant};

struct Label {
    run: TextRun,
    baseline: [f32; 2],
    size: f32,
    color: [f32; 4],
}
pub struct TextScene {
    pub renderer: TextRenderer,
    labels: Vec<Label>,
}

impl TextScene {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Result<Self, Box<dyn Error>> {
        let start = Instant::now();
        let mut font = load_system_font()?;
        let white = [0.82, 0.88, 0.96, 1.];
        let muted = [0.36, 0.44, 0.54, 1.];
        let specs = [
            ("LAGUI", [40., 92.], 28., white),
            ("Native. Sharp. Responsive.", [40., 121.], 13., muted),
            ("Change accent", [59., 180.], 14., white),
            ("INTENSITY", [40., 246.], 12., muted),
            ("Drag the slider to scale text.", [40., 344.], 12., muted),
            ("Space: color / Arrows: intensity", [40., 369.], 12., muted),
            ("P: timing samples", [40., 394.], 12., muted),
            ("Outline text, on the GPU.", [380., 90.], 28., white),
            ("ベジェ曲線から、文字を描く。", [380., 131.], 24., white),
            (
                "日本語 / Aa Bb 0123456789 / ffi / café",
                [380., 163.],
                14.,
                muted,
            ),
            ("拡大しても、なめらか。", [380., 574.], 24., white),
        ];
        let mut labels = Vec::new();
        for (text, baseline, size, color) in specs {
            labels.push(Label {
                run: font.shape(text)?,
                baseline,
                size,
                color,
            });
        }
        let mut renderer = TextRenderer::new(device, format, 1024)?;
        renderer.sync_font(device, &font)?;
        println!(
            "Windfoil: {} cached glyphs, {} banded quadratic pieces, {} atlas bytes; cold font/shape/pipeline {:.2} ms",
            font.cached_glyphs(),
            font.curve_count(),
            font.atlas_bytes(),
            start.elapsed().as_secs_f64() * 1000.
        );
        // Runs own their positioned glyph metadata; the font is no longer needed
        // until a caller wants to shape additional strings.
        Ok(Self { renderer, labels })
    }
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        viewport: [f32; 2],
        dpi: f32,
        value: f32,
    ) -> Result<(), Box<dyn Error>> {
        let sections: [TextSection<'_>; 11] = std::array::from_fn(|i| {
            let label = &self.labels[i];
            TextSection {
                run: &label.run,
                baseline: if i == 10 {
                    [380., viewport[1] - 26.]
                } else {
                    label.baseline
                },
                size: if i == 10 {
                    14. + value * 24.
                } else {
                    label.size
                },
                color: label.color,
                clip: None,
            }
        });
        self.renderer.prepare(queue, &sections, viewport, dpi)?;
        Ok(())
    }
}
