//! The work of an [`Action`](super::Action) that can take long enough to be done in steps.
//!
//! [`State::start`](super::State::start) either does an action there and then, or hands back a
//! [`Task`]: a [`Job`] that works on a copy of what it needs, and whose output
//! [`State::finish`](super::State::finish) commits. Whoever runs the task decides how: all at
//! once ([`State::update`](super::State::update), for tests and the like), or a slice at a time
//! with the page free to respond in between (the app), where it may be dropped at any time to
//! cancel it. Nothing happens to the state until the task is finished, so cancelling leaves
//! everything as it was.

use std::collections::VecDeque;

use homotopy_core::{Diagram, Height};
use homotopy_graphics::{manim, stl, tikz};
use homotopy_model::{
    job::{Job, Progress},
    proof::{ActionsJob, ActionsOutcome, AttachOption, AttachOptionsJob, ProofState, Signature},
    remote::{Output, Work},
};

use super::{proof, ImageFormat};

/// The size in bytes below which importing a project is not worth having a worker do (the
/// import takes about as long as preparing the request and taking in the answer, and under
/// 100ms in the debug build).
const SMALL_IMPORT: usize = 64 * 1024;

/// What the actions of a task were, which decides what committing them involves.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ActionsKind {
    /// Actions that the user performs, which are recorded for the crash dump and clear the
    /// selection.
    Proof,
    /// Actions that are replayed from a file.
    Import,
}

pub enum Task {
    Actions {
        job: Box<ActionsJob>,
        kind: ActionsKind,
        /// Whether actions before those in the job were redone, so that something has changed
        /// even if the job itself changes nothing.
        redone: bool,
    },
    AttachOptions(Box<AttachOptionsJob>),
    Export(ExportJob),
}

pub enum TaskOutput {
    Actions {
        outcome: ActionsOutcome,
        kind: ActionsKind,
        redone: bool,
    },
    AttachOptions(Vec<AttachOption>),
    Export(ExportOutput),
}

impl Task {
    /// Whether the next step is one that cannot be split up and may take long, in which case the
    /// busy indicator should be on screen before it is done.
    #[must_use]
    pub fn next_step_may_block(&self) -> bool {
        match self {
            Self::Actions { job, .. } => job.next_step_is_heavy(),
            Self::AttachOptions(_) => false,
            Self::Export(job) => job.next_step_is_heavy(),
        }
    }
}

impl Task {
    /// What it takes to have the task done in a worker (see [`homotopy_model::remote`]) if that is
    /// worth the trouble: the proof it works on, given that of the state it was made from, and
    /// the work to do on it. A task is worth it when a step of it can take long and cannot be
    /// split up (the cost of passing the proof to the worker is then the smaller part of it),
    /// or when it is made of many. Cheap tasks, which are nearly all of them, stay where they are.
    #[must_use]
    pub fn offer<'a>(&'a self, proof: &'a ProofState) -> Option<(&'a ProofState, Work)> {
        match self {
            Self::Actions { job, .. } => {
                let (state, work) = job.offer()?;
                let worth_it = match &work {
                    // Importing a project is mostly turning bytes into diagrams, which has to
                    // be done again on this side of the worker for the result: so only a
                    // project too big for that to be quick is worth sending.
                    Work::Actions { actions, replay } => {
                        *replay
                            || (job.next_step_is_heavy()
                                && !matches!(
                                    actions.first(),
                                    Some(proof::Action::ImportProof(data))
                                        if data.0.len() < SMALL_IMPORT
                                ))
                    }
                    Work::AttachOptions { .. } => false,
                };
                worth_it.then_some((state, work))
            }
            // The search takes long when there are schemas to infer instances of.
            Self::AttachOptions(job) => job
                .has_schemas()
                .then(|| job.offer())
                .flatten()
                .map(|work| (proof, work)),
            Self::Export(_) => None,
        }
    }

    /// What the task came to, given what the worker answered.
    #[must_use]
    pub fn output_of(&self, output: Output) -> Option<TaskOutput> {
        match (self, output) {
            (Self::Actions { kind, redone, .. }, Output::Actions(outcome)) => {
                Some(TaskOutput::Actions {
                    outcome,
                    kind: *kind,
                    redone: *redone,
                })
            }
            (Self::AttachOptions(_), Output::AttachOptions(options)) => {
                Some(TaskOutput::AttachOptions(options))
            }
            _ => None,
        }
    }
}

