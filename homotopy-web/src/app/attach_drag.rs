//! Dragging a generator from the signature onto the diagram with the pointer.
//!
//! The browser's own drag and drop passes on neither the wheel nor the keys while it lasts, so
//! with it a sheet cannot be chosen during a drag (see `homotopy_model::sheets`). This is a drag
//! made of pointer events instead, in which the page gets everything: the diagram under the
//! pointer is told where the pointer is, as it is by a native drag (`DiagramSvgMessage::OnDragOver`
//! and its siblings), and it is also hovered over as usual, so that its tooltip lists the sheets
//! and Shift+wheel, `[`, `]` and `Tab` choose between them. Nothing here captures the pointer:
//! the listeners are on the window, and the wheel goes to whatever is under the pointer anyway.
//!
//! **Which drags are these.** The rows of the signature are also `draggable`, to move them
//! around in the signature, and that is still the browser's drag and drop. A native drag starts
//! (`dragstart`) at the first few pixels of movement, before it is known where the pointer is
//! going, and cannot be stopped later, so the two have to be told apart by what those pixels
//! are:
//!
//! - A press on the **pointing hand** (the button that picks a generator) is always a drag to the
//!   diagram. It is a handle: the choice is the user's, and it is the way for a touch screen.
//! - A press on the rest of the **row** is a drag to the diagram if it moves towards it, within
//!   45 degrees of the horizontal (the diagram is beside the signature, so moving a row to another
//!   place in the signature is mostly up or down). Anything else is left to the browser, so
//!   moving rows and folders is as it was, and such a drag can still be dropped on the diagram,
//!   by the native path (`attach_target`), only without choosing a sheet meanwhile.
//!
//! Either is started once the pointer has moved [`THRESHOLD`] pixels, and the native drag that
//! the browser would also start is cancelled ([`dragstart`]). A small ghost of the row follows the
//! pointer, and `attach_target` knows what is being dragged, as for a native drag, until it ends.
//!
//! **Ending.** Releasing the pointer over the diagram drops (the diagram works out the point and
//! `AttachGenerator` is dispatched, which is refused while a job runs); releasing anywhere else,
//! `pointercancel`, `Esc` (this one, and not a job that runs; the app looks at [`active`]), the
//! window losing focus and the buttons being up already all cancel. A click that follows is
//! swallowed, so that a drag that ends on a row does not select it.

use std::cell::{Cell, RefCell};

use gloo::events::{EventListener, EventListenerOptions, EventListenerPhase};
use gloo_timers::callback::Timeout;
use homotopy_core::{Diagram, Generator};
use homotopy_model::proof::{CannotAttach, Signature};
use wasm_bindgen::JsCast;
use web_sys::{DragEvent, Element, HtmlElement, KeyboardEvent, PointerEvent};
use yew::prelude::*;

use super::attach_target;
use crate::components::toast::{toast, Toast};

/// How far the pointer moves, in pixels, before a press is a drag.
pub const THRESHOLD: f64 = 4.0;

/// How long, in milliseconds, a click after a drag is swallowed for.
const CLICK_GUARD_MS: u32 = 100;

/// How wide the ghost of a row is at most, in pixels.
const GHOST_WIDTH: f64 = 180.0;

/// What a press was on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grip {
    /// The body of the row: it is a drag to the diagram if it moves towards it.
    Row,
    /// The pointing hand: it is a drag to the diagram wherever it moves.
    Handle,
}

/// A press of the primary button on a row of the signature, which may become a drag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Press {
    /// Where the pointer went down, in the window.
    pub origin: (f64, f64),
    pub grip: Grip,
    /// Which way the diagram is from the row, along the horizontal: 1 if to the right, -1 if to
    /// the left, 0 if beside it neither way (then the body of the row is only for the browser).
    pub toward: f64,
}

impl Press {
    /// Whether the pointer being at this place means a drag to the diagram (as opposed to one
    /// that moves the row in the signature), for a pointer that has moved at all.
    #[must_use]
    pub fn is_attach(&self, at: (f64, f64)) -> bool {
        match self.grip {
            Grip::Handle => true,
            Grip::Row => {
                let along = (at.0 - self.origin.0) * self.toward;
                let across = (at.1 - self.origin.1).abs();
                along > 0.0 && along >= across
            }
        }
    }

