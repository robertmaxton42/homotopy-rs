//! Saving the project in the background, so that a crash, a closed tab or a browser that gives
//! up does not lose what was done since it was last exported (see "Autosave" in `CLAUDE.md`).
//!
//! **When.** [`Plan`] (in `homotopy-model`) decides: some seconds after the project was last
//! changed, if the version of the project (its place in the history) is not one that is safe
//! already (opened, imported, exported, or the one last saved), and never while a job runs. Nothing
//! is saved for a project nobody has changed.
//!
//! **How.** Serialising a project of megabytes takes longer than the page may be blocked for, so a
//! worker of its own does (the same module as the others, see [`crate::worker`]): the page gives
//! it the project the way it gives jobs theirs, as `Work::Serialize`, and gets the `.hom` file
//! back. The worker keeps the signature between saves, so a request is only the workspace; the
//! first one, and one after the signature has changed, has to give it the signature too, which
//! the page packs a diagram at a time ([`Packer`]) and sends ahead in pieces, so that no step of
//! it is long. Where there is no worker (it cannot start, or is given up on) the page does the
//! same itself, a slice at a time, with one step at the end (encoding the file) that cannot be
//! split up; see "Known gaps". A worker of its own, and not the one for jobs, so that a save never
//! makes a job wait or the other way round, and cancelling a job does not lose the save.
//!
//! **Where.** IndexedDB (`autosave/store.js`), one backup for each tab ("session"), replaced as
//! it goes, and the backups of the two tabs before it that are gone. The backup is a `.hom`
//! file like an export, with the time, the title and the size next to it.
//!
//! **Restoring.** When the page is opened at `/` and there is a backup of a tab that is gone,
//! and the person has not exported what it holds, a notice offers it: nothing is restored
//! unasked, and nothing waits for the notice. *Restore* imports the file like any other (a job,
//! behind the busy indicator, in the worker if it is big).

use std::cell::RefCell;

use gloo_timers::future::TimeoutFuture;
use homotopy_core::serialize::Store;
use homotopy_model::{
    autosave::{Packer, Plan},
    proof::ProofState,
    remote::{Answer, Output, Remote, Work},
    serialize::serialize_with,
};
use instant::{Duration, Instant};
use serde::Deserialize;
use wasm_bindgen::prelude::*;
use yew::html::Scope;

use crate::{
    app::{App, Message as AppMessage},
    components::{
        autosave_note,
        toast::{toast, Toast},
    },
    model,
    worker::{self, Client as WorkerClient},
};

/// How long the page works on packing before it lets go of the processor, as for jobs.
const SLICE: Duration = Duration::from_millis(10);

/// How many backups of tabs that are gone are kept.
const KEEP_GONE: u32 = 2;

/// How many saves in a row fail before the person is told.
const TELL_AFTER: u32 = 3;

#[wasm_bindgen(module = "/src/autosave/store.js")]
extern "C" {
    #[wasm_bindgen(catch, js_name = "holdLock")]
    fn hold_lock(session: &str) -> Result<(), JsValue>;

    #[wasm_bindgen(catch)]
    async fn save(
        session: &str,
        title: &str,
        time: f64,
        bytes: &js_sys::Uint8Array,
        keep_gone: u32,
    ) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(catch)]
    async fn probe() -> Result<JsValue, JsValue>;

    #[wasm_bindgen(catch)]
    async fn list(session: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(catch)]
    async fn read(session: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(catch)]
    async fn remove(sessions: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(catch)]
    async fn clear(session: &str, include_own: bool) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(catch, js_name = "markClean")]
    async fn mark_clean(session: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(catch, js_name = "readSetting")]
    fn read_setting() -> Result<Option<String>, JsValue>;

    #[wasm_bindgen(catch, js_name = "writeSetting")]
    fn write_setting(on: bool) -> Result<(), JsValue>;
}

/// What a JavaScript failure says, as text.
fn describe(error: &JsValue) -> String {
    error
        .dyn_ref::<js_sys::Error>()
        .map(|error| String::from(error.message()))
        .or_else(|| error.as_string())
        .unwrap_or_else(|| format!("{error:?}"))
}

