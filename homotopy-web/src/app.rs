use boundary::BoundaryPreview;
use futures::future::select;
use gloo_timers::future::TimeoutFuture;
use homotopy_model::{job::Job, remote::Answer};
use instant::{Duration, Instant};
use mouse_buttons::MouseButtons;
use settings::{AppSettings, AppSettingsKey, AppSettingsMsg};
use sidebar::Sidebar;
use signature_stylesheet::SignatureStylesheet;
use wasm_bindgen::{closure::Closure, JsCast};
use workspace::WorkspaceView;
use yew::prelude::*;

use self::diagram_gl::GlViewControl;
use crate::{
    autosave::{self, Autosave},
    components::{
        autosave_note::AutosaveNote,
        busy::{self, BusyIndicator},
        delta::CallbackIdx,
        icon::{Icon, IconSize},
        layout_note::LayoutNote,
        modal::Modal,
        panzoom::PanZoom,
        restore_notice::RestoreNotice,
        toast::{toast, Toast, ToasterComponent},
    },
    model,
    worker::{self, Client as WorkerClient},
};

#[allow(clippy::mem_forget, clippy::future_not_send)] // due to wasm_bindgen
pub mod account;
mod attach;
mod boundary;
#[cfg(any(debug_assertions, feature = "show_debug_panel"))]
mod debug;
#[cfg(debug_assertions)]
pub mod debug_hooks;
pub mod diagram_gl;
mod diagram_svg;
mod image_export;
mod info;
mod keybindings;
mod mouse_buttons;
mod project;
pub mod settings;
mod sidebar;
mod signature;
mod signature_stylesheet;
mod stash;
mod tex;
mod tooltip;
mod workspace;

/// How long the app works on a job before it lets the page respond to input. Input is only
/// handled between such slices, so this is how long the page can be unresponsive for, at the
/// least; and a step that cannot be split up takes as long as it takes.
const SLICE: Duration = Duration::from_millis(10);

/// How long after the app starts the worker is started, so as not to compete with the page for
/// the processor while it loads.
const WORKER_DELAY: u32 = 500;

pub enum Message {
    /// Do something to the project, or change what is selected.
    Dispatch(model::Action),
    /// Work on the running job for another slice. The number is that of the job, so that the
    /// slice of a job that was cancelled in the meantime does nothing.
    Slice(u64),
    /// Cancel the running job, if there is one.
    Cancel,
    /// `Esc` was pressed (while focus was in a text field if the flag is set).
    Escape(bool),
    /// Start the worker, if it is not running.
    StartWorker,
    /// The worker (of the given generation) said something.
    Worker(u32, worker::Event),
    /// Something about saving the project in the background (see [`crate::autosave`]).
    Autosave(autosave::Message),
}

/// A task that the worker is doing.
struct Remote {
    /// What the worker is doing, as it last said.
    label: String,
    sent: Instant,
    /// For the log: how long it took to prepare the request, and how big it was.
    encode_ms: f64,
    bytes_out: usize,
    /// Whether the worker was asked again, with the signature.
    retried: bool,
}

/// A task that is being worked on a slice at a time. It works on a copy of what it needs, and
/// nothing is changed until it is done, so cancelling it is just forgetting it.
struct Running {
    /// Tells this job from any other, past or future.
    id: u64,
    task: model::Task,
    /// The output of the task once it is done, until it is committed.
    finished: Option<model::TaskOutput>,
    /// Set if the worker is doing the task, rather than the app a slice at a time.
    remote: Option<Remote>,
    resets_panzoom: bool,
    /// For the log.
    started: Instant,
    steps: usize,
    longest_step: Duration,
}

impl Running {
    /// What the job is doing.
    fn label(&self) -> String {
        self.remote.as_ref().map_or_else(
            || self.task.label().to_owned(),
            |remote| remote.label.clone(),
        )
    }
}

