use super::{Font, TextError, TextRun};
use crate::{
    Rect,
    gpu::Damage,
    windfoil::{Analytic, CoreError, Pipeline, Placement},
};
use std::ops::Range;

/// Logical pixel coordinates; baseline is the left/baseline origin of the run.
/// Colors are linear premultiplied RGBA. Runs may be reused at arbitrary sizes.
pub struct TextSection<'a> {
    pub run: &'a TextRun,
    pub baseline: [f32; 2],
    pub size: f32,
    pub color: [f32; 4],
    /// Optional scissor rectangle in logical pixels.
    pub clip: Option<Rect>,
}

/// Glyph runs of one [`Font`], drawn through the shared Windfoil [`Pipeline`].
/// Glyphs are ordinary Windfoil instances placed at `size` with y pointing up.
pub struct TextRenderer {
    core: Analytic,
}

impl From<CoreError> for TextError {
    fn from(error: CoreError) -> Self {
        match error {
            CoreError::Capacity => Self::Capacity,
            CoreError::InvalidGeometry => Self::InvalidGeometry,
        }
    }
}

impl TextRenderer {
    /// Compiles a private pipeline. Prefer [`Self::with_pipeline`] so every
    /// font shares one compiled shader.
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        max_glyphs: usize,
    ) -> Result<Self, TextError> {
        Self::with_pipeline(device, &Pipeline::new(device, format), max_glyphs)
    }

    pub fn with_pipeline(
        device: &wgpu::Device,
        pipeline: &Pipeline,
        max_glyphs: usize,
    ) -> Result<Self, TextError> {
        Ok(Self {
            core: Analytic::new(device, pipeline, "text instances", max_glyphs)
                .map_err(|_| TextError::Capacity)?,
        })
    }

    /// Grow instance storage while preserving the compiled pipeline and atlas.
    /// Submitted frames retain their old buffer through wgpu's resource lifetime.
    pub fn reserve(&mut self, _device: &wgpu::Device, required: usize) -> Result<(), TextError> {
        self.core.reserve(required).map_err(|_| TextError::Capacity)
    }

    /// Upload new outlines only after shaping has introduced new glyphs.
    /// Replaces the immutable atlas buffers; existing submitted frames retain theirs.
    pub fn sync_font(&mut self, device: &wgpu::Device, font: &Font) -> Result<(), TextError> {
        self.core
            .sync(device, font.id, &font.curves, &font.bands)
            .map_err(|_| TextError::Capacity)
    }

    /// No shaping or font parsing here. Only glyph instances that changed since
    /// the previous prepare are uploaded; see [`Self::damage`].
    /// Submit before preparing the next batch.
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        sections: &[TextSection<'_>],
        viewport: [f32; 2],
        scale_factor: f32,
    ) -> Result<(), TextError> {
        let result = self.encode(sections, viewport, scale_factor);
        match result {
            Ok(()) => self.core.commit(queue, viewport, scale_factor),
            Err(_) => self.core.abandon(),
        }
        result
    }

    fn encode(
        &mut self,
        sections: &[TextSection<'_>],
        viewport: [f32; 2],
        scale_factor: f32,
    ) -> Result<(), TextError> {
        if viewport.iter().any(|v| !v.is_finite() || *v <= 0.)
            || !scale_factor.is_finite()
            || scale_factor <= 0.
        {
            return Err(TextError::InvalidGeometry);
        }
        let required = sections.iter().try_fold(0usize, |count, section| {
            count
                .checked_add(section.run.glyph_count())
                .ok_or(TextError::Capacity)
        })?;
        self.core
            .reserve(required)
            .map_err(|_| TextError::Capacity)?;
        let (font, _, bands) = self.core.uploaded();
        let snap = |y: f32| (y * scale_factor).round() / scale_factor;
        self.core.begin();
        for section in sections {
            if section.run.font_id != font {
                return Err(TextError::WrongFont);
            }
            if !section.size.is_finite() || section.size <= 0. {
                return Err(TextError::InvalidGeometry);
            }
            // Baselines sit on physical pixel rows, so horizontal stems and the
            // x-height land on the same rows wherever a label is placed. Advances
            // keep subpixel precision horizontally.
            let baseline_y = snap(section.baseline[1]);
            let clip = section
                .clip
                .unwrap_or(Rect::new(0., 0., viewport[0], viewport[1]));
            for positioned in &section.run.glyphs {
                let g = positioned.glyph;
                // Reject runs shaped after the currently uploaded atlas revision.
                if g.band_start as usize + g.bands as usize > bands {
                    return Err(TextError::AtlasOutOfDate);
                }
                self.core.push(&Placement {
                    origin: [
                        section.baseline[0] + positioned.position[0] * section.size,
                        baseline_y - positioned.position[1] * section.size,
                    ],
                    scale: [section.size, -section.size],
                    bounds: g.bounds,
                    band_start: g.band_start,
                    bands: g.bands,
                    band_h: g.band_h,
                    inv_h: g.inv_h,
                    color: section.color,
                    clip,
                })?;
            }
        }
        Ok(())
    }

    /// Edge coverage gamma, [`crate::windfoil::PERCEPTUAL_COVERAGE_GAMMA`] by default; `1.0`
    /// restores linear coverage. Takes effect on the next prepare.
    pub fn set_coverage_gamma(&mut self, gamma: f32) {
        self.core.set_coverage_gamma(gamma);
    }

    /// Region changed by the latest prepare, for hosts that redraw partially.
    pub fn damage(&self) -> Damage {
        self.core.damage()
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.draw_range(pass, 0..self.core.count());
    }

    /// Draws a prepared glyph subrange so text can be interleaved with other primitives.
    pub fn draw_range(&self, pass: &mut wgpu::RenderPass<'_>, range: Range<u32>) {
        self.core.draw_range(pass, range);
    }

    pub fn prepared_len(&self) -> u32 {
        self.core.count()
    }
}
