use docray_core::ExtractError;
use pdfium_render::prelude::*;

// pdfium-render keeps its bindings in a process-global that is set once and
// never torn down; every later `bind_*` call fails with
// `PdfiumLibraryBindingsAlreadyInitialized` and the existing bindings must be
// reused instead.

/// Exclusive use of PDFium for one extraction. PDFium is not thread-safe, so a
/// process holds at most one session at a time and [`pdfium`] blocks until the
/// current one is dropped. Documents loaded through a session must be dropped
/// before it; pdfium-render's handles are `!Send`, so they cannot escape to
/// another thread.
#[cfg(not(target_arch = "wasm32"))]
pub struct PdfiumSession {
    pdfium: Pdfium,
    _exclusive: std::sync::MutexGuard<'static, ()>,
}

#[cfg(not(target_arch = "wasm32"))]
impl std::ops::Deref for PdfiumSession {
    type Target = Pdfium;

    fn deref(&self) -> &Pdfium {
        &self.pdfium
    }
}

/// Opens a [`PdfiumSession`], binding to the pdfium dynamic library on first
/// use. Search order:
/// 1. `$DOCRAY_PDFIUM_DIR` (explicit override always wins)
/// 2. `<exe_dir>/../lib` and `<exe_dir>` — release archives and Homebrew kegs
///    install the library beside (or in `lib/` next to) the binaries, and this
///    makes them work from any working directory
/// 3. `./.pdfium/lib` — the source-checkout layout created by
///    `scripts/fetch-pdfium.sh`
/// 4. the system library path
#[cfg(not(target_arch = "wasm32"))]
pub fn pdfium() -> Result<PdfiumSession, ExtractError> {
    use std::sync::{Mutex, PoisonError};

    static EXCLUSIVE: Mutex<()> = Mutex::new(());
    let exclusive = EXCLUSIVE.lock().unwrap_or_else(PoisonError::into_inner);
    Ok(PdfiumSession {
        pdfium: bind_library()?,
        _exclusive: exclusive,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn bind_library() -> Result<Pdfium, ExtractError> {
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
        match Pdfium::bind_to_library(path) {
            Ok(bindings) => return Ok(Pdfium::new(bindings)),
            // `Pdfium::default()` reuses the process-global bindings without
            // touching the filesystem once they exist.
            Err(PdfiumError::PdfiumLibraryBindingsAlreadyInitialized) => {
                return Ok(Pdfium::default())
            }
            Err(_) => {}
        }
    }
    Pdfium::bind_to_system_library()
        .map(Pdfium::new)
        .map_err(|e| ExtractError::Io(format!("cannot bind pdfium: {e}")))
}

/// On wasm32, `pdfium-render` binds to a pre-initialized Emscripten module.
/// Javascript must call its exported `initialize_pdfium_render()` before
/// extraction; there is no dynamic-library path to search. The target is
/// single-threaded, and `Pdfium::default()` cannot reuse bindings there, so the
/// first binding is kept for the life of the module.
#[cfg(target_arch = "wasm32")]
pub fn pdfium() -> Result<&'static Pdfium, ExtractError> {
    use std::cell::OnceCell;

    thread_local! {
        static PDFIUM: OnceCell<&'static Pdfium> = const { OnceCell::new() };
    }
    PDFIUM.with(|slot| {
        if let Some(pdfium) = slot.get() {
            return Ok(*pdfium);
        }
        let pdfium: &'static Pdfium = Box::leak(Box::new(
            Pdfium::bind_to_system_library()
                .map(Pdfium::new)
                .map_err(|e| ExtractError::Io(format!("cannot bind pdfium WASM module: {e}")))?,
        ));
        Ok(*slot.get_or_init(|| pdfium))
    })
}
