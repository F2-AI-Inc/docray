---
name: add-test-fixture
description: Add a deterministic PDF test fixture via gen_fixtures.rs with hand-computed expected geometry. Use when a new feature or bug fix needs a PDF exercising specific structure (fonts, forms, rotation, images, annotations, malformed content).
---

# Adding a test fixture

Fixtures are **generated, never downloaded or hand-crafted binaries** — the
generator is `crates/docray-pdf/examples/gen_fixtures.rs` (lopdf-based).
Real-world PDFs must not be committed (size, licensing, and privacy).

## Rules

1. **Deterministic**: no timestamps, no randomness. Running the generator
   twice must produce byte-identical files — CI enforces this
   (`gen_fixtures` then `git diff --exit-code testdata/`).
2. **Compute expected geometry by hand** before writing the test. Example:
   text at PDF-space `(72, 720)` on a 612×792 page lands at top-left-space
   `y ≈ 792 − 720 − ascent`. Put the arithmetic in a comment next to the
   assertion. Tolerances: ±2pt for glyph loose bounds (platform font
   substitution), exact for rects/images/annotations you placed yourself.
3. **Placement in `testdata/`**:
   - Top level (`testdata/foo.pdf`) → automatically picked up by the golden
     harness; you must generate its golden (see the `regenerate-goldens`
     skill) and eyeball it against your hand math before committing.
   - `testdata/malformed/` → crash-corpus; excluded from goldens; test via
     CLI exit codes / warnings instead.
   - `testdata/encrypted/` → excluded from goldens (extraction fails by
     design).
4. **Write the test at the level the behavior lives**: extraction behavior
   in `crates/docray-pdf/tests/`, CLI contract in `crates/docray-cli/tests/`,
   HTTP behavior in `crates/docray-server/tests/`.
5. A fixture earns its place by pinning behavior no existing fixture pins —
   check what `simple.pdf`, `form.pdf`, `rotated.pdf`, `image.pdf`,
   `link.pdf`, and `scan.pdf` already cover first.

## lopdf tips (learned the hard way)

- Build pages via the existing `base_doc()` helper; reuse the deterministic
  2×2 gray image for image content.
- Form XObjects: a `Stream` with `Type: XObject, Subtype: Form, BBox`,
  invoked with `q <matrix> cm /Fx Do Q` — the `cm` matrix is how PowerPoint
  places things, so prefer it over `/Matrix` entries.
- After changing the generator, regenerate twice and confirm `git status`
  shows only your intended new/changed files.
