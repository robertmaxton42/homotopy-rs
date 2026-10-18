//! Showing what deleting a generator (or a folder) would delete, while the pointer is on its
//! delete button or the button has the focus, and which generators those are and why.
//!
//! Deleting a generator deletes the generators that depend on it and so on (see
//! [`Signature::removal_reasons`]), which can be far from it in the signature, in a closed folder
//! or among the instances of a schema. The preview is made from the same code as the deletion,
//! so it cannot disagree with it.
//!
//! Like the tooltip and the busy indicator, it is told by calls ([`show`], [`hide`]) and not by
//! properties, so that hovering a button does not render the signature. Rows that would go get
//! `data-removal` (the stylesheet tints them and strikes out their names), the rows that hold
//! some that would go without being shown (a closed folder, a schema whose instances are closed)
//! get `data-removal-inside="N"`, both written to the elements directly, and a box under the row
//! ([`RemovalBox`], a small component of its own, so only it renders) says how many and, when
//! it is expanded, which, in groups, each with the reason it goes. Finding out which generators
//! the diagrams mention is what costs, and is done a few diagrams at a time
//! (`Signature::removal_preview_reasons`), yielding between, on a signature that has not been
//! looked at before: the box says it is checking until it is done.
//!
//! **Staying open.** The box is something to read and click, so it does not go with the pointer
//! leaving the button: it is held while the pointer is within [`GRACE_PX`] of the button or the
//! box (or the space between them: the distance is to the rectangle that holds both) or the
//! keyboard focus is in one of them, and is closed [`CLOSE_AFTER_MS`] after neither is the case,
//! at once by `Esc` or a press elsewhere. Going to a name in the list holds it until the pointer
//! next moves (the signature may have moved under it, and the box with it).

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gloo::events::{EventListener, EventListenerOptions, EventListenerPhase};
use gloo_timers::future::TimeoutFuture;
use homotopy_common::{
    hash::{FastHashMap, FastHashSet},
    idx::Idx,
    tex::seal,
    tree::Node,
};
use homotopy_core::signature::Signature as _;
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement, KeyboardEvent, MouseEvent};
use yew::{html::Scope, prelude::*};

use crate::{
    app::tex::TexSpan,
    components::icon::{Icon, IconSize},
    model::{
        proof::{self, RemovalReason, Signature, SignatureEdit, SignatureItem},
        Action,
    },
};

/// How many diagrams that have not been looked at before one step of the preview looks at.
const DIAGRAMS_PER_STEP: usize = 16;
/// The most generators the expanded box lists (then "and N more", as the search notice does).
pub const MOST_LISTED: usize = 50;
/// How far from the button and the box the pointer may be and still hold the box open.
pub const GRACE_PX: f64 = 40.0;
/// How long the pointer has to be farther than that, and the focus elsewhere, for the box to
/// close.
pub const CLOSE_AFTER_MS: u32 = 300;
/// How long the row that was gone to is marked.
const FLASH_MS: u32 = 1400;

/// Where in the box a generator that goes is listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    /// Under the folder deleted.
    Contents,
    /// Elsewhere: mentions or depends on something that goes.
    Dependents,
    /// An instance of a generator that goes.
    Instances,
}

impl Group {
    #[must_use]
    pub const fn title(self, folder: bool) -> &'static str {
        match (self, folder) {
            (Self::Contents, _) => "In this folder",
            (Self::Dependents, true) => "Outside it, mentioning or depending on what is in it",
            (Self::Dependents, false) => "Mentioning or depending on it",
            (Self::Instances, _) => "Instances",
        }
    }
}

/// One generator that goes, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub node: Node,
    /// Its name (TeX).
    pub name: String,
    pub group: Group,
    /// "mentions", "depends on placeholder", ...
    pub how: &'static str,
    /// The name (TeX) of the generator that `how` is about.
    pub of: Option<String>,
}

/// What a deletion does, in the terms of the rows that are on show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// The nodes that go and have a row, other than the one being deleted.
    pub going: Vec<Node>,
    /// The rows that stay and hold generators that go that are out of sight (a closed folder,
    /// the instances of a schema), with how many.
    pub inside: Vec<(Node, usize)>,
    /// Generators that go with the one deleted, being under it (the contents of a folder, the
    /// instances of a generator).
    pub contained: usize,
    /// Generators that go and are not under it.
    pub more: usize,
    /// Of those, how many are out of sight inside a row that stays (see `inside`).
    pub hidden: usize,
    /// The generators that go, other than the one deleted, in groups (in the order of
    /// [`Group`], each in the order the cascade found them) with the reason.
    pub entries: Vec<Entry>,
    /// Whether the node deleted is a folder.
    pub folder: bool,
}

