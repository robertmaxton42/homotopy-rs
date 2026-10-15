use std::{
    convert::{From, Into, TryInto},
    f32::consts::PI,
    rc::Rc,
};

use euclid::{
    default::{Point2D, Size2D, Transform2D, Vector2D},
    Angle,
};
use homotopy_core::{
    common::Direction,
    complex::{make_complex, Simplex},
    contraction::Bias,
    layout::Layout,
    projection::{Depths, Projection, Shading},
    rewrite::RewriteN,
    signature::Signature as _,
    Boundary, Diagram, DiagramN, Generator, Height, SliceIndex,
};
use homotopy_graphics::{
    style::{GeneratorStyle, SignatureStyleData, Stripes, VertexShape},
    svg::{
        color_index, generator_class_from_diagram_dim,
        render::{ActionRegion, GraphicElement},
        shape::{path_to_svg, project_2d, Point, Shape},
        PLACEHOLDER_HATCH_ID, PLACEHOLDER_HATCH_LIGHT_ID,
    },
};
use homotopy_model::sheets::Choice;
use wasm_bindgen::JsCast;
use web_sys::Element;
use yew::prelude::*;

use self::{
    highlight::{HighlightKind, HighlightSvg},
    sheets::SheetCommand,
};
use super::{
    attach_drag::{self, DragSignal},
    attach_target, tooltip,
};
use crate::{
    app::settings::{AppSettings, AppSettingsKey},
    components::{delta::CallbackIdx, read_touch_list_abs, Finger},
    layout::{self, Asked, Ticket},
    model::proof::{
        homotopy::{Contract, Expand, Homotopy},
        Signature,
    },
};

pub mod highlight;
mod sheets;

pub use self::sheets::{debug_command, debug_summary};

pub struct DiagramSvg<const N: usize> {
    /// What is shown. While the layout of the diagram of the properties is being made (`pending`)
    /// it is for an earlier diagram, or there is nothing yet.
    prepared: Option<PreparedDiagram<N>>,
    /// The layout that is being made elsewhere, of the diagram of the properties. While there is
    /// one, what is shown is stale and nothing about it is to be acted on: points of the old
    /// diagram are not points of the new one.
    pending: Option<Ticket>,
    /// Whether the layout has just arrived, and `on_laid_out` is to be called once it is drawn.
    announce: bool,
    drag_start: Option<Point2D<f32>>,
    /// The sheet that was chosen when the button went down, for the drag that may follow: the
    /// pointer moves before the drag is recognised, and moving to another shape forgets `choice`.
    drag_choice: Option<Choice<N>>,
    /// What the pointer is over, which the tooltip shown is for.
    hovered: Option<Simplex<N>>,
    /// The part of the diagram that a generator being dragged over it would be attached at.
    drop_target: Option<Simplex<N>>,
    /// Everything that is under the pointer, the one on top first (`hovered` is the first). Only
    /// kept for a diagram that has sheets to choose between (`sheets` of the properties).
    hits: Vec<Simplex<N>>,
    /// The sheet chosen for the place the pointer is over, if one was.
    choice: Option<Choice<N>>,
    /// Where the pointer is, in the window, for the tooltip.
    pointer: (f64, f64),
    /// Whether the pointer is over the diagram (or a generator being dragged is).
    hovering: bool,
    /// The keys that choose a sheet.
    keys: sheets::Keys,
    /// How a generator dragged with the pointer is told about this diagram (see
    /// [`attach_drag`]), if it takes drops.
    drag_sink: Option<usize>,
    /// The settings that change what is drawn on the diagram, which draw it again.
    settings: Vec<CallbackIdx>,
}

/// The settings that the drawing of the diagram depends on.
const DRAWING_SETTINGS: &[AppSettingsKey] = &[
    AppSettingsKey::sheet_outlines,
    AppSettingsKey::sheet_shading,
];

/// How many steps of shading there are for the number of sheets behind a surface (0 is none).
const SHADING_STEPS: usize = 4;

#[derive(Clone, PartialEq, Properties)]
pub struct DiagramSvgProps<const N: usize> {
    pub diagram: Diagram,
    pub id: String,
    pub signature: Signature,
    #[prop_or_default]
    pub style: RenderStyle,
    #[prop_or_default]
    pub on_select: Callback<Vec<SliceIndex>>,
    #[prop_or_default]
    pub on_homotopy: Callback<Homotopy>,
    #[prop_or_default]
    pub highlight: Option<HighlightSvg<N>>,
    #[prop_or_default]
    pub max_width: Option<f32>,
    #[prop_or_default]
    pub max_height: Option<f32>,
    #[prop_or_default]
    pub diagram_ref: NodeRef,
    /// Called when the diagram has been drawn after waiting for its layout: what is around it,
    /// and was sized to what was drawn before, may want to look again.
    #[prop_or_default]
    pub on_laid_out: Callback<()>,
    /// Whether a generator dragged from the signature can be dropped on the diagram, to attach it
    /// at the point where it is dropped, which is then given to `on_attach`.
    #[prop_or_default]
    pub accepts_drops: bool,
    #[prop_or_default]
    pub on_attach: Callback<(Generator, Vec<SliceIndex>)>,
    /// Whether the cells behind a position of the view (its sheets) can be chosen between, with
    /// Shift+wheel or `[` and `]` over the diagram, so that what is clicked or dropped is at the
    /// sheet (see `homotopy_model::sheets`).
    #[prop_or_default]
    pub sheets: bool,
}

#[allow(clippy::enum_variant_names)]
pub enum DiagramSvgMessage<const N: usize> {
    OnMouseDown(Point2D<f32>),
    OnMouseMove(Point2D<f32>, bool),
    OnMouseUp,
    OnMouseLeave,
    OnTouchUpdate(Vec<(Finger, Point2D<f32>)>),
    OnTouchMove(Vec<(Finger, Point2D<f32>)>),
    /// The layout that was asked for is there.
    Layout(Ticket, Rc<Layout<N>>),
    /// A generator is being dragged over the diagram, with the pointer at this place.
    OnDragOver(Point2D<f32>),
    OnDragLeave,
    /// A generator was dropped on the diagram, with the pointer at this place.
    OnDrop(Generator, Point2D<f32>),
    /// Something to do about the sheets: a key, the wheel, or a debug hook.
    Sheet(SheetCommand<N>),
    /// A setting that changes the drawing was changed.
    Setting,
}

