//! Doing a job in another instance of this code: a web worker, or any thread of its own.
//!
//! Diagrams are hash-consed in thread-locals and shared with `Rc`, so nothing of a proof can be
//! handed to another thread: it is turned into bytes ([`Remote::request`]), and the other end
//! ([`Executor`]) rebuilds it, runs the [`Job`] on that, and answers with bytes again
//! ([`Remote::response`]). Whatever carries the bytes is up to the caller (a `postMessage`, a
//! channel), and so is what to do when they never come back: the job works on a copy, so the
//! request can simply be forgotten, and made again to run the job here instead.
//!
//! What crosses is faithful, not merely equal: the signature keeps the numbering of its tree
//! (actions refer to its nodes) and everything else a proof state holds, so a job gives the same
//! outcome as it would here. This is not the file format of [`serialize`](crate::serialize),
//! which tidies the signature and leaves out what is not worth saving.
//!
//! Sending a large signature again and again would cost more than most of the jobs, so the
//! executor remembers the last signature it saw or produced (as the signature with a number),
//! and a request says to use that one when it is still the signature of the proof. Diagrams are
//! sent once each through a [`Store`], whatever the number of proofs that mention them.

pub mod layout;

use homotopy_common::tree::Tree;
use homotopy_core::{
    common::{Boundary, BoundaryPath, SliceIndex},
    serialize::{Key, Store},
    signature::{Invertibility, Signature as _, Symmetry},
    Diagram, DiagramN, Generator,
};
use homotopy_graphics::style::{Color, VertexShape};
use im::Vector;
use instant::Instant;
use serde::{Deserialize, Serialize};

use crate::{
    history::proof_from_state,
    job::{Job, Progress},
    proof::{
        generators::{GeneratorInfo, Instance},
        ActionsJob, ActionsOutcome, AttachOption, AttachOptionsJob, FolderInfo, Metadata,
        ProofError, ProofState, SelectedBoundary, Signature, SignatureItem, View, Workspace,
        REPLAYING,
    },
};

/// What a job is to do, apart from the proof it works on.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Work {
    /// An [`ActionsJob`]: a chain of actions, or recorded ones to replay.
    Actions {
        actions: Vec<crate::proof::Action>,
        replay: bool,
    },
    /// An [`AttachOptionsJob`].
    AttachOptions {
        point: Vec<SliceIndex>,
        weak_units: bool,
    },
}

/// What a job came to.
#[derive(Debug)]
pub enum Output {
    Actions(ActionsOutcome),
    AttachOptions(Vec<AttachOption>),
}

/// What there is to say about the job just before each of its steps, for the executor's caller to
/// pass on: it is only then that anything can be said.
#[derive(Copy, Clone, Debug)]
pub struct Step {
    pub label: &'static str,
    pub progress: Option<Progress>,
    /// For debugging: how long the step is to take at least.
    pub delay_ms: u32,
}

// The wire format.

#[derive(Serialize, Deserialize)]
struct WireSignature {
    tree: Tree<WireItem>,
    symmetries: Vec<Symmetry>,
}

#[derive(Serialize, Deserialize)]
enum WireItem {
    Folder(FolderInfo),
    Item(WireGenerator),
}

#[derive(Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)] // As in `GeneratorInfo`.
struct WireGenerator {
    generator: Generator,
    oriented: bool,
    invertibility: Invertibility,
    single_preview: bool,
    color: Color,
    shape: VertexShape,
    diagram: Key<Diagram>,
    name: String,
    placeholder: bool,
    instance: Option<WireInstance>,
    depends_on: Vec<Generator>,
    instances_open: bool,
}

#[derive(Serialize, Deserialize)]
struct WireInstance {
    schema: Generator,
    substitution: Vec<(Generator, Key<Diagram>)>,
}

#[derive(Serialize, Deserialize)]
struct WireWorkspace {
    view: View,
    diagram: Key<Diagram>,
    path: Vector<SliceIndex>,
}

/// A proof state but for its signature.
#[derive(Serialize, Deserialize)]
struct WireState {
    workspace: Option<WireWorkspace>,
    metadata: Metadata,
    boundary: Option<(Boundary, Key<Diagram>)>,
    stash: Vec<WireWorkspace>,
}

#[derive(Serialize, Deserialize)]
enum SignatureRef {
    /// The signature the executor was last left with, under this number.
    Held(u64),
    /// This signature, to be known under this number.
    Sent(u64, WireSignature),
}

#[derive(Serialize, Deserialize)]
struct Request {
    store: Store,
    signature: SignatureRef,
    state: WireState,
    work: Work,
    /// A number for the signature that the job ends with, if it is not the one it began with.
    fresh: u64,
    step_delay_ms: u32,
}

