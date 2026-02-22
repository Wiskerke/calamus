pub mod metadata;
pub mod rle;
#[cfg(feature = "stroke")]
pub mod stroke;

use std::fmt;
use std::path::Path;

use anyhow::{Context, Result, bail};

use metadata::{Metadata, MetadataExt, read_metadata_block};

const LAYER_KEYS: &[&str] = &["MAINLAYER", "LAYER1", "LAYER2", "LAYER3", "BGLAYER"];

/// A parsed Supernote notebook skeleton (header, footer, page addresses).
/// Page content is decoded on demand via `page()`.
pub struct NotebookMeta {
    pub file_type: String,
    pub signature: String,
    pub header: Metadata,
    pub footer: Metadata,
    pub page_width: u32,
    pub page_height: u32,
    page_addresses: Vec<usize>,
}

/// Lightweight page metadata — stores addresses, not decoded content.
pub struct PageMeta {
    pub metadata: Metadata,
    pub style: String,
    pub layers: Vec<LayerMeta>,
    totalpath_addr: usize,
    /// Non-layered (A5) bitmap address.
    a5_bitmap_addr: Option<usize>,
    /// Non-layered (A5) protocol.
    a5_protocol: Option<String>,
}

/// Layer metadata — stores the bitmap address rather than copying data.
pub struct LayerMeta {
    pub name: String,
    pub layer_type: String,
    pub protocol: String,
    bitmap_addr: usize,
}

impl NotebookMeta {
    pub fn page_count(&self) -> usize {
        self.page_addresses.len()
    }

    /// Whether the file signature supports high-resolution grayscale (X2-series).
    pub fn supports_highres_grayscale(&self) -> bool {
        self.signature.len() >= 8
            && self.signature[self.signature.len() - 8..]
                .parse::<u64>()
                .is_ok_and(|v| v >= 20230015)
    }

    /// Parse page metadata on demand. Returns a lightweight PageMeta
    /// that stores addresses but does not decode strokes or bitmap data.
    pub fn page(&self, data: &[u8], page: usize) -> Result<PageMeta> {
        if page >= self.page_addresses.len() {
            bail!(
                "Page {page} out of range (notebook has {} pages)",
                self.page_addresses.len()
            );
        }
        parse_page_meta(data, self.page_addresses[page])
            .with_context(|| format!("failed to parse page {page}"))
    }
}

impl PageMeta {
    /// Get the rendering order of layers from the page's LAYERSEQ metadata.
    pub fn layer_order(&self) -> Vec<String> {
        if let Some(seq) = self.metadata.get("LAYERSEQ").and_then(|v| v.first()) {
            seq.split(',').map(|s| s.to_string()).collect()
        } else {
            // Default order: BGLAYER first (bottom), then MAINLAYER, LAYER1-3
            vec![
                "BGLAYER".to_string(),
                "MAINLAYER".to_string(),
                "LAYER1".to_string(),
                "LAYER2".to_string(),
                "LAYER3".to_string(),
            ]
        }
    }

    /// Whether this page has stroke/path data.
    pub fn has_strokes(&self) -> bool {
        self.totalpath_addr > 0
    }

    /// Decode strokes and text boxes from the TOTALPATH data on demand.
    #[cfg(feature = "stroke")]
    pub fn decode_strokes(
        &self,
        data: &[u8],
    ) -> Result<(Vec<stroke::Stroke>, Vec<stroke::TextBox>)> {
        if self.totalpath_addr == 0 {
            return Ok((Vec::new(), Vec::new()));
        }
        let tp_data = data_block_slice(data, self.totalpath_addr)
            .context("failed to read TOTALPATH block")?;
        stroke::parse_totalpath(tp_data).context("failed to parse TOTALPATH")
    }

