//! Finding the ways to attach a generator at a point of the workspace diagram.

use std::{collections::VecDeque, rc::Rc, time::Duration};

use homotopy_common::{
    deadline,
    hash::{FastHashMap, FastHashSet},
};
use homotopy_core::{
    common::{BoundaryPath, Generator},
    signature::Signature as _,
    typecheck::{typecheck, Mode},
    Boundary, Diagram, Diagram0, DiagramN, Direction, Height, Orientation, SliceIndex,
};
use instant::Instant;
use serde::{Deserialize, Serialize};

use super::{
    generators::GeneratorInfo,
    inference::{Cells, InferSearch, Pool},
    AttachOption, ProofState, Signature, Structure,
};
use crate::{
    job::{Job, Progress},
    remote::Work,
};

/// Why there is nothing to attach to.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NoAttachment {
    /// There is no diagram in the workspace.
    NoWorkspace,
    /// The point is on a boundary that the diagram does not have.
    NoBoundary,
    /// The generator that is asked for is not in the signature.
    UnknownGenerator,
}

/// Why a generator cannot be attached anywhere in a diagram, found without searching.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CannotAttach {
    /// There is no diagram in the workspace.
    NoDiagram,
    /// The generator is not in the signature.
    UnknownGenerator,
    /// The generator has more dimensions than a cell that can be attached to the diagram.
    TooHigh { generator: usize, diagram: usize },
    /// A 0-cell can only be attached by way of weak units.
    NeedsWeakUnits,
    /// The cells that the generator starts at, and the cells that it ends at, are not all in the
    /// diagram: and there is nothing to attach it to.
    NotInDiagram,
}

impl CannotAttach {
    /// What to tell the person, in plain text (not naming the generator, whose name may be a
    /// formula: they have just picked it).
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::NoDiagram => "Open a diagram in the workspace to attach to.".to_owned(),
            Self::UnknownGenerator => "That generator is not in the signature.".to_owned(),
            Self::TooHigh { generator, diagram } => format!(
                "This generator has dimension {generator}, and the diagram in the workspace has \
                 dimension {diagram}: cells of dimension up to {} can be attached to it.",
                diagram + 1
            ),
            Self::NeedsWeakUnits => "This generator is a 0-cell, which can only be attached by \
                 way of weak units (see Settings)."
                .to_owned(),
            Self::NotInDiagram => "The diagram in the workspace has none of the cells that this \
                 generator starts or ends at, so there is nothing to attach it to."
                .to_owned(),
        }
    }
}

/// Whether a generator can be attached anywhere in the diagram, as far as can be told without
/// searching.
///
/// `Err` is a way of telling that it cannot, and `Ok` that it might. This is what a drag of the
/// generator over the diagram asks, so it never searches (the search is the
/// [`AttachOptionsJob`]): it only looks at dimensions and at which cells there are, for a result
/// that is not wrong when it says no.
///
/// It looks at the diagram as a whole, and not at a point of it, so that it is the same for every
/// point; the search at the point may find nothing all the same.
pub fn can_attach(
    signature: &Signature,
    diagram: Option<&Diagram>,
    generator: Generator,
    weak_units: bool,
    limits: InferenceLimits,
) -> Result<(), CannotAttach> {
    let diagram = diagram.ok_or(CannotAttach::NoDiagram)?;
    let info = signature
        .generator_info(generator)
        .ok_or(CannotAttach::UnknownGenerator)?;
    let (cell, workspace) = (generator.dimension, diagram.dimension());
    if cell > workspace + 1 {
        return Err(CannotAttach::TooHigh {
            generator: cell,
            diagram: workspace,
        });
    }
    // The instances of a schema are made of cells that are not its own, so what can be said
    // about it is whether they are looked for: at the boundary of the dimension below it, or
    // in the diagram, which is as high as the diagram.
    let schema = info.placeholder || !signature.parameters(generator).is_empty();
    if schema && limits.enabled && cell >= 1 && (cell - 1) as u32 <= limits.max_dimension {
        return Ok(());
    }
    let present = diagram.generators();
    let in_diagram = |part: &Diagram| part.generators().keys().all(|g| present.contains_key(g));
    match &info.diagram {
        Diagram::Diagram0(_) => {
            // Only a weak identity of it can be attached.
            if !weak_units {
                Err(CannotAttach::NeedsWeakUnits)
            } else if in_diagram(&info.diagram) {
                Ok(())
            } else {
                Err(CannotAttach::NotInDiagram)
            }
        }
        // Cells (and their inverses, reflections and what is raised from them) are attached
        // along their source or their target.
        Diagram::DiagramN(d) => {
            if in_diagram(&d.source()) || in_diagram(&d.target()) {
                Ok(())
            } else {
                Err(CannotAttach::NotInDiagram)
            }
        }
    }
}

/// How much to spend on one generator when looking for what can be attached, and on inferring
/// the instances of schemas.
///
/// The searches that take long are the inference of instances (see
/// [`Signature::infer_instances`]), which grows with every placeholder a schema has and with the
/// cells there are to replace them with, and the matching of cells and their reflections in high
/// dimensions. Both are done a generator at a time, and the limit is on what *one generator* may
/// take, not on the whole search: a search that has more generators to look at takes longer, which
/// is understood, but a generator that has no end to it must not make the rest wait. There is no
/// limit on the whole search; `Esc` ends it as any job.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceLimits {
    /// Whether to infer instances at all. If not, schemas are not offered for attaching, but
    /// everything else is.
    pub enabled: bool,
    /// The most time, in milliseconds, that one generator may take before it is given up on and
    /// the search goes on to the next: the matching of the generator, its inverse and the cells
    /// raised from it, its reflections, and (for a schema) the inference of its instances, all
    /// together. What was found for it by then is offered (it is valid, and among what the
    /// whole search finds for it, in the same order), and the report says which generators were
    /// given up on ([`InferenceReport::cut_short`] for schemas, [`PlainReport::cut_short`]
    /// for the others). `0` for no limit.
    pub generator_ms: u32,
    /// Diagrams with more dimensions than this are not inferred instances for. The search
    /// grows with the dimension (more placeholders, with more cells to choose from), and a
    /// diagram of a high dimension is the one that nobody can wait for.
    pub max_dimension: u32,
    /// A heuristic, off unless asked for: when the point was given with sheets (coordinates
    /// beyond those the view shows, see [`SHEET_REACH`]), the replacements of placeholders are
    /// only made of cells that occur near the point. That can miss instances that use cells from
    /// elsewhere in the diagram, but never offers one that is not valid: the instances are
    /// checked as before, the pool they are chosen from is only smaller. The report says when
    /// it was in effect ([`InferenceReport::narrowed`]).
    #[serde(default)]
    pub narrow_to_sheet: bool,
}

