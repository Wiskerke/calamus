# Line Width Study — `testfiles/linestudy.note`

Analysis of pen widths from a systematic line study file on a Nomad (N6).
All measurements are from the device-rendered bitmap (ground truth).

## Test File Structure

| Page | Content | Strokes |
|------|---------|---------|
| 0 | NeedlePoint at 5 UI sizes + InkPen at 3 sizes (hard/light push) | 127 |
| 1 | Marker (one line, single size) | 6 |
| 2 | Eraser at 4 sizes (heavy/light push) on black marker background | 211 |

## How Width Data is Stored

Each stroke carries three relevant fields:

- **`pen`** — pen type: NeedlePoint=10, InkPen=1, Marker=11
- **`thickness`** — integer, determined by the UI size selection (see mapping below)
- **`pressures`** — per-point array, 12-bit range (0..4095)

Not all pens use pressure for width. NeedlePoint and Marker have constant width
regardless of pressure. InkPen and Eraser use pressure to modulate width per-point.

## UI Size to Stored Thickness Mapping

### NeedlePoint (pen=10)

| UI label | thickness |
|----------|-----------|
| 0.1      | 200       |
| 0.3      | 400       |
| 0.6      | 800       |
| 1.0      | 1300      |
| 2.0      | 2400      |

### InkPen (pen=1)

| UI label | thickness |
|----------|-----------|
| 0.5      | 200       |
| 0.7      | 400       |
| 1.0      | 600       |

These are the only three sizes available in the UI.

### Marker (pen=11)

The Supernote UI only offers a single size for the Marker: thickness=3800.

### Eraser (color=255, any pen)

| UI label   | thickness |
|------------|-----------|
| smallest   | 400       |
| 2nd size   | 1000      |
| 3rd size   | 1600      |
| 4th size   | 2200      |

Eraser sizes are linearly spaced: `400 + 600*n` for n=0,1,2,3.

## Measured Pixel Widths from Device Bitmap

### NeedlePoint — constant width

Measurements taken across x=100..500, median values. NeedlePoint is **not
pressure-sensitive** — the bitmap shows uniform width even though pressure
data varies from ~100 to ~2400 within each stroke.

| thickness | measured pixels | physical units |
|-----------|----------------|----------------|
| 200       | 2.8            | 23.7           |
| 400       | 4.8            | 40.6           |
| 800       | 9.0            | 76.1           |
| 1300      | 14.1           | 119.1          |
| 2400      | 25.2           | 212.9          |

Physical = pixels × (11864/1404).

### InkPen — pressure-sensitive

Width varies along the stroke with pressure. Measured at x=300.

| thickness | hard push | light push |
|-----------|-----------|------------|
| 200       | ~6 px     | ~3 px      |
| 400       | ~8 px     | ~3 px      |
| 600       | ~10 px    | ~3 px      |

"Hard push" strokes reach pressures of 2000–4000; "light push" stays around 200–700.

### Marker — constant width

| thickness | measured pixels |
|-----------|----------------|
| 3800      | 39.1           |

Only one data point available.

## Pressure Details

- Raw range: 0..4095 (12-bit ADC)
- Values above ~2048 do not appear to increase width further
  (pressure is clamped to 2048 for the modifier calculation)
- Pressure values are recorded even for NeedlePoint/Marker, but don't affect width
- Typical ranges observed:
  - Hard push: 2000–4095
  - Normal writing: 800–1500
  - Light touch: 100–600

## Formula Analysis

### NeedlePoint

Best power-law fit to the 5 measured data points:

```
width_physical = 0.21 × thickness^0.89
```

| thickness | measured px | fitted px | error |
|-----------|-------------|-----------|-------|
| 200       | 2.8         | 2.7       | 3.9%  |
| 400       | 4.8         | 5.0       | 3.8%  |
| 800       | 9.0         | 9.2       | 2.5%  |
| 1300      | 14.1        | 14.2      | 0.7%  |
| 2400      | 25.2        | 24.5      | 2.9%  |

The snlib formula `t/20` seems ~45–58% too thin.

### Marker

The UI only offers a single Marker size (t=3800), so there's only one data point.
The NeedlePoint formula `0.21 × t^0.89` predicts 38.1 px (2.6% off), suggesting
the same width curve applies to both pen types on the Nomad.

### InkPen

The current calamus formula:

```
modifier = (min(pressure, 2048) / 2048) ^ 0.55
width_physical = thickness^0.63 × 1.5 × modifier
```

At max pressure (≥2048, modifier=1.0):

| thickness | formula px | measured hard push px |
|-----------|-----------|----------------------|
| 200       | 5.0       | ~6.0                 |
| 400       | 7.7       | ~8.0                 |
| 600       | 10.0      | ~10.0                |

