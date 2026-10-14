//! The chip that says that a generator was picked to be attached, and that the next click on the
//! diagram attaches it (see [`model::Action::Arm`](crate::model::Action::Arm)). Clicking the chip,
//! or pressing `Esc`, puts the generator down. It is what dragging the generator to the diagram
//! does for people who cannot or would rather not drag.

use yew::prelude::*;

use super::tex::TexSpan;

#[derive(Properties, PartialEq)]
pub struct Props {
    /// The name of the generator (which may be a formula).
    pub name: String,
    pub on_cancel: Callback<()>,
}

#[function_component]
pub fn ArmedChip(props: &Props) -> Html {
    let cancel = props.on_cancel.reform(|_| ());
    html! {
        <div class="armed-chip" role="status" aria-live="polite">
            <button
                class="armed-chip__pill"
                type="button"
                title="Click to cancel"
                onclick={cancel}
            >
                <span>{"Attaching"}</span>
                <TexSpan class="armed-chip__name" raw_tex={props.name.clone()} />
                <span class="armed-chip__hint">{"click a point in the diagram, Esc to cancel"}</span>
            </button>
        </div>
    }
}
