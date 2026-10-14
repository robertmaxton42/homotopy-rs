//! Hooks for driving and inspecting the app from the browser's console or an automated test, in
//! debug builds only: `window.__homotopy`.
//!
//! - `dispatch(json)` does an action, given as JSON (see [`Command`]); `importActions(bytes)`
//!   and `importProof(bytes)` do the same with a file's contents.
//! - `state()` is a JSON summary of the project and the job in progress (see [`Summary`]).
//! - `layouts()` is a JSON summary of the layouts of diagrams that were made and where (see
//!   `layout::summary`); `window.__homotopy_layout_delay_ms` makes each layout in the worker take
//!   at least that long, and `window.__homotopy_layout_worker` does for the layout worker what
//!   `__homotopy_worker` does for the other (`"off"`, `"broken"`, `"crash"`, `"garble"`);
//!   `window.__homotopy_layout_weight` sets the weight up to which diagrams are laid out on the
//!   page (0 sends all of them to the worker).
//! - `fillWithGenerator("[schema, placeholder, generator]")` chooses the diagram of a generator
//!   for a placeholder in the Instantiate panel of a schema (each as `[id, dimension]`).
//! - `profile(json)` replays recorded actions and times each kind (in milliseconds).
//! - `window.__homotopy_step_delay_ms`, if set, makes every step of a job take at least that long,
//!   to make jobs slow enough to test with (also those done in the worker, which is told when
//!   it is given the job).
//! - `crash("panic")` panics in the app and `crash("throw")` calls JavaScript that throws and
//!   nothing catches, to test the crash screen and recovering the project (see `panic`).
//! - `window.__homotopy_worker` can be set to `"off"` before the app starts to do without the
//!   worker, and to `"crash"` to have the first job given to it panic there, to test falling back.

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
    busy: Option<String>,
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
    static STATE: RefCell<(model::State, Option<String>)> = RefCell::default();
}

