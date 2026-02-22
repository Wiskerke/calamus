use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "calamus", about = "Supernote .note file parser and renderer")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show metadata about a .note file
    Info {
        /// Path to the .note file
        file: PathBuf,
    },
    /// Render pages as PNG images
    Render {
        /// Path to the .note file
        file: PathBuf,
        /// Output path (page number will be appended)
        output: PathBuf,
        /// Render a specific page (0-indexed), or all pages if omitted
        #[arg(short, long)]
        page: Option<usize>,
    },
    /// Export pages as SVG with individual strokes
    Svg {
        /// Path to the .note file
        file: PathBuf,
        /// Output path
        output: PathBuf,
        /// Export a specific page (0-indexed), or all pages if omitted
        #[arg(short, long)]
        page: Option<usize>,
    },
    /// Split pages by detected rectangles into separate SVGs
    Split {
        /// Path to the .note file
        file: PathBuf,
        /// Output directory for split SVGs
        output_dir: PathBuf,
        /// Split a specific page (0-indexed), or all pages if omitted
        #[arg(short, long)]
        page: Option<usize>,
        /// Minimum rectangle height in mm (default: 15)
        #[arg(long, default_value = "15.0")]
        min_height: f32,
    },
}

/// Resolve the output path for a given page.
///
/// If `output` is a directory, the filename is derived from the input note file's stem
/// with the given extension (e.g. `myfile.png`). For multi-page output, page numbers
/// are appended (e.g. `myfile_00.png`).
fn output_path(output: &Path, input: &Path, ext: &str, page: usize, multi_page: bool) -> PathBuf {
    if output.is_dir() {
        // Directory output: always include page number
        let stem = input.file_stem().unwrap_or_default().to_string_lossy();
        output.join(format!("{stem}_{page:02}.{ext}"))
    } else if multi_page {
        let stem = output.file_stem().unwrap_or_default().to_string_lossy();
        let ext = output.extension().unwrap_or_default().to_string_lossy();
        output.with_file_name(format!("{stem}_{page:02}.{ext}"))
    } else {
        output.to_path_buf()
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Info { file } => {
            let (_data, notebook) = calamus::format::load(&file)?;
            println!("{notebook}");
        }
        Command::Render { file, output, page } => {
            let (data, notebook) = calamus::format::load(&file)?;
            let pages = match page {
                Some(p) => vec![p],
                None => (0..notebook.page_count()).collect(),
            };
            let multi_page = page.is_none() && notebook.page_count() > 1;
            for p in pages {
                let img = calamus::render::to_image(&data, &notebook, p)?;
                let path = output_path(&output, &file, "png", p, multi_page);
                img.save(&path)?;
                println!("Saved {}", path.display());
            }
        }
        Command::Svg { file, output, page } => {
            let (data, notebook) = calamus::format::load(&file)?;
            let pages = match page {
                Some(p) => vec![p],
                None => (0..notebook.page_count()).collect(),
            };
            let multi_page = page.is_none() && notebook.page_count() > 1;
            for p in pages {
                let svg = calamus::render::to_svg(&data, &notebook, p)?;
                let path = output_path(&output, &file, "svg", p, multi_page);
                std::fs::write(&path, svg)?;
                println!("Saved {}", path.display());
            }
        }
        Command::Split {
            file,
            output_dir,
            page,
            min_height,
        } => {
            use std::collections::HashSet;

            let (data, notebook) = calamus::format::load(&file)?;
            let pages = match page {
                Some(p) => vec![p],
                None => (0..notebook.page_count()).collect(),
            };

            // Create output directory if it doesn't exist
            std::fs::create_dir_all(&output_dir)?;

            let config = calamus::split::SplitConfig {
                min_height: (min_height * 100.0) as i32, // mm to physical units
            };

            for p in pages {
                let pg = notebook.page(&data, p)?;
                let (strokes, text_boxes) = pg.decode_strokes(&data)?;

                // Get physical dimensions
                let (phys_w, phys_h) = strokes
                    .first()
                    .map(|s| (s.screen_width as i64, s.screen_height as i64))
                    .unwrap_or((11864, 15819));

                // Detect rectangles and classify content
                let split_result = calamus::split::split_page(
                    &strokes,
                    &text_boxes,
                    phys_w,
                    phys_h,
                    notebook.page_width,
                    &config,
                );

                // Build main SVG (strokes not in any rectangle, excluding rectangle strokes)
                let main_stroke_indices: HashSet<usize> = split_result
                    .stroke_assignments
                    .iter()
                    .enumerate()
                    .filter_map(|(i, assignment)| {
                        if assignment.is_none() {
                            // Check if this is a rectangle stroke
                            let is_rect =
                                split_result.rectangles.iter().any(|r| r.stroke_index == i);
                            if !is_rect { Some(i) } else { None }
                        } else {
                            None
                        }
                    })
                    .collect();

                let main_textbox_indices: HashSet<usize> = split_result
                    .textbox_assignments
                    .iter()
                    .enumerate()
                    .filter_map(
                        |(i, assignment)| {
                            if assignment.is_none() { Some(i) } else { None }
                        },
                    )
                    .collect();

                let main_svg = calamus::render::to_svg_subset(
                    &data,
                    &notebook,
                    p,
                    Some(&main_stroke_indices),
                    Some(&main_textbox_indices),
                    None,
                )?;

                let stem = file.file_stem().unwrap_or_default().to_string_lossy();
                let main_path = output_dir.join(format!("{}_p{:02}_main.svg", stem, p));
                std::fs::write(&main_path, main_svg)?;
                println!("Saved {}", main_path.display());

                // Generate SVG for each rectangle
                for (rect_idx, rect) in split_result.rectangles.iter().enumerate() {
                    let rect_stroke_indices: HashSet<usize> = split_result
                        .stroke_assignments
                        .iter()
                        .enumerate()
                        .filter_map(|(i, assignment)| {
                            if *assignment == Some(rect_idx) {
                                Some(i)
                            } else {
                                None
                            }
                        })
                        .collect();

                    let rect_textbox_indices: HashSet<usize> = split_result
                        .textbox_assignments
                        .iter()
                        .enumerate()
                        .filter_map(|(i, assignment)| {
                            if *assignment == Some(rect_idx) {
                                Some(i)
                            } else {
                                None
                            }
                        })
                        .collect();

                    // Define viewport with 2mm margin
                    let viewport = (
                        rect.bbox_min.y,
                        rect.bbox_min.x,
                        rect.bbox_max.y,
                        rect.bbox_max.x,
                        2.0, // 2mm margin
                    );

                    let rect_svg = calamus::render::to_svg_subset(
                        &data,
                        &notebook,
                        p,
                        Some(&rect_stroke_indices),
                        Some(&rect_textbox_indices),
                        Some(viewport),
                    )?;

                    let rect_path =
                        output_dir.join(format!("{}_p{:02}_rect{:02}.svg", stem, p, rect_idx));
                    std::fs::write(&rect_path, rect_svg)?;
                    println!("Saved {}", rect_path.display());
                }
            }
        }
    }

    Ok(())
}