/// The computed properties of a diagram that are potentially expensive to compute but can be
/// cached if the diagram does not change.
struct PreparedDiagram<const N: usize> {
    /// The diagram that this is for.
    diagram: Diagram,
    graphic: Vec<GraphicElement<N>>,
    /// For each of `graphic`, how many steps its surface is shaded (0: not at all). All 0 if the
    /// surfaces are not shaded (`shaded`).
    steps: Vec<usize>,
    /// Whether it was prepared for the surfaces to be shaded by the sheets behind them.
    shaded: bool,
    actions: Vec<(Simplex<N>, Shape)>,
    depths: Depths<N>,
    projection: Projection<N>,
    layout: Rc<Layout<N>>,

    /// The width and height of the diagram image in pixels.
    ///
    /// This is not the size of the diagram as it appears on the screen, since
    /// it might be zoomed by any parent component.
    dimensions: Size2D<f32>,

    /// Transform coordinates in the diagram layout to coordinates in the SVG image. In
    /// particular, the vertical direction is flipped so that diagrams are read from
    /// bottom to top.
    transform: Transform2D<f32>,
}

impl<const N: usize> PreparedDiagram<N> {
    /// Prepares the diagram for rendering, given its layout, which is the part that can take
    /// long (see [`layout`]).
    fn new(diagram: &Diagram, layout: Rc<Layout<N>>, style: RenderStyle, shade: bool) -> Self {
        assert!(diagram.dimension() >= N);

        let performance = web_sys::window().unwrap().performance().unwrap();
        performance.mark("startPrepareDiagram").unwrap();

        let complex = make_complex(diagram);
        let depths = Depths::new(diagram).unwrap();
        let projection = Projection::new(diagram, &layout, &depths).unwrap();
        // How many sheets pass through a surface: those of the region it belongs to, which is
        // the position that hovering over it lists the sheets of (see `Shading`).
        let shading = shade.then(|| {
            Shading::new(
                &depths,
                complex
                    .iter()
                    .filter_map(|(simplex, visible)| match simplex {
                        Simplex::Surface(corners) if *visible => Some(*corners),
                        _ => None,
                    }),
                SHADING_STEPS,
            )
        });
        let (graphic, steps): (Vec<_>, Vec<_>) =
            GraphicElement::build_shaded(&complex, &layout, &projection, &depths, |corners| {
                shading
                    .as_ref()
                    .map_or(0, |shading| shading.step(&depths, corners))
            })
            .into_iter()
            .unzip();
        let actions = ActionRegion::build(&complex, &layout, &projection);

        let dimensions = Point::from(project_2d(layout[&[Boundary::Target.into(); N]]))
            .max((1.0, 1.0).into())
            .to_vector()
            .to_size()
            * style.scale;

        let transform = Transform2D::scale(style.scale, -style.scale)
            .then_translate((0.0, dimensions.height).into());

        let actions = actions
            .into_iter()
            .map(|action| {
                let shape = action
                    .transformed(&transform)
                    .to_shape(style.wire_thickness, style.point_radius * 2.0);
                ((&action).into(), shape)
            })
            .collect();

        performance.mark("stopPrepareDiagram").unwrap();
        performance
            .measure_with_start_mark_and_end_mark(
                "prepareDiagram",
                "startPrepareDiagram",
                "stopPrepareDiagram",
            )
            .unwrap();
        tracing::info!(
            "preparing diagram for rendering took {}ms",
            js_sys::Reflect::get(
                &performance
                    .get_entries_by_name_with_entry_type("prepareDiagram", "measure")
                    .get(0),
                &wasm_bindgen::JsValue::from_str("duration")
            )
            .unwrap()
            .as_f64()
            .unwrap()
        );

        performance.clear_marks();
        performance.clear_measures();

        Self {
            diagram: diagram.clone(),
            graphic,
            steps,
            shaded: shade,
            actions,
            depths,
            projection,
            layout,
            dimensions,
            transform,
        }
    }
}

impl<const N: usize> Component for DiagramSvg<N> {
    type Message = DiagramSvgMessage<N>;
    type Properties = DiagramSvgProps<N>;

    fn create(ctx: &Context<Self>) -> Self {
        let mut this = Self {
            prepared: None,
            pending: None,
            announce: false,
            drag_start: Default::default(),
            drag_choice: None,
            hovered: None,
            drop_target: None,
            hits: vec![],
            choice: None,
            pointer: (0.0, 0.0),
            hovering: false,
            keys: sheets::Keys::default(),
            drag_sink: ctx.props().accepts_drops.then(|| {
                attach_drag::register(
                    ctx.props().diagram_ref.clone(),
                    ctx.link().callback(|signal| match signal {
                        DragSignal::Over(x, y) => DiagramSvgMessage::OnDragOver((x, y).into()),
                        DragSignal::Leave => DiagramSvgMessage::OnDragLeave,
                        DragSignal::Drop(generator, x, y) => {
                            DiagramSvgMessage::OnDrop(generator, (x, y).into())
                        }
                    }),
                )
            }),
            settings: AppSettings::subscribe(
                DRAWING_SETTINGS,
                ctx.link().callback(|_| DiagramSvgMessage::Setting),
            ),
        };
        this.install_sheets(ctx);
        this.prepare(ctx);
        this
    }

    fn update(&mut self, ctx: &Context<Self>, msg: Self::Message) -> bool {
        match msg {
            DiagramSvgMessage::OnMouseDown(point) => {
                self.drag_start = Some(point);
                self.drag_choice = self.choice.clone();
                self.hide_tooltip();
            }
            DiagramSvgMessage::OnMouseMove(point, shift_key) => {
                self.pointer_move(ctx, point, shift_key);
                // Not while a button is down: that is a click or a drag, not hovering.
                if self.drag_start.is_none() {
                    self.update_tooltip(ctx, point);
                }
            }
            DiagramSvgMessage::Sheet(command) => {
                self.sheet_command(ctx, command);
            }
            DiagramSvgMessage::Setting => {
                // The surfaces are kept apart by their shading when the diagram is prepared.
                if self
                    .current()
                    .is_some_and(|p| p.shaded != Self::shading(ctx))
                {
                    self.hide_tooltip();
                    self.prepare(ctx);
                }
                return true;
            }
            DiagramSvgMessage::OnMouseUp => {
                self.pointer_stop(ctx);
            }
            DiagramSvgMessage::OnMouseLeave => {
                self.hide_tooltip();
                self.leave(ctx);
            }
            DiagramSvgMessage::OnTouchUpdate(touches) => {
                if self.drag_start.is_none() && touches.len() == 1 {
                    self.drag_start = Some(touches[0].1);
                    self.drag_choice = None;
                } else if touches.is_empty() {
                    self.drag_start = None;
                }
            }
            DiagramSvgMessage::OnTouchMove(touches) => {
                if touches.len() == 1 {
                    self.pointer_move(ctx, touches[0].1, false);
                }
            }
            DiagramSvgMessage::OnDragOver(point) => {
                // Only the part under the pointer is looked up: nothing is searched for before
                // the drop. The sheet that was chosen for it is where it would be dropped.
                self.update_tooltip(ctx, point);
                let target = self.hit_to_act_on(&self.hits_at(ctx, point));
                if target == self.drop_target {
                    return false;
                }
                self.drop_target = target;
                return true;
            }
            DiagramSvgMessage::OnDragLeave => {
                self.hide_tooltip();
                self.leave(ctx);
                return self.drop_target.take().is_some();
            }
            DiagramSvgMessage::OnDrop(generator, point) => {
                self.drop_target = None;
                self.hide_tooltip();
                // (Not while the diagram waits for its layout: there is nothing to drop on.)
                if let Some(point) = self.point_at(ctx, point) {
                    ctx.props().on_attach.emit((generator, point));
                }
                return true;
            }
            DiagramSvgMessage::Layout(ticket, layout) => {
                // The layout of a diagram that has been replaced since is of no use.
                if self.pending != Some(ticket) {
                    return false;
                }
                self.pending = None;
                self.announce = true;
                self.drop_target = None;
                self.hide_tooltip();
                self.choice = None;
                self.hits.clear();
                self.prepared = Some(PreparedDiagram::new(
                    &ctx.props().diagram,
                    layout,
                    ctx.props().style,
                    Self::shading(ctx),
                ));
                return true;
            }
        }
        // Moving the pointer or hovering does not change what the diagram looks like.
        false
    }