/// A backup, as the store describes it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Snapshot {
    pub session: String,
    /// When it was made, in milliseconds since 1970.
    pub time: f64,
    pub title: Option<String>,
    pub size: usize,
}

impl Snapshot {
    /// When it was made, as a person would say it: the time if it was today, and else the date.
    #[must_use]
    pub fn when(&self) -> String {
        let then = js_sys::Date::new(&JsValue::from_f64(self.time));
        let today = js_sys::Date::new_0();
        let same_day = then.get_full_year() == today.get_full_year()
            && then.get_month() == today.get_month()
            && then.get_date() == today.get_date();
        let text = if same_day {
            then.to_locale_time_string("default")
        } else {
            then.to_locale_string("default", &JsValue::UNDEFINED)
        };
        String::from(text)
    }
}

#[derive(Debug)]
pub enum Message {
    /// A wait is over: is it time?
    Tick,
    /// Pack for a while longer (the number is that of the save).
    Slice(u64),
    /// The worker said something.
    Worker(u32, worker::Event),
    /// The backup was written (when, and how big), or not.
    Stored(u64, Result<(f64, usize), String>),
    /// Whether the store can be written to (the number is that of the save).
    Probed(u64, Result<(), String>),
    /// The backups that could be restored, as found when the page was opened.
    Listed(Result<Vec<Snapshot>, String>),
    /// Restore the backup on offer.
    Restore,
    /// The bytes of the backup on offer, to be imported.
    Restored(Result<Vec<u8>, String>),
    /// Delete the backups on offer, which the person does not want.
    Discard,
    /// The setting was changed.
    Setting(bool),
    /// The tab was hidden (switched away from, or on its way to being closed).
    Hidden,
}

/// Where a save is.
enum Stage {
    Idle,
    /// Seeing whether the store can be written to, after a save that failed.
    Probing {
        id: u64,
        proof: Box<ProofState>,
        to_worker: bool,
    },
    /// Packing the signature, to send it ahead (`to_worker`) or to make the file with it here.
    Packing {
        id: u64,
        proof: Box<ProofState>,
        packer: Box<Packer>,
        to_worker: bool,
        /// What was packed in the last slice, to be sent in the next.
        unsent: Option<Box<Store>>,
        /// Whether everything is packed.
        done: bool,
    },
    /// The worker has the project, and will answer with the file.
    Working {
        id: u64,
    },
    /// The file is being written to the store.
    Writing {
        id: u64,
    },
}

/// What the person can be told, and what a test can see.
#[derive(Clone, Debug, Default)]
pub struct Status {
    /// When the last backup was made (milliseconds since 1970), and how big it is.
    pub saved: Option<(f64, usize)>,
    /// Why saving is not working, if it is not.
    pub error: Option<String>,
    pub saving: bool,
    pub enabled: bool,
    pub session: String,
    pub stage: &'static str,
    /// How many backups have been made, and how many times it failed.
    pub saves: u32,
    pub failures: u32,
}

thread_local! {
    static STATUS: RefCell<Status> = RefCell::default();
}

fn publish(change: impl FnOnce(&mut Status)) {
    STATUS.with(|status| change(&mut status.borrow_mut()));
}

/// What there is to say about saving.
#[must_use]
pub fn status() -> Status {
    STATUS.with(|status| status.borrow().clone())
}

/// How long the app has been running, in milliseconds.
fn now() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map_or(0.0, |performance| performance.now())
}

fn epoch_ms() -> f64 {
    js_sys::Date::now()
}

fn new_session() -> String {
    format!(
        "{:08x}{:08x}",
        (js_sys::Math::random() * f64::from(u32::MAX)) as u32,
        (js_sys::Math::random() * f64::from(u32::MAX)) as u32
    )
}