pub struct App {
    state: model::State,
    /// The job in progress, if any. While there is one, actions that would change the project
    /// are refused (see [`model::Action::allowed_while_busy`]).
    job: Option<Running>,
    next_job: u64,
    /// The worker that does the jobs that would block the page, if it can.
    worker: WorkerClient,
    signature_stylesheet: SignatureStylesheet,
    before_unload: Option<Closure<dyn FnMut(web_sys::BeforeUnloadEvent)>>,
    // Hold onto the listeners so that they are dropped when the app is destroyed
    _mouse_buttons: MouseButtons,
    escape: Option<Closure<dyn FnMut(web_sys::KeyboardEvent)>>,
    /// Saving the project in the background, in case the tab is lost.
    autosave: Autosave,
    autosave_setting: Vec<CallbackIdx>,
}

impl Component for App {
    type Message = Message;
    type Properties = ();

    #[allow(unused_variables)]
    fn create(ctx: &Context<Self>) -> Self {
        let state = model::State::default();
        // Install the signature stylesheet
        let signature_stylesheet = SignatureStylesheet::new();
        signature_stylesheet.mount();

        let opened_from_url = Self::load_project_from_url_path(ctx);

        #[cfg(debug_assertions)]
        debug_hooks::install(ctx.link());

        #[allow(unused_mut)] // It is only changed by the debug hooks.
        let mut worker = WorkerClient::new(
            ctx.link()
                .callback(|(generation, event)| Message::Worker(generation, event)),
            worker::SCRIPT,
        );
        #[cfg(debug_assertions)]
        match debug_hooks::worker_mode().as_str() {
            "off" => worker.disable(),
            "broken" => worker.use_script("/no-such-worker.js"),
            _ => {}
        }
        ctx.link().send_future(async {
            TimeoutFuture::new(WORKER_DELAY).await;
            Message::StartWorker
        });

        let autosave = Autosave::new(ctx.link(), state.history.position(), !opened_from_url);
        // The one setting that is kept from one visit to the next.
        AppSettings::set_autosave(autosave.enabled());
        let autosave_setting = AppSettings::subscribe(
            &[AppSettingsKey::autosave],
            ctx.link().batch_callback(|msg| match msg {
                AppSettingsMsg::autosave(on) => {
                    Some(Message::Autosave(autosave::Message::Setting(on)))
                }
                _ => None,
            }),
        );

        Self {
            autosave,
            autosave_setting,
            state,
            job: None,
            next_job: 0,
            worker,
            signature_stylesheet,
            before_unload: None,
            _mouse_buttons: MouseButtons::install(ctx.link().callback(Message::Dispatch)),
            escape: Some(Self::install_escape(ctx)),
        }
    }

    fn update(&mut self, ctx: &Context<Self>, msg: Self::Message) -> bool {
        let rerender = self.handle(ctx, msg);
        // Cheap, and does nothing unless the project has changed.
        self.autosave.observe(self.state.history.position());
        #[cfg(debug_assertions)]
        debug_hooks::publish(&self.state, self.job.as_ref().map(Running::label));
        rerender
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        Self::render(ctx, &self.state, self.autosave.offer())
    }

    fn destroy(&mut self, _ctx: &Context<Self>) {
        AppSettings::unsubscribe(&[AppSettingsKey::autosave], &self.autosave_setting);
        self.autosave.destroy();
        self.signature_stylesheet.unmount();
        if let Some(escape) = self.escape.take() {
            web_sys::window()
                .unwrap()
                .remove_event_listener_with_callback_and_bool(
                    "keydown",
                    escape.as_ref().unchecked_ref(),
                    true,
                )
                .unwrap();
        }
    }
}

impl App {
    fn handle(&mut self, ctx: &Context<Self>, msg: Message) -> bool {
        match msg {
            Message::Dispatch(action) => self.dispatch(ctx, action),
            Message::Slice(id) => match self.job.take() {
                Some(job) if job.id == id => self.work(ctx, job),
                // The job was cancelled (and another may have started since).
                other => {
                    self.job = other;
                    false
                }
            },
            Message::Cancel => self.cancel(),
            Message::StartWorker => {
                self.worker.ensure();
                crate::layout::start();
                false
            }
            Message::Worker(generation, event) => self.worker_said(ctx, generation, event),
            Message::Autosave(msg) => self.autosave.update(msg, &self.state, self.job.is_some()),
            Message::Escape(in_text_field) => {
                if self.job.is_some() {
                    self.cancel()
                } else if !in_text_field && self.state.options.is_some() {
                    self.dispatch(ctx, model::Action::ClearSelections)
                } else {
                    false
                }
            }
        }
    }

