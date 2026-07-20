#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const [pkgArgument, nativeArgument] = process.argv.slice(2);
if (!pkgArgument || !nativeArgument) {
  console.error("usage: node scripts/wasm-parity.cjs NODEJS_PKG_DIR NATIVE_CLI");
  process.exit(2);
}

const root = path.resolve(__dirname, "..");
const pkgDir = path.resolve(pkgArgument);
const nativeCli = path.resolve(nativeArgument);
const fixtures = ["simple.pdf", "form.pdf", "rotated.pdf", "link.pdf"];

function exact(failures, label, a, b) {
  if (JSON.stringify(a) !== JSON.stringify(b)) {
    failures.push(`${label}: ${JSON.stringify(a)} != ${JSON.stringify(b)}`);
  }
}


/* Numeric-aware lean line comparison: numbers must agree within the same 2pt
   tolerance the JSON comparator uses (native and wasm pdfium builds have
   sub-point glyph-metric variance); everything non-numeric must match
   exactly, so any real format/content divergence still fails. */
function leanLineClose(a, b) {
  if (a === b) return true;
  if (a === undefined || b === undefined) return false;
  const num = /-?\d+(?:\.\d+)?/g;
  if (a.replace(num, "#") !== b.replace(num, "#")) return false;
  const an = a.match(num) || [];
  const bn = b.match(num) || [];
  if (an.length !== bn.length) return false;
  return an.every((x, i) => Math.abs(Number(x) - Number(bn[i])) <= 2);
}

function compare(native, wasm) {
  const failures = [];
  let geometryComparisons = 0;
  let maxGeometryDelta = 0;
  const seen = { text: 0, image: 0, path: 0, annotation: 0, chars: 0 };

  function near(label, a, b) {
    geometryComparisons += 1;
    const delta = Math.abs(a - b);
    maxGeometryDelta = Math.max(maxGeometryDelta, delta);
    if (!Number.isFinite(delta) || delta > 2) failures.push(`${label}: delta ${delta}pt`);
  }

  function bbox(label, a, b) {
    for (const key of ["x0", "y0", "x1", "y1"]) near(`${label}.${key}`, a[key], b[key]);
  }

  function compareText(label, a, b) {
    seen.text += 1;
    exact(failures, `${label}.content`, a.content, b.content);
    exact(failures, `${label}.font.name`, a.font.name, b.font.name);
    exact(failures, `${label}.font.bold`, a.font.bold, b.font.bold);
    exact(failures, `${label}.font.italic`, a.font.italic, b.font.italic);
    near(`${label}.font.size`, a.font.size, b.font.size);
    exact(failures, `${label}.color`, a.color, b.color);
    exact(failures, `${label}.line_count`, a.lines.length, b.lines.length);
    for (let li = 0; li < Math.min(a.lines.length, b.lines.length); li += 1) {
      const al = a.lines[li];
      const bl = b.lines[li];
      bbox(`${label}.lines[${li}].bbox`, al.bbox, bl.bbox);
      near(`${label}.lines[${li}].baseline_y`, al.baseline_y, bl.baseline_y);
      exact(failures, `${label}.lines[${li}].word_count`, al.words.length, bl.words.length);
      for (let wi = 0; wi < Math.min(al.words.length, bl.words.length); wi += 1) {
        const aw = al.words[wi];
        const bw = bl.words[wi];
        exact(failures, `${label}.lines[${li}].words[${wi}].content`, aw.content, bw.content);
        bbox(`${label}.lines[${li}].words[${wi}].bbox`, aw.bbox, bw.bbox);
        exact(failures, `${label}.lines[${li}].words[${wi}].char_count`, aw.chars.length, bw.chars.length);
        for (let ci = 0; ci < Math.min(aw.chars.length, bw.chars.length); ci += 1) {
          const ac = aw.chars[ci];
          const bc = bw.chars[ci];
          seen.chars += 1;
          exact(failures, `${label}.char[${li},${wi},${ci}].content`, ac.content, bc.content);
          exact(failures, `${label}.char[${li},${wi},${ci}].unicode`, ac.unicode, bc.unicode);
          bbox(`${label}.char[${li},${wi},${ci}].bbox`, ac.bbox, bc.bbox);
        }
      }
    }
  }

  exact(failures, "schema_version", native.schema_version, wasm.schema_version);
  exact(failures, "source", native.source, wasm.source);
  exact(failures, "document", native.document, wasm.document);
  exact(failures, "warnings", native.warnings, wasm.warnings);
  exact(failures, "page_count", native.pages.length, wasm.pages.length);

  for (let pi = 0; pi < Math.min(native.pages.length, wasm.pages.length); pi += 1) {
    const a = native.pages[pi];
    const b = wasm.pages[pi];
    const label = `pages[${pi}]`;
    exact(failures, `${label}.page_number`, a.page_number, b.page_number);
    near(`${label}.width`, a.width, b.width);
    near(`${label}.height`, a.height, b.height);
    exact(failures, `${label}.rotation`, a.rotation, b.rotation);
    exact(failures, `${label}.scanned`, a.scanned, b.scanned);
    exact(failures, `${label}.element_count`, a.elements.length, b.elements.length);
    for (let ei = 0; ei < Math.min(a.elements.length, b.elements.length); ei += 1) {
      const ae = a.elements[ei];
      const be = b.elements[ei];
      const elementLabel = `${label}.elements[${ei}]`;
      exact(failures, `${elementLabel}.type`, ae.type, be.type);
      exact(failures, `${elementLabel}.id`, ae.id, be.id);
      bbox(`${elementLabel}.bbox`, ae.bbox, be.bbox);
      if (ae.type === "text" && be.type === "text") compareText(elementLabel, ae, be);
      if (ae.type === "image" && be.type === "image") {
        seen.image += 1;
        for (const key of ["pixel_width", "pixel_height", "colorspace", "content_hash"]) {
          exact(failures, `${elementLabel}.${key}`, ae[key], be[key]);
        }
        for (let qi = 0; qi < 4; qi += 1) {
          near(`${elementLabel}.quad[${qi}].x`, ae.quad[qi][0], be.quad[qi][0]);
          near(`${elementLabel}.quad[${qi}].y`, ae.quad[qi][1], be.quad[qi][1]);
        }
      }
      if (ae.type === "path" && be.type === "path") {
        seen.path += 1;
        exact(failures, `${elementLabel}.fill`, ae.fill, be.fill);
        exact(failures, `${elementLabel}.stroke`, ae.stroke, be.stroke);
        if (ae.stroke_width == null || be.stroke_width == null) {
          exact(failures, `${elementLabel}.stroke_width`, ae.stroke_width, be.stroke_width);
        } else {
          near(`${elementLabel}.stroke_width`, ae.stroke_width, be.stroke_width);
        }
      }
      if (ae.type === "annotation" && be.type === "annotation") {
        seen.annotation += 1;
        exact(failures, `${elementLabel}.subtype`, ae.subtype, be.subtype);
        exact(failures, `${elementLabel}.uri`, ae.uri, be.uri);
      }
    }
  }

  return {
    ok: failures.length === 0,
    failures: failures.slice(0, 50),
    failure_count: failures.length,
    geometry_comparisons: geometryComparisons,
    max_geometry_delta_pt: maxGeometryDelta,
    seen,
  };
}