    /// Whether the pointer has moved far enough for a press to become a drag.
    #[must_use]
    pub fn far_enough(&self, at: (f64, f64)) -> bool {
        let (dx, dy) = (at.0 - self.origin.0, at.1 - self.origin.1);
        dx * dx + dy * dy >= THRESHOLD * THRESHOLD
    }

    /// Whether the press is now a drag to the diagram.
    #[must_use]
    pub fn begins(&self, at: (f64, f64)) -> bool {
        self.far_enough(at) && self.is_attach(at)
    }
}

/// Which way the diagram is from a row, along the horizontal (see [`Press::toward`]), from the
/// middle of each: beside it by less than this many pixels is neither way.
#[must_use]
pub fn direction_to(row_middle: f64, diagram_middle: f64) -> f64 {
    const BESIDE: f64 = 24.0;
    let gap = diagram_middle - row_middle;
    if gap.abs() < BESIDE {
        0.0
    } else {
        gap.signum()
    }
}

/// What the diagram under a dragged generator is told, which it takes as it does a native drag.
#[derive(Clone, Debug, PartialEq)]
pub enum DragSignal {
    /// The pointer is over the diagram, at this place in the window.
    Over(f32, f32),
    /// The pointer is no longer over the diagram.
    Leave,
    /// The generator was dropped on the diagram, with the pointer at this place.
    Drop(Generator, f32, f32),
}

/// Something that takes a generator dragged to it: the diagram of the workspace.
struct Sink {
    id: usize,
    element: NodeRef,
    signal: Callback<DragSignal>,
}

thread_local! {
    static SINKS: RefCell<Vec<Sink>> = const { RefCell::new(Vec::new()) };
    static NEXT_SINK: Cell<usize> = const { Cell::new(0) };
    static PHASE: RefCell<Phase> = const { RefCell::new(Phase::Idle) };
    /// Swallows the click that follows a drag, for a moment.
    static CLICK_GUARD: RefCell<Option<(EventListener, Timeout)>> = const { RefCell::new(None) };
}

/// A diagram takes drags: `element` is its `svg`, and `signal` is how it is told. Give the id
/// back to [`unregister`] when it goes.
pub fn register(element: NodeRef, signal: Callback<DragSignal>) -> usize {
    let id = NEXT_SINK.with(|next| {
        let id = next.get();
        next.set(id + 1);
        id
    });
    SINKS.with(|sinks| {
        sinks.borrow_mut().push(Sink {
            id,
            element,
            signal,
        });
    });
    id
}

/// The diagram is drawn in this element (which can change with its properties).
pub fn set_element(id: usize, element: &NodeRef) {
    SINKS.with(|sinks| {
        if let Some(sink) = sinks.borrow_mut().iter_mut().find(|s| s.id == id) {
            if sink.element != *element {
                sink.element = element.clone();
            }
        }
    });
}

pub fn unregister(id: usize) {
    SINKS.with(|sinks| sinks.borrow_mut().retain(|sink| sink.id != id));
}

/// What a drag of a generator needs to know, from the row it is dragged from.
#[derive(Clone)]
pub struct Source {
    pub generator: Generator,
    pub signature: Signature,
    pub workspace: Option<Diagram>,
}

enum Phase {
    Idle,
    /// The primary button is down on a row; it may become a drag.
    Pressed(Pressed),
    Dragging(Drag),
}

struct Pressed {
    press: Press,
    pointer: i32,
    source: Source,
    row: Element,
    _listeners: Vec<EventListener>,
}

struct Drag {
    pointer: i32,
    source: Source,
    /// Why the diagram cannot take the generator at all, if it cannot.
    refusal: Option<CannotAttach>,
    ghost: HtmlElement,
    /// The sink that the pointer is over.
    over: Option<usize>,
    _listeners: Vec<EventListener>,
}