/// The time one generator is given unless it is said otherwise: five seconds.
///
/// That is more than the matching of any one cell and its reflections takes in the optimised
/// build on the largest project there is a fixture of (the costliest, a reflection that does not
/// typecheck, 2.5 s natively), and what a schema got in practice when the time was for the whole
/// inference (20 s, a quarter of it for each of several schemas): the schemas that finish with
/// nothing to find take 4 to 7 s natively there, and the ones that do not finish never would. The
/// debug build, which is ten to twenty times slower, gives up on more.
pub const DEFAULT_GENERATOR_MS: u32 = 5_000;

/// How far from the point a cell may be to be in the narrowed pool.
///
/// In heights of each coordinate, when [`InferenceLimits::narrow_to_sheet`] is in effect: the
/// heights next to the point's are included, as the cells meeting at a point are the ones next to it.
pub const SHEET_REACH: usize = 1;

impl InferenceLimits {
    /// What the search is limited to unless it is said otherwise.
    pub const DEFAULT: Self = Self {
        enabled: true,
        generator_ms: DEFAULT_GENERATOR_MS,
        max_dimension: 7,
        narrow_to_sheet: false,
    };

    /// No limits at all: what the search did before there were any.
    pub const UNLIMITED: Self = Self {
        enabled: true,
        generator_ms: 0,
        max_dimension: u32::MAX,
        narrow_to_sheet: false,
    };

    /// How long one generator may take, if there is a limit.
    #[must_use]
    pub fn generator(self) -> Option<Duration> {
        (self.generator_ms > 0).then(|| Duration::from_millis(u64::from(self.generator_ms)))
    }
}

impl Default for InferenceLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// What came of inferring instances in a search, for telling the person when it did not go as
/// they might have thought.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceReport {
    /// The dimension of the diagram, if instances were not inferred because it is more than the
    /// limit ([`InferenceLimits::max_dimension`]) allows.
    pub too_high: Option<u32>,
    /// The names of the schemas that the inference gave up on because one took longer than the
    /// limit for a generator ([`InferenceLimits::generator_ms`]). What it found before that is
    /// among the options.
    pub cut_short: Vec<String>,
    /// The names of the schemas that were not looked at because the search was ended from
    /// outside before it got to them (see [`forced`](Self::forced)).
    pub not_tried: Vec<String>,
    /// How many schemas the inference went through to the end (the others are in `cut_short`).
    #[serde(default)]
    pub inferred: u32,
    /// Whether the search was for one generator, asked for by name (dropped on the diagram or
    /// picked), and not the search for everything at a point.
    #[serde(default)]
    pub targeted: bool,
    /// How long the inference took in all, and how long one generator was allowed to take.
    pub spent_ms: u64,
    pub budget_ms: u32,
    /// Whether the search did not stop when the time for a generator was up and had to be ended
    /// from outside (see [`Job::checkpoint`]): what is offered is what it had before the
    /// generator it was in the middle of.
    #[serde(default)]
    pub forced: bool,
    /// The longest a single step of the inference took, in milliseconds, for judging how well
    /// the time limit is kept.
    #[serde(default)]
    pub longest_step_ms: u64,
    /// If the replacements of placeholders were only looked for among the cells near the point
    /// ([`InferenceLimits::narrow_to_sheet`]): how many of the diagram's generators those were,
    /// and how many it has. Instances that need other cells are not found then.
    #[serde(default)]
    pub narrowed: Option<Narrowed>,
    /// What came of the time limit on the plain passes.
    #[serde(default)]
    pub plain: PlainReport,
}

/// What came of the time limit for a generator in the plain passes of a search (see
/// [`InferenceLimits::generator_ms`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlainReport {
    /// The names of the generators whose search was cut short: what it found is among the
    /// options, and is among what the whole search finds for the generator, in the same order.
    pub cut_short: Vec<String>,
    /// The names of the generators that were not looked at because the search was ended from
    /// outside before it got to them.
    pub not_tried: Vec<String>,
    /// How long the plain passes took in all, and how long one generator was allowed to take.
    pub spent_ms: u64,
    pub budget_ms: u32,
}

impl PlainReport {
    /// Whether the time ran out, for a generator or for all of them.
    #[must_use]
    pub const fn timed_out(&self) -> bool {
        !self.cut_short.is_empty() || !self.not_tried.is_empty()
    }
}

/// How much the pool of cells for the replacements of placeholders was narrowed to the cells
/// near the point (see [`InferenceLimits::narrow_to_sheet`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Narrowed {
    pub kept: u32,
    pub of: u32,
}

impl InferenceReport {
    /// Whether the time ran out, for a schema or for all of them.
    #[must_use]
    pub const fn timed_out(&self) -> bool {
        !self.cut_short.is_empty() || !self.not_tried.is_empty()
    }
}

/// Where an option comes in the final list of the search.
///
/// That is the index of its generator, and the
/// part of the search for the generator that it is from (see `Pass::rank`). The final list is
/// the options sorted by this (stably: the options of one slot are in the order that they were
/// found), which is what makes it possible to show the options as they are found without ever
/// reordering what is shown: an option goes to its slot, and the slots are in their order whatever
/// the order that the generators are searched in.
///
/// The instances of a schema are inferred in the order that the candidates for its placeholders
/// are tried in, and not in the order that they come out in the final list (the instances are
/// sorted by the numbers of the candidates chosen, placeholder by placeholder), so those carry
/// that sort key as well: an instance goes to its place among the instances found before it, in
/// the middle of the list if need be, and the options of any one slot are in the order found.
/// For every other option the key is empty.
///
/// A slot is only ever filled by one source, in order (the generators are searched one after the
/// other, and a search split over several workers would give each its own generators).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Slot {
    pub generator: u32,
    pub pass: u8,
    /// The candidates chosen for the placeholders of the schema, for an inferred instance.
    #[serde(default)]
    pub key: Vec<u32>,
}

/// An option as the search finds it, with its place in the final list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub slot: Slot,
    pub option: AttachOption,
}

/// The options found so far, from however many places, in the order that the final list will be
/// in. Adding options never moves the ones that are there: they only get further apart.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Streamed {
    slots: std::collections::BTreeMap<Slot, Vec<AttachOption>>,
    len: usize,
}

