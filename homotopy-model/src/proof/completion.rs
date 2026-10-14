//! Completing a partial substitution: which replacements follow from the ones chosen.
//!
//! Each replacement chosen has a type in terms of other placeholders. A cone `α₀ : X₀ ∘ ! ⇒ D₀`
//! for `α : X ∘ ! ⇒ D` can only be a cone over `D₀` with apex `X₀`. So this is a matching problem:
//! find the substitutions `σ` with `σ(source of α) = source of α₀` and likewise for the target.
//! It is solved by matching the types structurally, not by trying replacements:
//!
//! - A pattern that is the diagram of a placeholder is *bound* to the diagram it is matched with.
//! - Otherwise its boundaries are matched with the boundaries of the diagram, recursively down the
//!   dimensions: if `σ(S) = D` then `σ(∂S) = ∂D`.
//! - A pattern that is a vertical composite `g₁ ≫ … ≫ gₖ` of whole cells is matched with the
//!   diagram cut at heights, as `σ(S) = σ(g₁) ≫ … ≫ σ(gₖ)`: a *split*. Splits that can be done only
//!   one way are done at once. A split that is ambiguous (`μ ≫ η'` against a diagram of two heights
//!   can be cut in three places) is tried each way.
//!
//! What remains after matching are the placeholders that the types of the replacements chosen
//! mention but that matching did not reach. Those are searched for among the composites of the
//! cells that the replacements chosen are made of (see [`Pool`]), as before. Placeholders that
//! nothing mentions are never touched. Replacements are only trusted if the types that were not
//! matched exactly (in particular, those mentioning other generators that depend on placeholders)
//! check out under substitution.
//!
//! Every placeholder is determined if all the solutions agree on it. The search is bounded:
//! when it has to stop early, only what matching has forced (before any split was tried) is
//! returned, because that holds in every solution.

use std::rc::Rc;

use homotopy_common::tex::seal;
use homotopy_core::{
    signature::{Invertibility, Signature as _},
    Diagram, DiagramN, Generator, Orientation,
};

use super::{
    inference::{Cells, Pool, MAX_COMPLETIONS, MAX_COMPLETION_NODES},
    Signature,
};
use crate::job::{Job, Progress};

/// The most ways of cutting a diagram for a split that are tried.
const MAX_SPLITS: usize = 256;

pub type Replacements = Vec<(Generator, Diagram)>;

/// Why a diagram cannot replace a placeholder, given the replacements chosen for others.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplacementError {
    /// The schema has no such placeholder.
    NotAPlaceholder,
    /// The diagram has the wrong dimension for the placeholder.
    Dimension {
        expected: usize,
        found: usize,
        /// The source and target the replacement should have, if they are known.
        expected_boundaries: Option<(Diagram, Diagram)>,
    },
    /// The types of the replacements chosen do not agree on a placeholder: one requires it to be
    /// replaced with `existing`, because of the replacement chosen for `existing_because`, and
    /// another with `proposed`, because of the replacement chosen for `proposed_because`.
    Conflict {
        placeholder: Generator,
        existing: Diagram,
        existing_because: Generator,
        proposed: Diagram,
        proposed_because: Generator,
    },
    /// The boundary of the replacement chosen for `because` is not what the type of `owner`
    /// requires: where the type has `expected` (with placeholders in it), the boundary has `found`.
    Mismatch {
        because: Generator,
        owner: Generator,
        expected: Diagram,
        found: Diagram,
    },
    /// A diagram of the replacement chosen for `because` has to be cut into the replacements of
    /// `atoms` (the type of the placeholder says it is their composite), but cannot be cut so that
    /// the pieces are those already `known` for some of them.
    Uncuttable {
        because: Generator,
        atoms: Vec<Generator>,
        value: Diagram,
        /// The atoms that are already replaced, with their replacements and the replacement
        /// chosen that they follow from.
        known: Vec<(Generator, Diagram, Generator)>,
    },
}

/// A requirement that `σ(pattern) = value`.
#[derive(Clone)]
struct Constraint {
    pattern: Diagram,
    value: Diagram,
    /// The replacement chosen that this follows from, for messages.
    root: Generator,
    /// The placeholder whose type this is a part of.
    owner: Generator,
}

/// A whole cell in a vertical composite.
#[derive(Clone)]
struct Atom {
    generator: Generator,
    /// Whether it is the inverse of the cell of the generator.
    reflected: bool,
}

/// A requirement that a vertical composite of whole cells is the composite of the replacements
/// of its cells, so that the diagram it is matched with is cut into their replacements.
#[derive(Clone)]
struct Split {
    atoms: Vec<Atom>,
    value: DiagramN,
    root: Generator,
    owner: Generator,
}

#[derive(Clone, Default)]
struct State {
    /// The replacement of each parameter that matching has found, and what it follows from.
    bound: Vec<Option<(Diagram, Generator)>>,
    pending: Vec<Constraint>,
    splits: Vec<Split>,
    /// The placeholders whose types were not matched exactly: they are checked by substituting.
    inexact: Vec<Generator>,
}

enum Cuts {
    Impossible,
    Unique(Vec<usize>),
    Several(Vec<Vec<usize>>),
    TooMany,
}

/// What is known of the piece of a diagram that replaces a cell.
enum Want {
    Free,
    Size(usize),
    Piece(DiagramN),
}

/// Finds the ways of cutting after each cell, depth first.
fn cut_after(
    k: usize,
    n: usize,
    wants: &[Want],
    piece: &dyn Fn(usize, usize) -> DiagramN,
    cuts: &mut Vec<usize>,
    found: &mut Vec<Vec<usize>>,
) {
    if found.len() > MAX_SPLITS {
        return;
    }
    let pos = cuts[k];
    if k == wants.len() {
        if pos == n {
            found.push(cuts.clone());
        }
        return;
    }
    let last = k + 1 == wants.len();
    let sizes: Vec<usize> = match &wants[k] {
        Want::Size(s) => vec![*s],
        Want::Piece(d) => vec![d.size()],
        Want::Free if last => vec![n - pos],
        Want::Free => (0..=n - pos).collect(),
    };
    for size in sizes {
        if pos + size > n || (last && pos + size != n) {
            continue;
        }
        if let Want::Piece(d) = &wants[k] {
            if piece(pos, pos + size) != *d {
                continue;
            }
        }
        cuts.push(pos + size);
        cut_after(k + 1, n, wants, piece, cuts, found);
        cuts.pop();
    }
}

struct Matcher<'a> {
    signature: &'a Signature,
    parameters: &'a [Generator],
}

