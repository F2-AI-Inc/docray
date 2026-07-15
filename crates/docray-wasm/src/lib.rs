//! Browser-facing docray extraction.
//!
//! # Threading and failure contract
//!
//! A loaded module and its attached Pdfium Emscripten module have exactly one
//! owner: the JavaScript worker that initialized them. Calls must be serialized;
//! neither module may be shared across workers or used concurrently. An
//! Emscripten abort is fatal to that worker even if JavaScript catches the
//! exception. Discard both module instances and respawn the worker, mirroring
//! docray-server's native subprocess isolation.

use docray_core::{ExtractError, Extractor};
use docray_model::Granularity;
use docray_pdf::PdfExtractor;
use wasm_bindgen::prelude::*;

/// Extracts a PDF entirely in WASM and returns docray JSON.
///
/// `granularity` accepts "element", "word", "char", or the empty string for
/// the frozen schema 1.1 response. `max_input_bytes` is a caller-selected byte
/// cap; zero disables the cap.
///
/// Failures throw a JSON string with docray's stable error envelope. The caller
/// must parse the thrown string before inspecting `error.code`.
#[wasm_bindgen]
pub fn extract(bytes: &[u8], granularity: &str, max_input_bytes: usize) -> Result<String, JsValue> {
    install_panic_hook();
    extract_inner(bytes, granularity, max_input_bytes).map_err(WasmError::into_js)
}

fn extract_inner(
    bytes: &[u8],
    granularity: &str,
    max_input_bytes: usize,
) -> Result<String, WasmError> {
    if max_input_bytes != 0 && bytes.len() > max_input_bytes {
        return Err(WasmError::new(
            "too_large",
            format!(
                "input is {} bytes, limit is {max_input_bytes} bytes",
                bytes.len()
            ),
        ));
    }

    let extraction = PdfExtractor
        .extract(bytes, None)
        .map_err(WasmError::from_extract)?;

    if granularity.is_empty() {
        serde_json::to_string(&extraction).map_err(WasmError::parse_failure)
    } else {
        let granularity = granularity
            .parse::<Granularity>()
            .map_err(WasmError::parse_failure)?;
        serde_json::to_string(&extraction.with_granularity(granularity))
            .map_err(WasmError::parse_failure)
    }
}

#[derive(Debug, PartialEq)]
struct WasmError {
    code: &'static str,
    message: String,
}

impl WasmError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn from_extract(error: ExtractError) -> Self {
        Self::new(error.code(), error.to_string())
    }

    fn parse_failure(error: impl std::fmt::Display) -> Self {
        Self::new("parse_failure", error.to_string())
    }

    fn json(&self) -> String {
        serde_json::json!({
            "error": {
                "code": self.code,
                "message": self.message,
            }
        })
        .to_string()
    }

    fn into_js(self) -> JsValue {
        JsValue::from_str(&self.json())
    }
}

#[cfg(target_arch = "wasm32")]
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|_| {
        wasm_bindgen::throw_val(JsValue::from_str(
            &WasmError::new("crash", "WASM extraction crashed").json(),
        ));
    }));
}

#[cfg(not(target_arch = "wasm32"))]
fn install_panic_hook() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_envelope_escapes_messages() {
        let error = WasmError::new("parse_failure", "bad \"value\"\nnext");
        let value: serde_json::Value = serde_json::from_str(&error.json()).unwrap();

        assert_eq!(value["error"]["code"], "parse_failure");
        assert_eq!(value["error"]["message"], "bad \"value\"\nnext");
    }

    #[test]
    fn input_cap_precedes_pdfium() {
        let error = extract_inner(&[0; 3], "", 2).unwrap_err();

        assert_eq!(error.code, "too_large");
        assert_eq!(error.message, "input is 3 bytes, limit is 2 bytes");
    }

    #[test]
    fn zero_input_cap_is_uncapped() {
        let error = extract_inner(b"not a PDF", "", 0).unwrap_err();

        assert_ne!(error.code, "too_large");
    }
}
