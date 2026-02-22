use anyhow::{Context, Result, bail};
use base64::Engine;
use zerocopy::little_endian::{F32, I16, I32, U16, U32, U64};
use zerocopy::{FromBytes, Immutable, KnownLayout, Unaligned};

// ── Zerocopy binary structs (for direct memory-mapped parsing) ──

#[derive(Debug, Clone, Copy, PartialEq, Eq, FromBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C, packed)]
pub struct RawScreenCoord {
    pub y: I32,
    pub x: I32,
}

#[derive(Debug, Clone, Copy, PartialEq, FromBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C, packed)]
pub struct RawPixelCoord {
    pub x: F32,
    pub y: F32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, FromBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C, packed)]
pub struct RawVector {
    pub y: I16,
    pub x: I16,
}

/// 208-byte stroke header (confirmed for Nomad/N6).
#[derive(Debug, FromBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C, packed)]
pub struct RawStrokeConfig {
    pub pen: U32,
    pub color: U32,
    pub thickness: U32,
    pub rec_mod: U32,
    pub unk_1: U32,
    pub font_height: U32,
    pub unk_2: U32,
    pub page_num: U32,
    pub unk_3: U32,
    pub unk_4: U32,
    pub unk_5: U32,
    pub stroke_layer: U32,
    pub stroke_kind: [u8; 52],
    pub bounding_tl: RawScreenCoord,
    pub bounding_mid: RawScreenCoord,
    pub bounding_br: RawScreenCoord,
    pub unk_6: U32,
    pub screen_height: U32,
    pub screen_width: U32,
    pub doc_kind: [u8; 52],
    pub emr_point_axis: U32,
    pub unk_7: [U32; 4],
}

#[derive(Debug, FromBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C, packed)]
pub struct RawSection1 {
    pub unk_8: U32,
    pub unk_9: U32,
    pub stroke_uid: U32,
    pub unk_10: U32,
    pub unk_11: U32,
    pub unk_12: [U32; 4],
    pub unk_13: [U32; 4],
}

#[derive(Debug, FromBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C, packed)]
pub struct RawSection2 {
    pub unk_14: [U32; 2],
    pub unk_15: u8,
    pub render_flag: u8,
}

#[derive(Debug, FromBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C, packed)]
pub struct RawSection3 {
    pub unk_18: U32,
    pub unk_19: U64,
    pub unk_20: u8,
    pub rotation_degrees: I32,
}

#[derive(Debug, FromBytes, KnownLayout, Immutable, Unaligned)]
#[repr(C, packed)]
pub struct RawSection4 {
    pub pixel_width: U32,
    pub pixel_height: U32,
    pub unk_23: U32,
    pub unk_24: u8,
}

// ── Pen type constants (from device firmware) ──

pub const PEN_INK_PEN: u32 = 1;
pub const PEN_NEEDLE_POINT: u32 = 10;
pub const PEN_MARKER: u32 = 11;

/// Special color value indicating an eraser stroke.
pub const COLOR_ERASER: u32 = 255;

// ── Owned types (public API) ──

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenCoord {
    pub y: i32,
    pub x: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TiltVector {
    pub y: i16,
    pub x: i16,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    pub pen: u32,
    pub color: u32,
    pub thickness: u32,
    pub page_num: u32,
    pub stroke_layer: u32,
    pub stroke_kind: String,
    pub bounding_tl: ScreenCoord,
    pub bounding_mid: ScreenCoord,
    pub bounding_br: ScreenCoord,
    pub screen_width: u32,
    pub screen_height: u32,
    pub points: Vec<ScreenCoord>,
    pub pressures: Vec<u16>,
    pub tilts: Vec<TiltVector>,
    pub stroke_uid: u32,
    pub rotation_degrees: i32,
    pub pixel_width: u32,
    pub pixel_height: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextBox {
    pub content: String,
    /// (x, y, w, h) in pixel coordinates
    pub rect: (u32, u32, u32, u32),
    pub font_size: f32,
    pub font_path: String,
    pub italic: bool,
    pub datetime: String,
    pub stroke_layer: u32,
}

// ── Parsing helpers ──

fn read_u32(data: &[u8], pos: &mut usize) -> Result<u32> {
    if *pos + 4 > data.len() {
        bail!("read_u32 at offset {}: unexpected end of data", *pos);
    }
    let val = u32::from_le_bytes(data[*pos..*pos + 4].try_into().unwrap());
    *pos += 4;
    Ok(val)
}

fn read_raw_section<'a, T: FromBytes + Immutable + KnownLayout>(
    data: &'a [u8],
    pos: &mut usize,
) -> Result<&'a T> {
    let size = size_of::<T>();
    if *pos + size > data.len() {
        bail!(
            "read_raw_section at offset {}: need {} bytes, only {} available",
            *pos,
            size,
            data.len() - *pos
        );
    }
    let val = T::ref_from_bytes(&data[*pos..*pos + size])
        .map_err(|e| anyhow::anyhow!("zerocopy error at offset {}: {e}", *pos))?;
    *pos += size;
    Ok(val)
}

