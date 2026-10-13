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
  in `homotopy-model`; `ExportJob` in `homotopy-web`. A job holds no references into the app's
  state, only its own copy of its input, so it can run in the page a slice at a time or in the
  Web Worker (below) without changing its callers.
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
  cancels. In the page, a step that blocks it delays the `Esc` until it is over (the job is then
  cancelled instead of committed); in the worker, `Esc` is seen at once.
- **Refusal**: while a job runs, `Action::allowed_while_busy` is false for everything but
  highlighting, remote metadata and help: this includes undo/redo (keys and mouse buttons), as
  the job works on the history as it was. Refused input shakes the pill (and shows it at once).
  Pure view changes that do not go through `dispatch` (pan/zoom, opening drawers, settings)
  still work.
- **What is heavy**: `Action::heavy_label` / `proof::Action::work_label` name the actions that
  can take long, from timings of the debug build (see the doc comment on `work_label`). Those
  whose next step cannot be split up (a homotopy, a theorem, importing a project) go to the
  worker; when that is not ready the app puts the indicator on the page and waits for the
  browser to paint before blocking. Cheap
  actions (anything typed or stepped through quickly: renames, slices, ...) never get a label,
  because input that arrives while a job runs is refused.

- **The Web Worker** (`homotopy-web/src/worker.rs`, `static/worker.js`, `homotopy_model::remote`)
  runs the jobs that block the page for as long as a single step takes. It is the *same wasm
  module* as the page: `worker.js` imports `homotopy_web.js`, and the start function (`main_js`)
  sees there is no window and calls `worker::init` instead of rendering the app. So there is no
  second crate and no second `wasm-bindgen` run (`Makefile.toml`, `build-dist.sh` and
  `flake.nix` copy `static/*` and so ship `worker.js` as they are), and the browser fetches
  the `.wasm` once. There are no wasm threads (stable toolchain, no COOP/COEP headers), and
  diagrams are hash-consed in thread-locals, so a job crosses as bytes: `remote::Remote::request`
  (page) turns the proof state and the work into a message, `remote::Executor::run` (worker)
  rebuilds the proof, runs the same `ActionsJob`/`AttachOptionsJob` to the end and answers with
  bytes, and `Remote::response` turns the answer into the `ActionsOutcome`/options that
  `State::finish` commits. The wire format is faithful (the signature keeps the numbering of its
  tree, which actions refer to; it is *not* the file format of `serialize`), diagrams go once each
  through a `Store`, and the executor remembers the last signature it saw or made, so that a
  request leaves it out when it is `identical` (down to node numbering) to the proof's. Tests:
  `homotopy-model/tests/remote.rs` runs the executor on a thread of its own (fresh thread-locals,
  bytes only) and compares with the in-thread job on the whole example corpus;
  `homotopy-web/tests/tasks.rs` does the same through `Task::offer`/`output_of`.
