use crate::format::NotebookMeta;
use crate::format::stroke::{Stroke, TextBox};
use anyhow::{Context, Result, bail};
use image::RgbaImage;

/// Render a page as an RGBA image from the RLE-encoded bitmap layers.
pub fn to_image(data: &[u8], notebook: &NotebookMeta, page: usize) -> Result<RgbaImage> {
    if page >= notebook.page_count() {
        bail!(
            "Page {page} out of range (notebook has {} pages)",
            notebook.page_count()
        );
    }

    let pg = notebook.page(data, page)?;
    let w = notebook.page_width;
    let h = notebook.page_height;

    // Use the lightweight bitmap renderer for standard RLE compositing
    let (_, _, rgba) = crate::bitmap::render_bitmap(data, notebook, page)?;
    let mut img = RgbaImage::from_raw(w, h, rgba).expect("RGBA buffer has correct size");

    // Handle custom PNG backgrounds that bitmap.rs skips
    if pg.style.starts_with("user_") {
        let layer_order = pg.layer_order();
        for layer_name in layer_order.iter().rev() {
            let layer_idx = match pg.layers.iter().position(|l| l.name == *layer_name) {
                Some(i) => i,
                None => continue,
            };
            let layer = &pg.layers[layer_idx];
            if layer.name == "BGLAYER"
                && let Some(bmp_data) = pg.layer_bitmap(data, layer_idx)
                && !bmp_data.is_empty()
            {
                let png_img = image::load_from_memory(bmp_data)
                    .context("decoding custom PNG background")?
                    .to_rgba8();
                composite_rgba_onto(&mut img, &png_img);
            }
        }
    }

    Ok(img)
}

/// Composite an RGBA image onto the target, using alpha blending.
fn composite_rgba_onto(target: &mut RgbaImage, src: &RgbaImage) {
    let w = target.width().min(src.width());
    let h = target.height().min(src.height());
    for y in 0..h {
        for x in 0..w {
            let sp = src.get_pixel(x, y);
            if sp[3] == 0 {
                continue; // fully transparent
            }
            if sp[3] == 255 {
                target.put_pixel(x, y, *sp);
            } else {
                let tp = target.get_pixel(x, y);
                let alpha = sp[3] as f32 / 255.0;
                let inv = 1.0 - alpha;
                let r = (sp[0] as f32 * alpha + tp[0] as f32 * inv) as u8;
                let g = (sp[1] as f32 * alpha + tp[1] as f32 * inv) as u8;
                let b = (sp[2] as f32 * alpha + tp[2] as f32 * inv) as u8;
                let a = (sp[3] as f32 + tp[3] as f32 * inv).min(255.0) as u8;
                target.put_pixel(x, y, image::Rgba([r, g, b, a]));
            }
        }
    }
}

/// Map a stroke's `stroke_layer` field to the corresponding layer key
/// used in LAYERSEQ and the page metadata.
fn stroke_layer_name(stroke_layer: u32) -> &'static str {
    match stroke_layer {
        0 => "MAINLAYER",
        1 => "LAYER1",
        2 => "LAYER2",
        3 => "LAYER3",
        _ => "MAINLAYER",
    }
}

