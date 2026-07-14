---
name: regenerate-goldens
description: Regenerate golden JSON test files the only correct way — on Linux via Docker, since goldens are Linux-canonical (platform font substitution shifts glyph metrics across OSes). Use when an intended behavior change makes golden tests fail, or after adding a fixture.
---

# Regenerating golden files

Golden JSON under `testdata/golden/` is the byte-exact regression contract,
**canonical on Linux** (the CI and production platform). Never hand-edit a
golden, and never commit goldens generated on macOS/Windows — CI will reject
them (fixture fonts are non-embedded, so pdfium substitutes platform fonts
and glyph metrics drift ~1pt between OSes).

## Procedure

From the repo root (works on any host with Docker):

```bash
docker run --rm -v "$PWD":/w -w /w rust:1.88-slim bash -c '
set -e
apt-get update -qq >/dev/null && apt-get install -y -qq curl ca-certificates >/dev/null
mkdir -p /tmp/pdfium
case $(uname -m) in x86_64) A=linux-x64;; aarch64) A=linux-arm64;; esac
V=$(grep -o "chromium/[0-9]*" scripts/fetch-pdfium.sh | head -1 | tr / %2F)
curl -fsL https://github.com/bblanchon/pdfium-binaries/releases/download/$V/pdfium-$A.tgz | tar -xz -C /tmp/pdfium
export DOCRAY_PDFIUM_DIR=/tmp/pdfium/lib CARGO_TARGET_DIR=/tmp/target
UPDATE_GOLDEN=1 cargo test -p docray-pdf --test golden 2>&1 | tail -3
'
```

## Review the diff before committing — this is the point

```bash
git diff testdata/golden/
```

- **Intended behavior change?** Every changed line must be explained by your
  change. An unexpected changed field means your change altered behavior you
  didn't intend — stop and investigate, do not commit.
- **Adding a fixture?** New golden files only; existing goldens must be
  byte-identical.
- Verify numeric-only expectations: if you only touched geometry, only
  numbers should change.

Then run the suite normally on your machine (`cargo test -p docray-pdf`) —
non-Linux hosts validate structurally with 1.5pt tolerance and should pass.

State in your PR description *why* the goldens changed, line-class by
line-class (e.g. "baseline_y on all text elements: the new origin-based
baseline").
