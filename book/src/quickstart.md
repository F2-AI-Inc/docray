# Quickstart

## Try it without installing

The playground runs entirely in your browser — extraction happens locally
via WebAssembly and **your document never leaves your machine**:
**[try docray →](try/index.html)** *(also embedded in every docray-server at
`/playground`)*

## Install

```bash
# Homebrew (macOS/Linux)
brew install f2-ai-inc/tap/docray

# or a container (server + playground included)
docker run -d --rm -p 41619:41619 ghcr.io/f2-ai-inc/docray:latest

# or grab a prebuilt archive from the releases page — pdfium is bundled
```

Building from source instead? Clone the repo, run `./scripts/fetch-pdfium.sh`
once, then `cargo build --release -p docray-cli -p docray-server`.

## Extract your first PDF

```bash
docray extract your.pdf --granularity element | jq .
```

PPTX uses element granularity as well:

```bash
docray extract deck.pptx --granularity element | jq .
```

## Run the server + playground

```bash
cargo build --release -p docray-cli -p docray-server
./target/release/docray-server
```

```text
docray-server listening on http://localhost:41619
playground UI:        http://localhost:41619/playground
```

Open the playground and drop a PDF, PPTX, or DOCX on it. PDFs appear as rendered
pages; PPTX slides appear as offline visual renders inside a locked-down browser
sandbox, with an extraction-derived structure schematic as the fallback. Word
documents appear as a scrollable isolated render beside honest flow lenses:
positioned containers only in BOXES and a labeled reading-order schematic in
X-RAY. All formats sit beside extracted content and live JSON — see
[the playground](playground.md).

Extract over HTTP:

```bash
curl -sf -F file=@your.pdf 'http://localhost:41619/v1/extract?granularity=element'
curl -sf -F file=@deck.pptx 'http://localhost:41619/v1/extract?granularity=element'
```

## Docker

```bash
docker build -t docray .
docker run -d --rm -p 41619:41619 docray
curl -sf http://localhost:41619/healthz
```

The image bundles the pinned PDFium build; no host dependencies.

## Why port 41619?

It's deliberately obscure so it never collides with your other dev servers.
Override with `DOCRAY_PORT`.
