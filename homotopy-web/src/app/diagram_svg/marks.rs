//! Where sheets meet, drawn over the 2D view: a small fan of pages at each point where sheets
//! meet (one page for each sheet, at most four), a single page where a sheet begins or ends, and
//! echoes alongside the wires that go into the layers behind the front one at such a point.
//!
//! What they are made of is in [`homotopy_core::projection::Depths`] (`sheet_boundaries`,
//! `sheets_meeting_at`, `echoes`) and the geometry in [`homotopy_graphics::svg::marks`]. They are
//! a still picture: nothing here responds to the pointer, and the diagram is not drawn again to
//! show or hide them except when the setting changes.

use std::fmt::Write as _;

use euclid::default::Vector2D;
use homotopy_common::hash::FastHashMap;
use homotopy_core::{
    projection::SheetBoundaryKind::{BeginsOrEnds, MergesOrSplits},
    SliceIndex,
};
use homotopy_graphics::{
    style::{GeneratorRepresentation, GeneratorStyle, SignatureStyleData},
    svg::{
        color_index,
        marks::{echo_side, fan, reach_from, MAX_PAGES},
        shape::{path_to_svg, Point, Shape},
    },
};
use yew::{prelude::*, virtual_dom::VTag};

use super::{DiagramSvg, PreparedDiagram};
use crate::app::settings::AppSettings;

/// How far from the point an echo runs before it has faded out, in pixels.
const FADE: f32 = 36.0;

/// How far from a wire the first echo is, past the edge of the wire, and how far the next ones
/// are from it, in pixels.
const FIRST_GAP: f32 = 2.5;
const NEXT_GAP: f32 = 3.0;

/// An echo of a wire: the wire, as a path, drawn again beside it as many times as it is layers
/// behind, fading out from the point.
pub(super) struct Echo {
    pub generator: homotopy_core::Diagram0,
    /// The wire in the coordinates of the image.
    pub path: String,
    /// The point where sheets meet, and where the echo has faded out.
    pub from: Point,
    pub to: Point,
    /// The unit vector from the wire to where the echoes are.
    pub side: Vector2D<f32>,
    pub layers: usize,
}

/// The marks of a diagram, worked out when first drawn.
#[derive(Default)]
pub(super) struct SheetMarks {
    /// Path data of the fans where sheets meet.
    pub fans: String,
    /// Path data of the single pages where a sheet begins or ends.
    pub ends: String,
    pub fan_count: usize,
    pub end_count: usize,
    pub echoes: Vec<Echo>,
}

impl SheetMarks {
    pub(super) fn new<const N: usize>(prepared: &PreparedDiagram<N>, point_radius: f32) -> Self {
        let depths = &prepared.depths;
        let mut marks = Self::default();

        // The places that have boundaries, and whether sheets meet at them.
        let mut places: FastHashMap<[SliceIndex; N], bool> = FastHashMap::default();
        for boundary in depths.sheet_boundaries() {
            let meets = places.entry(boundary.to).or_insert(false);
            *meets |= boundary.kind == MergesOrSplits;
            debug_assert!(matches!(boundary.kind, BeginsOrEnds | MergesOrSplits));
        }
        let meetings: Vec<_> = places
            .iter()
            .filter(|&(_, &meets)| meets)
            .map(|(&place, _)| place)
            .collect();
        for (&place, &meets) in &places {
            let Some(centre) = DiagramSvg::position(prepared, place) else {
                continue;
            };
            if meets {
                marks.fan_count += 1;
                let pages = depths.sheets_meeting_at(place).min(MAX_PAGES);
                marks.fans.push_str(&fan(centre, pages, point_radius * 1.5));
            } else {
                marks.end_count += 1;
                marks.ends.push_str(&fan(centre, 1, point_radius * 1.5));
            }
        }

        // The wires at those places, on a layer behind.
        for (simplex, shape) in &prepared.actions {
            let (homotopy_core::complex::Simplex::Wire(wire), Shape::Stroke(stroke)) =
                (simplex, shape)
            else {
                continue;
            };
            if !meetings.iter().any(|place| wire.contains(place)) {
                continue;
            }
            let layers = depths.echoes(*wire);
            if layers == 0 {
                continue;
            }
            let vertex = if meetings.contains(&wire[1]) {
                wire[1]
            } else {
                wire[0]
            };
            let Some(centre) = DiagramSvg::position(prepared, vertex) else {
                continue;
            };
            let Some((from, to)) = reach_from(&stroke.path, centre, FADE) else {
                continue;
            };
            let Some(side) = echo_side(to - from) else {
                continue;
            };
            marks.echoes.push(Echo {
                generator: prepared.projection.front_generator(wire[0]).0,
                path: path_to_svg(&stroke.path),
                from,
                to,
                side,
                layers,
            });
        }
        marks
    }
}