pub struct Autosave {
    link: Scope<App>,
    plan: Plan<usize>,
    /// Tells this tab's backup from others'.
    session: String,
    worker: WorkerClient,
    stage: Stage,
    next_id: u64,
    /// Whether a [`Message::Tick`] is on its way.
    ticking: bool,
    /// Listens for the tab being hidden.
    visibility: Option<(web_sys::Document, Closure<dyn FnMut()>)>,
    /// For the log: when the save began, and how long the page spent on it.
    began: Instant,
    page: Duration,
    /// How many slices the page's work was in, and how long the longest was.
    slices: u32,
    longest_slice: Duration,
    /// What was in the way of the save that is in progress, to say when it is over.
    title: Option<String>,
    version: usize,
    /// The backup on offer to restore, if any.
    offer: Option<Snapshot>,
    /// Whether the person was told that saving does not work.
    told: bool,
    /// Whether the next save is made on the page, as the worker answered with nonsense.
    page_next: bool,
}

impl Autosave {
    /// Starts saving for a project at the given version of the history. `can_offer` is whether to
    /// look for a backup to offer when the page opens (it was not opened to a project's own address).
    pub fn new(link: &Scope<App>, version: usize, can_offer: bool) -> Self {
        let session = new_session();
        if let Err(error) = hold_lock(&session) {
            tracing::warn!(
                "Autosave could not tell other tabs it is open: {}",
                describe(&error)
            );
        }
        // Turned off in an earlier visit.
        let enabled = !matches!(read_setting(), Ok(Some(setting)) if setting == "off");
        let mut plan = Plan::new(version, now());
        #[cfg(debug_assertions)]
        if let Some(idle) = crate::app::debug_hooks::autosave_idle_ms() {
            plan = plan.with_timing(idle, idle * 12.0);
        }
        plan.set_enabled(enabled);

        let events = link.callback(|(generation, event)| {
            AppMessage::Autosave(Message::Worker(generation, event))
        });
        #[allow(unused_mut)] // It is only changed by the debug hooks.
        let mut worker = WorkerClient::new(events, worker::SCRIPT);
        #[cfg(debug_assertions)]
        match crate::app::debug_hooks::autosave_worker_mode().as_str() {
            "off" => worker.disable(),
            "broken" => worker.use_script("/no-such-worker.js"),
            _ => {}
        }

        publish(|status| {
            status.enabled = enabled;
            status.session.clone_from(&session);
            status.stage = "idle";
        });
        if enabled && can_offer {
            // Not before the page has had its time to start.
            let own = session.clone();
            link.send_future(async move {
                TimeoutFuture::new(1000).await;
                AppMessage::Autosave(Message::Listed(
                    list(&own)
                        .await
                        .map_err(|error| describe(&error))
                        .and_then(|json| {
                            serde_json::from_str(&json.as_string().unwrap_or_default())
                                .map_err(|error| error.to_string())
                        }),
                ))
            });
        }
        let visibility = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| {
                let link = link.clone();
                let listener = {
                    let document = document.clone();
                    Closure::<dyn FnMut()>::new(move || {
                        if document.hidden() {
                            link.send_message(AppMessage::Autosave(Message::Hidden));
                        }
                    })
                };
                document
                    .add_event_listener_with_callback(
                        "visibilitychange",
                        listener.as_ref().unchecked_ref(),
                    )
                    .ok()?;
                Some((document, listener))
            });
        Self {
            visibility,
            link: link.clone(),
            plan,
            session,
            worker,
            stage: Stage::Idle,
            next_id: 0,
            ticking: false,
            began: Instant::now(),
            page: Duration::ZERO,
            slices: 0,
            longest_slice: Duration::ZERO,
            title: None,
            version,
            offer: None,
            told: false,
            page_next: false,
        }
    }

    /// Stops listening: the app is going away.
    pub fn destroy(&mut self) {
        if let Some((document, listener)) = self.visibility.take() {
            document
                .remove_event_listener_with_callback(
                    "visibilitychange",
                    listener.as_ref().unchecked_ref(),
                )
                .ok();
        }
    }

    /// Whether saving was turned off in an earlier visit.
    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.plan.enabled()
    }

    /// The backup that is on offer to restore.
    #[must_use]
    pub const fn offer(&self) -> Option<&Snapshot> {
        self.offer.as_ref()
    }

    /// Takes note of the version the project is at, which may have changed.
    pub fn observe(&mut self, version: usize) {
        if version != self.version {
            self.version = version;
            self.plan.observe(version, now());
            self.schedule();
        }
    }

    /// The project at this version is safe elsewhere (it was imported): there is nothing to protect.
    pub fn clean(&mut self, version: usize) {
        self.plan.mark_clean(version);
    }

    /// The project at this version was exported, which supersedes the backup of this tab: it is
    /// not offered when the page is next opened, and nothing is saved until the project changes.
    pub fn exported(&mut self, version: usize) {
        self.plan.mark_clean(version);
        if !self.plan.enabled() {
            return;
        }
        let session = self.session.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(error) = mark_clean(&session).await {
                tracing::debug!(
                    "Autosave could not mark the backup as exported: {}",
                    describe(&error)
                );
            }
        });
    }

    /// Makes sure that there is a wait going on until the project is to be saved, if it is.
    fn schedule(&mut self) {
        if self.ticking {
            return;
        }
        if let Some(wait) = self.plan.wait(now()) {
            self.later(wait);
        }
    }

    fn later(&mut self, milliseconds: f64) {
        if self.ticking {
            return;
        }
        self.ticking = true;
        let wait = milliseconds.clamp(0.0, 3.6e6).ceil() as u32;
        self.link.send_future(async move {
            TimeoutFuture::new(wait).await;
            AppMessage::Autosave(Message::Tick)
        });
    }

    const fn stage_name(&self) -> &'static str {
        match self.stage {
            Stage::Idle => "idle",
            Stage::Probing { .. } => "probing",
            Stage::Packing { .. } => "packing",
            Stage::Working { .. } => "working",
            Stage::Writing { .. } => "writing",
        }
    }

    /// Whether the app should render again.
    pub fn update(&mut self, message: Message, state: &model::State, job_running: bool) -> bool {
        let rerender = match message {
            Message::Tick => {
                self.ticking = false;
                self.tick(state, job_running);
                false
            }
            Message::Slice(id) => {
                self.slice(id);
                false
            }
            Message::Worker(generation, event) => {
                self.worker_said(generation, event);
                false
            }
            Message::Probed(id, result) => {
                self.probed(id, result);
                false
            }
            Message::Stored(id, result) => {
                self.stored(id, result);
                false
            }
            Message::Listed(result) => self.listed(result),
            Message::Restore => self.restore(),
            Message::Restored(result) => {
                Self::restored(&self.link, result);
                false
            }
            Message::Discard => self.discard(),
            Message::Setting(on) => self.set_enabled(on),
            Message::Hidden => {
                // Not after the wait: it may not get the chance.
                self.plan.hurry();
                self.ticking = false;
                self.schedule();
                false
            }
        };
        publish(|status| {
            status.stage = self.stage_name();
            status.saving = !matches!(self.stage, Stage::Idle);
            status.failures = self.plan.failures();
        });
        autosave_note::show(!matches!(self.stage, Stage::Idle));
        rerender
    }

    /// The backups that could be restored are known: the newest is offered.
    fn listed(&mut self, result: Result<Vec<Snapshot>, String>) -> bool {
        match result {
            Ok(found) => {
                self.offer = found.into_iter().next();
                if let Some(offer) = &self.offer {
                    tracing::info!(
                        "Autosave has a backup to restore from {} ({} bytes)",
                        offer.when(),
                        offer.size
                    );
                }
                self.offer.is_some()
            }
            Err(error) => {
                tracing::warn!("Autosave could not look for backups: {error}");
                false
            }
        }
    }

    /// *Restore* was pressed: reads the backup, to import it. The notice goes at once, so that
    /// it cannot be pressed twice.
    fn restore(&mut self) -> bool {
        let Some(offer) = self.offer.take() else {
            return false;
        };
        self.link.send_future(async move {
            AppMessage::Autosave(Message::Restored(
                read(&offer.session)
                    .await
                    .map_err(|error| describe(&error))
                    .and_then(|bytes| {
                        bytes
                            .dyn_into::<js_sys::Uint8Array>()
                            .map(|bytes| bytes.to_vec())
                            .map_err(|_gone| "the backup is gone".to_owned())
                    }),
            ))
        });
        true
    }

    fn restored(link: &Scope<App>, result: Result<Vec<u8>, String>) {
        match result {
            Ok(bytes) => {
                // Like any other import: a job, behind the busy indicator.
                link.send_message(AppMessage::Dispatch(model::Action::Proof(
                    model::proof::Action::ImportProof(bytes.into()),
                )));
            }
            Err(error) => {
                tracing::warn!("Autosave could not read the backup: {error}");
                toast(Toast::error(format!(
                    "The backup could not be read: {error}"
                )));
            }
        }
    }

    /// *Discard* was pressed: the backups of tabs that are gone are deleted.
    fn discard(&mut self) -> bool {
        let session = self.session.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(error) = clear(&session, false).await {
                tracing::warn!(
                    "Autosave could not delete the backups: {}",
                    describe(&error)
                );
            }
        });
        self.offer.take().is_some()
    }

    /// The setting was changed. Turning saving off deletes the backups, which are what the person
    /// does not want kept; it is remembered for the next visit.
    fn set_enabled(&mut self, on: bool) -> bool {
        if on == self.plan.enabled() {
            return false;
        }
        if let Err(error) = write_setting(on) {
            tracing::debug!(
                "The autosave setting is not remembered: {}",
                describe(&error)
            );
        }
        self.plan.set_enabled(on);
        publish(|status| {
            status.enabled = on;
            status.error = None;
        });
        if on {
            self.schedule();
            return false;
        }
        // What is on its way is dropped; the worker is not needed.
        if !matches!(self.stage, Stage::Idle) {
            self.plan.abandon();
            self.stage = Stage::Idle;
        }
        self.worker.stop();
        self.plan.forget_saved();
        let session = self.session.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(error) = clear(&session, true).await {
                tracing::warn!(
                    "Autosave could not delete the backups: {}",
                    describe(&error)
                );
            }
        });
        let had_offer = self.offer.take().is_some();
        publish(|status| status.saved = None);
        had_offer
    }

    /// A wait is over: save now if it is the time, and a good one.
    fn tick(&mut self, state: &model::State, job_running: bool) {
        if !matches!(self.stage, Stage::Idle) {
            return;
        }
        match self.plan.wait(now()) {
            None => {}
            Some(wait) if wait > 0.0 => self.later(wait),
            // Never while a job is running: look again when it may be over.
            Some(_) if job_running => self.later(1000.0),
            Some(_) => self.begin(state),
        }
    }

    /// Starts to save the project as it is.
    fn begin(&mut self, state: &model::State) {
        // Where to do it: in the worker if there is one and it is ready, and otherwise here.
        let to_worker = if self.worker.is_disabled() || self.page_next {
            false
        } else {
            self.worker.ensure();
            if self.worker.is_disabled() {
                false
            } else if self.worker.is_ready() {
                true
            } else {
                // It is starting. Look again soon.
                self.later(500.0);
                return;
            }
        };

        self.page_next = false;
        let proof: &ProofState = state.proof();
        self.plan.start();
        self.next_id += 1;
        let id = self.next_id;
        self.began = Instant::now();
        self.page = Duration::ZERO;
        self.slices = 0;
        self.longest_slice = Duration::ZERO;
        self.title.clone_from(&proof.metadata.title);
        let proof = Box::new(proof.clone());

        if self.plan.failures() == 0 {
            self.proceed(id, proof, to_worker);
            return;
        }
        // It did not work last time. Before the work of making the file, see whether the store
        // is there to put it in: if it is not, it fails again at once, for no cost.
        self.stage = Stage::Probing {
            id,
            proof,
            to_worker,
        };
        self.link.send_future(async move {
            AppMessage::Autosave(Message::Probed(
                id,
                probe().await.map(|_| ()).map_err(|error| describe(&error)),
            ))
        });
    }

    /// Makes the file: gives the worker the project, which is all of it if the worker holds the
    /// signature, and otherwise the signature is packed first.
    fn proceed(&mut self, id: u64, proof: Box<ProofState>, to_worker: bool) {
        if to_worker && self.worker.remote().holds(&proof.signature) {
            // Only the workspace is to be sent.
            let began = Instant::now();
            let bytes = self
                .worker
                .remote()
                .request(&proof, Work::Serialize, 0, false);
            self.note_slice(began.elapsed());
            self.send(id, &bytes);
            return;
        }
        self.stage = Stage::Packing {
            id,
            packer: Box::new(Packer::new(&proof.signature)),
            proof,
            to_worker,
            unsent: None,
            done: false,
        };
        self.link
            .send_message(AppMessage::Autosave(Message::Slice(id)));
    }

    /// Gives the worker the request.
    fn send(&mut self, id: u64, bytes: &[u8]) {
        #[cfg(debug_assertions)]
        let fault = crate::app::debug_hooks::autosave_worker_mode();
        #[cfg(not(debug_assertions))]
        let fault = String::new();
        if let Err(error) = self.worker.run(id, bytes, &fault) {
            self.worker.remote().forget();
            self.worker_failed(&format!("could not give it the project: {error:?}"));
            return;
        }
        self.stage = Stage::Working { id };
    }

    /// Packs for a slice, and then does what follows if that is all of it.
    fn slice(&mut self, id: u64) {
        let began = Instant::now();
        let deadline = began + SLICE;
        let Stage::Packing {
            id: current,
            packer,
            to_worker,
            unsent,
            done,
            ..
        } = &mut self.stage
        else {
            return;
        };
        if *current != id {
            return;
        }
        if let Some(piece) = unsent.take() {
            // What was packed goes ahead, so that the request does not have to hold it. It is
            // not done in the same slice as the packing, so that the page can respond in between.
            if let Err(error) = self.worker.preload(&Remote::preload(&piece)) {
                self.worker.remote().forget();
                self.worker_failed(&format!("could not give it the project: {error:?}"));
                return;
            }
        } else if *done {
            self.finish_packing(id, began);
            return;
        } else {
            while Instant::now() < deadline {
                if packer.step() {
                    *done = true;
                    break;
                }
            }
            if *to_worker {
                let piece = packer.take();
                if !piece.is_empty() {
                    *unsent = Some(Box::new(piece));
                }
            }
        }
        self.note_slice(began.elapsed());
        self.link.send_future(async move {
            TimeoutFuture::new(0).await;
            AppMessage::Autosave(Message::Slice(id))
        });
    }

    fn probed(&mut self, id: u64, result: Result<(), String>) {
        if !matches!(self.stage, Stage::Probing { id: current, .. } if current == id) {
            return;
        }
        let Stage::Probing {
            proof, to_worker, ..
        } = std::mem::replace(&mut self.stage, Stage::Idle)
        else {
            return;
        };
        match result {
            Ok(()) => self.proceed(id, proof, to_worker),
            Err(error) => self.failed(&error),
        }
    }

    /// Takes note of the time a slice of the page's work took.
    fn note_slice(&mut self, took: Duration) {
        self.page += took;
        self.slices += 1;
        self.longest_slice = self.longest_slice.max(took);
    }

    /// Everything is packed: gives the worker the rest, or makes the file here.
    fn finish_packing(&mut self, id: u64, began: Instant) {
        let Stage::Packing {
            proof,
            packer,
            to_worker,
            ..
        } = std::mem::replace(&mut self.stage, Stage::Idle)
        else {
            return;
        };
        if to_worker {
            let bytes = self.worker.remote().request_with(
                packer.into_store(),
                &proof,
                Work::Serialize,
                0,
                false,
            );
            self.note_slice(began.elapsed());
            self.send(id, &bytes);
        } else {
            // The one step that cannot be split up.
            let file = serialize_with(
                packer.into_store(),
                proof.signature.clone(),
                proof.workspace.clone(),
                proof.metadata.clone(),
            );
            self.note_slice(began.elapsed());
            self.write(id, &file);
        }
    }

    fn worker_said(&mut self, generation: u32, event: worker::Event) {
        if !self.worker.heard(generation, &event) {
            return;
        }
        match event {
            worker::Event::Ready => tracing::info!("The autosave worker is ready"),
            worker::Event::Progress { .. }
            | worker::Event::Partial { .. }
            | worker::Event::Options { .. } => {}
            worker::Event::Done { job, bytes } => self.worker_done(job, &bytes),
            worker::Event::Crashed { job, message } => {
                tracing::error!("The autosave worker crashed: {message}");
                self.worker.stop();
                let mine = match self.stage {
                    Stage::Working { id } => job.is_none_or(|job| job == id),
                    Stage::Packing { to_worker, .. } => to_worker,
                    _ => false,
                };
                self.worker.ensure();
                if mine {
                    self.worker_failed(&format!("it crashed: {message}"));
                }
            }
        }
    }

    /// The worker answered for the save that is waiting for it.
    fn worker_done(&mut self, job: u64, bytes: &[u8]) {
        if !matches!(self.stage, Stage::Working { id } if id == job) {
            return;
        }
        let began = Instant::now();
        let answer = self.worker.remote().response(bytes);
        self.note_slice(began.elapsed());
        match answer {
            Ok(Answer::Done {
                output: Output::Serialized(file),
                worked_ms,
            }) => {
                tracing::info!("Autosave: the worker took {worked_ms:.1}ms");
                self.write(job, &file);
            }
            Ok(Answer::NeedsSignature) => {
                // It was restarted, and does not hold the signature. The next save sends it.
                self.stage = Stage::Idle;
                self.plan.abandon();
                self.later(500.0);
            }
            Ok(Answer::Done { .. }) => self.worker_failed("it answered with something else"),
            Err(message) => {
                // It did not crash, but what it says is of no use. Doing it again would just as
                // likely come to the same, so the page makes this one.
                self.worker.fail();
                self.worker.stop();
                self.worker.ensure();
                self.page_next = true;
                self.worker_failed(&format!("it failed: {message}"));
            }
        }
    }

    /// The file is made: writes it.
    fn write(&mut self, id: u64, file: &[u8]) {
        self.stage = Stage::Writing { id };
        let began = Instant::now();
        let array = js_sys::Uint8Array::from(file);
        self.note_slice(began.elapsed());
        let session = self.session.clone();
        let title = self.title.clone().unwrap_or_default();
        let (time, size) = (epoch_ms(), file.len());
        self.link.send_future(async move {
            let result = save(&session, &title, time, &array, KEEP_GONE)
                .await
                .map(|_| (time, size))
                .map_err(|error| describe(&error));
            AppMessage::Autosave(Message::Stored(id, result))
        });
    }

    fn stored(&mut self, id: u64, result: Result<(f64, usize), String>) {
        if !matches!(self.stage, Stage::Writing { id: current } if current == id) {
            return;
        }
        self.stage = Stage::Idle;
        match result {
            Ok(saved) => {
                self.plan.finish(true, now());
                self.told = false;
                tracing::info!(
                    "Autosaved the project ({} KB) in {:.0}ms: {:.0}ms on the page, in {} slices \
                     (longest {:.1}ms)",
                    saved.1 / 1024,
                    self.began.elapsed().as_secs_f64() * 1000.0,
                    self.page.as_secs_f64() * 1000.0,
                    self.slices,
                    self.longest_slice.as_secs_f64() * 1000.0
                );
                publish(|status| {
                    status.error = None;
                    status.saved = Some(saved);
                    status.saves += 1;
                });
                self.schedule();
            }
            Err(error) => self.failed(&error),
        }
    }

    /// The worker did not do the save: that is not for the store to answer for, so it is not a
    /// failure to save that backs off. It is tried again soon, and after a few of them the worker
    /// is given up on and the page does it (see [`WorkerClient`]).
    fn worker_failed(&mut self, reason: &str) {
        tracing::warn!("Autosave: the worker failed: {reason}");
        self.stage = Stage::Idle;
        self.plan.abandon();
        self.later(500.0);
    }

    /// A save did not work, for the reason: it is tried again later, and the person is told if
    /// it keeps not working.
    fn failed(&mut self, reason: &str) {
        tracing::warn!("Autosave failed: {reason}");
        self.stage = Stage::Idle;
        self.plan.finish(false, now());
        publish(|status| status.error = Some(reason.to_owned()));
        if self.plan.failures() >= TELL_AFTER && !self.told {
            self.told = true;
            toast(Toast::warn(format!(
                "Autosave is not working: {}. Your work is not being backed up: export it from the \
                 Project drawer to keep it.",
                reason.trim_end_matches('.')
            )));
        }
        self.schedule();
    }
}
