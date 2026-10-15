//! Definitions first: the cell that most likely follows a click on a cell.
//!
//! A click in the workspace looks for everything that can be attached at the point, which with a
//! big signature takes a while. When the click lands on one cell, though, what is wanted nearly
//! always is that cell's **definition**: a generator one dimension up that has the cell as its
//! source or its target, such as the unfolding that is made right after it (`Proof 63` after
//! `start`). So those are looked for first, by a search that is a handful of comparisons and
//! matches of cells, and offered at the top of the list while the rest of the search is one click
//! away. It is a shortcut: it is a heuristic in the sense of the policy in `CLAUDE.md` (it can
//! miss, never offers what does not attach, and says what it is).
//!
//! # What "lands on a cell" is
//!
//! The point of a click is a list of heights, one for each coordinate the view shows (and
//! those of a sheet, if one was chosen), see [`cell_at`]. It lands on the cell of a generator
//! `g` if
//!
//! 1. it is not empty and every height in it is singular (a vertex of the picture in each
//!    coordinate, not a wire or a region, which have no cell of their own to define), and
//! 2. the slice of the diagram at the point (the fibre: what is there for the coordinates that
//!    are not fixed by the point) has cells of the dimension of the diagram, which are all of
//!    one generator `g`.
//!
//! A position that has cells of several generators of that dimension (sheets of different cells
//! behind each other) is not one cell, and nor is one that has none (a vertex where only
//! lower-dimensional cells are, such as a crossing of wires): the click is then what it always was.
//! Cells of lower dimension than the diagram never count: a definition of one would be a cell that
//! has to be raised to be attached, which is not a definition of the cell in the diagram any more.
//!
//! # What a definition is
//!
//! A generator `d` of dimension `dim(g) + 1`, other than `g`, one of whose boundaries is exactly the
//! diagram of `g` (equal as diagrams, so it takes no search to see): `d : g ⇒ t`, or `d : s ⇒ g`
//! (which is attached as its inverse if it is invertible, as the search does). The generator
//! after `g` in the signature (`id + 1`) is looked at first, as definitions are made right after
//! the cell: then the others in the order of the signature. Each candidate is *verified* by the
//! plain search for attaching it (the generator itself and its inverse, in the same steps and with
//! the same matching as the full search, [`AttachOptionsJob::plain_only`]) so nothing that does not
//! attach at the click is offered, and what is offered is among what the whole search finds.
//!
//! Left out, and said so: instances of a schema are not inferred, only the instances that exist
//! as generators of the signature are candidates (an instance `d[σ]` of the definition of a
//! schema has the instance `g[σ]` as its boundary, so it is found like any other); reflections
//! of definitions (duals and the like) are not looked for either. Search everything for those.

use std::collections::VecDeque;

use homotopy_core::{common::Generator, signature::Signature as _, Diagram, Height, SliceIndex};

use super::{
    attach_options::Located, AttachOption, AttachOptionsJob, InferenceLimits, ProofState, Signature,
};
use crate::job::{Job, Progress};

/// The generator whose cell a point lands on, if it lands on one (see the [module](self)).
#[must_use]
pub fn cell_at(haystack: &Diagram, point: &[Height]) -> Option<Generator> {
    if point.is_empty()
        || point
            .iter()
            .any(|height| !matches!(height, Height::Singular(_)))
    {
        return None;
    }
    let dimension = haystack.dimension();
    let mut cells = haystack
        .generators_near(point, 0)
        .into_iter()
        .filter(|generator| generator.dimension == dimension);
    let cell = cells.next()?;
    cells.next().is_none().then_some(cell)
}

/// The generators that define `generator`: one dimension up, with its diagram as source or as
/// target. The one after it first, and then the rest in the order of the signature.
#[must_use]
pub fn definitions_of(signature: &Signature, generator: Generator) -> Vec<Generator> {
    let Some(cell) = signature.generator_info(generator) else {
        return vec![];
    };
    let dimension = generator.dimension + 1;
    let defines = |info: &super::generators::GeneratorInfo| match &info.diagram {
        Diagram::DiagramN(d) if info.generator.dimension == dimension => {
            d.source() == cell.diagram || d.target() == cell.diagram
        }
        _ => false,
    };
    let next = Generator::new(generator.id + 1, dimension);
    let mut found: Vec<Generator> = signature
        .generator_info(next)
        .filter(|info| defines(info))
        .map(|info| info.generator)
        .into_iter()
        .collect();
    found.extend(
        signature
            .iter()
            .filter(|info| info.generator != next && defines(info))
            .map(|info| info.generator),
    );
    found
}

