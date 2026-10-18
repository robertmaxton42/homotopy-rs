//! Where sheets meet, drawn in the 2D view over the surfaces and under the wires (so that a wire
//! in front hides them): a small stack of pages at each point where distinct sheets meet (one
//! page for each sheet, at most four, the sheet that is farthest back at the bottom), and echoes
//! of the wires that go into a sheet that has others in front of it at such a point.
//!
//! What they are made of is in [`homotopy_core::projection::Depths`] (`meeting_pages`,
//! `echo_sheet`) and the geometry in [`homotopy_graphics::svg::marks`]. They follow the sheet
//! that is chosen under the pointer (Shift+wheel, `[`, `]`, Tab): the pages of the sheets in front
//! of it vanish, with the echoes of those sheets, so that the chosen sheet's page is the top of the
//! stack, and the echoes of the sheets behind it still run under the pages that are left. Like the
//! dimming and the highlight, that is done to the elements that are already there (`apply_marks`:
//! paths and attributes set directly) and not by drawing the diagram again.

use std::fmt::Write as _;

use euclid::default::Vector2D;
use homotopy_common::hash::FastHashSet;
use homotopy_core::projection::SheetId;
use homotopy_graphics::{
    style::{GeneratorRepresentation, GeneratorStyle, SignatureStyleData},
    svg::{
        color_index,
        marks::{echo_side, leave_page, page, polyline, polyline_path, MAX_PAGES},
        shape::{Point, Shape},
    },
};
use wasm_bindgen::JsCast;
use web_sys::Element;
use yew::{prelude::*, virtual_dom::VTag};

use super::{DiagramSvg, PreparedDiagram};
use crate::app::settings::AppSettings;

/// How far from its page an echo runs before it has faded out, in pixels.
const FADE: f32 = 36.0;

/// How far from a wire the echo is, past the edge of the wire, in pixels.
const FIRST_GAP: f32 = 2.5;

/// A point where sheets meet, with the stack of pages drawn at it.
pub(super) struct Meeting {
    pub centre: Point,
    pub radius: f32,
    /// The sheets that meet, from the farthest back to the nearest: the pages from the bottom up.
    pub pages: Vec<SheetId>,
}

/// An echo of a wire: the wire, as a path, drawn again beside it and fading out from the page of
/// the sheet it is on.
pub(super) struct Echo {
    pub generator: homotopy_core::Diagram0,
    /// The wire in the coordinates of the image, as points.
    pub polyline: Vec<Point>,
    /// The meeting (index into [`SheetMarks::meetings`]) that the wire is at, and the sheet that
    /// it is on, which is one of its pages.
    pub meeting: usize,
    pub sheet: SheetId,
    /// The unit vector from the wire to where the echo is.
    pub side: Vector2D<f32>,
}

/// The marks of a diagram, worked out when first drawn.
#[derive(Default)]
pub(super) struct SheetMarks {
    pub meetings: Vec<Meeting>,
    pub echoes: Vec<Echo>,
}

/// What a fan looks like, for the sheet that is chosen (if one is): path data.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct FanLook {
    /// The rims of the pages, except the chosen one's.
    pub rims: String,
    /// The undersides of all the pages.
    pub undersides: String,
    /// The rim of the page of the chosen sheet, which is the top of the stack (empty if no sheet
    /// of this fan is chosen).
    pub chosen: String,
    /// The sheets of the pages that are drawn, from the bottom up, by name.
    pub sheets: String,
}

/// What an echo looks like: whether it is drawn, the stretch of the wire it is along (from where
/// it leaves the page of its sheet to where it has faded out), as path data, and the outlines of
/// the pages in front of it, which it runs under.
#[derive(Debug, PartialEq)]
pub(super) struct EchoLook {
    pub shown: bool,
    pub from: Point,
    pub to: Point,
    pub path: String,
    pub under: String,
}