#[derive(Serialize, Deserialize)]
enum Response {
    Done(Box<Done>),
    /// The executor does not hold the signature the request relied on.
    NeedsSignature,
    Failed(String),
}

#[derive(Serialize, Deserialize)]
struct Done {
    store: Store,
    /// The number of the signature of the proof that the job ended with.
    holds: u64,
    output: WireOutput,
    /// How long the job took where it was done, in milliseconds.
    worked_ms: f64,
}

#[derive(Serialize, Deserialize)]
enum WireOutput {
    Actions {
        steps: Vec<WireStep>,
        error: Option<String>,
    },
    AttachOptions(Vec<WireAttachOption>),
}

/// An [`AttachOption`] but for its diagrams.
#[derive(Serialize, Deserialize)]
struct WireAttachOption {
    generator: Generator,
    boundary_path: Option<BoundaryPath>,
    embedding: Vector<usize>,
    label: Option<String>,
    diagram: Key<Diagram>,
    substitution: Option<Vec<(Generator, Key<Diagram>)>>,
}

#[derive(Serialize, Deserialize)]
struct WireStep {
    /// Which of the actions of the request it was (they do not need to come back).
    position: usize,
    state: WireState,
    /// The signature after the action, if the action changed it.
    signature: Option<WireSignature>,
}

fn pack_signature(store: &mut Store, signature: &Signature) -> WireSignature {
    WireSignature {
        tree: signature.as_tree().map(|item| match item {
            SignatureItem::Folder(info) => WireItem::Folder(info),
            SignatureItem::Item(info) => WireItem::Item(WireGenerator {
                diagram: store.pack_diagram(&info.diagram),
                instance: info.instance.map(|instance| WireInstance {
                    schema: instance.schema,
                    substitution: instance
                        .substitution
                        .iter()
                        .map(|(generator, diagram)| (*generator, store.pack_diagram(diagram)))
                        .collect(),
                }),
                generator: info.generator,
                oriented: info.oriented,
                invertibility: info.invertibility,
                single_preview: info.single_preview,
                color: info.color,
                shape: info.shape,
                name: info.name,
                placeholder: info.placeholder,
                depends_on: info.depends_on,
                instances_open: info.instances_open,
            }),
        }),
        symmetries: signature.symmetries().to_vec(),
    }
}

fn unpack_signature(store: &mut Store, wire: WireSignature) -> Option<Signature> {
    let tree = wire
        .tree
        .map(|item| {
            Some(match item {
                WireItem::Folder(info) => SignatureItem::Folder(info),
                WireItem::Item(info) => SignatureItem::Item(GeneratorInfo {
                    diagram: store.unpack_diagram(info.diagram)?,
                    instance: match info.instance {
                        None => None,
                        Some(instance) => Some(Instance {
                            schema: instance.schema,
                            substitution: instance
                                .substitution
                                .into_iter()
                                .map(|(generator, key)| {
                                    Some((generator, store.unpack_diagram(key)?))
                                })
                                .collect::<Option<_>>()?,
                        }),
                    },
                    generator: info.generator,
                    oriented: info.oriented,
                    invertibility: info.invertibility,
                    single_preview: info.single_preview,
                    color: info.color,
                    shape: info.shape,
                    name: info.name,
                    placeholder: info.placeholder,
                    depends_on: info.depends_on,
                    instances_open: info.instances_open,
                }),
            })
        })
        .transpose()?;
    Some(Signature::from(tree).with_symmetries(wire.symmetries))
}

fn pack_workspace(store: &mut Store, workspace: &Workspace) -> WireWorkspace {
    WireWorkspace {
        view: workspace.view,
        diagram: store.pack_diagram(&workspace.diagram),
        path: workspace.path.clone(),
    }
}

fn unpack_workspace(store: &mut Store, wire: WireWorkspace) -> Option<Workspace> {
    Some(Workspace {
        view: wire.view,
        diagram: store.unpack_diagram(wire.diagram)?,
        path: wire.path,
    })
}

fn pack_state(store: &mut Store, state: &ProofState) -> WireState {
    WireState {
        workspace: state
            .workspace
            .as_ref()
            .map(|workspace| pack_workspace(store, workspace)),
        metadata: state.metadata.clone(),
        boundary: state
            .boundary
            .as_ref()
            .map(|selected| (selected.boundary, store.pack_diagram(&selected.diagram))),
        stash: state
            .stash
            .iter()
            .map(|workspace| pack_workspace(store, workspace))
            .collect(),
    }
}

