//! Highlighting a sheet over the whole diagram: where the sheet is, in every position that it
//! passes through. The sheet is the current one of the stack of sheets under the pointer (the
//! one that the tooltip marks: the default, or the one chosen with Shift+wheel, `[` and `]` or
//! Tab), so it follows the stack as that changes.
//!
//! Like the dimming of wires this is done to elements that are already there (a few paths in two
//! groups that are drawn with no data, filled in here), and not by drawing the diagram again. Its
//! paths are made once for each sheet, from the positions that the analysis keeps for it, and
//! kept.
//!
//! The regions that the sheet is in are tinted, and the wires that are drawn for it get a halo
//! (under the wires, so that it shows as an outline round them). Where the sheet is behind
//! another at a wire there is nothing of it drawn, so a dashed line is put along the wire
//! instead, and the vertices that it passes through get a disc under them.

use std::{fmt::Write as _, rc::Rc};

use homotopy_common::hash::FastHashMap;
use homotopy_core::{complex::Simplex, projection::SheetId, Height, SliceIndex};
use homotopy_graphics::svg::shape::{path_to_svg, Shape};
use homotopy_model::sheets::Stack;
use wasm_bindgen::JsCast;
use web_sys::Element;
use yew::prelude::*;

use super::{DiagramSvg, PreparedDiagram};

/// How far the disc under a vertex of the sheet reaches past the vertex, and the halo under a wire
/// past the wire, in pixels.
const DISC_MARGIN: f32 = 3.5;
const HALO_MARGIN: f32 = 3.0;

/// The shapes of a sheet, as path data in the coordinates of the image.
#[derive(Default)]
pub(super) struct Extent {
    pub regions: String,
    pub halo: String,
    pub discs: String,
    pub behind: String,
}

/// Which of the shapes of the diagram (indices into `PreparedDiagram::actions`) are of each sheet,
/// with the height of the sheet there.
pub(super) struct SheetIndex {
    by_sheet: Vec<Vec<(usize, usize)>>,
}

impl SheetIndex {
    pub(super) fn new<const N: usize>(prepared: &PreparedDiagram<N>) -> Self {
        let mut by_sheet = vec![vec![]; prepared.depths.sheet_count()];
        for (i, (simplex, _)) in prepared.actions.iter().enumerate() {
            for (z, id) in prepared.depths.sheet_ids_at(simplex[0]).iter().enumerate() {
                by_sheet[id.0 as usize].push((i, z));
            }
        }
        Self { by_sheet }
    }
}

/// What is made for the sheets so far.
#[derive(Default)]
pub(super) struct Extents(FastHashMap<SheetId, Rc<Extent>>);

impl<const N: usize> PreparedDiagram<N> {
    /// The shapes of a sheet, made the first time they are asked for.
    pub(super) fn extent(&self, id: SheetId, point_radius: f32) -> Rc<Extent> {
        if let Some(extent) = self.extents.borrow().0.get(&id) {
            return Rc::clone(extent);
        }
        let index = self.sheet_index.get_or_init(|| SheetIndex::new(self));
        let mut extent = Extent::default();
        for &(i, z) in index
            .by_sheet
            .get(id.0 as usize)
            .map_or(&[][..], Vec::as_slice)
        {
            let (simplex, shape) = &self.actions[i];
            match (simplex, shape) {
                (Simplex::Surface(_), Shape::Fill(fill)) => {
                    extent.regions.push_str(&path_to_svg(&fill.path));
                }
                (Simplex::Wire(wire), Shape::Stroke(stroke)) => {
                    let drawn = self.depths.node_depth(wire[0]) == Some(z);
                    let d = if drawn {
                        &mut extent.halo
                    } else {
                        &mut extent.behind
                    };
                    d.push_str(&path_to_svg(&stroke.path));
                }
                (Simplex::Point([point]), Shape::Circle(circle))
                    if point
                        .iter()
                        .all(|si| matches!(si, SliceIndex::Interior(Height::Singular(_))))
                        && !self.projection.front_generator(*point).1 =>
                {
                    // (A vertex that is drawn: not the points of the regions and wires, and not
                    // the ones that are an identity.)
                    let r = point_radius + DISC_MARGIN;
                    let (x, y) = (circle.center.x, circle.center.y);
                    write!(
                        extent.discs,
                        "M{} {y}a{r} {r} 0 1 0 {} 0a{r} {r} 0 1 0 {} 0",
                        x - r,
                        2.0 * r,
                        -2.0 * r
                    )
                    .ok();
                }
                _ => {}
            }
        }
        let extent = Rc::new(extent);
        self.extents.borrow_mut().0.insert(id, Rc::clone(&extent));
        extent
    }
}