impl SheetMarks {
    pub(super) fn new<const N: usize>(prepared: &PreparedDiagram<N>, point_radius: f32) -> Self {
        let depths = &prepared.depths;
        let mut marks = Self::default();

        // The places where sheets meet.
        let places: Vec<_> = depths
            .sheet_boundaries()
            .iter()
            .map(|boundary| boundary.to)
            .collect::<FastHashSet<_>>()
            .into_iter()
            .collect();
        let mut at = vec![];
        for &place in &places {
            let Some(centre) = DiagramSvg::position(prepared, place) else {
                continue;
            };
            at.push(place);
            marks.meetings.push(Meeting {
                centre,
                radius: point_radius,
                pages: depths
                    .meeting_pages(place)
                    .into_iter()
                    .map(|page| page.sheet)
                    .collect(),
            });
        }

        // The wires of sheets that have others in front of them at those places.
        for (simplex, shape) in &prepared.actions {
            let (homotopy_core::complex::Simplex::Wire(wire), Shape::Stroke(stroke)) =
                (simplex, shape)
            else {
                continue;
            };
            let Some(meeting) = at.iter().position(|place| wire.contains(place)) else {
                continue;
            };
            let Some(sheet) = depths.echo_sheet(*wire) else {
                continue;
            };
            let points = polyline(&stroke.path);
            let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
                continue;
            };
            let centre = marks.meetings[meeting].centre;
            let far = if (first - centre).square_length() > (last - centre).square_length() {
                first
            } else {
                last
            };
            let Some(side) = echo_side(far - centre) else {
                continue;
            };
            marks.echoes.push(Echo {
                generator: prepared.projection.front_generator(wire[0]).0,
                polyline: points,
                meeting,
                sheet,
                side,
            });
        }
        marks
    }

    /// The pages of a meeting that are drawn when `chosen` is the sheet that is chosen, from the
    /// bottom up: all of them, or, if the sheet is one of them, those up to and including its
    /// page (the sheets in front of it are put aside, so that its page is on top), and in any case
    /// the last few, as many as can be drawn.
    fn drawn(meeting: &Meeting, chosen: Option<SheetId>) -> Vec<SheetId> {
        let up_to = chosen
            .and_then(|sheet| meeting.pages.iter().position(|&page| page == sheet))
            .map_or(meeting.pages.len(), |i| i + 1);
        let from = up_to.saturating_sub(MAX_PAGES);
        meeting.pages[from..up_to].to_vec()
    }

    pub(super) fn fan_look(&self, meeting: usize, chosen: Option<SheetId>) -> FanLook {
        let meeting = &self.meetings[meeting];
        let drawn = Self::drawn(meeting, chosen);
        let mut look = FanLook::default();
        for (rank, &sheet) in drawn.iter().enumerate() {
            let marks = page(meeting.centre, rank, meeting.radius);
            look.undersides.push_str(&marks.underside);
            if chosen == Some(sheet) {
                look.chosen.push_str(&marks.rim);
            } else {
                look.rims.push_str(&marks.rim);
            }
            if !look.sheets.is_empty() {
                look.sheets.push(' ');
            }
            look.sheets.push_str(&sheet.label());
        }
        look
    }

    pub(super) fn echo_look(&self, echo: &Echo, chosen: Option<SheetId>) -> EchoLook {
        let meeting = &self.meetings[echo.meeting];
        let drawn = Self::drawn(meeting, chosen);
        let hidden = EchoLook {
            shown: false,
            from: Point::zero(),
            to: Point::zero(),
            path: String::new(),
            under: String::new(),
        };
        let Some(rank) = drawn.iter().position(|&sheet| sheet == echo.sheet) else {
            return hidden;
        };
        let Some(stretch) = leave_page(&echo.polyline, meeting.centre, rank, meeting.radius, FADE)
        else {
            return hidden;
        };
        let (Some(&from), Some(&to)) = (stretch.first(), stretch.last()) else {
            return hidden;
        };
        let under = (rank + 1..drawn.len())
            .map(|above| page(meeting.centre, above, meeting.radius).outline)
            .collect();
        EchoLook {
            shown: true,
            from,
            to,
            path: polyline_path(&stretch),
            under,
        }
    }
}