async function main() {
  for (const file of ["docray_wasm.js", "pdfium.js", "pdfium.wasm"]) {
    if (!fs.existsSync(path.join(pkgDir, file))) throw new Error(`missing ${path.join(pkgDir, file)}`);
  }
  if (!fs.existsSync(nativeCli)) throw new Error(`missing native CLI ${nativeCli}`);

  const PDFiumModule = require(path.join(pkgDir, "pdfium.js"));
  const pdfium = await PDFiumModule({
    locateFile: (file) => file.endsWith(".wasm") ? path.join(pkgDir, "pdfium.wasm") : file,
  });
  const docray = require(path.join(pkgDir, "docray_wasm.js"));
  if (!docray.initialize_pdfium_render(pdfium, docray.__wasm ?? docray, false)) {
    throw new Error("pdfium-render rejected the Emscripten Pdfium module");
  }

  try {
    docray.extract(Buffer.from("cap-check"), "", 1);
    throw new Error("input cap did not reject an oversized input");
  } catch (error) {
    const envelope = JSON.parse(String(error));
    if (envelope?.error?.code !== "too_large") {
      throw new Error(`input cap returned unexpected error: ${String(error)}`);
    }
  }

  const results = [];
  for (const fixture of fixtures) {
    const fixturePath = path.join(root, "testdata", fixture);
    const nativeRun = spawnSync(nativeCli, ["extract", fixturePath], {
      cwd: root,
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
    });
    if (nativeRun.status !== 0) {
      throw new Error(`native extraction failed for ${fixture}: ${nativeRun.stderr.trim()}`);
    }

    const native = JSON.parse(nativeRun.stdout);
    const wasm = JSON.parse(docray.extract(fs.readFileSync(fixturePath), "", 0));
    results.push({ fixture, ...compare(native, wasm) });

    // Lean output must match wasm-vs-native: same Rust renderer over the
    // same extraction. Structure/content compare exactly; numbers within
    // the JSON comparator's 2pt tolerance (pdfium builds differ sub-point).
    const leanNative = spawnSync(nativeCli, ["extract", fixturePath, "--format", "lean"], {
      cwd: root,
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
    });
    if (leanNative.status !== 0) {
      throw new Error(`native lean failed for ${fixture}: ${leanNative.stderr.trim()}`);
    }
    const leanWasm = docray.extract_lean(fs.readFileSync(fixturePath), "element", 0);
    const leanLines = { native: leanNative.stdout.split("\n").length, wasm: leanWasm.split("\n").length };
    const nNorm = leanNative.stdout.split("\n");
    const wNorm = leanWasm.split("\n");
    let firstDiff = -1;
    for (let i = 0; i < Math.max(nNorm.length, wNorm.length); i++) {
      if (!leanLineClose(nNorm[i], wNorm[i])) { firstDiff = i; break; }
    }
    const leanOk = firstDiff === -1;
    results.push({ fixture: fixture + " (lean)", ok: leanOk, failures: leanOk ? [] : [
      `lean differs at line ${firstDiff}: native=${JSON.stringify(nNorm[firstDiff])} wasm=${JSON.stringify(wNorm[firstDiff])}`,
    ], failure_count: leanOk ? 0 : 1 });
  }

  const ok = results.every((result) => result.ok);
  console.log(JSON.stringify({ ok, fixtures: results }, null, 2));
  if (!ok) process.exitCode = 1;
}

main().catch((error) => {
  console.error(error.stack ?? error);
  process.exitCode = 1;
});