fn unpack_state(store: &mut Store, wire: WireState, signature: Signature) -> Option<ProofState> {
    Some(ProofState {
        signature,
        workspace: match wire.workspace {
            None => None,
            Some(workspace) => Some(unpack_workspace(store, workspace)?),
        },
        metadata: wire.metadata,
        boundary: match wire.boundary {
            None => None,
            Some((boundary, key)) => Some(SelectedBoundary {
                boundary,
                diagram: store.unpack_diagram(key)?,
            }),
        },
        stash: wire
            .stash
            .into_iter()
            .map(|workspace| unpack_workspace(store, workspace))
            .collect::<Option<_>>()?,
    })
}

fn pack_option(store: &mut Store, option: AttachOption) -> WireAttachOption {
    WireAttachOption {
        diagram: store.pack_diagram(&option.diagram.into()),
        substitution: option.substitution.map(|substitution| {
            substitution
                .iter()
                .map(|(generator, diagram)| (*generator, store.pack_diagram(diagram)))
                .collect()
        }),
        generator: option.generator,
        boundary_path: option.boundary_path,
        embedding: option.embedding,
        label: option.label,
    }
}

fn unpack_option(store: &mut Store, wire: WireAttachOption) -> Option<AttachOption> {
    Some(AttachOption {
        diagram: DiagramN::try_from(store.unpack_diagram(wire.diagram)?).ok()?,
        substitution: match wire.substitution {
            None => None,
            Some(substitution) => Some(
                substitution
                    .into_iter()
                    .map(|(generator, key)| Some((generator, store.unpack_diagram(key)?)))
                    .collect::<Option<_>>()?,
            ),
        },
        generator: wire.generator,
        boundary_path: wire.boundary_path,
        embedding: wire.embedding,
        label: wire.label,
    })
}

fn encode<T: Serialize>(value: &T) -> Vec<u8> {
    rmp_serde::to_vec_named(value).expect("encoding to memory cannot fail")
}

const CORRUPT: &str = "the message was garbled";

// The side that asks.

/// The end of the line that asks for jobs to be done, and takes the answers.
///
/// It keeps track of the signature the [`Executor`] holds, and so must be told when that one is
/// lost (the executor restarted: [`forget`](Self::forget)).
#[derive(Default)]
pub struct Remote {
    next_number: u64,
    /// The signature the executor holds, as far as it is known, and its number.
    holds: Option<(u64, Signature)>,
    /// The actions of the last request, which the answer refers to.
    asked: Vec<crate::proof::Action>,
}

/// What came of a request.
#[derive(Debug)]
pub enum Answer {
    Done {
        output: Output,
        /// How long the job took, in milliseconds.
        worked_ms: f64,
    },
    /// The executor did not hold the signature the request relied on. Ask again with
    /// `send_signature` set.
    NeedsSignature,
}

impl Remote {
    const fn number(&mut self) -> u64 {
        self.next_number += 1;
        self.next_number
    }

    /// Forgets what the executor holds, as it is gone or not known to be what it was.
    pub fn forget(&mut self) {
        self.holds = None;
    }

    /// The bytes that ask for the work to be done on the proof. The signature is left out if the
    /// executor holds it (unless `send_signature`).
    pub fn request(
        &mut self,
        proof: &ProofState,
        work: Work,
        step_delay_ms: u32,
        send_signature: bool,
    ) -> Vec<u8> {
        self.asked = match &work {
            Work::Actions { actions, .. } => actions.clone(),
            Work::AttachOptions { .. } => vec![],
        };
        let mut store = Store::default();
        let signature = match &self.holds {
            Some((number, held)) if !send_signature && held.identical(&proof.signature) => {
                SignatureRef::Held(*number)
            }
            _ => {
                let number = self.number();
                self.holds = Some((number, proof.signature.clone()));
                SignatureRef::Sent(number, pack_signature(&mut store, &proof.signature))
            }
        };
        let state = pack_state(&mut store, proof);
        let fresh = self.number();
        encode(&Request {
            store,
            signature,
            state,
            work,
            fresh,
            step_delay_ms,
        })
    }

    /// Makes sense of the answer to a request. An `Err` is a failure of the executor (and what
    /// it says about it), after which what it holds is not known.
    pub fn response(&mut self, bytes: &[u8]) -> Result<Answer, String> {
        let result = self.decode(bytes);
        if result.is_err() {
            self.forget();
        }
        result
    }