/// Render a page as SVG with individual strokes as separate path elements.
///
/// The outer SVG uses pixel coordinates (e.g. 1404x1872 for Nomad) as
/// the viewBox, with an inner SVG element that maps physical coordinates
/// (e.g. 11864x15819) to the pixel space. Each stroke becomes one or more
/// `<path>` elements. Eraser strokes are rendered via SVG `<mask>` elements
/// to correctly occlude earlier pen strokes while preserving z-order.
///
/// Strokes are grouped by layer and composited in LAYERSEQ order (bottom
/// to top). Each layer is rendered independently — erasers only affect
/// strokes within the same layer, and upper layers opaquely cover lower
/// ones where they have content.
pub fn to_svg(data: &[u8], notebook: &NotebookMeta, page: usize) -> Result<String> {
    use std::fmt::Write;

    if page >= notebook.page_count() {
        bail!(
            "Page {page} out of range (notebook has {} pages)",
            notebook.page_count()
        );
    }

    let pg = notebook.page(data, page)?;
    let (strokes, text_boxes) = pg.decode_strokes(data)?;

    // Physical dimensions from the first stroke, falling back to Nomad defaults.
    // The stroke header contains screen_width/screen_height in physical units.
    let (phys_w, phys_h) = strokes
        .first()
        .map(|s| (s.screen_width as i64, s.screen_height as i64))
        .unwrap_or((11864, 15819));

    let mut svg = String::with_capacity(64 * 1024);

    let pixel_w = notebook.page_width;
    let pixel_h = notebook.page_height;

    // Header: outer SVG in pixel coordinates, inner SVG maps physical coords
    write!(
        svg,
        r#"<svg viewBox="0 0 {pixel_w} {pixel_h}" xmlns="http://www.w3.org/2000/svg">
<rect width="{pixel_w}" height="{pixel_h}" fill="white"/>
<svg viewBox="0 0 {phys_w} {phys_h}">
<g stroke-linejoin="round" stroke-linecap="round" stroke="black" fill="none">
"#
    )?;

    // Get layer rendering order. LAYERSEQ is top-to-bottom; we render
    // bottom-to-top so that upper layers paint over lower ones.
    let layer_order = pg.layer_order();

    // Group strokes by layer, preserving draw order within each layer.
    let mut strokes_by_layer: std::collections::HashMap<&str, Vec<&Stroke>> =
        std::collections::HashMap::new();
    for stroke in &strokes {
        strokes_by_layer
            .entry(stroke_layer_name(stroke.stroke_layer))
            .or_default()
            .push(stroke);
    }

    let mut mask_count: u32 = 0;

    // Render each layer bottom-to-top. BGLAYER has no strokes (it's the
    // background pattern rendered from bitmap data, not stroke data).
    for layer_name in layer_order.iter().rev() {
        let strokes = match strokes_by_layer.get(layer_name.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };

        render_layer_strokes(&mut svg, strokes, phys_w, phys_h, pixel_w, &mut mask_count);
    }

    svg.push_str("</g>\n</svg>\n");

    // Render text boxes in pixel coordinates (outside the inner physical-coord SVG)
    for tb in &text_boxes {
        render_text_box(&mut svg, tb);
    }

    svg.push_str("</svg>\n");
    Ok(svg)
}

