//! The launcher's pixel art, drawn here rather than shipped: the night sky behind every page is
//! `sky.rs`, skins and the grass block are baked from the player's own textures. GPUI samples
//! images smoothly, so every image is baked at the window's scale with each texel a whole block of
//! device pixels, and drawn at exactly its size: the edges stay sharp.
use gpui::RenderImage;
use std::sync::Arc;

/// `texels` (RGBA, row by row) as an image whose texels are `texel` logical pixels square at
/// `scale`. Texel edges fall on whole device pixels where the scale allows (1×, 2×, 3×).
pub fn bake(texels: &[[u8; 4]], w: usize, h: usize, texel: f32, scale: f32) -> Arc<RenderImage> {
    let size = |n: usize| ((n as f32 * texel * scale).round() as u32).max(1);
    let (dw, dh) = (size(w), size(h));
    let mut buf = image::RgbaImage::new(dw, dh);
    for (y, row) in buf.rows_mut().enumerate() {
        let ty = (y * h / dh as usize).min(h - 1);
        for (x, px) in row.enumerate() {
            let tx = (x * w / dw as usize).min(w - 1);
            let [r, g, b, a] = texels[ty * w + tx];
            // GPUI wants BGRA, its alpha premultiplied.
            let pre = |c: u8| (c as u32 * a as u32 / 255) as u8;
            *px = image::Rgba([pre(b), pre(g), pre(r), a]);
        }
    }
    Arc::new(RenderImage::new([image::Frame::new(buf)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texels_cover_whole_device_pixels() {
        let image = bake(&[[0x11, 0x22, 0x33, 255]; 4], 2, 2, 4., 2.);
        let size = image.size(0);
        assert_eq!((size.width.0, size.height.0), (16, 16));
    }
}
