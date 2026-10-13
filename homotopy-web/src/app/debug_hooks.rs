//! Hooks for driving and inspecting the app from the browser's console or an automated test, in
//! debug builds only: `window.__homotopy`.
//!
//! - `dispatch(json)` does an action, given as JSON (see [`Command`]); `importActions(bytes)`
//!   and `importProof(bytes)` do the same with a file's contents.
//! - `state()` is a JSON summary of the project and the job in progress (see [`Summary`]).
//! - `profile(json)` replays recorded actions and times each kind (in milliseconds).
//! - `window.__homotopy_step_delay_ms`, if set, makes every step of a job take at least that long,
//!   to make jobs slow enough to test with.

use std::cell::RefCell;

use homotopy_core::SliceIndex;
use serde::{Deserialize, Serialize};
use wasm_bindgen::{closure::Closure, JsValue};
use yew::html::Scope;

use super::{App, Message};
use crate::model::{self, history::UndoState, proof, serialize};

/// An action, as JSON.
#[derive(Deserialize)]
enum Command {
    Proof(proof::Action),
    SelectPoint(Vec<SliceIndex>, bool),
    Select(usize),
    ClearSelections,
    Undo,
    Redo,
    /// Export the workspace diagram as TikZ, as one file or one for each slice.
    ExportTikz(bool),
}

impl From<Command> for model::Action {
    fn from(command: Command) -> Self {
        use homotopy_core::Direction::{Backward, Forward};
        let linear = |direction| {
            Self::History(model::history::Action::Move(
                model::history::Direction::Linear(direction),
            ))
        };
        match command {
            Command::Proof(action) => Self::Proof(action),
            Command::SelectPoint(point, weak_units) => Self::SelectPoint(point, weak_units),
            Command::Select(index) => Self::Select(index),
            Command::ClearSelections => Self::ClearSelections,
            Command::Undo => linear(Backward),
            Command::Redo => linear(Forward),
            Command::ExportTikz(multiple) => Self::ExportImage(
                model::ImageFormat::Tikz(homotopy_graphics::tikz::TikzOptions {
                    left_to_right: false,
                    show_braidings: false,
                }),
                if multiple {
                    model::ImageOption::Multiple
                } else {
                    model::ImageOption::Single
                },
            ),
        }
    }
}

/// What is going on, for a test to compare before and after.
#[derive(Serialize)]
struct Summary {
    busy: Option<&'static str>,
    can_undo: bool,
    can_redo: bool,
    /// The number of actions in the history up to the current proof.
    actions: usize,
    /// A hash of the serialized project, and its size.
    project_hash: u64,
    project_bytes: usize,
    generators: usize,
    options: Option<usize>,
    /// The crash dump's action log: its number of actions, and the size of the import it starts
    /// from.
    log: (usize, usize),
}

thread_local! {
    static STATE: RefCell<(model::State, Option<&'static str>)> = RefCell::default();
}

/// Records what is going on, for `state()`.
pub fn publish(state: &model::State, busy: Option<&'static str>) {
    STATE.with(|s| *s.borrow_mut() = (state.clone(), busy));
}

fn summary() -> String {
    let (state, busy) = STATE.with(|s| s.borrow().clone());
    let proof = state.proof();
    let bytes = serialize::serialize(
        proof.signature.clone(),
        proof.workspace.clone(),
        proof.metadata.clone(),
    );
    // FNV-1a
    let project_hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    });
    let summary = Summary {
        busy,
        can_undo: proof.can_undo(),
        can_redo: proof.can_redo(),
        actions: state.history.get_actions().len(),
        project_hash,
        project_bytes: bytes.len(),
        generators: proof.signature.iter().count(),
        options: state.options.as_ref().map(|options| match options {
            model::Selectables::Merge(_, tos) => tos.len(),
            model::Selectables::Attach(options) => options.len(),
        }),
        log: crate::panic::debug_summary(),
    };
    serde_json::to_string(&summary).unwrap()
}

/// Makes a step take as long as `window.__homotopy_step_delay_ms` says, if it is set.
pub fn artificial_delay() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(delay) = js_sys::Reflect::get(&window, &"__homotopy_step_delay_ms".into())
        .ok()
        .and_then(|delay| delay.as_f64())
    else {
        return;
    };
    let Some(performance) = window.performance() else {
        return;
    };
    let until = performance.now() + delay;
    #[allow(clippy::while_float)] // Spinning is the point.
    while performance.now() < until {}
}