impl Job for Task {
    type Output = TaskOutput;

    fn label(&self) -> &'static str {
        match self {
            Self::Actions { job, .. } => job.label(),
            Self::AttachOptions(job) => job.label(),
            Self::Export(job) => job.label(),
        }
    }

    fn progress(&self) -> Option<Progress> {
        match self {
            Self::Actions { job, .. } => job.progress(),
            Self::AttachOptions(job) => job.progress(),
            Self::Export(job) => job.progress(),
        }
    }

    fn step(&mut self) -> Option<TaskOutput> {
        match self {
            Self::Actions { job, kind, redone } => job.step().map(|outcome| TaskOutput::Actions {
                outcome,
                kind: *kind,
                redone: *redone,
            }),
            Self::AttachOptions(job) => job.step().map(TaskOutput::AttachOptions),
            Self::Export(job) => job.step().map(TaskOutput::Export),
        }
    }
}

/// The rendering of a diagram, or of each of its slices, in a format that takes a while to
/// produce.
pub struct ExportJob {
    signature: Signature,
    format: ImageFormat,
    view_dimension: u8,
    /// The files to render, named by the heights of the slices if there are many.
    pending: VecDeque<(String, Diagram)>,
    total: usize,
    rendered: Vec<(String, String)>,
    multiple: bool,
}

/// The rendered files of an [`ExportJob`].
pub struct ExportOutput {
    pub files: Vec<(String, String)>,
    pub extension: &'static str,
    pub multiple: bool,
}

impl ExportJob {
    #[must_use]
    pub fn single(
        signature: Signature,
        format: ImageFormat,
        diagram: Diagram,
        view_dimension: u8,
    ) -> Self {
        Self {
            signature,
            format,
            view_dimension,
            pending: VecDeque::from([("homotopy_io_export".to_owned(), diagram)]),
            total: 1,
            rendered: vec![],
            multiple: false,
        }
    }

    /// An export of each slice of the diagram to a file of a zip.
    #[must_use]
    pub fn slices(
        signature: Signature,
        format: ImageFormat,
        diagram: &homotopy_core::DiagramN,
        view_dimension: u8,
    ) -> Self {
        let extension = format.extension();
        let pending: VecDeque<_> = diagram
            .slices()
            .enumerate()
            .map(|(i, slice)| {
                let name = match Height::from(i) {
                    Height::Regular(i) => format!("regular{i}"),
                    Height::Singular(i) => format!("singular{i}"),
                };
                (format!("{name}.{extension}"), slice)
            })
            .collect();
        Self {
            signature,
            format,
            view_dimension,
            total: pending.len(),
            pending,
            rendered: vec![],
            multiple: true,
        }
    }

    fn next_step_is_heavy(&self) -> bool {
        !self.pending.is_empty()
    }

    #[allow(clippy::unused_self)] // To go with the rest of what a job says about itself.
    const fn label(&self) -> &'static str {
        "Exporting"
    }

    fn progress(&self) -> Option<Progress> {
        self.multiple.then_some(Progress {
            done: self.rendered.len(),
            total: self.total,
        })
    }

    fn step(&mut self) -> Option<ExportOutput> {
        let Some((name, diagram)) = self.pending.pop_front() else {
            return Some(ExportOutput {
                files: std::mem::take(&mut self.rendered),
                extension: self.format.extension(),
                multiple: self.multiple,
            });
        };
        self.rendered.push((
            name,
            render(&diagram, self.view_dimension, &self.signature, self.format),
        ));
        None
    }
}

/// Renders a diagram as text in one of the formats that are.
pub fn render(
    diagram: &Diagram,
    dimension: u8,
    signature: &Signature,
    format: ImageFormat,
) -> String {
    match format {
        ImageFormat::Stl(options) => stl::render(diagram, signature, options).unwrap(),
        ImageFormat::Tikz(options) => tikz::render(diagram, dimension, signature, options).unwrap(),
        ImageFormat::Manim(options) => {
            let stylesheet = manim::stylesheet(signature);
            manim::render(diagram, dimension, signature, &stylesheet, options).unwrap()
        }
        _ => unreachable!(),
    }
}