    /// Handles an action: does it now, or starts the job that does it.
    fn dispatch(&mut self, ctx: &Context<Self>, action: model::Action) -> bool {
        tracing::info!("Received action: {:?}", action);
        crate::panic::note_action(&action);

        if self.refuse_while_busy(&action) {
            return false;
        }

        // Determine if the action is experimental and warn the user.
        if action.is_experimental() {
            toast(Toast::warn(
                "This feature is experimental, proceed with caution",
            ));
        }

        // Determine if the action needs to reset the panzoom
        // but do not reset it until we have performed the action.
        let resets_panzoom = self.state.resets_panzoom(&action);
        let exporting = matches!(action, model::Action::ExportProof);

        let started = Instant::now();
        let result = self.state.start(action);
        tracing::info!(
            "State update took {}ms.",
            started.elapsed().as_secs_f64() * 1000.0
        );

        match result {
            Ok(model::Started::Done(changed)) => {
                if exporting {
                    // What is on disk now does not need a backup.
                    self.autosave.exported(self.state.history.position());
                }
                self.updated(Ok(changed), resets_panzoom)
            }
            Ok(model::Started::Running(task)) => self.start_job(ctx, task, resets_panzoom),
            Err(error) => self.updated(Err(error), resets_panzoom),
        }
    }

    /// Refuses rather than queues what would change the project while a job works on it: the
    /// person asking for it cannot have known what it would come on top of, and cancelling the
    /// job must not replay anything. Returns whether the action was refused.
    fn refuse_while_busy(&self, action: &model::Action) -> bool {
        let Some(job) = &self.job else {
            return false;
        };
        if action.allowed_while_busy() {
            return false;
        }
        tracing::info!(
            "Refused action while busy ({}): {:?}",
            job.task.label(),
            action
        );
        busy::refused();
        true
    }

    /// Starts the job for a task that an action began.
    fn start_job(&mut self, ctx: &Context<Self>, task: model::Task, resets_panzoom: bool) -> bool {
        // What inferring is for was changed to start it (the replacement chosen), and that is
        // shown while the job runs.
        let changed_at_start = matches!(task, model::Task::Infer(_));
        self.next_job += 1;
        let job = Running {
            id: self.next_job,
            task,
            finished: None,
            remote: None,
            resets_panzoom,
            started: Instant::now(),
            steps: 0,
            longest_step: Duration::ZERO,
        };
        self.begin(ctx, job) || changed_at_start
    }

    /// What follows from the project having been updated, or not.
    fn updated(&mut self, result: Result<bool, model::ModelError>, resets_panzoom: bool) -> bool {
        homotopy_core::collect_garbage();

        if matches!(result, Ok(true)) {
            // What a crash from here on saves (see `panic`).
            crate::panic::remember(self.state.proof());

            // What a file held is on disk, and needs no backup until it is changed.
            if self.state.history.at_import() {
                self.autosave.clean(self.state.history.position());
            }

            // Instantiations in progress may be of what is no longer there.
            let signature = self.state.history.proof().signature.clone();
            self.state.drafts.retain_valid(&signature);

            if resets_panzoom {
                PanZoom::reset();
                GlViewControl::reset();
            }

            if self.before_unload.is_none() {
                self.install_unload_hook();
            }

            self.signature_stylesheet
                .update(self.state.proof().signature.clone());
        } else if let Err(error) = result {
            tracing::error!("Error occured: {}", error);
            toast(Toast::error(error.to_string()));
        }

        true
    }

    /// Starts on a job: the worker is given it if that is worth it and the worker is there,
    /// and otherwise the app works on it a slice at a time.
    fn begin(&mut self, ctx: &Context<Self>, mut job: Running) -> bool {
        if self.send_to_worker(&mut job, false) {
            Self::show(&job);
            self.job = Some(job);
            return false;
        }
        if job.task.next_step_may_block() {
            // A step that cannot be split up comes first, and the page cannot show
            // anything while it is on. So get the indicator on the page, and let the
            // browser paint it, before starting.
            self.wait(ctx, job)
        } else {
            self.work(ctx, job)
        }
    }

