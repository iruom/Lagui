//! Retained GPU instance storage shared by Lagui renderers.
//!
//! Renderers encode every prepared instance into `staged`. `commit` compares
//! it with the bytes already resident on the GPU and uploads only changed
//! instance runs, so an unchanged or lightly edited frame costs little bus
//! traffic. The CPU mirror is the only copy of what the GPU buffer contains.
use crate::Rect;
use std::vec::Vec;

/// Unchanged instances between two dirty runs that are still uploaded with them.
/// Merging short gaps trades a few redundant bytes for fewer queue writes.
const MERGE_GAP: usize = 8;

/// Region of the previous frame that a prepare changed, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Damage {
    /// The prepared output is identical to the previous one.
    #[default]
    None,
    /// Only this region changed; content outside it is unchanged.
    Region(Rect),
    /// Everything must be redrawn (first frame, viewport or scale change, regrowth).
    Full,
}

impl Damage {
    pub fn union(self, other: Self) -> Self {
        match (self, other) {
            (Self::Full, _) | (_, Self::Full) => Self::Full,
            (Self::None, damage) | (damage, Self::None) => damage,
            (Self::Region(a), Self::Region(b)) => Self::Region(union_rect(a, b)),
        }
    }

    fn include(&mut self, rect: Option<Rect>) {
        if let Some(rect) = rect {
            *self = self.union(Self::Region(rect));
        }
    }
}

