//! The tooltip that follows the pointer over a diagram, showing a label (rendered by KaTeX)
//! and where in the diagram the pointer is.
//!
//! The diagram tells the tooltip what to show and where through [`show`], [`move_to`] and
//! [`hide`], and not through the properties of a component, so that moving the pointer does
//! not make the diagram, or the app, render again: a move only changes the position of the
//! element, directly. The content is only rendered again when it changes.
//!
//! The tooltip never takes the pointer (`pointer-events: none` in `styles.css`), so showing
//! it cannot cause the diagram below to see the pointer leave, and flicker.

use std::cell::RefCell;

use web_sys::HtmlElement;
use yew::prelude::*;

use super::tex::TexSpan;

/// How far the tooltip is from the pointer, in pixels.
const OFFSET_X: f64 = 14.0;
const OFFSET_Y: f64 = 18.0;
/// How close the tooltip may get to the edge of the window, in pixels.
const MARGIN: f64 = 8.0;

/// What a tooltip says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Content {
    /// A label (see [`homotopy_common::tex`]), such as `$f^{-1}$`.
    pub label: String,
    /// Plain text, shown muted after the label.
    pub detail: String,
    /// The sheets behind the place, if there are any to choose from (see `homotopy_model::sheets`).
    pub sheets: Option<SheetStack>,
}

/// The sheets to choose from at a place: the stack that is listed under the label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetStack {
    /// The sheets fixed and the one being chosen, as `z₃=2 › z₄=0`, when one is chosen.
    pub breadcrumb: Option<String>,
    /// The name of the coordinate that the rows are sheets of (`z₃`).
    pub coordinate: String,
    pub rows: Vec<SheetRow>,
    /// Rows that are left out for lack of room.
    pub hidden: usize,
    /// What the keys do, or what stops them.
    pub hint: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetRow {
    /// The height of the sheet.
    pub z: usize,
    /// A label (see [`homotopy_common::tex`]).
    pub label: String,
    pub current: bool,
    /// Whether this is the one the view draws.
    pub front: bool,
    /// What kind of shape the sheet is of, when the stack has those of more than one.
    pub kind: Option<&'static str>,
}

#[derive(Debug)]
pub enum Event {
    Show(Content, (f64, f64)),
    Move((f64, f64)),
    Hide,
}

thread_local! {
    static TOOLTIP: RefCell<Option<Callback<Event>>> = const { RefCell::new(None) };
}

fn emit(event: Event) {
    let callback = TOOLTIP.with(|tooltip| tooltip.try_borrow().ok().and_then(|t| t.clone()));
    if let Some(callback) = callback {
        callback.emit(event);
    }
}

/// Shows the tooltip with the given content, near the pointer at the given position in the
/// window.
pub fn show(content: Content, pointer: (f64, f64)) {
    emit(Event::Show(content, pointer));
}

/// Moves the tooltip, if it is shown, to follow the pointer.
pub fn move_to(pointer: (f64, f64)) {
    emit(Event::Move(pointer));
}

/// Takes the tooltip down.
pub fn hide() {
    emit(Event::Hide);
}

pub enum Msg {
    Event(Event),
}

pub struct HoverTooltip {
    content: Option<Content>,
    pointer: (f64, f64),
    element: NodeRef,
}