    /// Gives the job to the worker, if it is one for the worker and the worker is ready.
    /// `with_signature` sends the signature of the proof whether or not the worker holds it.
    fn send_to_worker(&mut self, job: &mut Running, with_signature: bool) -> bool {
        if !self.worker.is_ready() {
            return false;
        }
        let proof: &model::proof::ProofState = self.state.proof();
        let Some((proof, work)) = job.task.offer(proof) else {
            return false;
        };
        let encoding = Instant::now();
        #[cfg(debug_assertions)]
        let (delay, fault) = (debug_hooks::step_delay_ms(), debug_hooks::worker_mode());
        #[cfg(not(debug_assertions))]
        let (delay, fault) = (0, String::new());
        let bytes = self
            .worker
            .remote()
            .request(proof, work, delay, with_signature);
        let encode_ms = encoding.elapsed().as_secs_f64() * 1000.0;
        if let Err(error) = self.worker.run(job.id, &bytes, &fault) {
            tracing::error!("Could not give the job to the worker: {:?}", error);
            self.worker.remote().forget();
            return false;
        }
        job.remote = Some(Remote {
            label: job.task.label().to_owned(),
            sent: Instant::now(),
            encode_ms,
            bytes_out: bytes.len(),
            retried: with_signature,
        });
        job.started = Instant::now();
        true
    }

    /// Takes note of what the worker said.
    fn worker_said(&mut self, ctx: &Context<Self>, generation: u32, event: worker::Event) -> bool {
        if !self.worker.heard(generation, &event) {
            return false;
        }
        match event {
            worker::Event::Ready => {
                tracing::info!("The worker is ready");
                false
            }
            worker::Event::Progress {
                job,
                label,
                progress,
            } => {
                if let Some(running) = self.job.as_mut().filter(|running| running.id == job) {
                    if let Some(remote) = &mut running.remote {
                        remote.label = label;
                        busy::show(busy::Info {
                            job,
                            label: remote.label.clone().into(),
                            progress,
                        });
                    }
                }
                false
            }
            worker::Event::Done { job, bytes } => self.worker_done(ctx, job, &bytes),
            worker::Event::Crashed { job, message } => self.worker_crashed(ctx, job, &message),
        }
    }

    /// The worker has answered for the running job: commits what it came to.
    fn worker_done(&mut self, ctx: &Context<Self>, id: u64, bytes: &[u8]) -> bool {
        if self.job.as_ref().is_none_or(|job| job.id != id) {
            // The answer to a job that was cancelled.
            return false;
        }
        let Some(mut job) = self.job.take() else {
            return false;
        };
        let Some(remote) = job.remote.take() else {
            self.job = Some(job);
            return false;
        };
        let received = Instant::now();
        let wall_ms = remote.sent.elapsed().as_secs_f64() * 1000.0;
        match self.worker.remote().response(bytes) {
            Ok(Answer::Done { output, worked_ms }) => {
                let Some(output) = job.task.output_of(output) else {
                    return self.worker_failed(ctx, job, "the answer was not for the job");
                };
                tracing::info!(
                    "{} took {:.1}ms in the worker: {:.1}ms working, {:.1}ms to prepare and {:.1}ms                      to take in the answer ({}KB there, {}KB back)",
                    remote.label,
                    wall_ms + remote.encode_ms,
                    worked_ms,
                    remote.encode_ms,
                    received.elapsed().as_secs_f64() * 1000.0,
                    remote.bytes_out / 1024,
                    bytes.len() / 1024
                );
                self.commit(ctx, job, output)
            }
            Ok(Answer::NeedsSignature) if !remote.retried => {
                // The worker does not hold the signature after all (it was restarted).
                if self.send_to_worker(&mut job, true) {
                    self.job = Some(job);
                    false
                } else {
                    self.worker_failed(ctx, job, "the worker could not be asked again")
                }
            }
            Ok(Answer::NeedsSignature) => {
                self.worker_failed(ctx, job, "the worker keeps not holding the signature")
            }
            Err(message) => self.worker_failed(ctx, job, &message),
        }
    }

