//! The spectrogram history on the GPU.
//!
//! Rows hold levels, not colours: each is the fixed 2,048-point log grid for both panes,
//! as Float16 pairs, so changing the palette, the axis or the range redraws the whole
//! history at once. Rows live in layers of a texture array, and each row's floor and
//! ceiling at the moment it was cut sit in a small side texture, so old history keeps
//! the colours it had.
//!
//! A CPU ring beside it keeps each row's timestamp, for the time axis, the hover readout
//! and double-click-to-seek.

use half::f16;

/// A row's levels and when it was cut.
#[derive(Clone, Copy, Debug)]
pub struct RowIn<'a> {
    pub index: u64,
    /// Frames of audio analysed when the row was cut.
    pub frames: u64,
    pub floor_db: f32,
    pub ceiling_db: f32,
    /// Levels for the first and second pane, low to high frequency.
    pub a: &'a [f16],
    pub b: &'a [f16],
}

/// What the CPU keeps about each row.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RowMeta {
    pub index: u64,
    pub frames: u64,
    pub floor_db: f32,
    pub ceiling_db: f32,
}

#[derive(Debug)]
pub struct HistoryStore {
    bins: u32,
    layer_rows: u32,
    layers: u32,
    levels: wgpu::Texture,
    ranges: wgpu::Texture,
    levels_view: wgpu::TextureView,
    ranges_view: wgpu::TextureView,
    written: u64,
    meta: Vec<RowMeta>,
    staging: Vec<f16>,
}

impl HistoryStore {
    /// Room for at least `rows` rows of `bins` levels, in layers as tall as the device
    /// allows up to 4,096.
    pub fn new(device: &wgpu::Device, bins: u32, rows: u32) -> HistoryStore {
        let limits = device.limits();
        let layer_rows = 4096.min(limits.max_texture_dimension_2d).max(64);
        let layers = rows
            .div_ceil(layer_rows)
            .clamp(1, limits.max_texture_array_layers);
        let levels = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("history levels"),
            size: wgpu::Extent3d {
                width: bins,
                height: layer_rows,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let ranges = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("history ranges"),
            size: wgpu::Extent3d {
                width: layer_rows,
                height: layers,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let levels_view = levels.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let ranges_view = ranges.create_view(&wgpu::TextureViewDescriptor::default());
        let capacity = (layer_rows * layers) as usize;
        HistoryStore {
            bins,
            layer_rows,
            layers,
            levels,
            ranges,
            levels_view,
            ranges_view,
            written: 0,
            meta: vec![RowMeta::default(); capacity],
            staging: Vec::with_capacity(bins as usize * 2),
        }
    }

    pub fn bins(&self) -> u32 {
        self.bins
    }

    pub fn layer_rows(&self) -> u32 {
        self.layer_rows
    }

    pub fn capacity(&self) -> u64 {
        self.layer_rows as u64 * self.layers as u64
    }

    /// Rows pushed since creation or the last clear.
    pub fn written(&self) -> u64 {
        self.written
    }

    /// Rows that can be drawn.
    pub fn available(&self) -> u64 {
        self.written.min(self.capacity())
    }

    /// The ring slot of the newest row.
    pub fn newest_slot(&self) -> u64 {
        if self.written == 0 {
            0
        } else {
            (self.written - 1) % self.capacity()
        }
    }

    pub fn levels_view(&self) -> &wgpu::TextureView {
        &self.levels_view
    }

    pub fn ranges_view(&self) -> &wgpu::TextureView {
        &self.ranges_view
    }

    /// Forgets everything; the next row starts a new history.
    pub fn clear(&mut self) {
        self.written = 0;
    }

    /// The row `age` rows back from the newest, if it's still held.
    pub fn meta(&self, age: u64) -> Option<&RowMeta> {
        if age >= self.available() {
            return None;
        }
        let slot = (self.written - 1 - age) % self.capacity();
        self.meta.get(slot as usize)
    }

    /// Uploads one row. Only the new row crosses the bus.
    pub fn push(&mut self, queue: &wgpu::Queue, row: &RowIn<'_>) {
        let n = self.bins as usize;
        self.staging.clear();
        for i in 0..n {
            self.staging
                .push(row.a.get(i).copied().unwrap_or(f16::ZERO));
            self.staging
                .push(row.b.get(i).copied().unwrap_or(f16::ZERO));
        }
        let slot = self.written % self.capacity();
        let (layer, y) = (
            (slot / self.layer_rows as u64) as u32,
            (slot % self.layer_rows as u64) as u32,
        );
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.levels,
                mip_level: 0,
                origin: wgpu::Origin3d { x: 0, y, z: layer },
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&self.staging),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.bins * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: self.bins,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.ranges,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: y,
                    y: layer,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&[row.floor_db, row.ceiling_db]),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        self.meta[slot as usize] = RowMeta {
            index: row.index,
            frames: row.frames,
            floor_db: row.floor_db,
            ceiling_db: row.ceiling_db,
        };
        self.written += 1;
    }
}
