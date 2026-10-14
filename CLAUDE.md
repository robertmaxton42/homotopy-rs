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
  cospan), `AttachOptionsJob` (one step per generator, including its `infer_instances`) and
  `InferJob` (what the replacements chosen in the Instantiate panel determine, see below) live
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
- **Instantiate panel**: the drafts (the replacements being chosen for each schema, with what
  was inferred) are `State::drafts`, owned by the app like the rest of the state, and a choice is
  `Action::Draft(DraftEdit)`. `State::start` makes the choice (it is kept in red if it does not
  fit its type, see `doc/placeholders.md`) and returns a `Task::Infer` holding an `InferJob`,
  which `State::finish` takes in; the job is the search for what follows from the choices, as a
  stack of things to try (`Machine` in `proof/completion.rs`), one cut or one candidate a step,
  so a step is 0.1 to 5ms (debug build) whatever the whole search takes, and the page runs it a
  slice at a time and does not need the worker. Cancelling drops the job: the choices stay, nothing
  is inferred (`Draft::needs_inference`, the panel says so and offers *Infer again*). Typical cases
  take 1 to 130ms in the debug build (the commutative square of the tangent categories project:
  130ms in 179 steps for the inverse of `l;Tp = p;0` alone, under 5ms with one more choice), so
  the indicator rarely shows; the search is bounded (`MAX_COMPLETION_NODES`), after which only
  what matching forced is filled in, and the panel says so. Choosing is not changing the project:
  `finish` returns `Done(false)`, so no unload warning.
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

- **Attach options and the inference of instances** (`proof/attach_options.rs`, `proof/inference.rs`,
  `app/attach_inference.rs`). A click in the workspace is `AttachOptionsJob`, and what made it take
  half an hour on a project with schemas (the tangent categories project, a 5-dimensional workspace
  and 64 generators, 15 placeholders) was not the plain matching of cells but `Signature::infer_instances`
  for the dimension-6 schemas, which search the replacements of 5 to 9 placeholders each (numbers are
  the release build, native; the debug wasm build is about 10 to 20 times slower):
  - Generator 8 (`π_η̇ def`) took 61s and generator 12 (`∀η̇, ζ[η̇]η̇ = 1_G`) 57s, to find nothing; 16
    (`ζ[η̇→η̇']`, 9 placeholders) and the other schemas with that many did not finish in ten minutes. The
    busy pill shows the index of the last progress the worker posted (at most every 40ms), so "0" was
    the run of cheap generators before 8 and "16" the first of the schemas that has no end to it.
    Generator 0 (`Cat`, a 0-cell) is cheap: it is only tried with weak units.
  - The search was a recursion that solved the placeholders from the lowest dimension up: the four
    functors `Ḟ, Ġ, Ḟ', Ġ'` (15 candidates each) before the transformations between them, so the
    product of what each pair has before anything is pruned; it found the candidates for a placeholder
    by a breadth first search of composites of up to three cells from its source, once per pair
    (85ms for each of the 225 pairs of gen. 8); it checked each complete substitution by
    enumerating all embeddings of the needle in the haystack and only then filtering by the click
    (10ms each, 282 of them). The click also did not prune the matching of the plain cells, whose
    reflections (`2^n - 1` of them, a second for a 6-cell) were all made for every cell that had any
    symmetry or invertible generator.
  - What is done about it, none of it changing what is found (tests: `homotopy-model/tests/embeddings_through.rs`,
    `inference::tests`, and the option lists of the examples (11,259 clicks) and the fixture (11) equal
    to those of the code before): `Diagram::embeddings_through` (only the offsets the click can be in);
    a cell is not looked for if one of its generators is not in the haystack (and a reflection not made
    if the generators of neither end of the cell, with the orientation the reflection gives them,
    are there: `DiagramN::reflections_where`); the composites of cells and the candidates for a
    placeholder are kept for the job (`Cells`), as they only depend on where they start and on the
    replacements of the placeholders the type is in terms of; the placeholders are solved as soon as the
    placeholders their type is in terms of are (`InferSearch`, a machine that does a candidate list or a
    check in each step, so that the page can stay responsive and the job can stop between steps), and
    the instances are sorted back into the order of the old search. Plain matching and reflections
    of all 64 generators: 5.4s → 0.7s (click on a vertex), 6.1s → 2.8s (the whole diagram, `[]`); generators 8 and 12: 61s → 6.6s
    and 57s → 4.3s. The ones that do not finish are still far from finishing: the final placeholder
    (dimension 6, no 6-cells to choose from) costs 5ms per node for the 2000 nodes of the search.
  - The search is in three passes over the generators, the plain cells, the reflections and then the
    instances, with the options sorted back into the order of the generators, so that what is cheap is
    done before what has no bound, and a search that is stopped has found all the rest.
  - **Limits** (`InferenceLimits`, settings in the drawer, kept in this browser in `localStorage`
    `homotopy-inference-settings`: `app/attach_inference.js`; they go with the `SelectPoint` action and
    into `Work::AttachOptions`, so the worker uses what the page has). *Infer instances of schemas when
    attaching* (on); *Stop inferring after N s* (20, 0 for no limit; one schema gets a quarter of it,
    at least a second, so that one that has no end does not use the time of the rest); *Infer instances up
    to dimension N* (7, the dimension of the diagram being attached to; 12 for no limit). Plain cells and
    their reflections are always offered. A search that went over its time offers what it found and says,
    in a notice that stays until it is dismissed (`components/inference_notice.rs`), "Instance inference took
    too long and was skipped for this search (N s). You can turn it off in Settings.", which schemas it
    gave up on and which it did not get to. Two searches in a row that run out of time turn the inference
    off for the rest of the session (`infer_suspended`, not kept, ended by toggling the setting): a search
    that spends all its time on every click is worse than one with fewer options. A diagram above the
    dimension limit says so once per session. Esc cancels the whole search as any job.
  - Debug hooks: `__homotopy.dispatch` takes `{"SelectPoint": [point, weak_units]}` (limits from the
    settings) and `{"SelectPointWith": [point, weak_units, {"enabled":..,"budget_ms":..,"max_dimension":..}]}`;
    a point is a list of `{"Singular": n}`, `{"Regular": n}` or `{"Boundary": "Source"}`. Measured in
    the browser (debug wasm, worker, the fixture, a 3s limit, click `[R3, R2]`): 11 options in 31s, the
    notice naming the 2 schemas given up on and the 6 not tried (a step is not interrupted, so the
    time overshoots by a step); with inference off or the dimension limit at 4, 25-28s, almost all of
    it the plain cells and their reflections, which are 10 to 20 times slower in the debug build than the
    1s they take natively (another reason for the dimension limit to be a setting, and not a
    reason to expect the debug build to be quick).
  - Known gaps: the time is measured between steps, so a step that takes long (the first candidate
    list of a new source, up to 1.6s) can go over; in the page (no worker yet) the search is stepped
    in slices as the others; the searches that do not finish are cut, not made quick; the options a cut
    schema would have had are not there (use the setting to give it more time).

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
- `.claude/dev/highs-shim.js` and `flake.nix`'s `post.js` assign `self.Highs_*` rather than
  `window.Highs_*`, so that HiGHS is there in a worker too. The shim was tested here; the `flake.nix`
  edit (a textual replacement of `window.` by `self.` in `post.js`, which goes through the Closure
  compiler as an extern; `self` is a standard extern there) has since been built by CI's nix build,
  and the 3D view was confirmed to work on a nix build of it.