    /// The worker failed to do the job without crashing: the job is done here instead.
    fn worker_failed(&mut self, ctx: &Context<Self>, mut job: Running, message: &str) -> bool {
        tracing::error!("The worker failed: {}", message);
        toast(Toast::warn(format!(
            "The background worker failed ({message}); doing {} here instead",
            job.task.label().to_lowercase()
        )));
        self.worker.fail();
        self.worker.stop();
        self.worker.ensure();
        job.remote = None;
        job.started = Instant::now();
        if job.task.next_step_may_block() {
            self.wait(ctx, job)
        } else {
            self.work(ctx, job)
        }
    }

    /// The worker crashed or never started. If it was doing the running job, that is lost, and
    /// the project is as it was.
    fn worker_crashed(&mut self, _ctx: &Context<Self>, job: Option<u64>, message: &str) -> bool {
        tracing::error!("The worker crashed: {}", message);
        self.worker.stop();
        let doing = self.job.as_ref().is_some_and(|running| {
            running.remote.is_some() && job.is_none_or(|id| id == running.id)
        });
        if doing {
            let running = self.job.take().expect("a job was being done");
            busy::hide();
            toast(Toast::error(format!(
                "The background worker crashed while {}: {message}. The project is unchanged.",
                running.task.label().to_lowercase()
            )));
        }
        if !self.worker.ensure() {
            // Too many have crashed, or none would start.
            toast(Toast::warn(
                "The background worker is not available; the app will do the work itself and may \
                 pause while it does",
            ));
        }
        doing
    }

    /// Puts the job on the indicator and gets the page to paint it, then works on the job.
    fn wait(&mut self, ctx: &Context<Self>, job: Running) -> bool {
        let id = job.id;
        Self::show(&job);
        self.job = Some(job);
        ctx.link().send_future(async move {
            next_paint().await;
            Message::Slice(id)
        });
        false
    }

    /// Works on a job for a slice. If that does not finish it, the page gets to respond to
    /// input before the next slice.
    fn work(&mut self, ctx: &Context<Self>, mut job: Running) -> bool {
        let deadline = Instant::now() + SLICE;
        loop {
            let output = if let Some(output) = job.finished.take() {
                // It was done, and waited for the page to respond to input before it was
                // committed. Nobody asked it not to be.
                Some(output)
            } else {
                let label = job.task.label();
                let step_started = Instant::now();
                #[cfg(debug_assertions)]
                debug_hooks::artificial_delay();
                let output = job.task.step();
                job.steps += 1;
                job.longest_step = job.longest_step.max(step_started.elapsed());
                if output.is_some() {
                    tracing::info!(
                        "{} took {:.1}ms in {} steps (longest {:.1}ms)",
                        label,
                        job.started.elapsed().as_secs_f64() * 1000.0,
                        job.steps,
                        job.longest_step.as_secs_f64() * 1000.0
                    );
                    if Instant::now() >= deadline {
                        // The page has not been able to respond for a while, and whoever was
                        // waiting may have pressed `Esc` in the meantime: that has to be seen
                        // before the project is changed.
                        job.finished = output;
                        break;
                    }
                }
                output
            };

            let Some(output) = output else {
                if Instant::now() >= deadline {
                    break;
                }
                continue;
            };

            // The job is done, and it is committed in one go.
            match self.state.finish(output) {
                Ok(model::Started::Done(changed)) => {
                    busy::hide();
                    return self.updated(Ok(changed), job.resets_panzoom);
                }
                Ok(model::Started::Running(task)) => {
                    // Something more to do before it is over.
                    job.task = task;
                    job.started = Instant::now();
                    job.steps = 0;
                    job.longest_step = Duration::ZERO;
                    if self.send_to_worker(&mut job, false) {
                        Self::show(&job);
                        self.job = Some(job);
                        return false;
                    }
                    if job.task.next_step_may_block() {
                        return self.wait(ctx, job);
                    }
                }
                Err(error) => {
                    busy::hide();
                    return self.updated(Err(error), job.resets_panzoom);
                }
            }
        }

        let id = job.id;
        Self::show(&job);
        self.job = Some(job);
        ctx.link().send_future(async move {
            TimeoutFuture::new(0).await;
            Message::Slice(id)
        });
        false
    }

