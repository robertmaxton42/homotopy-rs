//! Doing jobs in a Web Worker, so that the page stays responsive however long they take, and
//! cancelling one is just ending the worker.
//!
//! The worker runs the very same module as the page (`static/worker.js` imports
//! `homotopy_web.js`), whose start function notices that there is no window and sets up
//! [`init`] instead of the app. That keeps the build as it was: one `wasm-bindgen` run, one
//! `.wasm` file, which the browser has in its cache for the worker already. The worker and the
//! page share no memory (there are no threads in WebAssembly on a stable toolchain, and a
//! `SharedArrayBuffer` needs response headers that the hosting does not send), so jobs travel as
//! bytes (see [`homotopy_model::remote`]) in messages:
//!
//! - page to worker: `{kind: "run", job, bytes}` for a job, or `{kind: "layout", job, bytes}` for
//!   a layout (see [`homotopy_model::remote::layout`]), with the bytes transferred rather than
//!   copied; and `{kind: "preload", bytes}`, diagrams for the next job to find (see
//!   [`homotopy_model::remote::Remote::preload`]), which has no answer;
//! - worker to page: `{kind: "ready"}` (said by `static/worker.js`) once the module is running,
//!   then for each job any number of `{kind: "progress", job, label, done, total, limit_ms}`
//!   (`limit_ms`: the most that the step about to be done may take if the job keeps its limits,
//!   `0` for no limit: the page ends a worker that is silent for longer than that and a grace), for some jobs
//!   `{kind: "partial", job, bytes}` (an answer that will do if the job has to be ended: see
//!   [`homotopy_model::job::Job::checkpoint`]), and finally `{kind: "done", job, bytes}`, or
//!   `{kind: "panic", job, message}` if the worker crashed (after which it is of no use, and the
//!   page ends it).
//!
//! There are two workers, running the same code: one for jobs, one for laying out diagrams (see
//! [`crate::layout`]), so that cancelling a job, which ends its worker, does not lose a layout in
//! the making, and a layout does not make a job wait. The one for layout is started with
//! `?layout` after the name of the script, which makes `worker.js` load HiGHS too.
//!
//! The page side is [`Client`], which owns the worker and the [`Remote`] that knows what the worker
//! holds. It is told of what the worker says through a callback, as events tagged with the
//! generation of the worker (the number of workers started before it), so that late messages from
//! one that was ended are ignored.

use std::cell::RefCell;

use homotopy_model::{
    job::Progress,
    remote::{layout, Executor, Remote, Step},
};
use instant::{Duration, Instant};
use js_sys::{Array, Object, Reflect, Uint8Array};
use wasm_bindgen::{closure::Closure, prelude::*, JsCast};
use web_sys::{DedicatedWorkerGlobalScope, MessageEvent, Worker, WorkerOptions, WorkerType};
use yew::Callback;

/// Where the worker's script is. Absolute, as the page may be at any path.
pub const SCRIPT: &str = "/worker.js";

/// The script of the worker that lays diagrams out.
pub const LAYOUT_SCRIPT: &str = "/worker.js?layout";

/// How often the worker reports progress at the most, so that it does not flood the page.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(40);

// The page's side.

/// What the worker said.
#[derive(Debug)]
pub enum Event {
    /// The module is running and can be given jobs.
    Ready,
    Progress {
        job: u64,
        label: String,
        progress: Option<Progress>,
        /// The most that the step that follows may take if the job keeps its limits, in
        /// milliseconds, or `0` if it has none (see [`homotopy_model::job::Job::step_limit`]).
        limit_ms: u32,
    },
    /// What the job would come to if it were ended now: the answer for the page to use if it has
    /// to end the job because it takes too long.
    Partial {
        job: u64,
        bytes: Vec<u8>,
    },
    Done {
        job: u64,
        bytes: Vec<u8>,
    },
    /// The worker crashed (a panic, or an error that it did not catch) or could not start.
    Crashed {
        job: Option<u64>,
        message: String,
    },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Status {
    /// There is a worker, but it is not running yet.
    Starting,
    Ready,
    /// There is no worker, and there will not be one.
    Disabled,
}

/// A running worker, and what it takes to hear from it. Dropping it does not end the worker.
struct Spawned {
    worker: Worker,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_error: Closure<dyn FnMut(web_sys::Event)>,
}

pub struct Client {
    spawned: Option<Spawned>,
    status: Status,
    /// How many workers have been started, which tells them apart.
    generation: u32,
    /// How many times in a row a worker has crashed or failed to start without doing a job.
    failures: u32,
    events: Callback<(u32, Event)>,
    remote: Remote,
    script: &'static str,
}

impl Client {
    /// The most workers that are started one after another without any having done a job
    /// before giving up on them.
    const MAX_FAILURES: u32 = 3;

