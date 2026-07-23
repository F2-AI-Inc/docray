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

use docray_core::{check_granularity, sniff_format, ExtractError, Extractor, Format};
use docray_model::{Extraction, GranularExtraction, Granularity};
use docray_ooxml::{sniff_opc, OpcKind};
use docray_pdf::PdfExtractor;
use docray_pptx::PptxExtractor;
use wasm_bindgen::prelude::*;

const CFB_MAGIC: &[u8; 8] = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1";

/// Extracts a PDF or PPTX entirely in WASM and returns docray JSON.
///
/// `granularity` accepts "element", "word", "char", or the empty string for
/// the frozen schema 1.1 response. `max_input_bytes` is a caller-selected byte
/// cap; zero disables the cap.
///
/// Failures throw a JSON string with docray's stable error envelope. The caller
/// must parse the thrown string before inspecting `error.code`.
#[wasm_bindgen]
pub fn extract(
    bytes: &[u8],
    granularity: &str,
    max_input_bytes: usize,
    max_output_bytes: usize,
) -> Result<String, JsValue> {
    install_panic_hook();
    extract_inner(bytes, granularity, max_input_bytes, max_output_bytes).map_err(WasmError::into_js)
}

/// Extracts a PDF or PPTX and returns the token-lean line format (see the docs'
/// Output formats page). `granularity` accepts "element", "word", or the
/// empty string (implies element, matching the CLI/HTTP surfaces). "char" is
/// rejected with the stable `bad_format` error code, like everywhere else.
#[wasm_bindgen]
pub fn extract_lean(
    bytes: &[u8],
    granularity: &str,
    max_input_bytes: usize,
    max_output_bytes: usize,
) -> Result<String, JsValue> {
    install_panic_hook();
    extract_lean_inner(bytes, granularity, max_input_bytes, max_output_bytes)
        .map_err(WasmError::into_js)
}

fn extract_lean_inner(
    bytes: &[u8],
    granularity: &str,
    max_input_bytes: usize,
    max_output_bytes: usize,
) -> Result<String, WasmError> {
    let cap = output_cap(max_output_bytes);
    if max_input_bytes != 0 && bytes.len() > max_input_bytes {
        return Err(WasmError::new(
            "too_large",
            format!(
                "input is {} bytes, limit is {max_input_bytes} bytes",
                bytes.len()
            ),
        ));
    }
    let granularity = match granularity {
        "" | "element" => Granularity::Element,
        "word" => Granularity::Word,
        other => {
            return Err(WasmError::new(
                "bad_format",
                format!("lean format requires element or word granularity, got {other:?}"),
            ))
        }
    };
    let extraction = extract_document(bytes, Some(granularity))?;
    match extraction.with_granularity(granularity) {
        GranularExtraction::Compact(compact) => {
            let mut w = CappedString {
                buf: String::new(),
                remaining: cap,
            };
            compact.write_lean(&mut w).map_err(|_| {
                WasmError::new(OUTPUT_TOO_LARGE, format!("output exceeded {cap} bytes"))
            })?;
            Ok(w.buf)
        }
        GranularExtraction::Char(_) => unreachable!("char is rejected above"),
    }
}

fn extract_inner(
    bytes: &[u8],
    granularity: &str,
    max_input_bytes: usize,
    max_output_bytes: usize,
) -> Result<String, WasmError> {
    let cap = output_cap(max_output_bytes);
    if max_input_bytes != 0 && bytes.len() > max_input_bytes {
        return Err(WasmError::new(
            "too_large",
            format!(
                "input is {} bytes, limit is {max_input_bytes} bytes",
                bytes.len()
            ),
        ));
    }

    let requested = if granularity.is_empty() {
        None
    } else {
        Some(
            granularity
                .parse::<Granularity>()
                .map_err(WasmError::parse_failure)?,
        )
    };
    let extraction = extract_document(bytes, requested)?;

    if let Some(granularity) = requested {
        json_capped(&extraction.with_granularity(granularity), cap)
    } else {
        json_capped(&extraction, cap)
    }
}