    /// Commits the output of a job that the worker did.
    fn commit(&mut self, ctx: &Context<Self>, mut job: Running, output: model::TaskOutput) -> bool {
        match self.state.finish(output) {
            Ok(model::Started::Done(changed)) => {
                busy::hide();
                self.updated(Ok(changed), job.resets_panzoom)
            }
            Ok(model::Started::Running(task)) => {
                // Something more to do before it is over.
                job.task = task;
                job.started = Instant::now();
                job.steps = 0;
                job.longest_step = Duration::ZERO;
                self.begin(ctx, job)
            }
            Err(error) => {
                busy::hide();
                self.updated(Err(error), job.resets_panzoom)
            }
        }
    }

    fn show(job: &Running) {
        busy::show(busy::Info {
            job: job.id,
            label: job.label().into(),
            progress: job.remote.is_none().then(|| job.task.progress()).flatten(),
        });
    }

    /// Drops the running job, if there is one. It worked on a copy, so the project, its
    /// history, and the log of actions for the crash dump are exactly as they were.
    fn cancel(&mut self) -> bool {
        let Some(job) = self.job.take() else {
            return false;
        };
        tracing::info!(
            "Cancelled {} after {:.1}ms and {} steps",
            job.label(),
            job.started.elapsed().as_secs_f64() * 1000.0,
            job.steps
        );
        busy::hide();
        if job.remote.is_some() {
            // Ending the worker is how a job in it is cancelled; a new one takes its place.
            self.worker.stop();
            self.worker.ensure();
        }
        toast(Toast::success(format!("Cancelled: {}", job.label())));
        // What the job was started for may be shown differently now (an inference that was
        // cancelled leaves the replacements chosen and says that nothing was inferred).
        true
    }

    /// Listens for `Esc` anywhere on the page. In the capture phase so that what has focus does
    /// not get in first: while a job runs, `Esc` cancels it, whatever else it might do.
    fn install_escape(ctx: &Context<Self>) -> Closure<dyn FnMut(web_sys::KeyboardEvent)> {
        let link = ctx.link().clone();
        let listener = Closure::wrap(Box::new(move |event: web_sys::KeyboardEvent| {
            if event.key() != "Escape" || event.is_composing() {
                return;
            }
            let in_text_field = event
                .target()
                .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                .and_then(|element| {
                    element
                        .closest("input, textarea, select, [contenteditable]")
                        .ok()
                        .flatten()
                })
                .is_some();
            link.send_message(Message::Escape(in_text_field));
        }) as Box<dyn FnMut(_)>);
        web_sys::window()
            .unwrap()
            .add_event_listener_with_callback_and_bool(
                "keydown",
                listener.as_ref().unchecked_ref(),
                true,
            )
            .unwrap();
        listener
    }

    fn install_unload_hook(&mut self) {
        let before_unload = Closure::wrap(Box::new(move |event: web_sys::BeforeUnloadEvent| {
            event.set_return_value("Are you sure you want to leave? Unsaved changes will be lost!");
        }) as Box<dyn FnMut(_)>);

        web_sys::window()
            .unwrap()
            .set_onbeforeunload(Some(before_unload.as_ref().unchecked_ref()));

        self.before_unload = Some(before_unload);
    }

    /// What follows from trying to download the project that the page's URL names. If it cannot
    /// be (Firebase may be blocked, or the person offline), the URL is reset and they are told,
    /// and the app starts empty as for any other address.
    fn opened_from_url(
        result: Result<(account::RemoteProjectMetadata, Vec<u8>), account::CloudError>,
    ) -> Vec<Message> {
        match result {
            Ok((project, blob)) => vec![
                Message::Dispatch(model::Action::SetRemoteProjectMetadata(Some(project))),
                Message::Dispatch(model::Action::Proof(model::proof::Action::ImportProof(
                    blob.into(),
                ))),
            ],
            Err(error) => {
                tracing::warn!("Could not open the project from the address: {error}");
                toast(Toast::error(format!(
                    "Could not open the project at this address: {error}"
                )));
                // failed, reset the url
                model::update_window_url_path("/");
                Vec::default()
            }
        }
    }

