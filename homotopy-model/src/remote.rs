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
use instant::{Duration, Instant};
use serde::{Deserialize, Serialize};

use crate::{
    history::proof_from_state,
    job::{Job, Progress},
    proof::{
        generators::{GeneratorInfo, Instance},
        ActionsJob, ActionsOutcome, AttachOption, AttachOptionsJob, AttachOutcome, FolderInfo,
        Found, InferenceLimits, InferenceReport, Metadata, ProofError, ProofState,
        SelectedBoundary, Signature, SignatureItem, Slot, Streamed, View, Workspace, REPLAYING,
    },
    serialize::serialize,
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
        limits: InferenceLimits,
        /// The one generator to search for the ways to attach, if not all of them.
        #[serde(default)]
        only: Option<Generator>,
    },
    /// Make the project a `.hom` file (see [`serialize`](crate::serialize)), to save it in the
    /// background (see [`autosave`](crate::autosave)).
    Serialize,
}

/// What a job came to.
#[derive(Debug)]
pub enum Output {
    Actions(ActionsOutcome),
    AttachOptions(AttachOutcome),
    /// The bytes of the file.
    Serialized(Vec<u8>),
}

/// What there is to say about the job just before each of its steps, for the executor's caller to
/// pass on: it is only then that anything can be said.
#[derive(Copy, Clone, Debug)]
pub struct Step {
    pub label: &'static str,
    pub progress: Option<Progress>,
    /// The most that the step may take if the job keeps its limits, in milliseconds, or `0` if
    /// there is no such limit (see [`Job::step_limit`]).
    pub limit_ms: u32,
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
    /// For debugging: only the steps whose label contains this are slowed down (all, if it is
    /// empty).
    #[serde(default)]
    step_delay_label: String,
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
    /// How many bytes at the end of the message are not part of it but the file that a
    /// [`Work::Serialize`] made, which would be slower to wrap in the message than to append.
    trailer: usize,
}

#[derive(Serialize, Deserialize)]
enum WireOutput {
    Actions {
        steps: Vec<WireStep>,
        error: Option<String>,
    },
    AttachOptions {
        options: Vec<WireAttachOption>,
        inference: InferenceReport,
        /// If the options were sent ahead as they were found (see [`Delta`]): how many, and
        /// then `options` is empty, as the asker has them already.
        #[serde(default)]
        streamed: Option<u32>,
    },
    /// The file is in the trailer.
    Serialized,
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
    #[serde(default)]
    reflection: bool,
    #[serde(default)]
    inverse: bool,
}

/// An option with its place in the final list.
#[derive(Serialize, Deserialize)]
struct WireFound {
    slot: Slot,
    option: WireAttachOption,
}

/// The options that a search for attachments found since it last said, sent ahead of the
/// answer so that they can be shown while the search goes on. Only what is new is in it, and
/// what it has is complete (the diagrams are in its own store).
#[derive(Serialize, Deserialize)]
struct Delta {
    store: Store,
    found: Vec<WireFound>,
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
        reflection: option.reflection,
        inverse: option.inverse,
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
        reflection: wire.reflection,
        inverse: wire.inverse,
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
    /// For debugging: which steps the delay of a request applies to (see
    /// [`slow_only`](Self::slow_only)).
    slow_label: String,
    /// The options of the last request that the executor sent ahead (see
    /// [`options`](Self::options)), which an answer that says it has sent them refers to.
    streamed: Streamed,
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

    /// For debugging: makes the delay that requests ask for apply only to steps whose label
    /// contains this. Empty for all of them.
    pub fn slow_only(&mut self, label: &str) {
        label.clone_into(&mut self.slow_label);
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
        self.request_with(Store::default(), proof, work, step_delay_ms, send_signature)
    }

    /// Whether the executor is known to hold this signature, so that a request need not send
    /// it.
    #[must_use]
    pub fn holds(&self, signature: &Signature) -> bool {
        self.holds
            .as_ref()
            .is_some_and(|(_, held)| held.identical(signature))
    }

