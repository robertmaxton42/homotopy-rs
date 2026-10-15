//! A small "?" after the title of a setting, which shows its explanation in a popover on hover,
//! on keyboard focus and on a tap.
//!
//! The button can be reached with Tab and describes itself with the explanation
//! (`aria-describedby`, which also works while the popover is hidden); Esc closes the popover
//! without moving the focus, as hover and focus popovers are expected to. The popover belongs to
//! the button's wrapper, so that moving the pointer onto it does not close it. It is positioned
//! by the stylesheet over the row of the setting that holds the tip (`.settings__help-text`).

use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement};
use yew::prelude::*;

#[derive(Properties, Debug, Clone, PartialEq, Eq)]
pub struct HelpTipProps {
    /// The id of the popover, which the button points at. Unique in the page.
    pub id: AttrValue,
    /// The title of the setting, for the name of the button.
    pub label: AttrValue,
    /// The explanation: one or two short sentences.
    pub text: AttrValue,
}

/// What to build the `id`s of a setting's elements from: the words of its title, in lower case,
/// joined by dashes.
#[must_use]
pub fn slug(title: &str) -> String {
    let words: Vec<String> = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    words.join("-")
}

#[function_component]
pub fn HelpTip(props: &HelpTipProps) -> Html {
    let wrapper = use_node_ref();

    // (A tap does not always focus a button, so it also opens the popover by itself.)
    let on_click = {
        let wrapper = wrapper.clone();
        Callback::from(move |_: MouseEvent| {
            if let Some(wrapper) = wrapper.cast::<Element>() {
                wrapper.remove_attribute("data-dismissed").ok();
                if wrapper.has_attribute("data-open") {
                    wrapper.remove_attribute("data-open").ok();
                } else {
                    wrapper.set_attribute("data-open", "").ok();
                }
            }
        })
    };
    let on_key = {
        let wrapper = wrapper.clone();
        Callback::from(move |e: KeyboardEvent| {
            if e.key() == "Escape" {
                if let Some(wrapper) = wrapper.cast::<Element>() {
                    wrapper.remove_attribute("data-open").ok();
                    // Hovering would show it again: it stays shut until the pointer leaves.
                    wrapper.set_attribute("data-dismissed", "").ok();
                    e.stop_propagation();
                }
            }
        })
    };
    let on_leave = {
        let wrapper = wrapper.clone();
        Callback::from(move |_: MouseEvent| {
            if let Some(wrapper) = wrapper.cast::<Element>() {
                wrapper.remove_attribute("data-dismissed").ok();
            }
        })
    };
    let on_focus_out = {
        let wrapper = wrapper.clone();
        Callback::from(move |e: FocusEvent| {
            let Some(wrapper) = wrapper.cast::<Element>() else {
                return;
            };
            // Moving the focus within the wrapper (there is nothing else to focus) is not leaving.
            let stays = e
                .related_target()
                .and_then(|t| t.dyn_into::<HtmlElement>().ok())
                .is_some_and(|t| wrapper.contains(Some(&t)));
            if !stays {
                wrapper.remove_attribute("data-open").ok();
                wrapper.remove_attribute("data-dismissed").ok();
            }
        })
    };

    html! {
        <span
            class="settings__help"
            ref={wrapper}
            onmouseleave={on_leave}
            onfocusout={on_focus_out}
        >
            <button
                type="button"
                class="settings__help-button"
                aria-label={format!("About: {}", props.label)}
                aria-describedby={props.id.clone()}
                onclick={on_click}
                onkeydown={on_key}
            >
                {"?"}
            </button>
            <span class="settings__help-text" role="tooltip" id={props.id.clone()}>
                {props.text.clone()}
            </span>
        </span>
    }
}

#[cfg(test)]
mod tests {
    use super::slug;

    #[test]
    fn ids_are_made_of_the_words_of_the_title() {
        assert_eq!(
            slug("Give up on a generator after"),
            "give-up-on-a-generator-after"
        );
        assert_eq!(
            slug("Show where sheets meet (fans and echoes)"),
            "show-where-sheets-meet-fans-and-echoes"
        );
    }
}
