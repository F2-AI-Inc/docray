use docray_core::ExtractError;
use pdfium_render::prelude::*;

/// Binds to the pdfium dynamic library. Search order:
/// 1. `$DOCRAY_PDFIUM_DIR` (explicit override always wins)
/// 2. `<exe_dir>/../lib` and `<exe_dir>` — release archives and Homebrew kegs
///    install the library beside (or in `lib/` next to) the binaries, and this
///    makes them work from any working directory
/// 3. `./.pdfium/lib` — the source-checkout layout created by
///    `scripts/fetch-pdfium.sh`
/// 4. the system library path
#[cfg(not(target_arch = "wasm32"))]
pub fn pdfium() -> Result<Pdfium, ExtractError> {
    let exe_relative = std::env::current_exe().ok().and_then(|exe| {
        let dir = exe.parent()?.to_path_buf();
        Some([dir.join("../lib"), dir])
    });

    let mut candidates: Vec<String> = Vec::new();
    if let Ok(dir) = std::env::var("DOCRAY_PDFIUM_DIR") {
        candidates.push(dir);
    }
    if let Some(dirs) = exe_relative {
        candidates.extend(dirs.iter().filter_map(|d| d.to_str().map(String::from)));
    }
    candidates.push(".pdfium/lib".to_string());

    for dir in candidates {
        let path = Pdfium::pdfium_platform_library_name_at_path(&dir);
        if let Ok(bindings) = Pdfium::bind_to_library(path) {
            return Ok(Pdfium::new(bindings));
        }
    }
    Pdfium::bind_to_system_library()
        .map(Pdfium::new)
        .map_err(|e| ExtractError::Io(format!("cannot bind pdfium: {e}")))
}

/// On wasm32, `pdfium-render` binds to a pre-initialized Emscripten module.
/// Javascript must call its exported `initialize_pdfium_render()` before
/// extraction; there is no dynamic-library path to search.
#[cfg(target_arch = "wasm32")]
pub fn pdfium() -> Result<Pdfium, ExtractError> {
    Pdfium::bind_to_system_library()
        .map(Pdfium::new)
        .map_err(|e| ExtractError::Io(format!("cannot bind pdfium WASM module: {e}")))
}
