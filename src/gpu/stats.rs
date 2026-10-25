//! Frame measurements. Timestamp readback never waits for the GPU.
use std::{sync::mpsc, vec::Vec};

/// Queue traffic for the most recent prepare, including uniforms.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UploadStats {
    pub bytes: u64,
    pub writes: u64,
    pub instances: u32,
    pub diff_upload_us: f64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DrawStats {
    pub draw_calls: u64,
    pub pipeline_switches: u64,
    previous: Option<u64>,
}
impl DrawStats {
    /// Call only when an actual nonempty draw is issued. Start a fresh value per pass.
    pub fn record(&mut self, pipeline: u64) {
        self.draw_calls += 1;
        if self.previous != Some(pipeline) {
            self.pipeline_switches += 1;
        }
        self.previous = Some(pipeline);
    }
}

/// Host-owned measurements for one frame. GPU samples arrive in later frames.
#[derive(Clone, Debug, Default)]
pub struct FrameStats {
    pub encode_us: f64,
    pub diff_upload_us: f64,
    pub upload: UploadStats,
    pub culled: u32,
    pub instances_by_kind: Vec<(&'static str, u32)>,
    pub draw_calls: u64,
    pub pipeline_switches: u64,
    pub damage_ratio: f32,
    pub gpu_pass_ms: Vec<f64>,
    pub acquire_wait_us: f64,
    pub submit_us: f64,
    pub present_us: f64,
    pub present_mode: Option<wgpu::PresentMode>,
}

struct Slot {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    read: wgpu::Buffer,
    pending: Option<mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>>,
    recording: bool,
}

/// One pass per sample; create separate timers to name multiple passes.
/// Call `begin`, put `writes` on a pass, `resolve`, submit, then `submitted`.
/// A busy ring skips a sample rather than blocking or overwriting a mapping.
pub struct GpuTimer {
    slots: Vec<Slot>,
    cursor: usize,
    interval: u32,
    frame: u64,
    period: f64,
    active: bool,
}

impl GpuTimer {
    /// Release an unsubmitted recording after an aborted host frame.
    pub fn cancel(&mut self) {
        if self.active {
            self.slots[self.cursor].recording = false;
            self.active = false;
        }
    }
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frames_in_flight: usize,
        interval: u32,
    ) -> Self {
        let enabled = interval != 0 && device.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let slots = if enabled {
            (0..frames_in_flight.saturating_add(1).max(2))
                .map(|_| Slot {
                    queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                        label: Some("Lagui pass timestamps"),
                        ty: wgpu::QueryType::Timestamp,
                        count: 2,
                    }),
                    resolve: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Lagui timestamp resolve"),
                        size: wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT,
                        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                        mapped_at_creation: false,
                    }),
                    read: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("Lagui timestamp readback"),
                        size: 16,
                        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                        mapped_at_creation: false,
                    }),
                    pending: None,
                    recording: false,
                })
                .collect()
        } else {
            Vec::new()
        };
        Self {
            slots,
            cursor: 0,
            interval,
            frame: 0,
            period: f64::from(queue.get_timestamp_period()),
            active: false,
        }
    }

    /// Poll completed mappings; the host drives device polling without waiting.
    pub fn collect(&mut self, samples: &mut Vec<f64>) {
        for slot in &mut self.slots {
            let Some(receiver) = &slot.pending else {
                continue;
            };
            match receiver.try_recv() {
                Err(mpsc::TryRecvError::Empty) => continue,
                Ok(Ok(())) => {
                    if let Ok(bytes) = slot.read.slice(..).get_mapped_range() {
                        let start = u64::from_le_bytes(bytes[..8].try_into().unwrap());
                        let end = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
                        if end >= start {
                            samples.push((end - start) as f64 * self.period / 1e6);
                        }
                    }
                }
                _ => {}
            }
            slot.read.unmap();
            slot.pending = None;
        }
    }

    pub fn begin(&mut self) -> bool {
        self.active = false;
        let sample = self.interval != 0 && self.frame.is_multiple_of(u64::from(self.interval));
        self.frame = self.frame.wrapping_add(1);
        if !sample || self.slots.is_empty() {
            return false;
        }
        if let Some(index) = (0..self.slots.len())
            .map(|n| (self.cursor + n) % self.slots.len())
            .find(|&i| self.slots[i].pending.is_none() && !self.slots[i].recording)
        {
            self.cursor = index;
            self.slots[index].recording = true;
            self.active = true;
        }
        self.active
    }

    pub fn writes(&self) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        self.active.then(|| wgpu::RenderPassTimestampWrites {
            query_set: &self.slots[self.cursor].queries,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: Some(1),
        })
    }

    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        if !self.active {
            return;
        }
        let slot = &self.slots[self.cursor];
        encoder.resolve_query_set(&slot.queries, 0..2, &slot.resolve, 0);
        encoder.copy_buffer_to_buffer(&slot.resolve, 0, &slot.read, 0, 16);
    }

    /// Must follow submission of the encoder containing `resolve`.
    pub fn submitted(&mut self) {
        if !self.active {
            return;
        }
        let slot = &mut self.slots[self.cursor];
        let (send, receive) = mpsc::channel();
        slot.read
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        slot.pending = Some(receive);
        slot.recording = false;
        self.active = false;
    }
}