    // TODO: replace this with yew-router
    /// Starts to load the project that the address names, if it names one. Returns whether it does.
    fn load_project_from_url_path(ctx: &Context<Self>) -> bool {
        let path_str = web_sys::window().unwrap().location().pathname().unwrap();
        let path = &path_str.split('/').collect::<Vec<&str>>()[1..];
        if path[0] == "p" && path.len() == 2 {
            // Published project
            tracing::debug!("Load published project {}", path[1]);
            let tag = path[1].to_owned();
            ctx.link().send_future_batch(async move {
                Self::opened_from_url(account::download_published_project(&tag, 1 /* TODO */).await)
            });
            true
        } else if path[0] == "u" && path.len() == 3 {
            // Personal projects
            let uid = path[1].to_owned();
            let id = path[2].to_owned();
            tracing::debug!("Load personal project {uid}/{id}");
            ctx.link().send_future_batch(async move {
                Self::opened_from_url(account::download_personal_project(&uid, &id).await)
            });
            true
        } else {
            model::update_window_url_path("/");
            false
        }
    }

    #[allow(clippy::let_underscore_untyped, clippy::let_underscore_must_use)]
    fn render(
        ctx: &Context<Self>,
        state: &model::State,
        backup: Option<&autosave::Snapshot>,
    ) -> Html {
        let proof = state.proof();
        let dispatch = ctx.link().callback(Message::Dispatch);

        let workspace = html! {
            <WorkspaceView
                workspace={proof.workspace.clone()}
                signature={proof.signature.clone()}
                metadata={proof.metadata.clone()}
                dispatch={dispatch.clone()}
                attachment_highlight={state.attachment_highlight.clone()}
                slice_highlight={state.slice_highlight}
            />
        };

        let boundary_preview = match &proof.boundary {
            Some(b) => html! {
                <BoundaryPreview
                    boundary={b.clone()}
                    dispatch={dispatch.reform(model::Action::Proof)}
                    signature={proof.signature.clone()}
                />
            },
            None => Default::default(),
        };

        html! {
            <main class="app">
                <BusyIndicator on_cancel={ctx.link().callback(|()| Message::Cancel)} />
                <LayoutNote />
                <AutosaveNote />
                if let Some(backup) = backup {
                    <RestoreNotice
                        backup={backup.clone()}
                        on_restore={ctx.link().callback(|()| Message::Autosave(autosave::Message::Restore))}
                        on_discard={ctx.link().callback(|()| Message::Autosave(autosave::Message::Discard))}
                    />
                }
                <tooltip::HoverTooltip />
                <Sidebar
                    dispatch={dispatch}
                    proof={proof.clone()}
                    options={state.options.clone()}
                    drafts={state.drafts.clone()}
                    remote_project_metadata={state.remote_project_metadata.clone()}
                />
                <div class="toaster">
                    <ToasterComponent timeout={3000} />
                </div>
                <div class="boundary__and__workspace">
                    {boundary_preview}
                    {workspace}
                </div>
                <Modal id="panic" header="Unexpected crash" persistent=true>
                    {info::get_panic_message()}
                </Modal>
                <Modal id="about" header="About">
                    {info::get_about_message()}
                </Modal>
                <Modal id="help" header="Help">
                    {info::get_help_message()}
                </Modal>
                <span class="version">
                    {format!("Version: {}", option_env!("GIT_DESCRIBE").unwrap_or(env!("CARGO_PKG_VERSION")))}
                </span>
            </main>
        }
    }
}

/// Resolves once the browser has had the chance to paint what was changed, or after a while if
/// it will not (as in a tab that is not visible).
#[allow(clippy::future_not_send)] // It is not spawned on another thread, as there are none.
async fn next_paint() {
    let (sender, receiver) = futures::channel::oneshot::channel();
    let frame = gloo::render::request_animation_frame(move |_| {
        // The receiver is only gone if the timeout won.
        sender.send(()).ok();
    });
    // Whichever comes first.
    select(receiver, TimeoutFuture::new(100)).await;
    drop(frame);
    // The painting happens after the callbacks of the frame, so wait for whatever comes after.
    TimeoutFuture::new(0).await;
}
