use crate::format::NotebookMeta;
use crate::format::rle::decode_rle;
use anyhow::{Context, Result, bail};

/// The special BGLAYER data length that indicates an all-blank background
/// when combined with style_white.
const ALL_BLANK_DATA_LEN: usize = 0x140E;

/// Render a page's RLE bitmap layers to raw RGBA pixels.
///
/// Returns `(width, height, pixels)` where `pixels` is a `Vec<u8>` of length
/// `width * height * 4` in RGBA order. Layers are composited in
/// `page.layer_order()` order (bottom-to-top) onto a white canvas.
///
/// Custom PNG backgrounds (`user_*` styles) are skipped — those require
/// the `image` crate and are handled by `render::to_image()`.
pub fn render_bitmap(
    data: &[u8],
    notebook: &NotebookMeta,
    page: usize,
) -> Result<(u32, u32, Vec<u8>)> {
    if page >= notebook.page_count() {
        bail!(
            "Page {page} out of range (notebook has {} pages)",
            notebook.page_count()
        );
    }

    let pg = notebook.page(data, page)?;
    let w = notebook.page_width;
    let h = notebook.page_height;
    let highres = notebook.supports_highres_grayscale();

    // Start with white RGBA canvas
    let pixel_count = (w as usize) * (h as usize);
    let mut rgba = vec![255u8; pixel_count * 4];

    if pg.layers.is_empty() {
        // Non-layered (A5): single bitmap
        if let Some(bmp_data) = pg.a5_bitmap(data) {
            let protocol = pg.a5_protocol().unwrap_or("RATTA_RLE");
            if protocol == "RATTA_RLE" {
                let pixels =
                    decode_rle(bmp_data, w, h, false, highres).context("decoding A5 RLE bitmap")?;
                composite_grayscale(&mut rgba, &pixels);
            }
        }
    } else {
        // Layered (X-series): composite layers per LAYERSEQ
        let layer_order = pg.layer_order();

        // LAYERSEQ lists layers top-to-bottom, so iterate in reverse
        // to composite bottom-to-top.
        for layer_name in layer_order.iter().rev() {
            let layer_idx = match pg.layers.iter().position(|l| l.name == *layer_name) {
                Some(i) => i,
                None => continue,
            };
            let layer = &pg.layers[layer_idx];

            let bmp_data = match pg.layer_bitmap(data, layer_idx) {
                Some(d) if !d.is_empty() => d,
                _ => continue,
            };

            // Skip custom PNG backgrounds — they need the image crate
            if pg.style.starts_with("user_") && layer.name == "BGLAYER" {
                continue;
            }

            let all_blank = layer.name == "BGLAYER"
                && pg.style == "style_white"
                && bmp_data.len() == ALL_BLANK_DATA_LEN;

            if layer.protocol == "RATTA_RLE" {
                let pixels = decode_rle(bmp_data, w, h, all_blank, highres)
                    .with_context(|| format!("decoding RLE for layer {}", layer.name))?;
                composite_grayscale(&mut rgba, &pixels);
            }
        }
    }

    Ok((w, h, rgba))
}

/// Composite a grayscale pixel buffer onto an RGBA canvas.
/// Pixels with value 0xFF (transparent) are skipped.
fn composite_grayscale(rgba: &mut [u8], pixels: &[u8]) {
    for (idx, &gray) in pixels.iter().enumerate() {
        if gray == 0xFF {
            continue;
        }
        let base = idx * 4;
        if base + 3 >= rgba.len() {
            return;
        }
        rgba[base] = gray;
        rgba[base + 1] = gray;
        rgba[base + 2] = gray;
        rgba[base + 3] = 255;
    }
}
