//! What each pass costs on the GPU, from timestamp queries.
//!
//! The status line shows fps and what analysis costs; this adds the other half, which
//! is what the 3 ms frame budget is measured against. Where the GPU has no timestamp
//! queries (some OpenGL drivers, software renderers) the timer reports nothing rather
//! than guessing.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// Nothing mapped, a resolve in flight, or numbers waiting to be read.
const IDLE: u8 = 0;
const MAPPING: u8 = 1;
const READY: u8 = 2;

#[derive(Debug)]
pub struct GpuTimer {
    labels: Vec<&'static str>,
    set: Option<wgpu::QuerySet>,
    resolve: wgpu::Buffer,
    read: wgpu::Buffer,
    state: Arc<AtomicU8>,
    /// Nanoseconds per tick.
    period: f32,
    /// The last frame's times in milliseconds, one per pass.
    times: Vec<f32>,
}

impl GpuTimer {
    /// A timer for the named passes, in the order they're drawn. Cheap and inert when
    /// the device has no timestamp queries.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, labels: &[&'static str]) -> GpuTimer {
        let count = labels.len() as u32 * 2;
        let size = count as u64 * 8;
        let set = device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY)
            .then(|| {
                device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("pass timings"),
                    ty: wgpu::QueryType::Timestamp,
                    count,
                })
            });
        GpuTimer {
            labels: labels.to_vec(),
            set,
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pass timings"),
                size,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            read: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pass timings read"),
                size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            state: Arc::new(AtomicU8::new(IDLE)),
            period: queue.get_timestamp_period(),
            times: vec![0.0; labels.len()],
        }
    }

    pub fn is_available(&self) -> bool {
        self.set.is_some()
    }

    /// What to hand a render pass so it times itself, or `None` when queries are off.
    pub fn writes(&self, pass: usize) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        let set = self.set.as_ref()?;
        (pass < self.labels.len()).then(|| wgpu::RenderPassTimestampWrites {
            query_set: set,
            beginning_of_pass_write_index: Some(pass as u32 * 2),
            end_of_pass_write_index: Some(pass as u32 * 2 + 1),
        })
    }

    /// Copies the frame's timestamps out. Call once, after the last pass is encoded.
    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(set) = &self.set else { return };
        if self.state.load(Ordering::Relaxed) != IDLE {
            return; // the last frame's numbers haven't been read yet
        }
        encoder.resolve_query_set(set, 0..self.labels.len() as u32 * 2, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.read, 0, self.read.size());
    }

    /// Starts reading the resolved timestamps, and takes last frame's when they arrive.
    /// Never waits on the GPU.
    pub fn poll(&mut self) {
        if self.set.is_none() {
            return;
        }
        match self.state.load(Ordering::Acquire) {
            IDLE => {
                let state = self.state.clone();
                state.store(MAPPING, Ordering::Release);
                self.read
                    .slice(..)
                    .map_async(wgpu::MapMode::Read, move |r| {
                        state.store(if r.is_ok() { READY } else { IDLE }, Ordering::Release);
                    });
            }
            READY => {
                // The view has to go before the buffer can be unmapped.
                if let Ok(view) = self.read.slice(..).get_mapped_range() {
                    let ticks: &[u64] = bytemuck::cast_slice(&view);
                    for (i, t) in self.times.iter_mut().enumerate() {
                        let (start, end) = (ticks[i * 2], ticks[i * 2 + 1]);
                        *t = end.saturating_sub(start) as f32 * self.period / 1e6;
                    }
                }
                self.read.unmap();
                self.state.store(IDLE, Ordering::Release);
            }
            _ => {}
        }
    }

    /// The whole frame's GPU time in milliseconds.
    pub fn total_ms(&self) -> f32 {
        self.times.iter().sum()
    }

    /// Each pass and its time, for the status line: "visuals 1.2  glow 0.3".
    pub fn report(&self) -> String {
        self.labels
            .iter()
            .zip(&self.times)
            .map(|(name, ms)| format!("{name} {ms:.1}"))
            .collect::<Vec<_>>()
            .join("  ")
    }
}
