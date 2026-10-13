use folder::FolderView;
use homotopy_core::{signature::Signature as _, Diagram};
use homotopy_model::proof;
use symmetries::SymmetriesView;
use yew::prelude::*;
use yew_macro::function_component;

use crate::{
    app::sidebar::DrawerViewSize,
    model::{proof::Signature, Action},
};

mod folder;
mod item;
mod symmetries;

#[derive(Clone, PartialEq, Properties)]
pub struct Props {
    pub signature: Signature,
    pub dispatch: Callback<Action>,
    pub drawer_view_size: DrawerViewSize,
    /// The diagram in the workspace, if any.
    pub workspace: Option<Diagram>,
}

#[function_component]
pub fn SignatureView(props: &Props) -> Html {
    // TODO: Add search
    // TODO: On mobile, drag to the side to delete
    let suspension_controls = if props.signature.has_generators() {
        html! {
            <div>
                <button onclick={props.dispatch.reform(move |_| proof::Action::SuspendSignature.into())}>{"Suspend"}</button>
            </div>
        }
    } else {
        Default::default()
    };
    html! {
        <div>
            {suspension_controls}
            <FolderView
                dispatch={props.dispatch.clone()}
                signature={props.signature.clone()}
                drawer_view_size={props.drawer_view_size}
                workspace={props.workspace.clone()}
            />
            <SymmetriesView
                dispatch={props.dispatch.clone()}
                symmetries={props.signature.symmetries().to_vec()}
            />
        </div>
    }
}