impl Matcher<'_> {
    fn index(&self, generator: Generator) -> Option<usize> {
        self.parameters.iter().position(|&p| p == generator)
    }

    fn own(&self, generator: Generator) -> Option<DiagramN> {
        match &self.signature.generator_info(generator)?.diagram {
            Diagram::DiagramN(d) => Some(d.clone()),
            Diagram::Diagram0(_) => None,
        }
    }

    fn mark(state: &mut State, owner: Generator) {
        if !state.inexact.contains(&owner) {
            state.inexact.push(owner);
        }
    }

    fn bind(
        &self,
        state: &mut State,
        index: usize,
        value: Diagram,
        root: Generator,
    ) -> Result<(), ReplacementError> {
        match &state.bound[index] {
            Some((existing, _)) if *existing == value => Ok(()),
            Some((existing, because)) => Err(ReplacementError::Conflict {
                placeholder: self.parameters[index],
                existing: existing.clone(),
                existing_because: *because,
                proposed: value,
                proposed_because: root,
            }),
            None => {
                state.bound[index] = Some((value, root));
                Ok(())
            }
        }
    }

    /// Requires `σ(placeholder's own diagram) = value`, which binds the placeholder, and requires
    /// its type to be right.
    fn require_replacement(
        &self,
        state: &mut State,
        placeholder: Generator,
        value: Diagram,
        root: Generator,
    ) {
        if let Some(own) = self.signature.generator_info(placeholder) {
            state.pending.push(Constraint {
                pattern: own.diagram.clone(),
                value,
                root,
                owner: placeholder,
            });
        }
    }

    /// The whole cells that a diagram is a vertical composite of, if it is one.
    fn atoms(&self, diagram: &DiagramN) -> Option<Vec<Atom>> {
        let slices: Vec<Diagram> = diagram.slices().collect();
        let mut atoms = Vec::with_capacity(diagram.size());
        for (i, cospan) in diagram.cospans().iter().enumerate() {
            let generator = slices[2 * i + 1].max_generator().generator;
            if generator.dimension != diagram.dimension() {
                return None;
            }
            let info = self.signature.generator_info(generator)?;
            let Diagram::DiagramN(own) = &info.diagram else {
                return None;
            };
            let cell = DiagramN::new(slices[2 * i].clone(), vec![cospan.clone()]);
            let reflected = if cell == *own {
                false
            } else if info.invertibility != Invertibility::Directed && cell == own.inverse() {
                true
            } else {
                return None;
            };
            atoms.push(Atom {
                generator,
                reflected,
            });
        }
        Some(atoms)
    }

    /// A cell that is not a placeholder but depends on some (such as the limit of a diagram `D`,
    /// which depends on `D`) is replaced by the cell of its instance. If `piece` is one, the
    /// instance says what the placeholders it depends on are replaced with, so that they are
    /// bound without searching.
    fn match_instance(&self, state: &mut State, atom: &Atom, piece: &DiagramN, root: Generator) {
        if self.index(atom.generator).is_some() {
            return;
        }
        let cell = if atom.reflected {
            piece.inverse()
        } else {
            piece.clone()
        };
        let instance = cell.max_generator().generator;
        let Some(info) = self.signature.generator_info(instance) else {
            return;
        };
        let Some(made) = &info.instance else {
            return;
        };
        if made.schema != atom.generator || info.diagram != Diagram::from(cell) {
            return;
        }
        for (placeholder, replacement) in &made.substitution {
            if self.index(*placeholder).is_some() {
                self.require_replacement(state, *placeholder, replacement.clone(), root);
            }
        }
    }

    fn unify(&self, state: &mut State, constraint: Constraint) -> Result<(), ReplacementError> {
        let Constraint {
            pattern,
            value,
            root,
            owner,
        } = constraint;
        let (expected, found) = (pattern.clone(), value.clone());
        let mismatch = move || ReplacementError::Mismatch {
            because: root,
            owner,
            expected: expected.clone(),
            found: found.clone(),
        };
        match (&pattern, &value) {
            (Diagram::Diagram0(s), Diagram::Diagram0(v)) => {
                if let Some(index) = self.index(s.generator) {
                    match s.orientation {
                        Orientation::Positive => self.bind(state, index, value.clone(), root),
                        Orientation::Negative => {
                            let v = v.orientation_transform(Orientation::Negative);
                            self.bind(state, index, v.into(), root)
                        }
                        Orientation::Zero => {
                            Self::mark(state, owner);
                            Ok(())
                        }
                    }
                } else if s == v {
                    Ok(())
                } else {
                    Err(mismatch())
                }
            }
            (Diagram::DiagramN(s), Diagram::DiagramN(v)) => {
                if s.dimension() != v.dimension() {
                    return Err(mismatch());
                }
                let generator = s.max_generator().generator;
                if let Some(index) = self
                    .index(generator)
                    .filter(|_| generator.dimension == s.dimension())
                {
                    let own = self.own(generator).ok_or_else(&mismatch)?;
                    let invertible = self
                        .signature
                        .generator_info(generator)
                        .is_some_and(|info| info.invertibility != Invertibility::Directed);
                    let bound = if *s == own {
                        Some(v.clone())
                    } else if invertible && *s == own.inverse() {
                        Some(v.inverse())
                    } else {
                        None
                    };
                    if let Some(bound) = bound {
                        self.bind(state, index, bound.clone().into(), root)?;
                        // The type of the placeholder is matched by its replacement.
                        for (p, v) in [
                            (own.source(), bound.source()),
                            (own.target(), bound.target()),
                        ] {
                            state.pending.push(Constraint {
                                pattern: p,
                                value: v,
                                root,
                                owner: generator,
                            });
                        }
                        return Ok(());
                    }
                }

                for (p, v) in [(s.source(), v.source()), (s.target(), v.target())] {
                    state.pending.push(Constraint {
                        pattern: p,
                        value: v,
                        root,
                        owner,
                    });
                }
                if s.size() == 0 {
                    // The identity of `σ(X)` is an identity.
                    if v.size() != 0 {
                        return Err(mismatch());
                    }
                    return Ok(());
                }
                match self.atoms(s) {
                    Some(atoms) if atoms.len() == 1 => {
                        // One cell that is not a placeholder: its replacement is one height.
                        if v.size() != 1 {
                            return Err(mismatch());
                        }
                        Self::mark(state, owner);
                        self.match_instance(state, &atoms[0], v, root);
                    }
                    Some(atoms) => {
                        if atoms.iter().any(|a| self.index(a.generator).is_none()) {
                            Self::mark(state, owner);
                        }
                        state.splits.push(Split {
                            atoms,
                            value: v.clone(),
                            root,
                            owner,
                        });
                    }
                    None => Self::mark(state, owner),
                }
                Ok(())
            }
            _ => Err(mismatch()),
        }
    }

    /// The ways of cutting the diagram of a split into the replacements of its cells, given what
    /// is bound so far.
    fn cuts(&self, state: &State, split: &Split) -> Cuts {
        let value = &split.value;
        let n = value.size();
        let mut wants = Vec::with_capacity(split.atoms.len());
        for atom in &split.atoms {
            wants.push(match self.index(atom.generator) {
                None => Want::Size(1),
                Some(i) => match &state.bound[i] {
                    None => Want::Free,
                    Some((Diagram::DiagramN(d), _)) => Want::Piece(if atom.reflected {
                        d.inverse()
                    } else {
                        d.clone()
                    }),
                    Some(_) => return Cuts::Impossible,
                },
            });
        }
        let regular: Vec<Diagram> = value.regular_slices().collect();
        let piece =
            |a: usize, b: usize| DiagramN::new(regular[a].clone(), value.cospans()[a..b].to_vec());

        let mut found: Vec<Vec<usize>> = vec![];
        let mut cuts = vec![0];
        cut_after(0, n, &wants, &piece, &mut cuts, &mut found);

        match found.len() {
            0 => Cuts::Impossible,
            1 => Cuts::Unique(found.remove(0)),
            l if l > MAX_SPLITS => Cuts::TooMany,
            _ => Cuts::Several(found),
        }
    }

    /// Requires each cell of a split to be replaced with the piece of the diagram between cuts.
    fn cut(&self, state: &mut State, split: &Split, cuts: &[usize]) {
        let value = &split.value;
        let regular: Vec<Diagram> = value.regular_slices().collect();
        for (atom, window) in split.atoms.iter().zip(cuts.windows(2)) {
            let piece = DiagramN::new(
                regular[window[0]].clone(),
                value.cospans()[window[0]..window[1]].to_vec(),
            );
            let piece = if atom.reflected {
                piece.inverse()
            } else {
                piece
            };
            if let Some(own) = self.signature.generator_info(atom.generator) {
                state.pending.push(Constraint {
                    pattern: own.diagram.clone(),
                    value: piece.into(),
                    root: split.root,
                    owner: split.owner,
                });
            }
        }
    }

    /// Why a split cannot be done: which replacements already known for its cells get in the way.
    fn uncuttable(&self, state: &State, split: &Split) -> ReplacementError {
        let known = split
            .atoms
            .iter()
            .filter_map(|atom| {
                let (value, because) = state.bound[self.index(atom.generator)?].clone()?;
                Some((atom.generator, value, because))
            })
            .collect();
        ReplacementError::Uncuttable {
            because: split.root,
            atoms: split.atoms.iter().map(|atom| atom.generator).collect(),
            value: split.value.clone().into(),
            known,
        }
    }

    /// Matches until nothing more is forced: what is left are splits that could be done in
    /// several ways.
    fn propagate(&self, state: &mut State) -> Result<(), ReplacementError> {
        loop {
            while let Some(constraint) = state.pending.pop() {
                self.unify(state, constraint)?;
            }
            let mut progress = false;
            for split in std::mem::take(&mut state.splits) {
                match self.cuts(state, &split) {
                    Cuts::Impossible => return Err(self.uncuttable(state, &split)),
                    Cuts::Unique(cuts) => {
                        self.cut(state, &split, &cuts);
                        progress = true;
                    }
                    Cuts::Several(_) | Cuts::TooMany => state.splits.push(split),
                }
            }
            if !progress {
                return Ok(());
            }
        }
    }
}