impl Streamed {
    pub fn extend(&mut self, found: impl IntoIterator<Item = Found>) {
        for Found { slot, option } in found {
            self.slots.entry(slot).or_default().push(option);
            self.len += 1;
        }
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The options, in the order of the final list.
    #[must_use]
    pub fn options(&self) -> Vec<AttachOption> {
        self.slots.values().flatten().cloned().collect()
    }

    #[must_use]
    pub fn into_options(self) -> Vec<AttachOption> {
        self.slots.into_values().flatten().collect()
    }
}

/// The ways to attach, and what there is to say about how they were found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachOutcome {
    pub options: Vec<AttachOption>,
    pub inference: InferenceReport,
}

/// The parts of the search for each generator, which are done one after the other for all of
/// them: what is cheap first, and the inference of instances, which is limited by the time it
/// takes, last. The options are in the order of the generators all the same.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Pass {
    /// The generator itself, its inverse and the cells raised from it.
    Plain,
    /// The generator reflected: duals, mates and so on.
    Reflections,
    /// Instances of a schema, inferred.
    Instances,
}

impl Pass {
    /// Where the options of the pass come after those of the generator in the list: as the
    /// generator, its instances and then its reflections, as it has always been.
    const fn rank(self) -> u8 {
        match self {
            Self::Plain => 0,
            Self::Instances => 1,
            Self::Reflections => 2,
        }
    }
}

/// The search for the [`AttachOption`]s at a point of the workspace diagram, a generator at a time
/// (and the inference of the instances of a schema a candidate at a time).
///
/// This is what happens on every click in the workspace, and the time it takes grows with the
/// size of the signature and the number of schemas that are tried (see
/// [`Signature::infer_instances`]), which is limited (see [`InferenceLimits`]).
#[derive(Debug)]
pub struct AttachOptionsJob {
    signature: Signature,
    generators: Vec<GeneratorInfo>,
    haystack: Diagram,
    /// The generators in the haystack, with their orientations, to see that a diagram that has
    /// cells that it does not have is not in it.
    present: FastHashMap<Generator, FastHashSet<Orientation>>,
    boundary: Boundary,
    boundary_path: Option<BoundaryPath>,
    point: Vec<Height>,
    weak_units: bool,
    limits: InferenceLimits,
    /// The options with their places in the final list (see [`Slot`]), in the order that they
    /// were found (they are only added to).
    matches: Vec<Found>,
    /// How many of `matches` have been handed out by [`take_found`](Self::take_found).
    reported: usize,
    /// The point as it was asked for, relative to what the workspace shows.
    asked_for: Vec<SliceIndex>,

    pass: Pass,
    /// The generators that the first pass looks at, cheapest first, and those that each of the
    /// passes after it is for.
    plain_order: Vec<usize>,
    to_reflect: Vec<usize>,
    to_infer: Vec<usize>,
    /// How far the pass is: the generator, or for the later passes the place in its list.
    next: usize,
    /// How many generators the passes are for in all, and how many they have done.
    planned: usize,
    done: usize,
    /// If the search is for the ways to attach one generator (the index of it), and not all of
    /// them: its pieces are then the steps (see [`Piece`]), so that how far along it is means
    /// something, and `Esc` is seen between them.
    only: Option<usize>,
    /// The pieces of the pass for the generator that is `only` searched for, once made.
    pieces: Option<VecDeque<Piece>>,
    /// The cells that placeholders are replaced with, which the searches share the composites of.
    cells: Rc<Cells>,
    inferring: Option<Inferring>,
    /// The time spent inferring in all, for the report.
    spent: Duration,
    /// The time spent in the plain passes in all, for the report.
    plain_spent: Duration,
    /// The time spent on each generator (by index) in the plain passes, their reflections and
    /// the inference together, which is what [`InferenceLimits::generator_ms`] limits.
    by_generator: Vec<Duration>,
    report: InferenceReport,
    started: bool,
    /// Whether the options so far have changed since the last [`checkpoint`](Job::checkpoint)
    /// and the inference is going.
    checkpoint_due: bool,
    longest_step: Duration,
}

/// A piece of the search for the ways to attach one generator: a cell made from it to look for,
/// or a reflection of it to typecheck and look for.
#[derive(Debug)]
enum Piece {
    Variant(Variant),
    Reflection(ReflectionCandidate),
}

/// A cell made from a generator to match against the diagram.
#[derive(Debug)]
struct Variant {
    diagram: DiagramN,
    structure: Option<Structure>,
}

/// A reflection of a generator that may match.
#[derive(Debug)]
struct ReflectionCandidate {
    directions: Vec<usize>,
    reflected: DiagramN,
}

#[derive(Debug)]
struct Inferring {
    generator: usize,
    search: InferSearch,
}

/// Where a click is in the diagram of the workspace: the diagram that is searched (the workspace's
/// or one of its boundaries) and the heights of the point in it.
#[derive(Clone, Debug)]
pub(super) struct Located {
    pub(super) haystack: Diagram,
    pub(super) boundary_path: Option<BoundaryPath>,
    pub(super) point: Vec<Height>,
}

impl Located {
    /// The place that a point (given relative to what the workspace shows) is at.
    pub(super) fn of(proof: &ProofState, point: &[SliceIndex]) -> Result<Self, NoAttachment> {
        let workspace = proof.workspace.as_ref().ok_or(NoAttachment::NoWorkspace)?;
        let point = {
            let mut point_with_path: Vec<SliceIndex> = workspace.path.iter().copied().collect();
            point_with_path.extend(point);
            point_with_path
        };
        let (boundary_path, point) = BoundaryPath::split(&point);
        let haystack = match boundary_path {
            None => workspace.diagram.clone(),
            Some(boundary_path) => DiagramN::try_from(workspace.diagram.clone())
                .ok()
                .and_then(|diagram| diagram.boundary(boundary_path))
                .ok_or(NoAttachment::NoBoundary)?,
        };
        Ok(Self {
            haystack,
            boundary_path,
            point,
        })
    }
}