/// What came of looking for definitions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefinitionsOutcome {
    /// The cell the click landed on.
    pub of: Generator,
    /// The ways to attach its definitions at the click, in the order of the candidates.
    pub options: Vec<AttachOption>,
    /// The names of the candidates whose search ran out of the time for a generator
    /// ([`InferenceLimits::generator_ms`]): what is offered of them is valid, and may be less.
    pub cut_short: Vec<String>,
}

/// The search for the definitions of the cell that a click lands on: a plain search for each
/// candidate in turn (see the [module](self)), a piece of one at a time.
#[derive(Debug)]
pub struct DefinitionsJob {
    proof: ProofState,
    point: Vec<SliceIndex>,
    weak_units: bool,
    limits: InferenceLimits,
    of: Generator,
    /// The candidates not looked at yet, and the search that is going.
    pending: VecDeque<Generator>,
    current: Option<AttachOptionsJob>,
    total: usize,
    done: usize,
    options: Vec<AttachOption>,
    cut_short: Vec<String>,
}

impl DefinitionsJob {
    /// The search for the definitions of the cell at a point (given relative to what the
    /// workspace shows), if the point lands on a cell that has any: `None` for any other click,
    /// which is searched for as it always was.
    #[must_use]
    pub fn new(
        proof: &ProofState,
        point: &[SliceIndex],
        weak_units: bool,
        limits: InferenceLimits,
    ) -> Option<Self> {
        let Located {
            haystack,
            point: heights,
            ..
        } = Located::of(proof, point).ok()?;
        let of = cell_at(&haystack, &heights)?;
        let candidates = definitions_of(&proof.signature, of);
        if candidates.is_empty() {
            return None;
        }
        Some(Self {
            proof: proof.clone(),
            point: point.to_vec(),
            weak_units,
            // (No instances are inferred: see the module.)
            limits: InferenceLimits {
                enabled: false,
                ..limits
            },
            of,
            total: candidates.len(),
            pending: candidates.into(),
            current: None,
            done: 0,
            options: vec![],
            cut_short: vec![],
        })
    }

    /// The cell that the click landed on.
    #[must_use]
    pub const fn of(&self) -> Generator {
        self.of
    }

    fn outcome(&mut self) -> DefinitionsOutcome {
        DefinitionsOutcome {
            of: self.of,
            options: std::mem::take(&mut self.options),
            cut_short: std::mem::take(&mut self.cut_short),
        }
    }
}

impl Job for DefinitionsJob {
    type Output = DefinitionsOutcome;

    fn label(&self) -> &'static str {
        "Looking for definitions"
    }

    fn progress(&self) -> Option<Progress> {
        Some(Progress {
            done: self.done,
            total: self.total,
        })
    }

    /// The time for one generator, as for the full search: a step is within a candidate.
    fn step_limit(&self) -> Option<std::time::Duration> {
        self.limits.generator()
    }

    fn step(&mut self) -> Option<Self::Output> {
        if self.current.is_none() {
            let Some(candidate) = self.pending.pop_front() else {
                return Some(self.outcome());
            };
            self.current = AttachOptionsJob::new(
                &self.proof,
                &self.point,
                self.weak_units,
                self.limits,
                Some(candidate),
            )
            .ok()
            .map(AttachOptionsJob::plain_only);
            if self.current.is_none() {
                self.done += 1;
            }
            return None;
        }
        if let Some(found) = self.current.as_mut().and_then(Job::step) {
            self.current = None;
            self.done += 1;
            self.options.extend(found.options);
            for name in found.inference.plain.cut_short {
                if !self.cut_short.contains(&name) {
                    self.cut_short.push(name);
                }
            }
        }
        None
    }
}
