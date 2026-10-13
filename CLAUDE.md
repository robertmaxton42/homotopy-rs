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

While such an operation runs, further expensive input is **refused, not queued**: the indicator
stays up (and should visibly acknowledge the refused input), and cancelling never replays
anything. "Is this thing responding? *click click*" must not stack up several expensive jobs.

New features that add expensive work are not done until they meet all three. When touching an
existing expensive path that does not, fix it or note it as a known gap.

### How the policy is met

- **Jobs** (`homotopy_model::job::Job`) are the unit of expensive work: a state machine that
  works on a *copy* of what it needs, does a small `step` at a time, reports a `label` and
  `progress`, and ends in an output that is committed in one go. Dropping a job changes nothing.
  `ActionsJob` (a sequence of proof actions, with `MergeExchanges` split into one step per
  cospan) and `AttachOptionsJob` (one step per generator, including its `infer_instances`) live
  in `homotopy-model`; `ExportJob` in `homotopy-web`. A job holds no references into the app's state, only its own copy of its input; so
  running one in a Web Worker later means serialising that input and its output (diagrams are
  hash-consed in thread-locals and cannot be shared), without changing its callers.
- `State::start(action)` either finishes an action there and then, or returns a `Task` (a job
  for the web crate); `State::finish(output)` commits it (history, crash log, selection), and
  may return a further task. `State::update` runs those to the end for tests and the like, so
  chunked and unchunked results are the same code. Nothing is pushed to the history or the
  crash log until `finish`: that is what makes cancelling free.
- The app (`App::dispatch`/`work` in `app.rs`) runs a task ~10ms at a time (`SLICE`), yielding
  with `TimeoutFuture(0)`; a task that finishes in the first slice never shows anything.
  `Message::Slice(id)` carries the job's id, so slices of a cancelled job do nothing.
- **Indicator**: `components/busy.rs`, a pill at the top centre. It is put on the page when a
  job starts but fades in after 150ms with a CSS animation (so quick jobs do not flicker, and a
  step that blocks the page still gets it on screen: the browser keeps compositor animations
  going). It is told what to show through `busy::show/hide/refused`, so progress updates do not
  re-render the app. Clicking it or pressing `Esc` (a capturing `keydown` listener in the app)
  cancels; a step that blocks the page delays the `Esc` until it is over, but the job is then
  cancelled instead of committed.
- **Refusal**: while a job runs, `Action::allowed_while_busy` is false for everything but
  highlighting, remote metadata and help: this includes undo/redo (keys and mouse buttons), as
  the job works on the history as it was. Refused input shakes the pill (and shows it at once).
  Pure view changes that do not go through `dispatch` (pan/zoom, opening drawers, settings)
  still work.
- **What is heavy**: `Action::heavy_label` / `proof::Action::work_label` name the actions that
  can take long, from timings of the debug build (see the doc comment on `work_label`). For
  those whose next step cannot be split up (a homotopy, a theorem, importing a project) the app
  puts the indicator on the page and waits for the browser to paint before blocking. Cheap
  actions (anything typed or stepped through quickly: renames, slices, ...) never get a label,
  because input that arrives while a job runs is refused.

New expensive work: write it as a job (or give an existing job another kind of step), label it,
and check it with the debug hooks below.

## Development

- Format with `cargo +nightly fmt` (rustfmt.toml uses nightly options); clippy runs under the
  pinned toolchain with `-D warnings`. Cargo-husky pre-commit/pre-push hooks run test, check,
  clippy and fmt.
- Shallow clones lack a merge base: `git fetch --unshallow` first.
- Debug web build: `.claude/dev/build-dist.sh`, then `python3 -m http.server 8765` in `dist/`.
  It uses `.claude/dev/highs-shim.js` because the nix HiGHS build is blocked in the sandbox.
- Commit messages containing backticks: write them to a file and use `git commit -F`.
- Debug builds expose `window.__homotopy` (`app/debug_hooks.rs`): `dispatch(json)` for an action
  (`{"Proof": ...}`, `{"SelectPoint": [[...], false]}`, `{"Select": 0}`, `"Undo"`, ...),
  `importActions(bytes)`, `importProof(bytes)`, `cancel()`, `state()` (busy label, history
  length, a hash of the serialized project, crash log size), `profile(json)` (replays recorded
  actions, timing each), `profileDrafts()`. Setting `window.__homotopy_step_delay_ms = 50` makes
  every step of a job spin that long, to have something slow to test with in a headless
  browser (watch `state().busy`, the `.busy__pill` element, send `Escape`). Timings of jobs are
  logged: "<label> took Nms in K steps (longest Mms)".

## Known gaps against the policy above

Done (chunked, behind the indicator, cancellable, input refused meanwhile): attach options
for a click in the workspace (`SelectPoint`), `MergeExchanges` and the automatic merging after
a homotopy (one cancellable unit with the homotopy), replaying actions (`ImportActions`),
multi-slice and other text exports, and the single steps listed below at least get the
indicator on screen (and `Esc`) before and after they block.

Expensive work that still blocks the main thread while it runs (the page cannot respond, and
`Esc` is only seen when the step ends, when it still prevents the commit). Only a Web Worker
fixes these; numbers are the debug build, on `homotopy-model/tests/examples` and a project with
limits as schemas:

- A single homotopy (`Expand`/`Contract`, with its typecheck): 10ms to 1.5s; `Theorem` 20-260ms;
  `Squash` 10-230ms; importing a project (`ImportProof`) 50ms for 40KB; instantiating a
  schema 10-35ms.
- `Signature::infer_instances` for one generator inside the attach options: up to 30ms, in one
  step. Typically the whole search is 1-5ms.
- Laying out the diagram on render (`PreparedDiagram::new`, HiGHS; GL meshes): 5-70ms on the
  examples, not behind an indicator and not cancellable.
- `Signature::infer_replacements`, called when a replacement is chosen or cleared in the
  Instantiate panel (`Drafts::apply` in `homotopy-model/src/proof/drafts.rs`): at most 5ms on
  the limits project, bounded by `MAX_COMPLETION_NODES`, but it runs inside the sidebar
  component, outside the app's job machinery. To fix: turn `Completion::solve` into an
  explicit-stack job and have the app own the drafts.
- Editing the signature with a recursive typecheck (making a generator less invertible, removing
  a symmetry, removing a generator): under 20ms on the examples, but it grows with the
  signature.
- `ExportImage` as PNG (single frame, or one per slice at 60 frames per height) is done by the
  GL view across animation frames, with no indicator or way to cancel.
- Background tabs: slices are scheduled with `setTimeout(0)`, which browsers throttle in hidden
  tabs, so a job there crawls.