/// Replays recorded actions and times each kind of them.
fn profile(json: &str) -> Result<JsValue, JsValue> {
    let (_safe, actions): (bool, Vec<proof::Action>) =
        serde_json::from_str(json).map_err(|e| JsValue::from_str(&e.to_string()))?;
    let performance = web_sys::window().unwrap().performance().unwrap();
    let mut proof = model::Proof::default();
    let mut timings = vec![];
    for action in &actions {
        let kind = match serde_json::to_value(action).unwrap() {
            serde_json::Value::String(kind) => kind,
            serde_json::Value::Object(map) => map.keys().next().cloned().unwrap_or_default(),
            _ => String::new(),
        };
        let start = performance.now();
        let result = proof.update(action);
        let ms = performance.now() - start;
        timings.push(serde_json::json!({ "kind": kind, "ms": ms, "ok": result.is_ok() }));
    }
    Ok(JsValue::from_str(&serde_json::to_string(&timings).unwrap()))
}

/// Times choosing each cell of the right dimension as the replacement for each placeholder of
/// each schema in the current signature (see `Drafts::apply`).
fn profile_drafts(state: &model::State) -> String {
    use homotopy_core::signature::Signature as _;

    let performance = web_sys::window().unwrap().performance().unwrap();
    let signature = &state.proof().signature;
    let mut timings = vec![];
    for schema in signature.iter() {
        for placeholder in signature.parameters(schema.generator) {
            let Some(placeholder_info) = signature.generator_info(placeholder) else {
                continue;
            };
            for cell in signature.iter() {
                if cell.diagram.dimension() != placeholder_info.diagram.dimension()
                    || cell.placeholder
                {
                    continue;
                }
                let mut drafts = proof::Drafts::default();
                let start = performance.now();
                drafts.apply(
                    signature,
                    proof::DraftEdit::Fill(schema.generator, placeholder, cell.diagram.clone()),
                );
                timings.push(serde_json::json!({
                    "schema": schema.name,
                    "placeholder": placeholder_info.name,
                    "cell": cell.name,
                    "ms": performance.now() - start,
                }));
            }
        }
    }
    serde_json::to_string(&timings).unwrap()
}

pub fn install(link: &Scope<App>) {
    let window = web_sys::window().unwrap();
    let hooks = js_sys::Object::new();
    let set = |name: &str, function: JsValue| {
        js_sys::Reflect::set(&hooks, &name.into(), &function).unwrap();
    };

    let dispatch = link.clone();
    set(
        "dispatch",
        Closure::<dyn Fn(String) -> Result<(), JsValue>>::new(move |json: String| {
            let command: Command =
                serde_json::from_str(&json).map_err(|e| JsValue::from_str(&e.to_string()))?;
            dispatch.send_message(Message::Dispatch(command.into()));
            Ok(())
        })
        .into_js_value(),
    );

    let import = link.clone();
    set(
        "importActions",
        Closure::<dyn Fn(Vec<u8>)>::new(move |bytes: Vec<u8>| {
            import.send_message(Message::Dispatch(model::Action::ImportActions(
                bytes.into(),
            )));
        })
        .into_js_value(),
    );

    let import = link.clone();
    set(
        "importProof",
        Closure::<dyn Fn(Vec<u8>)>::new(move |bytes: Vec<u8>| {
            import.send_message(Message::Dispatch(model::Action::Proof(
                proof::Action::ImportProof(bytes.into()),
            )));
        })
        .into_js_value(),
    );

    let cancel = link.clone();
    set(
        "cancel",
        Closure::<dyn Fn()>::new(move || cancel.send_message(Message::Cancel)).into_js_value(),
    );

    set(
        "state",
        Closure::<dyn Fn() -> String>::new(summary).into_js_value(),
    );

    set(
        "profileDrafts",
        Closure::<dyn Fn() -> String>::new(|| {
            STATE.with(|state| profile_drafts(&state.borrow().0))
        })
        .into_js_value(),
    );

    set(
        "profile",
        Closure::<dyn Fn(String) -> Result<JsValue, JsValue>>::new(|json: String| profile(&json))
            .into_js_value(),
    );

    js_sys::Reflect::set(&window, &"__homotopy".into(), &hooks).unwrap();
}
