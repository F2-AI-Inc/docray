#!/usr/bin/env node

// Node verification harness for the temporary docray-wasm crate. Build first:
//   WASM_PACK_CACHE=/tmp/wasm-pack-cache wasm-pack build \
//     crates/docray-wasm --target nodejs --release --out-dir ../../spike/pkg

const fs = require("node:fs");
const path = require("node:path");

async function main() {
  const input = process.argv[2];
  const granularity = process.argv[3] ?? "";
  if (!input) throw new Error("usage: node spike/run-wasm.cjs PDF [GRANULARITY]");

  const root = path.resolve(__dirname, "..");
  const pdfiumDir = path.join(
    root,
    ".superpowers/sdd/scratch/pdfium-wasm/release/node",
  );
  const PDFiumModule = require(path.join(pdfiumDir, "pdfium.js"));
  const pdfium = await PDFiumModule({
    locateFile: (file) =>
      file.endsWith(".wasm") ? path.join(pdfiumDir, "pdfium.wasm") : file,
  });

  // Older wasm-bindgen node output exposed the Rust instance as `__wasm`;
  // 0.2.126 keeps it private but exports the callback wrappers on the package.
  // Either object gives pdfium-render the read/write callbacks it installs
  // into Pdfium's Emscripten function table.
  const docray = require(path.join(__dirname, "pkg/docray_wasm.js"));
  const localWasmModule = docray.__wasm ?? docray;
  if (!docray.initialize_pdfium_render(pdfium, localWasmModule, false)) {
    throw new Error("pdfium-render rejected the Emscripten Pdfium module");
  }

  process.stdout.write(docray.extract(fs.readFileSync(input), granularity));
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