    fn changed(&mut self, ctx: &Context<Self>, old_props: &Self::Properties) -> bool {
        if old_props.diagram != ctx.props().diagram {
            // What the tooltip said may no longer be there.
            self.hide_tooltip();
            self.drop_target = None;
            self.choice = None;
            self.hits.clear();
            // re-layout
            self.prepare(ctx);
        } else if old_props.style != ctx.props().style {
            self.hide_tooltip();
            // Not the layout, which is of the diagram only. If it is not there yet, it will be
            // prepared in the style of the time.
            if let Some(prepared) = self.prepared.as_ref().filter(|_| self.pending.is_none()) {
                let layout = Rc::clone(&prepared.layout);
                self.prepared = Some(PreparedDiagram::new(
                    &ctx.props().diagram,
                    layout,
                    ctx.props().style,
                    Self::shading(ctx),
                ));
            }
        }
        true
    }

    fn rendered(&mut self, ctx: &Context<Self>, _first_render: bool) {
        // (What the view drew again is not dimmed, whatever is chosen.)
        self.apply_dimming(ctx);
        if let Some(sink) = self.drag_sink {
            attach_drag::set_element(sink, &ctx.props().diagram_ref);
        }
        if std::mem::take(&mut self.announce) {
            ctx.props().on_laid_out.emit(());
        }
    }

    fn destroy(&mut self, _ctx: &Context<Self>) {
        self.hide_tooltip();
        self.keys.remove();
        if let Some(sink) = self.drag_sink.take() {
            attach_drag::unregister(sink);
        }
        AppSettings::unsubscribe(DRAWING_SETTINGS, &self.settings);
        if let Some(ticket) = self.pending.take() {
            layout::cancel(ticket);
        }
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        // Nothing yet: the first layout of a large diagram is still being made.
        let Some(prepared) = &self.prepared else {
            return html! {
                <svg
                    xmlns={"http://www.w3.org/2000/svg"}
                    class="diagram-svg--stale"
                    width="1"
                    height="1"
                    ref={ctx.props().diagram_ref.clone()}
                />
            };
        };
        let size = prepared.dimensions;

        let width = match ctx.props().max_width {
            Some(width) => width.min(size.width),
            None => size.width,
        };

        let height = match ctx.props().max_height {
            Some(height) => height.min(size.height),
            None => size.height,
        };

        let on_mouse_down = {
            let link = ctx.link().clone();
            Callback::from(move |e: MouseEvent| {
                if !e.alt_key() {
                    let x = e.client_x() as f32;
                    let y = e.client_y() as f32;
                    link.send_message(DiagramSvgMessage::OnMouseDown((x, y).into()));
                }
            })
        };

        let on_mouse_move = {
            let link = ctx.link().clone();
            Callback::from(move |e: MouseEvent| {
                if !e.alt_key() {
                    let x = e.client_x() as f32;
                    let y = e.client_y() as f32;
                    link.send_message(DiagramSvgMessage::OnMouseMove((x, y).into(), e.shift_key()));
                }
            })
        };

        let on_mouse_up = {
            let link = ctx.link().clone();
            Callback::from(move |_e: MouseEvent| {
                link.send_message(DiagramSvgMessage::OnMouseUp);
            })
        };

        // Not `mouseout`, which also fires when the pointer moves from one element of the
        // diagram to another, and would take the tooltip down every time.
        let on_mouse_leave = {
            let link = ctx.link().clone();
            Callback::from(move |_e: MouseEvent| {
                link.send_message(DiagramSvgMessage::OnMouseLeave);
            })
        };

        let on_touch_move = {
            let link = ctx.link().clone();
            Callback::from(move |e: TouchEvent| {
                let touches = read_touch_list_abs(&e.touches())
                    .map(|(finger, point)| (finger, point.cast()))
                    .collect();
                link.send_message(DiagramSvgMessage::OnTouchMove(touches));
            })
        };

        let on_touch_update = {
            let link = ctx.link().clone();
            Callback::from(move |e: TouchEvent| {
                let touches = read_touch_list_abs(&e.touches())
                    .map(|(finger, point)| (finger, point.cast()))
                    .collect();
                link.send_message(DiagramSvgMessage::OnTouchUpdate(touches));
            })
        };

        // A generator dragged from the signature: taken on the diagram if it might be attached
        // to it (as the drag started by working out, without a search) and the diagram is ready
        // to be acted on, which is not while it waits for its layout.
        let (on_drag_over, on_drag_leave, on_drop) = if ctx.props().accepts_drops
            && self.pending.is_none()
        {
            let over = {
                let link = ctx.link().clone();
                Callback::from(move |e: DragEvent| {
                    let (Some(dragged), Some(transfer)) =
                        (attach_target::dragged(), e.data_transfer())
                    else {
                        return;
                    };
                    if dragged.verdict.is_err() || !attach_target::carries_generator(&transfer) {
                        return;
                    }
                    e.prevent_default();
                    transfer.set_drop_effect("copy");
                    let (x, y) = (e.client_x() as f32, e.client_y() as f32);
                    link.send_message(DiagramSvgMessage::OnDragOver((x, y).into()));
                })
            };
            let leave = {
                let link = ctx.link().clone();
                let diagram_ref = ctx.props().diagram_ref.clone();
                Callback::from(move |e: DragEvent| {
                    // Moving from one part of the diagram to another leaves the one: that is not
                    // leaving the diagram.
                    let moved_within = e
                        .related_target()
                        .and_then(|target| target.dyn_into::<web_sys::Node>().ok())
                        .zip(diagram_ref.cast::<web_sys::Node>())
                        .is_some_and(|(target, svg)| svg.contains(Some(&target)));
                    if !moved_within {
                        link.send_message(DiagramSvgMessage::OnDragLeave);
                    }
                })
            };
            let drop = {
                let link = ctx.link().clone();
                Callback::from(move |e: DragEvent| {
                    let Some(generator) =
                        e.data_transfer().and_then(|t| attach_target::dropped(&t))
                    else {
                        return;
                    };
                    e.prevent_default();
                    // The drag is over, and the source may not be told it was dropped.
                    attach_target::end();
                    let (x, y) = (e.client_x() as f32, e.client_y() as f32);
                    link.send_message(DiagramSvgMessage::OnDrop(generator, (x, y).into()));
                })
            };
            (over, leave, drop)
        } else {
            Default::default()
        };

        // Shift+wheel chooses a sheet, where there are some, and is not a change of slice.
        let on_wheel = if ctx.props().sheets {
            let link = ctx.link().clone();
            let wanted = self.keys.wanted();
            Callback::from(move |e: WheelEvent| {
                if !e.shift_key() || !wanted.get() {
                    return;
                }
                // (Browsers turn the wheel into a horizontal one with Shift held.)
                let delta = if e.delta_y() == 0.0 {
                    e.delta_x()
                } else {
                    e.delta_y()
                };
                if delta == 0.0 {
                    return;
                }
                e.prevent_default();
                e.stop_propagation();
                link.send_message(DiagramSvgMessage::Sheet(SheetCommand::Step(
                    if delta > 0.0 { 1 } else { -1 },
                )));
            })
        } else {
            Callback::noop()
        };

        // TODO: Do not redraw diagram when highlight changes!
        // TODO: Do not redraw diagram for drags.

        tracing::info!("redrawing diagram");

        html! {
            <svg
                xmlns={"http://www.w3.org/2000/svg"}
                class={self.pending.is_some().then_some("diagram-svg--stale")}
                width={width.to_string()}
                height={height.to_string()}
                viewBox={format!("0 0 {} {}", size.width, size.height)}
                onmousedown={on_mouse_down}
                onmouseup={on_mouse_up}
                onmousemove={on_mouse_move}
                onmouseleave={on_mouse_leave}
                ontouchmove={on_touch_move}
                ontouchstart={on_touch_update.clone()}
                ontouchend={on_touch_update.clone()}
                ontouchcancel={on_touch_update.clone()}
                ondragover={on_drag_over}
                ondragleave={on_drag_leave}
                ondrop={on_drop}
                onwheel={on_wheel}
                ref={ctx.props().diagram_ref.clone()}
            >
                {Self::view_placeholder_hatch(ctx, prepared)}
                {prepared.graphic.iter().enumerate().map(|(i, e)| Self::view_element(ctx, prepared, i, e)).collect::<Html>()}
                {Self::view_sheet_outlines(ctx, prepared)}
                {self.view_highlight(ctx)}
                {self.view_drop_target(prepared)}
            </svg>
        }
    }
}