    #[must_use]
    pub fn new(events: Callback<(u32, Event)>, script: &'static str) -> Self {
        Self {
            spawned: None,
            status: Status::Starting,
            generation: 0,
            failures: 0,
            events,
            remote: Remote::default(),
            script,
        }
    }

    /// Uses another script for the worker, to test what happens when it is not found.
    #[cfg(debug_assertions)]
    pub const fn use_script(&mut self, script: &'static str) {
        self.script = script;
    }

    /// Starts a worker if there is none, and there is no reason to do without. Returns whether
    /// there is a worker (that may not be ready yet).
    pub fn ensure(&mut self) -> bool {
        if self.spawned.is_some() {
            return true;
        }
        if self.failures >= Self::MAX_FAILURES || self.status == Status::Disabled {
            self.status = Status::Disabled;
            return false;
        }
        self.generation += 1;
        let generation = self.generation;
        match spawn(self.script, generation, &self.events) {
            Ok(spawned) => {
                self.spawned = Some(spawned);
                self.status = Status::Starting;
                true
            }
            Err(error) => {
                tracing::error!("Could not start the worker: {:?}", error);
                self.failures = Self::MAX_FAILURES;
                self.status = Status::Disabled;
                false
            }
        }
    }

    /// Ends the worker, if there is one. What it was doing is lost, and nothing it says
    /// afterwards is heard.
    pub fn stop(&mut self) {
        if let Some(spawned) = self.spawned.take() {
            spawned.worker.terminate();
        }
        self.generation += 1;
        self.remote.forget();
        if self.status != Status::Disabled {
            self.status = Status::Starting;
        }
    }

    /// Whether the worker is ready for a job.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.status == Status::Ready && self.spawned.is_some()
    }

    pub const fn remote(&mut self) -> &mut Remote {
        &mut self.remote
    }

    /// Takes note of an event from the worker of the generation, returning whether it was from the
    /// current one.
    pub const fn heard(&mut self, generation: u32, event: &Event) -> bool {
        if generation != self.generation {
            return false;
        }
        match event {
            Event::Ready => {
                self.status = Status::Ready;
            }
            Event::Done { .. } => {
                // It works.
                self.failures = 0;
            }
            Event::Crashed { .. } => {
                self.failures += 1;
            }
            Event::Progress { .. } | Event::Partial { .. } => {}
        }
        true
    }

    /// Gives the worker a job, which is the bytes of a request to the [`Executor`].
    pub fn run(&self, job: u64, bytes: &[u8], fault: &str) -> Result<(), JsValue> {
        self.give("run", job, bytes, fault)
    }

    /// Gives the worker diagrams to hold for the request that follows, which is the bytes
    /// [`Remote::preload`] makes. There is no answer.
    pub fn preload(&self, bytes: &[u8]) -> Result<(), JsValue> {
        self.give("preload", 0, bytes, "")
    }

    /// Gives the worker a layout to do, which is the bytes of a request of
    /// [`homotopy_model::remote::layout`].
    pub fn lay_out(&self, job: u64, bytes: &[u8], fault: &str) -> Result<(), JsValue> {
        self.give("layout", job, bytes, fault)
    }

    fn give(&self, kind: &str, job: u64, bytes: &[u8], fault: &str) -> Result<(), JsValue> {
        let Some(spawned) = &self.spawned else {
            return Err("no worker".into());
        };
        let buffer = Uint8Array::from(bytes);
        let message = Object::new();
        Reflect::set(&message, &"kind".into(), &kind.into())?;
        Reflect::set(&message, &"job".into(), &(job as f64).into())?;
        Reflect::set(&message, &"bytes".into(), &buffer)?;
        if !fault.is_empty() {
            // For testing.
            Reflect::set(&message, &"fault".into(), &fault.into())?;
        }
        let transfer = Array::of1(&buffer.buffer());
        spawned
            .worker
            .post_message_with_transfer(&message, &transfer)
    }

