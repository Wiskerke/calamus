//! Rectangle detection and SVG splitting for Supernote notes.
//!
//! Detects rectangles drawn by the user and classifies strokes/textboxes as either
//! belonging to a rectangle (extracted as separate SVGs) or the main page.

use crate::format::stroke::{COLOR_ERASER, ScreenCoord, Stroke, TextBox};
use std::collections::HashSet;

/// Configuration for rectangle detection.
#[derive(Debug, Clone)]
pub struct SplitConfig {
    /// Minimum rectangle height in physical units (100 = 1mm).
    pub min_height: i32,
}

impl Default for SplitConfig {
    fn default() -> Self {
        Self {
            min_height: 1500, // 15mm
        }
    }
}

/// A detected rectangle with its bounding box in raw physical coordinates.
#[derive(Debug, Clone)]
pub struct DetectedRectangle {
    /// The four corners of the detected rectangle (may not be axis-aligned).
    pub corners: [ScreenCoord; 4],
    /// Axis-aligned bounding box minimum (min_y, min_x).
    pub bbox_min: ScreenCoord,
    /// Axis-aligned bounding box maximum (max_y, max_x).
    pub bbox_max: ScreenCoord,
    /// Index of the stroke that was detected as a rectangle.
    pub stroke_index: usize,
}

/// Result of splitting a page into rectangles and content assignments.
#[derive(Debug)]
pub struct SplitResult {
    /// Detected rectangles, sorted top-to-bottom, left-to-right.
    pub rectangles: Vec<DetectedRectangle>,
    /// For each stroke: None = main SVG, Some(idx) = rectangle idx.
    /// Rectangle strokes themselves are excluded (not in main, not in any rect).
    pub stroke_assignments: Vec<Option<usize>>,
    /// For each textbox: None = main SVG, Some(idx) = rectangle idx.
    pub textbox_assignments: Vec<Option<usize>>,
}

/// Detect rectangles and classify content on a page.
pub fn split_page(
    strokes: &[Stroke],
    textboxes: &[TextBox],
    phys_w: i64,
    phys_h: i64,
    pixel_w: u32,
    config: &SplitConfig,
) -> SplitResult {
    // Step 1: Detect rectangles from strokes
    let mut rectangles = detect_rectangles(strokes, config);

    // Step 2: Resolve nesting (if a rectangle's centroid is inside another, demote it)
    resolve_nesting(&mut rectangles);

    // Step 3: Sort rectangles top-to-bottom, left-to-right
    rectangles.sort_by(|a, b| {
        let ay = (a.bbox_min.y + a.bbox_max.y) / 2;
        let by = (b.bbox_min.y + b.bbox_max.y) / 2;
        let ax = (a.bbox_min.x + a.bbox_max.x) / 2;
        let bx = (b.bbox_min.x + b.bbox_max.x) / 2;
        ay.cmp(&by).then(ax.cmp(&bx))
    });

    // Step 4: Build a set of rectangle stroke indices for exclusion
    let rect_stroke_indices: HashSet<usize> = rectangles.iter().map(|r| r.stroke_index).collect();

    // Step 5: Assign strokes to rectangles or main
    let stroke_assignments: Vec<Option<usize>> = strokes
        .iter()
        .enumerate()
        .map(|(i, stroke)| {
            if rect_stroke_indices.contains(&i) {
                // Rectangle stroke itself: not assigned to main or any rect
                // We'll represent this as None, and the caller filters it out
                None
            } else {
                // Compute centroid from actual points (not bounding box from header)
                if stroke.points.is_empty() {
                    return None;
                }
                let min_x = stroke.points.iter().map(|p| p.x).min().unwrap();
                let max_x = stroke.points.iter().map(|p| p.x).max().unwrap();
                let min_y = stroke.points.iter().map(|p| p.y).min().unwrap();
                let max_y = stroke.points.iter().map(|p| p.y).max().unwrap();
                let cx = (min_x + max_x) / 2;
                let cy = (min_y + max_y) / 2;
                find_containing_rectangle(&rectangles, cx, cy)
            }
        })
        .collect();

    // Step 6: Assign textboxes to rectangles or main
    let textbox_assignments: Vec<Option<usize>> = textboxes
        .iter()
        .map(|tb| {
            // Convert pixel rect centroid to physical coordinates
            let pixel_cx = tb.rect.0 + tb.rect.2 / 2;
            let pixel_cy = tb.rect.1 + tb.rect.3 / 2;
            // Physical coords: x is mirrored, y is direct scaling
            let phys_cx = phys_w - (pixel_cx as i64 * phys_w / pixel_w as i64);
            let phys_cy = pixel_cy as i64 * phys_h / pixel_w as i64; // Assuming square pixels
            find_containing_rectangle(&rectangles, phys_cx as i32, phys_cy as i32)
        })
        .collect();

    SplitResult {
        rectangles,
        stroke_assignments,
        textbox_assignments,
    }
}