impl AttachOptionsJob {
    /// The search for the ways to attach generators at a point of the diagram in the workspace
    /// (given relative to what it shows), also by way of weak units if `weak_units`.
    ///
    /// With `only`, it is for the ways to attach that generator, and what is found is what the
    /// whole search finds for it, in the same order: the generator itself, its inverse, the cells
    /// raised from it, its reflections and its instances if it is a schema. The search is then
    /// split into a step for each of those, and a schema that is the only one searched for has
    /// all the time that inferring is given.
    pub fn new(
        proof: &ProofState,
        point: &[SliceIndex],
        weak_units: bool,
        limits: InferenceLimits,
        only: Option<Generator>,
    ) -> Result<Self, NoAttachment> {
        let workspace = proof.workspace.as_ref().ok_or(NoAttachment::NoWorkspace)?;
        let asked_for = point.to_vec();
        let Located {
            haystack,
            boundary_path,
            point,
        } = Located::of(proof, point)?;
        let generators: Vec<GeneratorInfo> = proof.signature.iter().cloned().collect();
        let only = only
            .map(|generator| {
                generators
                    .iter()
                    .position(|info| info.generator == generator)
                    .ok_or(NoAttachment::UnknownGenerator)
            })
            .transpose()?;
        // Coordinates of the point beyond the ones the view shows are a sheet (and what is inside
        // it): all of the point is then what the cells for the replacements are to be near, if
        // that is asked for.
        let sheeted = asked_for.len() > usize::from(workspace.view.dimension());
        let all_generators = haystack.generators();
        let mut narrowed = None;
        let cells = if limits.narrow_to_sheet && sheeted && !point.is_empty() {
            let near = haystack.generators_near(&point, SHEET_REACH);
            narrowed = Some(Narrowed {
                kept: near.len() as u32,
                of: all_generators.len() as u32,
            });
            Pool::new(&proof.signature, near).cells()
        } else {
            Pool::new(&proof.signature, all_generators.keys().copied()).cells()
        };
        let dimension = haystack.dimension();

        let mut report = InferenceReport {
            budget_ms: limits.generator_ms,
            targeted: only.is_some(),
            ..InferenceReport::default()
        };
        let mut limits = limits;
        if limits.enabled && dimension > limits.max_dimension as usize {
            report.too_high = Some(dimension as u32);
            limits.enabled = false;
        }

        let mut job = Self {
            signature: proof.signature.clone(),
            generators,
            present: all_generators,
            haystack,
            boundary: boundary_path.map_or(Boundary::Target, BoundaryPath::boundary),
            boundary_path,
            point,
            weak_units,
            limits,
            matches: vec![],
            reported: 0,
            asked_for,
            only,
            pieces: None,
            pass: Pass::Plain,
            to_reflect: vec![],
            to_infer: vec![],
            next: 0,
            planned: 0,
            done: 0,
            cells,
            inferring: None,
            spent: Duration::ZERO,
            plain_spent: Duration::ZERO,
            by_generator: vec![],
            plain_order: vec![],
            report,
            started: false,
            checkpoint_due: false,
            longest_step: Duration::ZERO,
        };
        job.plain_order = job.cheapest_first((0..job.generators.len()).collect());
        job.to_reflect = {
            let reflecting = job.reflecting().map(|(index, _)| index).collect();
            job.cheapest_first(reflecting)
        };
        job.by_generator = vec![Duration::ZERO; job.generators.len()];
        job.report.plain.budget_ms = limits.generator_ms;
        // The first checkpoint, before anything is done, gives whoever runs this somewhere that
        // it can be ended something to fall back on from the start (see `checkpoint`).
        job.checkpoint_due = limits.generator().is_some();
        job.to_infer = job.inferring_for().map(|(index, _)| index).collect();
        // Only said when there are instances that it could have made a difference to.
        job.report.narrowed = narrowed.filter(|_| !job.to_infer.is_empty());
        // For one generator the pieces make up the steps, and are counted when they are made.
        job.planned = if only.is_some() {
            1 + job.to_reflect.len() + job.to_infer.len()
        } else {
            job.generators.len() + job.to_reflect.len() + job.to_infer.len()
        };
        Ok(job)
    }

    /// Narrows the search to the plain pass: the generators themselves, their inverses and the
    /// cells raised from them, with no reflections and no inference of instances. What it finds
    /// is among what the whole search finds, in the same order (the passes are the same, there
    /// are fewer of them). For a search that is done in the page and is to stay cheap (see
    /// [`DefinitionsJob`](super::DefinitionsJob)), so it is not offered to a worker, and has to be
    /// called before the first step.
    #[must_use]
    pub fn plain_only(mut self) -> Self {
        debug_assert!(!self.started);
        self.to_reflect.clear();
        self.to_infer.clear();
        self.limits.enabled = false;
        self.report.too_high = None;
        self.report.narrowed = None;
        self.planned = if self.only.is_some() {
            1
        } else {
            self.generators.len()
        };
        self
    }

    /// The work to do to do the search somewhere else (see [`remote`](crate::remote)), on the
    /// proof it was made from. `None` if the search has begun.
    #[must_use]
    pub fn offer(&self) -> Option<Work> {
        (!self.started).then(|| Work::AttachOptions {
            point: self.asked_for.clone(),
            weak_units: self.weak_units,
            limits: self.requested_limits(),
            only: self.only.map(|index| self.generators[index].generator),
        })
    }

    /// The limits the job was asked for, which the worker is to search with.
    const fn requested_limits(&self) -> InferenceLimits {
        InferenceLimits {
            enabled: self.limits.enabled || self.report.too_high.is_some(),
            ..self.limits
        }
    }

    /// Whether the search can take long enough to be worth doing somewhere else (see
    /// [`remote`](crate::remote)): when there are schemas to infer instances of (see
    /// [`Signature::infer_instances`]), or cells to reflect in a diagram of a dimension that has
    /// many reflections (`2^n`).
    #[must_use]
    pub fn may_take_long(&self) -> bool {
        let schemas = self.limits.enabled
            && self
                .generators
                .iter()
                .enumerate()
                .filter(|(index, _)| self.only.is_none_or(|only| only == *index))
                .any(|(_, info)| info.placeholder || !info.depends_on.is_empty());
        schemas || (self.haystack.dimension() >= 4 && !self.to_reflect.is_empty())
    }

    /// The generators in the order that the plain passes look at them: the cheapest first, so
    /// that when the time runs out it is the generators that are costly to look at that are left.
    /// The cost is a guess: a generator none of whose ends can be in the diagram is nothing, and
    /// the rest cost as much as their diagrams are big. The options are put back in the order
    /// of the generators (see [`outcome`](Self::outcome)), so the order of the search does not
    /// show in the answer.
    fn cheapest_first(&self, mut indices: Vec<usize>) -> Vec<usize> {
        let cost = |index: usize| {
            let info = &self.generators[index];
            let present = match &info.diagram {
                Diagram::Diagram0(_) => self.could_be_in_haystack(&info.diagram),
                Diagram::DiagramN(d) => {
                    self.could_be_in_haystack(&d.source()) || self.could_be_in_haystack(&d.target())
                }
            };
            (present, homotopy_core::layout::weight(&info.diagram, 4096))
        };
        indices.sort_by_cached_key(|&index| cost(index));
        indices
    }

    /// How long the plain passes took, in milliseconds, if they have a limit to say it against
    /// (without one it is not said, so that the same search always has the same report).
    fn plain_spent_ms(&self) -> u64 {
        if self.limits.generator().is_some() {
            self.plain_spent.as_millis() as u64
        } else {
            0
        }
    }

