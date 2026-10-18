//! Showing what deleting a generator (or a folder) would delete, while the pointer is on its
//! delete button or the button has the focus.
//!
//! Deleting a generator deletes the generators that depend on it and so on (see
//! [`Signature::removal_nodes`]), which can be far from it in the signature, in a closed folder
//! or among the instances of a schema. The preview is made from the same code as the deletion,
//! so it cannot disagree with it.
//!
//! Like the tooltip and the busy indicator, it is told by calls ([`show`], [`hide`]) and not by
//! properties, and it writes to the elements directly, so that hovering a button does not render
//! the signature: rows that would go get `data-removal` (the stylesheet tints them and strikes
//! out their names), the rows that hold some that would go without being shown (a closed folder,
//! a schema whose instances are closed) get `data-removal-inside="N"`, and a note next to the
//! button says how many. Finding out which generators the diagrams mention is what costs, and is
//! done a few diagrams at a time (`Signature::removal_preview`), yielding between, on a signature
//! that has not been looked at before: the note says it is checking until it is done.

use std::cell::{Cell, RefCell};

use gloo_timers::future::TimeoutFuture;
use homotopy_common::{
    hash::{FastHashMap, FastHashSet},
    idx::Idx,
    tree::Node,
};
use homotopy_core::signature::Signature as _;
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement};
use yew::prelude::*;

use crate::{
    components::icon::{Icon, IconSize},
    model::{
        proof::{self, Signature, SignatureEdit},
        Action,
    },
};

/// How many diagrams that have not been looked at before one step of the preview looks at.
const DIAGRAMS_PER_STEP: usize = 16;
/// How many names the note lists in its tooltip.
const MOST_NAMES: usize = 12;

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
    /// The names of the generators that go and are not under the one deleted.
    pub names: Vec<String>,
    /// Whether the node deleted is a folder.
    pub folder: bool,
}

