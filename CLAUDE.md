# Calamus

A tool and library in Rust for parsing and rendering Supernote `.note` files.

## Goal

- Provide a CLI tool, with support to write to svg and png
- Provide a wasm output, to display note pages on a webpage
- Programmatic access to stroke data for shape detection and content extraction

## Architecture

Workspace crates live under `crates/`:

- crates/core: Core library for parsing and rendering
- crates/cli: CLI tool with `info`, `render`, and `svg` subcommands
- crates/wasm: Create a webassembly output

## Development

Requires Nix with flakes. Enter the dev shell with `direnv allow` or `nix develop`.

```
just build # release build
just check # clippy + fmt check
just fmt   # format code
just test  # run all tests
just run   # run the CLI
just testoutput  # Render all notes in testfiles to svg and png
just wasm  # build wasm target
```

Available tools for analysing data: uv for python scripts, resvg to transform svg to png.

## Supernote .note Format

See `docs/note-format.md` for the full format specification.
See `docs/linestudy.md` for the investigation of how to draw lines from the path data.

Key points:
- File header: `note` (4 bytes) + `SN_FILE_VER_20230015` (20 bytes)
- Footer address at last 4 bytes of file; footer is the table of contents
- Metadata blocks: length-prefixed UTF-8 with `<KEY:VALUE>` pairs
- up to 5 layers per page
- Bitmap data: RLE encoded (`RATTA_RLE` protocol) -- pre-rendered by device
- Stroke/path data: `TOTALPATH` contains per-stroke vectors with points, pressure, tilt
- Stroke header is 208 bytes on Nomad (N6), followed by sized arrays
- Coordinates in strokes are (y, x) order, in 10-micrometer physical units
- Device writes both bitmap AND stroke data; bitmap is useful for png, stroke data for svg
- Files during live editing are fragmented; proper save compacts them

## Test Files

- `testfiles`: folder contains multiple test notes.
- All data from Nomad (N6), signature `SN_FILE_VER_20230015`

## Validation

Convert the svg output to png with resvg and compare pixel-by-pixel with the png output of the tool.
After changes, run `just testoutput` to allow user to check the differences between svg and png.

## Reference Implementations

Download with `just inspirations`.

- `inspirations/supernote-tool/` -- Python, primary reference for metadata + RLE decoding
- `inspirations/pysn-digest/` -- Python, extended fork with more device support
- `inspirations/snlib/` -- Rust, exact stroke binary layout + ImHex pattern file
