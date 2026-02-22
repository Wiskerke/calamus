//! Integration test: render SVG to PNG via resvg and compare against reference bitmaps.
//!
//! This test requires `resvg` in PATH (available via `nix shell nixpkgs#resvg`).
//! If resvg is not found, the test prints a message and passes (skips gracefully).
#![cfg(feature = "render")]

use std::path::PathBuf;
use std::process::Command;

fn test_file(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../testfiles")
        .join(name)
}

fn resvg_available() -> bool {
    Command::new("resvg")
        .arg("--help")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn svg_pixel_comparison() {
    if !resvg_available() {
        eprintln!("resvg not found in PATH — skipping pixel comparison test");
        eprintln!("Install with: nix shell nixpkgs#resvg");
        return;
    }

    let (data, nb) = calamus::format::load(&test_file("test_after_save.note")).unwrap();
    let tmp = tempfile::tempdir().unwrap();

    let references = ["test_after_save_00.png", "test_after_save_01.png"];

    for page in 0..nb.page_count() {
        let svg = calamus::render::to_svg(&data, &nb, page).unwrap();
        let svg_path = tmp.path().join(format!("page_{page:02}.svg"));
        let png_path = tmp.path().join(format!("page_{page:02}.png"));
        std::fs::write(&svg_path, &svg).unwrap();

        // Render SVG to PNG at the pixel dimensions (1404x1872)
        let output = Command::new("resvg")
            .args([
                svg_path.to_str().unwrap(),
                png_path.to_str().unwrap(),
                "-w",
                "1404",
                "-h",
                "1872",
            ])
            .output()
            .expect("failed to run resvg");

        assert!(
            output.status.success(),
            "resvg failed for page {page}: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        // Load rendered and reference images
        let rendered = image::open(&png_path)
            .unwrap_or_else(|e| panic!("failed to open rendered PNG for page {page}: {e}"))
            .to_rgba8();
        let reference = image::open(test_file(references[page]))
            .unwrap_or_else(|e| panic!("failed to open reference PNG for page {page}: {e}"))
            .to_rgba8();

        assert_eq!(
            rendered.dimensions(),
            reference.dimensions(),
            "page {page}: dimension mismatch"
        );

        // Compare pixels
        let total_pixels = rendered.width() as u64 * rendered.height() as u64;
        let mut differing_pixels: u64 = 0;
        let mut max_channel_diff: u8 = 0;
        let mut total_channel_diff: u64 = 0;

        for (r, e) in rendered.pixels().zip(reference.pixels()) {
            let channel_diff =
                r.0.iter()
                    .zip(e.0.iter())
                    .map(|(a, b)| a.abs_diff(*b))
                    .max()
                    .unwrap();
            if channel_diff > 0 {
                differing_pixels += 1;
                max_channel_diff = max_channel_diff.max(channel_diff);
                total_channel_diff += channel_diff as u64;
            }
        }

        let diff_pct = differing_pixels as f64 / total_pixels as f64 * 100.0;
        let avg_diff = if differing_pixels > 0 {
            total_channel_diff as f64 / differing_pixels as f64
        } else {
            0.0
        };

        eprintln!(
            "Page {page}: {differing_pixels}/{total_pixels} pixels differ ({diff_pct:.2}%), \
             max channel diff={max_channel_diff}, avg diff of differing={avg_diff:.1}"
        );
    }
}
