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

/// How much to spend on inferring the instances of schemas that can be attached.
///
/// That (see [`Signature::infer_instances`]) is the part of the search that can take minutes
/// where everything else takes milliseconds: the search for the replacements of placeholders
/// grows with every placeholder a schema has, and the cells there are to replace them with.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceLimits {
    /// Whether to infer instances at all. If not, schemas are not offered for attaching, but
    /// everything else is.
    pub enabled: bool,
    /// The most time, in milliseconds, that all of the inference of one search takes, after
    /// which the schemas that are left are not looked at (and the one being looked at is left
    /// as it is: what it had found is offered). `0` for no limit. A single schema gets a
    /// quarter of it, but at least a second, so that one schema that has no end to it does
    /// not use up the time of the rest.
    pub budget_ms: u32,
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

/// How far from the point a cell may be to be in the narrowed pool.
///
/// In heights of each coordinate, when [`InferenceLimits::narrow_to_sheet`] is in effect: the
/// heights next to the point's are included, as the cells meeting at a point are the ones next to it.
pub const SHEET_REACH: usize = 1;

impl InferenceLimits {
    /// What the search is limited to unless it is said otherwise.
    pub const DEFAULT: Self = Self {
        enabled: true,
        budget_ms: 20_000,
        max_dimension: 7,
        narrow_to_sheet: false,
    };

    /// No limits at all: what the search did before there were any.
    pub const UNLIMITED: Self = Self {
        enabled: true,
        budget_ms: 0,
        max_dimension: u32::MAX,
        narrow_to_sheet: false,
    };

    fn budget(self) -> Option<Duration> {
        (self.budget_ms > 0).then(|| Duration::from_millis(u64::from(self.budget_ms)))
    }

