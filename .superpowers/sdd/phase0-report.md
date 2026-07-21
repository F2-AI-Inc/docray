# Phase 0 implementation report

Branch: `feature/optional-hierarchy`

Implementation commit: `27a2595` (`feat: add capability-aware text hierarchy`)

## Change evidence

1. `TextElement.lines` is now `Option<Vec<Line>>` with serde defaulting and
   omission for `None`. PDF extraction always constructs `Some(lines)`.
   `optional_lines_preserve_old_json_shape_and_omit_when_absent` proves the
   old literal JSON shape is unchanged for `Some`, `None` omits the key, old
   JSON deserializes to `Some`, and a missing key deserializes to `None`.
2. Word projection treats a missing hierarchy as an empty word sequence. The
   defensive behavior is documented at the projection and
   `word_projection_with_missing_hierarchy_has_no_words` proves both JSON
   `"words": []` and a lean `T` header with no `w` records.
3. `Extractor` now requires explicit `capabilities() -> Capabilities` and
   `Capabilities` remains an extensible struct with only
   `finest_granularity`. `Granularity::rank()` orders Element < Word < Char.
   `PdfExtractor` explicitly advertises Char.
4. `ExtractError::GranularityUnavailable { requested, finest }` exposes the
   stable `granularity_unavailable` code. `check_granularity` treats an absent
   request as Char and rejects requests finer than the extractor capability.
   Element-only stub tests cover None, Char, Word, Element, and lean's
   normalized Element request; the PDF capability test covers every request.
5. The CLI checks capabilities before extraction and maps the new error to
   exit 8 through a directly tested mapping seam. The server recognizes worker
   exits 2 through 8 and maps `granularity_unavailable` to HTTP 400, with unit
   tests for both mappings.
6. Public CLI and HTTP error tables document the new stable error without
   naming future formats. `AGENTS.md` includes exit 8 and the new error code.

## PDF byte-identity proof

- PDF construction always supplies `Some(lines)`, so serde emits the same
  `lines` key, in the same field position, with the same nested representation.
- The model test compares serialization against a complete literal of the old
  `TextElement` JSON shape.
- The full workspace golden suite passed, including `goldens_match`, compact
  goldens, lean goldens, and extraction determinism.
- After running the fixture generator, `git diff --exit-code -- testdata/`
  exited 0. No fixture or golden changed.

## Gates

- `cargo fmt --check` — passed.
- `cargo clippy --all-targets -- -D warnings` — passed.
- `cargo build -p docray-cli` — passed.
- `cargo test --workspace` — passed, including all socket-backed server tests.
- `cargo run -p docray-pdf --example gen_fixtures` — passed.
- `git diff --exit-code -- testdata/` — passed; empty diff.

## Deviations

None.
