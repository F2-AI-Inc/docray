#!/usr/bin/env bash
# Fetches a pinned PDFium build from bblanchon/pdfium-binaries into .pdfium/
set -euo pipefail

# Pinned to chromium/7934: pdfium-render 0.8.37 requires symbols (e.g.
# FPDFFormObj_RemoveObject) absent from older builds such as chromium/7047.
VERSION="${PDFIUM_VERSION:-chromium/7934}"
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)  ASSET="pdfium-mac-arm64.tgz" ;;
  Darwin-x86_64) ASSET="pdfium-mac-x64.tgz" ;;
  Linux-x86_64)  ASSET="pdfium-linux-x64.tgz" ;;
  Linux-aarch64) ASSET="pdfium-linux-arm64.tgz" ;;
  *) echo "unsupported platform" >&2; exit 1 ;;
esac

URL="https://github.com/bblanchon/pdfium-binaries/releases/download/${VERSION}/${ASSET}"
mkdir -p .pdfium
echo "fetching ${URL}"
curl -fL "$URL" | tar -xz -C .pdfium
echo "pdfium installed under .pdfium/lib"
