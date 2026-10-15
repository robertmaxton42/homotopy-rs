#!/bin/bash
# Builds a debug copy of the web app into dist/ for local (headless-browser) testing,
# without nix. HiGHS (used for diagram layout) comes from the npm `highs` package
# through a small shim exposing the same self.Highs_* globals as flake.nix's post.js.
# Serve with: (cd dist && python3 -m http.server 8765)
set -euo pipefail
cd "$(dirname "$0")/../.."

# (A CARGO_TARGET_DIR in the environment is a build directory shared with other checkouts: the
# wasm build goes in a directory of its own under it.)
WASM_TARGET="${CARGO_TARGET_DIR:-target}/wasm"
CARGO_TARGET_DIR="$WASM_TARGET" cargo build -p homotopy-web --target wasm32-unknown-unknown
mkdir -p dist
wasm-bindgen --out-dir dist --no-typescript --target web \
  "$WASM_TARGET/wasm32-unknown-unknown/debug/homotopy_web.wasm"
cp -r homotopy-web/static/* dist/

if [ ! -f dist/highs.wasm ]; then
  tmp=$(mktemp -d)
  (cd "$tmp" && npm pack highs@1.15.3 --silent >/dev/null && tar xzf highs-*.tgz)
  cp "$tmp"/package/build/highs.{mjs,wasm} dist/
  rm -rf "$tmp"
  # The emscripten module keeps HEAPU8 private; the post.js glue needs it.
  sed -i 's/HEAPU8=new Uint8Array(b);/HEAPU8=new Uint8Array(b);Module["HEAPU8"]=HEAPU8;/' dist/highs.mjs
fi
cp .claude/dev/highs-shim.js dist/highs.js