/// What deleting `pressed` deletes (`removed`: [`Signature::removal_reasons`]), given which
/// nodes have a row on show. Pure, and the same wherever the rows are.
#[must_use]
pub fn plan(
    signature: &Signature,
    pressed: Node,
    removed: &[(Node, RemovalReason)],
    has_row: impl Fn(Node) -> bool,
) -> Plan {
    let tree = signature.as_tree();
    let under: FastHashSet<Node> = tree.descendents_of(pressed).collect();
    let going: FastHashSet<Node> = removed.iter().map(|r| r.0).collect();
    let name_of = |g| {
        signature
            .generator_info(g)
            .map(|info| info.name.clone())
            .unwrap_or_default()
    };
    let mut plan = Plan {
        folder: signature.find_generator(pressed).is_none(),
        ..Plan::default()
    };
    let mut inside: FastHashMap<Node, usize> = FastHashMap::default();
    let mut order: Vec<Node> = vec![];
    for &(node, why) in removed {
        if node == pressed {
            continue;
        }
        let item = signature
            .find_generator(node)
            .and_then(|g| signature.generator_info(g));
        if let Some(info) = item {
            if under.contains(&node) {
                plan.contained += 1;
            } else {
                plan.more += 1;
            }
            let (group, how, of) = match why {
                RemovalReason::Mentions(g) => (Group::Dependents, "mentions", Some(name_of(g))),
                RemovalReason::Substitutes(g) => (
                    Group::Dependents,
                    "mentions, in its replacements,",
                    Some(name_of(g)),
                ),
                RemovalReason::DependsOn(g) => (
                    Group::Dependents,
                    "depends on placeholder",
                    Some(name_of(g)),
                ),
                RemovalReason::Itself | RemovalReason::Within(_) => {
                    // (Under the generator whose instance it is, and not the node whose
                    // removal found it: that is the folder, for all that is in it.)
                    match tree
                        .ancestors_of(node)
                        .nth(1)
                        .and_then(|parent| signature.find_generator(parent))
                    {
                        Some(parent) => (Group::Instances, "instance of", Some(name_of(parent))),
                        None => (Group::Contents, "in the folder", None),
                    }
                }
            };
            plan.entries.push(Entry {
                node,
                name: info.name.clone(),
                group,
                how,
                of,
            });
        }
        if has_row(node) {
            plan.going.push(node);
        } else if item.is_some() {
            // The nearest row on show holds it, unless that goes too (and then it goes with it
            // in plain sight).
            if let Some(holder) = tree.ancestors_of(node).skip(1).find(|&a| has_row(a)) {
                if holder != pressed && !going.contains(&holder) {
                    plan.hidden += 1;
                    let count = inside.entry(holder).or_insert_with(|| {
                        order.push(holder);
                        0
                    });
                    *count += 1;
                }
            }
        }
    }
    plan.inside = order.into_iter().map(|n| (n, inside[&n])).collect();
    plan.entries.sort_by_key(|e| e.group); // (stable)
    plan
}

impl Plan {
    /// What the note says.
    #[must_use]
    pub fn text(&self) -> String {
        let plural =
            |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let mut pieces = vec![];
        if self.contained > 0 {
            pieces.push(if self.folder {
                format!(
                    "{} inside",
                    plural(self.contained, "generator", "generators")
                )
            } else {
                plural(self.contained, "instance", "instances")
            });
        }
        if self.more > 0 {
            pieces.push(if self.folder {
                format!("{} more outside it", self.more)
            } else {
                format!("{} more", self.more)
            });
        }
        if pieces.is_empty() {
            return if self.folder {
                "Nothing outside it depends on it".to_owned()
            } else {
                "Nothing else depends on it".to_owned()
            };
        }
        let mut text = format!("Deletes {}", pieces.join(" and "));
        if self.hidden > 0 {
            text = format!("{text} ({} out of sight)", self.hidden);
        }
        text
    }

    /// The entries to list, and how many more there are.
    #[must_use]
    pub fn listed(&self) -> (&[Entry], usize) {
        let shown = self.entries.len().min(MOST_LISTED);
        (&self.entries[..shown], self.entries.len() - shown)
    }
}

/// What has to be opened to show the row of `target`: the closed folders it is in and the
/// closed instances of the generators it is an instance of, outermost first.
#[must_use]
pub fn closed_above(signature: &Signature, target: Node) -> Vec<SignatureEdit> {
    let tree = signature.as_tree();
    let mut closed: Vec<SignatureEdit> = tree
        .ancestors_of(target)
        .skip(1)
        .filter_map(|ancestor| match tree.get(ancestor)?.inner() {
            SignatureItem::Folder(info) if !info.open => {
                Some(SignatureEdit::ToggleFolder(ancestor))
            }
            SignatureItem::Item(info) if !info.instances_open => {
                Some(SignatureEdit::ToggleInstances(ancestor))
            }
            _ => None,
        })
        .collect();
    closed.reverse();
    closed
}

// What holds the box open.

/// What keeps the box open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hold {
    /// The pointer is within [`GRACE_PX`] of the button or the box.
    pub pointer: bool,
    /// The keyboard focus is in the button or the box.
    pub focus: bool,
    /// Someone went to a name in the list, which may have moved the signature (and the box)
    /// from under the pointer: held until the pointer moves, and then it is where it is.
    pub pinned: bool,
}

impl Hold {
    #[must_use]
    pub const fn held(self) -> bool {
        self.pointer || self.focus || self.pinned
    }