impl<const N: usize> DiagramSvg<N> {
    /// Marks where sheets meet or end (see the [module](self)): over the diagram, a halo under
    /// each so that they show on any colour. Nothing for a diagram with no more dimensions than
    /// the view, or if it is off.
    pub(super) fn view_sheet_marks(ctx: &Context<Self>, prepared: &PreparedDiagram<N>) -> Html {
        if !ctx.props().sheets || !AppSettings::get_sheet_outlines() {
            return html! {};
        }
        let marks = prepared
            .marks
            .get_or_init(|| SheetMarks::new(prepared, ctx.props().style.point_radius));
        if marks.fan_count + marks.end_count + marks.echoes.len() == 0 {
            return html! {};
        }
        let pages = |class: &'static str, d: &String| {
            if d.is_empty() {
                return html! {};
            }
            html! {
                <path d={d.clone()} class={classes!("sheet-mark", class)} />
            }
        };
        let echoes = marks
            .echoes
            .iter()
            .enumerate()
            .map(|(i, echo)| Self::view_echo(ctx, prepared, i, echo))
            .collect::<Html>();
        html! {
            <g
                class="sheet-marks"
                pointer-events="none"
                data-fans={marks.fan_count.to_string()}
                data-ends={marks.end_count.to_string()}
                data-echoes={marks.echoes.len().to_string()}
            >
                {echoes}
                {pages("sheet-mark--fan", &marks.fans)}
                {pages("sheet-mark--end", &marks.ends)}
            </g>
        }
    }

    /// The echoes of a wire: its path again, in its colour, a little to one side for each layer
    /// it is behind, and fading out along the wire from the point.
    fn view_echo(
        ctx: &Context<Self>,
        prepared: &PreparedDiagram<N>,
        index: usize,
        echo: &Echo,
    ) -> Html {
        let props = ctx.props();
        let colour = props
            .signature
            .generator_style(echo.generator.generator)
            .map_or_else(
                || "#555555".to_owned(),
                |style| {
                    let c = color_index(
                        echo.generator.generator,
                        prepared.diagram.dimension(),
                        GeneratorRepresentation::Wire,
                    );
                    style.color().lighten(c, echo.generator.orientation).hex()
                },
            );
        let id = format!("{}-echo-{index}", props.id);
        let edge = props.style.wire_thickness / 2.0 + FIRST_GAP;
        let copies = (1..=echo.layers)
            .map(|layer| {
                let away = edge + NEXT_GAP * (layer - 1) as f32;
                let mut transform = String::new();
                write!(
                    transform,
                    "translate({:.2} {:.2})",
                    echo.side.x * away,
                    echo.side.y * away
                )
                .ok();
                html! {
                    <>
                        <path
                            d={echo.path.clone()}
                            transform={transform.clone()}
                            stroke={format!("url(#{id}-halo)")}
                            class="sheet-echo sheet-echo--halo"
                        />
                        <path
                            d={echo.path.clone()}
                            transform={transform}
                            stroke={format!("url(#{id})")}
                            class="sheet-echo"
                        />
                    </>
                }
            })
            .collect::<Html>();
        html! {
            <>
                {Self::gradient(&id, echo, &colour, "1")}
                {Self::gradient(&format!("{id}-halo"), echo, "#ffffff", "0.6")}
                {copies}
            </>
        }
    }

    /// The gradient that an echo is stroked with: its colour, opaque at the point where sheets
    /// meet and clear where it has faded out. (Made by hand, as `html!` lowercases the name of
    /// the element, which is not the one that SVG has.)
    fn gradient(id: &str, echo: &Echo, colour: &str, opacity: &str) -> Html {
        let mut gradient = VTag::new("linearGradient");
        gradient.add_attribute("id", id.to_owned());
        gradient.add_attribute("gradientUnits", "userSpaceOnUse");
        gradient.add_attribute("x1", echo.from.x.to_string());
        gradient.add_attribute("y1", echo.from.y.to_string());
        gradient.add_attribute("x2", echo.to.x.to_string());
        gradient.add_attribute("y2", echo.to.y.to_string());
        for (offset, opacity) in [("0", opacity), ("1", "0")] {
            let mut stop = VTag::new("stop");
            stop.add_attribute("offset", offset);
            stop.add_attribute("stop-color", colour.to_owned());
            stop.add_attribute("stop-opacity", opacity.to_owned());
            gradient.add_child(stop.into());
        }
        gradient.into()
    }
}
