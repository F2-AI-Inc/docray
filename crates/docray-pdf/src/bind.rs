use docray_core::ExtractError;
use pdfium_render::prelude::*;

/// Binds to the pdfium dynamic library.
/// Order: $DOCRAY_PDFIUM_DIR -> ./.pdfium/lib -> system library.
pub fn pdfium() -> Result<Pdfium, ExtractError> {
    let candidates = [
        std::env::var("DOCRAY_PDFIUM_DIR").ok(),
        Some(".pdfium/lib".to_string()),
    ];
    for dir in candidates.into_iter().flatten() {
        let path = Pdfium::pdfium_platform_library_name_at_path(&dir);
        if let Ok(bindings) = Pdfium::bind_to_library(path) {
            return Ok(Pdfium::new(bindings));
        }
    }
    Pdfium::bind_to_system_library()
        .map(Pdfium::new)
        .map_err(|e| ExtractError::Io(format!("cannot bind pdfium: {e}")))
}