    /// The pointer has moved; whether it is near. A pin is let go of.
    #[must_use]
    pub const fn pointer_is(self, near: bool) -> Self {
        Self {
            pointer: near,
            pinned: false,
            ..self
        }
    }
}

/// A rectangle on the page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl Rect {
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    /// How far the point is from the rectangle (0 inside it).
    #[must_use]
    pub fn distance(self, x: f64, y: f64) -> f64 {
        let dx = (self.left - x).max(x - self.right).max(0.0);
        let dy = (self.top - y).max(y - self.bottom).max(0.0);
        dx.hypot(dy)
    }
}

fn rect_of(element: &Element) -> Option<Rect> {
    let r = element.get_bounding_client_rect();
    (r.width() > 0.0 && r.height() > 0.0).then(|| Rect {
        left: r.left(),
        top: r.top(),
        right: r.right(),
        bottom: r.bottom(),
    })
}

fn button_id(node: Node) -> String {
    format!("removal-button-{}", node.index())
}

fn box_id(node: Node) -> String {
    format!("removal-box-{}", node.index())
}

fn note_id(node: Node) -> String {
    format!("removal-note-{}", node.index())
}

fn toggle_id(node: Node) -> String {
    format!("removal-toggle-{}", node.index())
}

fn by_id(id: &str) -> Option<Element> {
    document()?.get_element_by_id(id)
}

fn focus_button(node: Node) {
    if let Some(button) = by_id(&button_id(node)).and_then(|b| b.dyn_into::<HtmlElement>().ok()) {
        QUIET_FOCUS.with(|q| q.set(true));
        button.focus().ok();
    }
}

/// The rectangle that holds the button and the box.
fn area(node: Node) -> Option<Rect> {
    let button = by_id(&button_id(node)).and_then(|e| rect_of(&e));
    let held = by_id(&box_id(node)).and_then(|e| rect_of(&e));
    match (button, held) {
        (Some(a), Some(b)) => Some(a.union(b)),
        (a, b) => a.or(b),
    }
}

fn inside(node: Node, target: &web_sys::EventTarget) -> bool {
    let Some(target) = target.dyn_ref::<web_sys::Node>() else {
        return false;
    };
    [button_id(node), box_id(node)]
        .iter()
        .filter_map(|id| by_id(id))
        .any(|e| e.contains(Some(target)))
}

// The showing.

/// What a box is about: the signature and what deleting its node deletes.
type Known = (Signature, Rc<Vec<(Node, RemovalReason)>>);

/// The preview that is on show.
struct Session {
    token: u64,
    node: Node,
    hold: Hold,
    /// Counts the changes of `hold`, so that a timer can tell that it is out of date.
    epoch: u64,
    /// What the box is about, when it is known.
    known: Option<Known>,
    _listeners: Vec<EventListener>,
}

thread_local! {
    static TOKEN: Cell<u64> = const { Cell::new(0) };
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    /// The boxes there are, to tell them what to show.
    static BOXES: RefCell<FastHashMap<Node, Scope<RemovalBox>>> =
        RefCell::new(FastHashMap::default());
    /// The next focus of a delete button is not a request to show (focus given back to it).
    static QUIET_FOCUS: Cell<bool> = const { Cell::new(false) };
    /// Where the pointer was last seen, to tell a move of the pointer from the browser saying
    /// again where it is after the page moved under it.
    static POINTER_AT: Cell<(i32, i32)> = const { Cell::new((0, 0)) };
}

fn document() -> Option<web_sys::Document> {
    web_sys::window()?.document()
}

fn tell(node: Node, message: BoxMessage) {
    if let Some(scope) = BOXES.with(|b| b.borrow().get(&node).cloned()) {
        scope.send_message(message);
    }
}

fn clear_marks() {
    let Some(document) = document() else {
        return;
    };
    for selector in ["[data-removal]", "[data-removal-inside]"] {
        if let Ok(rows) = document.query_selector_all(selector) {
            for i in 0..rows.length() {
                if let Some(row) = rows.item(i).and_then(|n| n.dyn_into::<Element>().ok()) {
                    row.remove_attribute("data-removal").ok();
                    row.remove_attribute("data-removal-inside").ok();
                }
            }
        }
    }
}

fn row_of(node: Node) -> Option<Element> {
    document()?
        .query_selector(&format!("[data-node=\"{}\"]", node.index()))
        .ok()
        .flatten()
}

/// Marks the rows on show, and tells the box.
fn apply(signature: &Signature, pressed: Node, removed: &[(Node, RemovalReason)]) {
    let Some(document) = document() else {
        return;
    };
    clear_marks();
    let mut rows: FastHashMap<Node, Element> = FastHashMap::default();
    if let Ok(found) = document.query_selector_all("[data-node]") {
        for i in 0..found.length() {
            if let Some(row) = found.item(i).and_then(|n| n.dyn_into::<Element>().ok()) {
                if let Some(index) = row.get_attribute("data-node").and_then(|s| s.parse().ok()) {
                    rows.insert(Node::new(index), row);
                }
            }
        }
    }
    let plan = plan(signature, pressed, removed, |n| rows.contains_key(&n));
    for node in &plan.going {
        if let Some(row) = rows.get(node) {
            row.set_attribute("data-removal", "").ok();
        }
    }
    for (node, count) in &plan.inside {
        if let Some(row) = rows.get(node) {
            row.set_attribute("data-removal-inside", &count.to_string())
                .ok();
        }
    }
    tell(pressed, BoxMessage::Set(Content::Ready(Rc::new(plan))));
}

