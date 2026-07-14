---
name: verify-in-playground
description: Visually verify extraction changes against real PDFs using the embedded playground before opening a PR. Use after any change to extraction geometry, grouping, granularity shapes, or the playground itself.
---

# Verifying changes in the playground

Unit tests and goldens pin known fixtures; the playground is how you check a
change against *real* documents — coordinate bugs that pass synthetic
fixtures are obvious the moment boxes don't sit on the ink.

## Procedure

```bash
./scripts/fetch-pdfium.sh          # once
cargo build -p docray-cli          # the server spawns this binary
cargo run -p docray-server         # then open http://localhost:41619/playground
```

Drop in PDFs that exercise your change (use your own local PDFs — never
commit them). Then check:

1. **boxes lens vs source lens**: boxes must sit exactly on the rendered
   ink. Off-by-scale or flipped-axis bugs are instantly visible here.
2. **Rotated pages**: if your change touches coordinates, test a rotated
   document specifically — rotation is where coordinate bugs hide.
3. **Granularity selector**: flip char/word/element on the same document;
   the boxes should tell the same story at every level.
4. **Warnings**: the status line reports the count; document JSON shows
   them. New unexplained warnings on documents that were clean before =
   regression.
5. **json lens + click-through**: click a box you changed the handling of,
   confirm the JSON element matches expectations.

For playground code changes, additionally verify: upload a second document
mid-render (generation guard), an encrypted/garbage file (error surface),
and a 50+ page document (lazy rendering, memory).

This is a verification step, not a substitute for tests — anything you
confirm visually that could regress silently should also gain an assertion
if one doesn't exist.