- Debug builds expose `window.__homotopy` (`app/debug_hooks.rs`): `dispatch(json)` for an action
  (`{"Proof": ...}`, `{"SelectPoint": [[...], false]}`, `{"Select": 0}`, `"Undo"`, ...),
  `importActions(bytes)`, `importProof(bytes)`, `cancel()`, `state()` (busy label, history
  length, a hash of the serialized project, crash log size), `profile(json)` (replays recorded
  actions, timing each), `profileDrafts()`, `fillFromInstance("[schema, placeholder, instance,
  dimension]")` (each generator as `[id, dimension]`: chooses, as *Use workspace* does, the
  replacement of that dimension that an instance was made with, for diagrams that are hard to put
  in the workspace, such as a cell whiskered by another), `fillWithGenerator("[schema,
  placeholder, generator]")` (the same with the diagram of a generator of the signature),
  `crash("panic"|"throw")` (the crash screen, see below), `autosave()` (see "Autosave") and
  `profileAutosave()` (times serialising the project on the page, and building the request for the
  worker with and without the signature, for choosing where it runs). Setting `window.__homotopy_step_delay_ms = 50` makes
  every step of a job spin that long (also in the worker: the value is sent with each job), to
  have something slow to test with in a headless browser (watch `state().busy`, the
  `.busy__pill` element, send `Escape`). Timings of jobs are logged: "<label> took Nms in K steps
  (longest Mms)" in the page, "<label> took Nms in the worker: Wms working, Pms to prepare and Tms
  to take in the answer (A KB there, B KB back)" for the worker.

