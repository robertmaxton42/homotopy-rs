//! The indicator that something is being worked on, which can be clicked to cancel it.
//!
//! The app tells the indicator what is going on through [`show`], [`hide`] and [`refused`], so
//! that the progress of a job, which changes many times a second, does not make the app itself
//! render again.
//!
//! The indicator is on the page from the moment a job starts, but fades in only after
//! [`DELAY`], with a CSS animation. That way a job that turns out to be quick causes no flicker
//! at all, and one that blocks the page for a long time without a chance to do anything about it
//! (a single step that cannot be split up) still shows up, as the browser keeps CSS animations
//! of this kind going while the page is blocked.

use std::cell::RefCell;

use homotopy_model::job::Progress;
use yew::prelude::*;

/// How long a job runs before its indicator is shown, in milliseconds. This must match the
/// delay of the animation of `.busy` in `styles.css`.
const DELAY: f64 = 150.0;

/// What there is to say about a running job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Info {
    /// Tells jobs apart.
    pub job: u64,
    pub label: &'static str,
    pub progress: Option<Progress>,
}

#[derive(Debug)]
pub enum Event {
    Show(Info),
    Hide,
    Refused,
}

thread_local! {
    static INDICATOR: RefCell<Option<Callback<Event>>> = const { RefCell::new(None) };
}

fn emit(event: Event) {
    // Not while the indicator is being updated, as it is when it calls out to cancel.
    let callback = INDICATOR.with(|indicator| indicator.try_borrow().ok().and_then(|i| i.clone()));
    if let Some(callback) = callback {
        callback.emit(event);
    }
}

/// Shows what a job is doing, or updates what is shown about it.
pub fn show(info: Info) {
    emit(Event::Show(info));
}

/// Takes the indicator down.
pub fn hide() {
    emit(Event::Hide);
}

/// Acknowledges input that was refused because of the job: the indicator, which may not have
/// faded in yet, is shown right away, and shakes.
pub fn refused() {
    emit(Event::Refused);
}

fn now() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map_or(0.0, |performance| performance.now())
}

#[derive(Properties, PartialEq)]
pub struct Props {
    pub on_cancel: Callback<()>,
}

pub enum Msg {
    Event(Event),
    Click,
}

pub struct BusyIndicator {
    info: Option<Info>,
    /// When the job started.
    started: f64,
    /// How many times input has been refused during the job.
    refusals: u32,
}

impl Component for BusyIndicator {
    type Message = Msg;
    type Properties = Props;

    fn create(ctx: &Context<Self>) -> Self {
        let callback = ctx.link().callback(Msg::Event);
        INDICATOR.with(|indicator| *indicator.borrow_mut() = Some(callback));
        Self {
            info: None,
            started: 0.0,
            refusals: 0,
        }
    }

    fn update(&mut self, ctx: &Context<Self>, msg: Self::Message) -> bool {
        match msg {
            Msg::Event(Event::Show(info)) => {
                if self.info.as_ref().map(|i| i.job) != Some(info.job) {
                    self.started = now();
                    self.refusals = 0;
                }
                let changed = self.info.as_ref() != Some(&info);
                self.info = Some(info);
                changed
            }
            Msg::Event(Event::Hide) => self.info.take().is_some(),
            Msg::Event(Event::Refused) => {
                self.refusals += 1;
                self.info.is_some()
            }
            Msg::Click => {
                // Before it fades in, it is not something to click on.
                if self.refusals > 0 || now() - self.started >= DELAY {
                    ctx.props().on_cancel.emit(());
                }
                false
            }
        }
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        let Some(info) = &self.info else {
            return html! {};
        };
        let shake = match self.refusals {
            0 => None,
            // Alternating, as an animation that is already running does not restart if it is
            // asked for again.
            n if n % 2 == 1 => Some("busy__pill--shake-a"),
            _ => Some("busy__pill--shake-b"),
        };
        let progress = info.progress.filter(|p| p.total > 1).map(|p| {
            html! { <span class="busy__progress">{format!("{}/{}", p.done.min(p.total), p.total)}</span> }
        });
        html! {
            <div
                class={classes!("busy", (self.refusals > 0).then_some("busy--shown"))}
                role="status"
                aria-live="polite"
            >
                <button
                    type="button"
                    class={classes!("busy__pill", shake)}
                    title="Cancel (Esc)"
                    onclick={ctx.link().callback(|_| Msg::Click)}
                >
                    <span class="busy__spinner" aria-hidden="true"></span>
                    <span class="busy__label">{format!("{}…", info.label)}</span>
                    {progress}
                    <span class="busy__hint">{"Esc or click to cancel"}</span>
                </button>
            </div>
        }
    }

    fn destroy(&mut self, _ctx: &Context<Self>) {
        INDICATOR.with(|indicator| *indicator.borrow_mut() = None);
    }
}