/// Detect rectangles from the stroke list.
fn detect_rectangles(strokes: &[Stroke], config: &SplitConfig) -> Vec<DetectedRectangle> {
    let mut rectangles = Vec::new();

    for (i, stroke) in strokes.iter().enumerate() {
        if let Some(rect) = try_detect_rectangle(stroke, config) {
            rectangles.push(DetectedRectangle {
                corners: rect.corners,
                bbox_min: rect.bbox_min,
                bbox_max: rect.bbox_max,
                stroke_index: i,
            });
        }
    }

    rectangles
}

struct DetectedRect {
    corners: [ScreenCoord; 4],
    bbox_min: ScreenCoord,
    bbox_max: ScreenCoord,
}

/// Try to detect if a single stroke is a rectangle.
///
/// Uses a "bbox-hugging" approach: a rectangle is a closed stroke where nearly
/// all points lie close to the bounding box edges. This is robust against the
/// common hand-drawing pattern of overlapping start/end lines.
fn try_detect_rectangle(stroke: &Stroke, config: &SplitConfig) -> Option<DetectedRect> {
    // Pre-filter: minimum points
    if stroke.points.len() < 20 {
        return None;
    }

    // Pre-filter: not an eraser
    if stroke.color == COLOR_ERASER {
        return None;
    }

    // Compute bounding box from points
    let min_x = stroke.points.iter().map(|p| p.x).min()?;
    let max_x = stroke.points.iter().map(|p| p.x).max()?;
    let min_y = stroke.points.iter().map(|p| p.y).min()?;
    let max_y = stroke.points.iter().map(|p| p.y).max()?;

    let width = max_x - min_x;
    let height = max_y - min_y;

    // Pre-filter: minimum height and reasonable aspect ratio
    if height < config.min_height {
        return None;
    }
    if width < config.min_height / 2 {
        return None; // too narrow to be a useful rectangle
    }

    // Closure check: both endpoints must be near the same bounding box edge.
    // This handles three common hand-drawing patterns:
    // 1. Endpoints close together (cleanly closed)
    // 2. Overlapping: the end runs past the start along the same edge
    // 3. Gap: the pen was lifted a bit before reaching the start point
    // In all cases, both endpoints are on (or near) the same bbox edge.
    let n = stroke.points.len();
    let first = &stroke.points[0];
    let last = &stroke.points[n - 1];
    let diagonal = ((width.pow(2) + height.pow(2)) as f64).sqrt();
    let edge_closure_tol = (diagonal * 0.12) as i32; // 12% of diagonal

    let same_edge = |p1: &ScreenCoord, p2: &ScreenCoord| -> bool {
        // Both near top edge
        ((p1.y - min_y).abs() <= edge_closure_tol && (p2.y - min_y).abs() <= edge_closure_tol)
        // Both near bottom edge
        || ((p1.y - max_y).abs() <= edge_closure_tol && (p2.y - max_y).abs() <= edge_closure_tol)
        // Both near left edge
        || ((p1.x - min_x).abs() <= edge_closure_tol && (p2.x - min_x).abs() <= edge_closure_tol)
        // Both near right edge
        || ((p1.x - max_x).abs() <= edge_closure_tol && (p2.x - max_x).abs() <= edge_closure_tol)
    };

    if !same_edge(first, last) {
        return None;
    }

    // Core test: do the stroke's points hug the bounding box edges?
    // For each point, compute the minimum distance to any of the four
    // bbox edges.  If most points (≥85%) are within a tolerance, the
    // stroke traces a rectangle.
    let edge_tol = (diagonal * 0.10) as i32; // 10% of diagonal

    let mut near_edge_count = 0usize;
    for pt in &stroke.points {
        let d_top = (pt.y - min_y).abs();
        let d_bot = (pt.y - max_y).abs();
        let d_left = (pt.x - min_x).abs();
        let d_right = (pt.x - max_x).abs();
        let min_d = d_top.min(d_bot).min(d_left).min(d_right);
        if min_d <= edge_tol {
            near_edge_count += 1;
        }
    }

    let hugging_ratio = near_edge_count as f64 / n as f64;
    if hugging_ratio < 0.85 {
        return None;
    }

    // All four edges must be visited — this distinguishes a rectangle
    // from a long single line near one edge.  We check that at least
    // some points are close to each of the four edges.
    let min_visits = n / 10; // at least 10% of points per edge
    let near_top = stroke
        .points
        .iter()
        .filter(|p| (p.y - min_y).abs() <= edge_tol)
        .count();
    let near_bot = stroke
        .points
        .iter()
        .filter(|p| (p.y - max_y).abs() <= edge_tol)
        .count();
    let near_left = stroke
        .points
        .iter()
        .filter(|p| (p.x - min_x).abs() <= edge_tol)
        .count();
    let near_right = stroke
        .points
        .iter()
        .filter(|p| (p.x - max_x).abs() <= edge_tol)
        .count();

    if near_top < min_visits
        || near_bot < min_visits
        || near_left < min_visits
        || near_right < min_visits
    {
        return None;
    }

    // Use bbox corners as the rectangle corners.
    let corners = [
        ScreenCoord { y: min_y, x: min_x },
        ScreenCoord { y: min_y, x: max_x },
        ScreenCoord { y: max_y, x: max_x },
        ScreenCoord { y: max_y, x: min_x },
    ];

    Some(DetectedRect {
        corners,
        bbox_min: ScreenCoord { y: min_y, x: min_x },
        bbox_max: ScreenCoord { y: max_y, x: max_x },
    })
}

