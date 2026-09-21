//! Reading a rendered frame back from the GPU and saving it as a PNG: for checking the
//! renderer without capturing the screen, and for golden images.

use std::path::Path;

/// A copy of a texture on its way to the CPU.
#[derive(Debug)]
pub struct Readback {
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    padded_row: u32,
    bgra: bool,
}

impl Readback {
    /// Records a copy of `texture` (8-bit RGBA or BGRA, created with `COPY_SRC`) into
    /// `encoder`. Submit the encoder, then [`Readback::pixels`] or [`Readback::save`].
    pub fn record(
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
    ) -> Readback {
        let (width, height) = (texture.width(), texture.height());
        let padded_row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: padded_row as u64 * height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: None,
                },
            },
            texture.size(),
        );
        let bgra = matches!(
            texture.format(),
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        Readback {
            buffer,
            width,
            height,
            padded_row,
            bgra,
        }
    }

    /// Waits for the copy and returns tightly packed RGBA rows, top first.
    pub fn pixels(&self, device: &wgpu::Device) -> Result<(u32, u32, Vec<u8>), String> {
        let slice = self.buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| format!("waiting for the GPU: {e}"))?;
        rx.recv()
            .map_err(|_| "the copy never finished".to_owned())?
            .map_err(|e| format!("cannot read the frame back: {e}"))?;
        let mapped = slice
            .get_mapped_range()
            .map_err(|e| format!("cannot read the frame back: {e}"))?;
        let mut rgba = Vec::with_capacity((self.width * self.height * 4) as usize);
        for row in mapped.chunks(self.padded_row as usize) {
            for px in row[..(self.width * 4) as usize].as_chunks::<4>().0 {
                if self.bgra {
                    rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
                } else {
                    rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
                }
            }
        }
        drop(mapped);
        self.buffer.unmap();
        Ok((self.width, self.height, rgba))
    }

    /// Waits for the copy and writes it to `path` as a PNG.
    pub fn save(&self, device: &wgpu::Device, path: &Path) -> Result<(), String> {
        let (width, height, rgba) = self.pixels(device)?;
        write_png(path, width, height, &rgba)
    }
}

/// Writes RGBA rows to `path` as an 8-bit PNG.
pub fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let bytes = encode_png(width, height, rgba).map_err(|e| format!("{}: {e}", path.display()))?;
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// The same PNG as a block of bytes, for callers with somewhere else to put it: the
/// icon packs several of these into a Windows `.ico` without any of them being a file.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(bytes)
}
