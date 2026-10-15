//! A trap for the browser's back and forward navigation, so that it undoes and redoes instead
//! of leaving the project (setting "Mouse back and forward buttons undo and redo").
//!
//! Some mice (a Logitech under Firefox, found with the input log) send their back and forward
//! buttons to the browser as navigation commands, with no `mouse*` or `auxclick` event, no
//! `Browser*` key and no `popstate`: a page with no history entries of its own simply leaves
//! (`beforeunload`, `pagehide`). [`mouse_buttons`](super::mouse_buttons) cannot see that. So the
//! page keeps entries of its own on either side of the one it is on, and a navigation becomes a
//! `popstate` in the page, which is turned into undo or redo. Alt+Left/Right and the
//! `BrowserBack`/`BrowserForward` keys, and the browser's own back and forward buttons, are
//! navigations too and are handled the same way, once, by this. The decisions are made by the pure
//! [`Machine`]; this file listens for the events and does what it says.
//!
//! # The scheme
//!
//! The history is `[back, centre, forward]` and the page sits on the centre. Each entry has the
//! same URL and a `state` of `{homotopy: "back" | "centre" | "forward"}`.
//!
//! - *Installing* happens at the first user gesture (`pointerdown`, or a `keydown` that is not
//!   Escape or a modifier alone), because browsers skip entries that a page made without user
//!   activation when going back. The current entry is replaced with `back`, `centre` and
//!   `forward` are pushed, and `history.go(-1)` puts the page on the centre. (If the page was
//!   reloaded on a centre, the entries are still there and it is adopted as it is.)
//! - *Landing* on `back` is undo and on `forward` is redo. The page then goes home with
//!   `go(+1)` or `go(-1)`: it never pushes in a `popstate`, which would destroy the entry ahead,
//!   and so **forward survives** as long as nothing else pushes. The `popstate` of that
//!   move home is told from a request by the phase the machine is in (it is *restoring*) and the
//!   marker it lands on (`centre`). A move that does not land is looked into after a second.
//! - Entries that the app makes itself would destroy the one ahead, so it does not make any: the
//!   address changes with `replaceState` (`update_window_url_path`, through [`url_changed`], which
//!   keeps the marker on the centre; the sentinels get the new address as they are landed on), and
//!   links to a fragment (the modals: `#about`, `#help`, `#`) are not followed by the browser
//!   but by `location.replace`, which makes no entry (the click handler here and [`fragment`]).
//!   An entry that has no marker (made by something else) makes the machine start again from it.
//! - *Mouse buttons seen by the page.* A mouse that also sends DOM events for button 3 and 4
//!   goes through the same machine (`mouse_buttons` calls [`mouse`]), which drops a press seen
//!   by the other path within 300 ms in the same direction. Two quick presses from the same path
//!   are two undos. Both paths prevent the default action, so normally only one of them fires.
//! - *Keys.* Nothing is done at the `keydown` of Alt+Left or `BrowserBack`: the navigation that
//!   follows arrives as a `popstate`, and handling the key as well would undo twice. The history
//!   entries are global, so this also holds in a text field, where on some platforms Alt+Left
//!   would be a word movement: the browser decides, and if it navigates it is an undo.
//! - *Off.* Turning the setting off while on the centre goes back once, onto the first
//!   sentinel, and clears its marker, so that one more back leaves the page as it did before
//!   (the forward ones remain in the browser's list but are ordinary entries). Turning it on
//!   again installs again at once, the page having had a gesture.
//! - *Leaving on purpose.* The toolbar's back button is a navigation as well, so while the trap
//!   is on it undoes. The way out is the setting (its note says so), and `beforeunload` still
//!   guards unsaved work. Under Chrome, back from the first entry or a long press of the button
//!   still gets to the earlier pages.
//!
//! The history of the browser is global state that other code can change (the page's address,
//! a fragment, other tabs' entries cannot), so each phase has a timeout rather than trusting
//! that a `go` lands.

mod machine;

use std::cell::RefCell;

use gloo::{
    events::{EventListener, EventListenerOptions},
    timers::callback::Timeout,
};
use homotopy_core::Direction;
use machine::{Event, Kind, Machine, Marking, Output, LANDING_MS};
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Element, History};
use yew::Callback;