    /// The bytes that give the executor diagrams to hold for the requests that follow, which
    /// are packed in `store` (see [`Packer`](crate::autosave::Packer)): a signature that has to
    /// be sent can be sent a piece at a time, ahead of the request, so that no one piece takes
    /// long to prepare.
    #[must_use]
    pub fn preload(store: &Store) -> Vec<u8> {
        encode(store)
    }

    /// [`request`](Self::request) with some of the diagrams packed in the store already. What
    /// has been sent ahead of it with [`preload`](Self::preload) has to have been taken out of
    /// the store, and has to have reached the executor first.
    pub fn request_with(
        &mut self,
        mut store: Store,
        proof: &ProofState,
        work: Work,
        step_delay_ms: u32,
        send_signature: bool,
    ) -> Vec<u8> {
        self.asked = match &work {
            Work::Actions { actions, .. } => actions.clone(),
            Work::AttachOptions { .. } | Work::Serialize => vec![],
        };
        self.streamed = Streamed::default();
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
            step_delay_label: self.slow_label.clone(),
        })
    }

    /// Makes sense of options that a search for attachments sent ahead of its answer (see
    /// [`Executor::run_streaming`]): the options that were found since the last, each with its
    /// place in the final list. They are kept, for the answer to refer to. An `Err` is as for
    /// [`response`](Self::response).
    pub fn options(&mut self, bytes: &[u8]) -> Result<Vec<Found>, String> {
        let result = self.decode_options(bytes);
        if result.is_err() {
            self.forget();
        }
        result
    }

    fn decode_options(&mut self, bytes: &[u8]) -> Result<Vec<Found>, String> {
        let Delta { mut store, found } = rmp_serde::from_slice(bytes).map_err(|e| e.to_string())?;
        let found: Vec<Found> = found
            .into_iter()
            .map(|WireFound { slot, option }| {
                Some(Found {
                    slot,
                    option: unpack_option(&mut store, option)?,
                })
            })
            .collect::<Option<_>>()
            .ok_or(CORRUPT)?;
        self.streamed.extend(found.iter().cloned());
        Ok(found)
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
            WireOutput::AttachOptions {
                options,
                inference,
                streamed,
            } => {
                let options = match streamed {
                    // They were all sent ahead, and are put in their places: if one is missing
                    // the answer is not one to use.
                    Some(count) => {
                        let streamed = std::mem::take(&mut self.streamed);
                        if streamed.len() != count as usize || !options.is_empty() {
                            return Err(CORRUPT.to_owned());
                        }
                        streamed.into_options()
                    }
                    None => options
                        .into_iter()
                        .map(|option| unpack_option(&mut store, option))
                        .collect::<Option<_>>()
                        .ok_or(CORRUPT)?,
                };
                Output::AttachOptions(AttachOutcome { options, inference })
            }
            WireOutput::Serialized => {
                let start = bytes.len().checked_sub(done.trailer).ok_or(CORRUPT)?;
                Output::Serialized(bytes[start..].to_vec())
            }
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
    /// Diagrams that were sent ahead of a request (see [`Remote::preload`]), for the next
    /// request to find the rest of the way.
    preloaded: Store,
}

impl Executor {
    /// Does the job that the request describes, and gives the bytes to answer with. `before_step`
    /// is told what is going on before each step of the job, which is the only time it can be.
    pub fn run(&mut self, request: &[u8], before_step: &mut dyn FnMut(Step)) -> Vec<u8> {
        self.run_with_checkpoints(request, before_step, &mut |_| {})
    }

    /// [`run`](Self::run), and `on_checkpoint` is given the bytes of an answer that does for the
    /// request, if the job is ended before it is done, whenever a job that has such a thing says
    /// it has a new one (see [`Job::checkpoint`]). It is for the asker to keep the latest of: the
    /// bytes are an answer as the end of the job gives it, to [`Remote::response`].
    pub fn run_with_checkpoints(
        &mut self,
        request: &[u8],
        before_step: &mut dyn FnMut(Step),
        on_checkpoint: &mut dyn FnMut(Vec<u8>),
    ) -> Vec<u8> {
        self.run_streaming(request, before_step, on_checkpoint, None)
    }