    /// How much of its time a generator has left: `None` if there is no limit.
    fn time_left(&self, index: usize) -> Option<Duration> {
        let each = self.limits.generator()?;
        Some(each.saturating_sub(self.by_generator[index]))
    }

    /// Does a plain step for a generator within the time that it has: `None` if it was not done,
    /// because it has none left (the passes before used it up, and the generator is said to have
    /// been given up on); otherwise what `work` came to, after a note that the generator was cut
    /// short if the time ran out in the middle of it.
    fn plain_step<R>(&mut self, index: usize, work: impl FnOnce(&mut Self) -> R) -> Option<R> {
        let left = self.time_left(index);
        if left.is_some_and(|left| left.is_zero()) {
            self.note_plain(index);
            return None;
        }
        let started = Instant::now();
        let limit = left.map(|left| deadline::until(started + left));
        let found = work(self);
        let cut = limit.as_ref().is_some_and(deadline::Limit::expired);
        drop(limit);
        let took = started.elapsed();
        self.plain_spent += took;
        self.by_generator[index] += took;
        if cut {
            self.note_plain(index);
            // What it was cut at is the end of its time (though the clock may say there is a
            // little left).
            if let Some(each) = self.limits.generator() {
                self.by_generator[index] = self.by_generator[index].max(each);
            }
        }
        self.checkpoint_due = self.limits.generator().is_some();
        Some(found)
    }

    /// Records that the plain search for a generator was cut short.
    fn note_plain(&mut self, index: usize) {
        let name = self.generators[index].name.clone();
        let plain = &mut self.report.plain;
        if !plain.cut_short.contains(&name) {
            plain.cut_short.push(name);
        }
    }

    /// The generators that are of the dimension to attach, and cells.
    fn eligible(&self) -> impl Iterator<Item = (usize, &GeneratorInfo)> {
        let dimension = self.haystack.dimension() + 1;
        self.generators
            .iter()
            .enumerate()
            .filter(move |(index, info)| {
                info.generator.dimension == dimension && self.only.is_none_or(|only| only == *index)
            })
    }

    /// The generators to reflect: those that are cells (to attach, not raised).
    fn reflecting(&self) -> impl Iterator<Item = (usize, &GeneratorInfo)> {
        self.eligible()
            .filter(|(_, info)| matches!(info.diagram, Diagram::DiagramN(_)))
    }

    /// The generators to infer the instances of.
    fn inferring_for(&self) -> impl Iterator<Item = (usize, &GeneratorInfo)> {
        let enabled = self.limits.enabled;
        self.reflecting().filter(move |(_, info)| {
            enabled && (info.placeholder || !self.signature.parameters(info.generator).is_empty())
        })
    }

    /// Whether a diagram can be in the haystack: all its cells are.
    fn could_be_in_haystack(&self, diagram: &Diagram) -> bool {
        diagram
            .generators()
            .keys()
            .all(|generator| self.present.contains_key(generator))
    }

    /// The options for attaching a diagram at the point, by every embedding of its slice to
    /// attach along that contains the point.
    fn options(
        &self,
        info: &GeneratorInfo,
        diagram: &DiagramN,
        structure: Option<&Structure>,
    ) -> Vec<AttachOption> {
        let needle = diagram.slice(self.boundary.flip()).unwrap();
        if !self.could_be_in_haystack(&needle) {
            return vec![];
        }
        self.haystack
            .embeddings_through(&needle, &self.point)
            .map(|embedding| AttachOption {
                generator: info.generator,
                diagram: diagram.clone(),
                label: structure.map(|structure| structure.label(&info.name)),
                boundary_path: self.boundary_path,
                embedding: embedding.into_iter().collect(),
                substitution: None,
            })
            .collect()
    }

    /// The cells to look for to attach the generator itself, its inverse and cells raised from
    /// it, in the order that the options are in.
    fn plain_variants(&self, index: usize) -> Vec<Variant> {
        let info = &self.generators[index];
        let haystack = &self.haystack;
        let weak_units = self.weak_units;
        let bubble = |mut diagram: DiagramN| {
            while diagram.dimension() < haystack.dimension() + 1 {
                diagram = diagram.bubble().unwrap();
            }
            diagram
        };
        let weak_unit = |mut diagram: Diagram| {
            while diagram.dimension() < haystack.dimension() + 1 {
                diagram = diagram.weak_identity().into();
            }
            DiagramN::try_from(diagram).unwrap()
        };
        let variant =
            |diagram: DiagramN, structure: Option<Structure>| Variant { diagram, structure };

        let mut variants = vec![];
        match info.generator.dimension.cmp(&(haystack.dimension() + 1)) {
            std::cmp::Ordering::Less => {
                // How many dimensions the cell is raised by.
                let k = haystack.dimension() + 1 - info.generator.dimension;
                if weak_units {
                    variants.push(variant(
                        weak_unit(info.diagram.clone()),
                        Some(Structure::Identity(k)),
                    ));
                }

                if let Diagram::DiagramN(d) = &info.diagram {
                    if info.invertibility.is_invertible() {
                        if weak_units {
                            variants.push(variant(
                                weak_unit(d.inverse().into()),
                                Some(Structure::InverseIdentity(k)),
                            ));
                        }

                        variants.push(variant(bubble(d.clone()), Some(Structure::Bubble(k))));
                        variants.push(variant(
                            bubble(d.inverse()),
                            Some(Structure::InverseBubble(k)),
                        ));
                    }
                }
            }
            std::cmp::Ordering::Equal => {
                if let Diagram::DiagramN(d) = &info.diagram {
                    variants.push(variant(d.clone(), None));
                    if info.invertibility.is_invertible() {
                        variants.push(variant(d.inverse(), Some(Structure::Inverse)));
                    }
                }
            }
            std::cmp::Ordering::Greater => (),
        }
        variants
    }

    /// The ways to attach a cell made from the generator.
    fn variant_options(&self, index: usize, variant: &Variant) -> Vec<AttachOption> {
        self.options(
            &self.generators[index],
            &variant.diagram,
            variant.structure.as_ref(),
        )
    }

    /// Finds the ways to attach the generator itself, its inverse and cells raised from it.
    fn plain(&mut self, index: usize) {
        let mut found = vec![];
        for variant in self.plain_variants(index) {
            if deadline::expired() {
                break;
            }
            found.extend(self.variant_options(index, &variant));
        }
        self.found(index, Pass::Plain, found);
    }

    /// Adds options found for a generator in a pass.
    fn found(&mut self, index: usize, pass: Pass, options: Vec<AttachOption>) {
        self.matches.extend(options.into_iter().map(|option| Found {
            slot: Slot {
                generator: index as u32,
                pass: pass.rank(),
                key: vec![],
            },
            option,
        }));
    }

