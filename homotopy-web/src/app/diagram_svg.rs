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
    projection::{Depths, Projection},
    rewrite::RewriteN,
    signature::Signature as _,
    Boundary, Diagram, DiagramN, Generator, Height, SliceIndex,
};
use homotopy_graphics::{
    style::{GeneratorStyle, SignatureStyleData, VertexShape},
    svg::{
        generator_class_from_diagram_dim,
        render::{ActionRegion, GraphicElement},
        shape::{path_to_svg, project_2d, Point, Shape},
        PLACEHOLDER_HATCH_ID,
    },
};
use web_sys::Element;
use yew::prelude::*;

use self::highlight::{HighlightKind, HighlightSvg};
use super::tooltip;
use crate::{
    app::settings::AppSettings,
    components::{read_touch_list_abs, Finger},
    layout::{self, Asked, Ticket},
    model::proof::{
        homotopy::{Contract, Expand, Homotopy},
        Signature,
    },
};

pub mod highlight;

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
    /// What the pointer is over, which the tooltip shown is for.
    hovered: Option<Simplex<N>>,
}

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
}

/// The computed properties of a diagram that are potentially expensive to compute but can be
/// cached if the diagram does not change.
struct PreparedDiagram<const N: usize> {
    /// The diagram that this is for.
    diagram: Diagram,
    graphic: Vec<GraphicElement<N>>,
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
    fn new(diagram: &Diagram, layout: Rc<Layout<N>>, style: RenderStyle) -> Self {
        assert!(diagram.dimension() >= N);

        let performance = web_sys::window().unwrap().performance().unwrap();
        performance.mark("startPrepareDiagram").unwrap();

        let complex = make_complex(diagram);
        let depths = Depths::new(diagram).unwrap();
        let projection = Projection::new(diagram, &layout, &depths).unwrap();
        let graphic = GraphicElement::build(&complex, &layout, &projection, &depths);
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
            hovered: None,
        };
        this.prepare(ctx);
        this
    }

    fn update(&mut self, ctx: &Context<Self>, msg: Self::Message) -> bool {
        match msg {
            DiagramSvgMessage::OnMouseDown(point) => {
                self.drag_start = Some(point);
                self.hide_tooltip();
            }
            DiagramSvgMessage::OnMouseMove(point, shift_key) => {
                self.pointer_move(ctx, point, shift_key);
                // Not while a button is down: that is a click or a drag, not hovering.
                if self.drag_start.is_none() {
                    self.update_tooltip(ctx, point);
                }
            }
            DiagramSvgMessage::OnMouseUp => {
                self.pointer_stop(ctx);
            }
            DiagramSvgMessage::OnMouseLeave => {
                self.hide_tooltip();
            }
            DiagramSvgMessage::OnTouchUpdate(touches) => {
                if self.drag_start.is_none() && touches.len() == 1 {
                    self.drag_start = Some(touches[0].1);
                } else if touches.is_empty() {
                    self.drag_start = None;
                }
            }
            DiagramSvgMessage::OnTouchMove(touches) => {
                if touches.len() == 1 {
                    self.pointer_move(ctx, touches[0].1, false);
                }
            }
            DiagramSvgMessage::Layout(ticket, layout) => {
                // The layout of a diagram that has been replaced since is of no use.
                if self.pending != Some(ticket) {
                    return false;
                }
                self.pending = None;
                self.announce = true;
                self.hide_tooltip();
                self.prepared = Some(PreparedDiagram::new(
                    &ctx.props().diagram,
                    layout,
                    ctx.props().style,
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
                ));
            }
        }
        true
    }

    fn rendered(&mut self, ctx: &Context<Self>, _first_render: bool) {
        if std::mem::take(&mut self.announce) {
            ctx.props().on_laid_out.emit(());
        }
    }

    fn destroy(&mut self, _ctx: &Context<Self>) {
        self.hide_tooltip();
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
                ref={ctx.props().diagram_ref.clone()}
            >
                {Self::view_placeholder_hatch(ctx, prepared)}
                {prepared.graphic.iter().enumerate().map(|(i, e)| Self::view_element(ctx, prepared, i, e)).collect::<Html>()}
                {self.view_highlight(ctx)}
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
                self.prepared = Some(PreparedDiagram::new(&props.diagram, layout, props.style));
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

    fn hatch_id(ctx: &Context<Self>) -> String {
        format!("{}-{PLACEHOLDER_HATCH_ID}", ctx.props().id)
    }

    /// Defines the pattern with which the surfaces of placeholders are hatched, if there are any.
    fn view_placeholder_hatch(ctx: &Context<Self>, prepared: &PreparedDiagram<N>) -> Html {
        let has_placeholder_surface = prepared.graphic.iter().any(|element| {
            matches!(element, GraphicElement::Surface(d, _) if Self::is_placeholder(ctx, d.generator))
        });
        if !has_placeholder_surface {
            return html! {};
        }

        html! {
            <defs>
                <pattern
                    id={Self::hatch_id(ctx)}
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
                        stroke="black"
                        stroke-width="2"
                        stroke-opacity="0.55"
                    />
                </pattern>
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
                if Self::is_placeholder(ctx, d.generator) {
                    // Placeholders are translucent (by their stylesheet) and also hatched.
                    html! {
                        <>
                            <path d={path.clone()} class={class} />
                            <path
                                d={path}
                                fill={format!("url(#{})", Self::hatch_id(ctx))}
                                stroke="none"
                            />
                        </>
                    }
                } else {
                    html! {
                        <path d={path} class={class} />
                    }
                }
            }
            GraphicElement::Wire(_, _, path, mask) => {
                let path = path_to_svg(&path.clone().transformed(&prepared.transform));

                if mask.is_empty() {
                    html! {
                        <path
                            d={path}
                            class={class}
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

    fn position(prepared: &PreparedDiagram<N>, point: [SliceIndex; N]) -> Option<Point2D<f32>> {
        let point = project_2d(*prepared.layout.get(&point)?).into();
        Some(prepared.transform.transform_point(point))
    }

    /// The part of the diagram at the point of the screen, if there is one and the diagram is
    /// ready to be acted on (not while it is waiting for its layout).
    fn simplex_at(&self, ctx: &Context<Self>, point: Point2D<f32>) -> Option<Simplex<N>> {
        let prepared = self.current()?;
        let point = Self::transform_screen_to_image(prepared, ctx).transform_point(point);
        prepared
            .actions
            .iter()
            .find(|(_, shape)| shape.contains_point(point, 0.01))
            .map(|(simplex, _)| *simplex)
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

            let (Some(simplex), Some(prepared)) = (self.simplex_at(ctx, start), self.current())
            else {
                return;
            };

            let homotopy = drag_to_homotopy(
                angle,
                &simplex,
                prepared.diagram.clone(),
                &prepared.depths,
                shift_key,
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
            if let Some(simplex) = self.simplex_at(ctx, point) {
                ctx.props().on_select.emit(simplex[0].to_vec());
            }
        }
    }

    /// Shows the tooltip for what is under the pointer, or moves it with the pointer if that
    /// has not changed.
    fn update_tooltip(&mut self, ctx: &Context<Self>, point: Point2D<f32>) {
        let pointer = (f64::from(point.x), f64::from(point.y));
        let simplex = self.simplex_at(ctx, point);
        // (There is none while the diagram is waiting for its layout.)
        if simplex.is_some() && simplex == self.hovered {
            tooltip::move_to(pointer);
            return;
        }
        self.hovered = simplex;
        let content = simplex
            .zip(self.current())
            .map(|(simplex, prepared)| Self::hover_content(ctx, prepared, &simplex));
        match content {
            Some(content) => tooltip::show(content, pointer),
            None => tooltip::hide(),
        }
    }

    fn hide_tooltip(&mut self) {
        if self.hovered.take().is_some() {
            tooltip::hide();
        }
    }

    /// What the tooltip says about a part of the diagram: the label of the cell there (a
    /// formula, so that the names of generators that are formulas are shown as such), and where
    /// in the diagram it is.
    fn hover_content(
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
        tooltip::Content {
            label,
            detail: format!("@ {}", p.map(|si| si.to_string()).join(", ")),
        }
    }
}

fn drag_to_homotopy<const N: usize>(
    angle: Angle<f32>,
    simplex: &Simplex<N>,
    diagram: Diagram,
    depths: &Depths<N>,
    force_same: bool,
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
                    Interior(Singular(_)) => {
                        Height::Singular(depths.node_depth(*point).unwrap_or_default())
                    }
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
