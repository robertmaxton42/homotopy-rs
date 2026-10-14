//! The note that the project is being saved in the background (see [`crate::autosave`]), shown
//! while it is.
//!
//! Like the layout note it is put on the page at once and fades in after [`DELAY`] with a CSS
//! animation, so that a save that is done in no time causes no flicker. It is nothing to click on:
//! a save is not something that can be cancelled or that input is refused for, since it works on
//! a copy of the project in another worker.

use std::cell::RefCell;

use yew::prelude::*;

/// How long a save takes before the note is shown, in milliseconds. This must match the delay of
/// the animation of `.autosave-note` in `styles.css`, and the busy indicator's.
#[allow(dead_code)] // Documentation of the stylesheet.
const DELAY: f64 = 150.0;

thread_local! {
    static NOTE: RefCell<Option<Callback<bool>>> = const { RefCell::new(None) };
}

/// Says whether a save is going on.
pub fn show(saving: bool) {
    let callback = NOTE.with(|note| note.try_borrow().ok().and_then(|note| note.clone()));
    if let Some(callback) = callback {
        callback.emit(saving);
    }
}

pub struct AutosaveNote {
    saving: bool,
}

impl Component for AutosaveNote {
    type Message = bool;
    type Properties = ();

    fn create(ctx: &Context<Self>) -> Self {
        let callback = ctx.link().callback(|saving| saving);
        NOTE.with(|note| *note.borrow_mut() = Some(callback));
        Self { saving: false }
    }

    fn update(&mut self, _ctx: &Context<Self>, saving: bool) -> bool {
        // It stays as it is (and so keeps its animation) while the save goes on.
        let changed = saving != self.saving;
        self.saving = saving;
        changed
    }

    fn view(&self, _ctx: &Context<Self>) -> Html {
        if !self.saving {
            return html! {};
        }
        html! {
            <div class="autosave-note" role="status" aria-live="off">
                <span class="autosave-note__spinner" aria-hidden="true"></span>
                {"Saving a backup…"}
            </div>
        }
    }

    fn destroy(&mut self, _ctx: &Context<Self>) {
        NOTE.with(|note| *note.borrow_mut() = None);
    }
}
