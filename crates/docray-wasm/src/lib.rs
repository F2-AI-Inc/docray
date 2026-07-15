use docray_core::Extractor;
use docray_model::Granularity;
use docray_pdf::PdfExtractor;
use wasm_bindgen::prelude::*;

/// Extracts a PDF entirely in WASM and returns docray JSON.
///
/// `granularity` accepts "element", "word", "char", or the empty string for
/// the frozen schema 1.1 response.
#[wasm_bindgen]
pub fn extract(bytes: &[u8], granularity: &str) -> Result<String, JsValue> {
    let extraction = PdfExtractor
        .extract(bytes, None)
        .map_err(|error| JsValue::from_str(&error.to_string()))?;

    if granularity.is_empty() {
        serde_json::to_string(&extraction).map_err(js_error)
    } else {
        let granularity = granularity.parse::<Granularity>().map_err(js_error)?;
        serde_json::to_string(&extraction.with_granularity(granularity)).map_err(js_error)
    }
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}