The formula matches well at t=600, slightly undershoots at t=200 (the "hard
push" likely exceeded 2048 at the measurement point, where the formula clamps).

The `pow(0.55)` pressure curve boosts visibility at low pressure — important
for stroke beginnings/endings to remain visible rather than disappearing.

The snlib formula for InkPen is linear: `thickness × (pressure/2048) × (1404/11864)`,
which gives significantly thinner lines than the bitmap.

### Eraser

Erasers use color=255 and are rendered the same way as InkPen (pressure-sensitive
variable-width strokes). The same InkPen formula applies. When used in SVG masks,
the stroke-width determines how much is erased.

## Coordinate System Reminder

Our SVG uses nested viewBoxes:
- **Outer**: `0 0 1404 1872` (pixel coordinates)
- **Inner**: `0 0 11864 15819` (physical coordinates, ~10-micrometer units)

All `stroke-width` values in the inner SVG are in **physical units**.
To convert: `physical = pixels × (11864/1404)`, or `pixels = physical × (1404/11864)`.

## Duplicate Pen-Down Points

Every InkPen stroke starts with **two points at the exact same (x,y) but different
pressures**. The device records the initial contact (low pressure) and the ramp-up
(higher pressure) before the pen physically moves. Example from 0.5 hard push:

```
[0] x=5382 y=6611  pressure=388   (initial contact)
[1] x=5382 y=6611  pressure=1079  (pressure ramping up)
[2] x=5376 y=6607  pressure=1145  (pen starts moving)
```

This pattern is universal — all 92 InkPen strokes on page 0 exhibit it. Some strokes
also have near-duplicate points at the end (pen lifting).

NeedlePoint strokes show the same duplicate start point, but it's harmless for
constant-width rendering (just a redundant moveto in the polyline).

### Impact on polygon rendering

The filled-polygon renderer computes a perpendicular normal at each point to offset
the left and right edges. When two consecutive points share the same position, the
tangent vector is (0,0), making the normal undefined. Both edges collapse to the
center point, producing a degenerate start cap (a zero-size arc instead of a
semicircle). The polygon then fans out into a sharp V at point 2 instead of a
round cap.

**Fix**: When computing the tangent at endpoints, skip past duplicate-position
points to find the first (or last) point with a meaningfully different position.
This gives a proper tangent direction even when the first two points overlap.

### How the device handles it

The device likely uses a stamp/brush approach — rendering each point as an individual
circle at its pressure-dependent radius. Overlapping circles at the same position
just produce the union, naturally forming a round start. No tangent calculation needed.

## Polygon Self-Intersection in Tight Curves

A single outline polygon (left edge forward, right edge backward) self-intersects
when the stroke curves tighter than its width — common in handwriting. The polygon
edges cross, and the default fill rule leaves white holes inside what should be
solid ink.

Example: a "period" dot drawn as a small circular gesture produces a polygon where
the edges jump back and forth wildly, creating a broken/hollow shape instead of a
solid dot.

**Fix**: Render each InkPen stroke as a polygon+circles hybrid within a single
`<path>` element:

1. The outline polygon handles smooth variable-width transitions between points
2. A filled circle at every sample point (as subpaths) fills any holes from
   self-intersection, matching the device's stamp rendering

Both layers use the same fill, so their union is seamless. The circles add ~2×
to the SVG path data size but eliminate all visible white gaps.

## Marker End Cap Behavior

On the device, the Marker pen uses a different brush shape than NeedlePoint or InkPen:

1. **Pen-down (stationary)**: A circle appears at the touch point, with diameter
   equal to the marker width — identical to a round linecap.

2. **Movement begins**: The device switches to square stamps oriented in the
   movement direction. A half-square is drawn on the **outward** side of the
   initial circle (away from the stroke body), turning the outside edge flat
   while the inward half remains round.

3. **Pen-up**: The same happens at the end — a half-square extends outward in
   the final movement direction.

The result is that marker strokes have **flat/square outer edges** at both ends,
with the inward side staying round. This is visually distinct from NeedlePoint
(fully round caps) and InkPen (pressure-tapered ends).

### SVG implementation

We render marker strokes as:

1. A standard stroked `<path>` with `stroke-linecap="round"` (same as
   NeedlePoint), which gives the circular base shape at each endpoint.

2. Two additional filled half-rectangles — one at each end — that overlay the
   outward half of the round cap to flatten it. Each rectangle:
   - Spans the full stroke width perpendicular to the movement direction
   - Extends outward by half the stroke width (from the endpoint center)
   - Uses a stable movement direction calculated ~10% into the stroke,
     looking past any pen-down/pen-up wobble points

The exact outward direction may differ slightly from the device's calculation,
since the trailing points at the end of a stroke can be averaged or weighted
in different ways. The current approximation is visually close.

## Recommendations for SVG Implementation

1. **NeedlePoint**: Use `0.21 × thickness^0.89` for physical stroke-width.
   No pressure modulation needed. Round linecaps.

2. **Marker**: Use the same width formula as NeedlePoint (`0.21 × t^0.89`).
   Only one size exists in the UI (t=3800), and it matches the NeedlePoint curve.
   Round linecaps with half-square end cap overlays (see above).

3. **InkPen**: Keep the current pressure formula. Consider bumping the base
   width slightly — the `0.63` power and `1.5` coefficient match well at larger
   thicknesses but underestimate at t=200 by ~17%.

4. **Eraser**: Same formula as InkPen (already the case).

5. **Pressure clamping**: Continue clamping to 2048. Values above this don't
   increase width in the device bitmap.