/// Records what is going on, for `state()`.
pub fn publish(state: &model::State, busy: Option<String>) {
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

/// How long `window.__homotopy_step_delay_ms` says a step is to take, if it is set (in
/// milliseconds). This is passed to the worker with each job, as it has no such window.
pub fn step_delay_ms() -> u32 {
    web_sys::window()
        .and_then(|window| js_sys::Reflect::get(&window, &"__homotopy_step_delay_ms".into()).ok())
        .and_then(|delay| delay.as_f64())
        .map_or(0, |delay| delay as u32)
}

/// What `window.__homotopy_worker` says about the worker, if anything: `"off"` for none, `"broken"`
/// for one whose script cannot be loaded, `"crash"` for one that panics in each job, `"garble"`
/// for one that answers with nonsense.
pub fn worker_mode() -> String {
    web_sys::window()
        .and_then(|window| js_sys::Reflect::get(&window, &"__homotopy_worker".into()).ok())
        .and_then(|mode| mode.as_string())
        .unwrap_or_default()
}

/// What `window.__homotopy_layout_worker` says about the worker that lays diagrams out, if
/// anything: the same as `__homotopy_worker` says about the other (`"off"`, `"broken"`,
/// `"crash"`, `"garble"`).
pub fn layout_worker_mode() -> String {
    web_sys::window()
        .and_then(|window| js_sys::Reflect::get(&window, &"__homotopy_layout_worker".into()).ok())
        .and_then(|mode| mode.as_string())
        .unwrap_or_default()
}

/// What `window.__homotopy_layout_weight` says is the weight up to which diagrams are laid out on
/// the page, if it is set (0 sends everything to the worker).
pub fn layout_weight() -> Option<usize> {
    web_sys::window()
        .and_then(|window| js_sys::Reflect::get(&window, &"__homotopy_layout_weight".into()).ok())
        .and_then(|weight| weight.as_f64())
        .map(|weight| weight as usize)
}

/// How long `window.__homotopy_layout_delay_ms` says a layout in the worker is to take at least,
/// if it is set (in milliseconds).
pub fn layout_delay_ms() -> u32 {
    web_sys::window()
        .and_then(|window| js_sys::Reflect::get(&window, &"__homotopy_layout_delay_ms".into()).ok())
        .and_then(|delay| delay.as_f64())
        .map_or(0, |delay| delay as u32)
}

/// Makes a step take as long as `window.__homotopy_step_delay_ms` says, if it is set.
pub fn artificial_delay() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let delay = f64::from(step_delay_ms());
    if delay == 0.0 {
        return;
    }
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

/// Times laying out the diagrams of the project (those of the generators, the workspace's and
/// their slices) in the dimensions of the views that can show them, next to their weight (see
/// `homotopy_core::layout::weight`), to choose the weight below which this is done on the page.
fn profile_layouts(state: &model::State) -> String {
    use homotopy_core::{layout::Layout, Diagram, DiagramN};

    let performance = web_sys::window().unwrap().performance().unwrap();
    let proof = state.proof();
    let mut diagrams: Vec<Diagram> = proof.signature.iter().map(|g| g.diagram.clone()).collect();
    if let Some(workspace) = &proof.workspace {
        diagrams.push(workspace.diagram.clone());
    }
    let mut slices = vec![];
    for diagram in &diagrams {
        if let Ok(diagram) = DiagramN::try_from(diagram.clone()) {
            slices.extend(diagram.slices());
        }
    }
    diagrams.extend(slices);
    let mut timings = vec![];
    for diagram in diagrams {
        let weight = homotopy_core::layout::weight(&diagram, usize::MAX);
        let mut time = |n: usize, run: &dyn Fn()| {
            if diagram.dimension() >= n {
                let start = performance.now();
                run();
                timings.push(serde_json::json!({
                    "n": n, "weight": weight, "dimension": diagram.dimension(),
                    "ms": performance.now() - start,
                }));
            }
        };
        time(1, &|| drop(Layout::<1>::new(&diagram)));
        time(2, &|| drop(Layout::<2>::new(&diagram)));
        time(3, &|| drop(Layout::<3>::new(&diagram)));
        // The geometry of the 3D view, which includes the layout.
        if diagram.dimension() >= 3 {
            let start = performance.now();
            drop(homotopy_graphics::geom::CubicalGeometry::new::<3>(
                &diagram, false,
            ));
            timings.push(serde_json::json!({
                "n": 33, "weight": weight, "dimension": diagram.dimension(),
                "ms": performance.now() - start,
            }));
        }
    }
    serde_json::to_string(&timings).unwrap()
}

/// Times what saving the project in the background takes on the page: serialising it whole,
/// and, for doing that in the worker, building the request that gives it the project (with the
/// signature, and again without) and copying the bytes that come back.
fn profile_autosave(state: &model::State) -> String {
    use homotopy_model::remote::{Remote, Work};

    let performance = web_sys::window().unwrap().performance().unwrap();
    let proof = state.proof();
    let timed = |name: &str, bytes: &mut usize, run: &mut dyn FnMut() -> usize| {
        let start = performance.now();
        *bytes = run();
        serde_json::json!({ "what": name, "ms": performance.now() - start, "bytes": *bytes })
    };
    let (mut bytes, mut out) = (0, vec![]);
    let work = || Work::AttachOptions {
        point: vec![],
        weak_units: false,
    };
    out.push(timed("serialize", &mut bytes, &mut || {
        serialize::serialize(
            proof.signature.clone(),
            proof.workspace.clone(),
            proof.metadata.clone(),
        )
        .len()
    }));
    let mut remote = Remote::default();
    out.push(timed("request with signature", &mut bytes, &mut || {
        remote.request(proof, work(), 0, false).len()
    }));
    out.push(timed(
        "request with signature held",
        &mut bytes,
        &mut || remote.request(proof, work(), 0, false).len(),
    ));
    let serialized = serialize::serialize(
        proof.signature.clone(),
        proof.workspace.clone(),
        proof.metadata.clone(),
    );
    out.push(timed("copy to a Uint8Array", &mut bytes, &mut || {
        js_sys::Uint8Array::from(serialized.as_slice()).length() as usize
    }));
    serde_json::to_string(&out).unwrap()
}

#[wasm_bindgen::prelude::wasm_bindgen(
    inline_js = "export function throw_from_js() { throw new Error('thrown by the debug hooks'); }"
)]
extern "C" {
    /// An import that throws and is not `catch`: what it does to the app is what a binding that
    /// was not meant to throw does when it does.
    fn throw_from_js();
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

    // `fillFromInstance(schema, placeholder, instance, dimension)`, each generator as `[id,
    // dimension]` and the dimension of the replacement to take: chooses for the placeholder the
    // replacement of that dimension that the instance was made with, as clicking *Use workspace*
    // with that diagram in the workspace does (there is no way to put such a diagram, a cell
    // whiskered by another, in the workspace without a lot of clicking).
    let fill = link.clone();
    set(
        "fillFromInstance",
        Closure::<dyn Fn(String) -> Result<(), JsValue>>::new(move |json: String| {
            use homotopy_core::{signature::Signature as _, Generator};
            let [schema, placeholder, instance, dimension]: [serde_json::Value; 4] =
                serde_json::from_str(&json).map_err(|e| JsValue::from_str(&e.to_string()))?;
            let generator = |v: &serde_json::Value| {
                Generator::new(
                    v[0].as_u64().unwrap_or_default() as usize,
                    v[1].as_u64().unwrap_or_default() as usize,
                )
            };
            let (schema, placeholder, instance) = (
                generator(&schema),
                generator(&placeholder),
                generator(&instance),
            );
            let dimension = dimension.as_u64().unwrap_or_default() as usize;
            let diagram = STATE.with(|state| {
                let state = state.borrow();
                let info = state.0.proof().signature.generator_info(instance)?;
                let made = info.instance.as_ref()?;
                made.substitution
                    .iter()
                    .find(|(p, _)| p.dimension == dimension)
                    .map(|(_, d)| d.clone())
            });
            let diagram = diagram.ok_or_else(|| JsValue::from_str("no such replacement"))?;
            fill.send_message(Message::Dispatch(model::Action::Draft(
                proof::DraftEdit::Fill(schema, placeholder, diagram),
            )));
            Ok(())
        })
        .into_js_value(),
    );

    // `fillWithGenerator(schema, placeholder, generator)`, each as `[id, dimension]`: chooses for
    // the placeholder the diagram of a generator of the signature, as *Use workspace* does with
    // that cell in the workspace.
    let fill = link.clone();
    set(
        "fillWithGenerator",
        Closure::<dyn Fn(String) -> Result<(), JsValue>>::new(move |json: String| {
            use homotopy_core::{signature::Signature as _, Generator};
            let [schema, placeholder, cell]: [[usize; 2]; 3] =
                serde_json::from_str(&json).map_err(|e| JsValue::from_str(&e.to_string()))?;
            let [schema, placeholder, cell] =
                [schema, placeholder, cell].map(|[id, dimension]| Generator::new(id, dimension));
            let diagram = STATE.with(|state| {
                let state = state.borrow();
                let info = state.0.proof().signature.generator_info(cell)?;
                Some(info.diagram.clone())
            });
            let diagram = diagram.ok_or_else(|| JsValue::from_str("no such generator"))?;
            fill.send_message(Message::Dispatch(model::Action::Draft(
                proof::DraftEdit::Fill(schema, placeholder, diagram),
            )));
            Ok(())
        })
        .into_js_value(),
    );

    set(
        "crash",
        Closure::<dyn Fn(String)>::new(|kind: String| match kind.as_str() {
            "throw" => throw_from_js(),
            _ => panic!("crash(\"panic\") from the debug hooks"),
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
        "layouts",
        Closure::<dyn Fn() -> String>::new(crate::layout::summary).into_js_value(),
    );

    set(
        "profileLayouts",
        Closure::<dyn Fn() -> String>::new(|| {
            STATE.with(|state| profile_layouts(&state.borrow().0))
        })
        .into_js_value(),
    );

    set(
        "profileAutosave",
        Closure::<dyn Fn() -> String>::new(|| {
            STATE.with(|state| profile_autosave(&state.borrow().0))
        })
        .into_js_value(),
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