impl<const N: usize> DiagramSvg<N> {
    /// Prepares the diagram of the properties for rendering. If its layout is not there at once,
    /// what was prepared before is kept for now, and replaced when the layout is.
    fn prepare(&mut self, ctx: &Context<Self>) {
        if let Some(ticket) = self.pending.take() {
            layout::cancel(ticket);
        }
        let props = ctx.props();
        let link = ctx.link().clone();
        let on_layout = Callback::from(move |(ticket, layout)| {
            link.send_message(DiagramSvgMessage::Layout(ticket, layout));
        });
        match layout::ask::<N>(&props.diagram, on_layout) {
            Asked::Ready(layout) => {
                self.prepared = Some(PreparedDiagram::new(
                    &props.diagram,
                    layout,
                    props.style,
                    Self::shading(ctx),
                ));
            }
            Asked::Pending(ticket) => self.pending = Some(ticket),
        }
    }

    /// What is prepared, if it is for the diagram that is to be shown, and so can be acted on.
    fn current(&self) -> Option<&PreparedDiagram<N>> {
        self.prepared.as_ref().filter(|_| self.pending.is_none())
    }

    /// Transform coordinates on the screen (such as those in `MouseEvent`s) to coordinates in the
    /// SVG image. This incorporates translation and zoom of the diagram component.
    fn transform_screen_to_image(
        prepared: &PreparedDiagram<N>,
        ctx: &Context<Self>,
    ) -> Transform2D<f32> {
        let rect = ctx
            .props()
            .diagram_ref
            .cast::<Element>()
            .unwrap()
            .get_bounding_client_rect();

        let screen_size = Size2D::new(rect.width() as f32, rect.height() as f32);
        let image_size = prepared.dimensions;

        Transform2D::translation(-rect.left() as f32, -rect.top() as f32).then_scale(
            image_size.width / screen_size.width,
            image_size.height / screen_size.height,
        )
    }

    fn is_placeholder(ctx: &Context<Self>, generator: Generator) -> bool {
        ctx.props()
            .signature
            .generator_style(generator)
            .is_some_and(GeneratorStyle::is_placeholder)
    }

    /// The id of the hatch pattern for stripes in the given colour.
    fn hatch_id(ctx: &Context<Self>, stripes: Stripes) -> String {
        let id = match stripes {
            Stripes::Dark => PLACEHOLDER_HATCH_ID,
            Stripes::Light => PLACEHOLDER_HATCH_LIGHT_ID,
        };
        format!("{}-{id}", ctx.props().id)
    }

    /// The colour of the stripes of a placeholder surface: the one that its colour, as it is drawn
    /// in this diagram, does not hide.
    fn stripes_of(ctx: &Context<Self>, element: &GraphicElement<N>, dimension: usize) -> Stripes {
        let generator = element.generator();
        let c = color_index(generator, dimension, element.clone().into());
        ctx.props()
            .signature
            .generator_style(generator)
            .map_or(Stripes::Dark, |style| {
                style.color().lighten(c, element.orientation()).stripes()
            })
    }

