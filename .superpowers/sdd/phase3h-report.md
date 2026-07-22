# Phase 3h: first-class chart element

Date: 2026-07-22

Branch: `feature/chart-element`

Implementation commit: `698a0e5`

## Result

Phase 3h is complete. PPTX charts are emitted as first-class `chart` elements
instead of synthesized text. The model carries the graphic-frame bbox, mapped
chart type, optional chart title, ordered series, optional series names, and
index-paired category/value points. Values continue to use the existing
`formatCode` formatter, including percentage rendering such as `0.41` with
`0%` becoming `41%`.

Granularity-shaped Char, Word, and Element responses now report schema `1.6`.
The default/no-parameter PDF response remains schema `1.1` and byte-identical.
SmartArt remains synthesized text.

## Model and formats

- Added `Element::Chart(ChartElement)`, `ChartSeries`, and `ChartPoint` with
  the specified optional-field serialization.
- Added compact chart projection with one-decimal bbox rounding and unchanged
  formatted value strings.
- Added deterministic lean records:

  ```text
  CH x0 y0 x1 y1 type [title]
  s series-name
  p [category] value
  ```

- Unnamed series omit `s`; values without a category emit `p value`.
- Chart title, series name, category, and value all pass through
  `escape_text`; the hostile-series-name test proves a document cannot forge
  `CH`, `s`, or `p` records.
- Updated element/word legends and the JSON, lean, granularity, PPTX, README,
  and agent contract documentation for schema `1.6`.

## PPTX extraction

- The first chart node in `plotArea` determines `chart_type`; combo-chart
  series remain in document order. Known types map to `bar`, `pie`,
  `doughnut`, `line`, `area`, and `scatter`; all other chart nodes map to
  `other`.
- Every `c:ser` becomes a `ChartSeries`. Category and finite value caches are
  paired by `idx`; category-only entries are omitted because `value` is
  required, while unmatched values remain points with no category.
- Series names keep the existing rich-text/cache resolution. Numeric values
  keep the existing finite-number parsing and `format_chart_number` path.
- A chart with neither title nor series retains the existing
  `chart graphicFrame has no extractable text` warning. Missing or unreadable
  related parts retain the existing warning. Geometry still comes from the
  containing graphic frame.
- Axis-title strings from the former synthesized text are not fields in the
  structured `ChartElement` contract and are therefore not serialized.

## Tests

- Model chart JSON shape, optional-field omission, round-trip, compact bbox,
  and schema `1.6` assertions.
- Lean `CH`/`s`/`p` rendering, unnamed series/value-only point behavior, and
  hostile record-injection escaping.
- Chart type mapping, combo-chart first-node behavior, `other` fallback,
  hostile indices, non-finite values, and required-value pairing.
- `chart.pptx`: `bar`, title `Quarterly revenue`, named Revenue/Costs series,
  and Q1/Q2 formatted values.
- `percent-chart.pptx`: `doughnut`, unnamed series, and
  Direct `41%` / Reseller `59%`.
- WASM/native parity now includes both chart fixtures in JSON and lean modes.

## Golden justification

Goldens were regenerated through their test harnesses; PDF goldens used the
Linux/Docker canonical procedure.

- All six frozen no-parameter PDF `1.1` goldens are unchanged relative to
  `main`; the frozen literal serialization test also passes.
- The four explicit compact PDF goldens (`simple.element/word` JSON and lean)
  change only the required schema/header token from `1.5` to `1.6`.
- Non-chart PPTX element JSON changes only `schema_version: 1.5 -> 1.6`.
- Non-chart PPTX lean changes only the `v1.6` header and, where structured
  detail is present, the fixed legend that now documents `CH`/`s`/`p`.
- `chart` and `percent-chart` are the only goldens with payload changes: the
  synthesized text/font object and `T` line are replaced by structured chart
  JSON and `CH`/`s`/`p` records. The percent strings remain `41%` and `59%`.

An automated diff audit rejected any non-chart golden change outside those
schema-header and legend classes.

## Validation

All required gates passed on the committed implementation:

- `cargo fmt --all --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo build -p docray-cli`
- `cargo test --workspace`
- PDF and PPTX `gen_fixtures` each run twice, followed by
  `git diff --exit-code -- testdata` (zero diff)
- frozen `1.1` golden diff against `main` (zero diff)
- `cargo build --release -p docray-cli`
- `./scripts/build-wasm.sh --target nodejs`
- `node scripts/wasm-parity.cjs web/wasm-nodejs target/release/docray`:
  `ok: true`, 16 JSON/lean checks
- `./scripts/build-wasm.sh --target web`
- `./scripts/build-try.sh`
- `mdbook build book`

The only cargo diagnostic was the pre-existing warning that both fixture
examples share the output filename `gen_fixtures`; all commands exited zero.