/// Render a subset of strokes and textboxes as SVG, optionally with a cropped viewport.
///
/// This is used for extracting individual rectangles or filtered content. The viewport
/// can be specified in raw physical coordinates (y, x order), with an optional margin.
/// Strokes and textboxes are filtered by index sets.
///
/// # Arguments
/// * `data` - The raw .note file bytes
/// * `notebook` - The notebook skeleton
/// * `page` - The page index
/// * `stroke_indices` - Set of stroke indices to include (None = include all)
/// * `textbox_indices` - Set of textbox indices to include (None = include all)
/// * `viewport` - Optional viewport: (min_y, min_x, max_y, max_x, margin_mm)
pub fn to_svg_subset(
    data: &[u8],
    notebook: &NotebookMeta,
    page: usize,
    stroke_indices: Option<&std::collections::HashSet<usize>>,
    textbox_indices: Option<&std::collections::HashSet<usize>>,
    viewport: Option<(i32, i32, i32, i32, f32)>,
) -> Result<String> {
    use std::fmt::Write;

    if page >= notebook.page_count() {
        bail!(
            "Page {page} out of range (notebook has {} pages)",
            notebook.page_count()
        );
    }

    let pg = notebook.page(data, page)?;
    let (strokes, text_boxes) = pg.decode_strokes(data)?;

    // Physical dimensions from the first stroke, falling back to Nomad defaults
    let (phys_w, phys_h) = strokes
        .first()
        .map(|s| (s.screen_width as i64, s.screen_height as i64))
        .unwrap_or((11864, 15819));

    let pixel_w = notebook.page_width;
    let pixel_h = notebook.page_height;

    // Determine viewport and pixel dimensions
    let (view_min_y, view_min_x, view_max_y, view_max_x, out_pixel_w, out_pixel_h) =
        if let Some((min_y, min_x, max_y, max_x, margin_mm)) = viewport {
            // Apply margin (in mm, convert to physical units: 1mm = 100 units)
            let margin = (margin_mm * 100.0) as i32;
            let vmin_y = (min_y - margin).max(0);
            let vmin_x = (min_x - margin).max(0);
            let vmax_y = (max_y + margin).min(phys_h as i32);
            let vmax_x = (max_x + margin).min(phys_w as i32);

            let view_w = vmax_x - vmin_x;
            let view_h = vmax_y - vmin_y;

            // Compute proportional pixel dimensions
            let out_w = (view_w as f64 * pixel_w as f64 / phys_w as f64).round() as u32;
            let out_h = (view_h as f64 * pixel_h as f64 / phys_h as f64).round() as u32;

            (vmin_y, vmin_x, vmax_y, vmax_x, out_w, out_h)
        } else {
            // Full page viewport
            (0, 0, phys_h as i32, phys_w as i32, pixel_w, pixel_h)
        };

    let view_w = view_max_x - view_min_x;
    let view_h = view_max_y - view_min_y;

    // Inner SVG viewBox accounts for x-mirroring
    let inner_x = phys_w - view_max_x as i64;
    let inner_y = view_min_y;
    let inner_w = view_w;
    let inner_h = view_h;

    let mut svg = String::with_capacity(64 * 1024);

    // Header: outer SVG in pixel coordinates, inner SVG maps physical coords
    write!(
        svg,
        r#"<svg viewBox="0 0 {out_pixel_w} {out_pixel_h}" xmlns="http://www.w3.org/2000/svg">
<rect width="{out_pixel_w}" height="{out_pixel_h}" fill="white"/>
<svg viewBox="{inner_x} {inner_y} {inner_w} {inner_h}">
<g stroke-linejoin="round" stroke-linecap="round" stroke="black" fill="none">
"#
    )?;

    // Filter strokes by indices
    let filtered_strokes: Vec<&Stroke> = strokes
        .iter()
        .enumerate()
        .filter(|(i, _)| stroke_indices.is_none_or(|set| set.contains(i)))
        .map(|(_, s)| s)
        .collect();

    // Group filtered strokes by layer
    let layer_order = pg.layer_order();
    let mut strokes_by_layer: std::collections::HashMap<&str, Vec<&Stroke>> =
        std::collections::HashMap::new();
    for stroke in &filtered_strokes {
        strokes_by_layer
            .entry(stroke_layer_name(stroke.stroke_layer))
            .or_default()
            .push(stroke);
    }

    let mut mask_count: u32 = 0;

    // Render each layer bottom-to-top
    for layer_name in layer_order.iter().rev() {
        let strokes = match strokes_by_layer.get(layer_name.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };

        render_layer_strokes(&mut svg, strokes, phys_w, phys_h, pixel_w, &mut mask_count);
    }

    svg.push_str("</g>\n</svg>\n");

    // Render filtered text boxes in pixel coordinates
    for (i, tb) in text_boxes.iter().enumerate() {
        if textbox_indices.is_none_or(|set| set.contains(&i)) {
            render_text_box(&mut svg, tb);
        }
    }

    svg.push_str("</svg>\n");
    Ok(svg)
}