    /// The reflections of the generator that could match, to typecheck and look for.
    fn reflection_candidates(&self, index: usize) -> Vec<ReflectionCandidate> {
        let info = &self.generators[index];
        let Diagram::DiagramN(d) = &info.diagram else {
            return vec![];
        };
        reflection_candidates(
            &self.signature,
            info.invertibility.is_invertible(),
            d,
            &self.present,
            self.boundary,
        )
    }

    /// The ways to attach a reflection of the generator, if it typechecks.
    fn reflection_options(
        &self,
        index: usize,
        candidate: &ReflectionCandidate,
    ) -> Vec<AttachOption> {
        let info = &self.generators[index];
        let Diagram::DiagramN(d) = &info.diagram else {
            return vec![];
        };
        reflection_options(
            &self.signature,
            info,
            d,
            candidate,
            &self.haystack,
            &self.present,
            &self.point,
            self.boundary,
            self.boundary_path,
        )
    }

    /// Finds the ways to attach the reflections of the generator.
    fn reflections(&mut self, index: usize) {
        let mut found = vec![];
        for candidate in self.reflection_candidates(index) {
            if deadline::expired() {
                break;
            }
            found.extend(self.reflection_options(index, &candidate));
        }
        self.found(index, Pass::Reflections, found);
    }

    /// Does a piece of the search for the one generator that is searched for: makes the pieces of
    /// the pass the first time, and then one of them at a time. `true` when the pass is over.
    fn piece(&mut self, index: usize, pass: Pass) -> bool {
        let Some(pieces) = &mut self.pieces else {
            let pieces: VecDeque<Piece> = match pass {
                Pass::Plain => self
                    .plain_variants(index)
                    .into_iter()
                    .map(Piece::Variant)
                    .collect(),
                _ => self
                    .reflection_candidates(index)
                    .into_iter()
                    .map(Piece::Reflection)
                    .collect(),
            };
            // The one unit that stood for the pass is now as many as there are pieces.
            self.planned = (self.planned + pieces.len()).saturating_sub(1);
            self.pieces = Some(pieces);
            return false;
        };
        let Some(piece) = pieces.pop_front() else {
            self.pieces = None;
            return true;
        };
        let found = match &piece {
            Piece::Variant(variant) => self.variant_options(index, variant),
            Piece::Reflection(candidate) => self.reflection_options(index, candidate),
        };
        self.found(index, pass, found);
        self.done += 1;
        false
    }

    /// Does a little of the inference of instances of schemas: the next candidate of the search
    /// that is going, or the start of the next one. `true` when it is over.
    ///
    /// A step does not run past the time that the schema has left: the search looks at the clock
    /// as it goes (see [`deadline`]), so that one candidate list or one check that would take
    /// long is cut short, and the schema is given up on with what it had found. The time is the
    /// schema's own (see [`InferenceLimits::generator_ms`]): one that has no end to it does not
    /// use the time of those after it.
    fn infer(&mut self) -> bool {
        let started = Instant::now();

        if self.inferring.is_none() {
            let Some(&index) = self.to_infer.get(self.next) else {
                return true;
            };
            let generator = self.generators[index].generator;
            self.next += 1;
            let left = self.time_left(index);
            if left.is_some_and(|left| left.is_zero()) {
                // The matching of the schema used up its time (and said so): there is none for
                // its instances.
                self.done += 1;
                self.checkpoint_due = true;
                return false;
            }
            let _limit = left.map(|left| deadline::until(started + left));
            self.inferring = InferSearch::new(
                &self.signature,
                Rc::clone(&self.cells),
                generator,
                &self.haystack,
                self.boundary,
                &self.point,
            )
            .map(|search| Inferring {
                generator: index,
                search,
            });
            if self.inferring.is_none() {
                self.done += 1;
                self.checkpoint_due = true;
            }
            let took = started.elapsed();
            self.spent += took;
            self.by_generator[index] += took;
            return false;
        }

        let index = self.inferring.as_ref().unwrap().generator;
        // The most this step may take: what is left of the time for the schema.
        let each = self.limits.generator();
        let left = self.time_left(index);
        let limit_now = left.map(|left| deadline::until(started + left));
        let over = self.inferring.as_mut().unwrap().search.step();
        let out_of_time = limit_now.as_ref().is_some_and(deadline::Limit::expired);
        drop(limit_now);
        // The instances this step found are options at once, each in its place among those
        // found before (see [`Slot`]), and not when the schema is done with: a schema that does
        // not finish in a reasonable time has listed what it found all the same.
        self.collect_instances(index);
        let took = started.elapsed();
        self.spent += took;
        self.by_generator[index] += took;
        self.longest_step = self.longest_step.max(took);
        // A search that was cut short by the clock is not over, whatever it says: it ran out of
        // what it was looking at.
        let cut =
            out_of_time || (!over && each.is_some_and(|each| self.by_generator[index] >= each));
        if over || cut {
            let Inferring { generator, .. } = self.inferring.take().unwrap();
            if cut {
                self.report
                    .cut_short
                    .push(self.generators[generator].name.clone());
            } else {
                self.report.inferred += 1;
            }
            self.done += 1;
            self.checkpoint_due = true;
        }
        false
    }

    /// Makes options of the instances that the inference of the schema `index` has found since
    /// the last time, and adds them to the options found.
    fn collect_instances(&mut self, index: usize) {
        let Some(inferring) = &mut self.inferring else {
            return;
        };
        let info = &self.generators[index];
        for (key, inferred) in inferring.search.take_results() {
            self.matches.push(Found {
                slot: Slot {
                    generator: index as u32,
                    pass: Pass::Instances.rank(),
                    key: key.into_iter().map(|candidate| candidate as u32).collect(),
                },
                option: AttachOption {
                    generator: info.generator,
                    label: Some(
                        Structure::Instance(
                            self.signature.describe_substitution(&inferred.substitution),
                        )
                        .label(&info.name),
                    ),
                    diagram: inferred.diagram,
                    boundary_path: self.boundary_path,
                    embedding: inferred.embedding.into_iter().collect(),
                    substitution: Some(inferred.substitution),
                },
            });
        }
    }

    /// Goes on to the inference of instances.
    const fn begin_instances(&mut self) {
        self.pass = Pass::Instances;
        self.next = 0;
        self.checkpoint_due = !self.to_infer.is_empty();
    }