    /// Defines the pattern with which the surfaces of placeholders are hatched, if there are any.
    fn view_placeholder_hatch(ctx: &Context<Self>, prepared: &PreparedDiagram<N>) -> Html {
        let has_placeholder_surface = prepared.graphic.iter().any(|element| {
            matches!(element, GraphicElement::Surface(d, _) if Self::is_placeholder(ctx, d.generator))
        });
        if !has_placeholder_surface {
            return html! {};
        }

        let pattern = |stripes: Stripes| {
            let (colour, opacity) = match stripes {
                Stripes::Dark => ("black", "0.55"),
                Stripes::Light => ("white", "0.8"),
            };
            html! {
                <pattern
                    id={Self::hatch_id(ctx, stripes)}
                    width="7"
                    height="7"
                    patternUnits="userSpaceOnUse"
                    patternTransform="rotate(45)"
                >
                    <line
                        x1="0"
                        y1="0"
                        x2="0"
                        y2="7"
                        stroke={colour}
                        stroke-width="2"
                        stroke-opacity={opacity}
                    />
                </pattern>
            }
        };

        html! {
            <defs>
                {pattern(Stripes::Dark)}
                {pattern(Stripes::Light)}
            </defs>
        }
    }

    /// Creates the SVG elements for the diagram.
    fn view_element(
        ctx: &Context<Self>,
        prepared: &PreparedDiagram<N>,
        index: usize,
        element: &GraphicElement<N>,
    ) -> Html {
        let class = generator_class_from_diagram_dim(
            element.generator(),
            element.orientation(),
            prepared.diagram.dimension(),
            element.clone().into(),
        );

        match element {
            GraphicElement::Surface(d, path) => {
                let path = path_to_svg(&path.clone().transformed(&prepared.transform));
                // How many sheets are behind the surface, in steps, for the stylesheet to shade
                // (and not set at all for the thinnest, which is as it always was).
                let thickness = Some(prepared.steps[index])
                    .filter(|&step| step > 0)
                    .map(|step| step.to_string());
                if Self::is_placeholder(ctx, d.generator) {
                    // Placeholders are translucent (by their stylesheet) and also hatched, in
                    // stripes that show on their colour.
                    let stripes = Self::stripes_of(ctx, element, prepared.diagram.dimension());
                    html! {
                        <>
                            <path d={path.clone()} class={class} data-thickness={thickness} />
                            <path
                                d={path}
                                fill={format!("url(#{})", Self::hatch_id(ctx, stripes))}
                                stroke="none"
                            />
                        </>
                    }
                } else {
                    html! {
                        <path d={path} class={class} data-thickness={thickness} />
                    }
                }
            }
            GraphicElement::Wire(_, depth, path, mask) => {
                let path = path_to_svg(&path.clone().transformed(&prepared.transform));
                // Which sheet the wire is on, for dimming those that are not the one chosen.
                let depth = depth.to_string();

                if mask.is_empty() {
                    html! {
                        <path
                            d={path}
                            class={class}
                            data-depth={depth}
                            stroke-width={ctx.props().style.wire_thickness.to_string()}
                        />
                    }
                } else {
                    let mask_paths: Html = mask
                        .iter()
                        .map(|mask_path| {
                            html! {
                                <path
                                    d={path_to_svg(&mask_path.clone().transformed(&prepared.transform))}
                                    stroke-width={(ctx.props().style.wire_thickness * 2.0).to_string()}
                                    fill="none"
                                    stroke="black"
                                    stroke-linecap="round"
                                />
                            }
                        })
                        .collect();

                    let mask_id = format!("{}-mask-{index}", ctx.props().id);

                    html! {
                        <>
                            <defs>
                                <mask maskUnits="userSpaceOnUse" id={mask_id.clone()}>
                                    <rect width="100%" height="100%" fill="white" />
                                    {mask_paths}
                                </mask>
                            </defs>
                            <path
                                d={path}
                                class={class}
                                data-depth={depth}
                                stroke-width={ctx.props().style.wire_thickness.to_string()}
                                mask={format!("url(#{mask_id})")}
                            />
                        </>
                    }
                }
            }
            GraphicElement::Point(d, point) => {
                use VertexShape::{Circle, Square};
                let point = prepared.transform.transform_point(*point);
                let radius = ctx.props().style.point_radius;
                let shape = if let Some(info) = ctx.props().signature.generator_info(d.generator) {
                    info.shape.clone()
                } else {
                    Default::default()
                };
                match shape {
                    Circle => html! {
                        <circle
                            r={radius.to_string()}
                            cx={point.x.to_string()}
                            cy={point.y.to_string()}
                            class={class} />
                    },
                    Square => html! {
                        <rect
                            x={(point.x - radius).to_string()}
                            y={(point.y - radius).to_string()}
                            width={(radius * 2.0).to_string()}
                            height={(radius * 2.0).to_string()}
                            class={class} />
                    },
                }
            }
        }
    }

    /// Whether the surfaces are to be shaded by the sheets behind them.
    fn shading(ctx: &Context<Self>) -> bool {
        ctx.props().sheets && AppSettings::get_sheet_shading()
    }