impl<const N: usize> DiagramSvg<N> {
    /// Marks where sheets meet (see the [module](self)): the stacks of pages, and the echoes
    /// under them, to be drawn over the surfaces and under the wires and vertices. Nothing for a
    /// diagram with no more dimensions than the view, or with one sheet, or if it is off.
    pub(super) fn view_sheet_marks(ctx: &Context<Self>, prepared: &PreparedDiagram<N>) -> Html {
        if !ctx.props().sheets || !AppSettings::get_sheet_outlines() {
            return html! {};
        }
        let marks = prepared
            .marks
            .get_or_init(|| SheetMarks::new(prepared, ctx.props().style.point_radius));
        if marks.meetings.is_empty() {
            return html! {};
        }
        let echoes = marks
            .echoes
            .iter()
            .enumerate()
            .map(|(i, echo)| Self::view_echo(ctx, prepared, marks, i, echo))
            .collect::<Html>();
        let fans = (0..marks.meetings.len())
            .map(|i| {
                let look = marks.fan_look(i, None);
                html! {
                    <g class="sheet-fan" data-fan={i.to_string()} data-sheets={look.sheets}>
                        <path d={look.undersides} class="sheet-mark sheet-mark--under" />
                        <path d={look.rims} class="sheet-mark sheet-mark--fan" />
                        <path d="" class="sheet-mark sheet-mark--chosen" />
                    </g>
                }
            })
            .collect::<Html>();
        html! {
            <g
                class="sheet-marks"
                pointer-events="none"
                data-fans={marks.meetings.len().to_string()}
                data-echoes={marks.echoes.len().to_string()}
            >
                {echoes}
                {fans}
            </g>
        }
    }

    /// The echo of a wire: its path again, in its colour, a little to one side, fading out along
    /// the wire from the page of its sheet, and under the pages of the sheets in front of it
    /// (which dim it, by a mask).
    fn view_echo(
        ctx: &Context<Self>,
        prepared: &PreparedDiagram<N>,
        marks: &SheetMarks,
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
        let mut transform = String::new();
        write!(
            transform,
            "translate({:.2} {:.2})",
            echo.side.x * edge,
            echo.side.y * edge
        )
        .ok();
        let look = marks.echo_look(echo, None);
        html! {
            <g
                class="sheet-echo-group"
                data-echo={index.to_string()}
                data-sheet={echo.sheet.label()}
                display={(!look.shown).then_some("none")}
            >
                <defs>
                    {Self::gradient(&id, "echo-gradient", &look, &colour, "1")}
                    {Self::gradient(&format!("{id}-halo"), "echo-gradient-halo", &look, "#ffffff", "0.6")}
                    <mask maskUnits="userSpaceOnUse" id={format!("{id}-mask")}>
                        <rect width="100%" height="100%" fill="white" />
                        <path d={look.under.clone()} class="sheet-echo-veil" />
                    </mask>
                </defs>
                <g mask={format!("url(#{id}-mask)")}>
                    <path
                        d={look.path.clone()}
                        transform={transform.clone()}
                        stroke={format!("url(#{id}-halo)")}
                        class="sheet-echo sheet-echo--halo"
                    />
                    <path
                        d={look.path.clone()}
                        transform={transform}
                        stroke={format!("url(#{id})")}
                        class="sheet-echo sheet-echo--main"
                    />
                </g>
            </g>
        }
    }