    /// Get a zero-copy reference to a layer's RLE bitmap data.
    pub fn layer_bitmap<'a>(&self, data: &'a [u8], layer_index: usize) -> Option<&'a [u8]> {
        let layer = self.layers.get(layer_index)?;
        if layer.bitmap_addr == 0 {
            return None;
        }
        data_block_slice(data, layer.bitmap_addr).ok()
    }

    /// Get a zero-copy reference to the A5 (non-layered) bitmap data.
    pub fn a5_bitmap<'a>(&self, data: &'a [u8]) -> Option<&'a [u8]> {
        let addr = self.a5_bitmap_addr?;
        if addr == 0 {
            return None;
        }
        data_block_slice(data, addr).ok()
    }

    /// Get the A5 protocol string.
    pub fn a5_protocol(&self) -> Option<&str> {
        self.a5_protocol.as_deref()
    }
}

impl fmt::Display for NotebookMeta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "File type: {}", self.file_type)?;
        writeln!(f, "Signature: {}", self.signature)?;
        if let Some(equip) = self.header.get_str("APPLY_EQUIPMENT") {
            writeln!(f, "Device: {equip}")?;
        }
        writeln!(f, "Page size: {}x{}", self.page_width, self.page_height)?;
        writeln!(f, "Pages: {}", self.page_addresses.len())?;
        Ok(())
    }
}

/// Return a zero-copy slice of a length-prefixed data block at the given offset.
fn data_block_slice(data: &[u8], offset: usize) -> Result<&[u8]> {
    if offset + 4 > data.len() {
        bail!("data block at offset {offset}: not enough data for length prefix");
    }
    let block_len = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
    let start = offset + 4;
    let end = start + block_len;
    if end > data.len() {
        bail!(
            "data block at offset {offset}: length {block_len} exceeds file size {}",
            data.len()
        );
    }
    Ok(&data[start..end])
}

/// Load and parse a Supernote .note file.
/// Returns the raw file data and a lightweight notebook skeleton.
pub fn load(path: &Path) -> Result<(Vec<u8>, NotebookMeta)> {
    let data =
        std::fs::read(path).with_context(|| format!("failed to read file: {}", path.display()))?;
    let notebook = parse(&data)?;
    Ok((data, notebook))
}

/// Parse a Supernote .note file from raw bytes.
/// Returns a lightweight skeleton — page content is decoded on demand.
pub fn parse(data: &[u8]) -> Result<NotebookMeta> {
    // Detect format
    let (file_type, signature) = detect_format(data)?;

    // Read footer address from last 4 bytes
    if data.len() < 4 {
        bail!("file too small");
    }

    // Parse footer, which address is always at the end of the file
    let footer_addr = u32::from_le_bytes(data[data.len() - 4..].try_into().unwrap()) as usize;
    let footer = read_metadata_block(data, footer_addr).context("failed to parse footer")?;

    // Parse header
    let header_addr = footer
        .get_address("FILE_FEATURE")
        .context("footer missing FILE_FEATURE")?;
    let header = read_metadata_block(data, header_addr).context("failed to parse header")?;

    // Determine page dimensions from device
    let (page_width, page_height) = match header.get_str("APPLY_EQUIPMENT") {
        Some("N5") => (1920, 2560),
        _ => (1404, 1872),
    };

    // Collect page addresses (no page parsing)
    let page_addresses = get_page_addresses(&footer)?;

    Ok(NotebookMeta {
        file_type,
        signature,
        header,
        footer,
        page_width,
        page_height,
        page_addresses,
    })
}

fn detect_format(data: &[u8]) -> Result<(String, String)> {
    // X-series: starts with "note" (4B) + "SN_FILE_VER_*" (20B)
    if data.len() >= 24 && &data[..4] == b"note" {
        let sig = std::str::from_utf8(&data[4..24]).context("invalid signature encoding")?;
        if sig.starts_with("SN_FILE_VER_") {
            return Ok(("note".to_string(), sig.to_string()));
        }
    }
    // Also check for "mark" file type
    if data.len() >= 24 && &data[..4] == b"mark" {
        let sig = std::str::from_utf8(&data[4..24]).context("invalid signature encoding")?;
        if sig.starts_with("SN_FILE_VER_") {
            return Ok(("mark".to_string(), sig.to_string()));
        }
    }
    // Original A5: starts with "SN_FILE_ASA_*" (no separate file type)
    if data.len() >= 20 {
        let sig = std::str::from_utf8(&data[..20]).context("invalid signature encoding")?;
        if sig.starts_with("SN_FILE_ASA_") {
            // file type is embedded in the signature for A5
            return Ok(("note".to_string(), sig.to_string()));
        }
    }
    bail!("not a valid Supernote .note file: unrecognized format");
}