    /// Marks where sheets end or meet (see `Depths::sheet_boundaries`), over the diagram: for a
    /// sheet that ends, the last stretch of it and a dashed ring where it ends; for sheets that
    /// meet, a double ring. Each kind is one path, with a pale halo under it so that it shows on
    /// any colour. Nothing for a diagram with no more dimensions than the view, or if it is off.
    fn view_sheet_outlines(ctx: &Context<Self>, prepared: &PreparedDiagram<N>) -> Html {
        use std::fmt::Write as _;

        use homotopy_core::projection::SheetBoundaryKind::{BeginsOrEnds, MergesOrSplits};

        if !ctx.props().sheets || !AppSettings::get_sheet_outlines() {
            return html! {};
        }
        let boundaries = prepared.depths.sheet_boundaries();
        if boundaries.is_empty() {
            return html! {};
        }
        let at = |coordinate: &[SliceIndex; N]| {
            prepared
                .transform
                .transform_point(Point::from(project_2d(prepared.layout[coordinate])))
        };
        let radius = ctx.props().style.point_radius;
        let ring = |path: &mut String, centre: Point, r: f32| {
            write!(
                path,
                "M{} {}a{r} {r} 0 1 0 {} 0a{r} {r} 0 1 0 {} 0",
                centre.x - r,
                centre.y,
                2.0 * r,
                -2.0 * r
            )
            .ok();
        };
        let (mut stretches, mut ends, mut meets) = (String::new(), String::new(), String::new());
        let (mut ends_count, mut meets_count) = (0, 0);
        let mut seen = homotopy_common::hash::FastHashSet::default();
        for b in boundaries {
            let to = at(&b.to);
            match b.kind {
                BeginsOrEnds => {
                    let from = at(&b.from);
                    ends_count += 1;
                    write!(stretches, "M{} {}L{} {}", from.x, from.y, to.x, to.y).ok();
                    ring(&mut ends, to, radius * 1.9);
                }
                MergesOrSplits => {
                    // (Several sheets can meet at the one place.)
                    if seen.insert(b.to) {
                        meets_count += 1;
                        ring(&mut meets, to, radius * 1.9);
                        ring(&mut meets, to, radius * 2.7);
                    }
                }
            }
        }
        let layer = |class: &'static str, d: &String| {
            if d.is_empty() {
                return html! {};
            }
            html! {
                <>
                    <path d={d.clone()} class="sheet-boundary__halo" />
                    <path d={d.clone()} class={classes!("sheet-boundary", class)} />
                </>
            }
        };
        html! {
            <g
                class="sheet-boundaries"
                pointer-events="none"
                data-ends={ends_count.to_string()}
                data-meets={meets_count.to_string()}
            >
                {layer("sheet-boundary--stretch", &stretches)}
                {layer("sheet-boundary--end", &ends)}
                {layer("sheet-boundary--meet", &meets)}
            </g>
        }
    }

    fn view_highlight(&self, ctx: &Context<Self>) -> Html {
        let Some(highlight) = &ctx.props().highlight else {
            return Default::default();
        };
        // The highlight is of points of the new diagram, which are not on the one that is shown.
        let Some(prepared) = self.current() else {
            return Default::default();
        };

        let (mut from, mut to) = (
            Point2D::splat(f32::INFINITY),
            Point2D::splat(f32::NEG_INFINITY),
        );

        for p in &highlight.points {
            match Self::position(prepared, *p) {
                None => {
                    tracing::error!("Encountered highlight point outside prepared diagram (point: {:?}). This is a bug!", p);
                    return Default::default();
                }
                Some(pos) => {
                    from = from.min(pos);
                    to = to.max(pos);
                }
            }
        }

        let padding = match highlight.kind {
            HighlightKind::Attach => {
                let padding = ctx.props().style.scale * 0.25;
                Vector2D::new(padding, padding)
            }
            HighlightKind::Slice => Vector2D::new(0.0, ctx.props().style.scale * 0.5),
        };

        let from = from - padding;
        let to = to + padding;

        let path = format!(
            "M {from_x} {from_y} L {from_x} {to_y} L {to_x} {to_y} L {to_x} {from_y} Z",
            from_x = if N == 1 { 0.0 } else { from.x },
            from_y = from.y,
            to_x = if N == 1 {
                ctx.props().style.scale
            } else {
                to.x
            },
            to_y = to.y
        );

        let class = match highlight.kind {
            HighlightKind::Attach => "diagram-svg__attach-highlight",
            HighlightKind::Slice => "diagram-svg__slice-highlight",
        };

        html! {
            <path d={path} class={class}/>
        }
    }

    /// The part of the diagram that a generator dragged over it would be attached at.
    fn view_drop_target(&self, prepared: &PreparedDiagram<N>) -> Html {
        let Some(shape) = self.drop_target.as_ref().and_then(|target| {
            prepared
                .actions
                .iter()
                .find(|(simplex, _)| simplex == target)
                .map(|(_, shape)| shape)
        }) else {
            return Default::default();
        };
        match shape {
            Shape::Fill(fill) => html! {
                <path d={path_to_svg(&fill.path)} class="diagram-svg__drop-highlight" />
            },
            Shape::Stroke(stroke) => html! {
                <path
                    d={path_to_svg(&stroke.path)}
                    class="diagram-svg__drop-highlight diagram-svg__drop-highlight--line"
                    stroke-width={stroke.width.to_string()}
                />
            },
            Shape::Circle(circle) => html! {
                <circle
                    cx={circle.center.x.to_string()}
                    cy={circle.center.y.to_string()}
                    r={circle.radius.to_string()}
                    class="diagram-svg__drop-highlight"
                />
            },
        }
    }

    fn position(prepared: &PreparedDiagram<N>, point: [SliceIndex; N]) -> Option<Point2D<f32>> {
        let point = project_2d(*prepared.layout.get(&point)?).into();
        Some(prepared.transform.transform_point(point))
    }

    /// Everything at the point of the screen, the part on top first: wires and vertices over
    /// regions (none while the diagram is waiting for its layout). Only the first when the diagram has no sheets to choose between, and nothing
    /// but the first is looked for then.
    fn hits_at(&self, ctx: &Context<Self>, point: Point2D<f32>) -> Vec<Simplex<N>> {
        let Some(prepared) = self.current() else {
            return vec![];
        };
        let point = Self::transform_screen_to_image(prepared, ctx).transform_point(point);
        let mut hits = prepared
            .actions
            .iter()
            .filter(|(_, shape)| shape.contains_point(point, 0.01))
            .map(|(simplex, _)| *simplex);
        if ctx.props().sheets {
            hits.collect()
        } else {
            hits.next().into_iter().collect()
        }
    }

    /// Which of the things at a place a click or a drop is for: the one that the sheet chosen is
    /// of, if it is there, and else the one on top.
    fn hit_to_act_on(&self, hits: &[Simplex<N>]) -> Option<Simplex<N>> {
        pick_hit(self.choice.as_ref(), hits)
    }

    /// The point of the diagram that a click or a drop at this point of the screen is at: the
    /// position of the part there, and if a sheet was chosen for it the sheet (and what is
    /// chosen inside it) as further coordinates.
    fn point_at(&self, ctx: &Context<Self>, point: Point2D<f32>) -> Option<Vec<SliceIndex>> {
        let hit = self.hit_to_act_on(&self.hits_at(ctx, point))?;
        Some(match &self.choice {
            Some(choice) if choice.position == hit[0] => choice.point(),
            _ => hit[0].to_vec(),
        })
    }

    fn pointer_move(&mut self, ctx: &Context<Self>, point: Point2D<f32>, shift_key: bool) {
        if let Some(start) = self.drag_start {
            let diff: Vector2D<f32> = point - start;
            let distance = ctx.props().style.scale * 0.5;

            if diff.square_length() < distance * distance {
                return;
            }

            let angle = diff.angle_from_x_axis();
            self.drag_start = None;

            // The sheet chosen when the button went down, if it is of the place the drag starts
            // from (as for a click); else the front one.
            let choice = self.drag_choice.take();
            let hits = self.hits_at(ctx, start);
            let (Some(simplex), Some(prepared)) =
                (pick_hit(choice.as_ref(), &hits), self.current())
            else {
                return;
            };
            let sheet = chosen_sheet(choice.as_ref(), simplex[0]);

            let homotopy = drag_to_homotopy(
                angle,
                &simplex,
                prepared.diagram.clone(),
                &prepared.depths,
                shift_key,
                sheet,
            );

            if let Some(homotopy) = homotopy {
                tracing::info!("Homotopy: {:?}", homotopy);
                ctx.props().on_homotopy.emit(homotopy);
            } else {
                tracing::info!("No homotopy");
            }
        }
    }

    fn pointer_stop(&mut self, ctx: &Context<Self>) {
        // If the mouse button is released without having travelled a distance great enough
        // to indicate a drag, it should be interpreted as a click.  This is preferrable to
        // a separate onclick handler since drags aren't interpreted as clicks anymore.
        if let Some(point) = self.drag_start {
            self.drag_start = None;
            if let Some(point) = self.point_at(ctx, point) {
                ctx.props().on_select.emit(point);
            }
        }
    }

    /// Shows the tooltip for what is under the pointer, or moves it with the pointer if that
    /// has not changed.
    fn update_tooltip(&mut self, ctx: &Context<Self>, point: Point2D<f32>) {
        let pointer = (f64::from(point.x), f64::from(point.y));
        self.pointer = pointer;
        self.hovering = true;
        let hits = self.hits_at(ctx, point);
        let simplex = hits.first().copied();
        // (There is none while the diagram is waiting for its layout.)
        if simplex.is_some() && simplex == self.hovered && hits == self.hits {
            tooltip::move_to(pointer);
            return;
        }
        // A sheet that was chosen stays while the place it was chosen for is under the pointer.
        if self
            .choice
            .as_ref()
            .is_some_and(|choice| !hits.iter().any(|hit| hit[0] == choice.position))
        {
            self.choice = None;
        }
        self.hovered = simplex;
        self.hits = hits;
        self.refresh(ctx);
    }

    /// Shows what the pointer is over again, with the sheets as they are now.
    fn refresh(&self, ctx: &Context<Self>) {
        let content = self
            .hovered
            .zip(self.current())
            .map(|(simplex, prepared)| self.hover_content(ctx, prepared, &simplex));
        self.keys
            .set_wanted(content.as_ref().is_some_and(|c| c.sheets.is_some()));
        match content {
            Some(content) => tooltip::show(content, self.pointer),
            None => tooltip::hide(),
        }
        self.apply_dimming(ctx);
        self.publish_summary(ctx);
    }

    fn hide_tooltip(&mut self) {
        if self.hovered.take().is_some() {
            tooltip::hide();
        }
        self.hits.clear();
        self.keys.set_wanted(false);
    }

    /// What the tooltip says about a part of the diagram: the label of the cell there (a
    /// formula, so that the names of generators that are formulas are shown as such), and where
    /// in the diagram it is, and the sheets behind it if there are several.
    fn hover_content(
        &self,
        ctx: &Context<Self>,
        prepared: &PreparedDiagram<N>,
        simplex: &Simplex<N>,
    ) -> tooltip::Content {
        let p = simplex[0];
        let d = match simplex {
            Simplex::Surface(_) => prepared.projection.generator(p),
            Simplex::Wire(_) | Simplex::Point(_) => prepared.projection.front_generator(p).0,
        };
        let props = ctx.props();
        // A vertex may be where strands cross, or a cell is cancelled or created, which is named
        // for that (it is a step of the diagram); wires and surfaces are named by their cell.
        let label = match simplex {
            Simplex::Point(_) => props.signature.describe_point(&prepared.diagram, &p, d),
            Simplex::Wire(_) | Simplex::Surface(_) => props.signature.describe_cell(d),
        };
        let mut detail = format!("@ {}", p.map(|si| si.to_string()).join(", "));
        if let Some(note) = Self::boundary_note(ctx, prepared, p) {
            detail.push_str(" · ");
            detail.push_str(note);
        }
        tooltip::Content {
            label,
            detail,
            sheets: self.sheet_stack(ctx, prepared),
        }
    }
}