/// Render all strokes for a single layer into the SVG string.
///
/// Handles eraser grouping within the layer: pen strokes between eraser
/// transitions are grouped and masked by the preceding erasers.
fn render_layer_strokes(
    svg: &mut String,
    strokes: &[&Stroke],
    phys_w: i64,
    phys_h: i64,
    pixel_w: u32,
    mask_count: &mut u32,
) {
    use crate::format::stroke::{COLOR_ERASER, PEN_MARKER, PEN_NEEDLE_POINT};
    use std::fmt::Write;

    let mut pen_paths: Vec<String> = Vec::new();
    let mut eraser_paths: Vec<String> = Vec::new();
    let mut prev_was_eraser = false;

    for stroke in strokes {
        if stroke.points.len() <= 1 {
            continue;
        }

        let is_eraser = stroke.color == COLOR_ERASER;

        if !is_eraser && prev_was_eraser {
            // Transition from eraser back to pen: flush the current group
            flush_group(
                svg,
                &mut pen_paths,
                &mut eraser_paths,
                mask_count,
                phys_w,
                phys_h,
            );
        }

        match (stroke.pen, is_eraser) {
            // Eraser strokes (any pen type with eraser color) — pressure-sensitive
            (_, true) => {
                prev_was_eraser = true;
                render_pressure_stroke(stroke, phys_w, &mut eraser_paths);
            }
            // NeedlePoint or Marker — uniform width, no pressure.
            // Pen IDs 0 and 2 are legacy equivalents from older firmware.
            (PEN_NEEDLE_POINT | PEN_MARKER | 0 | 2, false) => {
                prev_was_eraser = false;
                // Minimum width of ~1 pixel so thin strokes stay visible
                let min_width = (phys_w as f32 / pixel_w as f32).ceil() as u32;
                // Non-linear thickness mapping (0.21 × t^0.89), empirically
                // fitted to match the device bitmap. Both NeedlePoint and
                // Marker share the same curve.
                let (power, coeff) = (0.89_f32, 0.21_f32);
                let width = ((stroke.thickness as f32).powf(power) * coeff)
                    .round()
                    .max(min_width as f32) as u32;

                // Both NeedlePoint and Marker use a stroked path with round
                // linecaps. The round caps give natural circles at endpoints.
                let mut d = String::new();
                write!(d, "M").unwrap();
                for pt in &stroke.points {
                    let x = phys_w - pt.x as i64;
                    let y = pt.y as i64;
                    write!(d, " {x},{y}").unwrap();
                }
                let color_attr = color_attribute(stroke.color);
                let is_marker = stroke.pen == PEN_MARKER || stroke.pen == 2;
                // Marker uses darken blend mode so overlapping strokes merge
                // (highlighter behavior). White marker (color 254) is opaque.
                let blend_attr = if is_marker && stroke.color != 254 {
                    r#" style="mix-blend-mode:darken""#
                } else {
                    ""
                };
                pen_paths.push(format!(
                    r#"<path stroke-width="{width}"{color_attr}{blend_attr} d="{d}"/>"#
                ));

                // Marker: the device overlays a filled square at each end of
                // the stroke, oriented in the movement direction. This turns
                // the round linecap circles into flat/square ends. We add
                // these as separate filled rectangles.
                if is_marker && stroke.points.len() >= 2 {
                    let fill_attr = fill_attribute(stroke.color);
                    render_marker_end_caps(
                        stroke,
                        phys_w,
                        width,
                        &fill_attr,
                        blend_attr,
                        &mut pen_paths,
                    );
                }
            }
            // InkPen and any other pen types — pressure-sensitive.
            // Rendered as filled variable-width polygons for smooth
            // pressure transitions (no visible width steps at joints).
            (_, false) => {
                prev_was_eraser = false;
                let fill_attr = fill_attribute(stroke.color);
                render_filled_pressure_stroke(stroke, phys_w, &fill_attr, &mut pen_paths);
            }
        }
    }

    // Flush any remaining strokes in this layer
    flush_group(
        svg,
        &mut pen_paths,
        &mut eraser_paths,
        mask_count,
        phys_w,
        phys_h,
    );
}

/// Render a pressure-sensitive stroke into path segments, splitting when
/// stroke width changes. Uses relative coordinates after the initial move.
fn render_pressure_stroke(stroke: &Stroke, phys_w: i64, paths: &mut Vec<String>) {
    render_pressure_stroke_colored(stroke, phys_w, "", paths);
}