- **What goes to the worker**: `Task::offer` decides. Actions whose next step is heavy
  (`work_label`: homotopies, `Squash`, `Theorem`, `Instantiate` and attaching an instance,
  `ImportProof` of 64KB or more) and every `ImportActions` replay; `AttachOptionsJob` when the
  signature has placeholders (schemas, where `infer_instances` is what takes long). Everything
  else, and everything while the worker is not ready (it is started 500ms after the app, and is
  ready about 1s later), stays in the page and runs in slices as before. Exports stay in the
  page (they need the DOM and the signature's styles, and are chunked per slice).
- **Cancel, refusal and progress with the worker**: cancelling is `Worker.terminate()` and a new
  worker started at once (ready again in ~180ms in the debug build); nothing else is needed as
  jobs only commit in `State::finish`. The worker posts the label and progress before each step
  (at most every 40ms), which `app.rs` forwards to the busy pill. Refusal while busy is the same
  as for in-page jobs. The page never gets stuck on the worker: a *panic* (the worker posts it
  from its panic hook), an uncaught error or a script that does not load ends the worker,
  restarts it, shows a toast and leaves the project unchanged (three crashes in a row without a
  finished job, and workers are given up on for the session); a *failure without a crash* (a
  garbled message) does the job in the page instead.
- **Cost on the page**: the page still has to build the request (`Remote::request`) and take in
  the answer (`Remote::response`, which unpacks diagrams into the page's hash-consing tables), and
  these are not chunked: 1-15ms for a homotopy on a workspace of 20-30KB, up to 36ms for a
  theorem with a 180KB answer (debug build), plus about 12ms to send a 16-generator signature
  the first time. Importing a project is mostly turning bytes into diagrams, which has to be
  done again on the page for the result, so it only saves about half of the page's time
  (below 64KB it is done in the page).

- **What is cheap by construction**: things that run on every render or pointer move are bounded
  instead of made into jobs. `Signature::describe` and `describe_point` (names for diagrams in
  the Instantiate panel and the attach options, and for the cell under the pointer; see
  `doc/labels.md`) look at no more than `MAX_STEPS` diagrams whatever the size, and cut long
  composites and types short. The hover tooltip (`app/tooltip.rs`) is told what to show through
  `tooltip::show/move_to/hide` like the busy indicator, and `DiagramSvg` only works out a new label
  when the pointer changes what it is over: moving over one part of the diagram renders nothing.

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
- The worker needs no build step of its own (see above), but it is only used once ready: in a
  headless browser wait ~3s after load (or for the log line "The worker is ready") before
  starting a job that should go to it, or it runs in the page. `window.__homotopy_worker` set
  before the app starts (e.g. with Playwright's `addInitScript`) forces the other cases: `"off"`
  does without a worker, `"broken"` points it at a script that does not exist (falls back after
  three failed starts), `"crash"` or `"garble"` (can be set at any time) make the next jobs panic in the
  worker or be answered with nonsense (which falls back to the page).
- Debug builds expose `window.__homotopy` (`app/debug_hooks.rs`): `dispatch(json)` for an action
  (`{"Proof": ...}`, `{"SelectPoint": [[...], false]}`, `{"Select": 0}`, `"Undo"`, ...),
  `importActions(bytes)`, `importProof(bytes)`, `cancel()`, `state()` (busy label, history
  length, a hash of the serialized project, crash log size), `profile(json)` (replays recorded
  actions, timing each), `profileDrafts()`. Setting `window.__homotopy_step_delay_ms = 50` makes
  every step of a job spin that long (also in the worker: the value is sent with each job), to
  have something slow to test with in a headless browser (watch `state().busy`, the
  `.busy__pill` element, send `Escape`). Timings of jobs are logged: "<label> took Nms in K steps
  (longest Mms)" in the page, "<label> took Nms in the worker: Wms working, Pms to prepare and Tms
  to take in the answer (A KB there, B KB back)" for the worker.

## Known gaps against the policy above

Done (behind the indicator, cancellable, input refused meanwhile): attach options for a click in
the workspace (`SelectPoint`), `MergeExchanges` and the automatic merging after a homotopy (one
cancellable unit with the homotopy), replaying actions (`ImportActions`), multi-slice and other
text exports. Done in the Web Worker, so that the page stays responsive however long they take
and `Esc` takes effect at once (the page's longest gap while a 1.5s homotopy ran was 40ms, against
1.4s in the page; cancelling took 33ms): a single homotopy (`Expand`/`Contract`, with its
typecheck; 10ms to 1.5s in the debug build), `Theorem` (20-260ms), `Squash` (10-230ms),
`Instantiate` and attaching an instance (10-35ms), replaying actions, importing a project of 64KB
or more, and the attach options in a project with schemas (one generator's `infer_instances` took
up to 30ms of the page).

What is left on the page (numbers are the debug build, then the optimised build with debug
assertions, on `homotopy-model/tests/examples` and a project with limits as schemas):

- The page's share of a worker job: building the request and unpacking the answer into the
  page's hash-consing tables, which is not chunked. 5-12ms and 7-36ms for a homotopy or theorem
  on a workspace of 20-30KB with an answer of up to 180KB (0.5-1ms and 1-4.5ms optimised), plus
  about 12ms (debug) the first time the signature of a 16-generator project is sent (it is
  not sent again while it is the same). Tiny jobs cost a little more on the page than they would
  have done in place (about 5ms in the debug build); there is no threshold, because guessing a
  heavy job to be light is far worse than the reverse.
- Importing a project under 64KB (50ms for 38KB in the debug build, 24ms optimised), and above
  that the page still has to turn the answer into diagrams (28ms of the 56ms in the debug build
  for 38KB): there is no way around making the diagrams on the page. Chunking
  `Remote::response` by generator would make that interruptible. Replaying actions parses the
  recorded JSON in `State::start` (27ms for the 1MB file in the debug build).
- Attach options without schemas, and everything while the worker is not ready (it is started
  500ms after the app and ready ~1s later; ~180ms after a cancel or crash): done in the page
  in slices as before (the whole search is 1-5ms typically).
- Laying out the diagram on render (`PreparedDiagram::new`, HiGHS; GL meshes): 5-70ms on the
  examples in the debug build (45ms optimised for a freshly imported project, 2ms after), not
  behind an indicator and not cancellable. HiGHS is a set of `window.Highs_*` globals set by
  `highs.js` (`flake.nix`'s `post.js`, `.claude/dev/highs-shim.js`), so it is not available in
  the worker: see the plan below.
- `Signature::infer_replacements`, called when a replacement is chosen or cleared in the
  Instantiate panel (`Drafts::apply` in `homotopy-model/src/proof/drafts.rs`): at most 5ms on
  the limits project, bounded by `MAX_COMPLETION_NODES`, but it runs inside the sidebar
  component, outside the app's job machinery. To fix: turn `Completion::solve` into an
  explicit-stack job and have the app own the drafts.
- Editing the signature with a recursive typecheck (making a generator less invertible, removing
  a symmetry, removing a generator): under 20ms on the examples, but it grows with the
  signature. These are `EditSignature` actions, so they can be sent to the worker the same way
  (add them to `work_label` and they go through `Task::offer`).
- `ExportImage` as PNG (single frame, or one per slice at 60 frames per height) is done by the
  GL view across animation frames, with no indicator or way to cancel.
- Background tabs: in-page slices are scheduled with `setTimeout(0)`, which browsers throttle in
  hidden tabs, so a job there crawls (a job in the worker does not).
- Browsers without module workers (Firefox before 114, Safari before 15) fall back to the page
  after three failed starts, with a toast.

Plan for laying out in the worker (the remaining big blocker): make HiGHS available as
`self.Highs_*` rather than `window.Highs_*` (in `post.js`, which goes through the Closure
compiler: `self` is safe there, `globalThis` may not be), have `worker.js` also load `highs.js`
before saying it is ready, and add a `Work::Layout` to `homotopy_model::remote` whose request is
a diagram (through a `Store`) and whose answer is the `Layout<N>` map (plain data). `DiagramSvg`
and the GL scene then ask for the layout, keep showing the previous diagram with a "Laying
out…" note meanwhile, drop answers that are not for the diagram any more (as slices drop by
job id), and build the rest of `PreparedDiagram` (a few ms) from the answer. Use a second
worker for layout so that cancelling an action does not lose it, and cancelling a layout (a
newer diagram) does not lose a job.