fn union_rect(a: Rect, b: Rect) -> Rect {
    let left = a.x.min(b.x);
    let top = a.y.min(b.y);
    let right = (a.x + a.width).max(b.x + b.width);
    let bottom = (a.y + a.height).max(b.y + b.height);
    Rect::new(left, top, right - left, bottom - top)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CapacityError;

pub(crate) struct InstanceBuffer {
    device: wgpu::Device,
    label: &'static str,
    stride: usize,
    buffer: wgpu::Buffer,
    capacity: usize,
    /// Bytes resident in `buffer`, or `None` after the buffer was replaced.
    resident: Option<Vec<u8>>,
    staged: Vec<u8>,
    uniforms: Option<[u8; 16]>,
    count: u32,
    damage: Damage,
}

impl InstanceBuffer {
    pub fn new(
        device: &wgpu::Device,
        label: &'static str,
        stride: usize,
        capacity: usize,
    ) -> Result<Self, CapacityError> {
        let capacity = capacity.max(1);
        let size = Self::size(device, stride, capacity)?;
        Ok(Self {
            device: device.clone(),
            label,
            stride,
            buffer: Self::create(device, label, size),
            capacity,
            resident: None,
            staged: Vec::with_capacity(capacity * stride),
            uniforms: None,
            count: 0,
            damage: Damage::Full,
        })
    }

    fn create(device: &wgpu::Device, label: &'static str, size: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Byte size for `count` instances, bounded by the device and u32 instance addressing.
    pub fn size(device: &wgpu::Device, stride: usize, count: usize) -> Result<u64, CapacityError> {
        let bytes = count.checked_mul(stride).ok_or(CapacityError)?;
        let bytes = u64::try_from(bytes).map_err(|_| CapacityError)?;
        if count > u32::MAX as usize || bytes > device.limits().max_buffer_size {
            return Err(CapacityError);
        }
        Ok(bytes)
    }

    pub fn limit(&self) -> usize {
        (self.device.limits().max_buffer_size / self.stride as u64).min(u64::from(u32::MAX))
            as usize
    }

    /// Grows storage to hold `required` instances. Resident contents are not
    /// copied; the next commit uploads every instance into the new buffer.
    pub fn reserve(&mut self, required: usize) -> Result<(), CapacityError> {
        if required <= self.capacity {
            return Ok(());
        }
        let limit = self.limit();
        if required > limit {
            return Err(CapacityError);
        }
        let capacity = required
            .checked_next_power_of_two()
            .unwrap_or(limit)
            .min(limit);
        let size = Self::size(&self.device, self.stride, capacity)?;
        self.buffer = Self::create(&self.device, self.label, size);
        self.staged
            .reserve(capacity * self.stride - self.staged.len());
        self.capacity = capacity;
        self.resident = None;
        Ok(())
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Starts encoding a new frame. The previous upload stays drawable until commit.
    pub fn begin(&mut self) -> &mut Vec<u8> {
        self.staged.clear();
        &mut self.staged
    }

    pub fn staged(&mut self) -> &mut Vec<u8> {
        &mut self.staged
    }

    pub fn staged_len(&self) -> usize {
        self.staged.len() / self.stride
    }

    /// Invalidates the prepared frame after a failed encode.
    pub fn abandon(&mut self) {
        self.staged.clear();
        self.count = 0;
        self.resident = None;
        self.damage = Damage::Full;
    }

    /// Uploads changed instance runs and the 16-byte uniform block. `bounds`
    /// returns the logical-pixel region one encoded instance can touch.
    pub fn commit(
        &mut self,
        queue: &wgpu::Queue,
        uniform: &wgpu::Buffer,
        uniforms: [u8; 16],
        bounds: impl Fn(&[u8]) -> Option<Rect>,
    ) {
        let stride = self.stride;
        let mut damage = Damage::None;
        if self.uniforms != Some(uniforms) {
            queue.write_buffer(uniform, 0, &uniforms);
            // Instances are positioned relative to the viewport and scale.
            if self.uniforms.is_some() || self.resident.is_some() {
                damage = Damage::Full;
            }
            self.uniforms = Some(uniforms);
        }
        let staged = &self.staged;
        match &self.resident {
            None => {
                if !staged.is_empty() {
                    queue.write_buffer(&self.buffer, 0, staged);
                }
                damage = Damage::Full;
            }
            Some(resident) => {
                let new_count = staged.len() / stride;
                let old_count = resident.len() / stride;
                let shared = new_count.min(old_count);
                let mut run: Option<(usize, usize)> = None;
                let flush = |range: (usize, usize)| {
                    queue.write_buffer(
                        &self.buffer,
                        (range.0 * stride) as u64,
                        &staged[range.0 * stride..range.1 * stride],
                    );
                };
                for index in 0..shared {
                    let span = index * stride..(index + 1) * stride;
                    if staged[span.clone()] == resident[span.clone()] {
                        continue;
                    }
                    damage.include(bounds(&resident[span.clone()]));
                    damage.include(bounds(&staged[span]));
                    run = match run {
                        Some((start, end)) if index - end <= MERGE_GAP => Some((start, index + 1)),
                        Some(previous) => {
                            flush(previous);
                            Some((index, index + 1))
                        }
                        None => Some((index, index + 1)),
                    };
                }
                if new_count > shared {
                    for index in shared..new_count {
                        damage.include(bounds(&staged[index * stride..(index + 1) * stride]));
                    }
                    run = match run {
                        Some((start, end)) if shared - end <= MERGE_GAP => Some((start, new_count)),
                        Some(previous) => {
                            flush(previous);
                            Some((shared, new_count))
                        }
                        None => Some((shared, new_count)),
                    };
                }
                for index in shared..old_count {
                    damage.include(bounds(&resident[index * stride..(index + 1) * stride]));
                }
                if let Some(range) = run {
                    flush(range);
                }
            }
        }
        let resident = self.resident.get_or_insert_with(Vec::new);
        resident.clear();
        resident.extend_from_slice(&self.staged);
        self.count = (self.staged.len() / stride) as u32;
        self.damage = damage;
    }

    pub fn buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    pub fn count(&self) -> u32 {
        self.count
    }

    pub fn damage(&self) -> Damage {
        self.damage
    }
}

pub(crate) fn push_f32(bytes: &mut Vec<u8>, values: &[f32]) {
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
}

pub(crate) fn push_u32(bytes: &mut Vec<u8>, values: &[u32]) {
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
}

/// Packs four values as IEEE half floats for `Float16x4` vertex attributes.
pub(crate) fn push_f16x4(bytes: &mut Vec<u8>, values: [f32; 4]) {
    for value in values {
        bytes.extend_from_slice(&f16_bits(value).to_le_bytes());
    }
}

pub(crate) fn read_f32(bytes: &[u8], index: usize) -> f32 {
    let offset = index * 4;
    f32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

/// Round-to-nearest-even f32 to binary16 conversion, including subnormals.
pub(crate) fn f16_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x7f_ffff;
    if exponent == 0xff {
        return sign | 0x7c00 | if mantissa != 0 { 0x200 } else { 0 };
    }
    let half_exponent = exponent - 127 + 15;
    if half_exponent >= 0x1f {
        return sign | 0x7c00;
    }
    if half_exponent <= 0 {
        if half_exponent < -10 {
            return sign;
        }
        let full = mantissa | 0x80_0000;
        let shift = (14 - half_exponent) as u32;
        let half = full >> shift;
        let remainder = full & ((1 << shift) - 1);
        let halfway = 1 << (shift - 1);
        let round = u32::from(remainder > halfway || (remainder == halfway && half & 1 == 1));
        return sign | (half + round) as u16;
    }
    let half = ((half_exponent as u32) << 10) | (mantissa >> 13);
    let remainder = mantissa & 0x1fff;
    let round = u32::from(remainder > 0x1000 || (remainder == 0x1000 && half & 1 == 1));
    // A rounding carry correctly advances the exponent, up to infinity.
    sign | (half + round) as u16
}

pub(crate) fn f16_value(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let mantissa = f32::from(bits & 0x3ff);
    sign * match exponent {
        0 => mantissa * (2f32).powi(-24),
        0x1f if mantissa == 0. => f32::INFINITY,
        0x1f => f32::NAN,
        _ => (1. + mantissa / 1024.) * (2f32).powi(exponent - 15),
    }
}

pub(crate) fn read_f16(bytes: &[u8], offset: usize) -> f32 {
    f16_value(u16::from_le_bytes([bytes[offset], bytes[offset + 1]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_conversion_rounds_to_nearest_even() {
        for (value, bits) in [
            (0.0, 0x0000),
            (-0.0, 0x8000),
            (1.0, 0x3c00),
            (0.5, 0x3800),
            (-2.0, 0xc000),
            (0.1, 0x2e66),
            (65504.0, 0x7bff),
            (65520.0, 0x7c00),
            (1.0e-8, 0x0000),
            (5.960_464_5e-8, 0x0001),
            (6.097_555e-5, 0x03ff),
            (f32::INFINITY, 0x7c00),
            // Exactly halfway between 1.0 and the next half: ties to even.
            (1.0 + 1.0 / 2048.0, 0x3c00),
            (1.0 + 3.0 / 2048.0, 0x3c02),
        ] {
            assert_eq!(f16_bits(value), bits, "{value}");
        }
        assert!(f16_value(f16_bits(f32::NAN)).is_nan());
        for value in [0.25f32, 0.73, 12.5, 1024.0, -3.0e-5] {
            let round_trip = f16_value(f16_bits(value));
            assert!((round_trip - value).abs() <= value.abs() / 1024., "{value}");
        }
    }

    #[test]
    fn damage_union_keeps_full_and_extends_regions() {
        let a = Damage::Region(Rect::new(0., 0., 10., 10.));
        let b = Damage::Region(Rect::new(20., 5., 5., 10.));
        assert_eq!(a.union(b), Damage::Region(Rect::new(0., 0., 25., 15.)));
        assert_eq!(a.union(Damage::None), a);
        assert_eq!(Damage::Full.union(a), Damage::Full);
    }
}