    /// The options found since the last time this was asked, with their places in the final
    /// list (see [`Slot`]). Together they are, in the end, exactly the options of the outcome:
    /// putting them in their slots gives the same list in the same order.
    ///
    /// This is for showing the options while the search goes on; the search itself does not
    /// depend on it being asked.
    pub fn take_found(&mut self) -> Vec<Found> {
        let found = self.matches[self.reported..].to_vec();
        self.reported = self.matches.len();
        found
    }

    /// The options in the order of the final list: those of the generators in the order of the
    /// generators, each generator's in the order of the passes, the instances of a schema in
    /// the order of the candidates chosen for its placeholders, and the rest in the order
    /// they were found.
    fn sorted_options(&self) -> Vec<AttachOption> {
        let mut matches: Vec<&Found> = self.matches.iter().collect();
        matches.sort_by(|a, b| a.slot.cmp(&b.slot));
        matches
            .into_iter()
            .map(|found| found.option.clone())
            .collect()
    }

    fn outcome(&mut self) -> AttachOutcome {
        self.report.spent_ms = self.spent.as_millis() as u64;
        self.report.plain.spent_ms = self.plain_spent_ms();
        self.report.longest_step_ms = self.longest_step.as_millis() as u64;
        AttachOutcome {
            options: self.sorted_options(),
            inference: std::mem::take(&mut self.report),
        }
    }
}

impl Job for AttachOptionsJob {
    type Output = AttachOutcome;

    fn label(&self) -> &'static str {
        match self.pass {
            // (Not when there are no schemas to infer instances of, and what is left is to say so.)
            Pass::Instances if !self.to_infer.is_empty() => "Inferring instances",
            Pass::Instances | Pass::Plain | Pass::Reflections => "Searching for attachments",
        }
    }

    fn progress(&self) -> Option<Progress> {
        Some(Progress {
            done: self.done,
            total: self.planned,
        })
    }

    /// The time for one generator: a step is within one generator, and if it keeps its limits
    /// (the search looks at the clock as it goes, see [`deadline`]) it takes no more. For whoever
    /// runs the search somewhere that it can be ended from outside (a worker) to know when
    /// something that the search cannot see is holding it up.
    fn step_limit(&self) -> Option<Duration> {
        self.limits.generator()
    }

    /// What the search would come to if it were ended now, if that is different from the last
    /// time this was asked and it is in the inference of instances.
    ///
    /// The inference is the part of the search that has no end to it, and a single step of it
    /// can in principle go on for long: so where the search is done by something that can be
    /// ended from outside (a worker), this is the answer to fall back on if it has to be, with
    /// the schemas that are not done with, and the one it was in the middle of, reported as not
    /// finished.
    fn checkpoint(&mut self) -> Option<Self::Output> {
        if !self.checkpoint_due {
            return None;
        }
        self.checkpoint_due = false;
        let mut report = self.report.clone();
        report.forced = true;
        report.spent_ms = self.spent.as_millis() as u64;
        report.plain.spent_ms = self.plain_spent_ms();
        report.longest_step_ms = self.longest_step.as_millis() as u64;
        // The generator that is being looked at (or the next, which is about to be) is the one
        // that does not end, if anything is, and what comes after it was not looked at.
        let mut unfinished_plain: Vec<usize> = match self.pass {
            Pass::Plain => match self.only {
                Some(only) => vec![only],
                None => self
                    .plain_order
                    .iter()
                    .skip(self.next)
                    .chain(&self.to_reflect)
                    .copied()
                    .collect(),
            },
            Pass::Reflections => match self.only {
                Some(only) => vec![only],
                None => self.to_reflect.iter().skip(self.next).copied().collect(),
            },
            Pass::Instances => vec![],
        };
        unfinished_plain.dedup();
        let mut first = true;
        for index in unfinished_plain {
            let name = self.generators[index].name.clone();
            let plain = &mut report.plain;
            let list = if std::mem::take(&mut first) {
                &mut plain.cut_short
            } else {
                &mut plain.not_tried
            };
            if !list.contains(&name) {
                list.push(name);
            }
        }
        if self.pass == Pass::Instances {
            // The schema that is running (or the next, which is about to) is the one that does
            // not end, if anything is.
            let mut unfinished = self
                .inferring
                .iter()
                .map(|inferring| inferring.generator)
                .chain(self.to_infer.iter().skip(self.next).copied());
            if let Some(running) = unfinished.next() {
                report.cut_short.push(self.generators[running].name.clone());
            }
            report
                .not_tried
                .extend(unfinished.map(|index| self.generators[index].name.clone()));
        } else {
            // The inference has not begun, and what it would have looked at is not looked at:
            // that is part of what the plain passes did not get to, not a failure of the
            // inference to keep its time.
            for &index in &self.to_infer {
                let name = self.generators[index].name.clone();
                if !report.plain.not_tried.contains(&name) {
                    report.plain.not_tried.push(name);
                }
            }
        }
        Some(AttachOutcome {
            options: self.sorted_options(),
            inference: report,
        })
    }

    fn step(&mut self) -> Option<Self::Output> {
        self.started = true;
        match self.pass {
            Pass::Plain => {
                if let Some(index) = self.only {
                    let over = self
                        .plain_step(index, |job| job.piece(index, Pass::Plain))
                        // Out of time: what is left of the pass is not looked at.
                        .unwrap_or(true);
                    if over {
                        self.pieces = None;
                        self.pass = Pass::Reflections;
                        self.next = 0;
                    }
                } else if let Some(&index) = self.plain_order.get(self.next) {
                    self.plain_step(index, |job| job.plain(index));
                    self.next += 1;
                    self.done += 1;
                } else {
                    self.pass = Pass::Reflections;
                    self.next = 0;
                }
            }
            Pass::Reflections => {
                if let Some(index) = self.only {
                    let over = !self.to_reflect.contains(&index)
                        || self
                            .plain_step(index, |job| job.piece(index, Pass::Reflections))
                            .unwrap_or(true);
                    if over {
                        self.pieces = None;
                        self.begin_instances();
                    }
                } else if let Some(&index) = self.to_reflect.get(self.next) {
                    self.plain_step(index, |job| job.reflections(index));
                    self.next += 1;
                    self.done += 1;
                } else {
                    self.begin_instances();
                }
            }
            Pass::Instances => {
                if self.infer() {
                    return Some(self.outcome());
                }
            }
        }
        None
    }
}