fn read_raw_array<'a, T: FromBytes + Immutable>(
    data: &'a [u8],
    pos: &mut usize,
) -> Result<&'a [T]> {
    let count = read_u32(data, pos)? as usize;
    let byte_len = count * size_of::<T>();
    if *pos + byte_len > data.len() {
        bail!(
            "read_raw_array at offset {}: need {} elements ({} bytes), only {} available",
            *pos,
            count,
            byte_len,
            data.len() - *pos
        );
    }
    let val = <[T]>::ref_from_bytes_with_elems(&data[*pos..*pos + byte_len], count)
        .map_err(|e| anyhow::anyhow!("zerocopy array error at offset {}: {e}", *pos))?;
    *pos += byte_len;
    Ok(val)
}

fn read_sized_bytes<'a>(data: &'a [u8], pos: &mut usize) -> Result<&'a [u8]> {
    let len = read_u32(data, pos)? as usize;
    if *pos + len > data.len() {
        bail!(
            "read_sized_bytes at offset {}: need {} bytes, only {} available",
            *pos,
            len,
            data.len() - *pos
        );
    }
    let val = &data[*pos..*pos + len];
    *pos += len;
    Ok(val)
}

fn extract_c_string(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

fn raw_coord_to_owned(raw: &RawScreenCoord) -> ScreenCoord {
    ScreenCoord {
        y: raw.y.get(),
        x: raw.x.get(),
    }
}

// ── TOTALPATH parser ──

/// Parse a TOTALPATH blob (after the length prefix has been stripped) into Strokes.
///
/// Expected layout: `[stroke_count: u32] [for each: [stroke_size: u32] [stroke_data]]`
pub fn parse_totalpath(data: &[u8]) -> Result<(Vec<Stroke>, Vec<TextBox>)> {
    let mut pos = 0;

    let stroke_count = read_u32(data, &mut pos).context("reading stroke_count")?;

    let mut strokes = Vec::with_capacity(stroke_count as usize);
    let mut text_boxes = Vec::new();
    for i in 0..stroke_count {
        let stroke_size =
            read_u32(data, &mut pos).with_context(|| format!("reading stroke {i} size"))?;
        let stroke_end = pos + stroke_size as usize;
        if stroke_end > data.len() {
            bail!(
                "stroke {i}: size {stroke_size} exceeds remaining data ({} bytes)",
                data.len() - pos
            );
        }
        let (stroke, text_data) = parse_single_stroke(&data[pos..stroke_end])
            .with_context(|| format!("parsing stroke {i}"))?;

        if stroke.pen == 0 && !text_data.is_empty() {
            if let Some(tb) = parse_text_block(&text_data, stroke.stroke_layer) {
                text_boxes.push(tb);
            } else {
                strokes.push(stroke);
            }
        } else {
            strokes.push(stroke);
        }
        pos = stroke_end;
    }

    Ok((strokes, text_boxes))
}

/// Parse text block data from the str1 field of a pen==0 stroke.
///
/// The data is a comma-separated list of base64-encoded fields. Field indices
/// follow the TEXT_BLOCK_FIELDS convention from pysn-digest:
///   [2] title_style — "0" for text boxes, non-zero for title headings
///   [3] datetime
///   [4] rect — "x,y,w,h" in pixel coordinates
///   [10] font_size — float string
///   [11] font_path — device font path
///   [12] content — the actual text
///   [13] transformation — CSV; position 8 is italic flag ("1" = italic)
fn parse_text_block(data: &[u8], stroke_layer: u32) -> Option<TextBox> {
    let ascii = std::str::from_utf8(data).ok()?;
    let b64 = base64::engine::general_purpose::STANDARD;

    let fields: Vec<String> = ascii
        .split(',')
        .map(|s| {
            if s.trim().is_empty() {
                return String::new();
            }
            b64.decode(s.trim())
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or_default()
        })
        .collect();

    if fields.len() < 13 {
        return None;
    }

    // Skip title headings (title_style != "0")
    let title_style = fields.get(2).map(|s| s.as_str()).unwrap_or("0");
    if title_style != "0" {
        return None;
    }

    let content = fields.get(12).cloned().unwrap_or_default();
    if content.is_empty() || content == "none" {
        return None;
    }

    // Parse rect "x,y,w,h"
    let rect_str = fields.get(4).map(|s| s.as_str()).unwrap_or("");
    let rect_parts: Vec<u32> = rect_str.split(',').filter_map(|s| s.parse().ok()).collect();
    let rect = if rect_parts.len() >= 4 {
        (rect_parts[0], rect_parts[1], rect_parts[2], rect_parts[3])
    } else {
        (0, 0, 0, 0)
    };

    let font_size = fields
        .get(10)
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(24.0);

    let font_path = fields.get(11).cloned().unwrap_or_default();

    // Parse italic from transformation field (position 8 in comma-separated values)
    let italic = fields
        .get(13)
        .map(|s| {
            s.split(',')
                .nth(8)
                .map(|v| v.trim() == "1")
                .unwrap_or(false)
        })
        .unwrap_or(false);

    let datetime = fields.get(3).cloned().unwrap_or_default();

    Some(TextBox {
        content,
        rect,
        font_size,
        font_path,
        italic,
        datetime,
        stroke_layer,
    })
}

fn parse_single_stroke(data: &[u8]) -> Result<(Stroke, Vec<u8>)> {
    let mut pos = 0;

    // 1. Fixed header (208 bytes)
    let config: &RawStrokeConfig =
        read_raw_section(data, &mut pos).context("reading stroke config")?;

    // 2. Disable area list
    let _disable_areas: &[[RawScreenCoord; 3]] =
        read_raw_array(data, &mut pos).context("reading disable_area_list")?;

    // 3. Points
    let raw_points: &[RawScreenCoord] = read_raw_array(data, &mut pos).context("reading points")?;

    // 4. Pressures
    let raw_pressures: &[U16] = read_raw_array(data, &mut pos).context("reading pressures")?;

    // 5. Tilts
    let raw_tilts: &[RawVector] = read_raw_array(data, &mut pos).context("reading tilts")?;

    // 6. Draw flags
    let _flag_draw: &[u8] = read_raw_array(data, &mut pos).context("reading flag_draw")?;

    // 7. EPA points
    let _epa_points: &[RawScreenCoord] =
        read_raw_array(data, &mut pos).context("reading epa_points")?;

    // 8. EPA grays
    let _epa_grays: &[I32] = read_raw_array(data, &mut pos).context("reading epa_grays")?;

    // 9. Section1 (52 bytes)
    let section1: &RawSection1 = read_raw_section(data, &mut pos).context("reading section1")?;

    // 10. Control nums
    let _control_nums: &[I32] = read_raw_array(data, &mut pos).context("reading control_nums")?;

    // 11. Section2 (10 bytes)
    let _section2: &RawSection2 = read_raw_section(data, &mut pos).context("reading section2")?;

    // 12. Point contours (nested arrays)
    let outer_count = read_u32(data, &mut pos).context("reading point_contour outer count")?;
    for _ in 0..outer_count {
        let _inner: &[RawPixelCoord] =
            read_raw_array(data, &mut pos).context("reading point_contour inner")?;
    }

    // 13. Unknown 16-byte chunks
    let unk17_count = read_u32(data, &mut pos).context("reading unk_17 count")?;
    pos += unk17_count as usize * 16;
    if pos > data.len() {
        bail!("unk_17 exceeds data");
    }

    // 14. Section3 (17 bytes)
    let section3: &RawSection3 = read_raw_section(data, &mut pos).context("reading section3")?;

    // 15. Unknown i32 array
    let _unk_22: &[I32] = read_raw_array(data, &mut pos).context("reading unk_22")?;

    // 16. Section4 (13 bytes)
    let section4: &RawSection4 = read_raw_section(data, &mut pos).context("reading section4")?;

    // 17. Three sized strings
    let text_data = read_sized_bytes(data, &mut pos)
        .context("reading sized_str_1")?
        .to_vec();
    let _str2 = read_sized_bytes(data, &mut pos).context("reading sized_str_2")?;
    let _str3 = read_sized_bytes(data, &mut pos).context("reading sized_str_3")?;

    // 18. Final fields
    let _unk_25 = read_u32(data, &mut pos).context("reading unk_25")?;
    let _mark_pen_d_fill_dir: &[RawPixelCoord] =
        read_raw_array(data, &mut pos).context("reading mark_pen_d_fill_dir")?;

    // Convert to owned types
    Ok((
        Stroke {
            pen: config.pen.get(),
            color: config.color.get(),
            thickness: config.thickness.get(),
            page_num: config.page_num.get(),
            stroke_layer: config.stroke_layer.get(),
            stroke_kind: extract_c_string(&config.stroke_kind),
            bounding_tl: raw_coord_to_owned(&config.bounding_tl),
            bounding_mid: raw_coord_to_owned(&config.bounding_mid),
            bounding_br: raw_coord_to_owned(&config.bounding_br),
            screen_width: config.screen_width.get(),
            screen_height: config.screen_height.get(),
            points: raw_points.iter().map(raw_coord_to_owned).collect(),
            pressures: raw_pressures.iter().map(|p| p.get()).collect(),
            tilts: raw_tilts
                .iter()
                .map(|t| TiltVector {
                    y: t.y.get(),
                    x: t.x.get(),
                })
                .collect(),
            stroke_uid: section1.stroke_uid.get(),
            rotation_degrees: section3.rotation_degrees.get(),
            pixel_width: section4.pixel_width.get(),
            pixel_height: section4.pixel_height.get(),
        },
        text_data,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_u32() {
        let data = 42u32.to_le_bytes();
        let mut pos = 0;
        assert_eq!(read_u32(&data, &mut pos).unwrap(), 42);
        assert_eq!(pos, 4);
    }

    #[test]
    fn test_read_u32_truncated() {
        let data = [0u8; 2];
        let mut pos = 0;
        assert!(read_u32(&data, &mut pos).is_err());
    }

    #[test]
    fn test_read_raw_array_empty() {
        let data = 0u32.to_le_bytes();
        let mut pos = 0;
        let arr: &[U32] = read_raw_array(&data, &mut pos).unwrap();
        assert_eq!(arr.len(), 0);
        assert_eq!(pos, 4);
    }

    #[test]
    fn test_read_raw_array_values() {
        let mut data = Vec::new();
        data.extend_from_slice(&2u32.to_le_bytes()); // count = 2
        data.extend_from_slice(&100u32.to_le_bytes());
        data.extend_from_slice(&200u32.to_le_bytes());
        let mut pos = 0;
        let arr: &[U32] = read_raw_array(&data, &mut pos).unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0].get(), 100);
        assert_eq!(arr[1].get(), 200);
    }

    #[test]
    fn test_extract_c_string() {
        let mut buf = [0u8; 52];
        buf[..6].copy_from_slice(b"others");
        assert_eq!(extract_c_string(&buf), "others");

        let mut buf2 = [0u8; 52];
        buf2[..12].copy_from_slice(b"straightLine");
        assert_eq!(extract_c_string(&buf2), "straightLine");
    }

    #[test]
    fn test_raw_stroke_config_size() {
        assert_eq!(
            size_of::<RawStrokeConfig>(),
            208,
            "StrokeConfig must be exactly 208 bytes"
        );
    }

    #[test]
    fn test_section_sizes() {
        assert_eq!(size_of::<RawSection1>(), 52);
        assert_eq!(size_of::<RawSection2>(), 10);
        assert_eq!(size_of::<RawSection3>(), 17);
        assert_eq!(size_of::<RawSection4>(), 13);
        assert_eq!(size_of::<RawScreenCoord>(), 8);
        assert_eq!(size_of::<RawPixelCoord>(), 8);
        assert_eq!(size_of::<RawVector>(), 4);
    }
}
