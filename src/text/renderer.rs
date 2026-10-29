use super::{Font, TextError, TextRun};
use crate::{
    Rect,
    gpu::Damage,
    windfoil::{Analytic, CoreError, Pipeline, Placement},
};
use std::{ops::Range, vec::Vec};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Cull {
    #[default]
    Clip,
    Guard {
        margin: f32,
    },
    Off,
}

/// Logical pixel coordinates; baseline is the left/baseline origin of the run.
/// Colors are linear premultiplied RGBA. Runs may be reused at arbitrary sizes.
pub struct TextSection<'a> {
    pub transform: crate::gpu::TransformId,
    pub cull: Cull,
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
    ranges: Vec<Range<u32>>,
    guards: Vec<Option<Rect>>,
    culled: u32,
    cpu_source: Option<(u64, usize, usize)>,
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
        Self::with_pipeline(device, &Pipeline::new_text(device, format), max_glyphs)
    }

    pub fn with_pipeline(
        device: &wgpu::Device,
        pipeline: &Pipeline,
        max_glyphs: usize,
    ) -> Result<Self, TextError> {
        let pipeline = pipeline.for_text();
        Ok(Self {
            ranges: Vec::new(),
            guards: Vec::new(),
            culled: 0,
            cpu_source: None,
            core: Analytic::new(device, &pipeline, "text instances", max_glyphs)
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
    ) -> Result<&[Range<u32>], TextError> {
        self.prepare_impl(queue, sections, viewport, scale_factor, None)
    }
    pub fn prepare_transformed(
        &mut self,
        queue: &wgpu::Queue,
        sections: &[TextSection<'_>],
        viewport: [f32; 2],
        scale_factor: f32,
        transforms: &crate::gpu::Transforms,
    ) -> Result<&[Range<u32>], TextError> {
        self.prepare_impl(queue, sections, viewport, scale_factor, Some(transforms))
    }
    fn prepare_impl(
        &mut self,
        queue: &wgpu::Queue,
        sections: &[TextSection<'_>],
        viewport: [f32; 2],
        scale_factor: f32,
        transforms: Option<&crate::gpu::Transforms>,
    ) -> Result<&[Range<u32>], TextError> {
        if sections.iter().any(|s| {
            transforms.map_or(s.transform != crate::gpu::TransformId::IDENTITY, |t| {
                t.get(s.transform).is_none()
            })
        }) {
            self.core.abandon();
            return Err(TextError::InvalidGeometry);
        }
        let result = self.encode(sections, viewport, scale_factor, transforms, None);
        match result {
            Ok(()) => self
                .core
                .commit_transformed(queue, viewport, scale_factor, transforms),
            Err(_) => self.core.abandon(),
        }
        result?;
        Ok(&self.ranges)
    }

    fn encode(
        &mut self,
        sections: &[TextSection<'_>],
        viewport: [f32; 2],
        scale_factor: f32,
        transforms: Option<&crate::gpu::Transforms>,
        source: Option<(u64, usize, usize)>,
    ) -> Result<(), TextError> {
        if viewport.iter().any(|v| !v.is_finite() || *v <= 0.)
            || !scale_factor.is_finite()
            || scale_factor <= 0.
        {
            return Err(TextError::InvalidGeometry);
        }
        if sections.iter().any(|s| {
            transforms.map_or(s.transform != crate::gpu::TransformId::IDENTITY, |t| {
                t.get(s.transform).is_none()
            })
        }) {
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
        let (font, _, bands) = source.unwrap_or_else(|| self.core.uploaded());

        self.core.begin();
        self.ranges.clear();
        self.culled = 0;
        self.guards.resize(sections.len(), None);
        let mut emitted = 0;
        for (section_index, section) in sections.iter().enumerate() {
            if section.run.font_id != font {
                return Err(TextError::WrongFont);
            }
            if !section.size.is_finite()
                || section.size <= 0.
                || section
                    .baseline
                    .iter()
                    .chain(&section.color)
                    .any(|v| !v.is_finite())
                || section
                    .clip
                    .is_some_and(|r| [r.x, r.y, r.width, r.height].iter().any(|v| !v.is_finite()))
                || matches!(section.cull, Cull::Guard { margin } if !margin.is_finite() || margin < 0.)
            {
                return Err(TextError::InvalidGeometry);
            }
            // Baselines sit on physical pixel rows, so horizontal stems and the
            // x-height land on the same rows wherever a label is placed. Advances
            // keep subpixel precision horizontally.
            let baseline_y = section.baseline[1];
            let clip = section
                .clip
                .unwrap_or(Rect::new(0., 0., viewport[0], viewport[1]));
            let transform = transforms
                .and_then(|t| t.get(section.transform))
                .unwrap_or_default();
            let screen_clip = if matches!(section.cull, Cull::Guard { .. }) {
                transforms
                    .and_then(|t| t.guard_clip(section.transform))
                    .or_else(|| {
                        transforms
                            .map(|t| t.clip(section.transform, clip, false))
                            .unwrap_or(Some(clip))
                    })
                    .unwrap_or_default()
            } else {
                transforms
                    .map(|t| t.clip(section.transform, clip, false))
                    .unwrap_or(Some(clip))
                    .unwrap_or_default()
            };
            let local_clip = Rect::new(
                (screen_clip.x - transform.offset[0]) / transform.scale,
                (screen_clip.y - transform.offset[1]) / transform.scale,
                screen_clip.width / transform.scale,
                screen_clip.height / transform.scale,
            );
            let cull_rect = match section.cull {
                Cull::Off => None,
                Cull::Clip => {
                    self.guards[section_index] = None;
                    Some(local_clip)
                }
                Cull::Guard { margin } => {
                    let guard = &mut self.guards[section_index];
                    if !guard.is_some_and(|r| {
                        r.x <= local_clip.x
                            && r.y <= local_clip.y
                            && r.x + r.width >= local_clip.x + local_clip.width
                            && r.y + r.height >= local_clip.y + local_clip.height
                    }) {
                        let m = margin / transform.scale;
                        *guard = Some(Rect::new(
                            local_clip.x - m,
                            local_clip.y - m,
                            local_clip.width + 2. * m,
                            local_clip.height + 2. * m,
                        ));
                    }
                    *guard
                }
            };
            // Include both the one-pixel footprint and baseline rounding displacement.
            let margin = 1.5 / (scale_factor * transform.scale);
            let visible = |r: Rect| {
                cull_rect.is_none_or(|clip| {
                    Rect::new(
                        r.x - margin,
                        r.y - margin,
                        r.width + 2. * margin,
                        r.height + 2. * margin,
                    )
                    .intersects(clip)
                })
            };
            let start = emitted;
            let section_visible = section.run.ink_bounds().is_some_and(|r| {
                visible(Rect::new(
                    section.baseline[0] + r.x * section.size,
                    baseline_y + r.y * section.size,
                    r.width * section.size,
                    r.height * section.size,
                ))
            });
            for positioned in &section.run.glyphs {
                let g = positioned.glyph;
                // Reject runs shaped after the currently uploaded atlas revision.
                if g.band_start as usize + g.bands as usize > bands {
                    return Err(TextError::AtlasOutOfDate);
                }
                let rect = Rect::new(
                    section.baseline[0] + (positioned.position[0] + g.bounds[0]) * section.size,
                    baseline_y - (positioned.position[1] + g.bounds[3]) * section.size,
                    (g.bounds[2] - g.bounds[0]) * section.size,
                    (g.bounds[3] - g.bounds[1]) * section.size,
                );
                if !section_visible || !visible(rect) {
                    self.culled += 1;
                    continue;
                }
                self.core.push(&Placement {
                    transform: section.transform,
                    baseline: Some(baseline_y),
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
                emitted += 1;
            }
            self.ranges.push(start..emitted);
        }
        Ok(())
    }

    /// Encode into a shared outline batch without uploading a private instance or atlas buffer.
    pub fn prepare_cpu(
        &mut self,
        font: &Font,
        sections: &[TextSection<'_>],
        viewport: [f32; 2],
        dpi: f32,
        transforms: Option<&crate::gpu::Transforms>,
    ) -> Result<&[Range<u32>], TextError> {
        let source = (font.id, font.curves.len(), font.bands.len());
        self.cpu_source = None;
        if let Err(error) = self.encode(sections, viewport, dpi, transforms, Some(source)) {
            self.core.abandon();
            return Err(error);
        }
        self.cpu_source = Some(source);
        Ok(&self.ranges)
    }
    pub fn prepared_source<'a>(
        &'a self,
        font: &'a Font,
    ) -> Result<crate::gpu::outlines::PreparedOutline<'a>, TextError> {
        if self.cpu_source != Some((font.id, font.curves.len(), font.bands.len())) {
            return Err(TextError::AtlasOutOfDate);
        }
        Ok(crate::gpu::outlines::PreparedOutline::new(
            0,
            font.id,
            &font.curves,
            &font.bands,
            self.core.encoded(),
        ))
    }

    /// Edge coverage gamma, [`crate::windfoil::PERCEPTUAL_COVERAGE_GAMMA`] by default; `1.0`
    /// restores linear coverage. Takes effect on the next prepare.
    pub fn set_coverage_gamma(&mut self, gamma: f32) {
        self.core.set_coverage_gamma(gamma);
    }

    /// Region changed by the latest prepare, for hosts that redraw partially.
    pub fn upload_stats(&self) -> crate::gpu::stats::UploadStats {
        self.core.upload_stats()
    }

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

    pub fn culled(&self) -> u32 {
        self.culled
    }

    pub fn prepared_len(&self) -> u32 {
        self.core.count()
    }
}
