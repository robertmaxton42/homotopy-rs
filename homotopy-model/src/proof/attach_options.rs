//! Finding the ways to attach a generator at a point of the workspace diagram.

use std::{rc::Rc, time::Duration};

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
}

impl InferenceLimits {
    /// What the search is limited to unless it is said otherwise.
    pub const DEFAULT: Self = Self {
        enabled: true,
        budget_ms: 20_000,
        max_dimension: 7,
    };

    /// No limits at all: what the search did before there were any.
    pub const UNLIMITED: Self = Self {
        enabled: true,
        budget_ms: 0,
        max_dimension: u32::MAX,
    };

    fn budget(self) -> Option<Duration> {
        (self.budget_ms > 0).then(|| Duration::from_millis(u64::from(self.budget_ms)))
    }

    /// How long one schema may take, when there are `schemas` of them to infer instances of: a
    /// quarter of the budget, but at least a second, so that one schema that has no end to it
    /// does not use up the time of the rest. A schema that is the only one has all of it.
    fn budget_each(self, schemas: usize) -> Option<Duration> {
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

#[derive(Debug)]
struct Inferring {
    generator: usize,
    search: InferSearch,
    spent: Duration,
}

impl AttachOptionsJob {
    /// The search for the ways to attach generators at a point of the diagram in the workspace
    /// (given relative to what it shows), also by way of weak units if `weak_units`.
    pub fn new(
        proof: &ProofState,
        point: &[SliceIndex],
        weak_units: bool,
        limits: InferenceLimits,
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
        let cells = Pool::new(&proof.signature, haystack.generators().into_keys()).cells();
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
            present: haystack.generators(),
            haystack,
            boundary: boundary_path.map_or(Boundary::Target, BoundaryPath::boundary),
            boundary_path,
            point,
            weak_units,
            limits,
            matches: vec![],
            asked_for,
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
        job.planned = job.generators.len() + job.to_reflect.len() + job.to_infer.len();
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
                .any(|info| info.placeholder || !info.depends_on.is_empty());
        schemas || (self.haystack.dimension() >= 4 && !self.to_reflect.is_empty())
    }

    /// The generators that are of the dimension to attach, and cells.
    fn eligible(&self) -> impl Iterator<Item = (usize, &GeneratorInfo)> {
        let dimension = self.haystack.dimension() + 1;
        self.generators
            .iter()
            .enumerate()
            .filter(move |(_, info)| info.generator.dimension == dimension)
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

    /// Finds the ways to attach the generator itself, its inverse and cells raised from it.
    fn plain(&mut self, index: usize) {
        let info = self.generators[index].clone();
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

        let mut found = vec![];
        match info.generator.dimension.cmp(&(haystack.dimension() + 1)) {
            std::cmp::Ordering::Less => {
                // How many dimensions the cell is raised by.
                let k = haystack.dimension() + 1 - info.generator.dimension;
                if weak_units {
                    found.extend(self.options(
                        &info,
                        &weak_unit(info.diagram.clone()),
                        Some(&Structure::Identity(k)),
                    ));
                }

                if let Diagram::DiagramN(d) = &info.diagram {
                    if info.invertibility.is_invertible() {
                        if weak_units {
                            found.extend(self.options(
                                &info,
                                &weak_unit(d.inverse().into()),
                                Some(&Structure::InverseIdentity(k)),
                            ));
                        }

                        found.extend(self.options(
                            &info,
                            &bubble(d.clone()),
                            Some(&Structure::Bubble(k)),
                        ));
                        found.extend(self.options(
                            &info,
                            &bubble(d.inverse()),
                            Some(&Structure::InverseBubble(k)),
                        ));
                    }
                }
            }
            std::cmp::Ordering::Equal => {
                if let Diagram::DiagramN(d) = &info.diagram {
                    found.extend(self.options(&info, d, None));
                    if info.invertibility.is_invertible() {
                        found.extend(self.options(&info, &d.inverse(), Some(&Structure::Inverse)));
                    }
                }
            }
            std::cmp::Ordering::Greater => (),
        }
        self.matches.extend(
            found
                .into_iter()
                .map(|option| (index, Pass::Plain.rank(), option)),
        );
    }

    /// Finds the ways to attach the reflections of the generator.
    fn reflections(&mut self, index: usize) {
        let info = self.generators[index].clone();
        let Diagram::DiagramN(d) = &info.diagram else {
            return;
        };
        let found = extend_with_reflections(
            &self.signature,
            &info,
            d,
            &self.haystack,
            &self.present,
            &self.point,
            self.boundary,
            self.boundary_path,
        );
        self.matches.extend(
            found
                .into_iter()
                .map(|option| (index, Pass::Reflections.rank(), option)),
        );
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
            Pass::Instances => "Inferring instances",
            Pass::Plain | Pass::Reflections => "Searching for attachments",
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
                if self.next < self.generators.len() {
                    self.plain(self.next);
                    self.next += 1;
                    self.done += 1;
                } else {
                    self.pass = Pass::Reflections;
                    self.next = 0;
                }
            }
            Pass::Reflections => {
                if let Some(&index) = self.to_reflect.get(self.next) {
                    self.reflections(index);
                    self.next += 1;
                    self.done += 1;
                } else {
                    self.pass = Pass::Instances;
                    self.next = 0;
                    self.checkpoint_due = !self.to_infer.is_empty();
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

/// Offers the well-typed reflections of a generator other than its inverse: duals, mates,
/// adjoints and so on. These are only valid when some of the generators involved are
/// invertible or dualisable, or when the signature has symmetries, and are typechecked only
/// once they are known to match. Reflections by a symmetry are tagged with its name.
///
/// A reflection has the cells that the diagram has, in other directions, so none of them can be
/// in the haystack if the cells of neither end of the diagram are: which is nearly always, and
/// is found out before the (up to `2^n - 1`) reflections are made.
#[allow(clippy::too_many_arguments)]
fn extend_with_reflections(
    signature: &Signature,
    info: &GeneratorInfo,
    diagram: &DiagramN,
    haystack: &Diagram,
    present: &FastHashMap<Generator, FastHashSet<Orientation>>,
    point: &[Height],
    boundary: Boundary,
    boundary_path: Option<BoundaryPath>,
) -> Vec<AttachOption> {
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

    let mut matches = vec![];
    let n = diagram.dimension();
    for (directions, reflected) in diagram.reflections_where(could_embed) {
        if info.invertibility.is_invertible() && directions == [n] {
            // Already offered as the inverse.
            continue;
        }
        let needle = reflected.slice(boundary.flip()).unwrap();
        if !in_haystack(&needle) {
            continue;
        }
        let embeddings: Vec<_> = haystack.embeddings_through(&needle, point).collect();
        if embeddings.is_empty()
            || typecheck(&reflected.clone().into(), signature, Mode::default(), true).is_err()
        {
            continue;
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
        matches.extend(embeddings.into_iter().map(|embedding| AttachOption {
            generator: info.generator,
            diagram: reflected.clone(),
            label: Some(label.clone()),
            boundary_path,
            embedding: embedding.into_iter().collect(),
            substitution: None,
        }));
    }
    matches
}
