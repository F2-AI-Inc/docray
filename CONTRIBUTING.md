# Contributing to docray

Thanks for your interest! docray is a small, sharply-scoped codebase and we
want contributions to be pleasant. This page is the human onboarding path;
the deep technical context lives in [AGENTS.md](AGENTS.md) — written for
coding agents but equally useful to people, and kept rigorously current.

## Where to start

- **Try it first**: the [hosted demo](https://f2-ai-inc.github.io/docray/try/)
  and the [docs](https://f2-ai-inc.github.io/docray/) show what docray does
  and the contracts it keeps.
- **Issues** labeled `good first issue` are curated entry points; `bug`
  issues with an attached reproducing PDF are the most valuable thing you
  can pick up.
- Open an issue before large changes — especially anything touching the JSON
  contract, which is frozen at the current schema version (see below).

## Development setup

```bash
git clone https://github.com/F2-AI-Inc/docray
cd docray
./scripts/fetch-pdfium.sh        # one-time: pinned native pdfium
cargo build -p docray-cli        # server tests spawn this binary
cargo test --workspace
```

Rust stable (1.88+). Run everything from the workspace root.

## Working with coding agents

This repo is deliberately agent-friendly: `AGENTS.md` is the canonical
context (with `CLAUDE.md` and `.cursorrules` symlinked to it), and
task-specific procedures live in `.claude/skills/`. If you contribute with
Claude Code, Codex, Cursor, or similar, your agent already has what it
needs — including the procedures it must not improvise (golden regeneration,
fixture authoring). Agent-assisted PRs are welcome; you own what you submit.

## The gates (CI enforces all of these)

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
cargo run -p docray-pdf --example gen_fixtures   # must produce zero diffs
```

Golden JSON files are **canonical on Linux** — never regenerate them on
macOS/Windows and never hand-edit them; the exact procedure is in
[`.claude/skills/regenerate-goldens`](.claude/skills/regenerate-goldens/SKILL.md).

## The rules that protect users

1. The no-parameter JSON output is **frozen** — byte-identical across
   changes. New capability is parameter-gated or additive with a version
   bump.
2. Same input bytes → byte-identical output (per platform + PDFium build).
   No timestamps, randomness, or unordered iteration near output.
3. **No silent failures** — skipped or unreadable content must surface in
   `warnings`.
4. Malformed input never panics; error codes and CLI exit codes are stable,
   machine-parsed contract.
5. PDFs are hostile input. Anything touching the worker/process lifecycle
   gets extra scrutiny.

## Tests

Every test must pin real behavior that could regress — a fix's test should
fail on the pre-fix code. We decline tests that exist for coverage's own
sake, and we decline fixes without a test that would have caught the bug.

## Pull requests

- Small and focused beats big-bang. The PR template's checklist mirrors the
  gates above.
- If your change alters the JSON contract, CLI, HTTP API, or configuration,
  update the corresponding page under `book/src/` in the same PR.
- Golden diffs must be explained class-by-class in the PR description.

## Developer Certificate of Origin (DCO)

By contributing, you certify the [Developer Certificate of
Origin](https://developercertificate.org/) — that you wrote the code or
otherwise have the right to submit it under this project's licenses. Sign
off each commit:

```bash
git commit -s    # adds "Signed-off-by: Your Name <you@example.com>"
```

## Licensing of contributions

docray is dual-licensed under [MIT](LICENSE-MIT) and
[Apache-2.0](LICENSE-APACHE). Unless you explicitly state otherwise, any
contribution intentionally submitted for inclusion in docray by you, as
defined in the Apache-2.0 license, shall be dual licensed as above, without
any additional terms or conditions.

## Security issues

Do **not** open public issues for suspected vulnerabilities — see
[SECURITY.md](SECURITY.md).

## Conduct

We follow the [Contributor Covenant](CODE_OF_CONDUCT.md). Be excellent to
each other.
