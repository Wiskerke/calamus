# Supernote `.note` File Format

This document describes the binary format used by Ratta Supernote devices for
their `.note` notebook files. There is no official specification; this is
reconstructed from the following unofficial sources:

- [Investigating the SuperNote Notebook Format](https://walnut356.github.io/posts/inspecting-the-supernote-note-format/) by walnut356
- [supernote-tool](https://github.com/jya-dev/supernote-tool) (Python, Apache 2.0) by jya-dev
- [pysn-digest](https://gitlab.com/mmujynya/pysn-digest) (Python) by mmujynya -- extended fork of supernote-tool
- [snlib](https://github.com/Walnut356/snlib) (Rust) by walnut356 -- exact stroke binary layout
- [SupernoteSharp](https://github.com/nelinory/SupernoteSharp) (.NET) by nelinory

The `.mark` file format (used for PDF annotations) apparantly shares the same structure.


## 1. High-Level Structure

A `.note` file is a flat binary container. It uses text-based metadata blocks to
describe its hierarchical structure and raw binary blobs for content data
(bitmaps, stroke paths, etc.). All multi-byte integers are **little-endian**.

```
+---------------------------+
| File Type (4 bytes ASCII) |  "note" or "mark"
+---------------------------+
| Signature (ASCII string)  |  e.g. "SN_FILE_VER_20230015". Always 20 bytes.
+---------------------------+
| Header metadata block     |  <KEY:VALUE> pairs
+---------------------------+
|                           |
| Data blocks               |  Bitmaps, paths, keywords, etc.
| (length-prefixed blobs)   |  Interleaved with layer/page metadata
|                           |
+---------------------------+
| Footer metadata block     |  Table of contents
+---------------------------+
| "tail" (4 bytes ASCII)    |  Literal string "tail"
+---------------------------+
| Footer address (4 bytes)  |  u32 LE offset to footer block
+---------------------------+
```

### Reading order

1. Read the last 4 bytes of the file to get the footer address.
2. Seek to the footer address and parse the footer metadata block.
3. The footer's `FILE_FEATURE` value gives the header address.
4. The footer's `PAGE*` entries give the page metadata addresses.
5. Each page's metadata points to layers, bitmaps, and stroke data.

### Live editing vs saved files

During live editing on the device, new data is **appended incrementally** to
the file. This means the header may end up near the end of the file, and there
can be large gaps between data blocks.
It also means that old data will remain in the file, and the header which is
visible at the start of the file might no longer be used.

When the note is properly saved (e.g. by closing it), the device performs a
**compaction/defragmentation** pass that repacks all data sequentially, often
reducing file size by 30-40%. The `DIRTY` field in the footer tracks the number
of unsaved modifications (reset to `0` on save).

Saved file layout order:
```
file type → signature → header → style bitmaps → layer bitmaps →
TOTALPATH stroke data → page/layer metadata → footer → "tail" → footer address
```

Parsers should handle both layouts since data blocks can appear at any offset.

### Addressing

All addresses are absolute byte offsets from the start of the file, stored as
**4-byte (u32) little-endian** integers. This limits file size to ~2 GB in
practice (some users have hit this limit with large notebooks).

### Data blocks

Content data (bitmaps, stroke paths, keyword bitmaps, etc.) is stored as
**length-prefixed blobs**:

```
+----------------------------+
| Block length (4 bytes, LE) |  u32, number of bytes following
+----------------------------+
| Block data (N bytes)       |
+----------------------------+
```

Metadata blocks use the same length-prefixed layout, but their content is
UTF-8 encoded text in `<KEY:VALUE>` format (see below).


## 2. File Type and Signature

### File type

The first 4 bytes are an ASCII file type identifier:
- `note` -- notebook file
- `mark` -- PDF annotation markup file

### Signature

Immediately after the file type (at offset 4 for X-series), an ASCII signature
string identifies the format version:

| Signature | Firmware | Notes |
|---|---|---|
| `SN_FILE_ASA_20190529` | B000.432 | Original A5 (starts at offset 0) |
| `SN_FILE_VER_20200001` | C.053 | First X-series |
| `SN_FILE_VER_20200005` | C.077 | |
| `SN_FILE_VER_20200006` | C.130 | |
| `SN_FILE_VER_20200007` | C.159 | |
| `SN_FILE_VER_20200008` | C.237 | |
| `SN_FILE_VER_20210009` | C.291 | |
| `SN_FILE_VER_20210010` | Chauvet 2.1.6 | |
| `SN_FILE_VER_20220011` | Chauvet 2.5.17 | |
| `SN_FILE_VER_20220013` | Chauvet 2.6.19 | |
| `SN_FILE_VER_20230014` | Chauvet 2.10.25 | |
| `SN_FILE_VER_20230015` | Chauvet 3.14.27 | High-res grayscale support |

**Original A5 format**: The signature `SN_FILE_ASA_*` starts at byte offset 0
and encompasses the file type. This is the older, simpler format without layer
support.

**X-series format**: The signature `SN_FILE_VER_*` starts at byte offset 4
(after the file type). This is the modern format with layers.

The signature pattern is `SN_FILE_VER_YYYYNNNN` where YYYY is a year and NNNN
is a sequence number. Signatures with `>= 20230015` support high-resolution
grayscale (X2-series color codes).


## 3. Metadata Blocks

Metadata blocks are length-prefixed UTF-8 strings containing repeated
`<KEY:VALUE>` pairs:

```
<FILE_TYPE:note><APPLY_EQUIPMENT:SN100><DEVICE_DPI:226>...
```

### Parsing rules

- Each pair matches the regex `<([^:<>]+):(.*?)>` (supernote-tool) or
  `<([^:<>]+):([^:<>]*)>` (pysn-digest, slightly stricter).
- Keys are ASCII identifiers (e.g. `PAGESTYLE`, `LAYERBITMAP`).
- Values are typically decimal integers (as ASCII strings), plain strings, or
  `none` for absent values.
- **Duplicate keys**: When the same key appears multiple times, values are
  collected into a list. For example, in the original A5 format the footer
  repeats the `PAGE` key: `<PAGE:1048><PAGE:5672>`, parsed as
  `{"PAGE": ["1048", "5672"]}`. This also occurs for annotations when
  multiple keywords or titles share the same position key.

### Example footer blocks (decoded)

Original A5 format (duplicate `PAGE` keys):
```
<FILE_FEATURE:24><PAGE:1048><PAGE:5672><COVER_0:0>
```

X-series format (numbered `PAGE` keys):
```
<FILE_FEATURE:24><PAGE1:1048><PAGE2:5672><COVER_0:0><STYLE_style_white:892>
```


## 4. Footer (Table of Contents)

The footer is the main index of the file. Its address is stored in the **last 4
bytes** of the file as a u32 LE integer. Immediately before the footer address
is the literal ASCII string `tail`.

### Footer fields

| Key | Value | Description |
|---|---|---|
| `FILE_FEATURE` | address | Points to the header metadata block |
| `PAGE` (A5) or `PAGE1`, `PAGE2`, ... (X-series) | address | Points to each page's metadata block |
| `COVER_0`, `COVER_1`, or `COVER_2` | address | Cover image. `COVER_0` with value `0` = no cover. `COVER_2` is current, `COVER_1` is older. Only one is present. |
| `STYLE_*` | address | Background style bitmaps |
| `KEYWORD_PPPPSSSS` | address | Keyword annotation metadata (PPPP=page, SSSS=sort position) |
| `TITLE_PPPPSSSS` | address | Title annotation metadata |
| `LINKO_PPPPSSSS` | address | Link annotation metadata |
| `DIRTY` | integer | Dirty/modified counter |

For `KEYWORD_`, `TITLE_`, and `LINK` prefixed keys, the page number is encoded
in the key name (4-digit, 1-based) followed by a sort/position string.


## 5. Header

The header block (pointed to by `FILE_FEATURE`) contains notebook-level
properties:

| Key | Example Value | Description |
|---|---|---|
| `FILE_TYPE` | `note` | File type |
| `FILE_ID` | UUID string | Unique file identifier |
| `FILE_RECOGN_TYPE` | `0` or `1` | `1` = real-time recognition notebook |
| `APPLY_EQUIPMENT` | `SN100`, `A6X`, `N5` | Device model (`N5` = A5X2) |
| `DEVICE_DPI` | `226` | Device DPI |
| `SOFT_DPI` | `226` | Software DPI |
| `FILE_PARSE_TYPE` | string | Parse type identifier |
| `RATTA_ETMD` | string | Device metadata |
| `APP_VERSION` | `0` | Application version |
| `MODULE_LABEL` | `none` | Module label |
| `PDFSTYLE` | `none` | PDF style |
| `PDFSTYLEMD5` | `0` | PDF style hash |
| `STYLEUSAGETYPE` | `0` | Style usage type |
| `HORIZONTAL_CHECK` | `0` | Horizontal check flag |
| `IS_OLD_APPLY_EQUIPMENT` | `0` or `1` | Old equipment flag |
| `ANTIALIASING_CONVERT` | `2` | Antialiasing conversion setting |
| `HIGHLIGHTINFO` | address | (pysn-digest) Highlight info block, base64-encoded JSON |

### Page dimensions by device

| `APPLY_EQUIPMENT` | Width | Height | Device |
|---|---|---|---|
| Default (A5, A5X, A6X, etc.) | 1404 | 1872 | |
| `N5` (A5X2) | 1920 | 2560 | |
| `N6` (Nomad) | 1404 | 1872 | Nomad |

Note: `DEVICE_DPI` and `SOFT_DPI` may be `0` on newer devices (observed on
Nomad/N6).


## 6. Pages

Each page has a metadata block pointed to by the footer's `PAGE*` entries.

### Page metadata fields

| Key | Description |
|---|---|
| `PAGEID` | Unique page identifier (UUID) |
| `PAGESTYLE` | Background style name (e.g. `style_white`, `style_lined`, `user_*`) |
| `PAGESTYLEMD5` | Hash of custom style (`0` if none) |
| `LAYERSEQ` | Comma-separated layer render order, **top-to-bottom** (e.g. `LAYER3,MAINLAYER,LAYER2,LAYER1,BGLAYER`) |
| `LAYERINFO` | Base64-encoded JSON with layer visibility, names, and ordering (see §7) |
| `ORIENTATION` | `1000` = vertical (portrait), `1090` = horizontal (landscape) |
| `RECOGNSTATUS` | `0` = none, `1` = done, `2` = running |
| `TOTALPATH` | Address of stroke/path data block |
| `RECOGNFILE` | Address of recognition file data |
| `RECOGNTEXT` | Address of recognition text data |
| `RECOGNTYPE` | Recognition type |
| `RECOGNFILESTATUS` | Recognition file status |
| `RECOGNLANGUAGE` | Recognition language (e.g. `none`) |
| `THUMBNAILTYPE` | Thumbnail type |
| `EXTERNALLINKINFO` | Address of external link info (0 = none) |
| `IDTABLE` | Address of ID table (0 = none) |
| `PAGETEXTBOX` | Address of text box data (0 = none) |
| `DISABLE` | Disable flag (`none`) |

**Original A5 (non-layered) pages** additionally have:
| Key | Description |
|---|---|
| `DATA` | Address of the bitmap data |
| `PROTOCOL` | Compression protocol (`SN_ASA_COMPRESS` or `RATTA_RLE`) |

**X-series (layered) pages** additionally have:
| Key | Description |
|---|---|
| `MAINLAYER` | Address of main layer metadata |
| `LAYER1` | Address of layer 1 metadata |
| `LAYER2` | Address of layer 2 metadata |
| `LAYER3` | Address of layer 3 metadata |
| `BGLAYER` | Address of background layer metadata |


## 7. Layers

Each page in the X-series format has exactly **5 layers**, always in this order:

| Index | Name | Purpose |
|---|---|---|
| 0 | `MAINLAYER` | Primary drawing layer |
| 1 | `LAYER1` | Additional layer 1 |
| 2 | `LAYER2` | Additional layer 2 |
| 3 | `LAYER3` | Additional layer 3 |
| 4 | `BGLAYER` | Background layer (template/style) |

### Layer metadata fields

| Key | Description |
|---|---|
| `LAYERNAME` | Layer name (matches the layer key names above) |
| `LAYERBITMAP` | Address of the layer's bitmap data |
| `LAYERPROTOCOL` | Compression protocol for this layer's bitmap |
| `LAYERTYPE` | Layer type (`NOTE` for note layers, `MARK` for markup layers) |
| `LAYERVECTORGRAPH` | Address of vector graph data (0 = none) |
| `LAYERRECOGN` | Address of layer recognition data (0 = none) |
| `LAYERPATH` | Address of layer-specific path data (0 = none) |

### Layer info (LAYERINFO)

The `LAYERINFO` field contains base64-encoded JSON (with `#` replaced by `:` in
the raw metadata) describing each layer's properties. The array order matches
`LAYERSEQ` (top-to-bottom). Each entry has:

```json
[
  {
    "layerId": 3,
    "name": "custom name",
    "isBackgroundLayer": false,
    "isCurrentLayer": false,
    "isVisible": true,
    "isDeleted": false,
    "isAllowAdd": false,
    "isAllowUp": false,
    "isAllowDown": true
  },
  {
    "layerId": 0,
    "name": "Main Layer",
    "isBackgroundLayer": false,
    "isCurrentLayer": true,
    "isVisible": true,
    "isDeleted": false,
    "isAllowAdd": false,
    "isAllowUp": true,
    "isAllowDown": true
  },
  {
    "layerId": -1,
    "name": "Background Layer",
    "isBackgroundLayer": true,
    "isCurrentLayer": false,
    "isVisible": true,
    "isDeleted": false,
    "isAllowAdd": false,
    "isAllowUp": false,
    "isAllowDown": false
  }
]
```

The `layerId` maps to the stroke `stroke_layer` field: 0=MAINLAYER, 1=LAYER1,
2=LAYER2, 3=LAYER3, -1=BGLAYER. Users can rename layers; the `name` field
stores the display name. `isAllowUp`/`isAllowDown` reflect whether the layer
can be reordered further in each direction.

### Layer rendering order

`LAYERSEQ` lists layers **top-to-bottom** (e.g. `LAYER3,MAINLAYER,LAYER2,LAYER1,BGLAYER`).
Layers are composited bottom-to-top. Each layer is rendered independently —
erasers only affect strokes within the same layer. Non-transparent pixels from
upper layers opaquely overwrite lower layers (0xFF = transparent, all other
values are opaque).

### Workaround: duplicate MAINLAYER name

Some firmware versions produce files where the BGLAYER is incorrectly named
`MAINLAYER`. The workaround is: if a second layer named `MAINLAYER` is
encountered, treat it as `BGLAYER`.


## 8. Bitmap Encoding

Layer bitmaps are stored as length-prefixed binary blobs. The compression
protocol is indicated by the `PROTOCOL` (non-layered) or `LAYERPROTOCOL`
(layered) metadata field.

### 8.1 `SN_ASA_COMPRESS` (Flate/zlib) -- Original A5

Used by the original Supernote A5. The bitmap is zlib-compressed.

**Decoding steps:**
1. zlib decompress the data.
2. Interpret as a 2D array of **u16** values, shaped as 1404 x 1888 (width x height).
3. Rotate 90 degrees clockwise.
4. Crop the bottom 16 rows (result: 1872 x 1404, i.e. height x width).

**Color codes (u16):**

| Code | Color |
|---|---|
| `0x0000` | Black |
| `0x2104` | Dark gray |
| `0xE1E2` | Gray |
| `0xFFFF` | Background (white/transparent) |

### 8.2 `RATTA_RLE` -- X-series Run-Length Encoding

The primary encoding for X-series devices. Data is a stream of
**(color_code, length)** byte pairs with a multi-byte length extension scheme.

**Color codes (u8) -- X-series (non-highres):**

All codes are symbolic and map to fixed palette values.

| Code | Color | Output value |
|---|---|---|
| `0x61` | Black | `0x00` |
| `0x62` | Background (transparent) | `0xFF` |
| `0x63` | Dark gray | `0x9D` |
| `0x64` | Gray | `0xC9` |
| `0x65` | White | `0xFE` |
| `0x66` | Marker black | `0x00` |
| `0x67` | Marker dark gray | `0x9D` |
| `0x68` | Marker gray | `0xC9` |

**Color codes (u8) -- X2-series (highres, signature >= `20230015`):**

Only a small set of codes are symbolic. The marker codes were renumbered (0x9E,
0xCA) and the old X-series codes 0x63/0x64 now map to different compat values.
All other byte values are treated as **literal grayscale intensities**, enabling
smoother anti-aliased stroke edges.

| Code | Color | Output value |
|---|---|---|
| `0x61` | Black | `0x00` |
| `0x62` | Background (transparent) | `0xFF` |
| `0x63` | Dark gray (compat) | `0x30` |
| `0x64` | Gray (compat) | `0x50` |
| `0x65` | White | `0xFE` |
| `0x66` | Marker black | `0x00` |
| `0x9D` | Dark gray | `0x9D` |
| `0x9E` | Marker dark gray | `0x9D` |
| `0xC9` | Gray | `0xC9` |
| `0xCA` | Marker gray | `0xC9` |
| other | Literal grayscale | same as code |

**Length decoding algorithm:**

The RLE stream is consumed as byte pairs `(color, length_byte)`:

1. **If `length_byte == 0xFF`**: Special marker.
   - Run length = `0x4000` (16384 pixels).
   - Exception: if `all_blank` hint is set, length = `0x400` (1024 pixels).

2. **If `length_byte` has bit 7 set (`& 0x80 != 0`)**: Multi-byte length.
   - Hold this pair. Read the next `(color2, length2)` pair.
   - If `color2 == color` (same color continues):
     - Combined length = `1 + length2 + (((length_byte & 0x7F) + 1) << 7)`
   - If `color2 != color` (different color):
     - First run: `color` with length `((length_byte & 0x7F) + 1) << 7`
     - Then process `(color2, length2)` normally.

3. **Otherwise** (bit 7 clear):
   - Run length = `length_byte + 1` (range 1-128).

4. **End of stream with held data**: Adjust the tail length to fit the
   remaining expected pixels using the formula
   `((length & 0x7F) + 1) << i` for decreasing `i` from 7 to 0.

The expected total pixel count is `page_width * page_height`.

### 8.3 PNG -- Custom backgrounds

When `PAGESTYLE` starts with `user_`, the BGLAYER bitmap is stored as raw PNG
data. Use a standard PNG decoder.


## 9. Stroke/Path Data (TOTALPATH)

The `TOTALPATH` field in page metadata points to a length-prefixed blob
containing all stroke (pen path) data for the page. This is the structured
vector data that enables per-stroke rendering.

> **Note**: This section is based on reverse engineering by
> [walnut356](https://walnut356.github.io/posts/inspecting-the-supernote-note-format/)
> and their [snlib](https://github.com/Walnut356/snlib) Rust library, which
> provides exact struct layouts. An ImHex pattern file is available in that repo.
> Field layouts are confirmed for the Nomad; other devices may vary.

### 9.1 Container structure

The `TOTALPATH` address points to a standard length-prefixed data block (see
section 1). After stripping the block length prefix, the payload contains:

```
+-------------------------------+
| Stroke count (u32 LE)         |  Number of strokes following
+-------------------------------+
| For each stroke:              |
|   +---------------------------+
|   | Stroke size (u32 LE)      |  Byte length of this stroke's data
|   +---------------------------+
|   | Stroke data (N bytes)     |  See below
|   +---------------------------+
+-------------------------------+
```

### 9.2 Coordinate system

The Supernote uses two coordinate spaces:

**Pixel coordinates**: Native display resolution.
- Standard (Nomad, A5X, A6X): 1404 x 1872
- A5X2: 1920 x 2560

**Screen/canvas coordinates**: Physical dimensions in 10-micrometer increments.
- Nomad: 11,864 x 15,819 units (~118.6 mm x 158.2 mm)

Stroke point data uses screen coordinates as **i32** values in **(y, x)** order.
The origin is at the top-left. When converting to pixel coordinates:

```
pixel_x = screen_x * pixel_width / physical_width
pixel_y = screen_y * pixel_height / physical_height
```

The physical dimensions are stored per-stroke in the stroke header
(`screen_width`, `screen_height`) and again in a later section (`pixel_width`,
`pixel_height`).

### 9.3 Stroke header (StrokeConfig)

Each stroke begins with a fixed-size header. On the Nomad (N6) this is
**208 bytes**; other devices may differ.

| Offset | Size | Type | Field | Description |
|--------|------|------|-------|-------------|
| 0 | 4 | u32 | `pen` | Pen type (see 9.8) |
| 4 | 4 | u32 | `color` | Grayscale color (see 9.8) |
| 8 | 4 | u32 | `thickness` | Pen thickness (see 9.8) |
| 12 | 4 | u32 | `rec_mod` | Unknown ("name via supernote partner app") |
| 16 | 4 | u32 | `unk_1` | Usually 0 |
| 20 | 4 | u32 | `font_height` | Default 32 |
| 24 | 4 | u32 | `unk_2` | Usually `0xFFFFFFFF` |
| 28 | 4 | u32 | `page_num` | 1-indexed page number |
| 32 | 4 | u32 | `unk_3` | Usually 0 |
| 36 | 4 | u32 | `unk_4` | Usually 0 |
| 40 | 4 | u32 | `unk_5` | Always 5000 (observed) |
| 44 | 4 | u32 | `stroke_layer` | 0-indexed layer (excluding background) |
| 48 | 52 | char[52] | `stroke_kind` | Null-padded C string: `"others"` (freehand) or `"straightLine"` |
| 100 | 8 | ScreenCoord | `bounding_tl` | Bounding box top-left (y, x) |
| 108 | 8 | ScreenCoord | `bounding_mid` | Bounding box midpoint (y, x) |
| 116 | 8 | ScreenCoord | `bounding_br` | Bounding box bottom-right (y, x) |
| 124 | 4 | u32 | `unk_6` | Usually 26 |
| 128 | 4 | u32 | `screen_height` | Physical height in 10um units (15819 on Nomad) |
| 132 | 4 | u32 | `screen_width` | Physical width in 10um units (11864 on Nomad) |
| 136 | 52 | char[52] | `doc_kind` | Always `"superNoteNote"` (null-padded) |
| 188 | 4 | u32 | `emr_point_axis` | Always 1 |
| 192 | 16 | u32[4] | `unk_7` | Unknown |

Where `ScreenCoord` is:
```
struct ScreenCoord {
    y: i32,  // little-endian
    x: i32,  // little-endian
}
```

### 9.4 Stroke body -- variable-length arrays

After the fixed header, the stroke body consists of a sequence of
length-prefixed arrays and fixed-size sections, always in this order:

#### 1. Disable area list
```
count: u32          -- number of triangular areas
data:  [ScreenCoord; 3] * count   -- 3 vertices per triangle (24 bytes each)
```

#### 2. Points
```
count: u32          -- number of points
data:  ScreenCoord * count        -- (y, x) pairs in screen coordinates (8 bytes each)
```

#### 3. Pressures
```
count: u32          -- same as point count
data:  u16 * count                -- pressure values, range 0-2048 observed
```
Pressure is used to modulate stroke thickness:
`effective_thickness = thickness * min(pressure, 2048) / 2048.0`

#### 4. Tilts
```
count: u32          -- same as point count
data:  Vector * count             -- pen tilt per point (4 bytes each)
```
Where `Vector` is:
```
struct Vector {
    y: i16,  // little-endian
    x: i16,  // little-endian
}
```
Convert to angle: `atan2(y as f32, x as f32)`.
Coordinate frame: 270=up, 180=right, 90=down, 0=left.
NeedlePoint and Marker pens typically have uniform tilt across the stroke.

#### 5. Draw flags
```
count: u32          -- same as point count
data:  u8 * count                 -- typically 0x01
```

#### 6. EPA points (screen-space)
```
count: u32
data:  ScreenCoord * count        -- (y, x) pairs
```

#### 7. EPA grays
```
count: u32
data:  i32 * count                -- grayscale values
```

#### 8. Section1 -- 52 bytes fixed
| Size | Type | Field | Description |
|------|------|-------|-------------|
| 4 | u32 | `unk_8` | Often -99 if eraser stroke |
| 4 | u32 | `unk_9` | Non-zero (0x61) if lasso-moved/rotated |
| 4 | u32 | `stroke_uid` | Unique stroke ID, monotonically increasing, never recycled |
| 4 | u32 | `unk_10` | Unused |
| 4 | u32 | `unk_11` | Unused |
| 16 | u32[4] | `unk_12` | Observed: [0, 0, 1, 1] |
| 16 | u32[4] | `unk_13` | Observed: [0, 0, 1, 1] |

#### 9. Control nums
```
count: u32
data:  i32 * count
```

#### 10. Section2 -- 10 bytes fixed
| Size | Type | Field |
|------|------|-------|
| 8 | u32[2] | `unk_14` |
| 1 | u8 | `unk_15` |
| 1 | u8 | `render_flag` |

#### 11. Point contours (nested arrays)
```
outer_count: u32
for each outer:
    inner_count: u32
    data: PixelCoord * inner_count    -- (x, y) as f32 pairs (8 bytes each)
```
Where `PixelCoord` is `(x: f32, y: f32)` -- note the x-first order, unlike
ScreenCoord.

#### 12. Unknown 16-byte chunks
```
count: u32
data:  [u8; 16] * count
```

#### 13. Section3 -- 17 bytes fixed
| Size | Type | Field | Description |
|------|------|-------|-------------|
| 4 | u32 | `unk_18` | |
| 8 | u64 | `unk_19` | |
| 1 | u8 | `unk_20` | |
| 4 | i32 | `rotation_degrees` | Degrees rotated via lasso tool |

#### 14. Unknown i32 array
```
count: u32
data:  i32 * count
```

#### 15. Section4 -- 13 bytes fixed
| Size | Type | Field | Description |
|------|------|-------|-------------|
| 4 | u32 | `pixel_width` | 1404 on Nomad |
| 4 | u32 | `pixel_height` | 1872 on Nomad |
| 4 | u32 | `unk_23` | |
| 1 | u8 | `unk_24` | |

#### 16. Three sized strings
```
len: u32, data: u8 * len    -- typically "none"
len: u32, data: u8 * len    -- typically "none"
len: u32, data: u8 * len    -- typically empty
```

#### 17. Final fields
```
unk_25: u32                  -- single value
mark_pen_d_fill_dir:
    count: u32
    data: PixelCoord * count -- directional fill for marker pen
```

### 9.5 Pen types, colors, and thickness

**Pen types** (`pen` field):

| Value | Pen |
|-------|-----|
| 1 | Ink Pen (pressure-sensitive) |
| 10 | Needle Point (uniform pressure/tilt) |
| 11 | Marker (uniform pressure) |

Other pen types exist but are not yet mapped.

**Colors** (`color` field, 8-bit grayscale):

| Value | Color |
|-------|-------|
| 0 | Black |
| 1 | Marker black |
| 158 | Dark grey |
| 202 | Light grey |
| 254 | White (ink) |
| 255 | Eraser (special: masks underlying strokes) |

**Thickness** (`thickness` field):

| Value | UI Size |
|-------|---------|
| 200 | Point 1 |
| 300 | Point 2 |
| 400 | Point 3 |
| 500 | Point 4 |
| 700 | Point 5 |
| 800 | Point 6 |
| 1000 | Point 7 |
| 1100 | Point 8 |
| 1200 | Point 9 |
| 1300 | Size 1 |
| 2400 | Size 2 |
| 3800 | Marker |

### 9.6 Eraser strokes

Erasers are represented as regular strokes with color value `255` (vs `254` for
white ink). The eraser stroke acts as a mask against underlying strokes. The
device firmware performs cleanup to remove fully-masked strokes and their
corresponding eraser traces. Eraser strokes have `unk_8 = -99` in Section1.

### 9.7 Per-layer paths (LAYERPATH)

Individual layers may have their own path data referenced by `LAYERPATH` fields.
Each stroke's `stroke_layer` field (offset 44 in the header) indicates which
layer it belongs to (0-indexed, excluding the background layer). The exact
relationship between `TOTALPATH` (page-level) and per-layer paths is not fully
documented.


## 10. Annotations

### 10.1 Keywords

Keywords are annotation markers on pages. The footer contains `KEYWORD_PPPPSSSS`
entries (PPPP = 4-digit 1-based page number, SSSS = vertical position for
sorting).

**Keyword metadata fields:**

| Key | Description |
|---|---|
| `KEYWORD` | The keyword text |
| `KEYWORDPAGE` | Page number (1-based) |
| `KEYWORDRECT` | Bounding rectangle as `left,top,width,height` |
| `KEYWORDRECTORI` | Original rectangle |
| `KEYWORDSITE` | Address of keyword content bitmap |

### 10.2 Titles

Similar to keywords, `TITLE_PPPPSSSS` entries in the footer.

| Key | Description |
|---|---|
| `TITLEBITMAP` | Address of title bitmap |
| `TITLERECTORI` | Rectangle as `left,top,width,height` |

### 10.3 Links

`LINKO_PPPPSSSS` entries in the footer for page links, file links, and web
links.

| Key | Description |
|---|---|
| `LINKTYPE` | `0` = page link, `1` = file link, `4` = web link |
| `LINKINOUT` | `0` = outgoing, `1` = incoming |
| `LINKRECT` | Bounding rectangle as `left,top,width,height` |
| `LINKTIMESTAMP` | Creation timestamp |
| `LINKFILE` | Base64-encoded file path or URL |
| `LINKFILEID` | Target file's FILE_ID (`none` if N/A) |
| `PAGEID` | Target page's PAGEID (`none` if N/A) |
| `LINKBITMAP` | Address of link visual bitmap |

### 10.4 Cover

The notebook cover is referenced from the footer:
- `COVER_2` (preferred), `COVER_1`, or `COVER_0` (value `0` = no cover).


## 11. Real-Time Recognition

Notebooks with `FILE_RECOGN_TYPE=1` in the header support real-time handwriting
recognition. Each page may have:

- `RECOGNFILE` -- Address of recognition file data
- `RECOGNTEXT` -- Address of recognition text data (base64-encoded JSON)
- `RECOGNSTATUS` -- `0`=none, `1`=done, `2`=running

The recognition text is base64-encoded JSON with an `elements` array. Each
element has a `type` (e.g. `Text`) and a `label` (recognized text), plus
spatial information in a `words` array with `bounding-box` coordinates.


## 12. Supported Devices

| Device | Equipment ID | Signature Format | Notes |
|---|---|---|---|
| Supernote A5 | (in signature) | `SN_FILE_ASA_*` | Original, no layers |
| Supernote A5X | `SN100` etc. | `SN_FILE_VER_*` | X-series with layers |
| Supernote A6X | various | `SN_FILE_VER_*` | X-series with layers |
| Supernote A5X2 | `N5` | `SN_FILE_VER_*` | Higher resolution (1920x2560), X2 color codes |
| Supernote A6X2 | various | `SN_FILE_VER_*` | X2 color codes |
| Supernote Nomad | `N6` | `SN_FILE_VER_*` | Canvas coords: 11864x15819 |
| Supernote Manta | various | `SN_FILE_VER_*` | (pysn-digest reports support) |


## 13. Open Questions and Unknowns

The following aspects of the format are not fully understood:

- **TOTALPATH on non-Nomad devices**: The stroke binary layout in section 9 is
  confirmed for the Nomad. Other devices likely share the same layout but may
  differ in details (e.g. different physical dimensions, additional pen types).
- **LAYERPATH vs TOTALPATH**: The relationship and exact structure of per-layer
  path data is unclear. Each stroke has a `stroke_layer` field, but LAYERPATH
  blocks may duplicate or subset the TOTALPATH data.
- **Incomplete pen type enumeration**: Only NeedlePoint (10), InkPen (1), and
  Marker (11) are confirmed. Other pen tools (fountain, calligraphy, etc.)
  have unknown numeric values.
- **Unknown stroke fields**: Many `unk_*` fields in the stroke body (sections
  1-4) have unknown purpose. Some correlate with lasso operations, erasure
  state, or rendering hints.
- **Eraser cleanup logic**: The firmware's algorithm for removing fully-erased
  strokes is not documented.
- **Newer signature versions**: Firmware updates may introduce new signature
  versions with additional features or changed layouts.
- **All possible metadata keys**: Only commonly-observed keys are documented.
  Other keys may exist in specific firmware versions or file configurations.
- **`all_blank` RLE hint**: The special handling for `0xFF` length markers when
  the background layer uses `style_white` with a specific block size
  (`0x140E` bytes) is a quirk that may be firmware-version-dependent.
- **EPA points/grays**: The exact purpose of these arrays is unclear -- they
  may be a pre-computed contour approximation.
- **Point contours**: Nested PixelCoord arrays likely represent vectorized
  stroke outlines, but the rendering algorithm is not documented.


## References

- [supernote-tool](https://github.com/jya-dev/supernote-tool) -- Primary
  reference implementation (Python)
- [pysn-digest](https://gitlab.com/mmujynya/pysn-digest) -- Extended fork with
  additional device support (Python)
- [snlib](https://github.com/Walnut356/snlib) -- Rust library with exact
  stroke binary layout and ImHex pattern file
- [Investigating the SuperNote Notebook Format](https://walnut356.github.io/posts/inspecting-the-supernote-note-format/)
  -- Detailed reverse engineering of the stroke/path format
- [SupernoteSharp](https://github.com/nelinory/SupernoteSharp) -- .NET
  implementation
- [GoSNare](https://github.com/alefaraci/GoSNare) -- Go tool for vectorized
  PDF rendering
