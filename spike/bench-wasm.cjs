#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");
const { performance } = require("node:perf_hooks");

async function initialized() {
  const root = path.resolve(__dirname, "..");
  const bytes = fs.readFileSync(path.join(root, "testdata/form.pdf"));
  const pdfiumDir = path.join(root, ".superpowers/sdd/scratch/pdfium-wasm/release/node");
  const t0 = performance.now();
  const PDFiumModule = require(path.join(pdfiumDir, "pdfium.js"));
  const t1 = performance.now();
  const pdfium = await PDFiumModule({
    locateFile: (file) => file.endsWith(".wasm") ? path.join(pdfiumDir, "pdfium.wasm") : file,
  });
  const t2 = performance.now();
  const docray = require(path.join(__dirname, "pkg/docray_wasm.js"));
  const t3 = performance.now();
  if (!docray.initialize_pdfium_render(pdfium, docray.__wasm ?? docray, false)) {
    throw new Error("initialize_pdfium_render returned false");
  }
  const t4 = performance.now();
  return {
    bytes,
    docray,
    phases: {
      pdfium_js_ms: t1 - t0,
      pdfium_wasm_ms: t2 - t1,
      docray_wasm_ms: t3 - t2,
      bind_ms: t4 - t3,
      total_init_ms: t4 - t0,
    },
  };
}

async function main() {
  const { bytes, docray, phases } = await initialized();
  const firstStart = performance.now();
  docray.extract(bytes, "");
  const first_extract_ms = performance.now() - firstStart;

  if (process.argv[2] === "cold") {
    console.log(JSON.stringify({ ...phases, first_extract_ms }));
    return;
  }

  const samples = [];
  for (let index = 0; index < 100; index += 1) {
    const start = performance.now();
    docray.extract(bytes, "");
    samples.push(performance.now() - start);
  }
  console.log(JSON.stringify({ phases, first_extract_ms, samples }));
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