impl<const N: usize> DiagramSvg<N> {
    /// What to say of the place, if a sheet ends or sheets meet there (when they are outlined).
    fn boundary_note(
        ctx: &Context<Self>,
        prepared: &PreparedDiagram<N>,
        position: [SliceIndex; N],
    ) -> Option<&'static str> {
        use homotopy_core::projection::SheetBoundaryKind::{BeginsOrEnds, MergesOrSplits};

        if !ctx.props().sheets || !AppSettings::get_sheet_outlines() {
            return None;
        }
        let boundaries = prepared.depths.sheet_boundaries_at(position);
        let meets = boundaries.iter().any(|b| b.kind == MergesOrSplits);
        let ends = boundaries.iter().any(|b| b.kind == BeginsOrEnds);
        match (ends, meets) {
            (_, true) if ends => Some("sheets begin, end, merge or split here"),
            (_, true) => Some("sheets merge or split here"),
            (true, false) => Some("a sheet begins or ends here"),
            (false, false) => None,
        }
    }
}

/// Which of the things at a place a click, a drop or a drag is for: the one that the sheet chosen
/// is of, if it is there, and else the one on top.
fn pick_hit<const N: usize>(choice: Option<&Choice<N>>, hits: &[Simplex<N>]) -> Option<Simplex<N>> {
    choice
        .and_then(|choice| hits.iter().find(|hit| hit[0] == choice.position))
        .or_else(|| hits.first())
        .copied()
}

/// The height `z` of the sheet that a choice made at `position` picks out for the next
/// coordinate (the first of its path), if it was made at that position.
fn chosen_sheet<const N: usize>(
    choice: Option<&Choice<N>>,
    position: [SliceIndex; N],
) -> Option<usize> {
    choice
        .filter(|choice| choice.position == position)
        .and_then(|choice| choice.path.first().copied())
}