use super::settings::{AppSettings, AppSettingsKey, AppSettingsMsg};
use crate::{
    components::delta::CallbackIdx,
    model::{history, Action},
};

struct Shared {
    machine: Machine,
    dispatch: Callback<Action>,
}

thread_local! {
    static TRAP: RefCell<Option<Shared>> = const { RefCell::new(None) };
    /// Set while the page changes its own entry, whose `popstate` (if the browser sends one at
    /// once) is not a move.
    static CHANGING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn history() -> Option<History> {
    web_sys::window()?.history().ok()
}

fn now() -> f64 {
    js_sys::Date::now()
}

const fn name(kind: Kind) -> &'static str {
    match kind {
        Kind::Back => "back",
        Kind::Centre => "centre",
        Kind::Forward => "forward",
    }
}

fn marker(kind: Kind) -> JsValue {
    let object = js_sys::Object::new();
    if js_sys::Reflect::set(&object, &"homotopy".into(), &name(kind).into()).is_err() {
        tracing::warn!("Could not make a history marker");
    }
    object.into()
}

/// The marker of an entry's state, if it has one.
fn kind_of(state: &JsValue) -> Option<Kind> {
    let value = js_sys::Reflect::get(state, &"homotopy".into()).ok()?;
    match value.as_string()?.as_str() {
        "back" => Some(Kind::Back),
        "centre" => Some(Kind::Centre),
        "forward" => Some(Kind::Forward),
        _ => None,
    }
}

fn current() -> Option<Kind> {
    kind_of(&history()?.state().ok()?)
}

/// Gives the machine an event and does what it says.
fn feed(event: Event) {
    let (outputs, dispatch) = TRAP.with(|trap| {
        let mut trap = trap.borrow_mut();
        let Some(shared) = trap.as_mut() else {
            return (vec![], None);
        };
        (shared.machine.handle(event), Some(shared.dispatch.clone()))
    });
    // The borrow is over: what is done here can come back through `feed`.
    for output in outputs {
        perform(output, dispatch.as_ref());
    }
}

fn replace(history: &History, state: &JsValue, url: Option<&str>) {
    if let Err(error) = history.replace_state_with_url(state, "", url) {
        tracing::warn!("Could not replace the history entry: {error:?}");
    }
}

fn perform(output: Output, dispatch: Option<&Callback<Action>>) {
    let Some(history) = history() else { return };
    match output {
        Output::Move(direction) => {
            if let Some(dispatch) = dispatch {
                dispatch.emit(Action::History(history::Action::Move(
                    history::Direction::Linear(direction),
                )));
            }
        }
        Output::Replace { marking, url } => {
            let state = match marking {
                Marking::Keep => history.state().unwrap_or(JsValue::NULL),
                Marking::Set(kind) => marker(kind),
                Marking::Clear => JsValue::NULL,
            };
            replace(&history, &state, url.as_deref());
        }
        Output::Push(kind) => {
            if let Err(error) = history.push_state_with_url(&marker(kind), "", None) {
                tracing::warn!("Could not push a history entry: {error:?}");
            }
        }
        Output::Go(n) => {
            if let Err(error) = history.go_with_delta(n) {
                tracing::warn!("Could not move in the history: {error:?}");
            }
        }
        Output::Wait(epoch) => wait(epoch),
    }
}

/// Asks to be told if the move has not landed after a while.
fn wait(epoch: u32) {
    Timeout::new(LANDING_MS, move || {
        feed(Event::TimedOut {
            epoch,
            state: current(),
        });
    })
    .forget();
}

/// A back or forward button of a mouse, as the DOM reported it (see `mouse_buttons`).
pub fn mouse(direction: Direction) {
    feed(Event::Mouse {
        direction,
        now: now(),
    });
}

/// The app changes the address of the page to `url` (a full URL) without making an entry.
pub fn url_changed(url: &str) {
    let installed = TRAP.with(|trap| trap.borrow().is_some());
    if installed {
        feed(Event::UrlChanged(url.to_owned()));
    } else if let Some(history) = history() {
        // No trap: keep whatever state the entry has.
        let state = history.state().unwrap_or(JsValue::NULL);
        replace(&history, &state, Some(url));
    }
}