fn get_page_addresses(footer: &Metadata) -> Result<Vec<usize>> {
    // X-series: numbered keys PAGE1, PAGE2, ...
    let mut numbered: Vec<(usize, usize)> = Vec::new();
    for key in footer.keys() {
        if let Some(num_str) = key.strip_prefix("PAGE")
            && let Ok(num) = num_str.parse::<usize>()
        {
            let addr = footer
                .get_address(key)
                .with_context(|| format!("invalid address for {key}"))?;
            numbered.push((num, addr));
        }
    }

    if !numbered.is_empty() {
        numbered.sort_by_key(|(n, _)| *n);
        return Ok(numbered.into_iter().map(|(_, addr)| addr).collect());
    }

    // Original A5: repeated PAGE key
    let page_values = footer.get_all("PAGE");
    if !page_values.is_empty() {
        let addrs: Result<Vec<usize>> = page_values
            .iter()
            .map(|v| {
                v.parse::<usize>()
                    .with_context(|| format!("invalid PAGE address: {v}"))
            })
            .collect();
        return addrs;
    }

    bail!("no page addresses found in footer");
}

fn parse_page_meta(data: &[u8], page_addr: usize) -> Result<PageMeta> {
    let page_meta =
        read_metadata_block(data, page_addr).context("failed to parse page metadata")?;

    let style = page_meta
        .get_str("PAGESTYLE")
        .unwrap_or("style_white")
        .to_string();

    // Check if this page has layers (X-series) or not (A5)
    let has_layers = LAYER_KEYS.iter().any(|k| page_meta.get_str(k).is_some());

    let mut layers = Vec::new();
    let mut a5_bitmap_addr = None;
    let mut a5_protocol = None;

    if has_layers {
        // X-series: parse each layer's metadata (addresses only, no bitmap data)
        for &layer_key in LAYER_KEYS {
            let layer = parse_layer_meta(data, &page_meta, layer_key)?;
            layers.push(layer);
        }
    } else {
        // Original A5: store bitmap address
        a5_protocol = page_meta.get_str("PROTOCOL").map(|s| s.to_string());
        if let Some(data_addr_str) = page_meta.get_str("DATA") {
            let data_addr: usize = data_addr_str
                .parse()
                .with_context(|| format!("invalid DATA address: {data_addr_str}"))?;
            if data_addr > 0 {
                a5_bitmap_addr = Some(data_addr);
            }
        }
    }

    // Store TOTALPATH address (don't decode yet)
    let totalpath_addr = if let Some(tp_addr_str) = page_meta.get_str("TOTALPATH") {
        tp_addr_str.parse::<usize>().unwrap_or(0)
    } else {
        0
    };

    Ok(PageMeta {
        metadata: page_meta,
        style,
        layers,
        totalpath_addr,
        a5_bitmap_addr,
        a5_protocol,
    })
}

