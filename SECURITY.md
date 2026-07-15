# Security Policy

docray parses untrusted documents by design, so we take reports seriously.

## Reporting a vulnerability

**Please do not open a public issue.** Use GitHub's private vulnerability
reporting: [Security → Report a vulnerability](https://github.com/F2-AI-Inc/docray/security/advisories/new)
on this repository. We will acknowledge reports on a best-effort basis and
coordinate a fix and disclosure with you.

## Threat model (what counts as a vulnerability)

docray's security posture assumes **every input document is hostile**:

- The HTTP server never parses documents in-process; each extraction runs in
  an isolated worker subprocess under a wall-clock timeout, memory limit,
  and streaming output cap. The in-browser (WASM) engine applies the same
  philosophy with worker isolation and input/output caps.
- A crafted document that **escapes these bounds** — hangs a worker past its
  timeout, bypasses a cap, crashes the *server* rather than a worker,
  achieves code execution, or exfiltrates data from the playground — is a
  vulnerability. Please report it.
- A crafted document that merely produces wrong or incomplete extraction
  output (with or without `warnings`) is a bug, not a vulnerability — an
  ordinary public issue is perfect for those.

## Supported versions

The latest release and `main`. We do not backport fixes to older releases.

## Hardening guidance for operators

docray ships with no authentication and is intended to run behind your own
access controls. Do not expose `docray-server` directly to the public
internet; see the deployment documentation for resource-limit configuration.

## No warranty

docray is provided "AS IS", without warranty of any kind. Operating it —
including on untrusted input — is at your own risk; see
[LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE) (§7–§8) and
[NOTICE](NOTICE).
