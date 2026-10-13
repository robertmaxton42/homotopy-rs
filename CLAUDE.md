# Notes for Claude

## Policy: never make the user wait blind

It is sometimes necessary to make the user wait. It is very rarely necessary to make them wait
with no explanation and no way out. Any operation that can take noticeably long (rule of thumb:
anything that can exceed ~100 ms on a realistic diagram) must:

1. **Not lock up the interface.** Run it off the main thread (web worker), or, where that is not
   possible, split it into chunks that yield back to the event loop between steps.
2. **Say that it is working, and on what.** Show a visible busy indicator naming the operation
   (e.g. "Inferring instances…"), with progress where progress is knowable.
3. **Be cancellable.** Clicking the indicator or pressing `Esc` cancels it, leaving the project
   exactly as it was before the operation started.

New features that add expensive work are not done until they meet all three. When touching an
existing expensive path that does not, fix it or note it as a known gap.

## Development

- Format with `cargo +nightly fmt` (rustfmt.toml uses nightly options); clippy runs under the
  pinned toolchain with `-D warnings`. Cargo-husky pre-commit/pre-push hooks run test, check,
  clippy and fmt.
- Shallow clones lack a merge base: `git fetch --unshallow` first.
- Debug web build: `.claude/dev/build-dist.sh`, then `python3 -m http.server 8765` in `dist/`.
  It uses `.claude/dev/highs-shim.js` because the nix HiGHS build is blocked in the sandbox.
- Commit messages containing backticks: write them to a file and use `git commit -F`.
