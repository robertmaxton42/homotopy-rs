use homotopy_core::signature::Symmetry;
use web_sys::HtmlInputElement;
use yew::prelude::*;
use yew_macro::function_component;

use crate::{
    components::icon::{Icon, IconSize},
    model::{
        proof::{self, SignatureEdit, MAX_SYMMETRY_DIRECTION},
        subscript, Action,
    },
};

#[derive(Clone, PartialEq, Properties)]
pub struct Props {
    pub symmetries: Vec<Symmetry>,
    pub dispatch: Callback<Action>,
}

/// Parses a symmetry from a name and a comma- or space-separated list of directions, such as
/// `1,3`. Returns `None` if the name is empty or already taken, or if there are no directions,
/// any of them is not a number between 1 and [`MAX_SYMMETRY_DIRECTION`].
fn parse_symmetry(name: &str, directions: &str, existing: &[Symmetry]) -> Option<Symmetry> {
    let name = name.trim();
    let directions = directions
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<usize>().ok())
        .collect::<Option<Vec<_>>>()?;
    let valid = !name.is_empty()
        && !directions.is_empty()
        && directions
            .iter()
            .all(|j| (1..=MAX_SYMMETRY_DIRECTION).contains(j))
        && existing.iter().all(|s| s.name != name);
    valid.then(|| Symmetry {
        name: name.to_owned(),
        directions,
    })
}

#[function_component]
pub fn SymmetriesView(props: &Props) -> Html {
    let name = use_state(String::new);
    let directions = use_state(String::new);
    let invalid = use_state(|| false);

    let add = {
        let (name, directions, invalid) = (name.clone(), directions.clone(), invalid.clone());
        let (symmetries, dispatch) = (props.symmetries.clone(), props.dispatch.clone());
        Callback::from(move |()| {
            if let Some(symmetry) = parse_symmetry(&name, &directions, &symmetries) {
                dispatch.emit(
                    proof::Action::EditSignature(SignatureEdit::AddSymmetry(symmetry)).into(),
                );
                name.set(String::new());
                directions.set(String::new());
            } else {
                invalid.set(true);
            }
        })
    };

    let oninput = |state: &UseStateHandle<String>, invalid: &UseStateHandle<bool>| {
        let (state, invalid) = (state.clone(), invalid.clone());
        Callback::from(move |e: InputEvent| {
            state.set(e.target_unchecked_into::<HtmlInputElement>().value());
            invalid.set(false);
        })
    };
    // Keyboard events must not trigger the global keybindings while typing.
    let onkeyup = {
        let add = add.clone();
        Callback::from(move |e: KeyboardEvent| {
            e.stop_propagation();
            if e.key().eq_ignore_ascii_case("enter") {
                add.emit(());
            }
        })
    };

    let symmetries = props.symmetries.iter().enumerate().map(|(index, symmetry)| {
        let remove = props
            .dispatch
            .reform(move |_| proof::Action::EditSignature(SignatureEdit::RemoveSymmetry(index)).into());
        let directions = (symmetry.directions.iter())
            .map(|j| format!("x{}", subscript(*j)))
            .collect::<Vec<_>>()
            .join(", ");
        html! {
            <li class="signature__symmetry">
                <span class="signature__symmetry-name">{format!("{}: {directions}", symmetry.name)}</span>
                <span class="signature__symmetry-remove" title="Remove symmetry" onclick={remove}>
                    <Icon name="close" size={IconSize::Icon18} />
                </span>
            </li>
        }
    });

    html! {
        <fieldset class="signature__symmetries">
            <legend>{"Symmetries"}</legend>
            <ul>{for symmetries}</ul>
            <div class={classes!("signature__symmetry-form", (*invalid).then_some("invalid"))}>
                <input
                    type="text"
                    placeholder="name"
                    value={(*name).clone()}
                    oninput={oninput(&name, &invalid)}
                    onkeyup={onkeyup.clone()}
                />
                <input
                    type="text"
                    placeholder="directions"
                    title="Reflected directions, numbered from the innermost, e.g. 1,3"
                    value={(*directions).clone()}
                    oninput={oninput(&directions, &invalid)}
                    {onkeyup}
                />
                <button onclick={add.reform(|_| ())}>{"Add"}</button>
            </div>
        </fieldset>
    }
}
