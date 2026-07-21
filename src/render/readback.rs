//! Texture readback for offscreen stills. Native only — it blocks on the map.

use anyhow::{Context, Result};

/// Read an Rgba8 texture back to tightly-packed RGBA bytes (row 0 = top).
pub fn read_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    tex: &wgpu::Texture,
) -> Result<(u32, u32, Vec<u8>)> {
    let (w, h) = (tex.width(), tex.height());
    let bpr = w * 4;
    let padded_bpr =
        bpr.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: padded_bpr as u64 * h as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("readback") });
    encoder.copy_texture_to_buffer(
        tex.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bpr),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    queue.submit([encoder.finish()]);

    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device.poll(wgpu::PollType::wait_indefinitely()).context("poll failed")?;
    rx.recv().context("map channel closed")?.context("map failed")?;

    let data = slice.get_mapped_range();
    let mut out = Vec::with_capacity((bpr * h) as usize);
    for row in 0..h {
        let start = (row * padded_bpr) as usize;
        out.extend_from_slice(&data[start..start + bpr as usize]);
    }
    drop(data);
    buffer.unmap();
    Ok((w, h, out))
}

pub fn save_png(path: &std::path::Path, w: u32, h: u32, rgba: &[u8]) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(rgba)?;
    Ok(())
}