    /// How long one schema may take, when there are `schemas` of them to infer instances of: a
    /// quarter of the budget, but at least a second, so that one schema that has no end to it
    /// does not use up the time of the rest. A schema that is the only one has all of it.
    pub(super) fn budget_each(self, schemas: usize) -> Option<Duration> {
        self.budget().map(|total| {
            if schemas <= 1 {
                total
            } else {
                (total / 4).max(total.min(Duration::from_secs(1)))
            }
        })
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
    /// The names of the schemas that the inference gave up on because it took too long. What it
    /// found before that is among the options.
    pub cut_short: Vec<String>,
    /// The names of the schemas that were not looked at because the time was used up.
    pub not_tried: Vec<String>,
    /// How long the inference took, and how long it was allowed to.
    pub spent_ms: u64,
    pub budget_ms: u32,
    /// Whether the search did not stop when the time was up and had to be ended from outside
    /// (see [`Job::checkpoint`]): what is offered is what it had before the
    /// schema it was in the middle of.
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
    /// The options with the generator they are for and their part of the search.
    matches: Vec<(usize, u8, AttachOption)>,
    /// The point as it was asked for, relative to what the workspace shows.
    asked_for: Vec<SliceIndex>,

    pass: Pass,
    /// The generators that each of the passes after the first is for.
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
    /// The time spent inferring, and whether it is all used.
    spent: Duration,
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
    spent: Duration,
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
            budget_ms: limits.budget_ms,
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
            report,
            started: false,
            checkpoint_due: false,
            longest_step: Duration::ZERO,
        };
        job.to_reflect = job.reflecting().map(|(index, _)| index).collect();
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
        let found: Vec<_> = self
            .plain_variants(index)
            .iter()
            .flat_map(|variant| self.variant_options(index, variant))
            .collect();
        self.matches.extend(
            found
                .into_iter()
                .map(|option| (index, Pass::Plain.rank(), option)),
        );
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
        let found: Vec<_> = self
            .reflection_candidates(index)
            .iter()
            .flat_map(|candidate| self.reflection_options(index, candidate))
            .collect();
        self.matches.extend(
            found
                .into_iter()
                .map(|option| (index, Pass::Reflections.rank(), option)),
        );
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
        self.matches
            .extend(found.into_iter().map(|option| (index, pass.rank(), option)));
        self.done += 1;
        false
    }

    /// Does a little of the inference of instances of schemas: the next candidate of the search
    /// that is going, or the start of the next one. `true` when it is over.
    ///
    /// A step does not run past the time that is left: the search looks at the clock as it goes
    /// (see [`deadline`]), so that one candidate list or one check that would take long is cut
    /// short, and the schema is given up on with what it had found.
    fn infer(&mut self) -> bool {
        let limit = self.limits.budget();
        let limit_each = self.limits.budget_each(self.to_infer.len());
        let started = Instant::now();

        if self.inferring.is_none() {
            let Some(&index) = self.to_infer.get(self.next) else {
                return true;
            };
            let (name, generator) = {
                let info = &self.generators[index];
                (info.name.clone(), info.generator)
            };
            self.next += 1;
            if limit.is_some_and(|limit| self.spent >= limit) {
                self.report.not_tried.push(name);
                self.done += 1;
                self.checkpoint_due = true;
                return false;
            }
            let time_up = limit.map(|limit| started + limit.saturating_sub(self.spent));
            let _limit = time_up.map(deadline::until);
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
                spent: Duration::ZERO,
            });
            if self.inferring.is_none() {
                self.done += 1;
                self.checkpoint_due = true;
            }
            self.spent += started.elapsed();
            return false;
        }

        let inferring = self.inferring.as_mut().unwrap();
        // The most this step may take: what is left of the time for the schema and for all.
        let left = [
            limit_each.map(|each| each.saturating_sub(inferring.spent)),
            limit.map(|limit| limit.saturating_sub(self.spent)),
        ]
        .into_iter()
        .flatten()
        .min();
        let limit_now = left.map(|left| deadline::until(started + left));
        let over = inferring.search.step();
        let out_of_time = limit_now.as_ref().is_some_and(deadline::Limit::expired);
        drop(limit_now);
        let took = started.elapsed();
        inferring.spent += took;
        self.spent += took;
        self.longest_step = self.longest_step.max(took);
        // A search that was cut short by the clock is not over, whatever it says: it ran out of
        // what it was looking at.
        let cut = out_of_time
            || (!over
                && (limit_each.is_some_and(|limit| inferring.spent >= limit)
                    || limit.is_some_and(|limit| self.spent >= limit)));
        if over || cut {
            let Inferring {
                generator, search, ..
            } = self.inferring.take().unwrap();
            let info = self.generators[generator].clone();
            if cut {
                self.report.cut_short.push(info.name.clone());
            }
            for inferred in search.finish() {
                self.matches.push((
                    generator,
                    Pass::Instances.rank(),
                    AttachOption {
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
                ));
            }
            self.done += 1;
            self.checkpoint_due = true;
        }
        false
    }

    /// The most time that the inference of instances can take, if it is going to happen and has
    /// a limit: for whoever runs the search somewhere that it can be ended from outside (see
    /// [`checkpoint`](Job::checkpoint)) to know when to.
    #[must_use]
    pub fn inference_budget(&self) -> Option<Duration> {
        if self.to_infer.is_empty() {
            return None;
        }
        self.limits.budget()
    }

    /// Goes on to the inference of instances.
    const fn begin_instances(&mut self) {
        self.pass = Pass::Instances;
        self.next = 0;
        self.checkpoint_due = !self.to_infer.is_empty();
    }

    fn outcome(&mut self) -> AttachOutcome {
        let mut matches = std::mem::take(&mut self.matches);
        matches.sort_by_key(|(index, rank, _)| (*index, *rank));
        self.report.spent_ms = self.spent.as_millis() as u64;
        self.report.longest_step_ms = self.longest_step.as_millis() as u64;
        AttachOutcome {
            options: matches.into_iter().map(|(_, _, option)| option).collect(),
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

    /// What the search would come to if it were ended now, if that is different from the last
    /// time this was asked and it is in the inference of instances.
    ///
    /// The inference is the part of the search that has no end to it, and a single step of it
    /// can in principle go on for long: so where the search is done by something that can be
    /// ended from outside (a worker), this is the answer to fall back on if it has to be, with
    /// the schemas that are not done with, and the one it was in the middle of, reported as not
    /// finished.
    fn checkpoint(&mut self) -> Option<Self::Output> {
        if self.pass != Pass::Instances || !self.checkpoint_due {
            return None;
        }
        self.checkpoint_due = false;
        let mut matches = self.matches.clone();
        matches.sort_by_key(|(index, rank, _)| (*index, *rank));
        let mut report = self.report.clone();
        report.forced = true;
        report.spent_ms = self.spent.as_millis() as u64;
        report.longest_step_ms = self.longest_step.as_millis() as u64;
        // The schema that is running (or the next, which is about to) is the one that does not
        // end, if anything is.
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
        Some(AttachOutcome {
            options: matches.into_iter().map(|(_, _, option)| option).collect(),
            inference: report,
        })
    }

    fn step(&mut self) -> Option<Self::Output> {
        self.started = true;
        match self.pass {
            Pass::Plain => {
                if let Some(index) = self.only {
                    if self.piece(index, Pass::Plain) {
                        self.pass = Pass::Reflections;
                        self.next = 0;
                    }
                } else if self.next < self.generators.len() {
                    self.plain(self.next);
                    self.next += 1;
                    self.done += 1;
                } else {
                    self.pass = Pass::Reflections;
                    self.next = 0;
                }
            }
            Pass::Reflections => {
                if let Some(index) = self.only {
                    if !self.to_reflect.contains(&index) || self.piece(index, Pass::Reflections) {
                        self.begin_instances();
                    }
                } else if let Some(&index) = self.to_reflect.get(self.next) {
                    self.reflections(index);
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
