# docray — agent context

docray extracts PDFs into lossless JSON: every physical element (text with a
char → word → line hierarchy, images, paths, annotations) with bounding
boxes, fonts, and colors. Rust workspace; PDFium does glyph geometry; our
code owns everything above it. This file is the canonical context for coding
agents (CLAUDE.md and .cursorrules are symlinks to it).

## Repository map

| Path | Role |
|---|---|
| `crates/docray-model` | The serde JSON schema — the contract every consumer depends on |
| `crates/docray-core` | `Extractor` trait, format sniffing, geometric char→word→line grouping |
| `crates/docray-pdf` | PDFium-backed extractor (`pdfium-render`, pinned exact version) |
| `crates/docray-cli` | `docray` binary — also the server's isolation worker subprocess |
| `crates/docray-server` | axum HTTP: sync + async jobs; embedded playground at `/playground` |
| `testdata/` | Generated fixtures + golden JSON (see skills below before touching) |
| `book/` | Docs site (mdBook → GitHub Pages) |

## Setup & commands

```bash
./scripts/fetch-pdfium.sh        # REQUIRED once: native pdfium into ./.pdfium
cargo build -p docray-cli        # server tests spawn this binary — build it first
cargo test --workspace           # full suite; tests self-set DOCRAY_PDFIUM_DIR
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo run -p docray-pdf --example gen_fixtures  # must produce ZERO diffs (determinism)
cargo run -p docray-server       # http://localhost:41619 (+ /playground)
mdbook serve book                # docs preview
```

Run everything from the workspace root. CI enforces all of the above plus a
Docker build/smoke test.

## Hard rules (the contract — violating these breaks consumers)

1. **Schema stability.** The no-parameter (`char`) response is frozen at
   schema `1.1` and must stay **byte-identical**: never add, rename, remove,
   or reorder fields on that path. Granularity-shaped responses are schema
   `1.2`. New capability = new field behind a parameter, or a minor version
   bump with explicit review.
2. **Determinism.** Identical input bytes → byte-identical JSON on a given
   platform + PDFium build. No timestamps, randomness, or HashMap iteration
   anywhere near output. Fixture generation must be deterministic too.
3. **No silent failures.** Anything skipped, unreadable, or partially parsed
   goes into `warnings`. Option-typed metadata may be silently `None` (that
   is its meaning); *geometry* failures must warn. Never let a malformed
   input panic the extractor — return typed `ExtractError`s.
4. **Coordinates** are top-left origin, y-down, PDF points, **after page
   rotation**, rounded via `round3`. Compact granularities round to 1
   decimal.
5. **Stable error contract.** CLI exit codes (0/2/3/4/5/6/7/8) and error code
   strings (`unsupported_format`, `encrypted_pdf`, `parse_failure`,
   `io_error`, `too_many_pages`, `bad_format`, `timeout`, `crash`,
   `output_too_large`, `granularity_unavailable`) are
   parsed by machines. Do not change them; add new ones deliberately.
6. **Hostile input is the norm.** The server must never parse a PDF
   in-process — extraction happens in the spawned CLI worker under a
   timeout, memory rlimit, and streaming output cap. Changes to
   worker/pipe/process lifecycle need adversarial review (deadlocks,
   orphaned children, unbounded buffering are the historical bugs here).
7. **Pinned native dependency.** `pdfium-render` is pinned with `=` and the
   pdfium binary build is pinned in `scripts/fetch-pdfium.sh` — bump both
   together, deliberately, with the full suite + corpus spot-checks.

## Testing philosophy

- **Every test pins real behavior that could regress.** No ceremonial tests,
  no vacuous assertions (an assertion that passes on the pre-fix code is a
  bug in the test). If you fix a bug, the test must fail on the old code.
- **Goldens are canonical on Linux** (CI/production platform). Fixture fonts
  are non-embedded, so glyph metrics differ per OS; non-Linux machines
  compare structurally with 1.5pt tolerance. To regenerate goldens, use the
  `regenerate-goldens` skill below — never hand-edit golden JSON.
- The malformed-input corpus (`testdata/malformed/`, `testdata/encrypted/`)
  must always pass: structured errors, never hangs or panics.
- Server tests boot the real binary on ephemeral ports; fake workers are
  shell scripts (see `tests/sync_api.rs` for the pattern).

## Playground (crates/docray-server/assets/playground.html)

Single self-contained file, embedded via `include_str!`, no build step.
Rules: every PDF-controlled string is escaped before touching `innerHTML`;
pdf.js stays ≥ 4.2.67 (CVE-2024-4367) with `isEvalSupported: false`; canvas
memory is byte-capped; async renders are guarded by generation/request
tokens. Keep the light-table aesthetic (IBM Plex Mono / Instrument Serif,
amber on near-black).

## Docs (book/)

Docs describe **what is, not what is planned** — no roadmaps or intentions
in public docs, PR descriptions, or commit messages. When you change the
JSON contract, CLI flags, HTTP API, or configuration, update the
corresponding `book/src/` page in the same PR. The granularity guidance is
load-bearing: token-conscious consumers use `element`.

## Repo skills (agentskills.io format)

Task-specific procedures live in `.claude/skills/<name>/SKILL.md`. Agents
without native skill support should read the relevant file before starting
that kind of task:

- `.claude/skills/regenerate-goldens` — the only correct way to update
  golden JSON (Linux-canonical, via Docker), and how to review the diff.
- `.claude/skills/add-test-fixture` — adding deterministic PDF fixtures via
  `gen_fixtures.rs`, with hand-computed expected geometry.
- `.claude/skills/verify-in-playground` — visually verifying extraction
  changes against real documents before opening a PR.

## PR expectations

Before opening a PR: `cargo fmt`, clippy clean, full workspace suite green,
fixture regeneration diff-free, and golden changes justified line-by-line in
the PR description. CI must be green. Small, reviewable PRs over big-bang
changes; the PR template checklist mirrors this section.

## CI/workflow security rules

- GitHub-hosted runners only — never self-hosted.
- Every action is pinned to a full commit SHA with a version comment;
  Dependabot maintains the pins. Never add an action pinned to a tag/branch.
- Never introduce `pull_request_target`, `workflow_run`, or `issue_comment`
  triggered workflows that act on externally controllable input — that class
  of trigger has a history of secret-exfiltration exploits. Stick to `push`,
  `pull_request`, tags, and `workflow_dispatch`.
- Workflows declare least-privilege `permissions:` blocks explicitly. Write
  permissions (releases, packages, pages) exist only in workflows triggered
  exclusively by maintainer actions (tag/main pushes) — never grant write to
  anything an external user can trigger.
- **No AI bots/apps attached to this repository.** Do not install GitHub
  Apps that act as coding agents with repo permissions, and never add a
  workflow that routes externally controllable text (issues, comments, PR
  bodies) into AI tooling holding a token — prompt injection plus a write
  token is a supply-chain incident. Coding agents are welcome as LOCAL
  tools under a human contributor's own credentials (that is what this
  file is for); they are not welcome as repo-resident automation.

## Releasing

Tag `vX.Y.Z` on a green `main` (`git tag vX.Y.Z && git push origin vX.Y.Z`).
The Release workflow builds 4-platform binary archives (pdfium bundled,
smoke-tested from a foreign CWD), publishes a GitHub Release with checksums,
and pushes multi-arch images to `ghcr.io/f2-ai-inc/docray`. Then update
`Formula/docray.rb` in `F2-AI-Inc/homebrew-tap` with the new version and the
four sha256s from the release assets. Intel macOS binaries cross-compile
from the arm64 runner — do not reintroduce `macos-13` runners (30+ minute
queues).
