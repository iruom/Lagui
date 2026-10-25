use super::{PrepareError, instances};
use crate::transform::TransformEntry;
pub use crate::transform::{Transform2D, TransformId};
use std::vec::Vec;

/// Layouts shared by all UI pipelines on one device.
#[derive(Clone)]
pub struct Shared {
    pub(crate) transforms: wgpu::BindGroupLayout,
    pub(crate) identity: wgpu::BindGroup,
}
impl Shared {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Lagui transforms"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(32),
                },
                count: None,
            }],
        });
        use wgpu::util::DeviceExt;
        let mut bytes = Vec::new();
        instances::push_f32(&mut bytes, &[0., 0., 1., 0., -1e20, -1e20, 2e20, 2e20]);
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Lagui identity transform"),
            contents: &bytes,
            usage: wgpu::BufferUsages::STORAGE,
        });
        let identity = binding(device, &layout, &buffer);
        Self {
            transforms: layout,
            identity,
        }
    }
}

fn binding(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Lagui transform table"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    })
}

/// Host-owned presentation table. Call `begin`, `push`, `commit` before prepare.
/// Commit once per frame; every renderer sees the same old and new transforms.
pub struct Transforms {
    device: wgpu::Device,
    shared: Shared,
    current: Vec<TransformEntry>,
    previous: Vec<TransformEntry>,
    resident: Vec<TransformEntry>,
    buffer: wgpu::Buffer,
    pub(crate) binding: wgpu::BindGroup,
    capacity: usize,
    committed: bool,
    pub uploaded_bytes: u64,
}
impl Transforms {
    pub fn new(device: &wgpu::Device, shared: &Shared) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Lagui transforms"),
            size: 32,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let binding = binding(device, &shared.transforms, &buffer);
        Self {
            device: device.clone(),
            shared: shared.clone(),
            current: std::vec![TransformEntry::IDENTITY],
            previous: Vec::new(),
            resident: Vec::new(),
            buffer,
            binding,
            capacity: 1,
            committed: false,
            uploaded_bytes: 0,
        }
    }
    pub fn begin(&mut self) {
        self.committed = false;
        self.current.clear();
        self.current.push(TransformEntry::IDENTITY);
    }
    pub fn push(&mut self, transform: Transform2D) -> Result<TransformId, PrepareError> {
        if !transform.valid() {
            return Err(PrepareError::InvalidViewport);
        }
        let limit = self
            .device
            .limits()
            .max_storage_buffer_binding_size
            .min(self.device.limits().max_buffer_size)
            / 32;
        if self.current.len() as u64 >= limit || self.current.len() >= u32::MAX as usize {
            return Err(PrepareError::Capacity);
        }
        let id = TransformId(self.current.len() as u32);
        self.committed = false;
        self.current.push(TransformEntry {
            transform,
            ..TransformEntry::IDENTITY
        });
        Ok(id)
    }
    pub fn get(&self, id: TransformId) -> Option<Transform2D> {
        if !self.committed {
            return None;
        }
        self.current.get(id.0 as usize).map(|e| e.transform)
    }
    pub fn previous(&self, id: TransformId) -> Option<Transform2D> {
        self.previous.get(id.0 as usize).map(|e| e.transform)
    }
    #[cfg(feature = "alloc")]
    pub fn replace(&mut self, list: &crate::transform::TransformList) -> Result<(), PrepareError> {
        let limit = self
            .device
            .limits()
            .max_buffer_size
            .min(self.device.limits().max_storage_buffer_binding_size)
            / 32;
        if list.entries.len() as u64 > limit {
            return Err(PrepareError::Capacity);
        }
        self.current.clone_from(&list.entries);
        self.committed = false;
        Ok(())
    }
    pub fn find(&self, transform: Transform2D) -> Option<TransformId> {
        self.current
            .iter()
            .position(|e| !e.local_clip && e.transform == transform)
            .map(|i| TransformId(i as u32))
    }
    pub fn guard_clip(&self, id: TransformId) -> Option<crate::Rect> {
        self.current
            .get(id.0 as usize)
            .filter(|e| e.local_clip)
            .map(|e| e.screen_clip)
    }
    pub fn clip(&self, id: TransformId, clip: crate::Rect, old: bool) -> Option<crate::Rect> {
        let entries = if old { &self.previous } else { &self.current };
        entries.get(id.0 as usize).and_then(|e| e.clip(clip))
    }
    pub fn changed(&self) -> impl Iterator<Item = TransformId> + '_ {
        (0..self.current.len().max(self.previous.len()))
            .filter(|&i| self.current.get(i) != self.previous.get(i))
            .map(|i| TransformId(i as u32))
    }
    pub fn commit(&mut self, queue: &wgpu::Queue) {
        self.previous.clone_from(&self.resident);
        self.uploaded_bytes = 0;
        if self.current.len() > self.capacity {
            let limit = self
                .device
                .limits()
                .max_storage_buffer_binding_size
                .min(self.device.limits().max_buffer_size)
                / 32;
            self.capacity = self.current.len().next_power_of_two().min(limit as usize);
            self.buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Lagui transforms"),
                size: self.capacity as u64 * 32,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.binding = binding(&self.device, &self.shared.transforms, &self.buffer);
            self.resident.clear();
        }
        for (i, t) in self.current.iter().enumerate() {
            if self.resident.get(i) == Some(t) {
                continue;
            }
            let mut bytes = [0; 32];
            for (chunk, value) in bytes.chunks_exact_mut(4).zip([
                t.transform.offset[0],
                t.transform.offset[1],
                t.transform.scale,
                if t.local_clip { 1. } else { 0. },
                t.screen_clip.x,
                t.screen_clip.y,
                t.screen_clip.width,
                t.screen_clip.height,
            ]) {
                chunk.copy_from_slice(&value.to_le_bytes());
            }
            queue.write_buffer(&self.buffer, i as u64 * 32, &bytes);
            self.uploaded_bytes += 32;
        }
        self.resident.clone_from(&self.current);
        self.committed = true;
    }
    pub fn bind(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_bind_group(1, &self.binding, &[]);
    }
}
