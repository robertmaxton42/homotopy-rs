#!/bin/bash
# SessionStart hook for Claude Code cloud sessions: installs the tools needed to
# build/test homotopy-rs natively and to build the web app for in-browser testing.
set -euo pipefail

if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "$CLAUDE_PROJECT_DIR"

# Cloud checkouts are shallow; full history is useful for archaeology (git log/blame).
if [ "$(git rev-parse --is-shallow-repository)" = "true" ]; then
  git fetch --unshallow --quiet origin || true
fi

# Toolchain from rust-toolchain.toml (stable + wasm32 target + clippy/rustfmt).
rustup show active-toolchain >/dev/null
# rustfmt.toml uses nightly-only options and CI lints with nightly rustfmt:
# check formatting with `cargo +nightly fmt --all -- --check`.
rustup toolchain install nightly --profile minimal --component rustfmt >/dev/null 2>&1

# wasm-bindgen CLI must match the wasm-bindgen crate version in Cargo.lock exactly.
WBG_VERSION=$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | sed -n 's/^version = "\(.*\)"$/\1/p')
if ! wasm-bindgen --version 2>/dev/null | grep -qF "$WBG_VERSION"; then
  cargo install wasm-bindgen-cli --version "$WBG_VERSION" --locked --quiet
fi

# Warm build caches (cached with the container snapshot after this hook runs).
cargo fetch --quiet
cargo test --workspace --exclude homotopy-web --no-run --quiet
CARGO_TARGET_DIR=target/wasm cargo build -p homotopy-web --target wasm32-unknown-unknown --quiet