impl<const N: usize> DiagramSvg<N> {
    /// The groups that the highlight is drawn in: one under the wires and vertices (so that the
    /// halos show as outlines) and one over everything. Their paths have no data until a sheet is
    /// highlighted.
    pub(super) fn view_sheet_highlight_under(ctx: &Context<Self>) -> Html {
        if !ctx.props().sheets {
            return html! {};
        }
        let halo = ctx.props().style.wire_thickness + 2.0 * HALO_MARGIN;
        html! {
            <g class="sheet-highlight" pointer-events="none">
                <path class="sheet-highlight__regions" d="" />
                <path class="sheet-highlight__halo" d="" stroke-width={halo.to_string()} />
                <path class="sheet-highlight__discs" d="" />
            </g>
        }
    }

    pub(super) fn view_sheet_highlight_over(ctx: &Context<Self>) -> Html {
        if !ctx.props().sheets {
            return html! {};
        }
        html! {
            <g class="sheet-highlight" pointer-events="none">
                <path class="sheet-highlight__behind-halo" d="" />
                <path class="sheet-highlight__behind" d="" />
            </g>
        }
    }

    /// The sheet to highlight: the current one of the stack under the pointer, if the tooltip
    /// lists the stack (there are several sheets to choose between, or one was chosen). Inside a
    /// sheet that is fixed, it is that sheet that is highlighted.
    pub(super) fn highlighted_sheet(&self, ctx: &Context<Self>) -> Option<SheetId> {
        let (stack, sheet) = self.current_sheet(ctx)?;
        (stack.entries.len() >= 2 || stack.explicit).then_some(sheet)?
    }

    /// The sheet that has been chosen under the pointer (Shift+wheel, `[`, `]`, Tab), if one has:
    /// not the one that is only the default.
    pub(super) fn chosen_sheet(&self, ctx: &Context<Self>) -> Option<SheetId> {
        let (stack, sheet) = self.current_sheet(ctx)?;
        stack.explicit.then_some(sheet)?
    }

    /// The stack of sheets under the pointer and the sheet that is current in it (see
    /// [`highlighted_sheet`](Self::highlighted_sheet)), if the pointer is over the diagram.
    fn current_sheet(&self, ctx: &Context<Self>) -> Option<(Stack<N>, Option<SheetId>)> {
        if !ctx.props().sheets || !self.hovering {
            return None;
        }
        let prepared = self.current()?;
        let stack = Stack::new(&prepared.depths, &self.positions(), self.choice.as_ref())?;
        let entry = stack.entry();
        let sheet = match stack.fixed.first() {
            Some(&outer) => prepared
                .depths
                .sheet_ids_at(entry.position)
                .get(outer)
                .copied(),
            None => entry.sheet.id,
        };
        Some((stack, sheet))
    }

    /// Puts the highlight of the sheet that is current (if any) on the diagram.
    pub(super) fn apply_highlight(&self, ctx: &Context<Self>) {
        let Some(svg) = ctx.props().diagram_ref.cast::<Element>() else {
            return;
        };
        let extent = self
            .highlighted_sheet(ctx)
            .zip(self.current())
            .map(|(id, prepared)| prepared.extent(id, ctx.props().style.point_radius));
        let empty = Extent::default();
        let extent = extent.as_deref().unwrap_or(&empty);
        for (class, d) in [
            ("regions", &extent.regions),
            ("halo", &extent.halo),
            ("discs", &extent.discs),
            ("behind-halo", &extent.behind),
            ("behind", &extent.behind),
        ] {
            let Some(path) = svg
                .query_selector(&format!(".sheet-highlight__{class}"))
                .ok()
                .flatten()
                .and_then(|e| e.dyn_into::<Element>().ok())
            else {
                continue;
            };
            // (Not set again if it is as it was.)
            if path.get_attribute("d").as_deref() != Some(d.as_str()) {
                path.set_attribute("d", d).ok();
            }
        }
    }
}