    /// [`run_with_checkpoints`](Self::run_with_checkpoints), and a search for attachments gives
    /// `on_options` the bytes of the options it has found since it last did, as it goes (before
    /// each of its steps), for [`Remote::options`]. The answer then does not repeat them, so the
    /// asker has to have taken in all of them before the answer, and every checkpoint (which is
    /// given after the options it refers to) is only the report. Without `on_options` the
    /// answer has all of the options.
    pub fn run_streaming(
        &mut self,
        request: &[u8],
        before_step: &mut dyn FnMut(Step),
        on_checkpoint: &mut dyn FnMut(Vec<u8>),
        on_options: Option<&mut dyn FnMut(Vec<u8>)>,
    ) -> Vec<u8> {
        let (response, trailer) =
            match self.try_run(request, before_step, on_checkpoint, on_options) {
                Ok(done) => done,
                Err(message) => (Response::Failed(message), vec![]),
            };
        homotopy_core::collect_garbage();
        let mut bytes = encode(&response);
        bytes.extend(trailer);
        bytes
    }

    /// Takes in diagrams that [`Remote::preload`] made the bytes of. There is no answer: if they
    /// are garbled the request that comes after them is not understood, and says so.
    pub fn preload(&mut self, bytes: &[u8]) {
        match rmp_serde::from_slice::<Store>(bytes) {
            Ok(store) => self.preloaded.extend(store),
            Err(error) => tracing::error!("Could not read the diagrams sent ahead: {error}"),
        }
    }

