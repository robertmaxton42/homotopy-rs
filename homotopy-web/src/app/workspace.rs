use homotopy_core::{Generator, SliceIndex};
use path_control::PathControl;
use slice_control::SliceControl;
use view_control::ViewControl;
use yew::prelude::*;

use crate::{
    app::{
        attach_inference,
        diagram_gl::DiagramGl,
        diagram_svg::{
            highlight::{highlight_attachment, highlight_slice},
            DiagramSvg,
        },
        info::get_onboarding_message,
        tex::TexSpan,
        AppSettings,
    },
    components::panzoom::PanZoomComponent,
    model::{
        proof::{self, AttachOption, Metadata, Signature, Workspace},
        Action,
    },
};

mod path_control;
mod slice_control;
mod view_control;

// TODO: Workspace rerendering when panzoom is changed needs to be smoother.

#[derive(Clone, PartialEq, Properties)]
pub struct Props {
    pub workspace: Option<Workspace>,
    pub dispatch: Callback<Action>,
    pub signature: Signature,
    pub metadata: Metadata,
    pub attachment_highlight: Option<AttachOption>,
    pub slice_highlight: Option<SliceIndex>,
    /// The generator picked to be attached by the next click on the diagram, if there is one.
    pub armed: Option<Generator>,
}

#[function_component]
pub fn WorkspaceView(props: &Props) -> Html {
    let diagram_ref = NodeRef::default();
    // Counts the times that a diagram was drawn after waiting for its layout, which makes what is
    // sized to the diagram (the slice buttons) look at it again.
    let laid_out = use_state(|| 0_u32);
    let on_laid_out = {
        let laid_out = laid_out.clone();
        Callback::from(move |()| laid_out.set(*laid_out + 1))
    };
    let workspace = &props.workspace;
    let diagram_view = if workspace.is_some() {
        view_diagram(props, diagram_ref.clone(), on_laid_out)
    } else {
        // Show onboarding info if workspace and signature is empty
        get_onboarding_message()
    };
    let project_title = html! {
        <TexSpan
            class="workspace__project-title"
            error_color="#c004"
            raw_tex={props.metadata.title.clone().unwrap_or_default()}
        />
    };
    let slice_buttons = match workspace {
        Some(ref ws) if matches!(ws.view.dimension(), 1 | 2) => {
            let diagram = ws.visible_diagram();
            html! {
                <SliceControl
                    number_slices={diagram.size().unwrap()}
                    descend_slice={props.dispatch.reform(Action::Proof).reform(proof::Action::DescendSlice)}
                    diagram_ref={diagram_ref}
                    measure={*laid_out}
                    on_hover={props.dispatch.reform(Action::HighlightSlice)}
                />
            }
        }
        _ => Default::default(),
    };
    let toolbar = workspace.as_ref().map_or_else(
        Default::default,
        |ws| {
            html! {
                <div class="workspace__toolbar">
                    <PathControl
                        path={ws.path.clone()}
                        view={ws.view}
                        ascend_slice={props.dispatch.reform(Action::Proof).reform(proof::Action::AscendSlice)}
                        increase_view={props.dispatch.reform(Action::Proof).reform(proof::Action::IncreaseView)}
                        decrease_view={props.dispatch.reform(Action::Proof).reform(proof::Action::DecreaseView)}
                        dimension={ws.diagram.dimension()}
                    />
                    <ViewControl />
                </div>
            }
        },
    );

    html! {
        <div class="workspace">
            {diagram_view}
            {slice_buttons}
            <div class="workspace__overlay-top">
                {project_title}
                {toolbar}
            </div>
        </div>
    }
}

fn view_diagram(props: &Props, diagram_ref: NodeRef, on_laid_out: Callback<()>) -> Html {
    if let Some(ref ws) = props.workspace {
        match ws.view.dimension() {
            0 => view_diagram_svg::<0>(props, diagram_ref, on_laid_out),
            1 => view_diagram_svg::<1>(props, diagram_ref, on_laid_out),
            2 => view_diagram_svg::<2>(props, diagram_ref, on_laid_out),
            _ => {
                html! {
                    <DiagramGl
                        diagram={ws.visible_diagram()}
                        signature={props.signature.clone()}
                        view={ws.view}
                    />
                }
            }
        }
    } else {
        Default::default()
    }
}

#[allow(clippy::let_underscore_must_use)]
fn view_diagram_svg<const N: usize>(
    props: &Props,
    diagram_ref: NodeRef,
    on_laid_out: Callback<()>,
) -> Html {
    let armed = props.armed;
    let on_select = props.dispatch.reform(move |p| {
        let (weak_units, limits) = (AppSettings::get_weak_units(), attach_inference::limits());
        match armed {
            // A click attaches the generator that was picked, if it can be: for as long as it
            // takes (`Esc` ends it), whatever the time that a generator is given in a search
            // of everything.
            Some(generator) => Action::AttachGenerator(
                generator,
                p,
                weak_units,
                attach_inference::limits_for_generator(),
            ),
            None if AppSettings::get_offer_definitions() => {
                Action::SelectCell(p, weak_units, limits)
            }
            None => Action::SelectPoint(p, weak_units, limits),
        }
    });
    let on_attach = props.dispatch.reform(|(generator, point)| {
        Action::AttachGenerator(
            generator,
            point,
            AppSettings::get_weak_units(),
            attach_inference::limits_for_generator(),
        )
    });
    let on_homotopy = props
        .dispatch
        .reform(|homotopy| Action::Proof(proof::Action::Homotopy(homotopy)));

    if let Some(ref ws) = props.workspace {
        let attachment_highlight = props
            .attachment_highlight
            .as_ref()
            .map(|option| highlight_attachment::<N>(ws.path.len(), option));
        let slice_highlight = props.slice_highlight.map(highlight_slice::<N>);
        let highlight = attachment_highlight.or(slice_highlight);
        html! {
            <PanZoomComponent on_scroll={props.dispatch.reform(Action::Proof).reform(proof::Action::SwitchSlice)}>
                <DiagramSvg<N>
                    diagram={ws.visible_diagram()}
                    id="workspace__diagram"
                    signature={props.signature.clone()}
                    on_select={on_select}
                    accepts_drops=true
                    sheets=true
                    on_attach={on_attach}
                    on_homotopy={on_homotopy}
                    highlight={highlight}
                    diagram_ref={diagram_ref}
                    on_laid_out={on_laid_out}
                />
            </PanZoomComponent>
        }
    } else {
        Default::default()
    }
}
