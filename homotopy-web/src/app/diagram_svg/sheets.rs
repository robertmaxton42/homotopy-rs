//! Choosing a sheet over the diagram: the cells hidden behind a position of the view.
//!
//! The logic of what the choices are is in [`homotopy_model::sheets`]. This is the part that is
//! on the page: the keys, listed in the tooltip (the stack of sheets under the pointer), the
//! wires that are not on the sheet chosen drawn dim (by an attribute set on the elements, so
//! that choosing does not make the whole diagram render again), and the hooks for tests.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gloo::events::{EventListener, EventListenerOptions};
use homotopy_core::{complex::Simplex, projection::SheetId, SliceIndex};
use homotopy_model::sheets::{Choice, Stack};
use serde::Serialize;
use wasm_bindgen::JsCast;
use web_sys::Element;
use yew::prelude::*;

use super::{DiagramSvg, DiagramSvgMessage, PreparedDiagram};
use crate::app::tooltip::{SheetRow, SheetStack};

/// What can be done about the sheets under the pointer.
#[derive(Clone, Debug, PartialEq)]
pub enum SheetCommand<const N: usize> {
    /// Move to the next sheet (the previous if negative), round the stack.
    Step(isize),
    /// Fix the sheet, and go on to choose the one inside it, for the next coordinate.
    Drill,
    /// Let go of the innermost sheet that was fixed.
    Undrill,
    /// Choose no sheet.
    Clear,
    /// For tests: the pointer is at this place in the window.
    Hover(f32, f32),
    /// For tests: choose these sheets (outermost first) at the position under the pointer.
    Set(Vec<usize>),
}

/// How many sheets the tooltip lists at most, round the one that is current.
const ROWS: usize = 9;

/// The keys that choose a sheet, which are listened for in the whole window while the pointer is
/// over something that has sheets, and then only.
#[derive(Default)]
pub struct Keys {
    /// Whether the keys mean anything now (shared with the handlers that cannot ask the diagram).
    wanted: Rc<Cell<bool>>,
    listener: Option<EventListener>,
}

impl Keys {
    pub fn wanted(&self) -> Rc<Cell<bool>> {
        Rc::clone(&self.wanted)
    }

    pub fn set_wanted(&self, wanted: bool) {
        self.wanted.set(wanted);
    }

    pub fn remove(&mut self) {
        self.listener = None;
        self.wanted.set(false);
        DEBUG.with(|d| d.borrow_mut().command = None);
    }
}

/// Whether the event is for a field that is typed into, which keys are not taken from.
fn in_text_field(event: &web_sys::KeyboardEvent) -> bool {
    event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok())
        .and_then(|element| {
            element
                .closest("input, textarea, select, [contenteditable]")
                .ok()
                .flatten()
        })
        .is_some()
}

/// The subscript digits for the number of a coordinate: `z₃`.
fn subscript(n: usize) -> String {
    n.to_string()
        .chars()
        .map(|c| char::from_u32('₀' as u32 + c.to_digit(10).unwrap_or(0)).unwrap_or(c))
        .collect()
}

thread_local! {
    static DEBUG: RefCell<Debug> = RefCell::default();
}

/// For tests (`window.__homotopy.sheet(..)` and `.sheets()`, see `debug_hooks`).
#[derive(Default)]
struct Debug {
    command: Option<Box<dyn Fn(String)>>,
    summary: String,
}

/// Does what a debug hook says (as JSON: `{"step": 1}`, `"drill"`, `"undrill"`, `"clear"`,
/// `{"hover": [x, y]}`, `{"set": [z, ..]}`) in the diagram that has sheets. It is done a moment
/// later, as every message to a component is.
pub fn debug_command(json: &str) {
    DEBUG.with(|d| {
        if let Some(command) = &d.borrow().command {
            command(json.to_owned());
        }
    });
}

/// A JSON summary of the sheets under the pointer, as of the last time that they changed.
pub fn debug_summary() -> String {
    DEBUG.with(|d| d.borrow().summary.clone())
}

