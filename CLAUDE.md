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
   exactly as it was before the operation started. (A search for attachments lists the options it
   has found as it goes, and cancelling it keeps them listed: listing options is not changing the
   project. See "The list fills in as the search goes" under the attach options drawer.)

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
  fit its type, or is not as invertible or dualisable as the placeholder, see
  `doc/placeholders.md`: `Signature::check_invertibility`, which the kernel, `Matcher::bind` and
  the candidate pools all use) and returns a `Task::Infer` holding an `InferJob`,
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
- **Refusal**: while a job runs, `Action::allowed_while_busy` (an exhaustive `match`: whoever adds an
  action decides) refuses what updates the history, and what is expensive in itself, and allows the rest.
  Refused: every `proof::Action` but the view-only signature edits below, undo/redo (keys and mouse
  buttons), `ImportActions`, `Select`, `SelectPoint`/`SelectCell`/`SearchEverything`/`AttachGenerator`, `Draft` (it runs inference),
  `ExportImage` and `ExportProof` (it serialises the project on the page, and is the project as it was
  before the job). Allowed: highlights, `Arm`, `ClearSelections`, `Merge` (listing the options),
  `ExportActions`, remote metadata, help, and **opening and closing folders and the instances of a
  generator** (`SignatureEdit::is_view_only`: `ToggleFolder`, `ToggleInstances`). Those used to be
  history steps (each fold was an undo step, and refused while busy); now `State::start` applies them
  in place to the current snapshot (`History::amend`) and returns `Done(false)`: no history step, no
  undo of them, no unload warning, and the position (autosave's version) does not change. A job works
  on a copy, so a toggle made while a long `ActionsJob` runs is lost when that commits (the open state
  of that moment is the copy's): a known gap, harmless. Refused input shakes the pill (and shows it
  at once); allowed input neither shakes it nor cancels the job. Two actions are allowed that *do* end the
  running job, both only for a search for attachments whose options are on show (`Listing::Searching`):
  `PickStreamed(option)` (the app ends the job, then attaches the option as `Attach` does; refused if the
  running job is not an attach search) and `StopSearch` (what `Esc` does; the drawer's Stop button). Pure view changes that do not go
  through `dispatch` (pan/zoom, opening drawers, settings, opening a group of options or a list in a
  notice: component state) still work. Test: `what_may_be_done_while_busy_does_not_touch_the_project`
  in `homotopy-web/tests/tasks.rs`.
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
    attaching* (on); *Give up on a generator after N s* (`generator_ms`, 5, 0 for no limit); *Infer instances
    up to dimension N* (7, the dimension of the diagram being attached to; 12 for no limit). Plain cells and
    their reflections are always offered (within the time for their generator, below).
  - **The time limit is per generator, not for the search** (`InferenceLimits::generator_ms`,
    `DEFAULT_GENERATOR_MS` = 5 s; user decision: "it's not 'wasted time'; it's annoying primarily when the
    program gets blocked on tasks I know aren't relevant to begin with"). One setting covers everything that
    can be spent on one generator: matching it, its inverse and the cells raised from it (weak units, bubbles),
    its reflections, and for a schema inferring its instances. `AttachOptionsJob::by_generator` is the time
    spent on each generator (by index) across the three passes; a step is run with
    `deadline::until(start + what the generator has left)` (`plain_step`, `infer`), and when the clock runs out
    in the middle of one the generator is *given up on*: what it found stays (valid, among what the whole search
    finds, in the same order, `homotopy-model/tests/plain_limit.rs`), it has no time left for the later passes,
    and the search goes on to the next generator. The next generator has all of its own time, so there is no
    total to run out of and no generator is "not tried" for lack of time (`not_tried` only comes from a search
    that was ended from outside, below). There is no total limit by design (Esc ends the search and the project
    is as it was); an optional total was considered and dropped as one more setting that would bring back the
    skipping the user does not want. A search for one generator (`only`) has the same time, not more: raise the
    setting to wait longer for it. One setting, not two (cells and schemas), because the user thinks of "a
    generator" and a schema's plain matching is cheap: it is the inference that uses the time, and that gets what
    matching left.
    - **Why 5 s.** The costliest single generator's plain matching natively is 2.5 s (the reflections of `lc=l`,
      ill typed); the schemas that finish with nothing to find take 4 to 7 s natively (generators 8 and 12 of the
      fixture) and the ones that do not finish never would; and 5 s is what a schema got in practice under the old
      20 s total (a quarter of it each when there were several). The debug wasm build is 10 to 20 times slower
      and gives up on more.
    - **Reports and notices.** `InferenceReport { cut_short (schemas given up on), not_tried, inferred (schemas
      gone through to the end), targeted (a search for one generator), spent_ms, budget_ms (the time for a
      generator), forced, plain: PlainReport { cut_short, not_tried, spent_ms, budget_ms } }`. Notices
      (`app/attach_inference.rs`, `components/inference_notice.rs`, stay until dismissed): "Gave up on N
      generators after 5 s each (Settings, "Give up on a generator after"): A, B. What was found is valid but may
      be missing some. To look for one generator, drag it onto the diagram or pick it with the pointing hand;
      choosing a sheet narrows the search to it." for the plain passes, and "Instance inference gave up on N
      schemas after 5 s each: X, Y. What it had found for them is offered. You can give generators more time, or
      turn the inference off, in Settings." for schemas. A diagram above the dimension limit says so once per
      session.
    - **Suspending** (`infer_suspended`, not kept, ended by toggling the setting): under per-generator limits a
      search with a schema that never finishes always "times out", so the old rule (two searches that ran out of
      time) would fire on most clicks. Now two searches in a row that gave up on *every* schema they looked at
      (`gave_up_on_all`: `inferred == 0` and `cut_short` not empty, not `targeted`) suspend it: then the time
      bought nothing and would be spent again on every click. A search in which some schema finished, or one for
      a generator asked for by name, does not count; any other search resets the count.
    - **Stored settings.** The kept JSON has `version: 2` and `generator_seconds`. The old keys
      (`budget_seconds`, `plain_budget_seconds`, totals) are read but not written and not carried over (a total
      means nothing per generator): both 0 (no limit) stays 0, anything else gets the default. `enabled`,
      `max_dimension` and `narrow_to_sheet` are kept (`Kept::generator_seconds`, tested).
  - **The limit is hard** (`homotopy_common::deadline`, `Job::checkpoint`, `Job::step_limit`, `Message::Watchdog`
    in `app.rs`). The time was only looked at between steps, and one step (a candidate list, a check
    of a substitution) could take a long time: in the release build, native, a 1.5 s limit gave 2.46 s
    spent (+955ms), 700ms gave 975ms; in the debug wasm the steps are 10 to 30 times longer. Now in
    two layers:
    - *In the search*, whichever thread it is on (the page when the worker is not ready, or the
      worker): the plain passes and `AttachOptionsJob::infer` set a limit for the step with
      `deadline::until(start + what the generator has left)`, and the code that loops over what can be large
      looks at it with `deadline::expired()` (a thread-local read when no limit is set, the clock every 4th
      call otherwise; sticky until the limit is dropped): `DiagramN::embeddings` (per regular slice),
      `embeddings_through` (per offset), `Composites::level` (per composite and per embedding), between the
      variants and reflection candidates, and in `typecheck` (`TypeError::TimedOut`; a timed-out typecheck says the
      reflection is not known to be well typed, so it is not offered; the one stretch that took long was 2.5 s
      natively for one reflection of `lc=l`, up to `2^n` collapses of 180 ms each, checked between them and
      between the two collapses of `typecheck_worker`, which are not interrupted).
      Stopping early only makes a search find less: the results are a prefix of the whole
      (`homotopy-model/tests/embeddings_through.rs`, `inference::tests::a_search_cut_by_the_clock_is_incomplete_not_wrong`),
      and nothing that was cut is remembered (a candidate list that was cut short is not cached, a
      level of composites is rolled back), so later searches of the same job are not the poorer for it.
      Measured (with the old totals, which kept the same machinery) native release on the fixture, limits 200ms
      to 2.5s: spent at most 29ms over (14ms over with the clock looked at on every call; 4 is the compromise);
      debug wasm in the worker, 700ms to 3s: 9 to 21ms over (`spent_ms` and `longest_step_ms` of the report are
      logged as "Inferring instances spent N ms (M ms for a generator, longest step ...)"). Unit test of the
      accounting: `attach_options::tests::a_generator_that_uses_its_time_does_not_use_the_time_of_the_next`.
    - *From the page*, as a backstop for what the search cannot see (a stretch that does not look at
      the clock), **per step, not per search**: before each step the worker reports `{kind: "progress", job,
      label, done, total, limit_ms}`, where `limit_ms` is `Job::step_limit` (for `AttachOptionsJob` the time for
      one generator; a step is within one generator) plus the longest checkpoint the worker has made so far (the
      silence after a report is a step and perhaps one of those); `0` is no limit. A report is posted at once when
      its label or `limit_ms` changes, otherwise at most every 40 ms. The page (`Remote::heard`, `limit_ms`)
      starts `Message::Watchdog` on the first report with a limit; when it fires it looks at how long the
      worker has been silent: past `limit_ms` + 750ms (`STEP_GRACE`) it waits `STUCK_CONFIRM` (100ms) more, so that
      what the worker said while the page was busy is heard first, and then the worker is stuck. Any report
      resets it, so a search of any number of generators that each keep their time is never touched; only a
      single step that outlives its generator's time and the grace is. The worker also posts `{kind: "partial"}`
      with an answer that does for the job if it is ended (`Job::checkpoint`: for `AttachOptionsJob`, the options
      so far, with the generator it is in the middle of in `cut_short` and the ones after it in `not_tried`;
      `Executor::run_with_checkpoints`; the first before anything is done, then as generators are finished but
      not before ten times the time the last took to make). Since the options are streamed (below) a
      checkpoint of a streamed search is the report only (`WireOutput::AttachOptions { streamed: Some(n) }`,
      the options being those sent ahead, which the executor flushes *before* it makes the checkpoint), so it
      is cheap whatever the size; without a stream it still holds all the options (1.5MB for the fixture). The
      page keeps the latest without decoding it. **When stuck** the page
      decodes it, ends the worker as Esc would (`stop` + `ensure`) and commits it as the outcome of the job
      (`InferenceReport::forced`; the notice says that a generator took longer than its time and could not be
      stopped, so it was ended: what the search had found before then is offered, the generator it was in the
      middle of is given up on, and the ones after it were not tried). What the worker had found for that
      generator is lost, and so are the generators after it. This is the robust choice and not "continue from the
      next generator in a fresh worker": that would send the signature and the search again and put the results of
      two workers together, to save the rest of the generators in a case (a stretch that does not look at the
      clock) that should not happen; a stuck worker is a bug to be fixed in the search, and the notice makes it
      visible. If the checkpoint cannot be read (or there is none yet) the search is done in the page instead (where
      the first layer stops it). Busy pill, refusal and Esc are as for any job: the job is the running one until
      it is committed. Test (debug build): `window.__homotopy_step_delay_ms = 4000` with
      `window.__homotopy_step_delay_label = "Inferring"` makes every step of the inference spin
      4s, which no search can see. `window.__homotopy_infer_grace_ms` sets the grace
      (high, to see the search keep its time by itself).
    - The plain cells and their reflections are in the same limit (they were not, before; on the full workspace
      of the fixture they are most of the time of a search in the debug wasm build, about 31s of 35s for a click
      at `[R3, R2]`, so the debug build gives up on many generators).
  - **Order**: the generators are looked at cheapest first (`cheapest_first`: those with no end that can be in the
    diagram first, then by `layout::weight` of their diagram, which costs the same whatever the size), for the plain
    pass and for the reflections. The options are put back in the order of the generators by `outcome`, so the list is
    as before (tested: no limit and a far one give the same options, and the whole-vs-`only` equivalence tests).
  - Debug hooks: `__homotopy.dispatch` takes `{"SelectPoint": [point, weak_units]}` (limits from the
    settings) and `{"SelectPointWith": [point, weak_units, {"enabled":..,"generator_ms":..,"max_dimension":..}]}`;
    a point is a list of `{"Singular": n}`, `{"Regular": n}` or `{"Boundary": "Source"}`.
  - A latent crash found on the way: `DiagramN::reflections` (`Rewrite::reflect` → `RewriteN::new`)
    panics with "Rewrite is malformed: [NotOrderedCorrectly]" for the reflections of some cells in
    some states of the `equivalence_to_adjoint_equivalence` example (every click after its action 74,
    898 of the 11,259 clicks tried), which the old search made unconditionally on every click. The search
    now only makes reflections that could match, so those clicks work, but the panic is still there
    for a reflection that does.
  - Known gaps: stretches of the search that do not look at the clock (`Pool::substitute` and the
    instance lookups, `composite.attach`, the collapses in `typecheck`, which are not interrupted, and the
    construction of the variants: inverses, bubbles) can still go over by their length (up to 29ms in the
    release build, 180 ms for a collapse natively, several seconds in the debug wasm); in the page (no worker
    yet) a step is as long as one candidate list (the clock stops it, but the page does not respond meanwhile)
    and the search is stepped in slices as the others; the searches that do not finish are cut, not made quick;
    the options a cut generator would have had are not there (use the setting to give it more time); a search
    of many generators that each take their time takes long (the price of no total); `Proof` generators with no
    name show as "Proof N" in the notice.

- **Attaching one generator** (`AttachOptionsJob::new(.., only)`, `can_attach`, `Action::AttachGenerator`
  and `Action::Arm`, `app/attach_target.rs`, `app/armed_chip.rs`; user-facing text in
  `doc/attaching.md`). Dropping a generator from the signature on the diagram, or picking it with the
  button on its row and clicking a point, searches for the ways to attach *that* generator only, and the
  outcomes are as for a click (none: "no attachment found"; one: attached at once; several: the option
  list).
  - Model: `only: Option<Generator>` in `AttachOptionsJob::new` (and `Work::AttachOptions`, `#[serde(default)]`,
    so the worker does the same): the passes look at that generator and its variants only, and the
    options are what the whole search gives for it, in the same order (tests:
    `homotopy-model/tests/attach_only.rs`, on the example corpus at points of the diagrams and their
    boundaries, with and without weak units; `inference::tests::a_schema_searched_for_alone_is_what_the_whole_search_finds_for_it`).
    The work is cut into pieces so that progress and `Esc` mean something: one step for each cell
    made from the generator (itself, inverse, weak identity, bubble, ...: `Variant`), one for each
    reflection that could match (`ReflectionCandidate`: typecheck and embeddings), and the steps of the
    inference as before; the progress counts the pieces when they have been made. A schema that is the
    only one searched for has the time for a generator, like any other. The worker is offered it as for any attach search (`may_take_long`: a schema, or reflections
    in four dimensions or more); a plain cell is done in the page (119ms in the debug wasm for `c` on
    `start`, 6 steps). The busy pill says "Searching for attachments" (and "Inferring instances" only
    when there is a schema to infer for).
  - `can_attach(signature, diagram, generator, weak_units, limits) -> Result<(), CannotAttach>`:
    what a drag asks, once, when it starts (`attach_target::start`, from `FolderView`'s
    `on_drag_start`), never searching: the dimension (more than the diagram's plus one: no), whether the
    cells that the generator starts at or ends at are in the diagram (a 0-cell: only through weak
    units), and for a schema whether inference is on and the dimension within its limit. It looks at the
    diagram as a whole and not at the point (a drag over the diagram never runs a search), so it can say no
    for all points and yes for a point where the search finds nothing. Never says no wrongly: tested
    on the corpus against the options that the whole search finds (`attach_only.rs`). Computing it
    walks all the cells of the workspace diagram (`Diagram::generators`, not cached: 5.5ms for the
    fixture's `start` in the debug wasm, more for the whole workspace) and is not done for rendering.
  - Dragging, with the pointer (`app/attach_drag.rs`; user decision: "VS Code allows the wheel during
    drags", and the browser's own drag and drop passes on no wheel and no keys, so a sheet could not be
    chosen during a drag). `onpointerdown` on a generator row (`ItemView::on_press`, from `folder.rs`) records a
    `Press` (origin, grip, which way the diagram is) and listens on the window (no pointer capture: the wheel
    goes to what is under the pointer anyway, and the svg then also gets the real mouse moves). Once the
    pointer is `THRESHOLD` = 4px away the press is a drag to the diagram if `Press::is_attach`: always for a press
    on the **pointing hand** (`.signature__item-attach`, a handle: also the way for touch, which has
    `touch-action: none` there and no drag from the row body), and for a press on the **row** if the pointer moves
    towards the diagram (`direction_to`: the sign of the horizontal distance between the middles of the row and
    the diagram) within 45 degrees of the horizontal; up or down, or away, is left to the browser. **Drag vs reorder
    (the choice, and why)**: rows stay `draggable` for moving them in the signature (HTML5 drag and drop, untouched,
    folders too), a native `dragstart` comes after 3 or 4px with no way to take it back, so the first pixels
    decide: `folder.rs`'s `on_drag_start` asks `attach_drag::dragstart`, which, if the drag is to the diagram,
    `preventDefault`s it (a cancelled `dragstart` leaves the pointer events going: no `pointercancel`, tested in
    Chromium) and starts the pointer drag at once. The alternatives were: a handle for reordering (changes the
    muscle memory of the common sideways-and-up/down moves in the sidebar), the pointing hand only (loses
    dragging the row), and a custom reordering too (a rewrite of drop zones, folder rules and autoscrolling, against
    "keep HTML5 for reordering"). The price of the direction rule: a diagonal start past 45 degrees is a native
    drag, which can still be dropped on the diagram (below) but has no wheel or keys, the same as before this
    change. A drag is `Phase::{Idle, Pressed, Dragging}` in a thread-local; the pure part (`Press`, `direction_to`)
    has unit tests. While dragging: a ghost (`.signature__drag-ghost`, a copy of the row without its preview and
    buttons, `position: fixed`, `pointer-events: none`, above and right of the pointer so as not to cover the
    cell under it or the tooltip), `body.attach-dragging` (grabbing cursor, no text selection; `--refused`:
    not-allowed), and `attach_target::begin` (the verdict of `can_attach`, as for the native drag). The diagram under
    the pointer (`document.elementFromPoint` against the svg of each registered sink) is told through
    `attach_drag::register` (`DiagramSvg::drag_sink`, only with `accepts_drops`): `DragSignal::{Over, Leave, Drop}`
    become the same `DiagramSvgMessage::{OnDragOver, OnDragLeave, OnDrop}` that the native events send, so the
    outline, `hit_to_act_on`, `point_at`, the tooltip, `Keys::wanted` and the sheet choice are one code for both.
    A diagram the generator cannot be attached to at all (verdict no) is not told (no outline); the ghost is dimmed
    and a drop on it toasts why (as the pointing hand does). Drop: `pointerup` over the diagram. Cancel: `pointerup`
    elsewhere, `pointercancel`, window `blur`, a `pointermove` with no buttons down, and `Esc`, a capturing
    `keydown` that `App`'s own `Esc` listener, which runs first, steps aside for (`attach_drag::active()`): while a
    drag is on, `Esc` is for the drag and not for a running job. A click right after the drag is swallowed (a
    capturing `click` listener for 100ms) so that ending on a row does not select it. A job running refuses the
    drop like a click (the pill shakes); the drag itself still works. Listeners are dropped a tick later
    (`defer_drop`: they may be the ones running).
  - The native path is kept for what the pointer drag leaves to the browser: a signature row carries the
    generator as plain text (the node, for moving it in the
    signature) and as `application/x-homotopy-generator` (`id,dimension`), with
    `effectAllowed = "copyMove"`. The page cannot read the data until the drop, so `attach_target` keeps
    the generator and the verdict of `can_attach` in a thread-local from `dragstart` to `dragend`.
    `DiagramSvg` (`accepts_drops`, `on_attach`) accepts a drag (`preventDefault` on `dragover`) only if
    it carries that type, the verdict is yes and the diagram is not waiting for its layout (the handlers
    are not there while `pending`: drops are ignored as clicks are); it outlines the simplex under the
    pointer (`diagram-svg__drop-highlight`, from the first shape of `hits_at`, no search) and on drop
    emits `(generator, simplex[0])`, which the workspace turns into `AttachGenerator(generator, point, weak
    units, limits)`. While a job runs a drop is refused like a click (the pill shakes; dragover is
    still accepted, so that there is something to refuse). Folder drop zones and the signature are as
    before; a drag of a folder carries no generator, so the diagram ignores it.
  - Picking: `Action::Arm(Some(generator))` sets `State::armed` (not part of the project, `Done(false)`,
    cleared by `ClearSelections`, which `Esc` does when nothing is running); `App::render` shows the chip
    (`ArmedChip`: "Attaching <name> click a point in the diagram, Esc to cancel", a button that disarms) and
    passes `armed` to the workspace, whose click then dispatches `AttachGenerator` instead of
    `SelectPoint` (this disarms at the start of the action). The button (`signature__item-attach`, a
    pointing hand, shown by the stylesheet where the pointer is: it must be in the DOM all the time or it is
    gone before it can be clicked) checks `can_attach` when pressed and toasts why not (in words that do
    not name the generator, whose name is a formula).
  - Tests: `homotopy-web/tests/tasks.rs` (`attaching_one_generator_gives_the_options_for_that_generator_and_picking_one_is_not_the_project`:
    the options are those of a click for that generator, picking is `Done(false)` and leaves the project
    alone, `Esc`/`ClearSelections` puts it down, a generator that is not there is an error, the action is heavy
    and refused while busy); in the browser (Playwright, not committed): arming with the button, the chip,
    `Esc`, clicking the chip, `dragTo` of a row on the diagram at nine heights (8 options at the top, "no
    attachment found" elsewhere, the highlight outlines the vertex or wire under the pointer and goes on
    drop), a drag that the diagram does not take (6-cell on a 4-diagram: no highlight, nothing runs; the button
    toasts why), a drop while the layout is pending (ignored) and while a job runs (refused, pill shakes).
  - Debug hooks: `__homotopy.dispatch` takes `{"AttachGenerator": [{"id": 48, "dimension": 6}, [{"Regular": 1}], false]}`
    (limits from the settings), `{"AttachGeneratorWith": [generator, point, weak_units, limits]}` and
    `{"Arm": generator|null}`; `state().armed` is `[id, dimension]`. Rows of the signature have
    `data-generator="id:dimension"`. In Playwright, `locator.dragTo` does a real drag (dragstart, dragover,
    drop, dragend), as does `mouse.down`/`move`/`up` for most rows (some rows of schemas lose the drag after
    `dragstart` with hand-made mouse moves, and `dragTo` works for them; a click straight after a drag is
    swallowed); synthetic `DragEvent`s with a `DataTransfer` work too. Wait for the workspace's layout
    (`.diagram-svg--stale`, `.layout-note`) before dragging, or the diagram does not accept the drop.
  - Measured (debug wasm, the fixture, worker, 3s limit): generator 16 (`ζ[η̇→η̇′]`, the schema that
    never finishes) dropped on a vertex of `start`: 4.1 to 4.8s to "no attachment found" with the notice
    (the whole 3s of inference, 0.8s of which was sending the 5MB signature to the worker the first time),
    against 15.4s for the full search with the same limit, which gives it a quarter of it and spends the
    rest on the plain cells; a plain cell, `c` on `start` at `[R1]` (8 options): 119ms in 6 steps in the page,
    against 7.9s; `c^2 = 1` (6-cell, with reflections) on the whole workspace at `[R3, R2]`: 5.8s (4 options),
    against 33.8s for the full search. With the default limit (20s) generator 16 takes the 20s and then says so.
  - Gaps: the 3D view does not take drops or armed clicks (`DiagramGl` has no attach by clicking);
    the verdict is for the whole diagram and not the point, so a drop at a point where nothing attaches
    runs the (usually quick) search and then says "no attachment found"; the busy pill does not
    name the generator; touch devices drag from the pointing hand only (shown all the time there), not from the
    row body (which scrolls the sidebar); the pointer drag was checked in Chromium (mouse, and touch through
    CDP) and not in Firefox or Safari (whether pointer events go on after a cancelled `dragstart` is the thing to
    look at there); the diagram cannot be dropped on when it is scrolled out of view or under another panel (what
    is under the pointer decides); the tooltip of a diagram the generator cannot be attached to still shows while
    the refused ghost is over it.

- **Definitions first** (`proof/definitions.rs`, `Action::SelectCell`/`SearchEverything`, setting *Offer
  definitions first when clicking a cell*, `offer_definitions`, on by default and kept with the inference
  settings in `homotopy-inference-settings`; user-facing text in `doc/attaching.md`). A heuristic shortcut
  (policy above): a click on one cell offers that cell's *definition* at once instead of making the person
  wait for the whole search.
  - **Lands on a cell** (`cell_at(haystack, point)`, the point after the boundary split): the point is not
    empty, *every* height in it is `Singular` (a vertex in each coordinate: on a wire or a region there is
    no cell to define, so the click is as before), and the slice at the point (`generators_near(point, 0)`:
    the fibre, what the other coordinates have there) has cells of the dimension of the diagram that are all
    of *one* generator `g`. Several such generators (sheets of different cells), none (a vertex with only
    lower cells, a crossing) or a point with a chosen sheet that is not a vertex: as before. Cells of lower
    dimension never count (a definition of one would need raising, which is not a definition of the cell).
  - **Definition** (`definitions_of(signature, g)`): a generator of dimension `dim(g)+1` with `g`'s diagram as
    source or target (diagram equality, no search). `id + 1` first (they are made right after the cell), then
    the others in signature order. An instance `d[σ]` of a schema's definition has `g[σ]` as boundary, so it is
    found like any other *if it exists as a generator*; instances are not inferred, and reflections
    (duals...) are not looked for: "Search everything" does those.
  - **Verification**: `DefinitionsJob` (model crate; a `Job`, label "Looking for definitions") runs, for each
    candidate in turn, `AttachOptionsJob::new(.., Some(d)).plain_only()` (`plain_only`: only the plain pass, the
    candidate and its inverse, no reflections or inference), so the options are exactly the full search's options
    for `d` at the click, in its order and with its time limit per generator (`cut_short` lists the candidates
    given up on). It stays on the page (`Task::offer` gives it no worker): one step is the setup of a candidate
    or one cell variant. Native release, the tangent categories fixture, `start` (id 62, dim 5) as the
    workspace, click on its cell: `Proof 63` offered in 43ms (0.2ms to set up), against 2.95s for the full
    search at that point.
  - **Showing**: `State::start(SelectCell)` returns `Task::Definitions` when there are candidates, else the
    ordinary `Task::AttachOptions`; `finish` of the definitions with none verified starts the full search (the
    click is as it was), with some it sets `Selectables::Attach(defs)` and `State::definitions =
    Definitions { of, count, cut_short, more: Some(Search) }` (not `Done(false)`: the drawer changes; the
    project does not) and *always* shows the drawer, even for one definition (no auto-attach), because
    otherwise there would be no way to the rest. The drawer (`app/attach.rs`, `view_definitions`) has
    "Definitions of X", the definitions (`attach__option--definition`), a note that this is a shortcut, and
    "Search everything for more options" (`Action::SearchEverything`: `Task::AttachRest { job, shown }`, the
    full `AttachOptionsJob`, with the worker as for any attach search). Its output keeps the definitions at the
    top (in order) and lists the full search's options except those equal to them (`AttachOption` equality); the
    inference notice is the full search's. `count` leading options are the definitions, so the keys 1-9 and
    `Select(i)` index the flat list as before. `ClearSelections`, picking an option, and any other list clear
    `definitions`; cancelling (Esc) with nothing found yet leaves the drawer as it was, and with options found
    keeps the definitions and what was found (`more` stays, so *Search everything again* is offered). The task
    carries the shown definitions, so a cancelled search leaves nothing stale. Both actions are refused while
    busy and have the heavy label. The rest of the search **streams** (below): `State::begin_stream(Some(defs))`
    keeps the definitions on top and `Listing::Searching` from the start, and the options that arrive are
    filtered of those equal to a definition as the final one is.
  - **Setting off**: the workspace dispatches `SelectPoint` as before. Armed clicks and drops
    (`AttachGenerator`) never use it.
  - Tests: `homotopy-model/tests/definitions.rs` (every definition offered is among the full search's options,
    with and without weak units, on definitions that exist in the corpus at the cell's own vertices and on one
    made for every cell a click lands on; the `g+1` first; the fixture, only with `HOMOTOPY_TANGENT_FIXTURE=<.hom>`,
    add `HOMOTOPY_FIXTURE_FULL=1` to compare with the full search); `homotopy-web/tests/tasks.rs`
    (`a_click_on_a_cell_offers_its_definitions_first_and_everything_else_on_asking`: the task, the project
    untouched, the marks, merging, a click elsewhere being the old click). Debug hooks: `__homotopy.dispatch`
    takes `{"SelectCell": [point, weak_units]}` and `"SearchEverything"`; `state().definitions` is
    `[count, more_to_search]`. Web tests run natively: `cargo test --target x86_64-unknown-linux-gnu` in
    `homotopy-web`.
  - Gaps: schemas' instances are only found if they exist (not inferred, no "make the instance"); the 3D view,
    armed clicks and drops do not use it; the definitions are searched on the page, so a definition that is
    slow to match (high dimensions) takes its time per generator there (steps are one variant: the busy pill
    and Esc work between them); a click that lands on a cell whose definition is a *reflection* (dual) is not
    helped; `cell_at` ignores orientation, so a click on the inverse of `g` finds nothing and falls through to
    the full search.

- **Sheets** (`Depths::fibre/sheets` in `homotopy-core/src/projection.rs`, `homotopy_model::sheets`,
  `homotopy-web/src/app/diagram_svg/sheets.rs`; user-facing text in `doc/attaching.md`). The 2D view of a
  diagram of more dimensions shows at each position the front-most (`Depths::node_depth`, smaller z is nearer)
  cell of what the position carries; the rest is behind it. The position's *fibre* is the diagram its
  scaffold node holds (the diagram with the first `N` coordinates fixed), and its *sheets* are the singular
  heights `z` of the fibre in the next coordinate, each a `Sheet { z, generator, front }` (`front`: it is
  the one `node_depth` picks); `Depths::sheets(coord, within)` nests (`within = [z0, z1]` gives the sheets
  of the slice at `z0` then `z1`, for the coordinates further in). It costs one pass over the fibre's singular
  slices. Tests: `homotopy-core/tests/sheets.rs` (against slicing the diagram, at every position of the
  3 and 4 dimensional examples, in 1D and 2D views).
  - **The query**: choosing a sheet adds `Interior(Singular(z))` (and one more for each coordinate drilled) to
    the point of a click, a drop or an armed click, after the `N` coordinates of the position: the point is
    then longer than the view's dimension, which is all the model needs to know (`asked_for.len() >
    workspace.view.dimension()`). `AttachOptionsJob` searches it through `embeddings_through` as any point:
    it has always been a prefix of the coordinates, so the search fixes one more and the rest is free.
    `BoundaryPath::split` takes it as a point inside the boundary when the position has a boundary coordinate
    (the coordinates after the last boundary one are the interior path), so no change was needed. What is
    offered is exactly the options of the point without it whose embedding contains the sheet, in the same
    order (`homotopy-model/tests/sheets.rs`, on the example corpus, with and without weak units and at
    boundaries): morphisms that span several sheets qualify if their extent covers `z` (a reduction, not an
    exclusion). Nothing changes for a click with no sheet chosen.
  - **Choosing** (`homotopy_model::sheets`: `Choice { position, path }`, `Stack`; pure, tested natively): the
    stack under the pointer is the sheets of the position of each shape of the kind that is on top (wires that
    overlap each give theirs, tagged; regions under a wire are not listed), default the front sheet and *not*
    part of the point; `step` goes to the next, round; `drill` first fixes the default and then pushes the
    first sheet inside the chosen one (never silently: it is a key press), `undrilled` pops. A choice that is
    not of a position under the pointer is no choice. `DiagramSvg` (`sheets` property, set by the workspace)
    keeps `choice`, `hits` (`hits_at`: all shapes under the pointer, top first; the first is the one on top;
    only the first is looked for when `sheets` is off), and a `keydown` listener on the window
    (`gloo`, non-passive) that only takes `[`, `]`, `Tab` and `Shift+Tab` while the pointer is over a stack
    worth listing (`Keys::wanted`, a shared flag: several diagrams can exist, and Tab is not ours otherwise;
    not from text fields). `Shift+wheel` is an `onwheel` on the svg that stops the event before the pan/zoom
    component turns it into `SwitchSlice` (browsers give Shift+wheel as `deltaX`: both are read). Moving to
    another shape forgets the choice; leaving keeps it, so that a drop (`OnDragOver`/`OnDrop` use
    `hit_to_act_on`/`point_at` too, with the choice's shape outlined) or a click that comes back to the place
    uses it. During a drag of a generator with the pointer (`attach_drag.rs`, see "Attaching one generator") the
    diagram is told where the pointer is as `OnDragOver` says, which updates the tooltip and `Keys::wanted`, and
    Shift+wheel, `[`, `]`, Tab work as over any diagram (tested in Chromium with `mouse.down/move/wheel`: two
    sheets, the choice changes, drop uses it). During a *native* drag (a row started up or down) the browser
    passes no keys or wheel, so there is no choosing then.
  - **The tooltip** is the stack (`tooltip::SheetStack`: `z₃=k` rows with `describe_cell` names, the current
    one highlighted, the sheet's name `S2`, *front* (per sheet: `foremost`; *drawn here* if the drawn one is not that), a breadcrumb `z₃=1 › z₄=0` when chosen, a hint for the keys), shown when there
    are several sheets, one is chosen, or the sheet has sheets inside it. Wires whose edge depth (`z + 1` in
    `GraphicElement::Wire`, now on the element as `data-depth`) is not the first chosen `z` are dimmed by
    setting `data-dim` on the elements directly (`apply_dimming`, also after each render): choosing does not
    render the diagram again, which is much of the work of a large one. Surfaces cannot be dimmed per sheet
    (`Projection::generator` is the max over the whole fibre).
  - **Identity of sheets** (`homotopy-core/src/projection/sheets.rs`; module doc has the definitions). `Sheet`
    has `id: Option<SheetId>` (`S1`, `S2`, ... via `SheetId::label`; `None` for the sheets *inside* a sheet,
    `within` not empty: only the first coordinate after the position's is followed), `front` (as before: the
    one `node_depth` draws *at this position*; it can be another sheet at the next position) and `foremost`
    (the per-sheet notion, which the tooltip's *front* tag uses). `Depths::sheet_count/sheet_boundaries/
    sheet_boundaries_at/thickness` and `thickness_steps` (part C) are new. Everything is made lazily, on the
    first question (`OnceCell` in `Depths`), so a diagram that nobody asks about pays nothing: one pass
    over the edges of the scaffold (1 to 60 microseconds on the examples).
    - *Identity*: a union-find over (position, singular height) pairs, joined along every edge of the
      scaffold `Depths` keeps by the singular image of the rewrite (`cone_views`, `for_each_image`: each height
      of the source goes to one of the target; a cone with several source heights takes them all to one),
      **when the cell (`max_generator().generator`) is the same at both ends**. A different cell means
      something happens there (surfaces end on a line, lines meet at a 3-cell), so the associator's four lines
      and its 3-cell point are different sheets, and a line is one sheet along all of its length whatever
      its neighbours do (checked in the browser: the 2-cell `m` has the same `S9` at `[T,S0]` and `[R1,S0]`).
      Tried first and dropped: joining everything that is carried one to one and cutting only where three or
      more heights are carried into one: that cut every line that has surfaces ending on it (a line is a
      junction of the surfaces at every one of its positions) into one sheet per position. A cup is then two
      sheets (the two arms) and the cell of the cup between them.
    - *Foremost*: the sheet is not hidden by another anywhere (`hidden == 0`), or is the drawn one at more of
      the positions it shares with others than it is hidden at. A sheet alone everywhere is foremost; of two
      strands that overlap on only a part of their length (`crossing`, `half_braid`) one is foremost along
      the whole of its length and the other behind along the whole of its (it is `front` only where it is
      alone, which is the old flipping). Positions with the sheets are counted, regions too.
    - *Unit cones and the atomic filter*: `Depths` leaves out the regular slices of later coordinates
      (`ExternalRewrite::is_atomic`) and with them the edges that carry unit cones from a regular slice. No
      sheet information is lost: such an edge is a sparse identity then an edge of the last coordinate, which
      is kept. Tested against the scaffold with every edge (`leaving_out_regular_slices_loses_no_unit_cone`).
      Identity is checked on kept edges only: across the diagonals it differs where the way round passes a
      junction, which cuts the sheet.
  - **Where sheets meet: fans and echoes** (`Depths::sheet_boundaries`, `sheets_meeting_at`, `echoes`;
    `homotopy-graphics/src/svg/marks.rs` for the geometry; `diagram_svg/marks.rs`; setting *Show where sheets meet
    (fans and echoes)*, still `sheet_outlines`, on by default; workspace diagram only). These replace the rings
    and dashed stretches that were drawn at boundaries, which got attention and did not say what they were about.
    Only at *points* of the picture (every coordinate singular: where the cells are): a height there that **one**
    edge carries a height into is where a sheet *ends* (`BeginsOrEnds`; `matchsticks`, `stacks`, `monoid_unit`),
    and one that **three or more** carry heights into is where sheets *meet* (`MergesOrSplits`; the 3-cell of the
    associator: 10, of the snake: 6, the vertex of `start`: 18; `Sheet::arms` is the carried-in count). It is
    **not** "a cone with `source_len() == 0`": every edge goes from a position to one of a dimension less, so a
    sheet that lies along a wire has such a cone on every edge from the regions beside it (32 of them for
    `crossing`, which has none), and the junction rule at every position rang every vertex of the associator's
    wires (11, against the 1 now). Sheets that end or meet along a wire are not marked: the wire is drawn.
    - **Fan**: at a meeting, thin arcs about the vertex on its up and right side, one for each sheet that meets
      there (`sheets_meeting_at`, at most `MAX_PAGES` = 4), like the edge of a stack of pages; where a sheet
      only begins or ends, a single arc. A still picture (no curl, no arrow). Tried first: pages radiating from
      the vertex (looked like a claw) and arcs shifted like a stack (heavy); nested arcs about the vertex read best.
      No backing (user: it should read as part of the layer's material, not a stick pasted on top): the arcs
      are a translucent dark stroke with `mix-blend-mode: multiply`, a darker shade of the colour under them,
      like creases in the surface; `pointer-events: none`, in a `g.sheet-marks`
      (`data-fans`/`data-ends`/`data-echoes`), over everything, made once for the prepared diagram (`OnceCell`).
    - **Echoes** (`Depths::echoes(wire)`, at most `MAX_ECHOES` = 3): a wire (a segment between a meeting and the
      position beside it) gets one echo for each *step* it is behind the front-most of the wires at that point.
      A wire's layer is the height `z` that the hover marks as drawn at the position beside the point
      (`node_depth`), and a step is a different `z` among the wires there, so the wires of the first layer have
      none and the echoes say which wires go into which layer. (Tried first: the edge depth at the vertex, which is
      0 for every wire at `start`'s vertex and so said nothing.) Wires with no drawn sheet have none. On the
      associator one wire has one; at `start`'s vertex the three from the top, which are on the second layer. Each
      echo is the wire's own path (the layout's curve) again, translated by a few pixels to the up and right side
      of the wire (`echo_side`), in the wire's colour, stroked with a gradient (`reach_from`: 36px from the
      vertex) that fades out, over a paler wider one for contrast. The gradient element is made with `VTag`
      because `html!` lowercases `linearGradient`.
    The tooltip's detail still says "a sheet begins or ends here" / "sheets merge or split here". Gaps: where
    several wires leave in nearly one direction (the three at `start`) their echoes overlap; echoes are made for
    the wires of the diagram's own layout and so need no new geometry, but are not interrupted by wires in front;
    the 3D view has none; the fan for a sheet that ends was checked in a mock, not on an example (none of the
    five example projects or `start` has one in the 2D view).
  - **Highlighting a sheet** (`diagram_svg/sheet_highlight.rs`; `Depths::sheet_places`, `sheet_ids_at`). The
    current sheet of the stack under the pointer (the one chosen with Shift+wheel, `[`/`]`, Tab, or the
    default; the outer sheet when one is fixed) is highlighted over the whole diagram, wherever it is: the
    regions it is at are tinted (cyan), the wires drawn for it get a halo under the wires (so it shows as an
    outline), the drawn vertices it passes through a disc, and where it is behind another at a wire a dashed
    line runs along the wire. Always on (it follows the hover; no setting) and only where the tooltip lists a
    stack (several sheets, or one chosen). The tooltip is not interactive, so the "hover a row" behaviour is
    that the highlight follows the current entry. Done like `apply_dimming`: four paths in two groups
    (`g.sheet-highlight`, under the wires and over everything) are rendered empty and their `d` is set by
    `apply_highlight` (in `refresh`, `leave` and `rendered`); the diagram is not rendered again. The shapes of a
    sheet (`Extent`) are made from `PreparedDiagram::sheet_index` (which action shapes are at which sheet and
    height, built once from `Depths::sheet_ids_at` at the first hover) and kept per sheet. Shapes belong to the
    sheet by the position `simplex[0]`, as the hover does. Cost: one pass over the diagram's shapes at the first
    hover, then a few attribute writes. Gaps: wires are matched by their first position only; the colour is one
    cyan (it can be close to a generator's); the 3D view has none.
  - **Cost of the sheet marks on `start`** (release, native): `Depths::new` 60µs, the sheet data 70µs, the echoes
    of all 96 wires 120µs, listing the wires 190µs; shading reads one fibre size per triangle. Highlighting is
    attribute writes after the first hover of a sheet (one pass over the shapes).
  - **Thickness shading** (`thickness_step(s)` in `projection/sheets.rs`, `GraphicElement::build_shaded`,
    `DiagramSvg`, setting *Shade surfaces by the sheets behind them*, `sheet_shading`, on by default; workspace
    diagram only). A surface's thickness is the number of sheets through it: `Depths::thickness`, the number
    of singular heights of the fibre, **at the region the triangle belongs to** (`Depths::surface_position`, its
    first corner: the same position that hovering over it lists the sheets of, so a surface shaded for `k` is one
    that the hover says has `k` sheets; `Shading` in `projection/sheets.rs` is the range and the step). It was
    the fewest of the three corners, which was wrong: two of them are on the wire and the vertex beside the
    region, and where a sheet ends on a wire the wire has fewer sheets than the region, so a two-sheet region
    was shaded as one (20 of the 74 triangles of `start` of the tangent categories project, 16 of 44 of the
    associator; tested against `Stack` in `homotopy-model/tests/shading.rs`, with the fixture when
    `HOMOTOPY_TANGENT_FIXTURE` is set, and against `Depths::sheets` on the 3D examples in
    `homotopy-core/tests/sheets.rs`). The shade follows the grid of regions (one position each) and not the
    coloured areas of a generator, which can hold positions of different thickness; that is what the hover says.
    It is normalised to the thinnest *surface drawn* (0, as it always was), scaled into
    `SHADING_STEPS` = 4 steps (0 to 3: differences from the minimum when the range fits, else scaled and
    rounded so that anything thicker than the thinnest is at least 1 and the thickest is 3; tested). Surfaces
    of the same generator and step are merged as before, so the elements are those of `build` split by
    step (`build_shaded` returns each with its step; `build` is the same with step 0): no extra DOM. The
    step is `data-thickness` on the colour path, and `styles.css` has `filter: brightness() saturate()` for
    steps 1 to 3 (0.92, 0.84, 0.75): darker and a little more saturated. **Placeholders**: the hatching is a
    separate path over the surface, so the filter does not touch it, and both stripe colours stay visible
    (the light stripes on black are over a path that is already as dark as it goes). Changing the setting
    prepares the diagram again (the grouping is made then); a diagram whose surfaces all have as many sheets
    has no steps. On by default because the steps are subtle (about 8% each); the exports (`GraphicElement::build`)
    do not shade. Gaps: a CSS filter per shaded path (one per generator and step, not per surface); the 3D
    view is not shaded; a surface whose sheets are all hidden by a wire is still counted.
  - **The heuristic** (off by default; `InferenceLimits::narrow_to_sheet`, setting *Heuristic: infer instances
    only from cells near the chosen sheet*, kept with the inference settings): with a sheet chosen, the pool of
    cells for the replacements of placeholders (`Pool::new` in `AttachOptionsJob::new`) is the generators in
    the slices of the haystack within `SHEET_REACH` (1) heights of the point's, per coordinate
    (`Diagram::generators_near`), and not all of the diagram's generators. It can miss instances that use
    cells from elsewhere; every instance is checked as before, so none is invalid (test: every option attaches).
    It is not always a subset of what the whole pool finds: that search stops at `MAX_SUBSTITUTIONS` tried, and
    a smaller pool reaches substitutions that the whole one never gets to (on the fixture the whole pool found
    none in 5 s where the narrowed pool found 17). `InferenceReport::narrowed` says it was in effect and the
    notice says so and how many of the diagram's cells it looked at.
  - **Measured** (the fixture's workspace at the vertex `[S1,S2]`, whose fibre has two sheets `z₃=0` (the cell
    `l`, front) and `z₃=1` (`𝒞`); inference limit 5 s; the worker). Native release: a click 8.1 s (5.0 s of it
    inference), with `z=0` 8.0 s and 2 options, with `z=1` 5.6 s and 1 option, `z=1,0` and `z=1,0,0` the same;
    with the heuristic and `z=1` 0.62 s (inference 36 ms), and with `z=0` 5.9 s but 19 options, 17 of them
    instances that the whole pool did not reach. Debug wasm: click 74.5 s, `z=0` 68.7 s (2 options), `z=1`
    27.6 s (1 option, applied), `z=1,0` 27.1 s, `z=1,0,0` 26.7 s, `z=1` with the heuristic 22.5 s (inference
    0.8 s). Drops of `lc=l` (a plain 6-cell that attaches at `z=0`): native 2.15 s, `z=0` 2.3 s, `z=1` 60 ms;
    debug wasm 51 s, 46 s, 1.8 s. `c²=1`: 80 ms natively and 2.5 to 3.9 s in the debug wasm whatever the sheet.
    Schemas (5 s limit): the time is the limit, 5.0 to 5.9 s, with or without a sheet, except where
    the heuristic is on and `z=1`: 37 ms inference (1.4 s in the debug wasm). So choosing a sheet where nothing
    attaches saves the whole of the work, and choosing the sheet where something does attach saves little: the
    time is in matching the cell, which fixing more coordinates does not help (`z=0,0,0` is as slow as `z=0`).
    In the browser: a real click with `z=1` chosen applied the one option (46 s, the 20 s default inference
    limit), an armed click and a synthetic drop with `z=1` chosen said "no attachment found" in 2.9 s and 3.3 s
    where the plain ones attach in 46 s.
  - **Debug hooks**: `__homotopy.sheet(json)` (`{"hover": [x, y]}` puts the pointer somewhere in the window
    as a mouse move does, `{"step": n}`, `"drill"`, `"undrill"`, `"clear"`, `{"set": [z, ..]}` chooses at
    the position under the pointer; done a moment later) and `__homotopy.sheets()` (JSON: what is under the
    pointer, the entries of the stack with their positions and `z`, the current one, whether it was chosen,
    the point chosen, how many wires are dimmed). Real mouse moves, `page.keyboard.press("]")`,
    `Tab`, and `keyboard.down("Shift")` with `mouse.wheel` work in Playwright. The 5.3MB fixture takes ~40 s to
    import in the debug build; set `DecreaseView` once to get the 2D view (it opens in 3D).
  - **Homotopy drags**: a horizontal drag (an `Expand`, for a 3-dimensional diagram) acts on the chosen sheet. The
    choice is copied at pointer down (`drag_choice`; the pointer moves before the drag threshold, and moving to
    another shape clears `choice`), and `pointer_move` picks the shape to start from with `pick_hit` (as a click
    does) and hands `drag_to_homotopy` the first `z` of the choice (`chosen_sheet`, only if the choice is at that
    shape's position) instead of `Depths::node_depth`. No choice, or one at another position: the front sheet, as
    before. Contractions act on all sheets and vertical drags have no depth, so they are unchanged. Test:
    `a_horizontal_drag_expands_at_the_sheet_chosen_and_else_at_the_front` (`diagram_svg.rs`).
  - Gaps: only the first chosen
    `z` dims wires; surfaces and vertices are not dimmed; the 3D view has no sheets; a native drag (a row
    dragged up or down first, then to the diagram) gives no keys, so a drop uses a choice made before the drag
    (only when the pointer comes back to the same shape), while the pointer drag has none of this gap;
    `Stack` lists the sheets of the shapes of one kind (the regions behind a wire cannot be chosen). Identity of
    sheets (`SheetId`) is only followed for the first coordinate after the position's (the sheets inside a
    sheet have none and are never `foremost`); sheets of the same cell that are never linked by an edge stay
    different sheets (two separate wires of one generator), and one that is cut by a different cell on it is two
    (a line with a bead on it is `S1`, the bead, `S2`); "front" is a count (hidden at more positions than drawn),
    so two strands that cross over each other in turn are one front and one behind, or neither, by their
    majority; `Stack`'s default sheet is still the one drawn (`front`), not the foremost one; the fans and echoes mark
    points only, and shading counts heights (a sheet that comes back to the position counts twice).

New expensive work: write it as a job (or give an existing job another kind of step), label it,
and check it with the debug hooks below.

- **The list fills in as the search goes** (`AttachOptionsJob::take_found`, `Slot`, `Found`, `Streamed`;
  `State::found/stopped/restart_stream`, `Listing`; `Action::PickStreamed/StopSearch`; `Remote::options`,
  `Executor::run_streaming`; `components/search_status.rs`; user-facing text in `doc/attaching.md`). A search can
  take many seconds and the option wanted is often found early, so the drawer opens at the **first option found**
  and the rest are put into it as they come, instead of when the whole search is over.
  - **Deltas.** `take_found` hands out `matches[reported..]` (the job only appends to `matches`; `outcome` and
    `checkpoint` sort a copy, so taking is valid until the very end). In the page `App::work` takes them after
    each slice (`Task::take_found`); in the worker `run_checkpointed`'s `on_progress` hook takes them before each
    step (before the step's delay, so they are not held up by it) and the executor sends **only the new options**
    as `{kind: "options", job, bytes}` (`Delta { store, found }`: its own store, each option with its `Slot`),
    which the page decodes as they arrive (`Remote::options`, which also keeps them). The final answer then
    carries the report and `streamed: Some(n)`, **not the options again**; `Remote::response` puts what it kept
    in their slots and refuses the answer (`Err`, so the job is done in the page) if it does not have exactly
    `n`. If the executor did not stream (`run`, `run_with_checkpoints`) or `n` is not all of them, the answer is
    whole as before (`streamed: None`), so the wire stays readable both ways. A garbled delta is a failure of
    the worker like a garbled answer: the job is done in the page, and `State::restart_stream` forgets what the
    worker had sent (the page's job lists them again).
  - **Order and numbering.** The final list is the options sorted (stably) by `Slot { generator, pass }`: the
    index of the generator in the signature and the part of the search (`Pass::rank`), i.e. exactly the old
    `(index, rank)` sort. `Streamed` keeps a `BTreeMap<Slot, Vec<_>>` and lists it flat, so the list shown is
    *always* in the final order: an option is put in its slot, the list grows in the middle as well as at the
    end, and what was shown only gets further apart (test: each list is a subsequence of the next). A slot is
    filled in order by one source, so a search split over several workers (a later change, one per generator
    group) can send deltas for disjoint generators in any interleaving, and the page's `Streamed` is the same.
    Chosen: **no numbers while searching**. The drawer shows none anyway, and the keys 1-9 (`Select(i)`, indices
    of the final flat list) are refused while the job runs (they are not `allowed_while_busy`), so an index never
    changes meaning under the user's finger; when the search is over (or stopped) they index the list on show as
    before. Rendering: option rows have keys (`option_keys`: the generator, label, embedding, and a count for
    duplicates) so that an insertion in the middle does not typeset the shifted rows again; instance groups
    stay closed and render nothing under the header.
  - **State.** `State::stream` (private: `above` the definitions of an `AttachRest`, `found: Streamed`, `shown`)
    is made when the search is started (`begin_stream`) and `State::found` shows it (`options`, `definitions`,
    `listing = Searching`; the first replaces the list that was there and clears highlights). `Listing` is
    `Complete | Searching | Stopped`; `finish` sets `Complete` and the final list (so streamed and final are the
    same), `clear_selections` drops the stream and the listing. The app renders because `found` returns `true`.
  - **Drawer.** While `Searching`: a sticky line at the top (`SearchStatus`: "Still searching…", the busy job's
    label and `k of n` as the busy pill has them, from `busy::watch`, a second listener of the pill's events, so
    progress does not re-render the app) with a **Stop** button; while `Stopped`: a note. Clicking an option
    dispatches `PickStreamed(option)` while searching and `Proof(Attach(option))` otherwise.
  - **Picking while searching** (`App::dispatch`): `PickStreamed(option)` ends the job (`end_job`: take it, hide the
    pill, `stop` + `ensure` the worker, no toast; nothing was committed, so the project is as it was) and goes on
    to `State::start`, which clears the stream and starts `Proof(Attach(option))`. The option travels by value,
    not as an index, since the list grows between the render and the click; it is complete (`AttachOption`
    holds its diagrams), so nothing of the job is needed. Test: `picking_an_option_while_the_search_goes_on_ends_it_and_attaches_exactly_that_option`.
  - **Esc / pill / Stop keep the options** (`App::cancel` -> `State::stopped`): the job is dropped as ever (the
    worker ended) and `listing = Stopped`, the options stay, a notice says the search was stopped and the list
    may be incomplete (there is no report: the worker is gone), a toast says how many. If nothing had been
    found, the list is as before and the toast says "Cancelled". This is within the policy (the project is
    exactly as it was: listing options changes nothing). A worker that crashes mid-search keeps them too. The
    close button of the drawer (`ClearSelections`) while searching ends the job and puts the list away.
  - **One option.** `finish` attaches a lone option only when the search *ends* with it (`outcome.options.len()
    == 1`), never on the first arrival; and not when the outcome is `forced` (the stuck-worker path: it may have
    missed others), where the list is shown. Test: `one_option_is_attached_at_once_only_when_the_search_ends_with_it`.
  - **The stuck-worker path** is unchanged in kind: `end_stuck_worker` decodes the latest partial (now the report plus
    `streamed: n`, merged with the options the page has received; a mismatch falls back to doing the job in the
    page and `restart_stream`). Test: `options_sent_ahead_by_a_worker_are_the_options_of_the_answer` decodes
    every checkpoint against the deltas before it.
  - **Tests.** `homotopy-model/tests/streaming.rs` (job-level deltas equal the outcome, same order, on the example
    corpus at points with and without weak units, taking every step and every third; late-found before early;
    cut-short search; worker deltas, checkpoints and answer on a thread of its own; a missing delta refuses the
    answer; no stream = whole answer; the tangent categories fixture with `HOMOTOPY_TANGENT_FIXTURE`, at `[]`
    with 150 ms for a generator: 131 options in 9 arrivals, debug build 7 s), and in `homotopy-web/tests/tasks.rs`
    the listing, picking, stopping, one-option and definitions-plus-rest tests. Debug hooks: `{"PickStreamed":
    i}` (the i-th of the list on show) and `"StopSearch"` in `__homotopy.dispatch`, `state().listing`.
  - Gaps: the progress is the count of steps (generators, reflections, schemas), not "generator k of n"
    exactly; options of a stopped worker search that were found but not yet sent are lost (they are sent before
    each step, so only the step that was running); the page renders the whole drawer list on each arrival (rows
    are keyed, groups closed); several sources are supported by `Streamed` but there is one worker.

- **Attach options drawer: instances in groups** (`app/attach.rs`). The inferred instances of schemas
  (`AttachOption::substitution` is `Some`) are not listed one by one: plain options stay ungrouped at the
  top in their order, then one collapsible group per schema (in the order of its first instance), labelled
  with the schema's name (KaTeX, one formula) and "N instances". Typesetting the formulas of a screenful of
  instances was a couple of seconds of lag (user report), so **a closed group renders nothing under its
  header** (`InstanceGroup` renders its options only when open: no KaTeX for hidden ones). Closed by
  default; a group that was opened stays open for the rest of the session (`OPENED`, a thread-local set of
  schemas, so that the next click that has instances of the same schema does not shut it). Hover
  highlighting and clicking work as for any option (`view_option`). The keys 1 to 9 still index the flat
  list in the order the search made it (`Action::Select(i)`), which is not the order shown. The signature
  sidebar already keeps the instances of a schema as its children, closed by default and rendered only when
  open (`GeneratorInfo::instances_open`, the "▸ N" toggle). `group_instances` is pure and tested.
- **The notice about the search** (`components/inference_notice.rs`, lines are `NoticeLine::Text` or
  `NoticeLine::Names { text, names }`): the lists of generators ("Gave up on 19 schemas after 5 s each ▸",
  "Not tried: 14 generators ▸") show only a header with the count; opened, the names are rendered with
  `TexSpan` (KaTeX, via `seal`), at most `MOST_SHOWN` = 50, then "and N more". Nothing of a closed list is
  rendered, so the notice is cheap whatever the size; it scrolls (60vh) when a list is open.
- **Placeholder stripes** (`homotopy_graphics::style::Color::stripes`, `Stripes`, `LIGHT_STRIPES_BELOW`):
  a placeholder's stripes are drawn light when the relative luminance (WCAG) of its colour *as drawn*
  (lightened for the dimension and orientation, `Color::lighten`) is below 0.05 (black, a grey of about
  `#3f3f3f`; the palette's other colours, the darkest at 0.13, keep dark stripes), dark otherwise. 2D:
  both hatch patterns are defined per diagram (`PLACEHOLDER_HATCH_ID`, `PLACEHOLDER_HATCH_LIGHT_ID`) and
  each surface picks one (`DiagramSvg::stripes_of`); the signature rows too (CSS class
  `signature__generator-placeholder-light`). 3D: stripes are gaps (screen-door transparency) on dark
  stripes' colours and opaque white on a light-stripe one (`light_stripes` uniform, `frag.glsl`). Tests:
  `style::tests`; browser: a black placeholder region in 2D shows white stripes, blue, white and orange keep
  dark ones; the 3D view with black placeholders renders (software GL).

- **Deleting generators** (`Signature::removal_nodes`, `removal_cascade`, `removal_preview` in
  `proof/signature.rs`; user-facing text in `doc/placeholders.md`). `SignatureEdit::Remove` removes
  `removal_nodes(node)`: the node and its descendants (a folder's contents, a generator's instances) and,
  transitively, every generator that mentions one of them (diagram, or an instance's substitution) or
  lists a removed **placeholder** in `depends_on` ("there always exists"). Listing an *instance* does
  not cascade (the settings only offer placeholders, `parameters` ignores other entries, and a
  placeholder flag on an instance does not count); that rule is tied to the meaning of explicit
  dependencies and would differ for templates, whose instances would cascade. `depends_on` entries that
  name removed generators are dropped (ids are reused: `next_generator_id` is the max plus one). The
  workspace, boundary and stash are cleared by what the *whole* cascade mentions (`removal_generators`),
  not only the node (a generator that depends on a placeholder it does not mention is in the workspace
  without mentioning it). The preview shares the code, so it cannot disagree; the one pass over the
  diagrams of the signature remembers each diagram's generators in a thread-local (`mentioned_by`, keyed by
  the hash-consed diagram, so renames do not invalidate it) and `removal_preview(node, budget)` looks at
  at most `budget` new diagrams per call (`None`: ask again), so hovering stays cheap (fixture, 125
  generators: 8ms per cold preview in the release build, 44ms in the debug build, then microseconds).
  Tests: `homotopy-model/tests/removal.rs` (the cascade, transitivity, no cascade from an instance, undo,
  workspace, preview = deletion on the examples and, with `HOMOTOPY_TANGENT_FIXTURE=<.hom>`, the fixture).
  **Preview in the signature** (`app/signature/removal_preview.rs`): while the pointer is on the delete button
  of a row being edited, or the button has the focus (it is a focusable `role=button` with `aria-describedby`
  the note), `show` computes `removal_preview(node, 16)`, a few diagrams per step and yielding between
  (`TimeoutFuture(0)`), saying "Checking..." until it is done, then writes to the DOM directly like the
  tooltip: rows have `data-node`; those that go get `data-removal` (tint and strike-through in
  `styles.css`), the nearest row on show that holds some out of sight (a closed folder, a schema with its
  instances closed) gets `data-removal-inside="N"` (a "-N inside" badge), and `.signature__removal-note`
  under the row says "Deletes N more (M out of sight)" (for a folder: "N generators inside and M more
  outside it"), with the names in its `title`. `hide` clears it all (mouse leave, blur, click, and the
  row's `destroy`: a row that goes never sees the pointer leave). `plan` is the pure part (tested natively).
  There is no confirm step, so nothing else lists the names. Nothing re-renders the signature.

- **Settings drawer** (`app/settings.rs`, `components/help_tip.rs`). The drawer shows titles, controls and a
  line of *status* only (`settings__note`: when the last backup was, why backups do not work, inference
  turned off for the session); every explanation is in a `HelpTip` after the title: a "?" button (Tab
  reaches it, `aria-describedby` the popover, `role="tooltip"`) whose popover over the row shows on hover,
  focus and tap and is closed by Esc (`data-open`/`data-dismissed` on the wrapper, written directly). The
  texts are constants (`*_HELP`) of one or two short sentences; **new settings with an explanation add one
  there and keep it short**; keep the facts that matter (Esc ends a search; the limit is per generator and
  covers matching, reflections and inference; a search for one dragged generator has no limit; the
  browser's back button also undoes while *Mouse back and forward buttons undo and redo* is on).
  *Give up on a generator after* is a number box in seconds (`view_number`, `parse_count`: 0 = no limit, negative
  is 0, fractions round down, at most `MAX_GENERATOR_SECONDS` = 3600, not a number goes back to the setting;
  committed on input and put right on change; kept as `generator_seconds` as before). Every slider shows its
  number and updates it on `input`, by writing into its text node (`show_value`; never `set_text_content` on
  the `output`, which would replace the node Yew updates); settings that cost nothing to change (`live`:
  the inference dimension, the contraction step) also take effect on `input`, the 3D ones on `change` only
  (each change rebuilds geometry or the renderer). Ids are `help-<slug>` and `setting-<slug>`.

## Policy: heuristics

False negatives are acceptable: a useful heuristic may miss solutions. But a heuristic must never
produce an invalid result, and it must not be silent about being one: label heuristics in the UI
(say that this is a search that is bounded, or a guess, and what it left out when it can be said),
and make them switchable (a setting, as for inferring instances of schemas). Anything that
cuts a search short (a time limit, a bound on candidates) must leave behind only results that are
valid by themselves, and nothing remembered as if it were complete. The tests for this are of the
form "what it found is among what the whole search finds" (see `a_search_cut_by_the_clock_is_incomplete_not_wrong`).
Where the whole search is itself bounded (the inference stops at `MAX_SUBSTITUTIONS`), a heuristic that
changes what is searched can reach what the whole search does not, so the test is that all it found is
valid (`narrowing_to_a_sheet_only_finds_valid_instances`, for the sheet heuristic).

## Development

- Format with `cargo +nightly fmt` (rustfmt.toml uses nightly options); clippy runs under the
  pinned toolchain with `-D warnings`. Cargo-husky pre-commit/pre-push hooks run test, check,
  clippy and fmt.
- **Hashing** (`homotopy_common::hash`): every map and set is `FastHashMap`/`FastHashSet` (`clippy.toml`
  disallows the std ones), with `Hasher64`, the Fx hash of `rustc-hash` 1.1 with a 64-bit state on every
  target. `rustc_hash::FxHasher` kept a `usize`, 32 bits on wasm32: its `finish` had the top 32 bits zero,
  so hashbrown's 7-bit tag was always 0 (every probe a tag match), and the iteration order differed from
  native, so the searches took other paths and listed options in another order. Measured (Node/V8,
  release, the tangent categories fixture): the four slowest schema inferences went from 2.6-4.8x native to
  1.2x, all 32 from 254 s to 163 s (native 132 s), with the step counts and the options (and their order)
  now the same as native; native hashes are bit-identical to before.
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
- Debug builds expose `window.__homotopy` (`app/debug_hooks.rs`): `sheet(json)` and `sheets()` for choosing
  a sheet over the diagram (see "Sheets"), `dispatch(json)` for an action
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
  to take in the answer (A KB there, B KB back)" for the worker. `window.__homotopy_step_delay_label`
  limits the delay to steps whose label contains it (`"Inferring"`), and `window.__homotopy_infer_grace_ms`
  sets how long past the time for a step the page lets the worker be silent before it ends it (see "The limit is
  hard").

## The history trap (mouse back/forward, Alt+Left/Right)

`app/history_trap.rs` (glue) and `app/history_trap/machine.rs` (the pure state machine, unit-tested
natively), active only while *Mouse back and forward buttons undo and redo* is on. Some mice (a
Logitech under Firefox, found with the now removed input logger) send back/forward to the browser as
navigation commands with no DOM mouse event, no `Browser*` key and no `popstate`, and the page, having
no entries of its own, was simply left. So the page keeps `[back, centre, forward]` in the history
(same URL, `state` `{homotopy: "back"|"centre"|"forward"}`) and sits on the centre; a navigation is
then a `popstate` that is turned into undo (landed on `back`) or redo (`forward`), through the normal
dispatch (refused with the pill shaking while a job runs), and the page goes home with `history.go(+-1)`.
- **Installing** at the first trusted `pointerdown` or `keydown` (not Escape or a lone modifier): browsers
  skip entries made without user activation. `replaceState(back)`, `pushState(centre)`,
  `pushState(forward)`, `go(-1)`. A page reloaded on a `centre` adopts the entries it finds.
- **Forward survives** because nothing pushes after installing: not in `popstate` (restoring is `go`), not
  for the app's own address changes (`model::update_window_url_path` -> `history_trap::url_changed`:
  `replaceState` that keeps the state and puts the marker on the centre; the sentinels get the new URL
  as they are landed on) and not for links to a fragment (the modals use `href="#about"`/`"#"`, and `help()`
  sets `#help`, which push an entry): a capturing click handler and `history_trap::fragment` use
  `location.replace` and put the state back. Any new code that pushes history entries must not (use
  `fragment`/`url_changed`), or the machine finds an unmarked entry and installs again from there.
- **Dedupe**: key/button navigation and the DOM path of `mouse_buttons.rs` both go through the machine
  (`history_trap::mouse`); a press seen by the *other* source in the same direction within 300 ms is
  dropped, two quick presses of one source are two undos. Alt+Left/Right and `BrowserBack/Forward` have no
  keydown handler on purpose: the `popstate` that follows is the only thing that acts (also in text fields:
  the history is global; the browser decides whether the key navigates).
- **Off**: from the centre `go(-1)` once onto the first sentinel and clear its marker, so one more back
  leaves the page; on again installs at once (a gesture has been seen). Setting changes in the middle of a
  move are applied when it lands. Every move has a 1 s timeout that looks where the page is and starts
  again if it is not home (`Wait`/`TimedOut`, with an epoch so that stale ones do nothing).
- **Inherent**: the toolbar's back/forward buttons undo/redo as well while it is on, and the first back from
  the first entry never leaves the page; the setting's note says so. `beforeunload` still guards unsaved work.
- Not tested: Firefox (not available in the sandbox); Chromium with Playwright is.

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
