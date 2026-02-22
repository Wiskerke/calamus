use anyhow::{Result, bail};

/// Decode a RATTA_RLE encoded bitmap.
///
/// Returns a Vec<u8> of grayscale pixel values (page_width * page_height bytes).
/// Color code mapping:
///   0x61 -> 0x00 (black), 0x62 -> 0xFF (transparent/background),
///   0x63 -> 0x9D (dark gray), 0x64 -> 0xC9 (gray),
///   0x65 -> 0xFE (white), 0x66 -> 0x00 (marker black),
///   0x67 -> 0x9D (marker dark gray), 0x68 -> 0xC9 (marker gray)
/// X2 codes: 0x9D -> 0x9D, 0x9E -> 0x9D, 0xC9 -> 0xC9, 0xCA -> 0xC9
/// Compat codes: 0x63 -> 0x30, 0x64 -> 0x50 (when highres=true)
pub fn decode_rle(
    data: &[u8],
    page_width: u32,
    page_height: u32,
    all_blank: bool,
    highres: bool,
) -> Result<Vec<u8>> {
    // Reserve the memory for the bitmap
    let expected_pixel_count = (page_width * page_height) as usize;
    let mut pixels = Vec::with_capacity(expected_pixel_count);

    let mut iter = data.chunks_exact(2);
    let mut held: Option<(u8, u8)> = None;

    loop {
        let pair = iter.next().map(|chunk| (chunk[0], chunk[1]));

        if let Some((prev_cc, prev_len)) = held.take() {
            match pair {
                Some((cc, len)) => {
                    if cc == prev_cc {
                        // Same color continuation
                        let combined = 1 + len as usize + ((((prev_len & 0x7F) as usize) + 1) << 7);
                        let color = map_color(prev_cc, highres);
                        emit(&mut pixels, color, combined);
                    } else {
                        // Different color: emit the held run, then process new pair
                        let held_len = (((prev_len & 0x7F) as usize) + 1) << 7;
                        let color = map_color(prev_cc, highres);
                        emit(&mut pixels, color, held_len);
                        // Process the new pair normally
                        process_pair(&mut pixels, &mut held, cc, len, all_blank, highres);
                    }
                }
                None => {
                    // End of stream with held data: adjust tail
                    let adjusted = adjust_tail_length(prev_len, pixels.len(), expected_pixel_count);
                    if adjusted > 0 {
                        let color = map_color(prev_cc, highres);
                        emit(&mut pixels, color, adjusted);
                    }
                    break;
                }
            }
        } else {
            match pair {
                Some((cc, len)) => {
                    process_pair(&mut pixels, &mut held, cc, len, all_blank, highres);
                }
                None => break, // no more input data: Stop.
            }
        }
    }

    if pixels.len() != expected_pixel_count {
        bail!(
            "RLE decode: got {} pixels, expected {} ({}x{})",
            pixels.len(),
            expected_pixel_count,
            page_width,
            page_height
        );
    }

    Ok(pixels)
}

fn process_pair(
    pixels: &mut Vec<u8>,
    held: &mut Option<(u8, u8)>,
    cc: u8,
    len: u8,
    all_blank: bool,
    highres: bool,
) {
    if len == 0xFF {
        // Special marker
        let run_len = if all_blank { 0x400 } else { 0x4000 };
        let color = map_color(cc, highres);
        emit(pixels, color, run_len);
    } else if len & 0x80 != 0 {
        // Multi-byte: hold for next iteration
        *held = Some((cc, len));
    } else {
        // Simple: length + 1
        let color = map_color(cc, highres);
        emit(pixels, color, (len as usize) + 1);
    }
}

fn emit(pixels: &mut Vec<u8>, color: u8, count: usize) {
    pixels.extend(std::iter::repeat_n(color, count));
}

fn map_color(code: u8, highres: bool) -> u8 {
    if highres {
        // X2-series: only a few codes are symbolic, everything else
        // (including old X-series codes like 0x67/0x68) is a literal
        // grayscale intensity.
        match code {
            0x61 => 0x00, // black
            0x62 => 0xFF, // transparent/background
            0x63 => 0x30, // dark gray compat
            0x64 => 0x50, // gray compat
            0x65 => 0xFE, // white
            0x66 => 0x00, // marker black
            0x9D => 0x9D, // dark gray
            0x9E => 0x9D, // marker dark gray
            0xC9 => 0xC9, // gray
            0xCA => 0xC9, // marker gray
            other => other,
        }
    } else {
        // X-series: fixed set of symbolic color codes
        match code {
            0x61 => 0x00, // black
            0x62 => 0xFF, // transparent/background
            0x63 => 0x9D, // dark gray
            0x64 => 0xC9, // gray
            0x65 => 0xFE, // white
            0x66 => 0x00, // marker black
            0x67 => 0x9D, // marker dark gray
            0x68 => 0xC9, // marker gray
            other => other,
        }
    }
}