impl<const N: usize> DiagramSvg<N> {
    /// Listens for the keys, if the diagram has sheets to choose between.
    pub(super) fn install_sheets(&mut self, ctx: &Context<Self>) {
        if !ctx.props().sheets {
            return;
        }
        let link = ctx.link().clone();
        let wanted = self.keys.wanted();
        self.keys.listener = Some(EventListener::new_with_options(
            &web_sys::window().unwrap(),
            "keydown",
            EventListenerOptions::enable_prevent_default(),
            move |event| {
                let Some(event) = event.dyn_ref::<web_sys::KeyboardEvent>() else {
                    return;
                };
                if !wanted.get()
                    || event.ctrl_key()
                    || event.alt_key()
                    || event.meta_key()
                    || event.is_composing()
                    || in_text_field(event)
                {
                    return;
                }
                let command = match (event.key().as_str(), event.shift_key()) {
                    ("]", _) => SheetCommand::Step(1),
                    ("[", _) => SheetCommand::Step(-1),
                    ("Tab", false) => SheetCommand::Drill,
                    ("Tab", true) => SheetCommand::Undrill,
                    _ => return,
                };
                event.prevent_default();
                link.send_message(DiagramSvgMessage::Sheet(command));
            },
        ));

        let link = ctx.link().clone();
        DEBUG.with(|d| {
            d.borrow_mut().command = Some(Box::new(move |json| {
                let value: serde_json::Value = serde_json::from_str(&json).unwrap_or_default();
                let number = |v: &serde_json::Value| v.as_f64().unwrap_or_default();
                let command = match &value {
                    serde_json::Value::String(name) if name == "drill" => SheetCommand::Drill,
                    serde_json::Value::String(name) if name == "undrill" => SheetCommand::Undrill,
                    serde_json::Value::String(name) if name == "clear" => SheetCommand::Clear,
                    serde_json::Value::Object(map) => {
                        if let Some(step) = map.get("step") {
                            SheetCommand::Step(number(step) as isize)
                        } else if let Some(serde_json::Value::Array(at)) = map.get("hover") {
                            SheetCommand::Hover(
                                at.first().map_or(0.0, number) as f32,
                                at.get(1).map_or(0.0, number) as f32,
                            )
                        } else if let Some(serde_json::Value::Array(path)) = map.get("set") {
                            SheetCommand::Set(path.iter().map(|z| number(z) as usize).collect())
                        } else {
                            return;
                        }
                    }
                    _ => return,
                };
                link.send_message(DiagramSvgMessage::Sheet(command));
            }));
        });
    }

    /// Does something about the sheets under the pointer.
    pub(super) fn sheet_command(&mut self, ctx: &Context<Self>, command: SheetCommand<N>) {
        if let SheetCommand::Hover(x, y) = command {
            self.update_tooltip(ctx, euclid::default::Point2D::new(x, y));
            return;
        }
        let Some(prepared) = self.current() else {
            return;
        };
        let positions = self.positions();
        let stack = Stack::new(&prepared.depths, &positions, self.choice.as_ref());
        let choice = match (command, &stack) {
            (SheetCommand::Step(steps), Some(stack)) => Some(stack.step(steps)),
            (SheetCommand::Drill, Some(stack)) => stack
                .drill(&prepared.depths)
                // Nothing further in: it stays as it is.
                .or_else(|| self.choice.clone()),
            (SheetCommand::Undrill, _) => self.choice.as_ref().and_then(Choice::undrilled),
            (SheetCommand::Set(path), Some(stack)) => {
                let position = stack.entry().position;
                (!path.is_empty()).then_some(Choice { position, path })
            }
            _ => None,
        };
        if choice == self.choice {
            // (Say so, if there was nothing to do: the hint says why.)
            self.refresh(ctx);
            return;
        }
        self.choice = choice;
        self.refresh(ctx);
    }

    /// The positions of what is under the pointer that its sheets are listed for: of the kind
    /// that is on top (wires that overlap are each a choice, but the regions under a wire are
    /// not), the one on top first. A choice that was made keeps its place in the list even if
    /// it is of another kind.
    pub(super) fn positions(&self) -> Vec<[SliceIndex; N]> {
        let Some(top) = self.hits.first() else {
            return vec![];
        };
        let kind = std::mem::discriminant(top);
        self.hits
            .iter()
            .filter(|hit| {
                std::mem::discriminant(*hit) == kind
                    || self.choice.as_ref().is_some_and(|c| c.position == hit[0])
            })
            .map(|hit| hit[0])
            .collect()
    }

    /// The sheets under the pointer, to list in the tooltip, if there are any worth listing:
    /// several to choose between, or one that has been chosen, or one to go into.
    pub(super) fn sheet_stack(
        &self,
        ctx: &Context<Self>,
        prepared: &PreparedDiagram<N>,
    ) -> Option<SheetStack> {
        if !ctx.props().sheets {
            return None;
        }
        let positions = self.positions();
        let stack = Stack::new(&prepared.depths, &positions, self.choice.as_ref())?;
        let entry = stack.entry();
        let inner = prepared.depths.sheets(entry.position, &stack.path());
        if stack.entries.len() < 2 && !stack.explicit && inner.is_empty() {
            return None;
        }

        let first = N + 1 + stack.fixed.len();
        let coordinate = format!("z{}", subscript(first));
        let breadcrumb = stack.explicit.then(|| {
            stack
                .path()
                .iter()
                .enumerate()
                .map(|(i, z)| format!("z{}={z}", subscript(N + 1 + i)))
                .collect::<Vec<_>>()
                .join(" › ")
        });
        let several_places = stack
            .entries
            .iter()
            .any(|e| e.position != stack.entries[0].position);

        // A window of the rows, round the current one.
        let len = stack.entries.len();
        let start = stack
            .current
            .saturating_sub(ROWS / 2)
            .min(len.saturating_sub(ROWS));
        let end = (start + ROWS).min(len);
        let rows = (start..end)
            .map(|i| {
                let e = &stack.entries[i];
                SheetRow {
                    z: e.sheet.z,
                    label: ctx.props().signature.describe_cell(e.sheet.generator),
                    current: i == stack.current,
                    front: e.sheet.foremost,
                    drawn: e.sheet.front,
                    name: e.sheet.id.map(SheetId::label),
                    kind: several_places
                        .then(|| {
                            self.hits
                                .iter()
                                .find(|hit| hit[0] == e.position)
                                .map(|hit| match hit {
                                    Simplex::Point(_) => "vertex",
                                    Simplex::Wire(_) => "wire",
                                    Simplex::Surface(_) => "region",
                                })
                        })
                        .flatten(),
                }
            })
            .collect();

        let hint = if stack.explicit {
            if inner.is_empty() {
                "[ ] or Shift+wheel: other sheet · Shift+Tab: back · nothing further in".to_owned()
            } else {
                "[ ] or Shift+wheel: other sheet · Tab: fix and go in · Shift+Tab: back".to_owned()
            }
        } else if inner.is_empty() {
            "[ ] or Shift+wheel: choose a sheet · Tab: fix this one".to_owned()
        } else {
            "[ ] or Shift+wheel: choose a sheet · Tab: fix this one and go in".to_owned()
        };

        Some(SheetStack {
            breadcrumb,
            coordinate,
            rows,
            hidden: len - (end - start),
            hint,
        })
    }