/// The reflections of a generator other than its inverse that could match: duals, mates,
/// adjoints and so on. These are only valid when some of the generators involved are
/// invertible or dualisable, or when the signature has symmetries, and are typechecked only
/// once they are known to match (see [`reflection_options`]).
///
/// A reflection has the cells that the diagram has, in other directions, so none of them can be
/// in the haystack if the cells of neither end of the diagram are: which is nearly always, and
/// is found out before the (up to `2^n - 1`) reflections are made.
fn reflection_candidates(
    signature: &Signature,
    invertible: bool,
    diagram: &DiagramN,
    present: &FastHashMap<Generator, FastHashSet<Orientation>>,
    boundary: Boundary,
) -> Vec<ReflectionCandidate> {
    let any_invertible = Diagram::from(diagram.clone())
        .generators()
        .keys()
        .any(|g: &Generator| {
            signature
                .generator_info(*g)
                .is_some_and(|info| info.invertibility.is_invertible())
        });
    if !any_invertible && signature.symmetries().is_empty() {
        return vec![];
    }
    let in_haystack =
        |needle: &Diagram| needle.generators().keys().all(|g| present.contains_key(g));
    // The slice that a reflection is attached along is a reflection of one of the two ends.
    if ![boundary.flip(), boundary]
        .into_iter()
        .filter_map(|end| diagram.slice(end))
        .any(|end| in_haystack(&end))
    {
        return vec![];
    }

    // A reflection has the cells of one of the ends reflected, which flips their orientations
    // as the reflection says: those need to be in the haystack as well.
    let ends: Vec<_> = [boundary.flip(), boundary]
        .into_iter()
        .filter_map(|end| diagram.slice(end))
        .map(|end| end.generators())
        .collect();
    let could_embed = |directions: &[Direction]| {
        ends.iter().any(|end| {
            end.iter().all(|(generator, orientations)| {
                present.get(generator).is_some_and(|have| {
                    orientations.iter().all(|orientation| {
                        let reflected = Diagram0::new(*generator, *orientation).reflect(directions);
                        have.contains(&reflected.orientation)
                    })
                })
            })
        })
    };

    let n = diagram.dimension();
    diagram
        .reflections_where(could_embed)
        .into_iter()
        // (The reflection in the last direction alone is already offered as the inverse.)
        .filter(|(directions, _)| !(invertible && directions == &[n]))
        .map(|(directions, reflected)| ReflectionCandidate {
            directions,
            reflected,
        })
        .collect()
}

/// The ways to attach a reflection of a generator (see [`reflection_candidates`]), if it
/// typechecks. Reflections by a symmetry are tagged with its name.
#[allow(clippy::too_many_arguments)]
fn reflection_options(
    signature: &Signature,
    info: &GeneratorInfo,
    diagram: &DiagramN,
    candidate: &ReflectionCandidate,
    haystack: &Diagram,
    present: &FastHashMap<Generator, FastHashSet<Orientation>>,
    point: &[Height],
    boundary: Boundary,
    boundary_path: Option<BoundaryPath>,
) -> Vec<AttachOption> {
    let ReflectionCandidate {
        directions,
        reflected,
    } = candidate;
    let n = diagram.dimension();
    let needle = reflected.slice(boundary.flip()).unwrap();
    if !needle.generators().keys().all(|g| present.contains_key(g)) {
        return vec![];
    }
    let embeddings: Vec<_> = haystack.embeddings_through(&needle, point).collect();
    if embeddings.is_empty()
        || typecheck(&reflected.clone().into(), signature, Mode::default(), true).is_err()
    {
        return vec![];
    }
    let mask = directions.iter().fold(0_u64, |mask, j| mask | 1 << (j - 1));
    let structure = signature
        .symmetries()
        .iter()
        .find(|symmetry| symmetry.mask() & ((1 << n) - 1) == mask)
        .map_or_else(
            || Structure::Reflection(directions.clone()),
            |symmetry| Structure::Symmetry(symmetry.name.clone()),
        );
    let label = structure.label(&info.name);
    embeddings
        .into_iter()
        .map(|embedding| AttachOption {
            generator: info.generator,
            diagram: reflected.clone(),
            label: Some(label.clone()),
            boundary_path,
            embedding: embedding.into_iter().collect(),
            substitution: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use homotopy_common::deadline;

    use super::*;
    use crate::{
        history::Proof,
        job::{run_to_end, Job},
        proof::Action,
    };

    /// The state of an example with a workspace and the most generators.
    fn state() -> Proof {
        let json = include_str!("../../tests/examples/equivalence_to_adjoint_equivalence.json");
        let (_safe, actions): (bool, Vec<Action>) = serde_json::from_str(json).unwrap();
        let mut proof = Proof::default();
        let mut best: Option<Proof> = None;
        for action in actions {
            if proof.update(&action).is_err() {
                continue;
            }
            if proof.workspace.is_some()
                && best
                    .as_ref()
                    .is_none_or(|b| b.signature.iter().count() < proof.signature.iter().count())
            {
                best = Some(proof.clone());
            }
        }
        best.unwrap()
    }

    /// The time is each generator's own: one that uses all of its time is given up on, and
    /// the others still have all of theirs; there is no total to run out.
    #[test]
    fn a_generator_that_uses_its_time_does_not_use_the_time_of_the_next() {
        let state = state();
        let limits = InferenceLimits {
            generator_ms: 300,
            ..InferenceLimits::UNLIMITED
        };
        let mut job = AttachOptionsJob::new(&state, &[], true, limits, None).unwrap();
        assert!(job.generators.len() > 2);
        let (first, second) = (job.plain_order[0], job.plain_order[1]);
        let each = Duration::from_millis(300);
        assert_eq!(job.time_left(first), Some(each));

        // The first generator's search takes longer than its time, and the search sees the clock.
        let result = job.plain_step(first, |_| {
            std::thread::sleep(Duration::from_millis(350));
            (0..8).any(|_| deadline::expired())
        });
        assert_eq!(result, Some(true));
        assert_eq!(job.time_left(first), Some(Duration::ZERO));
        assert_eq!(job.time_left(second), Some(each));
        let name = job.generators[first].name.clone();
        let job_name = name.clone();
        assert_eq!(job.report.plain.cut_short, vec![name]);
        // It has no time for its reflections either, and is not done again.
        assert!(job.plain_step(first, |_| ()).is_none());

        // The rest of the search still looks at every other generator.
        let outcome = run_to_end(job);
        assert!(outcome.inference.plain.not_tried.is_empty());
        assert!(outcome.inference.plain.cut_short.contains(&job_name));
    }

    /// Without a limit a generator has all the time there is, and nothing is cut.
    #[test]
    fn without_a_limit_there_is_no_time_to_run_out_of() {
        let state = state();
        let mut job =
            AttachOptionsJob::new(&state, &[], true, InferenceLimits::UNLIMITED, None).unwrap();
        assert_eq!(job.time_left(0), None);
        assert_eq!(job.step_limit(), None);
        assert!(job.plain_step(0, |_| ()).is_some());
        assert!(!run_to_end(job).inference.plain.timed_out());
    }
}
