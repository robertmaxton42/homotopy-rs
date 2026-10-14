//! TEMPORARY: a diagnostic log of the input events that a mouse's back and forward buttons may
//! cause, for finding out what a browser sends when `mouse_buttons` does not see them (the mouse
//! driver may send browser navigation as an application command, or as `BrowserBack` and
//! `BrowserForward` keys). Remove it when the history-trap redesign no longer needs it.
//!
//! It is off unless `localStorage["homotopy-log-input"] === "1"` when the page starts (read once;
//! a `localStorage` that throws counts as off). When on, it writes to the browser console, with
//! the prefix `[homotopy input]`, what arrives at the window (in the capture phase, before the
//! page's own listeners):
//!
//! - `mousedown`, `mouseup`, `pointerdown`, `pointerup` for buttons other than the left, middle and
//!   right ones (0, 1, 2), and `auxclick` for all buttons;
//! - `keydown` and `keyup` of keys whose `key` or `code` starts with "Browser", and of Alt with
//!   the left or right arrow;
//! - `popstate` (with the length and state of the history), `beforeunload` and `pagehide`.
//!
//! It only logs: no event is cancelled or stopped, and it uses `console.log` and not `tracing`,
//! so that it works in the release build.

use gloo::events::{EventListener, EventListenerOptions, EventListenerPhase};
use wasm_bindgen::JsCast;
use web_sys::{Event, KeyboardEvent, MouseEvent};

/// The key in `localStorage` that turns the log on (with the value `"1"`).
const SETTING: &str = "homotopy-log-input";

/// What is done for an event.
type Handler = fn(&Event);

const PREFIX: &str = "[homotopy input]";

/// Listens for the events while it is alive, if the log is on.
pub struct InputLog {
    _listeners: Vec<EventListener>,
}

/// Whether the stored value of the setting turns the log on.
fn is_on(stored: Option<&str>) -> bool {
    stored == Some("1")
}

/// Reads the setting; `localStorage` that is missing or throws is off.
fn enabled() -> bool {
    let stored = web_sys::window()
        .and_then(|window| window.local_storage().ok().flatten())
        .and_then(|storage| storage.get_item(SETTING).ok().flatten());
    is_on(stored.as_deref())
}

fn log(line: &str) {
    web_sys::console::log_1(&format!("{PREFIX} {line}").into());
}

/// Whether a mouse event is worth a line: the buttons other than 0, 1 and 2 (left, middle,
/// right), which are the ones that matter and would otherwise be all of the log, and every
/// button of an `auxclick`.
fn mouse_wanted(kind: &str, button: i16) -> bool {
    kind == "auxclick" || !matches!(button, 0..=2)
}

fn log_mouse(event: &Event) {
    let Some(mouse) = event.dyn_ref::<MouseEvent>() else {
        return;
    };
    let kind = event.type_();
    if !mouse_wanted(&kind, mouse.button()) {
        return;
    }
    log(&format!(
        "{kind} button={} buttons={}",
        mouse.button(),
        mouse.buttons()
    ));
}

/// Whether a key is worth a line: a browser navigation key, or Alt with the left or right arrow
/// (the keyboard shortcut for going back and forward).
fn key_wanted(key: &str, code: &str, alt: bool) -> bool {
    key.starts_with("Browser")
        || code.starts_with("Browser")
        || (alt && matches!(key, "ArrowLeft" | "ArrowRight"))
        || (alt && matches!(code, "ArrowLeft" | "ArrowRight"))
}

fn log_key(event: &Event) {
    let Some(key) = event.dyn_ref::<KeyboardEvent>() else {
        return;
    };
    if !key_wanted(&key.key(), &key.code(), key.alt_key()) {
        return;
    }
    log(&format!(
        "{} key={:?} code={:?} altKey={}",
        event.type_(),
        key.key(),
        key.code(),
        key.alt_key()
    ));
}

fn log_popstate(_: &Event) {
    let Some(history) = web_sys::window().and_then(|window| window.history().ok()) else {
        log("popstate (no history)");
        return;
    };
    let length = history
        .length()
        .map_or_else(|_| "?".into(), |n| n.to_string());
    let state = history
        .state()
        .ok()
        .and_then(|state| js_sys::JSON::stringify(&state).ok())
        .map_or_else(|| "?".into(), String::from);
    log(&format!(
        "popstate history.length={length} history.state={state}"
    ));
}

fn log_lifecycle(event: &Event) {
    log(&event.type_());
}

impl InputLog {
    /// Starts listening if the setting is on.
    pub fn install() -> Self {
        let mut listeners = vec![];
        if enabled() {
            log("logging input (localStorage \"homotopy-log-input\" is \"1\")");
            // Not passive-cancelling: this only reads. In the capture phase, to see events
            // before anything stops them.
            let options = EventListenerOptions {
                phase: EventListenerPhase::Capture,
                passive: true,
            };
            let handlers: [(&str, Handler); 12] = [
                ("mousedown", log_mouse),
                ("mouseup", log_mouse),
                ("auxclick", log_mouse),
                ("pointerdown", log_mouse),
                ("pointerup", log_mouse),
                ("keydown", log_key),
                ("keyup", log_key),
                ("popstate", log_popstate),
                ("beforeunload", log_lifecycle),
                ("pagehide", log_lifecycle),
                ("pageshow", log_lifecycle),
                ("visibilitychange", log_lifecycle),
            ];
            if let Some(window) = web_sys::window() {
                for (kind, handler) in handlers {
                    listeners.push(EventListener::new_with_options(
                        &window, kind, options, handler,
                    ));
                }
            }
        }
        Self {
            _listeners: listeners,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{is_on, key_wanted, mouse_wanted};

    #[test]
    fn only_the_value_1_turns_it_on() {
        assert!(is_on(Some("1")));
        assert!(!is_on(Some("0")));
        assert!(!is_on(Some("true")));
        assert!(!is_on(None));
    }

    #[test]
    fn the_common_buttons_are_not_logged_except_for_auxclick() {
        for button in 0..=2 {
            assert!(!mouse_wanted("mousedown", button));
            assert!(!mouse_wanted("pointerup", button));
            assert!(mouse_wanted("auxclick", button));
        }
        assert!(mouse_wanted("mouseup", 3));
        assert!(mouse_wanted("pointerdown", 4));
        assert!(mouse_wanted("mousedown", 5));
    }

    #[test]
    fn navigation_keys_are_logged() {
        assert!(key_wanted("BrowserBack", "BrowserBack", false));
        assert!(key_wanted("Unidentified", "BrowserForward", false));
        assert!(key_wanted("ArrowLeft", "ArrowLeft", true));
        assert!(key_wanted("ArrowRight", "ArrowRight", true));
        assert!(!key_wanted("ArrowLeft", "ArrowLeft", false));
        assert!(!key_wanted("a", "KeyA", true));
    }
}
