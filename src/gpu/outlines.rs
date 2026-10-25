//! A painter-ordered outline stream spanning fonts and vector sources.
use super::{Damage, PrepareError, Transforms};
use crate::{
    Rect,
    windfoil::{Analytic, Pipeline},
};
use std::{ops::Range, vec::Vec};

pub struct PreparedOutline<'a> {
    key: (u8, u64, usize, usize),
    curves: &'a [[[f32; 2]; 3]],
    rows: &'a [[u32; 5]],
    instances: &'a [u8],
}
impl<'a> PreparedOutline<'a> {
    pub(crate) fn new(
        kind: u8,
        id: u64,
        curves: &'a [[[f32; 2]; 3]],
        rows: &'a [[u32; 5]],
        instances: &'a [u8],
    ) -> Self {
        Self {
            key: (kind, id, curves.len(), rows.len()),
            curves,
            rows,
            instances,
        }
    }
}

/// One outline atlas and instance buffer. Source order is arbitrary; `order`
/// determines painter order and returns the corresponding ranges in this stream.
pub struct OutlineBatch {
    device: wgpu::Device,
    core: Analytic,
    keys: Vec<(u8, u64, usize, usize)>,
    offsets: Vec<u32>,
    ranges: Vec<Range<u32>>,
    revision: u64,
}
impl OutlineBatch {
    pub fn new(device: &wgpu::Device, pipeline: &Pipeline) -> Result<Self, PrepareError> {
        Ok(Self {
            device: device.clone(),
            core: Analytic::new(device, pipeline, "shared outlines", 1)
                .map_err(|_| PrepareError::Capacity)?,
            keys: Vec::new(),
            offsets: Vec::new(),
            ranges: Vec::new(),
            revision: 0,
        })
    }
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        sources: &[PreparedOutline<'_>],
        order: &[(usize, Range<u32>)],
        viewport: [f32; 2],
        dpi: f32,
        transforms: Option<&Transforms>,
    ) -> Result<&[Range<u32>], PrepareError> {
        let result = self.encode(sources, order, viewport, dpi, transforms);
        if let Err(error) = result {
            self.core.abandon();
            return Err(error);
        }
        self.core
            .commit_transformed(queue, viewport, dpi, transforms);
        Ok(&self.ranges)
    }
    fn encode(
        &mut self,
        sources: &[PreparedOutline<'_>],
        order: &[(usize, Range<u32>)],
        viewport: [f32; 2],
        dpi: f32,
        transforms: Option<&Transforms>,
    ) -> Result<(), PrepareError> {
        const STRIDE: usize = crate::windfoil::STRIDE;
        if viewport.iter().any(|v| !v.is_finite() || *v <= 0.) || !dpi.is_finite() || dpi <= 0. {
            return Err(PrepareError::InvalidViewport);
        }
        let mut required = 0usize;
        for (source, range) in order {
            let source = sources.get(*source).ok_or(PrepareError::InvalidViewport)?;
            if range.end < range.start || range.end as usize > source.instances.len() / STRIDE {
                return Err(PrepareError::InvalidViewport);
            }
            for bytes in source.instances
                [range.start as usize * STRIDE..range.end as usize * STRIDE]
                .chunks_exact(STRIDE)
            {
                let id = super::TransformId(u32::from_le_bytes(bytes[72..76].try_into().unwrap()));
                if transforms.map_or(id != super::TransformId::IDENTITY, |t| t.get(id).is_none()) {
                    return Err(PrepareError::InvalidViewport);
                }
            }
            required = required
                .checked_add((range.end - range.start) as usize)
                .ok_or(PrepareError::Capacity)?;
        }
        // Repack only at a source revision/scene boundary. No source is retained
        // after it disappears, so changing connection LODs cannot leak atlas space.
        if self.keys.len() != sources.len()
            || self.keys.iter().zip(sources).any(|(a, b)| *a != b.key)
        {
            let mut curves = Vec::new();
            let mut rows = Vec::new();
            let mut offsets = Vec::new();
            for source in sources {
                let curve_base = u32::try_from(curves.len()).map_err(|_| PrepareError::Capacity)?;
                offsets.push(u32::try_from(rows.len()).map_err(|_| PrepareError::Capacity)?);
                for row in source.rows {
                    let mut row = *row;
                    row[0] = row[0]
                        .checked_add(curve_base)
                        .ok_or(PrepareError::Capacity)?;
                    rows.push(row);
                }
                curves.extend_from_slice(source.curves);
            }
            self.revision = self.revision.checked_add(1).ok_or(PrepareError::Capacity)?;
            self.core
                .sync(&self.device, self.revision, &curves, &rows)
                .map_err(|_| PrepareError::Capacity)?;
            self.offsets = offsets;
            self.keys.clear();
            self.keys.extend(sources.iter().map(|s| s.key));
        }
        self.core
            .reserve(required)
            .map_err(|_| PrepareError::Capacity)?;
        self.core.begin();
        self.ranges.clear();
        let mut start = 0u32;
        for (source, range) in order {
            let count = range.end - range.start;
            self.core
                .append_encoded(
                    &sources[*source].instances
                        [range.start as usize * STRIDE..range.end as usize * STRIDE],
                    self.offsets[*source],
                )
                .map_err(|_| PrepareError::Capacity)?;
            self.ranges.push(start..start + count);
            start += count;
        }
        Ok(())
    }
    pub fn draw_range(&self, pass: &mut wgpu::RenderPass<'_>, range: Range<u32>) {
        self.core.draw_range(pass, range);
    }
    pub fn damage(&self) -> Damage {
        self.core.damage()
    }
    pub fn upload_stats(&self) -> super::stats::UploadStats {
        self.core.upload_stats()
    }
    pub(crate) fn encoded(&self) -> &[u8] {
        self.core.encoded()
    }
    pub(crate) fn binding(&self) -> &wgpu::BindGroup {
        self.core.binding()
    }
    pub fn prepared_len(&self) -> u32 {
        self.core.count()
    }
}

/// Clip-free conservative output bounds are owned by the source encoders.
pub fn region_damage(bounds: Option<Rect>) -> Damage {
    bounds.map_or(Damage::None, Damage::Region)
}