    /// Takes note that the worker failed to do a job (without crashing).
    pub const fn fail(&mut self) {
        self.failures += 1;
    }

    /// Whether there is no worker, and there will not be one.
    #[must_use]
    pub fn is_disabled(&self) -> bool {
        self.status == Status::Disabled && self.spawned.is_none()
    }

    /// Does without workers, for testing.
    #[cfg(debug_assertions)]
    pub fn disable(&mut self) {
        self.stop();
        self.status = Status::Disabled;
    }
}

fn spawn(
    script: &str,
    generation: u32,
    events: &Callback<(u32, Event)>,
) -> Result<Spawned, JsValue> {
    let options = WorkerOptions::new();
    options.set_type(WorkerType::Module);
    let worker = Worker::new_with_options(script, &options)?;

    let sink = events.clone();
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        if let Some(event) = parse_event(&event.data()) {
            sink.emit((generation, event));
        }
    });
    worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

    // The script could not be loaded, or something went wrong that it did not catch.
    let sink = events.clone();
    let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        event.prevent_default();
        let message = event.dyn_ref::<web_sys::ErrorEvent>().map_or_else(
            || "the worker failed".to_owned(),
            web_sys::ErrorEvent::message,
        );
        sink.emit((generation, Event::Crashed { job: None, message }));
    });
    worker.set_onerror(Some(on_error.as_ref().unchecked_ref()));

    Ok(Spawned {
        worker,
        _on_message: on_message,
        _on_error: on_error,
    })
}

fn get(object: &JsValue, key: &str) -> JsValue {
    Reflect::get(object, &key.into()).unwrap_or(JsValue::UNDEFINED)
}

fn parse_event(data: &JsValue) -> Option<Event> {
    let kind = get(data, "kind").as_string()?;
    let job = || get(data, "job").as_f64().map(|job| job as u64);
    Some(match kind.as_str() {
        "ready" => Event::Ready,
        "progress" => Event::Progress {
            job: job()?,
            label: get(data, "label").as_string()?,
            limit_ms: get(data, "limit_ms").as_f64().map_or(0, |ms| ms as u32),
            progress: get(data, "total").as_f64().map(|total| Progress {
                done: get(data, "done").as_f64().unwrap_or(0.0) as usize,
                total: total as usize,
            }),
        },
        "partial" => Event::Partial {
            job: job()?,
            bytes: get(data, "bytes").dyn_into::<Uint8Array>().ok()?.to_vec(),
        },
        "done" => Event::Done {
            job: job()?,
            bytes: get(data, "bytes").dyn_into::<Uint8Array>().ok()?.to_vec(),
        },
        "panic" | "failed" => Event::Crashed {
            job: job(),
            message: get(data, "message")
                .as_string()
                .unwrap_or_else(|| "the worker crashed".to_owned()),
        },
        _ => return None,
    })
}

// The worker's side.

thread_local! {
    static EXECUTOR: RefCell<Executor> = RefCell::default();
    /// The job being done, for the panic hook to say which it was.
    static CURRENT: RefCell<Option<u64>> = const { RefCell::new(None) };
}

fn scope() -> DedicatedWorkerGlobalScope {
    js_sys::global().unchecked_into()
}

fn post(message: &Object, transfer: Option<&JsValue>) {
    let scope = scope();
    let result = match transfer {
        Some(buffer) => scope.post_message_with_transfer(message, &Array::of1(buffer)),
        None => scope.post_message(message),
    };
    if let Err(error) = result {
        tracing::error!("Could not post a message: {:?}", error);
    }
}

fn message(kind: &str) -> Object {
    let message = Object::new();
    Reflect::set(&message, &"kind".into(), &kind.into()).unwrap();
    message
}