/// Goes to a fragment of the page (`"#help"`), without making a history entry when the trap has
/// entries around the current one that a new one would destroy.
pub fn fragment(href: &str) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or(JsValue::NULL)?;
    let in_place = TRAP.with(|trap| trap.borrow().as_ref().is_some_and(|s| s.machine.in_place()));
    if !in_place {
        return window.location().set_href(href);
    }
    let history = window.history()?;
    let state = history.state()?;
    CHANGING.with(|changing| changing.set(true));
    // The replacement forgets the state, which is restored at once.
    let result = window
        .location()
        .replace(href)
        .and_then(|()| history.replace_state_with_url(&state, "", None));
    CHANGING.with(|changing| changing.set(false));
    result
}

/// Whether a key is one that gives the page user activation.
fn activates(key: &str) -> bool {
    !matches!(
        key,
        "Escape" | "Shift" | "Control" | "Alt" | "Meta" | "AltGraph" | "CapsLock" | "Unidentified"
    )
}

/// Keeps the entries around the current one while it is alive.
pub struct HistoryTrap {
    _listeners: Vec<EventListener>,
    setting: Vec<CallbackIdx>,
}

impl HistoryTrap {
    pub fn install(dispatch: Callback<Action>) -> Self {
        let wanted = AppSettings::get_mouse_undo();
        TRAP.with(|trap| {
            *trap.borrow_mut() = Some(Shared {
                machine: Machine::new(wanted),
                dispatch,
            });
        });

        let passive = EventListenerOptions {
            phase: gloo::events::EventListenerPhase::Capture,
            passive: true,
        };
        let window = web_sys::window().expect("a window");
        let mut listeners = vec![];
        listeners.push(EventListener::new_with_options(
            &window,
            "pointerdown",
            passive,
            |event| {
                if event.is_trusted() {
                    feed(Event::Gesture { state: current() });
                }
            },
        ));
        listeners.push(EventListener::new_with_options(
            &window,
            "keydown",
            passive,
            |event| {
                let activating = event
                    .dyn_ref::<web_sys::KeyboardEvent>()
                    .is_some_and(|key| activates(&key.key()));
                if event.is_trusted() && activating {
                    feed(Event::Gesture { state: current() });
                }
            },
        ));
        listeners.push(EventListener::new(&window, "popstate", |_| {
            if CHANGING.with(std::cell::Cell::get) {
                return;
            }
            feed(Event::Popstate {
                state: current(),
                now: now(),
            });
        }));
        // Links to a fragment make an entry, which would destroy the one ahead.
        let not_passive = EventListenerOptions {
            phase: gloo::events::EventListenerPhase::Capture,
            passive: false,
        };
        listeners.push(EventListener::new_with_options(
            &window,
            "click",
            not_passive,
            |event| {
                let Some(click) = event.dyn_ref::<web_sys::MouseEvent>() else {
                    return;
                };
                if click.button() != 0
                    || click.ctrl_key()
                    || click.meta_key()
                    || click.shift_key()
                    || click.alt_key()
                {
                    return;
                }
                let Some(href) = fragment_link(event) else {
                    return;
                };
                if TRAP.with(|t| t.borrow().as_ref().is_some_and(|s| s.machine.in_place())) {
                    event.prevent_default();
                    if let Err(error) = fragment(&href) {
                        tracing::warn!("Could not follow {href}: {error:?}");
                    }
                }
            },
        ));

        let setting = AppSettings::subscribe(
            &[AppSettingsKey::mouse_undo],
            Callback::from(|msg| {
                if let AppSettingsMsg::mouse_undo(on) = msg {
                    feed(Event::Setting(on));
                }
            }),
        );
        Self {
            _listeners: listeners,
            setting,
        }
    }
}

/// The `href` of the link that an event is on, if it is a link to a fragment of this page.
fn fragment_link(event: &web_sys::Event) -> Option<String> {
    let target = event.target()?.dyn_into::<Element>().ok()?;
    let link = target.closest("a[href^='#']").ok()??;
    link.get_attribute("href")
}

impl Drop for HistoryTrap {
    fn drop(&mut self) {
        AppSettings::unsubscribe(&[AppSettingsKey::mouse_undo], &self.setting);
        TRAP.with(|trap| trap.borrow_mut().take());
    }
}