fn render_pressure_stroke_colored(
    stroke: &Stroke,
    phys_w: i64,
    color_attr: &str,
    paths: &mut Vec<String>,
) {
    use std::fmt::Write;

    let first_pt = &stroke.points[0];
    let mut seg_x = phys_w - first_pt.x as i64;
    let mut seg_y = first_pt.y as i64;
    let mut d = format!("M {seg_x},{seg_y}");

    let first_pressure = stroke.pressures.first().copied().unwrap_or(0);
    let mut prev_width = pressure_width(stroke.thickness, first_pressure);

    for i in 1..stroke.points.len() {
        let pressure = stroke
            .pressures
            .get(i - 1)
            .copied()
            .unwrap_or(first_pressure);
        let cur_width = pressure_width(stroke.thickness, pressure);

        if cur_width != prev_width && !d.is_empty() {
            // Width changed — emit the accumulated segment
            paths.push(format!(
                r#"<path stroke-width="{prev_width}"{color_attr} d="{d}"/>"#
            ));

            // Start new segment from the previous point
            let prev_pt = &stroke.points[i - 1];
            seg_x = phys_w - prev_pt.x as i64;
            seg_y = prev_pt.y as i64;
            d = format!("M {seg_x},{seg_y}");
        }

        prev_width = cur_width;

        let pt = &stroke.points[i];
        let x = phys_w - pt.x as i64;
        let y = pt.y as i64;
        // Relative move from current position
        let dx = x - seg_x;
        let dy = y - seg_y;
        write!(d, " l {dx},{dy}").unwrap();
        seg_x = x;
        seg_y = y;
    }

    if d.len() > 2 {
        paths.push(format!(
            r#"<path stroke-width="{prev_width}"{color_attr} d="{d}"/>"#
        ));
    }
}

fn pressure_width(thickness: u32, pressure: u16) -> u32 {
    pressure_width_f(thickness, pressure).round() as u32
}

/// Non-linear pressure and thickness curves for InkPen strokes.
///
/// Pressure: pow(0.55) boosts low-pressure values so stroke beginnings
/// and endings remain visible while keeping high-pressure values the same.
///
/// Thickness: pow(0.63) with coefficient 1.5 maps device thickness units
/// to physical SVG coordinates. The sub-linear scaling matches the device
/// bitmap where doubling thickness does not double stroke width.
fn pressure_width_f(thickness: u32, pressure: u16) -> f32 {
    let linear = (pressure.min(2048) as f32) / 2048.0;
    let modifier = linear.powf(0.55);
    let width = (thickness as f32).powf(0.63) * 1.5 * modifier;
    width.max(1.0)
}