/// Sets up the module to be a worker, rather than the app. Called by the start function when
/// there is no window.
pub fn init() {
    std::panic::set_hook(Box::new(|info| {
        let reply = message("panic");
        let job = CURRENT.with(|job| job.try_borrow().ok().and_then(|job| *job));
        if let Some(job) = job {
            Reflect::set(&reply, &"job".into(), &(job as f64).into()).unwrap();
        }
        Reflect::set(&reply, &"message".into(), &info.to_string().into()).unwrap();
        post(&reply, None);
        #[cfg(feature = "console_error_panic_hook")]
        console_error_panic_hook::hook(info);
    }));
}

/// Handles a message from the page: does the job or the layout in it. Called by `static/worker.js`.
#[wasm_bindgen]
pub fn worker_message(data: &JsValue) {
    let kind = get(data, "kind").as_string();
    if kind.as_deref() == Some("preload") {
        // Diagrams for the next request, which nothing is said about.
        if let Ok(bytes) = get(data, "bytes").dyn_into::<Uint8Array>() {
            EXECUTOR.with(|executor| executor.borrow_mut().preload(&bytes.to_vec()));
        }
        return;
    }
    if !matches!(kind.as_deref(), Some("run" | "layout")) {
        return;
    }
    let (Some(job), Ok(bytes)) = (
        get(data, "job").as_f64().map(|job| job as u64),
        get(data, "bytes").dyn_into::<Uint8Array>(),
    ) else {
        return;
    };
    CURRENT.with(|current| *current.borrow_mut() = Some(job));
    let fault = get(data, "fault").as_string().unwrap_or_default();
    assert!(fault != "crash", "the worker was asked to crash");

    let response = if fault == "garble" {
        // The answer is not one.
        vec![0xc1, 0xc1]
    } else if kind.as_deref() == Some("layout") {
        layout::run(&bytes.to_vec(), &mut |delay_ms| spin(delay_ms))
    } else {
        let mut last_report: Option<Instant> = None;
        let mut last_label: Option<&'static str> = None;
        let mut last_limit: u32 = 0;
        EXECUTOR.with(|executor| {
            executor.borrow_mut().run_with_checkpoints(
                &bytes.to_vec(),
                &mut |step: Step| {
                    // A change of what the job is doing, or of how long a step may take, is
                    // said at once, as the next report may be a long step away.
                    if last_label != Some(step.label)
                        || last_limit != step.limit_ms
                        || last_report.is_none_or(|at| at.elapsed() >= PROGRESS_INTERVAL)
                    {
                        last_report = Some(Instant::now());
                        last_label = Some(step.label);
                        last_limit = step.limit_ms;
                        report(job, step);
                    }
                    spin(step.delay_ms);
                },
                &mut |partial: Vec<u8>| {
                    let buffer = Uint8Array::from(partial.as_slice());
                    let reply = message("partial");
                    Reflect::set(&reply, &"job".into(), &(job as f64).into()).unwrap();
                    Reflect::set(&reply, &"bytes".into(), &buffer).unwrap();
                    post(&reply, Some(&buffer.buffer()));
                },
            )
        })
    };
    CURRENT.with(|current| *current.borrow_mut() = None);

    let buffer = Uint8Array::from(response.as_slice());
    let reply = message("done");
    Reflect::set(&reply, &"job".into(), &(job as f64).into()).unwrap();
    Reflect::set(&reply, &"bytes".into(), &buffer).unwrap();
    post(&reply, Some(&buffer.buffer()));
}

fn report(job: u64, step: Step) {
    let reply = message("progress");
    Reflect::set(&reply, &"job".into(), &(job as f64).into()).unwrap();
    Reflect::set(&reply, &"label".into(), &step.label.into()).unwrap();
    Reflect::set(&reply, &"limit_ms".into(), &f64::from(step.limit_ms).into()).unwrap();
    if let Some(Progress { done, total }) = step.progress {
        Reflect::set(&reply, &"done".into(), &(done as f64).into()).unwrap();
        Reflect::set(&reply, &"total".into(), &(total as f64).into()).unwrap();
    }
    post(&reply, None);
}

/// Makes a step take at least this many milliseconds, to have something slow to test with.
fn spin(milliseconds: u32) {
    if milliseconds == 0 {
        return;
    }
    let until = Instant::now() + Duration::from_millis(u64::from(milliseconds));
    while Instant::now() < until {}
}

/// Whether the code is running in a worker rather than on the page.
#[must_use]
pub fn in_worker() -> bool {
    web_sys::window().is_none()
}
