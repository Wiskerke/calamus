use calamus::bitmap::render_bitmap;
use calamus::format;
use wasm_bindgen::prelude::*;

/// Parse a .note file and return notebook metadata as a JSON string.
///
/// Returns `{ page_count, page_width, page_height, file_type, signature, device }`.
#[wasm_bindgen]
pub fn parse(data: &[u8]) -> Result<String, JsError> {
    let nb = format::parse(data).map_err(|e| JsError::new(&e.to_string()))?;

    let device = nb
        .header
        .get("APPLY_EQUIPMENT")
        .and_then(|v| v.first())
        .map(|s| s.as_str())
        .unwrap_or("");

    // Returning this as a json string, to save a little bit on not having to include serde or
    // js_sys to create a json object.
    Ok(format!(
        r#"{{"page_count":{},"page_width":{},"page_height":{},"file_type":"{}","signature":"{}","device":"{}"}}"#,
        nb.page_count(),
        nb.page_width,
        nb.page_height,
        nb.file_type,
        nb.signature,
        device,
    ))
}

/// Render a page from a .note file as raw RGBA pixels.
///
/// Returns a `Vec<u8>` of length `width * height * 4` in RGBA order,
/// suitable for drawing onto an HTML canvas via `ImageData`.
#[wasm_bindgen]
pub fn render_page(data: &[u8], page: usize) -> Result<Vec<u8>, JsError> {
    let nb = format::parse(data).map_err(|e| JsError::new(&e.to_string()))?;
    let (_, _, rgba) = render_bitmap(data, &nb, page).map_err(|e| JsError::new(&e.to_string()))?;
    Ok(rgba)
}