impl HoverTooltip {
    /// Puts the element near the pointer, on the side of it that has room.
    fn place(&self) {
        let Some(element) = self.element.cast::<HtmlElement>() else {
            return;
        };
        let (x, y) = self.pointer;
        let (width, height) = (
            f64::from(element.offset_width()),
            f64::from(element.offset_height()),
        );
        let window = web_sys::window().unwrap();
        let inner = |size: Result<wasm_bindgen::JsValue, _>| size.ok().and_then(|v| v.as_f64());
        let (inner_width, inner_height) = (
            inner(window.inner_width()).unwrap_or(f64::INFINITY),
            inner(window.inner_height()).unwrap_or(f64::INFINITY),
        );
        let left = if x + OFFSET_X + width + MARGIN > inner_width {
            (x - OFFSET_X - width).max(MARGIN)
        } else {
            x + OFFSET_X
        };
        let top = if y + OFFSET_Y + height + MARGIN > inner_height {
            (y - OFFSET_Y - height).max(MARGIN)
        } else {
            y + OFFSET_Y
        };
        // Only fails for a style that cannot be set, which this is not.
        element
            .style()
            .set_property(
                "transform",
                &format!("translate({}px, {}px)", left.round(), top.round()),
            )
            .ok();
    }
}

impl HoverTooltip {
    fn view_sheets(stack: &SheetStack) -> Html {
        let coordinate = stack.coordinate.clone();
        html! {
            <div class="hover-tooltip__sheets" data-sheets="">
                if let Some(breadcrumb) = &stack.breadcrumb {
                    <div class="hover-tooltip__crumbs">{breadcrumb.clone()}</div>
                }
                <ul class="hover-tooltip__stack">
                    { for stack.rows.iter().map(|row| html! {
                        <li
                            class={classes!(
                                "hover-tooltip__sheet",
                                row.current.then_some("hover-tooltip__sheet--current"),
                            )}
                            data-z={row.z.to_string()}
                        >
                            <span class="hover-tooltip__z">{format!("{coordinate}={}", row.z)}</span>
                            <TexSpan class="hover-tooltip__sheet-label" raw_tex={row.label.clone()} />
                            if let Some(kind) = row.kind {
                                <span class="hover-tooltip__tag">{kind}</span>
                            }
                            if row.front {
                                <span class="hover-tooltip__tag">{"front"}</span>
                            }
                        </li>
                    }) }
                </ul>
                if stack.hidden > 0 {
                    <div class="hover-tooltip__more">{format!("and {} more", stack.hidden)}</div>
                }
                <div class="hover-tooltip__hint">{stack.hint.clone()}</div>
            </div>
        }
    }
}

impl Component for HoverTooltip {
    type Message = Msg;
    type Properties = ();

    fn create(ctx: &Context<Self>) -> Self {
        let callback = ctx.link().callback(Msg::Event);
        TOOLTIP.with(|tooltip| *tooltip.borrow_mut() = Some(callback));
        Self {
            content: None,
            pointer: (0.0, 0.0),
            element: NodeRef::default(),
        }
    }

    fn update(&mut self, _ctx: &Context<Self>, msg: Self::Message) -> bool {
        match msg {
            Msg::Event(Event::Show(content, pointer)) => {
                self.pointer = pointer;
                if self.content.as_ref() == Some(&content) {
                    self.place();
                    false
                } else {
                    // Placed once the new content is on the page, and has its size.
                    self.content = Some(content);
                    true
                }
            }
            Msg::Event(Event::Move(pointer)) => {
                self.pointer = pointer;
                if self.content.is_some() {
                    self.place();
                }
                false
            }
            Msg::Event(Event::Hide) => self.content.take().is_some(),
        }
    }

    fn rendered(&mut self, _ctx: &Context<Self>, _first_render: bool) {
        if self.content.is_some() {
            self.place();
        }
    }

    fn view(&self, _ctx: &Context<Self>) -> Html {
        let Some(content) = &self.content else {
            return html! {};
        };
        html! {
            <div class="hover-tooltip" role="tooltip" ref={self.element.clone()}>
                <TexSpan class="hover-tooltip__label" raw_tex={content.label.clone()} />
                <span class="hover-tooltip__detail">{content.detail.clone()}</span>
                {content.sheets.as_ref().map(Self::view_sheets)}
            </div>
        }
    }

    fn destroy(&mut self, _ctx: &Context<Self>) {
        TOOLTIP.with(|tooltip| *tooltip.borrow_mut() = None);
    }
}