/// Marks the rows again (some have come or gone) and updates the box.
fn refresh(token: u64) {
    let known = SESSION.with(|s| {
        s.borrow()
            .as_ref()
            .filter(|s| s.token == token)
            .and_then(|s| s.known.clone().map(|k| (s.node, k)))
    });
    if let Some((node, (signature, removed))) = known {
        apply(&signature, node, &removed);
    }
}

/// Which way `show` was asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    Pointer,
    Focus,
}

/// Starts showing what deleting `node` deletes, or, if that is on show already, holds it.
pub fn show(signature: &Signature, node: Node, via: Via) {
    let shown = SESSION.with(|s| s.borrow().as_ref().is_some_and(|s| s.node == node));
    if shown {
        set_hold(|h| match via {
            Via::Pointer => h.pointer_is(true),
            Via::Focus => Hold { focus: true, ..h },
        });
        return;
    }
    hide();
    let token = TOKEN.with(|t| {
        t.set(t.get() + 1);
        t.get()
    });
    let session = Session {
        token,
        node,
        hold: Hold {
            pointer: via == Via::Pointer,
            focus: via == Via::Focus,
            pinned: false,
        },
        epoch: 0,
        known: None,
        _listeners: listeners(node),
    };
    SESSION.with(|s| *s.borrow_mut() = Some(session));
    tell(node, BoxMessage::Set(Content::Checking));
    step(signature.clone(), node, token);
}

fn step(signature: Signature, node: Node, token: u64) {
    if TOKEN.with(Cell::get) != token {
        return;
    }
    if let Some(removed) = signature.removal_preview_reasons(node, DIAGRAMS_PER_STEP) {
        let removed = Rc::new(removed);
        let live = SESSION.with(|s| {
            s.borrow_mut()
                .as_mut()
                .filter(|s| s.token == token)
                .map(|s| s.known = Some((signature.clone(), removed.clone())))
                .is_some()
        });
        if live {
            apply(&signature, node, &removed);
        }
    } else {
        wasm_bindgen_futures::spawn_local(async move {
            TimeoutFuture::new(0).await;
            step(signature, node, token);
        });
    }
}

/// Stops showing it. (The listeners are dropped a tick later: one of them may be calling this.)
pub fn hide() {
    TOKEN.with(|t| t.set(t.get() + 1));
    if let Some(session) = SESSION.with(|s| s.borrow_mut().take()) {
        clear_marks();
        tell(session.node, BoxMessage::Set(Content::Hidden));
        wasm_bindgen_futures::spawn_local(async move {
            TimeoutFuture::new(0).await;
            drop(session);
        });
    }
}

/// `node` is going away: stops showing what it would delete, if that is what is on show.
pub fn forget(node: Node) {
    if SESSION.with(|s| s.borrow().as_ref().is_some_and(|s| s.node == node)) {
        hide();
    }
}

/// Changes what holds the box, and closes it a little after nothing does.
fn set_hold(change: impl FnOnce(Hold) -> Hold) {
    let due = SESSION.with(|s| {
        let mut s = s.borrow_mut();
        let session = s.as_mut()?;
        let hold = change(session.hold);
        if hold == session.hold {
            return None;
        }
        session.hold = hold;
        session.epoch += 1;
        (!hold.held()).then_some((session.token, session.epoch))
    });
    if let Some((token, epoch)) = due {
        wasm_bindgen_futures::spawn_local(async move {
            TimeoutFuture::new(CLOSE_AFTER_MS).await;
            let still = SESSION.with(|s| {
                s.borrow()
                    .as_ref()
                    .is_some_and(|s| s.token == token && s.epoch == epoch && !s.hold.held())
            });
            if still {
                hide();
            }
        });
    }
}

