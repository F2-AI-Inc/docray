#!/usr/bin/env node

const fs = require("node:fs");

const [nativePath, wasmPath] = process.argv.slice(2);
if (!nativePath || !wasmPath) {
  console.error("usage: node spike/compare.cjs NATIVE.json WASM.json");
  process.exit(2);
}

const native = JSON.parse(fs.readFileSync(nativePath));
const wasm = JSON.parse(fs.readFileSync(wasmPath));
const failures = [];
let geometryComparisons = 0;
let maxGeometryDelta = 0;
const seen = { text: 0, image: 0, path: 0, annotation: 0, chars: 0 };

function exact(label, a, b) {
  if (JSON.stringify(a) !== JSON.stringify(b)) {
    failures.push(`${label}: ${JSON.stringify(a)} != ${JSON.stringify(b)}`);
  }
}

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
  exact(`${label}.content`, a.content, b.content);
  exact(`${label}.font.name`, a.font.name, b.font.name);
  exact(`${label}.font.bold`, a.font.bold, b.font.bold);
  exact(`${label}.font.italic`, a.font.italic, b.font.italic);
  near(`${label}.font.size`, a.font.size, b.font.size);
  exact(`${label}.color`, a.color, b.color);
  exact(`${label}.line_count`, a.lines.length, b.lines.length);
  for (let li = 0; li < Math.min(a.lines.length, b.lines.length); li += 1) {
    const al = a.lines[li];
    const bl = b.lines[li];
    bbox(`${label}.lines[${li}].bbox`, al.bbox, bl.bbox);
    near(`${label}.lines[${li}].baseline_y`, al.baseline_y, bl.baseline_y);
    exact(`${label}.lines[${li}].word_count`, al.words.length, bl.words.length);
    for (let wi = 0; wi < Math.min(al.words.length, bl.words.length); wi += 1) {
      const aw = al.words[wi];
      const bw = bl.words[wi];
      exact(`${label}.lines[${li}].words[${wi}].content`, aw.content, bw.content);
      bbox(`${label}.lines[${li}].words[${wi}].bbox`, aw.bbox, bw.bbox);
      exact(`${label}.lines[${li}].words[${wi}].char_count`, aw.chars.length, bw.chars.length);
      for (let ci = 0; ci < Math.min(aw.chars.length, bw.chars.length); ci += 1) {
        const ac = aw.chars[ci];
        const bc = bw.chars[ci];
        seen.chars += 1;
        exact(`${label}.char[${li},${wi},${ci}].content`, ac.content, bc.content);
        exact(`${label}.char[${li},${wi},${ci}].unicode`, ac.unicode, bc.unicode);
        bbox(`${label}.char[${li},${wi},${ci}].bbox`, ac.bbox, bc.bbox);
      }
    }
  }
}

exact("schema_version", native.schema_version, wasm.schema_version);
exact("source", native.source, wasm.source);
exact("document", native.document, wasm.document);
exact("warnings", native.warnings, wasm.warnings);
exact("page_count", native.pages.length, wasm.pages.length);

for (let pi = 0; pi < Math.min(native.pages.length, wasm.pages.length); pi += 1) {
  const a = native.pages[pi];
  const b = wasm.pages[pi];
  const label = `pages[${pi}]`;
  exact(`${label}.page_number`, a.page_number, b.page_number);
  near(`${label}.width`, a.width, b.width);
  near(`${label}.height`, a.height, b.height);
  exact(`${label}.rotation`, a.rotation, b.rotation);
  exact(`${label}.scanned`, a.scanned, b.scanned);
  exact(`${label}.element_count`, a.elements.length, b.elements.length);
  for (let ei = 0; ei < Math.min(a.elements.length, b.elements.length); ei += 1) {
    const ae = a.elements[ei];
    const be = b.elements[ei];
    const el = `${label}.elements[${ei}]`;
    exact(`${el}.type`, ae.type, be.type);
    exact(`${el}.id`, ae.id, be.id);
    bbox(`${el}.bbox`, ae.bbox, be.bbox);
    if (ae.type === "text" && be.type === "text") compareText(el, ae, be);
    if (ae.type === "image" && be.type === "image") {
      seen.image += 1;
      exact(`${el}.metadata`, {
        pixel_width: ae.pixel_width,
        pixel_height: ae.pixel_height,
        colorspace: ae.colorspace,
        content_hash: ae.content_hash,
      }, {
        pixel_width: be.pixel_width,
        pixel_height: be.pixel_height,
        colorspace: be.colorspace,
        content_hash: be.content_hash,
      });
      for (let qi = 0; qi < 4; qi += 1) {
        near(`${el}.quad[${qi}].x`, ae.quad[qi][0], be.quad[qi][0]);
        near(`${el}.quad[${qi}].y`, ae.quad[qi][1], be.quad[qi][1]);
      }
    }
    if (ae.type === "path" && be.type === "path") {
      seen.path += 1;
      exact(`${el}.fill`, ae.fill, be.fill);
      exact(`${el}.stroke`, ae.stroke, be.stroke);
      if (ae.stroke_width == null || be.stroke_width == null) {
        exact(`${el}.stroke_width`, ae.stroke_width, be.stroke_width);
      } else {
        near(`${el}.stroke_width`, ae.stroke_width, be.stroke_width);
      }
    }
    if (ae.type === "annotation" && be.type === "annotation") {
      seen.annotation += 1;
      exact(`${el}.subtype`, ae.subtype, be.subtype);
      exact(`${el}.uri`, ae.uri, be.uri);
    }
  }
}

const result = {
  ok: failures.length === 0,
  failures: failures.slice(0, 50),
  failure_count: failures.length,
  geometry_comparisons: geometryComparisons,
  max_geometry_delta_pt: maxGeometryDelta,
  seen,
};
console.log(JSON.stringify(result, null, 2));
if (failures.length) process.exitCode = 1;
