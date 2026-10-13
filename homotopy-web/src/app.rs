use boundary::BoundaryPreview;
use futures::future::select;
use gloo_timers::future::TimeoutFuture;
use homotopy_model::job::Job;
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
    components::{
        busy::{self, BusyIndicator},
        icon::{Icon, IconSize},
        modal::Modal,
        panzoom::PanZoom,
        toast::{toast, Toast, ToasterComponent},
    },
    model,
};

#[allow(clippy::mem_forget, clippy::future_not_send)] // due to wasm_bindgen
pub mod account;
mod attach;
mod boundary;
#[cfg(any(debug_assertions, feature = "show_debug_panel"))]
mod debug;
#[cfg(debug_assertions)]
mod debug_hooks;
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
mod workspace;

/// How long the app works on a job before it lets the page respond to input. Input is only
/// handled between such slices, so this is how long the page can be unresponsive for, at the
/// least; and a step that cannot be split up takes as long as it takes.
const SLICE: Duration = Duration::from_millis(10);

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
}

/// A task that is being worked on a slice at a time. It works on a copy of what it needs, and
/// nothing is changed until it is done, so cancelling it is just forgetting it.
struct Running {
    /// Tells this job from any other, past or future.
    id: u64,
    task: model::Task,
    /// The output of the task once it is done, until it is committed.
    finished: Option<model::TaskOutput>,
    resets_panzoom: bool,
    /// For the log.
    started: Instant,
    steps: usize,
    longest_step: Duration,
}

pub struct App {
    state: model::State,
    /// The job in progress, if any. While there is one, actions that would change the project
    /// are refused (see [`model::Action::allowed_while_busy`]).
    job: Option<Running>,
    next_job: u64,
    signature_stylesheet: SignatureStylesheet,
    before_unload: Option<Closure<dyn FnMut(web_sys::BeforeUnloadEvent)>>,
    // Hold onto the listeners so that they are dropped when the app is destroyed
    _mouse_buttons: MouseButtons,
    escape: Option<Closure<dyn FnMut(web_sys::KeyboardEvent)>>,
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

        Self::load_project_from_url_path(ctx);

        #[cfg(debug_assertions)]
        debug_hooks::install(ctx.link());

        Self {
            state,
            job: None,
            next_job: 0,
            signature_stylesheet,
            before_unload: None,
            _mouse_buttons: MouseButtons::install(ctx.link().callback(Message::Dispatch)),
            escape: Some(Self::install_escape(ctx)),
        }
    }

    fn update(&mut self, ctx: &Context<Self>, msg: Self::Message) -> bool {
        let rerender = self.handle(ctx, msg);
        #[cfg(debug_assertions)]
        debug_hooks::publish(&self.state, self.job.as_ref().map(|job| job.task.label()));
        rerender
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        Self::render(ctx, &self.state)
    }

    fn destroy(&mut self, _ctx: &Context<Self>) {
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

        // Refuse rather than queue what would change the project while a job works on it: the
        // person asking for it cannot have known what it would come on top of, and cancelling
        // the job must not replay anything.
        if let Some(job) = &self.job {
            if !action.allowed_while_busy() {
                tracing::info!(
                    "Refused action while busy ({}): {:?}",
                    job.task.label(),
                    action
                );
                busy::refused();
                return false;
            }
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

        let started = Instant::now();
        let result = self.state.start(action);
        tracing::info!(
            "State update took {}ms.",
            started.elapsed().as_secs_f64() * 1000.0
        );

        match result {
            Ok(model::Started::Done(changed)) => self.updated(Ok(changed), resets_panzoom),
            Ok(model::Started::Running(task)) => {
                self.next_job += 1;
                let job = Running {
                    id: self.next_job,
                    task,
                    finished: None,
                    resets_panzoom,
                    started: Instant::now(),
                    steps: 0,
                    longest_step: Duration::ZERO,
                };
                if job.task.next_step_may_block() {
                    // A step that cannot be split up comes first, and the page cannot show
                    // anything while it is on. So get the indicator on the page, and let the
                    // browser paint it, before starting.
                    self.wait(ctx, job)
                } else {
                    self.work(ctx, job)
                }
            }
            Err(error) => self.updated(Err(error), resets_panzoom),
        }
    }

    /// What follows from the project having been updated, or not.
    fn updated(&mut self, result: Result<bool, model::ModelError>, resets_panzoom: bool) -> bool {
        homotopy_core::collect_garbage();

        if matches!(result, Ok(true)) {
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

    fn show(job: &Running) {
        busy::show(busy::Info {
            job: job.id,
            label: job.task.label(),
            progress: job.task.progress(),
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
            job.task.label(),
            job.started.elapsed().as_secs_f64() * 1000.0,
            job.steps
        );
        busy::hide();
        toast(Toast::success(format!("Cancelled: {}", job.task.label())));
        false
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

    // TODO: replace this with yew-router
    fn load_project_from_url_path(ctx: &Context<Self>) {
        let path_str = web_sys::window().unwrap().location().pathname().unwrap();
        let path = &path_str.split('/').collect::<Vec<&str>>()[1..];
        if path[0] == "p" && path.len() == 2 {
            // Published project
            tracing::debug!("Load published project {}", path[1]);
            let tag = path[1].to_owned();
            ctx.link().send_future_batch(async move {
                if let Some((project, blob)) =
                    account::download_published_project(&tag, 1 /* TODO */).await
                {
                    vec![
                        Message::Dispatch(model::Action::SetRemoteProjectMetadata(Some(project))),
                        Message::Dispatch(model::Action::Proof(model::proof::Action::ImportProof(
                            blob.into(),
                        ))),
                    ]
                } else {
                    // failed, reset the url
                    model::update_window_url_path("/");
                    Vec::default()
                }
            });
        } else if path[0] == "u" && path.len() == 3 {
            // Personal projects
            let uid = path[1].to_owned();
            let id = path[2].to_owned();
            tracing::debug!("Load personal project {uid}/{id}");
            ctx.link().send_future_batch(async move {
                if let Some((project, blob)) = account::download_personal_project(&uid, &id).await {
                    vec![
                        Message::Dispatch(model::Action::SetRemoteProjectMetadata(Some(project))),
                        Message::Dispatch(model::Action::Proof(model::proof::Action::ImportProof(
                            blob.into(),
                        ))),
                    ]
                } else {
                    // failed, reset the url
                    model::update_window_url_path("/");
                    Vec::default()
                }
            });
        } else {
            model::update_window_url_path("/");
        }
    }

    #[allow(clippy::let_underscore_untyped, clippy::let_underscore_must_use)]
    fn render(ctx: &Context<Self>, state: &model::State) -> Html {
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
                <Sidebar
                    dispatch={dispatch}
                    proof={proof.clone()}
                    options={state.options.clone()}
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