    fn decode(&mut self, bytes: &[u8]) -> Result<Answer, String> {
        let response: Response = rmp_serde::from_slice(bytes).map_err(|e| e.to_string())?;
        let done = match response {
            Response::NeedsSignature => {
                self.forget();
                return Ok(Answer::NeedsSignature);
            }
            Response::Failed(message) => return Err(message),
            Response::Done(done) => done,
        };
        // The signature the job began with, which is what the last request left behind.
        let (_, mut signature) = self.holds.take().ok_or("nothing was asked")?;
        let mut store = done.store;
        let output = match done.output {
            WireOutput::Actions { steps, error } => {
                let mut applied = Vec::with_capacity(steps.len());
                let mut positions = Vec::with_capacity(steps.len());
                for step in steps {
                    if let Some(wire) = step.signature {
                        signature = unpack_signature(&mut store, wire).ok_or(CORRUPT)?;
                    }
                    let state =
                        unpack_state(&mut store, step.state, signature.clone()).ok_or(CORRUPT)?;
                    let action = self.asked.get(step.position).ok_or(CORRUPT)?;
                    positions.push(step.position);
                    applied.push((action.clone(), proof_from_state(state)));
                }
                Output::Actions(ActionsOutcome {
                    applied,
                    positions,
                    error: error.map(ProofError::Remote),
                })
            }
            WireOutput::AttachOptions(options) => Output::AttachOptions(
                options
                    .into_iter()
                    .map(|option| unpack_option(&mut store, option))
                    .collect::<Option<_>>()
                    .ok_or(CORRUPT)?,
            ),
        };
        self.holds = Some((done.holds, signature));
        Ok(Answer::Done {
            output,
            worked_ms: done.worked_ms,
        })
    }
}

// The side that works.

/// The end of the line that does the jobs.
#[derive(Default)]
pub struct Executor {
    /// The last signature seen or made, with its number.
    holds: Option<(u64, Signature)>,
}

impl Executor {
    /// Does the job that the request describes, and gives the bytes to answer with. `before_step`
    /// is told what is going on before each step of the job, which is the only time it can be.
    pub fn run(&mut self, request: &[u8], before_step: &mut dyn FnMut(Step)) -> Vec<u8> {
        let response = match self.try_run(request, before_step) {
            Ok(response) => response,
            Err(message) => Response::Failed(message),
        };
        homotopy_core::collect_garbage();
        encode(&response)
    }

    fn try_run(
        &mut self,
        request: &[u8],
        before_step: &mut dyn FnMut(Step),
    ) -> Result<Response, String> {
        let Request {
            mut store,
            signature,
            state,
            work,
            fresh,
            step_delay_ms,
        } = rmp_serde::from_slice(request).map_err(|e| e.to_string())?;

        let (number, signature) = match signature {
            SignatureRef::Held(number) => match &self.holds {
                Some((held, signature)) if *held == number => (number, signature.clone()),
                _ => return Ok(Response::NeedsSignature),
            },
            SignatureRef::Sent(number, wire) => {
                let signature = unpack_signature(&mut store, wire).ok_or(CORRUPT)?;
                self.holds = Some((number, signature.clone()));
                (number, signature)
            }
        };
        let state = unpack_state(&mut store, state, signature.clone()).ok_or(CORRUPT)?;

        let started = Instant::now();
        let mut store = Store::default();
        let mut holds = number;
        let output = match work {
            Work::Actions { actions, replay } => {
                let proof = proof_from_state(state);
                let job = if replay {
                    ActionsJob::replay(proof, actions, REPLAYING)
                } else {
                    ActionsJob::chain(proof, actions)
                };
                let outcome = run_with(job, step_delay_ms, before_step);
                let mut previous = signature;
                let mut steps = Vec::with_capacity(outcome.applied.len());
                for ((_, proof), position) in outcome.applied.into_iter().zip(outcome.positions) {
                    let changed = !proof.signature.identical(&previous);
                    if changed {
                        previous = proof.signature.clone();
                        holds = fresh;
                    }
                    steps.push(WireStep {
                        position,
                        state: pack_state(&mut store, &proof),
                        signature: changed.then(|| pack_signature(&mut store, &previous)),
                    });
                }
                self.holds = Some((holds, previous));
                WireOutput::Actions {
                    steps,
                    error: outcome.error.map(|error| error.to_string()),
                }
            }
            Work::AttachOptions { point, weak_units } => {
                let job = AttachOptionsJob::new(&state, &point, weak_units)
                    .map_err(|e| format!("nothing to attach to: {e:?}"))?;
                let options = run_with(job, step_delay_ms, before_step);
                WireOutput::AttachOptions(
                    options
                        .into_iter()
                        .map(|option| pack_option(&mut store, option))
                        .collect(),
                )
            }
        };
        Ok(Response::Done(Box::new(Done {
            store,
            holds,
            output,
            worked_ms: started.elapsed().as_secs_f64() * 1000.0,
        })))
    }
}