/// Render a pressure-sensitive stroke as a filled variable-width polygon.
///
/// Instead of splitting a stroke into constant-width segments (which creates
/// visible steps at width transitions), this builds a filled outline that
/// smoothly follows the pressure envelope. The result is a single `<path>`
/// with fill and no stroke, giving smooth, natural-looking pressure transitions.
fn render_filled_pressure_stroke(
    stroke: &Stroke,
    phys_w: i64,
    fill_attr: &str,
    paths: &mut Vec<String>,
) {
    use std::fmt::Write;

    let n = stroke.points.len();
    if n < 2 {
        return;
    }

    // Transform to SVG coordinates and compute per-point half-widths
    let mut px = Vec::with_capacity(n);
    let mut py = Vec::with_capacity(n);
    let mut hw = Vec::with_capacity(n);

    for i in 0..n {
        px.push((phys_w - stroke.points[i].x as i64) as f64);
        py.push(stroke.points[i].y as f64);
        let pressure = stroke.pressures.get(i).copied().unwrap_or(0);
        hw.push(pressure_width_f(stroke.thickness, pressure) as f64 / 2.0);
    }

    // Compute left and right edge points using the local normal at each point
    let mut lx = Vec::with_capacity(n);
    let mut ly = Vec::with_capacity(n);
    let mut rx = Vec::with_capacity(n);
    let mut ry = Vec::with_capacity(n);

    // Find the first and last indices where the position actually changes,
    // used as fallback tangent directions for endpoints. The device records
    // duplicate (x,y) at pen-down/pen-up with different pressures, so the
    // tangent between consecutive identical positions is zero — which would
    // collapse the polygon edges to a point instead of producing a round cap.
    let first_distinct = (1..n)
        .find(|&j| (px[j] - px[0]).abs() > 0.5 || (py[j] - py[0]).abs() > 0.5)
        .unwrap_or(1);
    let last_distinct = (0..n - 1)
        .rev()
        .find(|&j| (px[j] - px[n - 1]).abs() > 0.5 || (py[j] - py[n - 1]).abs() > 0.5)
        .unwrap_or(n - 2);

    for i in 0..n {
        // Tangent: forward at start, backward at end, central elsewhere.
        // At endpoints, reach past any duplicate-position points to find
        // a meaningful direction for the perpendicular offset.
        let (tx, ty) = if i == 0 {
            (px[first_distinct] - px[0], py[first_distinct] - py[0])
        } else if i == n - 1 {
            (px[n - 1] - px[last_distinct], py[n - 1] - py[last_distinct])
        } else {
            (px[i + 1] - px[i - 1], py[i + 1] - py[i - 1])
        };

        let len = (tx * tx + ty * ty).sqrt().max(0.001);
        // Normal (perpendicular to tangent, rotated 90° CCW)
        let nx = -ty / len;
        let ny = tx / len;

        lx.push(px[i] + nx * hw[i]);
        ly.push(py[i] + ny * hw[i]);
        rx.push(px[i] - nx * hw[i]);
        ry.push(py[i] - ny * hw[i]);
    }

    // Build the path in two layers within a single <path> element:
    //
    // 1. An outline polygon (left edge → end cap → right edge reversed → close)
    //    that smoothly interpolates the variable width between sample points.
    //
    // 2. A filled circle at every sample point, matching the device's stamp
    //    rendering. These fill holes that the polygon creates when the stroke
    //    curves tighter than its width (common in handwriting), causing the
    //    polygon edges to self-intersect.
    //
    // Both are the same fill color, so their union is seamless.
    let mut d = String::with_capacity(n * 80);

    // --- Outline polygon ---
    // Start cap: semicircular arc from right[0] to left[0]
    let r0 = hw[0];
    if r0 >= 0.5 {
        write!(d, "M {:.1},{:.1}", rx[0], ry[0]).unwrap();
        write!(d, " A {r0:.1},{r0:.1} 0 0 1 {:.1},{:.1}", lx[0], ly[0]).unwrap();
    } else {
        write!(d, "M {:.1},{:.1}", lx[0], ly[0]).unwrap();
    }

    // Left edge forward
    for i in 1..n {
        write!(d, " L {:.1},{:.1}", lx[i], ly[i]).unwrap();
    }

    // End cap: semicircular arc from left[n-1] to right[n-1]
    let rn = hw[n - 1];
    if rn >= 0.5 {
        write!(
            d,
            " A {rn:.1},{rn:.1} 0 0 1 {:.1},{:.1}",
            rx[n - 1],
            ry[n - 1]
        )
        .unwrap();
    }

    // Right edge backward
    for i in (0..n - 1).rev() {
        write!(d, " L {:.1},{:.1}", rx[i], ry[i]).unwrap();
    }
    d.push_str(" Z");

    // --- Per-point circles ---
    for i in 0..n {
        let r = hw[i];
        if r < 0.5 {
            continue;
        }
        let r2 = r * 2.0;
        write!(
            d,
            " M {:.1},{:.1} a {r:.1},{r:.1} 0 1,0 {r2:.1},0 a {r:.1},{r:.1} 0 1,0 -{r2:.1},0",
            px[i] - r,
            py[i]
        )
        .unwrap();
    }

    paths.push(format!(r#"<path{fill_attr} stroke="none" d="{d}"/>"#));
}

/// Add filled square end caps to a Marker stroke.
///
/// The Supernote device renders markers by stamping circles at pen-down, then
/// switching to square stamps once movement begins. The result is flat/square
/// ends instead of round. We replicate this by drawing a filled square at each
/// endpoint, oriented in the movement direction, which overlays the round
/// linecap from the stroked path.
fn render_marker_end_caps(
    stroke: &Stroke,
    phys_w: i64,
    width: u32,
    fill_attr: &str,
    blend_attr: &str,
    paths: &mut Vec<String>,
) {
    let n = stroke.points.len();
    if n < 2 {
        return;
    }

    let hw = width as f64 / 2.0;

    // Transform points to SVG coordinates
    let svgx = |i: usize| (phys_w - stroke.points[i].x as i64) as f64;
    let svgy = |i: usize| stroke.points[i].y as f64;

    // Find stable movement direction at each end by looking past any
    // pen-down/pen-up wobble. Use a reference point well into the stroke.
    let ref_dist = (n / 10).max(5).min(n - 1);

    // Start direction: from first point toward a stable reference
    let (sx, sy) = (svgx(0), svgy(0));
    let (srx, sry) = (svgx(ref_dist), svgy(ref_dist));
    let start_len = ((srx - sx).powi(2) + (sry - sy).powi(2)).sqrt();

    // End direction: from a stable reference toward the last point
    let end_ref = n.saturating_sub(ref_dist + 1);
    let (ex, ey) = (svgx(n - 1), svgy(n - 1));
    let (erx, ery) = (svgx(end_ref), svgy(end_ref));
    let end_len = ((ex - erx).powi(2) + (ey - ery).powi(2)).sqrt();

    // Emit a filled half-square at each end, extending only outward
    // (away from the stroke body). This covers the outward half of the
    // round linecap circle, turning it into a flat edge on the outside
    // while keeping the inward half round.
    // For start: outward = opposite to movement direction (-1)
    // For end: outward = same as movement direction (+1)
    for &(cx, cy, dx, dy, dlen, out_sign) in &[
        (sx, sy, srx - sx, sry - sy, start_len, -1.0_f64),
        (ex, ey, ex - erx, ey - ery, end_len, 1.0_f64),
    ] {
        if dlen < 0.001 {
            continue;
        }
        // Normalized direction and perpendicular
        let ddx = dx / dlen;
        let ddy = dy / dlen;
        let nx = -ddy;
        let ny = ddx;

        // Half-square: from the center line outward by hw
        let c1x = cx + nx * hw;
        let c1y = cy + ny * hw;
        let c2x = cx - nx * hw;
        let c2y = cy - ny * hw;
        let c3x = cx + out_sign * ddx * hw - nx * hw;
        let c3y = cy + out_sign * ddy * hw - ny * hw;
        let c4x = cx + out_sign * ddx * hw + nx * hw;
        let c4y = cy + out_sign * ddy * hw + ny * hw;

        let d = format!(
            "M {c1x:.1},{c1y:.1} L {c2x:.1},{c2y:.1} L {c3x:.1},{c3y:.1} L {c4x:.1},{c4y:.1} Z"
        );
        paths.push(format!(
            r#"<path{fill_attr}{blend_attr} stroke="none" d="{d}"/>"#
        ));
    }
}

/// Build a fill color attribute string from a stroke color value.
fn fill_attribute(color: u32) -> String {
    if color == 0 {
        r#" fill="black""#.to_string()
    } else {
        let val = color as u8;
        format!(" fill=\"#{val:02X}{val:02X}{val:02X}\"")
    }
}

fn color_attribute(color: u32) -> String {
    if color == 0 {
        return String::new();
    }
    let val = color as u8;
    format!(" stroke=\"#{val:02X}{val:02X}{val:02X}\"")
}

/// Render a text box as an SVG `<text>` element in pixel coordinates.
fn render_text_box(svg: &mut String, tb: &TextBox) {
    use std::fmt::Write;

    let font_family = map_font_family(&tb.font_path);
    let x = tb.rect.0;
    // SVG text baseline: position at rect top + font size
    let y = tb.rect.1 as f32 + tb.font_size;
    let font_style = if tb.italic {
        " font-style=\"italic\""
    } else {
        ""
    };

    // Escape XML special characters in content
    let content = tb
        .content
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;");

    // Handle multi-line text: split on newlines and use <tspan> elements
    let lines: Vec<&str> = content.split('\n').collect();
    if lines.len() == 1 {
        writeln!(
            svg,
            r#"<text x="{x}" y="{y:.0}" font-size="{:.0}" font-family="{font_family}"{font_style}>{content}</text>"#,
            tb.font_size
        )
        .unwrap();
    } else {
        write!(
            svg,
            r#"<text x="{x}" y="{y:.0}" font-size="{:.0}" font-family="{font_family}"{font_style}>"#,
            tb.font_size
        )
        .unwrap();
        for (i, line) in lines.iter().enumerate() {
            if i == 0 {
                write!(svg, r#"<tspan x="{x}" dy="0">{line}</tspan>"#).unwrap();
            } else {
                write!(
                    svg,
                    r#"<tspan x="{x}" dy="{:.0}">{line}</tspan>"#,
                    tb.font_size
                )
                .unwrap();
            }
        }
        svg.push_str("</text>\n");
    }
}

/// Map a Supernote device font path to a CSS font-family string.
fn map_font_family(font_path: &str) -> &'static str {
    // Extract the basename without extension
    let basename = font_path
        .rsplit('/')
        .next()
        .unwrap_or(font_path)
        .split('.')
        .next()
        .unwrap_or("");

    match basename {
        "Dolce" => "'Segoe Script', 'Bradley Hand', cursive",
        "Satisfy-Regular" => "'Satisfy', cursive",
        "851tegakizatsu" => "'Zen Kurenaido', 'Comic Sans MS', cursive",
        s if s.starts_with("Roboto") => "sans-serif",
        "DroidSansFallbackFull" => "sans-serif",
        _ => "sans-serif",
    }
}

/// Flush accumulated pen and eraser paths into the SVG string.
fn flush_group(
    svg: &mut String,
    pen_paths: &mut Vec<String>,
    eraser_paths: &mut Vec<String>,
    mask_count: &mut u32,
    phys_w: i64,
    phys_h: i64,
) {
    use std::fmt::Write;

    if pen_paths.is_empty() && eraser_paths.is_empty() {
        return;
    }

    let has_erasers = !eraser_paths.is_empty();

    if has_erasers {
        // Emit mask definition with eraser paths
        write!(
            svg,
            r#"<defs><mask id="eraser_{mask_count}" stroke="black" stroke-linejoin="round" stroke-linecap="round" fill="none">
<rect width="{phys_w}" height="{phys_h}" fill="white"/>
"#
        )
        .unwrap();
        for path in eraser_paths.drain(..) {
            svg.push_str(&path);
            svg.push('\n');
        }
        svg.push_str("</mask></defs>\n");
    }

    // Emit pen strokes group, masked if there were erasers
    if !pen_paths.is_empty() {
        if has_erasers {
            write!(svg, r#"<g mask="url(#eraser_{mask_count})">"#).unwrap();
            svg.push('\n');
        }
        for path in pen_paths.drain(..) {
            svg.push_str(&path);
            svg.push('\n');
        }
        if has_erasers {
            svg.push_str("</g>\n");
        }
    }

    if has_erasers {
        *mask_count += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_file(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../testfiles")
            .join(name)
    }

    #[test]
    fn svg_basic_structure() {
        let (data, nb) = crate::format::load(&test_file("test_after_save.note")).unwrap();
        for page in 0..nb.page_count() {
            let svg = to_svg(&data, &nb, page).unwrap();
            assert!(
                svg.starts_with("<svg"),
                "page {page}: should start with <svg"
            );
            assert!(svg.contains("viewBox"), "page {page}: should have viewBox");
            assert!(
                svg.contains("<path"),
                "page {page}: should contain <path elements"
            );
            assert!(
                svg.contains("</svg>"),
                "page {page}: should close svg element"
            );
        }
    }

    #[test]
    fn svg_out_of_range() {
        let (data, nb) = crate::format::load(&test_file("test_after_save.note")).unwrap();
        assert!(to_svg(&data, &nb, 999).is_err());
    }
}