/// What is known about the placeholders of a schema after matching and searching.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inference {
    /// The replacements that follow from the ones chosen: those that every way of filling in the
    /// types agrees on. These are never replacements that were chosen.
    pub determined: Replacements,
    /// The ways of filling in what is left when there are several, each a replacement for every
    /// placeholder that is neither chosen nor determined. Those with the fewest identities among
    /// the replacements come first. There are at most [`MAX_COMPLETIONS`] of them, whether or not
    /// there are more (see `exhaustive`), and never one alone: a single way is determined.
    pub candidates: Vec<Replacements>,
    /// Whether `candidates` are all the ways there are.
    pub exhaustive: bool,
    /// The cuts of a diagram into the replacements of its cells that the types leave open, which
    /// is where the ways come from. Choosing a replacement for one of the cells decides it.
    pub ambiguities: Vec<Ambiguity>,
    pub status: Status,
}

/// How the work of an [`Inference`] ended.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Status {
    /// Everything the types say was worked out.
    #[default]
    Complete,
    /// The search was stopped short: only what matching forced is determined.
    LimitReached,
    /// There is no way to fill in the placeholders so that the types of the replacements chosen
    /// are right.
    NoFilling,
    /// The replacements chosen contradict each other.
    Inconsistent(ReplacementError),
}

/// A diagram that has to be cut into the replacements of the cells of a composite in several
/// ways, such as `p ≫ ζ[p]` into `μ ≫ η'`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ambiguity {
    /// The cells of the composite, in order (`μ`, `η'`).
    pub cells: Vec<Generator>,
    /// The diagram to cut.
    pub value: Diagram,
    /// The number of ways, if it is known.
    pub ways: Option<usize>,
    /// The cells whose replacement is still open: choosing one of them cuts the diagram.
    pub open: Vec<Generator>,
}

/// The things to try in turn, most recent last.
enum Frame {
    /// The ways to cut one split.
    Branch {
        state: State,
        split: Split,
        cuts: Vec<Vec<usize>>,
        next: usize,
    },
    /// The candidates for one placeholder.
    Search(Searching),
}

/// What the search for replacements that matching did not reach shares between its frames.
struct Prepared {
    state: State,
    /// The placeholders that a replacement is needed for: those bound and those that the types
    /// mention.
    needed: Vec<bool>,
    /// The replacements to check by substituting, by the number of placeholders to be replaced
    /// before that can be done.
    checks: Vec<Vec<usize>>,
}

struct Searching {
    prepared: Rc<Prepared>,
    /// The length of the substitution below this frame.
    depth: usize,
    /// The placeholder, by position.
    j: usize,
    candidates: Vec<Diagram>,
    next: usize,
    /// Whether the candidates were looked for (as opposed to being bound by matching), which is
    /// what costs.
    searched: bool,
}

/// The things that do not change while the work for a schema is done.
struct Context<'a> {
    matcher: Matcher<'a>,
    pool: Pool<'a>,
}

/// The search for the placeholders determined by replacements chosen for others, as a state that
/// can be advanced a step at a time: what it is waiting to try is on a stack of its own.
///
/// A complete substitution is a solution if the type of each replacement chosen is as expected.
struct Machine {
    chosen: Replacements,
    stack: Vec<Frame>,
    /// The replacements tried, for the placeholders searched so far.
    substitution: Replacements,
    nodes: usize,
    solutions: Vec<Replacements>,
    /// Whether the search was cut short, so that its solutions are not all of them.
    cut_short: bool,
    /// What matching forced before anything was tried.
    forced: Replacements,
    ambiguities: Vec<Ambiguity>,
    /// The answer, when it was known at the start.
    done: Option<Inference>,
}

impl Machine {
    fn new(ctx: &Context, chosen: &Replacements) -> Self {
        let matcher = &ctx.matcher;
        let nothing_to_do = chosen.is_empty() || chosen.len() >= matcher.parameters.len();
        let mut machine = Self {
            chosen: chosen.clone(),
            stack: vec![],
            substitution: vec![],
            nodes: 0,
            solutions: vec![],
            cut_short: false,
            forced: vec![],
            ambiguities: vec![],
            done: nothing_to_do.then(Inference::default),
        };
        if nothing_to_do {
            return machine;
        }
        let mut state = State {
            bound: vec![None; matcher.parameters.len()],
            ..State::default()
        };
        for (p, d) in chosen {
            let expected = matcher
                .signature
                .generator_info(*p)
                .map(|info| info.diagram.dimension());
            if expected != Some(d.dimension()) {
                machine.done = Some(Inference {
                    status: Status::Inconsistent(ReplacementError::Dimension {
                        expected: expected.unwrap_or_default(),
                        found: d.dimension(),
                        expected_boundaries: None,
                    }),
                    ..Inference::default()
                });
                return machine;
            }
            matcher.require_replacement(&mut state, *p, d.clone(), *p);
        }
        // What is forced, whichever way the splits go.
        if let Err(error) = matcher.propagate(&mut state) {
            machine.done = Some(Inference {
                status: Status::Inconsistent(error),
                ..Inference::default()
            });
            return machine;
        }
        machine.forced = matcher
            .parameters
            .iter()
            .zip(&state.bound)
            .filter(|(p, _)| !chosen.iter().any(|(q, _)| q == *p))
            .filter_map(|(&p, b)| b.as_ref().map(|(d, _)| (p, d.clone())))
            .collect();
        machine.ambiguities = state
            .splits
            .iter()
            .map(|split| {
                let ways = match matcher.cuts(&state, split) {
                    Cuts::Several(cuts) => Some(cuts.len()),
                    Cuts::Unique(_) => Some(1),
                    Cuts::Impossible => Some(0),
                    Cuts::TooMany => None,
                };
                Ambiguity {
                    cells: split.atoms.iter().map(|atom| atom.generator).collect(),
                    value: split.value.clone().into(),
                    ways,
                    open: split
                        .atoms
                        .iter()
                        .map(|atom| atom.generator)
                        .filter(|&g| matcher.index(g).is_some_and(|i| state.bound[i].is_none()))
                        .collect(),
                }
            })
            .collect();
        machine.enter(ctx, state);
        machine
    }

    /// How many replacements have been tried, and the most that are.
    const fn progress(&self) -> Progress {
        Progress {
            done: self.nodes,
            total: MAX_COMPLETION_NODES,
        }
    }

    /// Does a bit of the work. Returns the answer when it is all done.
    fn step(&mut self, ctx: &Context) -> Option<Inference> {
        if let Some(done) = self.done.take() {
            return Some(done);
        }
        if self.cut_short {
            return Some(self.finish());
        }
        match self.stack.last_mut() {
            None => return Some(self.finish()),
            Some(Frame::Branch {
                state,
                split,
                cuts,
                next,
            }) => {
                if *next >= cuts.len() {
                    self.stack.pop();
                } else if self.nodes >= MAX_COMPLETION_NODES {
                    self.cut_short = true;
                } else {
                    self.nodes += 1;
                    let mut state = state.clone();
                    ctx.matcher.cut(&mut state, split, &cuts[*next]);
                    *next += 1;
                    self.enter(ctx, state);
                }
            }
            Some(Frame::Search(searching)) => {
                self.substitution.truncate(searching.depth);
                if searching.next >= searching.candidates.len() {
                    self.stack.pop();
                } else if searching.searched && self.nodes >= MAX_COMPLETION_NODES {
                    self.cut_short = true;
                } else {
                    let candidate = searching.candidates[searching.next].clone();
                    searching.next += 1;
                    self.nodes += usize::from(searching.searched);
                    let (j, prepared) = (searching.j, Rc::clone(&searching.prepared));
                    self.substitution
                        .push((ctx.matcher.parameters[j], candidate));
                    if self.consistent(ctx, &prepared, &prepared.checks[j + 1]) {
                        self.search(ctx, &prepared, j + 1);
                    }
                }
            }
        }
        None
    }