fn parse_layer_meta(data: &[u8], page_meta: &Metadata, layer_key: &str) -> Result<LayerMeta> {
    let addr_str = page_meta
        .get_str(layer_key)
        .with_context(|| format!("page missing {layer_key}"))?;
    let addr: usize = addr_str
        .parse()
        .with_context(|| format!("invalid {layer_key} address: {addr_str}"))?;

    // Address 0 means no data for this layer
    if addr == 0 {
        return Ok(LayerMeta {
            name: layer_key.to_string(),
            layer_type: String::new(),
            protocol: "RATTA_RLE".to_string(),
            bitmap_addr: 0,
        });
    }

    let layer_meta = read_metadata_block(data, addr)
        .with_context(|| format!("failed to parse {layer_key} metadata"))?;

    let name = layer_meta
        .get_str("LAYERNAME")
        .unwrap_or(layer_key)
        .to_string();
    let layer_type = layer_meta.get_str("LAYERTYPE").unwrap_or("").to_string();
    let layer_protocol = layer_meta
        .get_str("LAYERPROTOCOL")
        .unwrap_or("RATTA_RLE")
        .to_string();

    let bitmap_addr = layer_meta
        .get_str("LAYERBITMAP")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);

    Ok(LayerMeta {
        name,
        layer_type,
        protocol: layer_protocol,
        bitmap_addr,
    })
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
    fn load_saved_file() {
        let (data, nb) = load(&test_file("test_after_save.note")).unwrap();
        assert_eq!(nb.file_type, "note");
        assert_eq!(nb.signature, "SN_FILE_VER_20230015");
        assert_eq!(nb.header.get_str("APPLY_EQUIPMENT"), Some("N6"));
        assert_eq!(nb.page_width, 1404);
        assert_eq!(nb.page_height, 1872);
        assert_eq!(nb.page_count(), 2);

        let page0 = nb.page(&data, 0).unwrap();
        assert_eq!(page0.layers.len(), 5);
    }

    #[test]
    #[cfg(feature = "stroke")]
    fn load_saved_file_strokes() {
        let (data, nb) = load(&test_file("test_after_save.note")).unwrap();
        let page0 = nb.page(&data, 0).unwrap();
        let (strokes, _) = page0.decode_strokes(&data).unwrap();
        assert!(!strokes.is_empty());
    }

    #[test]
    fn live_and_saved_have_same_logical_data() {
        let (live_data, live) = load(&test_file("test.note")).unwrap();
        let (saved_data, saved) = load(&test_file("test_after_save.note")).unwrap();

        // File-level metadata
        assert_eq!(live.file_type, saved.file_type);
        assert_eq!(live.signature, saved.signature);
        assert_eq!(live.page_width, saved.page_width);
        assert_eq!(live.page_height, saved.page_height);
        assert_eq!(
            live.header.get_str("APPLY_EQUIPMENT"),
            saved.header.get_str("APPLY_EQUIPMENT"),
        );

        // DIRTY should differ (saved = 0)
        assert_eq!(saved.footer.get_str("DIRTY"), Some("0"));

        // Same page count
        assert_eq!(live.page_count(), saved.page_count());

        for pi in 0..live.page_count() {
            let lp = live.page(&live_data, pi).unwrap();
            let sp = saved.page(&saved_data, pi).unwrap();

            // Page style
            assert_eq!(lp.style, sp.style, "page {pi}: style mismatch");

            // Same layer count, names, types, protocols
            assert_eq!(
                lp.layers.len(),
                sp.layers.len(),
                "page {pi}: layer count mismatch"
            );
            for (li, (ll, sl)) in lp.layers.iter().zip(&sp.layers).enumerate() {
                assert_eq!(ll.name, sl.name, "page {pi} layer {li}: name mismatch");
                assert_eq!(
                    ll.layer_type, sl.layer_type,
                    "page {pi} layer {li}: type mismatch"
                );
                assert_eq!(
                    ll.protocol, sl.protocol,
                    "page {pi} layer {li}: protocol mismatch"
                );
            }
        }
    }

    #[test]
    #[cfg(feature = "stroke")]
    fn live_and_saved_have_same_stroke_data() {
        let (live_data, live) = load(&test_file("test.note")).unwrap();
        let (saved_data, saved) = load(&test_file("test_after_save.note")).unwrap();

        for pi in 0..live.page_count() {
            let lp = live.page(&live_data, pi).unwrap();
            let sp = saved.page(&saved_data, pi).unwrap();

            let (l_strokes, _) = lp.decode_strokes(&live_data).unwrap();
            let (s_strokes, _) = sp.decode_strokes(&saved_data).unwrap();
            assert_eq!(l_strokes, s_strokes, "page {pi}: strokes mismatch");
        }
    }
}