    /// Dims the wires that are not on the sheet that is chosen. Done to the elements, and not by
    /// drawing the diagram again, which is much of the work of a large one.
    pub(super) fn apply_dimming(&self, ctx: &Context<Self>) {
        let Some(svg) = ctx.props().diagram_ref.cast::<Element>() else {
            return;
        };
        let chosen = self
            .choice
            .as_ref()
            .filter(|_| self.hovering)
            .and_then(|choice| choice.path.first().copied());
        let Ok(wires) = svg.query_selector_all("[data-depth]") else {
            return;
        };
        for i in 0..wires.length() {
            let Some(wire) = wires.item(i).and_then(|n| n.dyn_into::<Element>().ok()) else {
                continue;
            };
            // The depth is one more than the sheet, and 0 for a wire with none.
            let depth = wire
                .get_attribute("data-depth")
                .and_then(|depth| depth.parse::<usize>().ok())
                .unwrap_or(0);
            let dim = chosen.is_some_and(|z| depth != 0 && depth - 1 != z);
            if dim {
                wire.set_attribute("data-dim", "1").ok();
            } else if wire.has_attribute("data-dim") {
                wire.remove_attribute("data-dim").ok();
            }
        }
    }

    /// The pointer has left the diagram (or what is dragged over it): what it was over is
    /// forgotten, and the choice is kept for if it comes back to the same place.
    pub(super) fn leave(&mut self, ctx: &Context<Self>) {
        self.hovering = false;
        self.hovered = None;
        self.hits.clear();
        self.keys.set_wanted(false);
        self.apply_dimming(ctx);
        self.apply_highlight(ctx);
        self.apply_marks(ctx);
        self.publish_summary(ctx);
    }

    /// Records what the sheets are, for the debug hook that says (see [`debug_summary`]).
    pub(super) fn publish_summary(&self, ctx: &Context<Self>) {
        #[derive(Serialize)]
        struct Row {
            position: String,
            z: usize,
            front: bool,
            foremost: bool,
            sheet: Option<u32>,
            label: String,
        }
        #[derive(Serialize)]
        struct Summary {
            hovering: bool,
            hits: Vec<String>,
            entries: Vec<Row>,
            current: Option<usize>,
            explicit: bool,
            fixed: Vec<usize>,
            choice: Option<Vec<String>>,
            wanted: bool,
            dimmed: usize,
        }
        if !ctx.props().sheets {
            return;
        }
        let place = |p: &[SliceIndex; N]| p.map(|si| si.to_string()).join(",");
        let stack = self.current().and_then(|prepared| {
            Stack::new(&prepared.depths, &self.positions(), self.choice.as_ref())
        });
        let dimmed = ctx
            .props()
            .diagram_ref
            .cast::<Element>()
            .and_then(|svg| svg.query_selector_all("[data-dim]").ok())
            .map_or(0, |list| list.length() as usize);
        let summary = Summary {
            hovering: self.hovering,
            hits: self.hits.iter().map(|hit| place(&hit[0])).collect(),
            entries: stack
                .iter()
                .flat_map(|stack| stack.entries.iter())
                .map(|e| Row {
                    position: place(&e.position),
                    z: e.sheet.z,
                    front: e.sheet.front,
                    foremost: e.sheet.foremost,
                    sheet: e.sheet.id.map(|id| id.0),
                    label: ctx.props().signature.describe_cell(e.sheet.generator),
                })
                .collect(),
            current: stack.as_ref().map(|s| s.current),
            explicit: stack.as_ref().is_some_and(|s| s.explicit),
            fixed: stack.as_ref().map(|s| s.fixed.clone()).unwrap_or_default(),
            choice: self
                .choice
                .as_ref()
                .map(|c| c.point().iter().map(ToString::to_string).collect()),
            wanted: self.keys.wanted.get(),
            dimmed,
        };
        if let Ok(json) = serde_json::to_string(&summary) {
            DEBUG.with(|d| d.borrow_mut().summary = json);
        }
    }
}