/// What deleting `pressed` deletes (`removed`: [`Signature::removal_nodes`]), given which nodes
/// have a row on show. Pure, and the same wherever the rows are.
#[must_use]
pub fn plan(
    signature: &Signature,
    pressed: Node,
    removed: &[Node],
    has_row: impl Fn(Node) -> bool,
) -> Plan {
    let tree = signature.as_tree();
    let under: FastHashSet<Node> = tree.descendents_of(pressed).collect();
    let going: FastHashSet<Node> = removed.iter().copied().collect();
    let mut plan = Plan {
        folder: signature.find_generator(pressed).is_none(),
        ..Plan::default()
    };
    let mut inside: FastHashMap<Node, usize> = FastHashMap::default();
    let mut order: Vec<Node> = vec![];
    for &node in removed {
        if node == pressed {
            continue;
        }
        let item = signature
            .find_generator(node)
            .and_then(|g| signature.generator_info(g));
        let is_under = under.contains(&node);
        if let Some(info) = item {
            if is_under {
                plan.contained += 1;
            } else {
                plan.more += 1;
                plan.names.push(info.name.replace('$', ""));
            }
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
}

thread_local! {
    /// Which showing is the current one, so that a step of an earlier one does nothing.
    static TOKEN: Cell<u64> = const { Cell::new(0) };
    /// The note of the showing.
    static NOTE: RefCell<Option<HtmlElement>> = const { RefCell::new(None) };
}

fn document() -> Option<web_sys::Document> {
    web_sys::window()?.document()
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

fn set_note(text: &str, names: &[String]) {
    NOTE.with(|note| {
        if let Some(note) = note.borrow().as_ref() {
            note.set_text_content(Some(text));
            if names.is_empty() {
                note.remove_attribute("title").ok();
            } else {
                let mut title = names
                    .iter()
                    .take(MOST_NAMES)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                if names.len() > MOST_NAMES {
                    title = format!("{title}, and {} more", names.len() - MOST_NAMES);
                }
                note.set_attribute("title", &title).ok();
            }
        }
    });
}

/// Marks the rows on show.
fn apply(signature: &Signature, pressed: Node, removed: &[Node]) {
    let Some(document) = document() else {
        return;
    };
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
    set_note(&plan.text(), &plan.names);
}

/// Starts showing what deleting `node` deletes. `note` is the element that says how much.
pub fn show(signature: &Signature, node: Node, note: &NodeRef) {
    let token = TOKEN.with(|t| {
        t.set(t.get() + 1);
        t.get()
    });
    clear_marks();
    NOTE.with(|n| *n.borrow_mut() = note.cast::<HtmlElement>());
    step(signature.clone(), node, token);
}

fn step(signature: Signature, node: Node, token: u64) {
    if TOKEN.with(Cell::get) != token {
        return;
    }
    if let Some(removed) = signature.removal_preview(node, DIAGRAMS_PER_STEP) {
        apply(&signature, node, &removed);
    } else {
        set_note("Checking what depends on it\u{2026}", &[]);
        wasm_bindgen_futures::spawn_local(async move {
            TimeoutFuture::new(0).await;
            step(signature, node, token);
        });
    }
}

/// Stops showing it.
pub fn hide() {
    TOKEN.with(|t| t.set(t.get() + 1));
    clear_marks();
    set_note("", &[]);
    NOTE.with(|n| *n.borrow_mut() = None);
}

#[derive(Properties, Debug, Clone, PartialEq)]
pub struct DeleteButtonProps {
    pub dispatch: Callback<Action>,
    pub signature: Signature,
    pub node: Node,
    /// The element that says what is deleted, which is also the description of the button.
    pub note: NodeRef,
    pub note_id: String,
    #[prop_or_default]
    pub light: bool,
}

/// The button that deletes a generator or a folder, and shows what that would delete while the
/// pointer is over it or it has the focus.
#[function_component]
pub fn DeleteButton(props: &DeleteButtonProps) -> Html {
    let on_show = {
        let (signature, node, note) = (props.signature.clone(), props.node, props.note.clone());
        Callback::from(move |_: FocusEvent| show(&signature, node, &note))
    };
    let on_hover = {
        let (signature, node, note) = (props.signature.clone(), props.node, props.note.clone());
        Callback::from(move |_: MouseEvent| show(&signature, node, &note))
    };
    let on_hide = Callback::from(|_: FocusEvent| hide());
    let on_leave = Callback::from(|_: MouseEvent| hide());
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
    let on_key = Callback::from(move |e: KeyboardEvent| {
        if matches!(e.key().as_str(), "Enter" | " ") {
            e.prevent_default();
            delete();
        }
    });

    html! {
        <span
            class="signature__item-child signature__removal-button"
            role="button"
            tabindex="0"
            aria-label="Delete"
            aria-describedby={props.note_id.clone()}
            onclick={on_click}
            onkeydown={on_key}
            onmouseenter={on_hover}
            onmouseleave={on_leave}
            onfocus={on_show}
            onblur={on_hide}
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
    fn setup() -> (Signature, [Generator; 5], Node) {
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
        let removed = sig.removal_nodes(pressed);
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
        let removed = sig.removal_nodes(pressed);
        let plan = plan(&sig, pressed, &removed, |_| true);
        assert_eq!(plan.text(), "Nothing else depends on it");
        assert!(plan.going.is_empty() && plan.names.is_empty());
    }

    #[test]
    fn a_folder_counts_its_contents_apart_from_what_goes_outside() {
        let (sig, [p, _, _, _, _], folder) = setup();
        // Deleting the folder deletes `u` and `v` in it, and nothing outside depends on them.
        let removed = sig.removal_nodes(folder);
        let plan = plan(&sig, folder, &removed, |_| true);
        assert_eq!((plan.contained, plan.more), (2, 0));
        assert_eq!(plan.text(), "Deletes 2 generators inside");
        assert!(sig.generator_info(p).is_some());
    }
}