/// Runs a job to the end, saying what it is doing before each step.
fn run_with<J: Job>(mut job: J, delay_ms: u32, before_step: &mut dyn FnMut(Step)) -> J::Output {
    loop {
        before_step(Step {
            label: job.label(),
            progress: job.progress(),
            delay_ms,
        });
        if let Some(output) = job.step() {
            return output;
        }
    }
}

#[cfg(test)]
mod tests {
    use homotopy_core::signature::Invertibility;

    use super::*;
    use crate::proof::{SignatureEdit, SignatureItemEdit};

    /// A signature with everything in it that a proof state can hold, including a folder and a
    /// removed generator that leaves a gap in the numbering of the nodes.
    fn busy_signature() -> Signature {
        let mut signature = Signature::default();
        let x = signature.create_generator_zero("x");
        let p = signature
            .create_generator(x.into(), x.into(), "P", Invertibility::Directed)
            .unwrap();
        let q = signature
            .create_generator(x.into(), x.into(), "Q", Invertibility::Dualisable(1))
            .unwrap();
        let e = signature
            .create_generator(
                p.clone().into(),
                p.clone().into(),
                "e",
                Invertibility::Invertible,
            )
            .unwrap();
        let junk = signature.create_generator_zero("junk").generator;
        let (p, q_diagram, e) = (p.max_generator().generator, q, e.max_generator().generator);
        let q = q_diagram.max_generator().generator;
        let node = |signature: &Signature, g| signature.find_node(g).unwrap();
        for edit in [
            SignatureEdit::Edit(
                node(&signature, p),
                SignatureItemEdit::MakePlaceholder(true),
            ),
            SignatureEdit::Edit(
                node(&signature, q),
                SignatureItemEdit::SetDependsOn(vec![p]),
            ),
            SignatureEdit::Edit(
                node(&signature, q),
                SignatureItemEdit::Reshape(VertexShape::Square),
            ),
            SignatureEdit::NewFolder(signature.as_tree().root()),
            SignatureEdit::Remove(node(&signature, junk)),
            SignatureEdit::AddSymmetry(Symmetry {
                name: "op".to_owned(),
                directions: vec![1],
            }),
        ] {
            signature.update(&edit).unwrap();
        }
        let instance = signature
            .instantiate(e, &[(p, Diagram::from(q_diagram))])
            .unwrap();
        signature
            .update(&SignatureEdit::ToggleInstances(node(&signature, e)))
            .unwrap();
        assert!(signature.generator_info(instance).is_some());
        signature
    }

    #[test]
    fn signatures_cross_whole() {
        let signature = busy_signature();
        let mut store = Store::default();
        let wire = pack_signature(&mut store, &signature);
        let bytes = encode(&(store, wire));
        let (mut store, wire): (Store, WireSignature) = rmp_serde::from_slice(&bytes).unwrap();
        let unpacked = unpack_signature(&mut store, wire).unwrap();

        assert_eq!(unpacked, signature);
        // Down to the numbering of the nodes, the gap and everything else about the items.
        assert!(unpacked.identical(&signature));
        assert_eq!(
            serde_json::to_string(&unpacked).unwrap(),
            serde_json::to_string(&signature).unwrap()
        );
    }

    #[test]
    fn identical_is_more_than_equal() {
        let mut signature = Signature::default();
        let a = signature.create_generator_zero("a");
        let b = signature.create_generator_zero("b");
        let (node_a, node_b) = (
            signature.find_node(a.generator).unwrap(),
            signature.find_node(b.generator).unwrap(),
        );
        // Moving b before a, and then a before b, leaves the tree as it was.
        let mut moved = signature.clone();
        moved
            .update(&SignatureEdit::MoveBefore(node_b, node_a))
            .unwrap();
        assert_ne!(moved, signature);
        moved
            .update(&SignatureEdit::MoveBefore(node_a, node_b))
            .unwrap();
        assert!(moved.identical(&signature));
        // A signature made differently, by creating and removing an item, has the same items in
        // the same order and is equal, but the nodes are numbered differently.
        let mut other = Signature::default();
        other.create_generator_zero("a");
        let junk = other.create_generator_zero("junk").generator;
        let junk_node = other.find_node(junk).unwrap();
        other.update(&SignatureEdit::Remove(junk_node)).unwrap();
        other.create_generator_zero("b");
        assert_eq!(other, signature);
        assert!(!other.identical(&signature));
    }
}