## Firebase may not be there, and crashes must not lose work

- **Firebase is optional.** It is only used for cloud storage (the Account drawer, and opening
  `/p/<tag>` and `/u/<uid>/<id>` URLs). Its scripts come from gstatic.com (`index.html`, marked
  `data-firebase`), which an extension (NoScript, uBlock) or being offline blocks. `static/firebase.js`
  never throws: it records in `window.homotopyCloud` whether Firebase is `ready` or why not
  (`error`); analytics and performance monitoring are optional and cannot stop it. Nothing in the
  Rust code may assume it: every binding in `account.rs` is `catch` (a `Result`), the data layer
  returns `CloudResult`, and failures are toasts. Before, `getFirebase()` threw `ReferenceError`
  through wasm, which does not panic: Yew was left half way through its scheduler, so the app froze
  with no crash screen. **Never add a Firebase binding without `catch`** (a rejected promise from a
  non-`catch` async import is a panic).
- **The Firebase project only accepts some sites.** The configuration in `static/firebase.js` is
  upstream homotopy.io's own project, whose API key only accepts requests from homotopy.io (and the
  hosts in its list of authorised domains, which has `localhost` but not `127.0.0.1`). A copy
  served from anywhere else fails with `Firebase: Error (auth/requests-from-referer-<origin>-are-blocked.)`,
  which no code here can fix (the user who found it served the app at `http://127.0.0.1:5000`).
  So `firebase.js` has `homotopyCloudProbe()`, which the Account drawer calls once when it opens
  (not at load: it is a request to Google): it asks `identitytoolkit.googleapis.com/v1/projects?key=`
  what the SDK asks first, and is told either the API key is restricted
  (`API_KEY_HTTP_REFERRER_BLOCKED`) or the list of authorised domains, and if this page is not
  accepted Firebase is made unavailable with `homotopyCloud.rejected` set. The drawer then says,
  plainly and without *Retry* (it is not a matter of scripts), "Cloud storage only works on
  homotopy.io: this copy is served from <origin>, which the homotopy.io Firebase project doesn't
  accept. To use cloud storage here, configure your own Firebase project". Errors from the SDK
  that say the same (`requests-from-referer`, `unauthorized-domain`; `explainCloudError`, used by
  `CloudError::from(JsValue)`) are explained in the same way wherever they turn up (opening
  `/p/<tag>`, saving). A probe that cannot be answered (offline, slow, not understood) is not a
  rejection. To use another project: `localStorage.setItem("homotopy-firebase-config",
  JSON.stringify({apiKey, authDomain, projectId, storageBucket, appId, ...}))` and reload (the
  rules, indexes and functions in the repository, `firebase.json`, are what that project needs).
- **The emulators are opt-in.** Before, `firebase.js` pointed auth, firestore, storage and
  functions at emulators on `127.0.0.1` whenever `location.hostname === "localhost"`, so anyone
  running the app on localhost without the emulators had cloud storage that failed. Now they are
  used only if the page is opened with `?emulators` or `localStorage["homotopy-firebase-emulators"]`
  is `"1"`; and a connection failure while they are in use is reported as "the Firebase emulators
  could not be reached ... remove ?emulators".
- The Account drawer says that cloud storage is unavailable, with *Retry*. Retry
  (`window.homotopyCloudRetry`) inserts the missing scripts again (in order, `async = false`) and
  initialises Firebase, so that someone who allows gstatic.com does not have to reload and lose
  their work. Test it in headless Chromium by aborting `gstatic.com` with `page.route`, then routing
  it to stub scripts that define `window.firebase` and `firebaseui` for the Retry that succeeds.
- **Crash screen** (`panic.rs`, `index.html`). Before, the crash dump only had the last imported
  file (the action log is disabled: `push_action` records nothing but imports), so everything done
  since was lost. Now `App::updated` calls `panic::remember`, which keeps a clone of the
  `ProofState` (shared with the history: pointers, no serialisation) as the last good project. The
  panic hook (and `recover_after_error`, which `index.html` calls for an uncaught JavaScript
  error that went through the wasm, as that leaves the app stuck without a panic) serialises it
  *then*, once, and hands the bytes and a short report (panic message, place, version, the last 12
  actions started, cut short) to `offer_recovery`, plain JavaScript that makes them two downloads
  in the crash message. Nothing there calls back into wasm, which cannot be relied on after a
  trap. What is lost is the action that crashed. Debug builds: `__homotopy.crash("panic")` and
  `__homotopy.crash("throw")` (call it from a `setTimeout` so that the error is not caught by the
  caller).