    /// The gradient that an echo is stroked with: its colour, opaque where it leaves the page of
    /// its sheet and clear where it has faded out. (Made by hand, as `html!` lowercases the name
    /// of the element, which is not the one that SVG has.)
    fn gradient(id: &str, class: &str, look: &EchoLook, colour: &str, opacity: &str) -> Html {
        let mut gradient = VTag::new("linearGradient");
        gradient.add_attribute("id", id.to_owned());
        gradient.add_attribute("class", class.to_owned());
        gradient.add_attribute("gradientUnits", "userSpaceOnUse");
        gradient.add_attribute("x1", look.from.x.to_string());
        gradient.add_attribute("y1", look.from.y.to_string());
        gradient.add_attribute("x2", look.to.x.to_string());
        gradient.add_attribute("y2", look.to.y.to_string());
        for (offset, opacity) in [("0", opacity), ("1", "0")] {
            let mut stop = VTag::new("stop");
            stop.add_attribute("offset", offset);
            stop.add_attribute("stop-color", colour.to_owned());
            stop.add_attribute("stop-opacity", opacity.to_owned());
            gradient.add_child(stop.into());
        }
        gradient.into()
    }

    /// Shows the fans and echoes for the sheet that is chosen under the pointer, if there is one:
    /// the pages of the sheets in front of it vanish, and with them the echoes of those sheets
    /// (see the [module](self)). With none chosen, or the pointer elsewhere, it is as drawn.
    pub(super) fn apply_marks(&self, ctx: &Context<Self>) {
        let Some(svg) = ctx.props().diagram_ref.cast::<Element>() else {
            return;
        };
        let Some(marks) = self.current().and_then(|prepared| prepared.marks.get()) else {
            return;
        };
        let chosen = self.chosen_sheet(ctx);

        let set = |element: &Element, name: &str, value: &str| {
            if element.get_attribute(name).as_deref() != Some(value) {
                element.set_attribute(name, value).ok();
            }
        };
        let find = |parent: &Element, selector: &str| {
            parent
                .query_selector(selector)
                .ok()
                .flatten()
                .and_then(|e| e.dyn_into::<Element>().ok())
        };

        for i in 0..marks.meetings.len() {
            let Some(fan) = find(&svg, &format!("[data-fan=\"{i}\"]")) else {
                continue;
            };
            let look = marks.fan_look(i, chosen);
            set(&fan, "data-sheets", &look.sheets);
            for (class, d) in [
                (".sheet-mark--under", &look.undersides),
                (".sheet-mark--fan", &look.rims),
                (".sheet-mark--chosen", &look.chosen),
            ] {
                if let Some(path) = find(&fan, class) {
                    set(&path, "d", d);
                }
            }
        }

        for (i, echo) in marks.echoes.iter().enumerate() {
            let Some(group) = find(&svg, &format!("[data-echo=\"{i}\"]")) else {
                continue;
            };
            let look = marks.echo_look(echo, chosen);
            if look.shown {
                group.remove_attribute("display").ok();
            } else {
                set(&group, "display", "none");
                continue;
            }
            for class in [".echo-gradient", ".echo-gradient-halo"] {
                if let Some(gradient) = find(&group, class) {
                    set(&gradient, "x1", &look.from.x.to_string());
                    set(&gradient, "y1", &look.from.y.to_string());
                    set(&gradient, "x2", &look.to.x.to_string());
                    set(&gradient, "y2", &look.to.y.to_string());
                }
            }
            if let Some(veil) = find(&group, ".sheet-echo-veil") {
                set(&veil, "d", &look.under);
            }
            for class in [".sheet-echo--halo", ".sheet-echo--main"] {
                if let Some(path) = find(&group, class) {
                    set(&path, "d", &look.path);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marks() -> SheetMarks {
        let sheets = |ids: &[u32]| ids.iter().map(|&i| SheetId(i)).collect::<Vec<_>>();
        SheetMarks {
            meetings: vec![
                Meeting {
                    centre: Point::new(50.0, 50.0),
                    radius: 7.0,
                    pages: sheets(&[4, 7, 2, 9]),
                },
                Meeting {
                    centre: Point::new(150.0, 50.0),
                    radius: 7.0,
                    pages: sheets(&[0, 1, 2, 3, 4, 5, 6]),
                },
            ],
            echoes: vec![Echo {
                generator: homotopy_core::Diagram0 {
                    generator: homotopy_core::Generator::new(0, 1),
                    orientation: homotopy_core::Orientation::Positive,
                },
                polyline: vec![Point::new(50.0, 50.0), Point::new(50.0, 150.0)],
                meeting: 0,
                sheet: SheetId(7),
                side: Vector2D::new(1.0, 0.0),
            }],
        }
    }

    #[test]
    fn with_no_choice_every_page_is_drawn() {
        let marks = marks();
        let look = marks.fan_look(0, None);
        assert_eq!(look.sheets, "S5 S8 S3 S10");
        assert_eq!(look.rims.matches('M').count(), 4);
        assert!(look.chosen.is_empty());
        // More sheets than pages: the nearest few, with the one on top the nearest.
        assert_eq!(marks.fan_look(1, None).sheets, "S4 S5 S6 S7");
    }

    #[test]
    fn the_pages_in_front_of_the_chosen_sheet_vanish() {
        let marks = marks();
        // S3 is the third from the back: the page of S10 in front of it is gone.
        let look = marks.fan_look(0, Some(SheetId(2)));
        assert_eq!(look.sheets, "S5 S8 S3");
        assert_eq!(look.rims.matches('M').count(), 2, "the others");
        assert_eq!(look.chosen.matches('M').count(), 1, "the chosen on top");
        assert_eq!(look.undersides.matches('M').count(), 3);
        // The page of the chosen sheet is where the third page is, when all are drawn.
        let all = marks.fan_look(0, None);
        let third = page(Point::new(50.0, 50.0), 2, 7.0).rim;
        assert!(all.rims.contains(&third));
        assert_eq!(look.chosen, third);
        // The sheet at the back: one page.
        assert_eq!(marks.fan_look(0, Some(SheetId(4))).sheets, "S5");
        // A sheet that is not a page here changes nothing.
        assert_eq!(
            marks.fan_look(0, Some(SheetId(100))),
            marks.fan_look(0, None)
        );
        // With many sheets the pages nearest the chosen one are drawn.
        assert_eq!(marks.fan_look(1, Some(SheetId(1))).sheets, "S1 S2");
        assert_eq!(marks.fan_look(1, Some(SheetId(5))).sheets, "S3 S4 S5 S6");
    }

    #[test]
    fn an_echo_begins_at_the_page_of_its_sheet_and_runs_under_the_pages_in_front() {
        let marks = marks();
        let echo = &marks.echoes[0];
        // S8 is the second page, with S3 and S10 in front of it.
        let look = marks.echo_look(echo, None);
        assert!(look.shown);
        assert_eq!(look.under.matches('Z').count(), 2);
        assert!(
            look.path.starts_with('M') && look.path.contains('L'),
            "{}",
            look.path
        );
        let (middle, _, ry) = ellipse_of(Point::new(50.0, 50.0), 1, 7.0);
        // It leaves the page below its centre (the wire goes down), on its outline.
        assert!((look.from.y - (middle.y + ry)).abs() < 0.1, "{look:?}");
        assert!((look.to.y - look.from.y - FADE).abs() < 0.1);
        // With S3 chosen only one page is in front of it fewer.
        assert_eq!(
            marks
                .echo_look(echo, Some(SheetId(2)))
                .under
                .matches('Z')
                .count(),
            1
        );
        // With S8 chosen it is the top page: nothing in front.
        assert_eq!(
            marks
                .echo_look(echo, Some(SheetId(7)))
                .under
                .matches('Z')
                .count(),
            0
        );
        // With the sheet at the back chosen the page of S8 is gone, and the echo with it.
        assert!(!marks.echo_look(echo, Some(SheetId(4))).shown);
    }

    /// The ellipse of a page, as the graphics crate draws it (its middle and half height), from its
    /// rim.
    fn ellipse_of(centre: Point, rank: usize, radius: f32) -> (Point, f32, f32) {
        let rim = page(centre, rank, radius).rim;
        let numbers: Vec<f32> = rim
            .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
            .filter_map(|n| n.parse().ok())
            .collect();
        let (left, cy, rx, ry) = (numbers[0], numbers[1], numbers[2], numbers[3]);
        (Point::new(left + rx, cy), rx, ry)
    }
}