    fn try_run(
        &mut self,
        request: &[u8],
        before_step: &mut dyn FnMut(Step),
        on_checkpoint: &mut dyn FnMut(Vec<u8>),
        on_options: Option<&mut dyn FnMut(Vec<u8>)>,
    ) -> Result<(Response, Vec<u8>), String> {
        let Request {
            mut store,
            signature,
            state,
            work,
            fresh,
            step_delay_ms,
            step_delay_label,
        } = rmp_serde::from_slice(request).map_err(|e| e.to_string())?;
        let mut before_step = |mut step: Step| {
            if !step.label.contains(&step_delay_label) {
                step.delay_ms = 0;
            }
            before_step(step);
        };
        let before_step: &mut dyn FnMut(Step) = &mut before_step;
        // What was sent ahead is for this request; it is not kept for the next.
        store.extend(std::mem::take(&mut self.preloaded));

        let (number, signature) = match signature {
            SignatureRef::Held(number) => match &self.holds {
                Some((held, signature)) if *held == number => (number, signature.clone()),
                _ => return Ok((Response::NeedsSignature, vec![])),
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
        let mut trailer = vec![];
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
            Work::AttachOptions {
                point,
                weak_units,
                limits,
                only,
            } => {
                let job = AttachOptionsJob::new(&state, &point, weak_units, limits, only)
                    .map_err(|e| format!("nothing to attach to: {e:?}"))?;
                // How many options were sent ahead (when they are).
                let sent = std::cell::Cell::new(0_usize);
                let mut on_options = on_options;
                let streaming = on_options.is_some();
                let outcome = run_checkpointed(
                    job,
                    step_delay_ms,
                    before_step,
                    &mut |outcome| {
                        // What a job that is ended in the middle comes to: an answer as any other.
                        let mut partial = Store::default();
                        let output = pack_attach_outcome(
                            &mut partial,
                            outcome,
                            streaming.then(|| sent.get()),
                        );
                        on_checkpoint(encode(&Response::Done(Box::new(Done {
                            store: partial,
                            holds: number,
                            output,
                            worked_ms: started.elapsed().as_secs_f64() * 1000.0,
                            trailer: 0,
                        }))));
                    },
                    &mut |job: &mut AttachOptionsJob| {
                        let Some(on_options) = on_options.as_mut() else {
                            return;
                        };
                        let found = job.take_found();
                        if found.is_empty() {
                            return;
                        }
                        sent.set(sent.get() + found.len());
                        let mut store = Store::default();
                        let found = found
                            .into_iter()
                            .map(|Found { slot, option }| WireFound {
                                slot,
                                option: pack_option(&mut store, option),
                            })
                            .collect();
                        on_options(encode(&Delta { store, found }));
                    },
                );
                pack_attach_outcome(&mut store, outcome, streaming.then(|| sent.get()))
            }
            Work::Serialize => {
                trailer = serialize(state.signature, state.workspace, state.metadata);
                WireOutput::Serialized
            }
        };
        Ok((
            Response::Done(Box::new(Done {
                store,
                holds,
                output,
                worked_ms: started.elapsed().as_secs_f64() * 1000.0,
                trailer: trailer.len(),
            })),
            trailer,
        ))
    }
}

/// How many times as long as a checkpoint took to make there is between checkpoints at least.
const CHECKPOINT_COST_FACTOR: u32 = 10;

/// Runs a job to the end, saying what it is doing before each step.
fn run_with<J: Job>(job: J, delay_ms: u32, before_step: &mut dyn FnMut(Step)) -> J::Output {
    run_checkpointed(job, delay_ms, before_step, &mut |_| {}, &mut |_| {})
}

/// [`run_with`], passing on what the job would come to if it were ended whenever it says it has
/// something new to say about that. `on_progress` is given the job before each step, and once
/// more when it is over, to take what it found from it (before the checkpoint of that step, so
/// that whatever a checkpoint refers to has been taken).
fn run_checkpointed<J: Job>(
    mut job: J,
    delay_ms: u32,
    before_step: &mut dyn FnMut(Step),
    on_checkpoint: &mut dyn FnMut(J::Output),
    on_progress: &mut dyn FnMut(&mut J),
) -> J::Output {
    // A checkpoint is as costly as an answer is (the diagrams of the options are packed, and
    // may be large), so they are not made again until their cost is a tenth of the time since:
    // a job that is ended loses what it found since the last one, and what it says it had not
    // finished is cut short or not tried, to be honest about that.
    let mut not_before: Option<Instant> = None;
    // The longest a checkpoint took, and the longest it took to take what the job found: the
    // silence after a report is a step and perhaps one of each, which the limit that is reported
    // allows for.
    let mut slowest = Duration::ZERO;
    let mut slowest_found = Duration::ZERO;
    loop {
        // (First: what the last step found is not made to wait for the next.)
        let taking = Instant::now();
        on_progress(&mut job);
        slowest_found = slowest_found.max(taking.elapsed());
        before_step(Step {
            label: job.label(),
            progress: job.progress(),
            limit_ms: job.step_limit().map_or(0, |limit| {
                u32::try_from((limit + slowest + slowest_found).as_millis()).unwrap_or(u32::MAX)
            }),
            delay_ms,
        });
        if not_before.is_none_or(|time| Instant::now() >= time) {
            if let Some(checkpoint) = job.checkpoint() {
                let started = Instant::now();
                on_checkpoint(checkpoint);
                let took = started.elapsed();
                slowest = slowest.max(took);
                not_before = Some(Instant::now() + took * CHECKPOINT_COST_FACTOR);
            }
        }
        if let Some(output) = job.step() {
            on_progress(&mut job);
            return output;
        }
    }
}

/// The outcome of a search for attachments as it goes on the wire. If `sent` says how many of
/// the options were sent ahead and that is all of them, they are not repeated.
fn pack_attach_outcome(
    store: &mut Store,
    outcome: AttachOutcome,
    sent: Option<usize>,
) -> WireOutput {
    let AttachOutcome { options, inference } = outcome;
    if let Some(sent) = sent.filter(|sent| *sent == options.len()) {
        return WireOutput::AttachOptions {
            options: vec![],
            inference,
            streamed: Some(u32::try_from(sent).unwrap_or(u32::MAX)),
        };
    }
    WireOutput::AttachOptions {
        options: options
            .into_iter()
            .map(|option| pack_option(store, option))
            .collect(),
        inference,
        streamed: None,
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