- **Autosave** (`homotopy-web/src/autosave.rs`, `autosave/store.js`, `homotopy_model::autosave`,
  `Work::Serialize` in `homotopy_model::remote`). The project is backed up in the browser a few
  seconds after the last change, so that a crash, a closed tab or a browser that gives up does not
  lose what was done since the last export, and the next visit offers to restore it. It is derived
  work, in the class of layouts: nobody asked for it and there is nothing to cancel or refuse, so it
  is held to the first two points of the policy (off the main thread, and it says that it is
  working: the small *Saving a backup...* note in the corner, `components/autosave_note.rs`, fades
  in after 150ms like the busy pill) and does not use the busy pill.
  - **Cost** (why it is in a worker). `serialize::serialize` of the 5.4MB tangent categories
    project: 0.84-0.93s in the debug build, 100-150ms optimised; the 2.5MB one 0.35s and 40ms. Of
    the debug time ~60% is packing diagrams into the `Store`, ~25% encoding it. That is past
    ~100ms for a project of that size even optimised, so it cannot be done on the page in one go.
    Doing it in the job worker was considered and not done: a job in flight would make the save
    wait or the save a job, `Remote` assumes one outstanding request, and cancelling a job ends the
    worker with its replica of the signature. So the save has a worker of its own (the same
    module as the others: `worker::Client` with its own `Remote`), started when the first save is
    due and kept: a third copy of the wasm module in memory (35MB in the debug build), the price of
    never making a job wait. It is a `Work::Serialize` request: the worker unpacks the project as
    for any job, makes the file with `serialize::serialize` and answers with it appended to the
    message (`trailer`, as a `Vec<u8>` in msgpack would be an array of integers); the file is
    byte-identical to one made on the page (`homotopy-model/tests/remote.rs`).
  - **The signature**. The worker holds the signature between saves, so a request after the first
    is the workspace and metadata only: 1-3ms on the page optimised (10-35ms debug), against 100ms
    (0.8-0.94s) with the signature. The first save, and the first after the signature has changed,
    need it sent, which is as expensive as serialising, so it is packed a little at a time on the
    page (`autosave::Packer`: a diagram's source, then the rewrites of its cospans one at a time,
    then the diagram) in slices of 10ms (`SLICE`), and what each slice packed goes ahead to the
    worker as `Store::take_packed` pieces (`Remote::preload`, `Executor::preload`; merged into the
    next request with `Store::extend`), encoded in the next slice so that the page can respond in
    between. What is left for the request is the tree of the signature and the workspace
    (20KB). Measured with the 5.4MB project open in the 2D view. Debug build: the page's longest
    slice 35-40ms in 73 slices (~850ms in all; 3.5-3.8s from the wait's end to the backup being
    written), no rAF gap above 67ms (that is the metadata edit that started it), longest long task
    56ms; a second save, signature held: 3 slices, longest 15ms, 14-25ms in all, 0.9s end to end,
    no gap. Optimised build with debug assertions: 21 slices, ~135ms in all, longest 14ms, 0.5s end
    to end; held: 3 slices, 7ms in all, longest 4ms, 0.16-0.2s end to end; no rAF gap or long task
    (>50ms) in either. Without a worker (it cannot start, is given up on after three
    crashes, or answered with nonsense) the page packs the same way and makes the file in **one
    step that cannot be split up** (`serialize_with` over the packed store: encoding it): 160ms for
    the 2.5MB project in the debug build. That is a known gap, and the file is the same.
  - **When** (`homotopy_model::autosave::Plan`, a state machine that knows no time but what it is
    told and compares versions only for equality; tests there). The version is the project's place
    in the history (`History::position`: a node of the tree, and undoing gives back the number it
    had), so noticing that the project changed costs a comparison per message. A save is due 5s
    after the last change, or 60s after the project became unsaved if it keeps changing, or when
    the tab is hidden (`visibilitychange`, so that leaving does not lose the last few seconds);
    never while a job runs (it looks again after a second); never for a version that is *clean*:
    the one the page opened with, those that importing a file made (`History::at_import`, which
    does not clone the file), those exported, and the one last saved. So nothing is saved for a
    project nobody has changed, and going back by undo to what was imported does not save over a
    backup of later work. A save that fails is tried again after 10s, doubling to 5 minutes, and
    after a failure the store is probed (`store::probe`) before any serialising, so a store that
    is not there costs nothing; after three failures in a row one toast says that work is not being
    backed up. A save that the worker did not do (crash, nonsense) is not a failure of the store:
    it is tried again at once, the worker is restarted, and after three crashes it is given up on
    and the page makes the file (nonsense: the page makes that one).
  - **Where** (IndexedDB, `autosave/store.js`; everything in `try`, every binding `catch`, as for
    Firebase). Two object stores, `meta` (session, time, title, size, clean) and `data` (the
    bytes), so that listing does not read the files; a save is one transaction, so a backup is
    replaced whole or not at all, and a tab that is closed while saving leaves the previous one.
    *Multiple tabs*: each tab has a random session id and keeps one backup under it, replaced as it
    goes; each holds a Web Lock named for its session for as long as it is open
    (`navigator.locks`), which is how a tab tells the backups of tabs that are still open (never
    offered, never deleted by another) from those of tabs that are gone. After a save, the backups
    of gone tabs are cut to the two newest: the latest and the previous, so that work from before
    this tab was opened is not lost to the first save of this one. Without Web Locks every other
    session counts as gone. Last writer wins nowhere: there is one key per tab.
  - **Restoring.** When the page is opened at `/` (not a project's address) a backup of a gone tab
    that has not been exported is offered by a small non-modal notice at the bottom
    (`components/restore_notice.rs`): "Restore unsaved work from <time> (<title>)?" with *Restore*
    and *Discard*. It is looked for a second after the page starts and nothing waits for it.
    *Restore* reads the file and imports it as any other import (`ImportProof`: a job, behind the
    busy pill, in the worker if it is 64KB or more), so it can be undone; nothing is ever restored
    unasked. *Discard* deletes the backups of gone tabs. The backup is kept after a restore (it is
    pruned by the next saves), so that closing the tab straight away loses nothing.
  - **Export.** After *Export* the project is clean at that version, and the tab's backup is marked
    clean (`store.markClean`) so that it is not offered again: the download cannot be known to
    have gone through, so it is kept, not deleted, and the next change saves a new one.
  - **Setting.** *Back up my work in this browser* in the settings drawer, on by default. The
    settings are not kept between visits (they are thread-locals, `declare_settings!`), except this
    one, which is kept in `localStorage` when it is off, as a switch that turns itself on again is
    not one. Turning it off deletes all the backups (the drawer says so) and stops the worker.
    The drawer shows when the last backup was made, or why they do not work.
  - **Size.** The optimised build with debug assertions went from 9.92MB to 10.12MB (2.2MB
    gzipped), with `Work::Serialize`, the packer, the store and the plan.
  - **Testing.** `window.__homotopy_autosave_idle_ms` (debug builds) sets the wait (retries and
    the longest wait follow), `window.__homotopy_autosave_worker` (`"off"`, `"broken"`, `"crash"`,
    `"garble"`) does for the saving worker what `__homotopy_worker` does for the other, and
    `__homotopy.autosave()` is a JSON summary (stage, saves, failures, last backup, session).
    Native tests: `Plan` (`homotopy_model::autosave`), and the worker's file against the page's, with
    the signature sent in pieces, held or not preloaded (`homotopy-model/tests/remote.rs`). In the
    browser (Playwright): a project survives reload with an identical hash; nothing is saved for an
    untouched project or an import; nothing while a job runs; turning it off deletes and is
    remembered; no IndexedDB (`undefined`, throwing, failing to open): the app is unaffected and
    says so once; a second tab does not offer, or touch, the first tab's backup, and a third offers
    the newest once both are closed; the cap keeps the two newest of gone tabs.

## Known gaps against the policy above

Done (behind the indicator, cancellable, input refused meanwhile): attach options for a click in
the workspace (`SelectPoint`), `MergeExchanges` and the automatic merging after a homotopy (one
cancellable unit with the homotopy), replaying actions (`ImportActions`), multi-slice and other
text exports, and the inference in the Instantiate panel (a job of small steps in the page). Done in the Web Worker, so that the page stays responsive however long they take
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
- Attach options without schemas (and with the inference of instances off), and everything while the worker is not ready (it is started
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
- Autosave without a worker makes the file in one step on the page (160ms for the 2.5MB project in
  the debug build; see "Autosave"); with the worker the page's longest slice is 35-40ms, and
  the copies of the file (it crosses the worker boundary, into wasm and out to IndexedDB as a
  `Uint8Array`: three copies of 5MB) are up to ~50ms in the debug build, none measurable optimised. In a background
  tab, in-page slices (the packing of a signature the worker does not hold yet) are throttled like
  the other jobs' (`setTimeout(0)`), so the first save there crawls; the worker's part does not.
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
