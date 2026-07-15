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
 * workers support).
 */
"use strict";

importScripts("wasm/pdfium.js"); // exposes global PDFiumModule (UMD)

let initPromise = null;

function initOnce() {
  if (!initPromise) {
    initPromise = (async () => {
      const pdfium = await PDFiumModule({
        locateFile: f => (f.endsWith(".wasm") ? "wasm/pdfium.wasm" : f),
      });
      const docray = await import("./wasm/docray_wasm.js");
      const wasm = await docray.default({ module_or_path: "wasm/docray_wasm_bg.wasm" });
      if (!docray.initialize_pdfium_render(pdfium, wasm, false)) {
        throw new Error("pdfium-render rejected the pdfium module");
      }
      return docray;
    })();
    initPromise.then(
      () => postMessage({ cmd: "ready" }),
      () => {}, // failures surface per-request below
    );
  }
  return initPromise;
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

self.onmessage = async ev => {
  const { id, cmd, bytes, granularity, cap } = ev.data;
  if (cmd !== "extract") return;
  try {
    const docray = await initOnce();
    const t0 = performance.now();
    const json = docray.extract(new Uint8Array(bytes), granularity || "", cap || 0);
    postMessage({ id, ok: true, json, ms: Math.round(performance.now() - t0) });
  } catch (e) {
    postMessage({ id, ok: false, error: errorEnvelope(e) });
  }
};

initOnce(); // warm eagerly: module fetch+compile overlaps with the user picking a file
