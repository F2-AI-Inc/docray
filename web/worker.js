/* docray extraction worker.
 *
 * The browser twin of the native architecture: extraction runs off the UI
 * thread in this worker, and the page treats a dead worker exactly like the
 * server treats a dead CLI subprocess — report the failure, respawn, move on.
 * An emscripten abort/OOM here is fatal to the worker BY DESIGN; never try
 * to reuse the pdfium module after one.
 *
 * Protocol (page -> worker):  {id, cmd: "extract", bytes: ArrayBuffer,
 *                              granularity: ""|"element"|"word"|"char",
 *                              cap: number}
 * (worker -> page)  success:  {id, ok: true, json: string, ms: number}
 *                   failure:  {id, ok: false, error: {code, message}}
 * The worker also posts {cmd: "ready"} after successful initialization.
 *
 * Classic worker on purpose: pdfium.js is UMD (importScripts-compatible),
 * docray_wasm.js is an ES module (loaded via dynamic import, which classic
 * workers support). Pdfium is loaded lazily only after PDF magic is seen;
 * PPTX extraction is pure Rust and never touches the Emscripten module.
 */
"use strict";

let wasmInitPromise = null;
let pdfiumInitPromise = null;

function initWasm() {
  if (!wasmInitPromise) {
    wasmInitPromise = (async () => {
      const docray = await import("./wasm/docray_wasm.js");
      const wasm = await docray.default({ module_or_path: "wasm/docray_wasm_bg.wasm" });
      return { docray, wasm };
    })();
    wasmInitPromise.then(
      () => postMessage({ cmd: "ready" }),
      () => {}, // failures surface per-request below
    );
  }
  return wasmInitPromise;
}

function initPdfium(docray, wasm) {
  if (!pdfiumInitPromise) {
    pdfiumInitPromise = (async () => {
      importScripts("wasm/pdfium.js"); // exposes global PDFiumModule (UMD)
      const pdfium = await PDFiumModule({
        locateFile: f => (f.endsWith(".wasm") ? "wasm/pdfium.wasm" : f),
      });
      if (!docray.initialize_pdfium_render(pdfium, wasm, false)) {
        throw new Error("pdfium-render rejected the pdfium module");
      }
      return pdfium;
    })();
  }
  return pdfiumInitPromise;
}

function sniffFormat(bytes) {
  if (bytes.length >= 4 && bytes[0] === 0x50 && bytes[1] === 0x4b &&
      bytes[2] === 0x03 && bytes[3] === 0x04) return "pptx";
  const end = Math.min(bytes.length - 5, 1023);
  for (let i = 0; i <= end; i += 1) {
    if (bytes[i] === 0x25 && bytes[i + 1] === 0x50 && bytes[i + 2] === 0x44 &&
        bytes[i + 3] === 0x46 && bytes[i + 4] === 0x2d) return "pdf";
  }
  return "other";
}

function errorEnvelope(e) {
  // docray-wasm throws a JsValue whose string form is the stable error JSON;
  // anything else (loader failures, emscripten aborts) becomes code "crash".
  try {
    const parsed = JSON.parse(typeof e === "string" ? e : e.message || String(e));
    if (parsed && parsed.error && parsed.error.code) return parsed.error;
  } catch (_) { /* not an envelope */ }
  return { code: "crash", message: String(e && e.message || e) };
}

// A pathological PDF can emit JSON orders of magnitude larger than itself;
// refuse to clone anything huge into the page (mirrors the server's
// streaming output cap). Measured in UTF-16 code units (string .length):
// 128M units bounds the in-memory string at ~256MB regardless of content.
const OUTPUT_CAP_UNITS = 128 * 1024 * 1024;

self.onmessage = async ev => {
  const { id, cmd, bytes, granularity, cap } = ev.data;
  if (cmd !== "extract" && cmd !== "extract_lean") return;
  try {
    const input = new Uint8Array(bytes);
    const { docray, wasm } = await initWasm();
    if (sniffFormat(input) === "pdf") await initPdfium(docray, wasm);
    const t0 = performance.now();
    // Output budget enforced INSIDE the wasm during serialization (a
    // pathological PDF can't OOM the worker by materializing a huge string);
    // the post-hoc units check below stays as a second belt.
    const json = cmd === "extract_lean"
      ? docray.extract_lean(input, granularity || "", cap || 0, OUTPUT_CAP_UNITS * 2)
      : docray.extract(input, granularity || "", cap || 0, OUTPUT_CAP_UNITS * 2);
    if (json.length > OUTPUT_CAP_UNITS) {
      postMessage({ id, ok: false, error: { code: "output_too_large",
        message: "extraction produced " + json.length + " code units (cap " + OUTPUT_CAP_UNITS + ")" } });
      return;
    }
    postMessage({ id, ok: true, json, ms: Math.round(performance.now() - t0) });
  } catch (e) {
    postMessage({ id, ok: false, error: errorEnvelope(e) });
  }
};

initWasm(); // warm Rust eagerly; Pdfium waits until a PDF request actually needs it
