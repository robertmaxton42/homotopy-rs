#!/bin/bash
# Builds a debug copy of the web app into dist/ for local (headless-browser) testing,
# without nix. HiGHS (used for diagram layout) comes from the npm `highs` package
# through a small shim exposing the same self.Highs_* globals as flake.nix's post.js.
# Serve with: (cd dist && python3 -m http.server 8765)
set -euo pipefail
cd "$(dirname "$0")/../.."

# A shared target directory (CARGO_TARGET_DIR) is honoured, with the wasm build in a folder of its
# own in it; DIST is where the build goes (default dist/).
WASM_TARGET="${CARGO_TARGET_DIR:+$CARGO_TARGET_DIR/wasm}"
WASM_TARGET="${WASM_TARGET:-target/wasm}"
DIST="${DIST:-dist}"
CARGO_TARGET_DIR="$WASM_TARGET" cargo build -p homotopy-web --target wasm32-unknown-unknown
mkdir -p "$DIST"
wasm-bindgen --out-dir "$DIST" --no-typescript --target web \
  "$WASM_TARGET/wasm32-unknown-unknown/debug/homotopy_web.wasm"
cp -r homotopy-web/static/* "$DIST"/

if [ ! -f "$DIST/highs.wasm" ]; then
  tmp=$(mktemp -d)
  (cd "$tmp" && npm pack highs@1.15.3 --silent >/dev/null && tar xzf highs-*.tgz)
  cp "$tmp"/package/build/highs.{mjs,wasm} "$DIST"/
  rm -rf "$tmp"
  # The emscripten module keeps HEAPU8 private; the post.js glue needs it.
  sed -i 's/HEAPU8=new Uint8Array(b);/HEAPU8=new Uint8Array(b);Module["HEAPU8"]=HEAPU8;/' "$DIST/highs.mjs"
fi
cp .claude/dev/highs-shim.js "$DIST/highs.js"