    /// Starts on the splits that remain, if any, once the state is matched as far as it goes.
    fn enter(&mut self, ctx: &Context, mut state: State) {
        if ctx.matcher.propagate(&mut state).is_err() {
            return;
        }
        let mut best: Option<(usize, Vec<Vec<usize>>)> = None;
        for (i, split) in state.splits.iter().enumerate() {
            match ctx.matcher.cuts(&state, split) {
                Cuts::Several(cuts) => {
                    if best.as_ref().is_none_or(|(_, b)| cuts.len() < b.len()) {
                        best = Some((i, cuts));
                    }
                }
                Cuts::TooMany => {
                    self.cut_short = true;
                    return;
                }
                Cuts::Impossible | Cuts::Unique(_) => return,
            }
        }
        match best {
            None => self.complete(ctx, state),
            Some((i, cuts)) => {
                let split = state.splits.remove(i);
                self.stack.push(Frame::Branch {
                    state,
                    split,
                    cuts,
                    next: 0,
                });
            }
        }
    }

    /// Prepares the search for the replacements that the types of the replacements bound mention,
    /// and that matching did not reach.
    fn complete(&mut self, ctx: &Context, state: State) {
        let parameters = ctx.matcher.parameters;
        let signature = ctx.matcher.signature;
        let position = |q: Generator| parameters.iter().position(|&p| p == q);

        // The placeholders that matter: those bound and those that their types mention.
        let mut needed: Vec<bool> = state.bound.iter().map(Option::is_some).collect();
        loop {
            let mut changed = false;
            for i in 0..parameters.len() {
                if needed[i] {
                    for q in signature.parameters(parameters[i]) {
                        if let Some(j) = position(q) {
                            if !needed[j] {
                                needed[j] = true;
                                changed = true;
                            }
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }

        // Replacements whose types are checked by substituting, and when they can be.
        let mut checks: Vec<Vec<usize>> = vec![vec![]; parameters.len() + 1];
        for (i, &p) in parameters.iter().enumerate() {
            if state.bound[i].is_some() && (state.inexact.contains(&p)) {
                let when = signature
                    .parameters(p)
                    .into_iter()
                    .filter_map(position)
                    .map(|j| j + 1)
                    .max()
                    .unwrap_or(0);
                checks[when].push(i);
            }
        }

        let prepared = Rc::new(Prepared {
            state,
            needed,
            checks,
        });
        self.substitution.clear();
        if self.consistent(ctx, &prepared, &prepared.checks[0]) {
            self.search(ctx, &prepared, 0);
        }
    }

    /// Starts on the next placeholder that needs a replacement, from the `from`th on, or finds a
    /// solution if there is none.
    fn search(&mut self, ctx: &Context, prepared: &Rc<Prepared>, from: usize) {
        let parameters = ctx.matcher.parameters;
        let Some(j) = (from..parameters.len()).find(|&j| prepared.needed[j]) else {
            if !self.solutions.contains(&self.substitution) {
                self.solutions.push(self.substitution.clone());
            }
            self.cut_short |= self.solutions.len() >= MAX_COMPLETIONS;
            return;
        };
        let (candidates, searched) = match &prepared.state.bound[j] {
            Some((d, _)) => (vec![d.clone()], false),
            None => (ctx.pool.candidates(parameters[j], &self.substitution), true),
        };
        self.stack.push(Frame::Search(Searching {
            prepared: Rc::clone(prepared),
            depth: self.substitution.len(),
            j,
            candidates,
            next: 0,
            searched,
        }));
    }

    /// Whether the replacements bound that can be checked now have the right types.
    fn consistent(&self, ctx: &Context, prepared: &Prepared, indices: &[usize]) -> bool {
        indices.iter().all(|&i| {
            let placeholder = ctx.matcher.parameters[i];
            let Some((replacement, _)) = &prepared.state.bound[i] else {
                return false;
            };
            Self::well_typed(ctx, placeholder, replacement, &self.substitution)
        })
    }

    fn well_typed(
        ctx: &Context,
        placeholder: Generator,
        replacement: &Diagram,
        substitution: &[(Generator, Diagram)],
    ) -> bool {
        let Some(own) = ctx
            .pool
            .signature
            .generator_info(placeholder)
            .map(|info| info.diagram.clone())
        else {
            return false;
        };
        match (own, replacement) {
            (Diagram::Diagram0(_), Diagram::Diagram0(_)) => true,
            (Diagram::DiagramN(own), Diagram::DiagramN(replacement)) => {
                own.dimension() == replacement.dimension()
                    && ctx
                        .pool
                        .substitute(placeholder, substitution, &own.source(), true)
                        == Some(replacement.source())
                    && ctx
                        .pool
                        .substitute(placeholder, substitution, &own.target(), true)
                        == Some(replacement.target())
            }
            _ => false,
        }
    }

    /// What the search came to.
    fn finish(&mut self) -> Inference {
        let determined = if self.cut_short {
            std::mem::take(&mut self.forced)
        } else {
            self.agreed()
        };
        let mut candidates: Vec<Replacements> = if self.solutions.len() > 1 {
            self.solutions
                .iter()
                .map(|solution| {
                    solution
                        .iter()
                        .filter(|(p, _)| {
                            !self.chosen.iter().any(|(q, _)| q == p)
                                && !determined.iter().any(|(q, _)| q == p)
                        })
                        .cloned()
                        .collect()
                })
                .collect()
        } else {
            vec![]
        };
        // Those with no identities among the cells of the cuts first. This only orders them: no
        // way is better than another as far as the types go. Stable, so that the others stay in
        // the order they were found.
        let pieces: Vec<Generator> = self
            .ambiguities
            .iter()
            .flat_map(|ambiguity| ambiguity.cells.iter().copied())
            .collect();
        candidates.sort_by_key(|candidate| identities(candidate, &pieces));
        let status = if self.cut_short {
            Status::LimitReached
        } else if self.solutions.is_empty() {
            Status::NoFilling
        } else {
            Status::Complete
        };
        Inference {
            determined,
            candidates,
            exhaustive: !self.cut_short,
            ambiguities: std::mem::take(&mut self.ambiguities),
            status,
        }
    }

    /// The replacements that all solutions agree on, other than those chosen.
    fn agreed(&self) -> Replacements {
        let Some((first, rest)) = self.solutions.split_first() else {
            return vec![];
        };
        first
            .iter()
            .filter(|(p, d)| {
                !self.chosen.iter().any(|(q, _)| q == p)
                    && rest
                        .iter()
                        .all(|solution| solution.iter().any(|(q, e)| q == p && e == d))
            })
            .cloned()
            .collect()
    }
}

/// How many of the replacements of `pieces` are identities.
fn identities(replacements: &Replacements, pieces: &[Generator]) -> usize {
    replacements
        .iter()
        .filter(|(p, d)| pieces.contains(p) && matches!(d, Diagram::DiagramN(d) if d.size() == 0))
        .count()
}

/// The work of [`Signature::infer`] as a [`Job`], so that it can be done a step at a time, or
/// not at all.
///
/// The job holds the signature it was made with, and nothing of the app's.
pub struct InferJob {
    signature: Signature,
    schema: Generator,
    parameters: Vec<Generator>,
    cells: Rc<Cells>,
    machine: Machine,
}

impl InferJob {
    /// The search for what the replacements `chosen` for some placeholders of a schema determine.
    #[must_use]
    pub fn new(signature: Signature, schema: Generator, chosen: &[(Generator, Diagram)]) -> Self {
        let parameters = signature.parameters(schema);
        let chosen: Replacements = chosen
            .iter()
            .filter(|(p, _)| parameters.contains(p))
            .cloned()
            .collect();
        let cells = Pool::new(
            &signature,
            chosen.iter().flat_map(|(_, d)| d.generators().into_keys()),
        )
        .cells();
        let machine = {
            let ctx = Context::of(&signature, &parameters, &cells);
            Machine::new(&ctx, &chosen)
        };
        Self {
            signature,
            schema,
            parameters,
            cells,
            machine,
        }
    }

    /// The schema the job is for.
    #[must_use]
    pub const fn schema(&self) -> Generator {
        self.schema
    }
}

impl<'a> Context<'a> {
    fn of(signature: &'a Signature, parameters: &'a [Generator], cells: &Rc<Cells>) -> Self {
        Self {
            matcher: Matcher {
                signature,
                parameters,
            },
            pool: Pool::from_cells(signature, Rc::clone(cells)),
        }
    }
}

impl Job for InferJob {
    type Output = Inference;

    fn label(&self) -> &'static str {
        "Inferring replacements"
    }

    fn progress(&self) -> Option<Progress> {
        (self.machine.nodes > 0).then(|| self.machine.progress())
    }

    fn step(&mut self) -> Option<Inference> {
        let ctx = Context::of(&self.signature, &self.parameters, &self.cells);
        self.machine.step(&ctx)
    }
}

impl Signature {
    /// What the replacements `chosen` for some placeholders of a schema determine, and the ways to
    /// fill in the rest when they leave several: see [`Inference`] and the [module](self).
    /// This is done in one go; [`InferJob`] does the same a step at a time.
    #[must_use]
    pub fn infer(&self, schema: Generator, chosen: &[(Generator, Diagram)]) -> Inference {
        crate::job::run_to_end(InferJob::new(self.clone(), schema, chosen))
    }

    /// Whether `value` can replace `placeholder` of `schema` as well as the replacements
    /// `chosen` for other placeholders, going by their types: its dimension, and then its
    /// boundaries against what the replacements chosen require of the placeholders its type
    /// mentions. This is cheap: it matches types and never searches.
    ///
    /// # Errors
    ///
    /// Why the value cannot replace the placeholder: see [`ReplacementError`], and
    /// [`describe_replacement_error`](Self::describe_replacement_error) to say so.
    pub fn check_replacement(
        &self,
        schema: Generator,
        chosen: &[(Generator, Diagram)],
        placeholder: Generator,
        value: &Diagram,
    ) -> Result<(), ReplacementError> {
        let mut entries: Replacements = chosen
            .iter()
            .filter(|(p, _)| *p != placeholder)
            .cloned()
            .collect();
        entries.push((placeholder, value.clone()));
        self.check_replacements(schema, &entries)
            .pop()
            .unwrap_or(Ok(()))
    }

    /// Checks replacements one after the other, each against the ones before it that were
    /// accepted: the answer for each is whether it is well typed given those. So an entry is
    /// only ever blamed on entries before it, and when two cannot both hold the earlier one wins.
    /// This is cheap, whatever the number of entries (see [`check_replacement`](Self::check_replacement)).
    #[must_use]
    pub fn check_replacements(
        &self,
        schema: Generator,
        entries: &[(Generator, Diagram)],
    ) -> Vec<Result<(), ReplacementError>> {
        let parameters = self.parameters(schema);
        let matcher = Matcher {
            signature: self,
            parameters: &parameters,
        };
        let mut accepted = State {
            bound: vec![None; parameters.len()],
            ..State::default()
        };
        entries
            .iter()
            .map(|(placeholder, value)| {
                let next = self.check_against(&matcher, &accepted, *placeholder, value)?;
                accepted = next;
                Ok(())
            })
            .collect()
    }

    /// Matches a replacement against what has been bound, giving what is bound after it.
    fn check_against(
        &self,
        matcher: &Matcher,
        accepted: &State,
        placeholder: Generator,
        value: &Diagram,
    ) -> Result<State, ReplacementError> {
        if !matcher.parameters.contains(&placeholder) {
            return Err(ReplacementError::NotAPlaceholder);
        }
        let expected = self.generator_info(placeholder).map(|info| &info.diagram);
        if expected.is_none_or(|own| own.dimension() != value.dimension()) {
            let expected_boundaries = expected.and_then(|own| {
                let Diagram::DiagramN(own) = own else {
                    return None;
                };
                let substitution: Replacements = matcher
                    .parameters
                    .iter()
                    .zip(&accepted.bound)
                    .filter_map(|(&p, b)| b.as_ref().map(|(d, _)| (p, d.clone())))
                    .collect();
                let pool = Pool::new(self, []);
                Some((
                    pool.substitute(placeholder, &substitution, &own.source(), true)?,
                    pool.substitute(placeholder, &substitution, &own.target(), true)?,
                ))
            });
            return Err(ReplacementError::Dimension {
                expected: expected.map_or(0, Diagram::dimension),
                found: value.dimension(),
                expected_boundaries,
            });
        }
        let mut state = accepted.clone();
        matcher.require_replacement(&mut state, placeholder, value.clone(), placeholder);
        matcher.propagate(&mut state)?;
        Ok(state)
    }

    fn name_sealed(&self, g: Generator) -> String {
        self.generator_info(g)
            .map_or_else(|| "?".to_owned(), |info| seal(&info.name))
    }

    /// A sentence saying why a value cannot replace `placeholder`, with the names sealed so that
    /// it can be shown as a label.
    #[must_use]
    pub fn describe_replacement_error(
        &self,
        placeholder: Generator,
        error: &ReplacementError,
    ) -> String {
        let name = |g: Generator| self.name_sealed(g);
        let cell = |d: usize| format!("{d}-dimensional diagram");
        // "X conflicts with Y: " unless X is Y.
        let conflicts = |others: &[Generator]| {
            let mut others: Vec<String> = others
                .iter()
                .filter(|&&g| g != placeholder)
                .map(|&g| name(g))
                .collect();
            others.dedup();
            if others.is_empty() {
                format!("{}: ", name(placeholder))
            } else {
                format!(
                    "{} conflicts with {}: ",
                    name(placeholder),
                    others.join(", ")
                )
            }
        };
        match error {
            ReplacementError::NotAPlaceholder => {
                format!("{} is not a placeholder of this schema", name(placeholder))
            }
            ReplacementError::Dimension {
                expected,
                found,
                expected_boundaries,
            } => {
                let boundaries = expected_boundaries
                    .as_ref()
                    .map_or_else(String::new, |(s, t)| {
                        format!(" from {} to {}", self.describe(s), self.describe(t))
                    });
                format!(
                    "{}: expected a {}{boundaries}, got a {}",
                    name(placeholder),
                    cell(*expected),
                    cell(*found)
                )
            }
            ReplacementError::Conflict {
                placeholder: conflicting,
                existing,
                existing_because,
                proposed,
                proposed_because,
            } => {
                if *proposed_because == placeholder {
                    format!(
                        "{}{} needs {} to be {}, not {}",
                        conflicts(&[*existing_because]),
                        name(*existing_because),
                        name(*conflicting),
                        self.describe(existing),
                        self.describe(proposed),
                    )
                } else {
                    format!(
                        "{}{} needs {} to be {}, but {} needs it to be {}",
                        conflicts(&[*existing_because, *proposed_because]),
                        name(*existing_because),
                        name(*conflicting),
                        self.describe(existing),
                        name(*proposed_because),
                        self.describe(proposed),
                    )
                }
            }
            ReplacementError::Mismatch {
                because,
                owner,
                expected,
                found,
            } => format!(
                "{}the boundary of {} has {} where the type of {} has {}",
                conflicts(&[*because]),
                name(*because),
                self.describe(found),
                name(*owner),
                self.describe(expected),
            ),
            ReplacementError::Uncuttable {
                because,
                atoms,
                value,
                known,
            } => {
                let blamed: Vec<Generator> = std::iter::once(*because)
                    .chain(known.iter().map(|(_, _, b)| *b))
                    .collect();
                let known_text = known
                    .iter()
                    .map(|(g, d, _)| format!("{} is {}", name(*g), self.describe(d)))
                    .collect::<Vec<_>>()
                    .join(" and ");
                format!(
                    "{}{} needs {} to be {}, but {}",
                    conflicts(&blamed),
                    name(*because),
                    self.describe_composite(atoms),
                    self.describe(value),
                    if known_text.is_empty() {
                        "that cannot be done".to_owned()
                    } else {
                        known_text
                    },
                )
            }
        }
    }

    /// The cells of a composite, like `μ ≫ η'`.
    fn describe_composite(&self, cells: &[Generator]) -> String {
        cells
            .iter()
            .map(|&g| self.name_sealed(g))
            .collect::<Vec<_>>()
            .join(" \u{226b} ")
    }

    /// A way of filling in placeholders, like `μ = p, η' = ζ[p]`, with names sealed so that it can
    /// be shown as a label.
    #[must_use]
    pub fn describe_filling(&self, filling: &[(Generator, Diagram)]) -> String {
        filling
            .iter()
            .map(|(p, d)| format!("{} = {}", self.name_sealed(*p), self.describe(d)))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// What there is to say about an inference beyond what it fills in: why something was left
    /// open or not filled in, each as a sentence.
    #[must_use]
    pub fn explain_inference(&self, inference: &Inference) -> Vec<String> {
        let mut notes = vec![];
        match &inference.status {
            Status::Complete => {}
            Status::LimitReached => notes
                .push("Search limit reached: only what the types force is filled in.".to_owned()),
            Status::NoFilling => notes.push(
                "No way of filling in the other placeholders fits the replacements chosen."
                    .to_owned(),
            ),
            Status::Inconsistent(error) => {
                let blamed = match error {
                    ReplacementError::Mismatch { because, .. }
                    | ReplacementError::Uncuttable { because, .. } => Some(*because),
                    ReplacementError::Conflict {
                        proposed_because, ..
                    } => Some(*proposed_because),
                    _ => None,
                };
                if let Some(blamed) = blamed {
                    notes.push(self.describe_replacement_error(blamed, error));
                }
            }
        }
        for ambiguity in &inference.ambiguities {
            let Some(ways) = ambiguity.ways.filter(|&ways| ways > 1) else {
                continue;
            };
            let deciding = ambiguity
                .open
                .iter()
                .map(|&g| self.name_sealed(g))
                .collect::<Vec<_>>()
                .join(" or ");
            notes.push(format!(
                "{} can be cut {ways} ways to make {}{}",
                self.describe_composite(&ambiguity.cells),
                self.describe(&ambiguity.value),
                if deciding.is_empty() {
                    String::new()
                } else {
                    format!("; choose {deciding} to decide")
                }
            ));
        }
        notes
    }
}

#[cfg(test)]
mod tests {
    use homotopy_core::{
        signature::{Invertibility, Signature as _},
        Boundary, Diagram, DiagramN, Generator,
    };

    use super::{InferJob, ReplacementError, Status};
    use crate::{
        job::Job,
        proof::{Signature, SignatureEdit, SignatureItemEdit},
    };

    /// A commutative square, as in a naturality condition: `x`; placeholders `F, G, F', G' : x → x`,
    /// `η : F ⇒ G`, `η' : F' ⇒ G'`, `μ : F ⇒ F'`, `ν : G ⇒ G'` and `h : μ ≫ η' ⇛ η ≫ ν`; the cells
    /// `T, U : x → x`, `p : T ⇒ U`, `z : U ⇒ T`, `l : T ⇒ T` and `k : p ≫ z ⇛ l ≫ l`; and a
    /// schema `s : x → x` that depends on `h`.
    struct Square {
        sig: Signature,
        schema: Generator,
        f: Generator,
        g: Generator,
        f2: Generator,
        g2: Generator,
        eta: Generator,
        eta2: Generator,
        mu: Generator,
        nu: Generator,
        h: Generator,
        t: Diagram,
        u: Diagram,
        p: Diagram,
        z: Diagram,
        l: Diagram,
        k: Diagram,
        /// The inverse of `k`.
        ki: Diagram,
    }

    fn generator(d: &DiagramN) -> Generator {
        d.max_generator().generator
    }

    fn square() -> Square {
        let mut sig = Signature::default();
        let x: Diagram = sig.create_generator_zero("x").into();
        let cell = |sig: &mut Signature, s: Diagram, t: Diagram, name: &str| {
            sig.create_generator(s, t, name, Invertibility::Directed)
                .unwrap()
        };
        let [f, g, f2, g2, t, u]: [DiagramN; 6] =
            ["F", "G", "F'", "G'", "T", "U"].map(|name| cell(&mut sig, x.clone(), x.clone(), name));
        let eta = cell(&mut sig, f.clone().into(), g.clone().into(), "eta");
        let eta2 = cell(&mut sig, f2.clone().into(), g2.clone().into(), "eta'");
        let mu = cell(&mut sig, f.clone().into(), f2.clone().into(), "mu");
        let nu = cell(&mut sig, g.clone().into(), g2.clone().into(), "nu");
        let mu_eta2 = mu.attach(&eta2, Boundary::Target, &[]).unwrap();
        let eta_nu = eta.attach(&nu, Boundary::Target, &[]).unwrap();
        let h = cell(&mut sig, mu_eta2.into(), eta_nu.into(), "h");

        let p = cell(&mut sig, t.clone().into(), u.clone().into(), "p");
        let z = cell(&mut sig, u.clone().into(), t.clone().into(), "z");
        let l = cell(&mut sig, t.clone().into(), t.clone().into(), "l");
        let pz = p.attach(&z, Boundary::Target, &[]).unwrap();
        let ll = l.attach(&l, Boundary::Target, &[]).unwrap();
        let k = sig
            .create_generator(pz.into(), ll.into(), "k", Invertibility::Invertible)
            .unwrap();
        let s = cell(&mut sig, x.clone(), x, "s");

        let generators = [&f, &g, &f2, &g2, &eta, &eta2, &mu, &nu, &h].map(generator);
        for placeholder in generators {
            let node = sig.find_node(placeholder).unwrap();
            sig.update(&SignatureEdit::Edit(
                node,
                SignatureItemEdit::MakePlaceholder(true),
            ))
            .unwrap();
        }
        let schema = generator(&s);
        let node = sig.find_node(schema).unwrap();
        sig.update(&SignatureEdit::Edit(
            node,
            SignatureItemEdit::SetDependsOn(vec![generator(&h)]),
        ))
        .unwrap();
        let [f, g, f2, g2, eta, eta2, mu, nu, h] = generators;
        Square {
            sig,
            schema,
            f,
            g,
            f2,
            g2,
            eta,
            eta2,
            mu,
            nu,
            h,
            t: t.into(),
            u: u.into(),
            p: p.into(),
            z: z.into(),
            l: l.into(),
            ki: k.inverse().into(),
            k: k.into(),
        }
    }

    #[test]
    fn a_square_determines_its_corners() {
        let sq = square();
        assert_eq!(sq.sig.parameters(sq.schema).len(), 9);
        // `p ≫ z ⇛ l ≫ l` goes from `T` to `T`, and `l ≫ l` has only `T` in between, however it
        // is cut in two. `p ≫ z` has `U` in the middle, so `F'` could be `U`, or `T` if `μ` or `η'`
        // is an identity: it is not determined, and neither are the cells.
        assert_eq!(
            sq.sig
                .infer_replacements(sq.schema, &[(sq.h, sq.k.clone())]),
            vec![
                (sq.f, sq.t.clone()),
                (sq.g, sq.t.clone()),
                (sq.g2, sq.t.clone())
            ]
        );

        // Choosing `η := l` determines the second half of the bottom.
        let found = sq
            .sig
            .infer_replacements(sq.schema, &[(sq.h, sq.k.clone()), (sq.eta, sq.l.clone())]);
        assert_eq!(
            found,
            vec![
                (sq.f, sq.t.clone()),
                (sq.g, sq.t.clone()),
                (sq.g2, sq.t.clone()),
                (sq.nu, sq.l.clone())
            ]
        );

        // Choosing the top as well determines the rest.
        let found = sq.sig.infer_replacements(
            sq.schema,
            &[
                (sq.h, sq.k.clone()),
                (sq.mu, sq.p.clone()),
                (sq.eta, sq.l.clone()),
            ],
        );
        assert_eq!(
            found,
            vec![
                (sq.f, sq.t.clone()),
                (sq.g, sq.t.clone()),
                (sq.f2, sq.u.clone()),
                (sq.g2, sq.t.clone()),
                (sq.eta2, sq.z.clone()),
                (sq.nu, sq.l.clone()),
            ]
        );

        // `η := p` contradicts `k`, whose bottom starts with `l`.
        assert!(sq
            .sig
            .infer_replacements(sq.schema, &[(sq.h, sq.k.clone()), (sq.eta, sq.p.clone())])
            .is_empty());
    }

    /// A chain of placeholders, each the type of the next: `a, b`, `A₀, A₀' : a → b` and
    /// `Aₖ, Aₖ' : Aₖ₋₁ ⇒ Aₖ₋₁'` up to `A₄ : A₃ ⇒ A₃'`, and for each of them a cell of the same shape
    /// that is not a placeholder (`x, y`, `C₀` ...), with a schema that depends on `A₄`.
    struct Chain {
        sig: Signature,
        schema: Generator,
        /// The placeholders from the bottom: `a, b, A₀, A₀', A₁, A₁', ..., A₃, A₃', A₄`.
        placeholders: Vec<Generator>,
        /// What they can be replaced with, in the same order.
        cells: Vec<Diagram>,
    }

    fn chain(length: usize) -> Chain {
        let mut sig = Signature::default();
        let mut placeholders = vec![];
        let mut cells = vec![];
        let zero =
            |sig: &mut Signature, name: &str| -> Diagram { sig.create_generator_zero(name).into() };
        let (a, b) = (zero(&mut sig, "a"), zero(&mut sig, "b"));
        let (x, y) = (zero(&mut sig, "x"), zero(&mut sig, "y"));
        placeholders.extend([a.clone(), b.clone()]);
        cells.extend([x.clone(), y.clone()]);
        // The two cells at each level, and the pair of boundaries for the next.
        let (mut ps, mut cs) = ((a, b), (x, y));
        for k in 0..length {
            let level = |sig: &mut Signature,
                         name: &str,
                         ps: &(Diagram, Diagram),
                         cs: &(Diagram, Diagram)| {
                let p = sig
                    .create_generator(
                        ps.0.clone(),
                        ps.1.clone(),
                        &format!("A{k}{name}"),
                        Invertibility::Directed,
                    )
                    .unwrap();
                let c = sig
                    .create_generator(
                        cs.0.clone(),
                        cs.1.clone(),
                        &format!("C{k}{name}"),
                        Invertibility::Directed,
                    )
                    .unwrap();
                (p, c)
            };
            let (p, c) = level(&mut sig, "", &ps, &cs);
            if k + 1 == length {
                placeholders.push(p.into());
                cells.push(c.into());
                break;
            }
            let (p2, c2) = level(&mut sig, "'", &ps, &cs);
            placeholders.extend([p.clone().into(), p2.clone().into()]);
            cells.extend([c.clone().into(), c2.clone().into()]);
            ps = (p.into(), p2.into());
            cs = (c.into(), c2.into());
        }
        let z: Diagram = sig.create_generator_zero("z").into();
        let s = sig
            .create_generator(z.clone(), z, "S", Invertibility::Directed)
            .unwrap();

        let placeholders: Vec<Generator> = placeholders
            .iter()
            .map(Diagram::max_generator)
            .map(|g| g.generator)
            .collect();
        for &placeholder in &placeholders {
            let node = sig.find_node(placeholder).unwrap();
            sig.update(&SignatureEdit::Edit(
                node,
                SignatureItemEdit::MakePlaceholder(true),
            ))
            .unwrap();
        }
        let schema = generator(&s);
        let node = sig.find_node(schema).unwrap();
        sig.update(&SignatureEdit::Edit(
            node,
            SignatureItemEdit::SetDependsOn(vec![*placeholders.last().unwrap()]),
        ))
        .unwrap();
        Chain {
            sig,
            schema,
            placeholders,
            cells,
        }
    }

    #[test]
    fn a_long_chain_is_filled_from_the_top_down() {
        // Only the top is chosen; it determines the two cells it goes between, which determine
        // the cells that they go between, and so on down to the points.
        for length in [2, 4, 5] {
            let ch = chain(length);
            let top = ch.placeholders.len() - 1;
            assert_eq!(ch.sig.parameters(ch.schema).len(), ch.placeholders.len());
            let found = ch
                .sig
                .infer_replacements(ch.schema, &[(ch.placeholders[top], ch.cells[top].clone())]);
            let expected: Vec<_> = ch.placeholders[..top]
                .iter()
                .copied()
                .zip(ch.cells[..top].iter().cloned())
                .collect();
            let mut found = found;
            found.sort_by_key(|(p, _)| (p.dimension, p.id));
            assert_eq!(found, expected, "a chain of length {length}");

            // The same through the checks: each of the cells is accepted given the top, and
            // none other of the same dimension is.
            let chosen = [(ch.placeholders[top], ch.cells[top].clone())];
            for (p, c) in expected {
                assert_eq!(ch.sig.check_replacement(ch.schema, &chosen, p, &c), Ok(()));
            }
        }
    }

    #[test]
    fn a_chain_filled_from_the_middle_goes_down_only() {
        let ch = chain(5);
        // `A₂ : A₁ ⇒ A₁'` determines `A₁, A₁'` and everything below them, and nothing above.
        let k = 6;
        let found = ch
            .sig
            .infer_replacements(ch.schema, &[(ch.placeholders[k], ch.cells[k].clone())]);
        assert_eq!(found.len(), k);
        assert!(found.iter().all(|(p, _)| ch.placeholders[..k].contains(p)));
        // A wrong cell at the bottom is caught by the others.
        let wrong = ch.cells[k - 1].clone();
        let chosen = [(ch.placeholders[k], ch.cells[k].clone())];
        assert!(ch
            .sig
            .check_replacement(ch.schema, &chosen, ch.placeholders[k - 2], &wrong)
            .is_err());
    }

    fn generator_of(d: &Diagram) -> Generator {
        d.max_generator().generator
    }

    /// The replacement of a placeholder in a list of them.
    fn of(replacements: &[(Generator, Diagram)], placeholder: Generator) -> Option<&Diagram> {
        replacements
            .iter()
            .find(|(p, _)| *p == placeholder)
            .map(|(_, d)| d)
    }

    #[test]
    fn the_inverse_of_a_square_determines_its_corners_and_leaves_the_cuts() {
        let sq = square();
        // `k⁻¹ : l ≫ l ⇛ p ≫ z` for `h : μ ≫ η' ⇛ η ≫ ν`. The corners are determined, and so is
        // `F'`, as `T` is all that `l ≫ l` goes through. How `l ≫ l` is cut into `μ ≫ η'` and
        // `p ≫ z` into `η ≫ ν` is not: three ways each, as either may be an identity.
        let inference = sq.sig.infer(sq.schema, &[(sq.h, sq.ki.clone())]);
        assert_eq!(inference.status, Status::Complete);
        assert_eq!(
            inference.determined,
            vec![
                (sq.f, sq.t.clone()),
                (sq.f2, sq.t.clone()),
                (sq.g2, sq.t.clone())
            ]
        );
        assert!(inference.exhaustive);
        assert_eq!(inference.candidates.len(), 9);
        // Each candidate fills in `G, η, η', μ, ν`, and the one without identities is first.
        assert!(inference.candidates.iter().all(|c| c.len() == 5));
        let first = &inference.candidates[0];
        assert_eq!(of(first, sq.g), Some(&sq.u));
        assert_eq!(of(first, sq.eta), Some(&sq.p));
        assert_eq!(of(first, sq.nu), Some(&sq.z));
        assert_eq!(of(first, sq.mu), Some(&sq.l));
        assert_eq!(of(first, sq.eta2), Some(&sq.l));
        for candidate in &inference.candidates[1..] {
            assert_ne!(candidate, first);
        }
        // The ambiguity is reported, with the cells that decide it.
        assert_eq!(inference.ambiguities.len(), 2);
        assert!(inference.ambiguities.iter().all(|a| a.ways == Some(3)));
        let open: Vec<_> = inference.ambiguities.iter().map(|a| a.open.len()).collect();
        assert_eq!(open, vec![2, 2]);
        let notes = sq.sig.explain_inference(&inference);
        assert_eq!(notes.len(), 2);
        assert!(notes.iter().all(|n| n.contains("can be cut 3 ways")));
        let name = |g: Generator| sq.sig.generator_info(g).unwrap().name.clone();
        assert!(notes.iter().any(|n| n.contains(&format!(
            "choose {} or {} to decide",
            name(sq.mu),
            name(sq.eta2)
        ))));
        assert!(!sq.sig.describe_filling(first).is_empty());

        // `η := p` decides one cut and leaves the other.
        let chosen = [(sq.h, sq.ki.clone()), (sq.eta, sq.p.clone())];
        let inference = sq.sig.infer(sq.schema, &chosen);
        assert_eq!(inference.candidates.len(), 3);
        for (placeholder, diagram) in [(sq.g, &sq.u), (sq.nu, &sq.z), (sq.f, &sq.t)] {
            assert_eq!(of(&inference.determined, placeholder), Some(diagram));
        }
        assert_eq!(inference.ambiguities.len(), 1);

        // And `μ := l` decides the other: everything follows.
        let chosen = [
            (sq.h, sq.ki.clone()),
            (sq.eta, sq.p.clone()),
            (sq.mu, sq.l.clone()),
        ];
        let inference = sq.sig.infer(sq.schema, &chosen);
        assert!(inference.candidates.is_empty());
        assert!(inference.ambiguities.is_empty());
        assert_eq!(of(&inference.determined, sq.eta2), Some(&sq.l));
        assert_eq!(inference.determined.len(), 6);
    }

    #[test]
    fn the_square_as_it_is_conflicts_with_the_cell_it_does_not_fit() {
        let sq = square();
        // `k : p ≫ z ⇛ l ≫ l` is the other way round: `η ≫ ν` must be `l ≫ l`, so `η` starts
        // with `l` and `η := p` cannot be. This is found at once, and says why.
        let chosen = [(sq.h, sq.k.clone())];
        let inference = sq
            .sig
            .infer(sq.schema, &[(sq.h, sq.k.clone()), (sq.eta, sq.p.clone())]);
        assert!(matches!(
            inference.status,
            Status::Inconsistent(ReplacementError::Uncuttable { .. })
        ));
        assert!(inference.determined.is_empty() && inference.candidates.is_empty());

        let Err(error) = sq.sig.check_replacement(sq.schema, &chosen, sq.eta, &sq.p) else {
            panic!("expected the replacement to be refused");
        };
        let message = sq.sig.describe_replacement_error(sq.eta, &error);
        let name = |g: Generator| sq.sig.generator_info(g).unwrap().name.clone();
        let (eta, h) = (name(sq.eta), name(sq.h));
        assert!(
            message.starts_with(&format!(
                "{eta} conflicts with {h}: {h} needs {eta} \u{226b} {} to be",
                name(sq.nu)
            )),
            "{message}"
        );
        assert!(
            message.ends_with(&format!("but {eta} is {}", name(generator_of(&sq.p)))),
            "{message}"
        );
        // The other way round, it is the earlier choice that is blamed.
        let entries = [(sq.eta, sq.p.clone()), (sq.h, sq.k.clone())];
        let results = sq.sig.check_replacements(sq.schema, &entries);
        assert_eq!(results[0], Ok(()));
        let Err(error) = &results[1] else {
            panic!("expected the replacement to be refused");
        };
        let message = sq.sig.describe_replacement_error(sq.h, error);
        assert!(
            message.starts_with(&format!("{} conflicts with {}: ", name(sq.h), name(sq.eta))),
            "{message}"
        );
    }

    #[test]
    fn inference_can_be_done_in_steps_and_dropped() {
        let sq = square();
        let mut job = InferJob::new(sq.sig.clone(), sq.schema, &[(sq.h, sq.ki.clone())]);
        let mut steps = 0;
        let inference = loop {
            steps += 1;
            if let Some(inference) = job.step() {
                break inference;
            }
            assert!(steps < 10_000);
        };
        // The 9 ways take a step each at least, and it is the same as doing it in one go.
        assert!(steps > 9, "{steps}");
        assert_eq!(inference, sq.sig.infer(sq.schema, &[(sq.h, sq.ki.clone())]));
        // Halfway, it has not touched anything; dropping it is all that cancelling takes.
        let mut job = InferJob::new(sq.sig.clone(), sq.schema, &[(sq.h, sq.ki.clone())]);
        assert!(job.step().is_none());
        drop(job);
    }

    #[test]
    fn instances_in_a_type_say_what_their_placeholders_are_replaced_with() {
        // `A : x → x` is a placeholder, `L : x → x` depends on it (as a limit does on its
        // diagram), and `γ : L ⇛ L` is a placeholder too. `A` is not mentioned by anything the
        // replacement of `γ` is made of, but `L[a]` is an instance of `L`, so `γ := γ₀` for
        // `γ₀ : L[a] ⇛ L[a]` can only be at `A := a`.
        let mut sig = Signature::default();
        let x: Diagram = sig.create_generator_zero("x").into();
        let cell = |sig: &mut Signature, s: Diagram, t: Diagram, name: &str| {
            sig.create_generator(s, t, name, Invertibility::Directed)
                .unwrap()
        };
        let a = cell(&mut sig, x.clone(), x.clone(), "A");
        let a0 = cell(&mut sig, x.clone(), x.clone(), "a");
        let l = cell(&mut sig, x.clone(), x.clone(), "L");
        let gamma = cell(&mut sig, l.clone().into(), l.clone().into(), "gamma");
        let s = cell(&mut sig, x.clone(), x, "S");
        let [a, l, gamma, s] = [&a, &l, &gamma, &s].map(generator);
        for placeholder in [a, gamma] {
            let node = sig.find_node(placeholder).unwrap();
            sig.update(&SignatureEdit::Edit(
                node,
                SignatureItemEdit::MakePlaceholder(true),
            ))
            .unwrap();
        }
        for (g, depends_on) in [(l, a), (s, gamma)] {
            let node = sig.find_node(g).unwrap();
            sig.update(&SignatureEdit::Edit(
                node,
                SignatureItemEdit::SetDependsOn(vec![depends_on]),
            ))
            .unwrap();
        }
        let l0 = sig.instantiate(l, &[(a, a0.clone().into())]).unwrap();
        let l0_diagram: DiagramN = sig
            .generator_info(l0)
            .unwrap()
            .diagram
            .clone()
            .try_into()
            .unwrap();
        let gamma0 = cell(
            &mut sig,
            l0_diagram.clone().into(),
            l0_diagram.into(),
            "gamma0",
        );
        assert_eq!(sig.parameters(s), vec![a, gamma]);

        let inference = sig.infer(s, &[(gamma, gamma0.into())]);
        assert_eq!(inference.determined, vec![(a, a0.into())]);
        assert_eq!(inference.status, Status::Complete);
    }

    #[test]
    fn a_cell_determines_its_boundary_and_nothing_else() {
        let sq = square();
        // `η : F ⇒ G` replaced by `p : T ⇒ U` fills `F` and `G`. Placeholders that nothing
        // mentions stay empty, however few cells there are to fill them with.
        assert_eq!(
            sq.sig
                .infer_replacements(sq.schema, &[(sq.eta, sq.p.clone())]),
            vec![(sq.f, sq.t.clone()), (sq.g, sq.u.clone())]
        );
        assert_eq!(
            sq.sig
                .infer_replacements(sq.schema, &[(sq.eta2, sq.z.clone())]),
            vec![(sq.f2, sq.u.clone()), (sq.g2, sq.t.clone())]
        );
    }

    #[test]
    fn replacements_are_checked_as_they_are_chosen() {
        let sq = square();
        let check = |chosen: &[(Generator, Diagram)], p, v: &Diagram| {
            sq.sig.check_replacement(sq.schema, chosen, p, v)
        };
        assert_eq!(check(&[], sq.f, &sq.t), Ok(()));
        assert_eq!(check(&[], sq.eta, &sq.p), Ok(()));
        assert_eq!(
            check(&[], sq.schema, &sq.t),
            Err(ReplacementError::NotAPlaceholder)
        );

        // The wrong dimension.
        let Err(ReplacementError::Dimension {
            expected, found, ..
        }) = check(&[(sq.eta, sq.p.clone())], sq.f, &sq.p)
        else {
            panic!("expected a dimension error");
        };
        assert_eq!((expected, found), (1, 2));

        // `η := p` needs `F := T` and `G := U`.
        let chosen = [(sq.eta, sq.p.clone())];
        assert_eq!(check(&chosen, sq.f, &sq.t), Ok(()));
        let Err(error) = check(&chosen, sq.f, &sq.u) else {
            panic!("expected a conflict");
        };
        assert_eq!(
            error,
            ReplacementError::Conflict {
                placeholder: sq.f,
                existing: sq.t.clone(),
                existing_because: sq.eta,
                proposed: sq.u.clone(),
                proposed_because: sq.f,
            }
        );
        assert!(!sq.sig.describe_replacement_error(sq.f, &error).is_empty());

        // `η := p` is a cell from `T` to `U`, not from `T` to `T` as `k` is.
        assert!(check(&[(sq.h, sq.k.clone())], sq.eta, &sq.p).is_err());
        assert!(check(&[(sq.h, sq.k.clone())], sq.eta, &sq.l).is_ok());
        // And the other way round.
        assert!(check(&[(sq.eta, sq.p.clone())], sq.h, &sq.k).is_err());
    }
}
