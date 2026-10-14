//! The notice that says that the inference of instances of schemas did not go as it might have
//! been thought to (see [`crate::app::attach_inference`]). It stays until it is dismissed, as it
//! has more to say than a toast has the time for.

use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct Props {
    pub lines: Vec<String>,
    pub on_dismiss: Callback<()>,
}

#[function_component]
pub fn InferenceNotice(props: &Props) -> Html {
    let dismiss = props.on_dismiss.reform(|_| ());
    html! {
        <div class="inference-notice" role="status">
            <div class="inference-notice__text">
                { for props.lines.iter().map(|line| html! { <p>{line.clone()}</p> }) }
            </div>
            <button class="restore-notice__button" onclick={dismiss}>{"Dismiss"}</button>
        </div>
    }
}
