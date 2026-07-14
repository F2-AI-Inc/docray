## What & why

<!-- What changes, and what consumer-visible behavior it affects. -->

## Contract impact

- [ ] No change to the schema-1.1 (`char`) output — or this PR bumps the schema version with justification
- [ ] Error codes / CLI exit codes unchanged — or additions are documented in `book/`
- [ ] Golden diffs (if any) are explained line-class-by-line-class below and were regenerated **on Linux** (see `.claude/skills/regenerate-goldens`)

## Gates (all must pass locally — CI re-checks)

- [ ] `cargo fmt --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo build -p docray-cli && cargo test --workspace`
- [ ] `cargo run -p docray-pdf --example gen_fixtures` produces zero diffs
- [ ] Docs in `book/` updated if the JSON contract, CLI, HTTP API, or configuration changed

## Tests

<!-- Which new assertions pin this change? A fix's test must fail on the pre-fix code. -->

## Golden changes (if any)

<!-- Why each class of changed lines is expected. -->
