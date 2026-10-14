//! The notice that says that the search for attachments did not go as it might have been thought
//! to (see [`crate::app::attach_inference`]). It stays until it is dismissed, as it has more to
//! say than a toast has the time for.
//!
//! Lists of generators (those given up on, those not tried) can be long and are made of
//! formulas, so they are behind a header that says how many there are, and are not rendered until
//! they are opened, and then only the first [`MOST_SHOWN`]: the notice itself costs next to
//! nothing to show.

use homotopy_common::tex::seal;
use yew::prelude::*;

use crate::app::tex::TexSpan;

/// The most names of a list that are shown when it is open.
pub const MOST_SHOWN: usize = 50;

/// One thing the notice says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoticeLine {
    Text(String),
    /// A statement with a list of generators (their names, as TeX) to open to see.
    Names {
        text: String,
        names: Vec<String>,
    },
}

#[derive(Properties, PartialEq)]
pub struct Props {
    pub lines: Vec<NoticeLine>,
    pub on_dismiss: Callback<()>,
}

#[function_component]
pub fn InferenceNotice(props: &Props) -> Html {
    let dismiss = props.on_dismiss.reform(|_| ());
    html! {
        <div class="inference-notice" role="status">
            <div class="inference-notice__text">
                { for props.lines.iter().map(|line| match line {
                    NoticeLine::Text(text) => html! { <p>{text.clone()}</p> },
                    NoticeLine::Names { text, names } => html! {
                        <NameList text={text.clone()} names={names.clone()} />
                    },
                }) }
            </div>
            <button class="restore-notice__button" onclick={dismiss}>{"Dismiss"}</button>
        </div>
    }
}

#[derive(Properties, PartialEq)]
struct NameListProps {
    text: String,
    names: Vec<String>,
}

/// A statement that has a list of generators to open. Closed, the names are not rendered.
#[function_component]
fn NameList(props: &NameListProps) -> Html {
    let open = use_state(|| false);
    let toggle = {
        let open = open.clone();
        Callback::from(move |_: MouseEvent| open.set(!*open))
    };
    let more = props.names.len().saturating_sub(MOST_SHOWN);
    html! {
        <div class="inference-notice__names">
            <button
                class="inference-notice__toggle"
                aria-expanded={open.to_string()}
                onclick={toggle}
            >
                {props.text.clone()}
                {" "}
                {if *open { "\u{25be}" } else { "\u{25b8}" }}
            </button>
            if *open {
                <ul class="inference-notice__list">
                    { for props.names.iter().take(MOST_SHOWN).map(|name| html! {
                        <li>
                            <TexSpan class="inference-notice__name" error_color="#c004" raw_tex={seal(name)} />
                        </li>
                    }) }
                    if more > 0 {
                        <li>{format!("and {more} more")}</li>
                    }
                </ul>
            }
        </div>
    }
}