fn window_listener<F>(name: &'static str, capture: bool, f: F) -> EventListener
where
    F: FnMut(&web_sys::Event) + 'static,
{
    let options = EventListenerOptions {
        phase: if capture {
            EventListenerPhase::Capture
        } else {
            EventListenerPhase::Bubble
        },
        passive: true,
    };
    EventListener::new_with_options(&web_sys::window().expect("a window"), name, options, f)
}

fn listeners(node: Node) -> Vec<EventListener> {
    let moved = window_listener("pointermove", false, move |e| {
        if let Some(e) = e.dyn_ref::<MouseEvent>() {
            let at = (e.client_x(), e.client_y());
            if POINTER_AT.with(|p| p.replace(at)) == at {
                return;
            }
            let (x, y) = (f64::from(at.0), f64::from(at.1));
            let near = area(node).is_some_and(|a| a.distance(x, y) <= GRACE_PX);
            set_hold(|h| h.pointer_is(near));
        }
    });
    let left_the_window = window_listener("pointerout", false, move |e| {
        if e.dyn_ref::<MouseEvent>()
            .is_some_and(|e| e.related_target().is_none())
        {
            set_hold(|h| h.pointer_is(false));
        }
    });
    let pressed = window_listener("pointerdown", true, move |e| {
        if let Some(e) = e.dyn_ref::<MouseEvent>() {
            POINTER_AT.with(|p| p.set((e.client_x(), e.client_y())));
        }
        if e.target().is_some_and(|t| inside(node, &t)) {
            set_hold(|h| h.pointer_is(true));
        } else {
            hide();
        }
    });
    let keys = window_listener("keydown", true, move |e| {
        if let Some(e) = e.dyn_ref::<KeyboardEvent>() {
            if e.key() == "Escape" {
                let focused = document()
                    .and_then(|d| d.active_element())
                    .is_some_and(|a| inside(node, &a));
                hide();
                if focused {
                    // (Back where it came from, without asking for the preview again.)
                    focus_button(node);
                }
            }
        }
    });
    let focus_in = window_listener("focusin", false, move |e| {
        let Some(target) = e.target() else { return };
        let visible = target
            .dyn_ref::<Element>()
            .is_some_and(|el| el.matches(":focus-visible").unwrap_or(false));
        let within = inside(node, &target);
        // (A box clicked with the pointer does not hold with its focus: the pointer does.)
        set_hold(|h| Hold {
            focus: within && visible,
            ..h
        });
    });
    let focus_out = window_listener("focusout", false, move |e| {
        let to = e
            .dyn_ref::<web_sys::FocusEvent>()
            .and_then(web_sys::FocusEvent::related_target);
        if to.is_none_or(|to| !inside(node, &to)) {
            set_hold(|h| Hold { focus: false, ..h });
        }
    });
    vec![moved, left_the_window, pressed, keys, focus_in, focus_out]
}

// The box.

/// What the box shows.
#[derive(Clone, Debug, Default)]
pub enum Content {
    #[default]
    Hidden,
    Checking,
    Ready(Rc<Plan>),
}

pub enum BoxMessage {
    Set(Content),
    /// Open or close the list.
    Toggle,
    /// Go to the row of a generator in the list, with the pointer where it is.
    Go(Node),
}

#[derive(Properties, Clone)]
pub struct BoxProps {
    pub node: Node,
    pub signature: Signature,
    pub dispatch: Callback<Action>,
}

impl PartialEq for BoxProps {
    fn eq(&self, other: &Self) -> bool {
        self.node == other.node && self.signature == other.signature
    }
}

/// The box under a row being edited that says what deleting it would delete, and, expanded,
/// which generators and why. It has a state of its own: the signature is not rendered when it
/// changes.
pub struct RemovalBox {
    content: Content,
    expanded: bool,
    /// The list has just been opened: bring it into view when it is on the page.
    scroll_to_it: bool,
}

impl Component for RemovalBox {
    type Message = BoxMessage;
    type Properties = BoxProps;

    fn create(ctx: &Context<Self>) -> Self {
        let node = ctx.props().node;
        BOXES.with(|b| b.borrow_mut().insert(node, ctx.link().clone()));
        Self {
            content: Content::Hidden,
            expanded: false,
            scroll_to_it: false,
        }
    }

    fn rendered(&mut self, ctx: &Context<Self>, _first_render: bool) {
        if std::mem::take(&mut self.scroll_to_it) {
            if let Some(held) = by_id(&box_id(ctx.props().node)) {
                scroll_nearest(&held);
            }
        }
    }

    fn update(&mut self, ctx: &Context<Self>, msg: Self::Message) -> bool {
        match msg {
            BoxMessage::Set(content) => {
                if matches!(content, Content::Hidden) {
                    self.expanded = false;
                }
                self.content = content;
                true
            }
            BoxMessage::Toggle => {
                self.expanded = !self.expanded;
                self.scroll_to_it = self.expanded;
                true
            }
            BoxMessage::Go(target) => {
                let props = ctx.props();
                go_to(&props.signature, &props.dispatch, props.node, target);
                false
            }
        }
    }

    fn destroy(&mut self, ctx: &Context<Self>) {
        let node = ctx.props().node;
        BOXES.with(|b| b.borrow_mut().remove(&node));
        forget(node);
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        let node = ctx.props().node;
        // Shift+Tab from the first thing in the box goes back to the button.
        let on_key = Callback::from(move |e: KeyboardEvent| {
            if e.key() == "Tab"
                && e.shift_key()
                && e.target()
                    .and_then(|t| t.dyn_into::<Element>().ok())
                    .is_some_and(|t| t.id() == toggle_id(node))
            {
                e.prevent_default();
                focus_button(node);
            }
        });
        let root = |children: Html| {
            html! {
                <div class="signature__removal-note" id={box_id(node)} onkeydown={on_key.clone()}>
                    {children}
                </div>
            }
        };
        match &self.content {
            Content::Hidden => html! {},
            Content::Checking => root(html! {
                <span id={note_id(node)} role="status">
                    {"Checking what depends on it\u{2026}"}
                </span>
            }),
            Content::Ready(plan) => {
                let text = plan.text();
                if plan.entries.is_empty() {
                    return root(html! {
                        <span id={note_id(node)} role="status">{text}</span>
                    });
                }
                let toggle = ctx.link().callback(|_: MouseEvent| BoxMessage::Toggle);
                root(html! {
                    <>
                        <button
                            class="removal-box__toggle"
                            id={toggle_id(node)}
                            type="button"
                            aria-expanded={self.expanded.to_string()}
                            aria-controls={format!("removal-list-{}", node.index())}
                            title={if self.expanded {
                                "Hide the generators"
                            } else {
                                "Show which generators"
                            }}
                            onclick={toggle}
                        >
                            <span class="removal-box__arrow" aria-hidden="true">
                                {if self.expanded { "\u{25be}" } else { "\u{25b8}" }}
                            </span>
                            <span id={note_id(node)} role="status">{text}</span>
                        </button>
                        if self.expanded {
                            {Self::view_list(ctx, plan)}
                        }
                    </>
                })
            }
        }
    }
}

impl RemovalBox {
    fn view_list(ctx: &Context<Self>, plan: &Plan) -> Html {
        let node = ctx.props().node;
        let (shown, more) = plan.listed();
        let mut groups: Vec<(Group, Vec<&Entry>)> = vec![];
        for entry in shown {
            match groups.last_mut() {
                Some((group, entries)) if *group == entry.group => entries.push(entry),
                _ => groups.push((entry.group, vec![entry])),
            }
        }
        let count = |group: Group| plan.entries.iter().filter(|e| e.group == group).count();
        html! {
            <div class="removal-box__list" id={format!("removal-list-{}", node.index())}>
                { for groups.into_iter().map(|(group, entries)| html! {
                    <div class="removal-box__group">
                        <div class="removal-box__title">
                            {format!("{} ({})", group.title(plan.folder), count(group))}
                        </div>
                        <ul>
                            { for entries.into_iter().map(|entry| {
                                let target = entry.node;
                                let go = ctx
                                    .link()
                                    .callback(move |_: MouseEvent| BoxMessage::Go(target));
                                html! {
                                    <li class="removal-box__entry" key={target.index()}>
                                        <button
                                            class="removal-box__name"
                                            type="button"
                                            title="Show in the signature"
                                            onclick={go}
                                        >
                                            <TexSpan
                                                class="removal-box__tex"
                                                error_color="#c004"
                                                raw_tex={seal(&entry.name)}
                                            />
                                        </button>
                                        <span class="removal-box__why">
                                            {entry.how}
                                            {" "}
                                            if let Some(of) = &entry.of {
                                                <TexSpan
                                                    class="removal-box__tex"
                                                    error_color="#c004"
                                                    raw_tex={seal(of)}
                                                />
                                            }
                                        </span>
                                    </li>
                                }
                            }) }
                        </ul>
                    </div>
                }) }
                if more > 0 {
                    <div class="removal-box__more">{format!("and {more} more")}</div>
                }
            </div>
        }
    }
}

/// Shows the row of `target` in the signature: opens the folders and the instances it is
/// under, scrolls to it and marks it for a moment. `from` is the node being deleted.
fn go_to(signature: &Signature, dispatch: &Callback<Action>, from: Node, target: Node) {
    // The signature may move under the pointer, and the box with it.
    set_hold(|h| Hold { pinned: true, ..h });
    let token = SESSION.with(|s| {
        s.borrow()
            .as_ref()
            .filter(|s| s.node == from)
            .map(|s| s.token)
    });
    let opening = closed_above(signature, target);
    let opened = !opening.is_empty();
    for edit in opening {
        dispatch.emit(proof::Action::EditSignature(edit).into());
    }
    wasm_bindgen_futures::spawn_local(async move {
        // (The rows come when the signature has been drawn again.)
        for _ in 0..40 {
            if let Some(row) = row_of(target) {
                if let Some(token) = token.filter(|_| opened) {
                    refresh(token);
                }
                reveal(&row);
                return;
            }
            TimeoutFuture::new(25).await;
        }
    });
}

/// Scrolls whatever scrolls so that the element is on show, moving no more than that.
fn scroll_nearest(element: &Element) {
    let options = js_sys::Object::new();
    js_sys::Reflect::set(&options, &"block".into(), &"nearest".into()).ok();
    if let Ok(scroll) = js_sys::Reflect::get(element, &"scrollIntoView".into()) {
        if let Some(scroll) = scroll.dyn_ref::<js_sys::Function>() {
            scroll.call1(element, &options).ok();
        }
    }
}

fn reveal(row: &Element) {
    scroll_nearest(row);
    // Again, so that it can be seen again.
    row.remove_attribute("data-removal-flash").ok();
    let _ = row.get_bounding_client_rect();
    row.set_attribute("data-removal-flash", "").ok();
    let row = row.clone();
    wasm_bindgen_futures::spawn_local(async move {
        TimeoutFuture::new(FLASH_MS).await;
        row.remove_attribute("data-removal-flash").ok();
    });
}

#[derive(Properties, Debug, Clone, PartialEq)]
pub struct DeleteButtonProps {
    pub dispatch: Callback<Action>,
    pub signature: Signature,
    pub node: Node,
    #[prop_or_default]
    pub light: bool,
}

/// The button that deletes a generator or a folder, and shows what that would delete while the
/// pointer is over it or it has the focus.
#[function_component]
pub fn DeleteButton(props: &DeleteButtonProps) -> Html {
    let on_show = {
        let (signature, node) = (props.signature.clone(), props.node);
        Callback::from(move |_: FocusEvent| {
            if !QUIET_FOCUS.with(|q| q.replace(false)) {
                show(&signature, node, Via::Focus);
            }
        })
    };
    let on_hover = {
        let (signature, node) = (props.signature.clone(), props.node);
        Callback::from(move |_: MouseEvent| show(&signature, node, Via::Pointer))
    };
    let delete = {
        let (dispatch, node) = (props.dispatch.clone(), props.node);
        move || {
            // (The button may go with the row, and then it never sees the pointer leave.)
            hide();
            dispatch.emit(proof::Action::EditSignature(SignatureEdit::Remove(node)).into());
        }
    };
    let on_click = {
        let delete = delete.clone();
        Callback::from(move |_: MouseEvent| delete())
    };
    let on_key = {
        let node = props.node;
        Callback::from(move |e: KeyboardEvent| match e.key().as_str() {
            "Enter" | " " => {
                e.prevent_default();
                delete();
            }
            // Into the box, which is not next to the button in the tab order.
            "Tab" if !e.shift_key() => {
                if let Some(toggle) =
                    by_id(&toggle_id(node)).and_then(|t| t.dyn_into::<HtmlElement>().ok())
                {
                    e.prevent_default();
                    toggle.focus().ok();
                }
            }
            _ => {}
        })
    };

    html! {
        <span
            class="signature__item-child signature__removal-button"
            id={button_id(props.node)}
            role="button"
            tabindex="0"
            aria-label="Delete"
            aria-describedby={note_id(props.node)}
            onclick={on_click}
            onkeydown={on_key}
            onmouseenter={on_hover}
            onfocus={on_show}
        >
            <Icon name={"delete"} size={IconSize::Icon18} light={props.light} />
        </span>
    }
}

#[cfg(test)]
mod tests {
    use homotopy_core::{signature::Invertibility, Generator};

    use super::*;
    use crate::model::proof::{SignatureItem, SignatureItemEdit};

    fn cell(
        sig: &mut Signature,
        source: homotopy_core::Diagram,
        target: homotopy_core::Diagram,
    ) -> Generator {
        sig.create_generator(source, target, "g", Invertibility::Directed)
            .unwrap()
            .max_generator()
            .generator
    }

    /// `x`, `Q`, a placeholder `P`, `u : Q => Q` that depends on `P`, `v : u => u`, `z : Q => Q`,
    /// with `u` and `v` in a folder.
    pub(super) fn setup() -> (Signature, [Generator; 5], Node) {
        let mut sig = Signature::default();
        let x = sig.create_generator_zero("x");
        let q = cell(&mut sig, x.into(), x.into());
        let p = cell(&mut sig, x.into(), x.into());
        let qd = sig.generator_info(q).unwrap().diagram.clone();
        let u = cell(&mut sig, qd.clone(), qd.clone());
        let ud = sig.generator_info(u).unwrap().diagram.clone();
        let v = cell(&mut sig, ud.clone(), ud);
        let z = cell(&mut sig, qd.clone(), qd);
        let edit = |sig: &mut Signature, g, edit| {
            let node = sig.find_node(g).unwrap();
            sig.update(&SignatureEdit::Edit(node, edit)).unwrap();
        };
        edit(&mut sig, p, SignatureItemEdit::MakePlaceholder(true));
        edit(&mut sig, u, SignatureItemEdit::SetDependsOn(vec![p]));
        let root = sig.as_tree().root();
        sig.update(&SignatureEdit::NewFolder(root)).unwrap();
        let folder = sig
            .as_tree()
            .iter()
            .find(|(n, d)| *n != root && matches!(d.inner(), SignatureItem::Folder(_)))
            .map(|(n, _)| n)
            .unwrap();
        for g in [u, v] {
            let node = sig.find_node(g).unwrap();
            sig.update(&SignatureEdit::MoveInto(node, folder)).unwrap();
        }
        (sig, [p, q, u, v, z], folder)
    }

    #[test]
    fn a_closed_folder_holds_what_goes_out_of_sight() {
        let (sig, [p, _, u, v, z], folder) = setup();
        let pressed = sig.find_node(p).unwrap();
        let removed = sig.removal_reasons(pressed);
        let (nu, nv, nz) = (
            sig.find_node(u).unwrap(),
            sig.find_node(v).unwrap(),
            sig.find_node(z).unwrap(),
        );
        // The folder is closed: no rows for what is in it.
        let plan = plan(&sig, pressed, &removed, |n| n != nu && n != nv);
        assert_eq!(plan.more, 2);
        assert_eq!(plan.hidden, 2);
        assert_eq!(plan.inside, vec![(folder, 2)]);
        assert!(plan.going.is_empty());
        assert!(!plan.going.contains(&nz));
        assert_eq!(plan.text(), "Deletes 2 more (2 out of sight)");

        // Open: both rows are on show, and tinted.
        let plan = super::plan(&sig, pressed, &removed, |_| true);
        assert_eq!(plan.hidden, 0);
        assert!(plan.inside.is_empty());
        assert_eq!(plan.going.len(), 2);
        assert_eq!(plan.text(), "Deletes 2 more");
    }

    #[test]
    fn something_nothing_else_depends_on_says_so() {
        let (sig, [_, _, _, _, z], _) = setup();
        let pressed = sig.find_node(z).unwrap();
        let removed = sig.removal_reasons(pressed);
        let plan = plan(&sig, pressed, &removed, |_| true);
        assert_eq!(plan.text(), "Nothing else depends on it");
        assert!(plan.going.is_empty() && plan.entries.is_empty());
    }

    #[test]
    fn a_folder_counts_its_contents_apart_from_what_goes_outside() {
        let (sig, [p, _, _, _, _], folder) = setup();
        // Deleting the folder deletes `u` and `v` in it, and nothing outside depends on them.
        let removed = sig.removal_reasons(folder);
        let plan = plan(&sig, folder, &removed, |_| true);
        assert_eq!((plan.contained, plan.more), (2, 0));
        assert_eq!(plan.text(), "Deletes 2 generators inside");
        assert!(sig.generator_info(p).is_some());
    }
}

#[cfg(test)]
mod hold_tests {
    use super::*;

    #[test]
    fn the_distance_to_a_rectangle_is_zero_inside_and_straight_or_diagonal_outside() {
        let r = Rect {
            left: 10.0,
            top: 10.0,
            right: 110.0,
            bottom: 50.0,
        };
        assert!(r.distance(50.0, 30.0).abs() < f64::EPSILON);
        assert!((r.distance(0.0, 30.0) - 10.0).abs() < 1e-9);
        assert!((r.distance(50.0, 90.0) - 40.0).abs() < 1e-9);
        assert!((r.distance(7.0, 6.0) - 5.0).abs() < 1e-9);
        let both = r.union(Rect {
            left: 90.0,
            top: 60.0,
            right: 130.0,
            bottom: 80.0,
        });
        // The space between the two is inside the union.
        assert!(both.distance(20.0, 70.0).abs() < f64::EPSILON);
    }

    #[test]
    fn it_is_held_by_the_pointer_the_focus_or_a_pin_and_a_pin_goes_when_the_pointer_moves() {
        let none = Hold::default();
        assert!(!none.held());
        assert!(none.pointer_is(true).held());
        assert!(Hold {
            focus: true,
            ..none
        }
        .held());
        let pinned = Hold {
            pointer: true,
            pinned: true,
            ..none
        };
        // The page moved under the pointer: still held, though the pointer is not near.
        assert!(Hold {
            pointer: false,
            ..pinned
        }
        .held());
        // Once the pointer moves, where it is decides.
        assert!(pinned.pointer_is(true).held() && !pinned.pointer_is(true).pinned);
        assert!(!pinned.pointer_is(false).held());
        // And the focus holds it however the pointer moves.
        let focused = Hold {
            focus: true,
            ..pinned
        };
        assert!(focused.pointer_is(false).held());
    }

    #[test]
    fn each_generator_listed_has_its_group_and_reason() {
        let (sig, [p, _, u, v, _], folder) = super::tests::setup();
        let pressed = sig.find_node(p).unwrap();
        let plan = plan(&sig, pressed, &sig.removal_reasons(pressed), |_| true);
        let how = |g| {
            let node = sig.find_node(g).unwrap();
            let e = plan.entries.iter().find(|e| e.node == node).unwrap();
            (e.group, e.how, e.of.is_some())
        };
        assert_eq!(how(u), (Group::Dependents, "depends on placeholder", true));
        assert_eq!(how(v), (Group::Dependents, "mentions", true));
        assert_eq!(plan.entries.len(), 2);
        let (shown, more) = plan.listed();
        assert_eq!((shown.len(), more), (2, 0));

        // The folder: what is in it is "in the folder", with nothing else to say.
        let removed = sig.removal_reasons(folder);
        let plan = super::plan(&sig, folder, &removed, |_| true);
        assert!(plan
            .entries
            .iter()
            .all(|e| e.group == Group::Contents && e.how == "in the folder" && e.of.is_none()));
        assert_eq!(plan.entries.len(), 2);
    }

    #[test]
    fn the_names_that_have_to_be_opened_to_see_a_row_are_listed_outermost_first() {
        let (mut sig, [_, _, _, v, _], folder) = super::tests::setup();
        let target = sig.find_node(v).unwrap();
        // The folder is open when it is made; shut it.
        sig.update(&SignatureEdit::ToggleFolder(folder)).unwrap();
        assert!(matches!(
            closed_above(&sig, target)[..],
            [SignatureEdit::ToggleFolder(f)] if f == folder
        ));
        // Open, and nothing needs opening.
        sig.update(&SignatureEdit::ToggleFolder(folder)).unwrap();
        assert!(closed_above(&sig, target).is_empty());
    }
}