/// Resolve nesting: if rectangle B's centroid is inside A, remove B from the list.
fn resolve_nesting(rectangles: &mut Vec<DetectedRectangle>) {
    let mut to_remove = HashSet::new();

    for (i, rect_i) in rectangles.iter().enumerate() {
        let cx = (rect_i.bbox_min.x + rect_i.bbox_max.x) / 2;
        let cy = (rect_i.bbox_min.y + rect_i.bbox_max.y) / 2;

        for (j, rect_j) in rectangles.iter().enumerate() {
            if i == j {
                continue;
            }
            if point_in_bbox(cx, cy, &rect_j.bbox_min, &rect_j.bbox_max) {
                // Rectangle i is inside j, mark i for removal
                to_remove.insert(i);
                break;
            }
        }
    }

    // Remove nested rectangles in reverse order to preserve indices
    let mut indices: Vec<_> = to_remove.into_iter().collect();
    indices.sort_unstable();
    for &idx in indices.iter().rev() {
        rectangles.remove(idx);
    }
}

/// Check if a point is inside a bounding box.
fn point_in_bbox(x: i32, y: i32, bbox_min: &ScreenCoord, bbox_max: &ScreenCoord) -> bool {
    x >= bbox_min.x && x <= bbox_max.x && y >= bbox_min.y && y <= bbox_max.y
}

/// Find which rectangle (if any) contains the given point.
/// Returns the index of the first matching rectangle.
fn find_containing_rectangle(rectangles: &[DetectedRectangle], x: i32, y: i32) -> Option<usize> {
    for (i, rect) in rectangles.iter().enumerate() {
        if point_in_bbox(x, y, &rect.bbox_min, &rect.bbox_max) {
            return Some(i);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_point_in_bbox() {
        let min = ScreenCoord { y: 0, x: 0 };
        let max = ScreenCoord { y: 100, x: 100 };
        assert!(point_in_bbox(50, 50, &min, &max));
        assert!(point_in_bbox(0, 0, &min, &max));
        assert!(point_in_bbox(100, 100, &min, &max));
        assert!(!point_in_bbox(-1, 50, &min, &max));
        assert!(!point_in_bbox(50, 101, &min, &max));
    }
}