/// Whether a generator is being dragged with the pointer (so `Esc` is for the drag).
#[must_use]
pub fn active() -> bool {
    PHASE.with(|phase| {
        phase
            .try_borrow()
            .is_ok_and(|phase| matches!(*phase, Phase::Dragging(_)))
    })
}

/// Drops something after the event that is being handled, which may be one of its listeners.
fn defer_drop<T: 'static>(value: T) {
    Timeout::new(0, move || drop(value)).forget();
}

fn window() -> web_sys::Window {
    web_sys::window().expect("a window")
}

fn listen<F>(name: &'static str, capture: bool, f: F) -> EventListener
where
    F: FnMut(&web_sys::Event) + 'static,
{
    let options = EventListenerOptions {
        phase: if capture {
            EventListenerPhase::Capture
        } else {
            EventListenerPhase::Bubble
        },
        passive: false,
    };
    EventListener::new_with_options(&window(), name, options, f)
}

fn at(event: &PointerEvent) -> (f64, f64) {
    (f64::from(event.client_x()), f64::from(event.client_y()))
}

/// The pointer went down on a row of the signature (the one that `event` is in).
pub fn press(event: &PointerEvent, source: Source) {
    reset();
    if event.button() != 0 || !event.is_primary() {
        return;
    }
    let Some(target) = event.target().and_then(|t| t.dyn_into::<Element>().ok()) else {
        return;
    };
    if target
        .closest("input, textarea, select, [contenteditable]")
        .ok()
        .flatten()
        .is_some()
    {
        return;
    }
    let Some(row) = target.closest(".signature__item").ok().flatten() else {
        return;
    };
    let grip = if target
        .closest(".signature__item-attach")
        .ok()
        .flatten()
        .is_some()
    {
        Grip::Handle
    } else {
        Grip::Row
    };
    // The diagram is there to be dragged to, or this is not a drag.
    let Some(diagram) = SINKS.with(|sinks| {
        sinks
            .borrow()
            .iter()
            .find_map(|sink| sink.element.cast::<Element>())
    }) else {
        return;
    };
    let middle = |element: &Element| {
        let rect = element.get_bounding_client_rect();
        rect.left() + rect.width() / 2.0
    };
    let press = Press {
        origin: at(event),
        grip,
        toward: direction_to(middle(&row), middle(&diagram)),
    };
    let pointer = event.pointer_id();
    let listeners = vec![
        listen("pointermove", false, move |e| {
            if let Some(e) = e.dyn_ref::<PointerEvent>() {
                moved(e);
            }
        }),
        listen("pointerup", false, move |e| {
            if let Some(e) = e.dyn_ref::<PointerEvent>() {
                released(e);
            }
        }),
        listen("pointercancel", false, move |e| {
            if let Some(e) = e.dyn_ref::<PointerEvent>() {
                if e.pointer_id() == pointer {
                    reset();
                }
            }
        }),
    ];
    PHASE.with(|phase| {
        *phase.borrow_mut() = Phase::Pressed(Pressed {
            press,
            pointer,
            source,
            row,
            _listeners: listeners,
        });
    });
}

/// The browser is starting a native drag of a row. Returns whether that is to be cancelled
/// because this is a drag to the diagram, which is then started.
pub fn dragstart(event: &DragEvent) -> bool {
    let place = (f64::from(event.client_x()), f64::from(event.client_y()));
    let take = PHASE.with(|phase| match &*phase.borrow() {
        Phase::Dragging(_) => Some(false),
        Phase::Pressed(pressed) => pressed.press.is_attach(place).then_some(true),
        Phase::Idle => None,
    });
    match take {
        None => false,
        Some(start) => {
            event.prevent_default();
            if start {
                begin(place);
            }
            true
        }
    }
}

/// What a movement of the pointer means.
enum Step {
    Ignore,
    Begin,
    LostPress,
    LostDrag,
    Move,
}

fn moved(event: &PointerEvent) {
    let place = at(event);
    let step = PHASE.with(|phase| match &*phase.borrow() {
        Phase::Idle => Step::Ignore,
        Phase::Pressed(p) if p.pointer != event.pointer_id() => Step::Ignore,
        Phase::Pressed(_) if event.buttons() == 0 => Step::LostPress,
        Phase::Pressed(p) => {
            if p.press.begins(place) {
                Step::Begin
            } else {
                Step::Ignore
            }
        }
        Phase::Dragging(d) if d.pointer != event.pointer_id() => Step::Ignore,
        // The button went up where nothing saw it.
        Phase::Dragging(_) if event.buttons() == 0 => Step::LostDrag,
        Phase::Dragging(_) => Step::Move,
    });
    match step {
        Step::Ignore => {}
        Step::LostPress => reset(),
        Step::LostDrag => finish(None),
        Step::Begin => begin(place),
        Step::Move => track(place),
    }
}

fn released(event: &PointerEvent) {
    if event.button() != 0 {
        return;
    }
    let state = PHASE.with(|phase| match &*phase.borrow() {
        Phase::Idle => None,
        Phase::Pressed(p) => Some((p.pointer, false)),
        Phase::Dragging(d) => Some((d.pointer, true)),
    });
    match state {
        Some((pointer, dragging)) if pointer == event.pointer_id() => {
            if dragging {
                finish(Some(at(event)));
            } else {
                // Just a click.
                reset();
            }
        }
        _ => {}
    }
}

/// Puts everything away, whatever there was.
fn reset() {
    let old = PHASE.with(|phase| std::mem::replace(&mut *phase.borrow_mut(), Phase::Idle));
    match old {
        Phase::Dragging(drag) => {
            clean_up(&drag);
            defer_drop(drag);
        }
        Phase::Pressed(pressed) => defer_drop(pressed),
        Phase::Idle => {}
    }
}

/// The press is a drag.
fn begin(place: (f64, f64)) {
    let old = PHASE.with(|phase| std::mem::replace(&mut *phase.borrow_mut(), Phase::Idle));
    let Phase::Pressed(pressed) = old else {
        PHASE.with(|phase| *phase.borrow_mut() = old);
        return;
    };
    let Pressed {
        pointer,
        source,
        row,
        _listeners: press_listeners,
        ..
    } = pressed;
    // (This may be called by one of the listeners that are dropped.)
    defer_drop(press_listeners);
    let dragged = attach_target::begin(
        &source.signature,
        source.workspace.as_ref(),
        source.generator,
    );
    let refusal = dragged.verdict.err();
    let ghost = make_ghost(&row, refusal.is_some());
    if let Some(body) = window().document().and_then(|d| d.body()) {
        let classes = body.class_list();
        classes.add_1("attach-dragging").ok();
        if refusal.is_some() {
            classes.add_1("attach-dragging--refused").ok();
        }
    }
    let listeners = vec![
        listen("pointermove", false, move |e| {
            if let Some(e) = e.dyn_ref::<PointerEvent>() {
                moved(e);
            }
        }),
        listen("pointerup", false, move |e| {
            if let Some(e) = e.dyn_ref::<PointerEvent>() {
                released(e);
            }
        }),
        listen("pointercancel", false, move |e| {
            if let Some(e) = e.dyn_ref::<PointerEvent>() {
                if e.pointer_id() == pointer {
                    finish(None);
                }
            }
        }),
        // (Before the keys of the diagram and the rest of the page: this one is for the drag.)
        listen("keydown", true, |e| {
            if let Some(e) = e.dyn_ref::<KeyboardEvent>() {
                if e.key() == "Escape" {
                    e.prevent_default();
                    e.stop_propagation();
                    finish(None);
                }
            }
        }),
        listen("blur", false, |_| finish(None)),
    ];
    PHASE.with(|phase| {
        *phase.borrow_mut() = Phase::Dragging(Drag {
            pointer,
            source,
            refusal,
            ghost,
            over: None,
            _listeners: listeners,
        });
    });
    track(place);
}

/// The ghost of the row, which follows the pointer: the row without its preview and buttons,
/// kept above and to the right of the pointer, where it hides neither what is under it nor the
/// tooltip, which is below.
fn make_ghost(row: &Element, refused: bool) -> HtmlElement {
    let rect = row.get_bounding_client_rect();
    let ghost: HtmlElement = row
        .clone_node_with_deep(true)
        .expect("a copy of the row")
        .unchecked_into();
    // (Not another row to tests and the like.)
    ghost.remove_attribute("data-generator").ok();
    ghost.remove_attribute("draggable").ok();
    ghost.class_list().add_1("signature__drag-ghost").ok();
    if refused {
        ghost
            .class_list()
            .add_1("signature__drag-ghost--refused")
            .ok();
    }
    let style = ghost.style();
    style
        .set_property("width", &format!("{}px", rect.width().min(GHOST_WIDTH)))
        .ok();
    style
        .set_property("height", &format!("{}px", rect.height()))
        .ok();
    if let Some(body) = window().document().and_then(|d| d.body()) {
        body.append_child(&ghost).ok();
    }
    ghost
}

/// The sink that is under a place in the window, if the diagram is.
fn sink_at(place: (f64, f64)) -> Option<(usize, Callback<DragSignal>)> {
    let element = window()
        .document()?
        .element_from_point(place.0 as f32, place.1 as f32)?;
    SINKS.with(|sinks| {
        sinks.borrow().iter().find_map(|sink| {
            let svg = sink.element.cast::<web_sys::Node>()?;
            svg.contains(Some(&element))
                .then(|| (sink.id, sink.signal.clone()))
        })
    })
}

fn signal_of(id: usize) -> Option<Callback<DragSignal>> {
    SINKS.with(|sinks| {
        sinks
            .borrow()
            .iter()
            .find(|sink| sink.id == id)
            .map(|sink| sink.signal.clone())
    })
}

/// The pointer is somewhere new during the drag.
fn track(place: (f64, f64)) {
    let found = sink_at(place);
    let now = found.as_ref().map(|(id, _)| *id);
    let (before, allowed) = PHASE.with(|phase| {
        let mut phase = phase.borrow_mut();
        let Phase::Dragging(drag) = &mut *phase else {
            return (None, false);
        };
        drag.ghost
            .style()
            .set_property(
                "transform",
                &format!(
                    "translate(calc({}px + 12px), calc({}px - 100% - 8px))",
                    place.0, place.1
                ),
            )
            .ok();
        let before = std::mem::replace(&mut drag.over, now);
        (before, drag.refusal.is_none())
    });
    if before != now {
        if let Some(signal) = before.and_then(signal_of) {
            signal.emit(DragSignal::Leave);
        }
    }
    // (A diagram that cannot take the generator at all is not told: nothing is outlined.)
    if let (Some((_, signal)), true) = (found, allowed) {
        signal.emit(DragSignal::Over(place.0 as f32, place.1 as f32));
    }
}

/// The drag is over: dropped at a place, or not (cancelled).
fn finish(dropped_at: Option<(f64, f64)>) {
    let old = PHASE.with(|phase| std::mem::replace(&mut *phase.borrow_mut(), Phase::Idle));
    let Phase::Dragging(drag) = old else {
        PHASE.with(|phase| *phase.borrow_mut() = old);
        return;
    };
    clean_up(&drag);
    guard_click();
    let target =
        dropped_at.and_then(|place| sink_at(place).map(|(id, signal)| (place, id, signal)));
    // What the diagram under the pointer was told it was over, if it is not what takes the drop.
    if let Some(signal) = drag
        .over
        .filter(|over| target.as_ref().map(|(_, id, _)| id) != Some(over))
        .and_then(signal_of)
    {
        signal.emit(DragSignal::Leave);
    }
    match (target, &drag.refusal) {
        (Some((place, _, signal)), None) => {
            signal.emit(DragSignal::Drop(
                drag.source.generator,
                place.0 as f32,
                place.1 as f32,
            ));
        }
        (Some((_, _, signal)), Some(reason)) => {
            signal.emit(DragSignal::Leave);
            toast(Toast::warn(reason.reason()));
        }
        (None, _) => {
            if let Some(signal) = drag.over.and_then(signal_of) {
                signal.emit(DragSignal::Leave);
            }
        }
    }
    defer_drop(drag);
}

/// Takes the ghost and the rest of the signs of the drag away.
fn clean_up(drag: &Drag) {
    drag.ghost.remove();
    if let Some(body) = window().document().and_then(|d| d.body()) {
        let classes = body.class_list();
        classes.remove_1("attach-dragging").ok();
        classes.remove_1("attach-dragging--refused").ok();
    }
    attach_target::end();
}

/// Swallows the click that follows the pointer going up after a drag: the browser sends it to
/// what the pointer went down and up on, which may be a row or its button.
fn guard_click() {
    let options = EventListenerOptions {
        phase: EventListenerPhase::Capture,
        passive: false,
    };
    let listener = EventListener::once_with_options(&window(), "click", options, |event| {
        event.prevent_default();
        event.stop_immediate_propagation();
    });
    let timeout = Timeout::new(CLICK_GUARD_MS, || {
        CLICK_GUARD.with(|guard| guard.borrow_mut().take());
    });
    CLICK_GUARD.with(|guard| *guard.borrow_mut() = Some((listener, timeout)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(grip: Grip, toward: f64) -> Press {
        Press {
            origin: (100.0, 100.0),
            grip,
            toward,
        }
    }

    #[test]
    fn a_press_is_not_a_drag_until_the_pointer_has_moved_the_threshold() {
        let p = press(Grip::Handle, 1.0);
        assert!(!p.far_enough((100.0, 100.0)));
        assert!(!p.far_enough((103.0, 100.0)));
        assert!(!p.far_enough((102.0, 102.0)));
        assert!(p.far_enough((104.0, 100.0)));
        assert!(p.far_enough((103.0, 103.0)));
        assert!(!p.begins((102.0, 102.0)));
        assert!(p.begins((100.0, 90.0)));
    }

    #[test]
    fn the_handle_is_a_drag_to_the_diagram_whichever_way_it_moves() {
        let p = press(Grip::Handle, 1.0);
        for to in [(200.0, 100.0), (0.0, 100.0), (100.0, 0.0), (100.0, 200.0)] {
            assert!(p.begins(to), "{to:?}");
        }
    }

    #[test]
    fn the_row_is_a_drag_to_the_diagram_only_when_it_moves_towards_it() {
        let right = press(Grip::Row, 1.0);
        assert!(right.begins((120.0, 100.0)));
        // Within 45 degrees of the horizontal, including the edge.
        assert!(right.begins((120.0, 110.0)));
        assert!(right.begins((120.0, 120.0)));
        assert!(right.begins((120.0, 80.0)));
        // Steeper, or the wrong way: for the browser (moving it in the signature).
        assert!(!right.begins((110.0, 130.0)));
        assert!(!right.begins((100.0, 130.0)));
        assert!(!right.begins((100.0, 70.0)));
        assert!(!right.begins((80.0, 100.0)));
        assert!(!right.begins((80.0, 90.0)));

        let left = press(Grip::Row, -1.0);
        assert!(left.begins((80.0, 100.0)));
        assert!(!left.begins((120.0, 100.0)));
        assert!(!left.begins((95.0, 130.0)));
    }

    #[test]
    fn a_row_with_the_diagram_neither_way_is_left_to_the_browser() {
        let p = press(Grip::Row, 0.0);
        for to in [(200.0, 100.0), (0.0, 100.0), (100.0, 0.0), (100.0, 200.0)] {
            assert!(!p.is_attach(to), "{to:?}");
        }
        // Not having moved at all is not towards anything either.
        assert!(!press(Grip::Row, 1.0).is_attach((100.0, 100.0)));
    }

    #[test]
    fn the_diagram_is_to_the_side_it_is_on() {
        assert_eq!(direction_to(100.0, 600.0), 1.0);
        assert_eq!(direction_to(600.0, 100.0), -1.0);
        assert_eq!(direction_to(100.0, 110.0), 0.0);
        assert_eq!(direction_to(100.0, 100.0), 0.0);
    }
}