/// The homotopy that dragging from a part of the diagram in a direction is. A horizontal drag in
/// a view of a 3-dimensional diagram acts on the sheet at `sheet` (its height `z` at the position)
/// if one was chosen, and else on the front one (`Depths::node_depth`).
fn drag_to_homotopy<const N: usize>(
    angle: Angle<f32>,
    simplex: &Simplex<N>,
    diagram: Diagram,
    depths: &Depths<N>,
    force_same: bool,
    sheet: Option<usize>,
) -> Option<Homotopy> {
    use Height::{Regular, Singular};
    use SliceIndex::{Boundary, Interior};

    let default_step = AppSettings::get_contraction_step() as usize;

    let abs_radians = angle.radians.abs();
    let horizontal = !(PI / 4.0..(3.0 * PI) / 4.0).contains(&abs_radians);

    let point = match simplex {
        Simplex::Surface([p0, _, _]) | Simplex::Wire([p0, _]) | Simplex::Point([p0]) => p0,
    };
    tracing::debug!("Point: {:?}", point);

    match N {
        1 => {
            let Interior(Singular(height)) = point[0] else {
                return None;
            };

            let direction = if angle.radians <= 0.0 {
                Direction::Forward
            } else {
                Direction::Backward
            };

            Some(Homotopy::Contract(Contract {
                step: default_step,
                bias: None,
                location: Default::default(),
                height,
                direction,
            }))
        }
        2 => {
            let diagram: DiagramN = diagram.try_into().ok()?;

            // Handle horizontal and vertical drags
            let (prefix, y, x, diagram) = if horizontal {
                let depth = match point[1] {
                    Interior(Singular(_)) => Height::Singular(
                        sheet.unwrap_or_else(|| depths.node_depth(*point).unwrap_or_default()),
                    ),
                    _ => return None,
                };

                let diagram: DiagramN = diagram.slice(point[0])?.try_into().ok()?;
                (Some(point[0]), point[1], depth.into(), diagram)
            } else {
                (None, point[0], point[1], diagram)
            };

            // TODO: Are there valid homotopies on boundary coordinates?
            let y = match y {
                Interior(y) => y,
                Boundary(_) => return None,
            };

            let x = match x {
                Interior(y) => y,
                Boundary(_) => return None,
            };

            // Decide if the drag is an expansion or a contraction
            let expansion = match y {
                Regular(_) => true,
                Singular(height) => {
                    if diagram.dimension() == 1 {
                        false
                    } else {
                        let cospan = &diagram.cospans()[height];
                        let forward: &RewriteN = (&cospan.forward).try_into().unwrap();
                        let backward: &RewriteN = (&cospan.backward).try_into().unwrap();

                        // TODO: This should probably be a method on Cospan.
                        let mut targets: Vec<_> = forward.targets().collect();
                        targets.extend(backward.targets());
                        targets.sort_unstable();
                        targets.dedup();
                        targets.len() > 1
                    }
                }
            };

            let direction = if horizontal {
                if (-0.5 * PI..0.5 * PI).contains(&angle.radians) {
                    Direction::Forward
                } else {
                    Direction::Backward
                }
            } else if angle.radians <= 0.0 {
                Direction::Forward
            } else {
                Direction::Backward
            };

            if expansion {
                Some(Homotopy::Expand(Expand {
                    location: prefix.into_iter().collect(),
                    point: [y, x],
                    direction,
                }))
            } else {
                let bias = if default_step != 1 {
                    None
                } else if force_same {
                    Some(Bias::Same)
                } else if horizontal || abs_radians >= PI / 2.0 {
                    Some(Bias::Lower)
                } else {
                    Some(Bias::Higher)
                };

                let height = match y {
                    Regular(_) => unreachable!(),
                    Singular(height) => height,
                };

                Some(Homotopy::Contract(Contract {
                    step: default_step,
                    bias,
                    location: prefix.into_iter().collect(),
                    height,
                    direction,
                }))
            }
        }
        _ => unreachable!(),
    }
}

#[derive(Copy, Clone, PartialEq, Debug)]
pub struct RenderStyle {
    pub scale: f32,
    pub wire_thickness: f32,
    pub point_radius: f32,
}

impl Default for RenderStyle {
    fn default() -> Self {
        Self {
            scale: 40.0,
            wire_thickness: 8.0,
            point_radius: 6.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use homotopy_core::{examples, projection::Depths, Diagram, DiagramN};

    use super::*;

    fn choice(z: &[usize]) -> Choice<2> {
        Choice {
            position: [Height::Singular(1).into(), Height::Singular(0).into()],
            path: z.to_vec(),
        }
    }

    #[test]
    fn the_sheet_is_the_first_height_of_a_choice_at_the_position() {
        let at = choice(&[2, 0]).position;
        assert_eq!(chosen_sheet(Some(&choice(&[2, 0])), at), Some(2));
        assert_eq!(chosen_sheet(Some(&choice(&[1])), at), Some(1));
        assert_eq!(chosen_sheet(None, at), None);
        // A choice at another position is none.
        let elsewhere = [Height::Singular(0).into(), Height::Singular(0).into()];
        assert_eq!(chosen_sheet(Some(&choice(&[2])), elsewhere), None);
    }

    #[test]
    fn a_choice_picks_its_own_shape_else_the_one_on_top() {
        let c = choice(&[0]);
        let other = [Height::Singular(0).into(), Height::Singular(0).into()];
        let (top, chosen) = (Simplex::Point([other]), Simplex::Point([c.position]));
        assert_eq!(pick_hit(Some(&c), &[top, chosen]), Some(chosen));
        assert_eq!(pick_hit(Some(&c), &[top]), Some(top));
        assert_eq!(pick_hit(None, &[top, chosen]), Some(top));
        assert_eq!(pick_hit::<2>(None, &[]), None);
    }

    /// The point that the expansion of a horizontal drag from a point of the diagram acts at.
    fn expansion_at(
        diagram: &DiagramN,
        depths: &Depths<2>,
        position: [SliceIndex; 2],
        sheet: Option<usize>,
    ) -> Option<[SliceIndex; 2]> {
        let drag = drag_to_homotopy(
            Angle::radians(0.0),
            &Simplex::Point([position]),
            Diagram::from(diagram.clone()),
            depths,
            false,
            sheet,
        )?;
        match drag {
            Homotopy::Expand(expand) => Some(expand.point.map(Into::into)),
            Homotopy::Contract(_) => None,
        }
    }

    #[test]
    fn a_horizontal_drag_expands_at_the_sheet_chosen_and_else_at_the_front() {
        let mut checked = 0;
        for (_, diagram) in [
            examples::associator(),
            examples::crossing(),
            examples::half_braid(),
            examples::bubble(),
            examples::algebraic_snake(),
            examples::lips(),
            examples::pants_unit(),
            examples::two_cups(),
        ] {
            if diagram.dimension() < 3 {
                continue;
            }
            let depths = Depths::<2>::new(&diagram.clone().into()).unwrap();
            for i in 0..=2 * diagram.size() {
                let Some(Ok(slice)) = diagram.slice(Height::from(i)).map(DiagramN::try_from) else {
                    continue;
                };
                for j in 0..=2 * slice.size() {
                    let position = [Height::from(i).into(), Height::from(j).into()];
                    let sheets = depths.sheets(position, &[]);
                    let front = depths.node_depth(position).unwrap_or_default();
                    let default = expansion_at(&diagram, &depths, position, None);
                    // Without a choice, it is as it was.
                    if let Some(point) = default {
                        assert_eq!(point[1], Height::Singular(front).into());
                    }
                    for sheet in sheets.iter().filter(|s| s.z != front) {
                        if let Some(point) =
                            expansion_at(&diagram, &depths, position, Some(sheet.z))
                        {
                            assert_eq!(point[1], Height::Singular(sheet.z).into());
                            checked += 1;
                        }
                    }
                }
            }
        }
        assert!(
            checked > 0,
            "some drag expands at a sheet that is not the front one"
        );
    }
}