fn adjust_tail_length(tail_length: u8, current_length: usize, total_length: usize) -> usize {
    // This logic is taken over from supernotelib.
    // It feels quite strange, as i=7 feels better from a protocol design point of view. And
    // it would make it consistent with a color change.
    // But let's keep it for now, as this is more battleharded and with the gap check and a
    // correct file would use the right i value anyway even if that is always 7.
    let gap = total_length.saturating_sub(current_length);
    for i in (0..8).rev() {
        let l = (((tail_length & 0x7F) as usize) + 1) << i;
        if l <= gap {
            return l;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_single_color_run() {
        // 3 pixels of black: cc=0x61, len=2 (length = 2+1 = 3)
        let data = [0x61, 0x02];
        let result = decode_rle(&data, 3, 1, false, false).unwrap();
        assert_eq!(result, vec![0x00, 0x00, 0x00]);
    }

    #[test]
    fn two_color_runs() {
        // 2 black + 2 transparent = 4 pixels
        let data = [0x61, 0x01, 0x62, 0x01];
        let result = decode_rle(&data, 4, 1, false, false).unwrap();
        assert_eq!(result, vec![0x00, 0x00, 0xFF, 0xFF]);
    }

    #[test]
    fn special_marker_0xff() {
        // 0xFF marker: 16384 pixels of transparent
        let data = [0x62, 0xFF];
        let result = decode_rle(&data, 16384, 1, false, false).unwrap();
        assert_eq!(result.len(), 16384);
        assert!(result.iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn special_marker_0xff_all_blank() {
        // 0xFF marker with all_blank: 1024 pixels
        let data = [0x62, 0xFF];
        let result = decode_rle(&data, 1024, 1, true, false).unwrap();
        assert_eq!(result.len(), 1024);
        assert!(result.iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn multi_byte_same_color() {
        // Multi-byte: first pair (0x61, 0x80), second pair (0x61, 0x03)
        // Same color: length = 1 + 3 + ((0x80 & 0x7F) + 1) << 7 = 1 + 3 + 128 = 132
        let data = [0x61, 0x80, 0x61, 0x03];
        let result = decode_rle(&data, 132, 1, false, false).unwrap();
        assert_eq!(result.len(), 132);
        assert!(result.iter().all(|&b| b == 0x00));
    }

    #[test]
    fn multi_byte_different_color() {
        // Multi-byte: first pair (0x61, 0x80), second pair (0x62, 0x01)
        // Different: first run = ((0 + 1) << 7) = 128 black, second run = 2 transparent
        let data = [0x61, 0x80, 0x62, 0x01];
        let result = decode_rle(&data, 130, 1, false, false).unwrap();
        assert_eq!(result.len(), 130);
        assert!(result[..128].iter().all(|&b| b == 0x00));
        assert!(result[128..].iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn tail_adjustment() {
        // Test the adjust_tail_length function directly
        // gap = 128, tail_length = 0x80 (bit7 set)
        // ((0x80 & 0x7F) + 1) << 7 = 128
        assert_eq!(adjust_tail_length(0x80, 0, 128), 128);
        // gap = 64, need to try smaller shifts
        // ((0 + 1) << 6) = 64
        assert_eq!(adjust_tail_length(0x80, 0, 64), 64);
    }

    #[test]
    fn highres_color_mapping() {
        // 0x63 in highres mode maps to 0x30 (compat dark gray)
        let data = [0x63, 0x00];
        let result = decode_rle(&data, 1, 1, false, true).unwrap();
        assert_eq!(result, vec![0x30]);

        // 0x63 in non-highres maps to 0x9D
        let result2 = decode_rle(&data, 1, 1, false, false).unwrap();
        assert_eq!(result2, vec![0x9D]);

        // 0x67 in highres mode is a literal grayscale value (103)
        let data67 = [0x67, 0x00];
        let result3 = decode_rle(&data67, 1, 1, false, true).unwrap();
        assert_eq!(result3, vec![0x67]);

        // 0x67 in non-highres maps to 0x9D (marker dark gray)
        let result4 = decode_rle(&data67, 1, 1, false, false).unwrap();
        assert_eq!(result4, vec![0x9D]);
    }

    #[test]
    fn wrong_pixel_count_errors() {
        let data = [0x61, 0x02]; // produces 3 pixels
        let result = decode_rle(&data, 4, 1, false, false);
        assert!(result.is_err());
    }
}
