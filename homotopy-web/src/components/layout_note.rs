//! The note that diagrams are being laid out (see [`crate::layout`]), shown while the views wait
//! for a layout and show what they showed before.
//!
//! Like the busy indicator it is put on the page at once and fades in after [`DELAY`], with a CSS
//! animation, so that a layout that is done in no time causes no flicker. Unlike it, it is
//! nothing to click on: a layout is not something that can be cancelled or that input is
//! refused for.

use std::cell::RefCell;

use yew::prelude::*;

/// How long a layout takes before the note is shown, in milliseconds. This must match the delay
/// of the animation of `.layout-note` in `styles.css`, and the busy indicator's.
#[allow(dead_code)] // Documentation of the stylesheet.
const DELAY: f64 = 150.0;

thread_local! {
    static NOTE: RefCell<Option<Callback<usize>>> = const { RefCell::new(None) };
}

/// Says how many layouts are being waited for.
pub fn show(pending: usize) {
    let callback = NOTE.with(|note| note.try_borrow().ok().and_then(|note| note.clone()));
    if let Some(callback) = callback {
        callback.emit(pending);
    }
}

pub struct LayoutNote {
    pending: usize,
}

impl Component for LayoutNote {
    type Message = usize;
    type Properties = ();

    fn create(ctx: &Context<Self>) -> Self {
        let callback = ctx.link().callback(|pending| pending);
        NOTE.with(|note| *note.borrow_mut() = Some(callback));
        Self { pending: 0 }
    }

    fn update(&mut self, _ctx: &Context<Self>, pending: usize) -> bool {
        // The note stays as it is (and so keeps its animation) while there is more to wait for.
        let changed = (pending > 0) != (self.pending > 0);
        self.pending = pending;
        changed
    }

    fn view(&self, _ctx: &Context<Self>) -> Html {
        if self.pending == 0 {
            return html! {};
        }
        html! {
            <div class="layout-note" role="status" aria-live="polite">
                <span class="layout-note__spinner" aria-hidden="true"></span>
                {"Laying out…"}
            </div>
        }
    }

    fn destroy(&mut self, _ctx: &Context<Self>) {
        NOTE.with(|note| *note.borrow_mut() = None);
    }
}
