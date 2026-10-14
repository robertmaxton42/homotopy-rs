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

What the page *derives from the project to show it* (the layout of the diagram on screen) is not
an operation in this sense: nobody asked for it, there is nothing to cancel or to refuse (the
project is not being changed), and a newer diagram simply replaces it. That is held to the first
two points (off the main thread, and it says that it is working), but it does not use the busy
indicator: see "Laying out diagrams" below.

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
- **Size**: the wire format and its MessagePack code added 1.2MB to the release `.wasm` (6.2MB to
  7.4MB; 1.54MB to 1.75MB gzipped). Most of that is `rmp_serde` instantiated for the proof types;
  if it matters, the request could carry less through the `Store` (the actions are sent with
  `serde`, diagrams and all).
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

- **Laying out diagrams** (`homotopy-web/src/layout.rs`, `homotopy_model::remote::layout`,
  `static/worker.js?layout`). The layout of a diagram (`homotopy_core::layout::Layout`, a linear
  program that HiGHS solves) and, for the 3D view, the `CubicalGeometry` made from it (mesh,
  layout, vertices and cells: all of what is plain data and takes longest) are made by a *second*
  Web Worker, the same wasm module with HiGHS loaded. It is a worker of its own so that cancelling
  a job (which ends the job worker) does not lose a layout in the making (tested: Esc during a
  replay while a layout is on its way; the layout still arrives from the worker), and a layout
  does not queue behind a job that takes seconds. Both workers are started 500ms after the app and
  are ready about 1s later. `worker.js` takes the role from its query (`?layout` loads
  `highs.js`, awaiting its start, and says `ready` only after that); HiGHS is installed as
  `self.Highs_*` (`flake.nix`'s `post.js`, `.claude/dev/highs-shim.js`), which is `window.Highs_*`
  on the page and exists in a worker too; the `highs` crate's imports are bare globals, so they
  resolve in both. The wire is the diagram through a `Store` and a request for a layout of `N`
  dimensions or for geometry (with or without time as a dimension); the answer is flat
  little-endian numbers (a layout is the coordinates and positions of points, geometry its
  vertices, cells and curves), which are the same as what the page makes itself (tests in
  `homotopy-model/tests/layout_remote.rs`: on a thread of its own, on the corpus and all the
  diagrams of its signatures, in every dimension, equal to the layout and geometry made in place;
  and in the browser: the same SVG elements and byte-identical screenshots of the 2D and 3D views).
- **Asking for a layout**: views `ask` for the layout (`layout::ask`, `ask_geometry`) of the
  diagram they are to show. They get it at once if the diagram is small (`layout::weight`, a
  count of its distinct parts that stops at a limit so that it costs the same whatever the size:
  300 for layouts, 100 for geometry, see below), if one was made recently (a cache of the last 8
  of large diagrams: going back to a slice is instant) or if there is no worker ready (it is
  made on the page, as before). Otherwise they get a ticket and the answer through a callback
  later, keep showing what they showed, and `cancel` the ticket if the diagram is replaced or
  they go. Only one request is with the worker at a time, so that the ones that were cancelled
  are dropped before they are sent (rapid slice switching on large diagrams: 8 steps, 7 cancelled,
  only the layouts that were still wanted done), and an answer for a cancelled ticket or an older
  diagram is ignored. If the worker crashes or answers nonsense, the layout is made on the page
  (and the worker restarted; three failures in a row without an answer and it is given up on,
  with a toast), as for jobs.
- **While a layout is on its way** `DiagramSvg` shows the diagram it prepared before (nothing if
  it is new, a one pixel `svg`), dimmed and with a "Laying out…" note (`components/layout_note.rs`,
  bottom centre), both fading in after 150ms with a CSS animation like the busy pill, so that
  layouts that are quick cause no flicker. Pointer input on the diagram is ignored (clicks, drags
  into homotopies, hover tooltips, highlights of attach options and slices: points of the old
  diagram are not points of the new one) until the layout is there; the sidebar and everything
  else works, and any change of the project just asks again. `DiagramGl` keeps its renderer in
  the same way, dimmed, until the geometry arrives, and then builds the next renderer. The
  slice buttons, which are as high as the diagram, are told to look again when it is drawn after
  waiting (`on_laid_out`). A layout is not a job, so the busy indicator is not used, Esc does not
  stop it and nothing is refused meanwhile.
- **Thresholds** (debug build, which is about ten times slower than the optimised one): on the
  examples a layout of a diagram of weight up to 300 takes 1 to 2ms in the middle and 10ms at
  most, the page's share of sending it to the worker (the request, and taking in the answer) is
  about 8ms for the 31KB request of the largest, so that is where they meet. Geometry (mesh,
  layout, cells, curves) takes 10 to 50ms in the debug build for diagrams that weigh up to 300,
  with no clear relation to the weight, so the limit for it is lower (100).

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
- The layout worker is `worker.js?layout`: wait ~3s after load for it too (the log line "The
  layout worker is ready"), or layouts of large diagrams are made on the page. In a debug build
  `window.__homotopy_layout_worker` (`"off"`, `"broken"`, `"crash"`, `"garble"`, as for the job
  worker, but for this one only), `__homotopy_layout_delay_ms` (each layout in the worker takes
  at least that long), `__homotopy_layout_weight` (the weight up to which layouts are made on the
  page: `0` sends everything to the worker, which is how to test with small diagrams) and
  `__homotopy.layouts()` (counts: made by the worker, small, made on the page for lack of a
  worker, cached, cancelled, pending) are for testing. `__homotopy.profileLayouts()` times the
  layout and geometry of every diagram of the project and its slices next to their weight, for
  choosing the thresholds.
- `.claude/dev/highs-shim.js` and `flake.nix`'s `post.js` now assign `self.Highs_*` instead of
  `window.Highs_*`. The shim was tested; **the `flake.nix` edit was made by analogy and has not
  been built** (no nix here): it is a textual replacement of `window.` by `self.` in `post.js`,
  which goes through the Closure compiler as an extern; `self` is a standard extern there.
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
- Diagram layout is done in the layout worker (see "Laying out diagrams" above), which took the
  3D view of the 184KB project from 312ms (geometry) on the page to 8.7ms to send the request and
  3.7ms to take in the answer (debug build; 113ms to 1.7ms in the optimised one with debug
  assertions). What it leaves on the page, per view:
  - `DiagramSvg`: building the rest of `PreparedDiagram` from the layout (`make_complex`,
    `Depths`, `Projection`, the graphic elements and action regions), which is not plain data: 4 to
    16ms for slices, 45 to 52ms (debug build) for the 2D view of the whole of that project's
    4-dimensional diagram, plus Yew building the SVG. (Its layout was only 15 to 45ms, and 2D
    layouts of the examples never took more than 8ms in the debug build, so the gain for 2D
    views is small; it is the 3D ones that were heavy.) Diagrams of weight up to 300 are laid
    out where they are asked for: about 1 to 2ms in the optimised build, 10ms at most in the debug.
  - `DiagramGl`: making the renderer from the geometry, 95 to 130ms in the debug build and 47ms
    in the optimised one, of which subdivision (48 and 3ms), inflating the surfaces and uploading
    buffers (43 and 16ms) and the GL objects that are made again with every renderer (shaders,
    framebuffers) could still move: the first two depend on the settings and the signature's
    styles, and the buffers are made on the page. The 3D view in headless Chromium with software
    GL takes ~150ms (debug) to 200ms (optimised) per frame, so rAF gaps cannot go under that
    there: the longest gap after importing that project went from 543, 572, 493ms to 316, 347,
    270ms; what is left is the frames themselves and the renderer.
  - A layout that is in the worker when it is no longer wanted is not interrupted (a layout worker
    that is ended costs about a second to start again, mostly loading HiGHS): the next one waits for it, 0.3s
    to 3s with `__homotopy_layout_delay_ms` set, and for the examples at most the time of a large
    geometry (about 0.5s in the debug build). It is only the next layouts that are late, and the views show what they showed.
  - Without a ready worker (until ~1.5s after the page loads, ~1s after a crash) layouts are made
    on the page as before; browsers that cannot start module workers get the fallback and a toast.
  - Two workers are two copies of the wasm module in memory (35MB in the debug build) and the
    second one HiGHS too; the optimised build is 0.7MB bigger (8.9 to 9.6MB with debug assertions,
    92KB more gzipped: the wire format, a second path through `CubicalGeometry`).
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