/// Mirrors the CLI's format dispatch and, critically, performs the capability
/// check before extraction. The PPTX arm never touches Pdfium.
fn extract_document(bytes: &[u8], requested: Option<Granularity>) -> Result<Extraction, WasmError> {
    let result = match sniff_format(bytes) {
        Some(Format::Pdf) => {
            let extractor = PdfExtractor;
            check_granularity(&extractor.capabilities(), requested)
                .and_then(|()| extractor.extract(bytes, None))
        }
        Some(Format::Zip) => {
            let extractor = PptxExtractor;
            check_granularity(&extractor.capabilities(), requested).and_then(|()| match sniff_opc(
                bytes,
            )? {
                OpcKind::Pptx => extractor.extract(bytes, None),
                OpcKind::Docx | OpcKind::OtherZip => Err(ExtractError::UnsupportedFormatMessage(
                    "zip archive is not a PowerPoint file".into(),
                )),
            })
        }
        None if bytes.starts_with(CFB_MAGIC) => PptxExtractor.extract(bytes, None),
        None => Err(ExtractError::UnsupportedFormat),
    };
    result.map_err(WasmError::from_extract)
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

/// Byte budget exceeded during output generation.
const OUTPUT_TOO_LARGE: &str = "output_too_large";

/// `fmt::Write` that errors once a byte budget is exhausted, so lean
/// rendering aborts instead of materializing an unbounded string.
struct CappedString {
    buf: String,
    remaining: usize,
}

impl std::fmt::Write for CappedString {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        if s.len() > self.remaining {
            return Err(std::fmt::Error);
        }
        self.remaining -= s.len();
        self.buf.push_str(s);
        Ok(())
    }
}

/// `io::Write` twin for serde_json, bounding JSON serialization the same way.
struct CappedVec {
    buf: Vec<u8>,
    remaining: usize,
}

impl std::io::Write for CappedVec {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if data.len() > self.remaining {
            return Err(std::io::Error::other("output budget exceeded"));
        }
        self.remaining -= data.len();
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn output_cap(max_output_bytes: usize) -> usize {
    if max_output_bytes == 0 {
        usize::MAX
    } else {
        max_output_bytes
    }
}

fn json_capped<T: serde::Serialize>(value: &T, cap: usize) -> Result<String, WasmError> {
    let mut w = CappedVec {
        buf: Vec::new(),
        remaining: cap,
    };
    match serde_json::to_writer(&mut w, value) {
        Ok(()) => {
            String::from_utf8(w.buf).map_err(|e| WasmError::new("parse_failure", e.to_string()))
        }
        Err(e) if e.is_io() => Err(WasmError::new(
            OUTPUT_TOO_LARGE,
            format!("output exceeded {cap} bytes"),
        )),
        Err(e) => Err(WasmError::new("parse_failure", e.to_string())),
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
        let error = extract_inner(&[0; 3], "", 2, 0).unwrap_err();

        assert_eq!(error.code, "too_large");
        assert_eq!(error.message, "input is 3 bytes, limit is 2 bytes");
    }

    #[test]
    fn zero_input_cap_is_uncapped() {
        let error = extract_inner(b"not a document", "", 0, 0).unwrap_err();

        assert_ne!(error.code, "too_large");
    }

    #[test]
    fn pptx_element_json_extracts_without_pdfium() {
        let bytes = include_bytes!("../../../testdata/pptx/table.pptx");
        let json = extract_inner(bytes, "element", 0, 0).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(value["schema_version"], "1.6");
        assert_eq!(value["granularity"], "element");
        assert_eq!(value["source"]["format"], "pptx");
        assert_eq!(value["pages"][0]["elements"][0]["type"], "table");
    }

    #[test]
    fn pptx_rejects_implicit_and_word_granularity() {
        let bytes = include_bytes!("../../../testdata/pptx/table.pptx");
        for granularity in ["", "char", "word"] {
            let error = extract_inner(bytes, granularity, 0, 0).unwrap_err();
            assert_eq!(error.code, "granularity_unavailable");
        }
    }

    #[test]
    fn cfb_magic_uses_the_cli_legacy_office_error() {
        let error = extract_inner(CFB_MAGIC, "element", 0, 0).unwrap_err();

        assert_eq!(error.code, "unsupported_format");
        assert_eq!(
            error.message,
            "legacy or encrypted Office documents are not supported"
        );
    }

    use std::fmt::Write as _;
    use std::io::Write as _;

    #[test]
    fn capped_string_errors_at_budget() {
        let mut w = CappedString {
            buf: String::new(),
            remaining: 5,
        };
        assert!(w.write_str("abc").is_ok());
        assert!(w.write_str("de").is_ok());
        assert!(w.write_str("f").is_err(), "budget exhausted must error");
        assert_eq!(w.buf, "abcde");
    }

    #[test]
    fn capped_vec_errors_at_budget_and_json_maps_to_output_too_large() {
        let mut w = CappedVec {
            buf: Vec::new(),
            remaining: 4,
        };
        assert!(w.write(b"1234").is_ok());
        assert!(w.write(b"5").is_err());

        let big = vec!["x".repeat(64); 4];
        let err = json_capped(&big, 16).unwrap_err();
        assert!(err.json().contains("output_too_large"), "{}", err.json());
        // Uncapped succeeds.
        assert!(json_capped(&big, 1 << 20).is_ok());
    }
}
