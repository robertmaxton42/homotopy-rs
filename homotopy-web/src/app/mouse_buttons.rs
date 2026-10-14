//! The back and forward buttons of a mouse, which undo and redo instead of navigating the
//! browser away from a project with unsaved work.
//!
//! Browsers navigate when such a button is released (`mouseup`), and some also on `auxclick`, so
//! all of the events of those buttons are intercepted before they reach the page and have their
//! default action cancelled. Undo and redo happen on release, once per press. Chromium and
//! Firefox both fire these events for the buttons, and cancelling the default action of any of
//! them prevents navigation in both.
//!
//! The page makes no history entries of its own (it only replaces the current one), and the
//! prompt before leaving catches any navigation that is not prevented, so there is no need for a
//! guard on `popstate`.

use homotopy_core::Direction;
use wasm_bindgen::{closure::Closure, JsCast};
use web_sys::{AddEventListenerOptions, EventTarget, MouseEvent};
use yew::Callback;

use super::settings::AppSettings;
use crate::model::{history, Action};

/// The events that browsers navigate on, or that tell the page about the buttons.
const EVENTS: [&str; 3] = ["mousedown", "mouseup", "auxclick"];

/// The `MouseEvent.button` of the back button.
const BACK: i16 = 3;
/// The `MouseEvent.button` of the forward button.
const FORWARD: i16 = 4;

/// The direction in history that a mouse button moves, if it is the back or forward button.
const fn direction(button: i16) -> Option<Direction> {
    match button {
        BACK => Some(Direction::Backward),
        FORWARD => Some(Direction::Forward),
        _ => None,
    }
}

/// Listens for the buttons while it is alive.
pub struct MouseButtons {
    listener: Closure<dyn FnMut(MouseEvent)>,
}

impl MouseButtons {
    /// Starts listening, dispatching undo and redo as long as the setting is on.
    pub fn install(dispatch: Callback<Action>) -> Self {
        let listener = Closure::wrap(Box::new(move |event: MouseEvent| {
            let Some(direction) = direction(event.button()) else {
                return;
            };
            if !AppSettings::get_mouse_undo() {
                return;
            }
            // Neither the browser nor the page (clicking the diagram, say) should see it.
            event.prevent_default();
            event.stop_propagation();
            if event.type_() == "mouseup" {
                dispatch.emit(Action::History(history::Action::Move(
                    history::Direction::Linear(direction),
                )));
            }
        }) as Box<dyn FnMut(_)>);

        // In the capture phase, so that nothing can get in first. Not passive, to be able to
        // cancel.
        let options = AddEventListenerOptions::new();
        options.set_capture(true);
        options.set_passive(false);
        let window: EventTarget = web_sys::window().unwrap().into();
        for event in EVENTS {
            window
                .add_event_listener_with_callback_and_add_event_listener_options(
                    event,
                    listener.as_ref().unchecked_ref(),
                    &options,
                )
                .unwrap();
        }
        Self { listener }
    }
}

impl Drop for MouseButtons {
    fn drop(&mut self) {
        let window: EventTarget = web_sys::window().unwrap().into();
        for event in EVENTS {
            window
                .remove_event_listener_with_callback_and_bool(
                    event,
                    self.listener.as_ref().unchecked_ref(),
                    true,
                )
                .unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use homotopy_core::Direction;

    use super::direction;

    #[test]
    fn back_and_forward_buttons_move_in_history() {
        // The primary, auxiliary (wheel) and secondary buttons are left alone.
        assert_eq!(direction(0), None);
        assert_eq!(direction(1), None);
        assert_eq!(direction(2), None);
        assert_eq!(direction(3), Some(Direction::Backward));
        assert_eq!(direction(4), Some(Direction::Forward));
        assert_eq!(direction(5), None);
    }
}
