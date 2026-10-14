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
//! - A cell that is not a placeholder but depends on some, matched with one of its instances,
//!   binds the placeholders that the instance was made with.
//!
//! A placeholder that is bound has a type of its own, which is matched in turn with the boundary
//! of its replacement, so what a replacement determines is followed down to the bottom (a chain of
//! any length) before anything is tried.
//!
//! Every placeholder is determined if all the solutions agree on it. The solutions themselves are
//! kept: when there are several, they are the candidates that the person can pick one of (the
//! one with the fewest identities among the cells that were cut comes first, which is only an
//! order: nothing is picked). The search is bounded: when it has to stop early, only what matching
//! has forced (before any split was tried) is determined, because that holds in every solution.
//!
//! The search is a [`Machine`] with a stack of its own, so that it can be done a step at a time
//! as a [`Job`] ([`InferJob`]): a step is one way of cutting or one candidate.

use std::rc::Rc;

use homotopy_common::{hash::FastHashMap, tex::seal};
use homotopy_core::{
    signature::{Invertibility, Signature as _},
    Diagram, DiagramN, Generator, Orientation,
};

use super::{
    describe::CompositePart,
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
    /// The replacement chosen for a placeholder has something where the type of another requires
    /// something else.
    Mismatch(Box<Mismatch>),
    /// A diagram of a replacement chosen has to be cut into the replacements of the cells of a
    /// composite, but cannot be.
    Uncuttable(Box<Uncuttable>),
}

/// What is wrong with a replacement: it has `found` where a type requires `expected`.
///
/// The replacement `chosen` for `because` has `found` where the type of `owner` requires
/// `expected` (with placeholders in it), at `path` from `chosen` (which is empty if it is
/// `chosen` itself, and `[Target]` if it is its target). `bound` is what the placeholders that
/// `expected` depends on are replaced with so far.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mismatch {
    pub because: Generator,
    pub chosen: Diagram,
    pub path: Vec<Part>,
    pub owner: Generator,
    pub expected: Diagram,
    pub found: Diagram,
    pub bound: Vec<(Generator, Diagram)>,
}

/// What is wrong with a replacement: a diagram of it cannot be cut as a type requires.
///
/// A diagram of the replacement `chosen` for `because`, at `path` from it, has to be cut into the
/// replacements of `atoms` (the type of the placeholder says it is their composite), but cannot
/// be cut so that the pieces are those already `known` for some of them. `bound` is what the
/// placeholders that `atoms` depend on are replaced with so far.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Uncuttable {
    pub because: Generator,
    pub chosen: Diagram,
    pub path: Vec<Part>,
    pub bound: Vec<(Generator, Diagram)>,
    pub atoms: Vec<Generator>,
    pub value: Diagram,
    /// The atoms that are already replaced, with their replacements and the replacement chosen
    /// that they follow from.
    pub known: Vec<(Generator, Diagram, Generator)>,
}

/// A step from a diagram to a part of it, for saying where in the replacement chosen for a
/// placeholder something is wrong.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Part {
    Source,
    Target,
    /// Some of its cells, or what a cell in it was made with.
    Piece,
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
    /// Where `value` is in the replacement chosen that this follows from.
    path: Vec<Part>,
}

/// `path` and one more step.
fn deeper(path: &[Part], step: Part) -> Vec<Part> {
    let mut path = path.to_vec();
    path.push(step);
    path
}

/// A whole cell in a vertical composite.
#[derive(Clone)]
struct Atom {
    generator: Generator,
    /// Whether it is the inverse of the cell of the generator.
    reflected: bool,
    /// Whether the generator is not replaced, as it neither is nor depends on a placeholder: its
    /// replacement is the cell itself.
    plain: bool,
}

/// A requirement that a vertical composite of whole cells is the composite of the replacements
/// of its cells, so that the diagram it is matched with is cut into their replacements.
#[derive(Clone)]
struct Split {
    atoms: Vec<Atom>,
    value: DiagramN,
    root: Generator,
    owner: Generator,
    path: Vec<Part>,
}

#[derive(Clone, Default)]
struct State {
    /// The replacement of each parameter that matching has found, and what it follows from.
    bound: Vec<Option<(Diagram, Generator)>>,
    pending: Vec<Constraint>,
    splits: Vec<Split>,
    /// The placeholders whose types were not matched exactly: they are checked by substituting.
    inexact: Vec<Generator>,
    /// The replacements chosen, for messages.
    roots: Vec<(Generator, Diagram)>,
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
        path: Vec<Part>,
    ) {
        if let Some(own) = self.signature.generator_info(placeholder) {
            state.pending.push(Constraint {
                pattern: own.diagram.clone(),
                value,
                root,
                owner: placeholder,
                path,
            });
        }
    }

    /// Requires `value` to replace `placeholder`, as chosen.
    fn choose(&self, state: &mut State, placeholder: Generator, value: Diagram) {
        state.roots.push((placeholder, value.clone()));
        self.require_replacement(state, placeholder, value, placeholder, vec![]);
    }

    /// The replacement chosen for `root`, or `fallback`.
    fn chosen(state: &State, root: Generator, fallback: &Diagram) -> Diagram {
        state
            .roots
            .iter()
            .rev()
            .find(|(p, _)| *p == root)
            .map_or_else(|| fallback.clone(), |(_, d)| d.clone())
    }

    /// What the placeholders that a diagram depends on are bound to so far.
    fn bindings(&self, state: &State, diagram: &Diagram) -> Vec<(Generator, Diagram)> {
        self.bindings_of(state, diagram.generators().into_keys())
    }

    /// What the placeholders that some generators depend on are bound to so far.
    fn bindings_of(
        &self,
        state: &State,
        generators: impl IntoIterator<Item = Generator>,
    ) -> Vec<(Generator, Diagram)> {
        let mut mentioned: Vec<Generator> = vec![];
        for g in generators {
            if self.index(g).is_some() {
                mentioned.push(g);
            } else {
                mentioned.extend(self.signature.parameters(g));
            }
        }
        self.parameters
            .iter()
            .zip(&state.bound)
            .filter(|(p, _)| mentioned.contains(*p))
            .filter_map(|(&p, b)| b.as_ref().map(|(d, _)| (p, d.clone())))
            .collect()
    }

    /// The whole cells that a diagram is a vertical composite of, if it is one.
    fn atoms(&self, diagram: &DiagramN) -> Option<Vec<Atom>> {
        let slices: Vec<Diagram> = diagram.slices().collect();
        let mut atoms = Vec::with_capacity(diagram.size());
        let mut plain_generators = FastHashMap::default();
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
            let plain = self.index(generator).is_none()
                && *plain_generators
                    .entry(generator)
                    .or_insert_with(|| self.signature.parameters(generator).is_empty());
            atoms.push(Atom {
                generator,
                reflected,
                plain,
            });
        }
        Some(atoms)
    }

    /// A cell that is not a placeholder but depends on some (such as the limit of a diagram `D`,
    /// which depends on `D`) is replaced by the cell of its instance. If `piece` is one, the
    /// instance says what the placeholders it depends on are replaced with, so that they are
    /// bound without searching.
    ///
    /// The cell may itself be an instance of a schema made with replacements that mention
    /// placeholders, such as `(Ẋη̇)₍₂₎ = η̇₍₂₎[Ḟ ≫ Ẋ/Ḟ, Ġ ≫ Ẋ/Ġ, η̇ ▷ Ẋ/η̇]`. Its replacement is the
    /// instance of the same schema whose replacements are those with the placeholders replaced, so
    /// each replacement it was made with is matched with the one of the instance in `piece`.
    fn match_instance(
        &self,
        state: &mut State,
        atom: &Atom,
        piece: &DiagramN,
        root: Generator,
        owner: Generator,
        path: &[Part],
    ) {
        if self.index(atom.generator).is_some() {
            return;
        }
        let path = deeper(path, Part::Piece);
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
        if info.diagram != Diagram::from(cell) {
            return;
        }
        if made.schema == atom.generator {
            for (placeholder, replacement) in &made.substitution {
                if self.index(*placeholder).is_some() {
                    self.require_replacement(
                        state,
                        *placeholder,
                        replacement.clone(),
                        root,
                        path.clone(),
                    );
                }
            }
            return;
        }
        // The cell is an instance of the schema of `piece` with placeholders in what it was made
        // with, or it is not an instance of the same schema and nothing is known.
        let Some(pattern) = self
            .signature
            .generator_info(atom.generator)
            .and_then(|info| info.instance.as_ref())
            .filter(|pattern| pattern.schema == made.schema)
            .map(|pattern| &pattern.substitution)
        else {
            return;
        };
        let replaced = |placeholder: Generator, substitution: &[(Generator, Diagram)]| {
            substitution
                .iter()
                .find(|(p, _)| *p == placeholder)
                .map(|(_, d)| d.clone())
                .or_else(|| {
                    self.signature
                        .generator_info(placeholder)
                        .map(|info| info.diagram.clone())
                })
        };
        for (placeholder, replacement) in pattern {
            if let Some(value) = replaced(*placeholder, &made.substitution) {
                state.pending.push(Constraint {
                    pattern: replacement.clone(),
                    value,
                    root,
                    owner,
                    path: path.clone(),
                });
            }
        }
        // What the schema's own placeholders that the cell leaves alone are replaced with.
        for (placeholder, value) in &made.substitution {
            if self.index(*placeholder).is_some() && !pattern.iter().any(|(p, _)| p == placeholder)
            {
                self.require_replacement(state, *placeholder, value.clone(), root, path.clone());
            }
        }
    }

    fn unify(&self, state: &mut State, constraint: Constraint) -> Result<(), ReplacementError> {
        let Constraint {
            pattern,
            value,
            root,
            owner,
            path,
        } = constraint;
        let (expected, found) = (pattern.clone(), value.clone());
        let mismatch = |state: &State| {
            ReplacementError::Mismatch(Box::new(Mismatch {
                because: root,
                chosen: Self::chosen(state, root, &found),
                path: path.clone(),
                owner,
                bound: self.bindings(state, &expected),
                expected: expected.clone(),
                found: found.clone(),
            }))
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
                    Err(mismatch(state))
                }
            }
            (Diagram::DiagramN(s), Diagram::DiagramN(v)) => {
                if s.dimension() != v.dimension() {
                    return Err(mismatch(state));
                }
                let generator = s.max_generator().generator;
                if let Some(index) = self
                    .index(generator)
                    .filter(|_| generator.dimension == s.dimension())
                {
                    let own = self.own(generator).ok_or_else(|| mismatch(state))?;
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
                        for (step, p, v) in [
                            (Part::Source, own.source(), bound.source()),
                            (Part::Target, own.target(), bound.target()),
                        ] {
                            state.pending.push(Constraint {
                                pattern: p,
                                value: v,
                                root,
                                owner: generator,
                                path: deeper(&path, step),
                            });
                        }
                        return Ok(());
                    }
                }

                for (step, p, v) in [
                    (Part::Source, s.source(), v.source()),
                    (Part::Target, s.target(), v.target()),
                ] {
                    state.pending.push(Constraint {
                        pattern: p,
                        value: v,
                        root,
                        owner,
                        path: deeper(&path, step),
                    });
                }
                if s.size() == 0 {
                    // The identity of `σ(X)` is an identity.
                    if v.size() != 0 {
                        return Err(mismatch(state));
                    }
                    return Ok(());
                }
                match self.atoms(s) {
                    Some(atoms) if atoms.len() == 1 => {
                        // One cell that is not a placeholder: its replacement is one height.
                        if v.size() != 1 {
                            return Err(mismatch(state));
                        }
                        if atoms[0].plain {
                            // And it is the cell itself.
                            if s != v {
                                return Err(mismatch(state));
                            }
                        } else {
                            Self::mark(state, owner);
                            self.match_instance(state, &atoms[0], v, root, owner, &path);
                        }
                    }
                    Some(atoms) => {
                        if atoms
                            .iter()
                            .any(|a| self.index(a.generator).is_none() && !a.plain)
                        {
                            Self::mark(state, owner);
                        }
                        state.splits.push(Split {
                            atoms,
                            value: v.clone(),
                            root,
                            owner,
                            path,
                        });
                    }
                    None => Self::mark(state, owner),
                }
                Ok(())
            }
            _ => Err(mismatch(state)),
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
                None => match self.own(atom.generator).filter(|_| atom.plain) {
                    // The cell itself.
                    Some(own) => Want::Piece(if atom.reflected { own.inverse() } else { own }),
                    None => Want::Size(1),
                },
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
                    path: deeper(&split.path, Part::Piece),
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
        let value: Diagram = split.value.clone().into();
        let atoms: Vec<Generator> = split.atoms.iter().map(|atom| atom.generator).collect();
        ReplacementError::Uncuttable(Box::new(Uncuttable {
            because: split.root,
            chosen: Self::chosen(state, split.root, &value),
            path: split.path.clone(),
            bound: self.bindings_of(state, atoms.iter().copied()),
            atoms,
            value,
            known,
        }))
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
    /// For each placeholder, the positions of the placeholders its type mentions, once they are
    /// needed.
    mentions: Option<Vec<Vec<usize>>>,
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
            mentions: None,
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
            matcher.choose(&mut state, *p, d.clone());
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
        let mentions = self.mentions.get_or_insert_with(|| {
            parameters
                .iter()
                .map(|&p| {
                    signature
                        .parameters(p)
                        .into_iter()
                        .filter_map(|q| parameters.iter().position(|&r| r == q))
                        .collect()
                })
                .collect()
        });

        // The placeholders that matter: those bound and those that their types mention.
        let mut needed: Vec<bool> = state.bound.iter().map(Option::is_some).collect();
        loop {
            let mut changed = false;
            for i in 0..parameters.len() {
                if needed[i] {
                    for &j in &mentions[i] {
                        if !needed[j] {
                            needed[j] = true;
                            changed = true;
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
                let when = mentions[i].iter().map(|j| j + 1).max().unwrap_or(0);
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
        let cells =
            Pool::of_diagrams(&signature, chosen.iter().map(|(_, d)| d.clone()).collect()).cells();
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

    /// How far along it is is not known: the replacements tried (`nodes`) are only bounded by
    /// [`MAX_COMPLETION_NODES`], which most searches end far short of.
    fn progress(&self) -> Option<Progress> {
        None
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
        matcher.choose(&mut state, placeholder, value.clone());
        matcher.propagate(&mut state)?;
        Ok(state)
    }

    pub(super) fn name_sealed(&self, g: Generator) -> String {
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
        // "X conflicts with Y: " if the error is about the replacement of Y, and nothing if it is
        // about the replacement of X itself.
        let blame = |others: &[Generator]| {
            if others.iter().all(|&g| g == placeholder) {
                String::new()
            } else {
                conflicts(others)
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
            ReplacementError::Mismatch(mismatch) => {
                let Mismatch {
                    because,
                    chosen,
                    path,
                    owner,
                    expected,
                    found,
                    bound,
                } = &**mismatch;
                format!(
                    "{}{} := {}: {} is {}, but the type of {} needs {}",
                    blame(&[*because]),
                    name(*because),
                    self.describe(chosen),
                    Self::describe_place(path),
                    self.describe(found),
                    name(*owner),
                    self.describe_needed(*owner, expected, bound),
                )
            }
            ReplacementError::Uncuttable(uncuttable) => {
                let Uncuttable {
                    because,
                    chosen,
                    path,
                    bound,
                    atoms,
                    value,
                    known,
                } = &**uncuttable;
                let blamed: Vec<Generator> = std::iter::once(*because)
                    .chain(known.iter().map(|(_, _, b)| *b))
                    .collect();
                format!(
                    "{}{} := {}: {} is {}, but the type requires {} there",
                    blame(&blamed),
                    name(*because),
                    self.describe(chosen),
                    Self::describe_place(path),
                    self.describe(value),
                    self.describe_composite_needed(atoms, bound),
                )
            }
        }
    }

    /// Where in a replacement something is, like "its target" or "the target of its source".
    fn describe_place(path: &[Part]) -> String {
        let mut place = "it".to_owned();
        for (i, step) in path.iter().enumerate() {
            let first = i == 0;
            place = match (step, first) {
                (Part::Source, true) => "its source".to_owned(),
                (Part::Target, true) => "its target".to_owned(),
                (Part::Piece, true) => "a part of it".to_owned(),
                (Part::Source, false) => format!("the source of {place}"),
                (Part::Target, false) => format!("the target of {place}"),
                (Part::Piece, false) => format!("a part of {place}"),
            };
        }
        place
    }

    /// What a type requires, with the placeholders in it replaced with what they are replaced with
    /// so far: "`B`" if nothing is, and otherwise "`A` (`B` with `X := a`)".
    fn describe_needed(
        &self,
        owner: Generator,
        expected: &Diagram,
        bound: &[(Generator, Diagram)],
    ) -> String {
        let needed = Pool::new(self, [])
            .substitute(owner, bound, expected, false)
            .filter(|needed| needed != expected);
        let Some(needed) = needed else {
            return self.describe(expected);
        };
        let bindings = bound
            .iter()
            .map(|(p, d)| format!("{} := {}", self.name_sealed(*p), self.describe(d)))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "{} ({} with {})",
            self.describe(&needed),
            self.describe(expected),
            bindings
        )
    }

    /// What a composite of the cells of a schema needs to be, with the replacements so far put in:
    /// "`B`" if there are none, and otherwise "`A` (`B` with `X := a`)", as for
    /// [`describe_needed`](Self::describe_needed).
    fn describe_composite_needed(
        &self,
        atoms: &[Generator],
        bound: &[(Generator, Diagram)],
    ) -> String {
        let pool = Pool::new(self, []);
        let replaced: Vec<CompositePart> = atoms
            .iter()
            .map(|&atom| {
                // A placeholder is replaced with what it is bound to, and a cell that depends on
                // placeholders with its instance, if there is one.
                bound
                    .iter()
                    .find(|(p, _)| *p == atom)
                    .map(|(_, d)| d.clone())
                    .or_else(|| {
                        let parameters = self.parameters(atom);
                        let restricted: Vec<_> = bound
                            .iter()
                            .filter(|(p, _)| parameters.contains(p))
                            .cloned()
                            .collect();
                        let restricted = self.normalise(&restricted);
                        if restricted.is_empty() {
                            None
                        } else {
                            pool.instance_of(atom, &restricted, true)
                        }
                    })
                    .map_or(CompositePart::Cell(atom), CompositePart::Diagram)
            })
            .collect();
        let needed = self.describe_composite_of(&replaced);
        let composite = self.describe_composite(atoms);
        if needed == composite {
            return composite;
        }
        let bindings = bound
            .iter()
            .map(|(p, d)| format!("{} := {}", self.name_sealed(*p), self.describe(d)))
            .collect::<Vec<_>>()
            .join(", ");
        format!("{needed} ({composite} with {bindings})")
    }

    /// The cells of a composite, like `μ ; η'`, written in the notation of names (see
    /// [`notation`](super::notation)) like the diagrams that are described.
    fn describe_composite(&self, cells: &[Generator]) -> String {
        let cells: Vec<_> = cells.iter().map(|&g| CompositePart::Cell(g)).collect();
        self.describe_composite_of(&cells)
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
                    ReplacementError::Mismatch(m) => Some(m.because),
                    ReplacementError::Uncuttable(u) => Some(u.because),
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
pub(in crate::proof) mod tests {
    use homotopy_common::tex::to_math;
    use homotopy_core::{
        signature::{Invertibility, Signature as _},
        typecheck::{typecheck, Mode},
        Boundary, Diagram, DiagramN, Generator,
    };

    use super::{InferJob, Part, ReplacementError, Status};
    use crate::{
        job::Job,
        proof::{
            notation::{Notation, Standard},
            InstantiationError, Signature, SignatureEdit, SignatureItemEdit,
        },
    };

    /// How a message writes a composite of cells with these names (or labels): as one formula, in
    /// the notation of names.
    pub(in crate::proof) fn composite(parts: &[String]) -> String {
        let math: Vec<String> = parts.iter().map(|part| to_math(part)).collect();
        format!("${}$", Notation::compose(&Standard, &math))
    }

    /// A commutative square, as in a naturality condition: `x`; placeholders `F, G, F', G' : x → x`,
    /// `η : F ⇒ G`, `η' : F' ⇒ G'`, `μ : F ⇒ F'`, `ν : G ⇒ G'` and `h : μ ≫ η' ⇛ η ≫ ν`; the cells
    /// `T, U : x → x`, `p : T ⇒ U`, `z : U ⇒ T`, `l : T ⇒ T` and `k : p ≫ z ⇛ l ≫ l`; and a
    /// schema `s : x → x` that depends on `h`.
    pub(in crate::proof) struct Square {
        pub(in crate::proof) sig: Signature,
        pub(in crate::proof) schema: Generator,
        pub(in crate::proof) f: Generator,
        pub(in crate::proof) g: Generator,
        pub(in crate::proof) f2: Generator,
        pub(in crate::proof) g2: Generator,
        pub(in crate::proof) eta: Generator,
        pub(in crate::proof) eta2: Generator,
        pub(in crate::proof) mu: Generator,
        pub(in crate::proof) nu: Generator,
        pub(in crate::proof) h: Generator,
        pub(in crate::proof) t: Diagram,
        pub(in crate::proof) u: Diagram,
        pub(in crate::proof) p: Diagram,
        pub(in crate::proof) z: Diagram,
        pub(in crate::proof) l: Diagram,
        pub(in crate::proof) k: Diagram,
        /// The inverse of `k`.
        pub(in crate::proof) ki: Diagram,
        /// `m : p ≫ z ⇛ p ≫ z`, which fits the square when `η := p` and `η' := z`.
        pub(in crate::proof) m: Diagram,
    }

    fn generator(d: &DiagramN) -> Generator {
        d.max_generator().generator
    }

    pub(in crate::proof) fn square() -> Square {
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
        let m = cell(&mut sig, pz.clone().into(), pz.clone().into(), "m");
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
            m: m.into(),
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
            Status::Inconsistent(ReplacementError::Uncuttable(_))
        ));
        assert!(inference.determined.is_empty() && inference.candidates.is_empty());

        let Err(error) = sq.sig.check_replacement(sq.schema, &chosen, sq.eta, &sq.p) else {
            panic!("expected the replacement to be refused");
        };
        let message = sq.sig.describe_replacement_error(sq.eta, &error);
        let name = |g: Generator| sq.sig.generator_info(g).unwrap().name.clone();
        let (eta, h) = (name(sq.eta), name(sq.h));
        // It says what `h` was replaced with and where in it the trouble is, and what `η` is.
        let k_target = DiagramN::try_from(sq.k.clone()).unwrap().target();
        assert!(
            message.starts_with(&format!("{eta} conflicts with {h}: {h} := ")),
            "{message}"
        );
        assert!(
            message.contains(&format!(
                ": its target is {}, but the type requires {} ({} with {eta} := {}) there",
                sq.sig.describe(&k_target),
                composite(&[name(generator_of(&sq.p)), name(sq.nu)]),
                composite(&[eta.clone(), name(sq.nu)]),
                name(generator_of(&sq.p)),
            )),
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

    /// `x`; placeholders `A, X : x → x`; `D : x → x` that depends on `A`; the schema `S : x → x`
    /// that depends on the placeholder `H : D ≫ X ⇒ E`, where `E = D[A ≫ X]` is the instance of
    /// `D` made with replacements that mention placeholders, so that it is still a different cell
    /// for every `A` and `X`. `t, a₀ : x → x` and the instances `D₀ = D[a₀]` and `D₁ = D[a₀ ≫ t]`,
    /// which is what `E` becomes at `A := a₀` and `X := t`.
    struct Instance {
        sig: Signature,
        schema: Generator,
        a: Generator,
        x: Generator,
        d: Generator,
        h: Generator,
        e: Generator,
        t: Diagram,
        a0: Diagram,
        d0: DiagramN,
        d1: DiagramN,
    }

    fn instance_at_placeholders() -> Instance {
        let mut sig = Signature::default();
        let zero: Diagram = sig.create_generator_zero("x").into();
        let cell = |sig: &mut Signature, s: Diagram, t: Diagram, name: &str| {
            sig.create_generator(s, t, name, Invertibility::Directed)
                .unwrap()
        };
        let a = cell(&mut sig, zero.clone(), zero.clone(), "A");
        let x = cell(&mut sig, zero.clone(), zero.clone(), "X");
        let d = cell(&mut sig, zero.clone(), zero.clone(), "D");
        let t = cell(&mut sig, zero.clone(), zero.clone(), "t");
        let a0 = cell(&mut sig, zero.clone(), zero.clone(), "a");
        let [a_gen, x_gen, d_gen] = [&a, &x, &d].map(generator);
        for placeholder in [a_gen, x_gen] {
            let node = sig.find_node(placeholder).unwrap();
            sig.update(&SignatureEdit::Edit(
                node,
                SignatureItemEdit::MakePlaceholder(true),
            ))
            .unwrap();
        }
        let node = sig.find_node(d_gen).unwrap();
        sig.update(&SignatureEdit::Edit(
            node,
            SignatureItemEdit::SetDependsOn(vec![a_gen]),
        ))
        .unwrap();

        let instance = |sig: &mut Signature, a_replacement: DiagramN| {
            let generator = sig
                .instantiate(d_gen, &[(a_gen, a_replacement.into())])
                .unwrap();
            let diagram: DiagramN = sig
                .generator_info(generator)
                .unwrap()
                .diagram
                .clone()
                .try_into()
                .unwrap();
            (generator, diagram)
        };
        let ax = a.attach(&x, Boundary::Target, &[]).unwrap();
        let a0t = a0.attach(&t, Boundary::Target, &[]).unwrap();
        let (e_gen, e) = instance(&mut sig, ax);
        let (_, d0) = instance(&mut sig, a0.clone());
        let (_, d1) = instance(&mut sig, a0t);

        let d_x = d.attach(&x, Boundary::Target, &[]).unwrap();
        let h = cell(&mut sig, d_x.into(), e.into(), "H");
        let h_gen = generator(&h);
        let s = cell(&mut sig, zero.clone(), zero, "S");
        let s_gen = generator(&s);
        for (g, edit) in [
            (h_gen, SignatureItemEdit::MakePlaceholder(true)),
            (s_gen, SignatureItemEdit::SetDependsOn(vec![h_gen])),
        ] {
            let node = sig.find_node(g).unwrap();
            sig.update(&SignatureEdit::Edit(node, edit)).unwrap();
        }
        Instance {
            sig,
            schema: s_gen,
            a: a_gen,
            x: x_gen,
            d: d_gen,
            h: h_gen,
            e: e_gen,
            t: t.into(),
            a0: a0.into(),
            d0,
            d1,
        }
    }

    #[test]
    fn an_instance_at_placeholders_depends_on_them() {
        let it = instance_at_placeholders();
        // `E` mentions neither `A` nor `X`, but it was made with `A ≫ X`.
        let mentioned = it.sig.generator_info(it.e).unwrap().diagram.generators();
        assert!(!mentioned.contains_key(&it.a) && !mentioned.contains_key(&it.x));
        assert_eq!(it.sig.parameters(it.e), vec![it.a, it.x]);
        // An instance made with cells that mention none of them depends on none.
        assert!(it.sig.parameters(generator(&it.d0)).is_empty());
        assert_eq!(it.sig.parameters(it.schema), vec![it.a, it.x, it.h]);

        // Instantiating it is instantiating `D` with those replaced: `E` at `a₀` and `t` is
        // `D[a₀ ≫ t]`, the instance that exists.
        let mut sig = it.sig.clone();
        let at_a0_t = sig
            .instantiate(it.e, &[(it.a, it.a0.clone()), (it.x, it.t.clone())])
            .unwrap();
        assert_eq!(
            sig.generator_info(at_a0_t).unwrap().diagram,
            Diagram::from(it.d1.clone())
        );
    }

    #[test]
    fn a_replacement_that_the_kernel_takes_is_not_refused() {
        let it = instance_at_placeholders();
        let d0_t = it
            .d0
            .attach(
                &DiagramN::try_from(it.t.clone()).unwrap(),
                Boundary::Target,
                &[],
            )
            .unwrap();
        let mut sig = it.sig.clone();
        let h0 = sig
            .create_generator(
                d0_t.into(),
                it.d1.clone().into(),
                "h0",
                Invertibility::Directed,
            )
            .unwrap();
        let chosen = vec![
            (it.x, it.t.clone()),
            (it.a, it.a0.clone()),
            (it.h, h0.clone().into()),
        ];
        // The check goes by the types alone, and agrees with the kernel.
        assert_eq!(
            sig.check_replacements(it.schema, &chosen),
            vec![Ok(()), Ok(()), Ok(())]
        );
        let instance = sig.instantiate(it.schema, &chosen).unwrap();
        let info = sig.generator_info(instance).unwrap();
        assert!(typecheck(&info.diagram, &sig, Mode::default(), true).is_ok());
        // And inferring from them has nothing against them.
        assert_eq!(sig.infer(it.schema, &chosen).status, Status::Complete);
    }

    #[test]
    fn a_replacement_the_kernel_refuses_is_refused_and_says_where() {
        let it = instance_at_placeholders();
        let mut sig = it.sig.clone();
        let t = DiagramN::try_from(it.t.clone()).unwrap();
        // `t ≫ D₀` instead of `D₀ ≫ t`: `D ≫ X` becomes `D₀ ≫ t`.
        let t_d0: Diagram = t.attach(&it.d0, Boundary::Target, &[]).unwrap().into();
        let wrong = sig
            .create_generator(
                t_d0.clone(),
                it.d1.clone().into(),
                "wrong",
                Invertibility::Directed,
            )
            .unwrap();
        let wrong = generator(&wrong);
        let chosen = vec![
            (it.x, it.t.clone()),
            (it.a, it.a0.clone()),
            (it.h, sig.generator_info(wrong).unwrap().diagram.clone()),
        ];
        let results = sig.check_replacements(it.schema, &chosen);
        assert_eq!(&results[..2], &[Ok(()), Ok(())]);
        let Err(error) = &results[2] else {
            panic!("expected the replacement to be refused");
        };
        assert!(matches!(
            error,
            ReplacementError::Uncuttable(uncuttable) if uncuttable.path == [Part::Source]
        ));
        assert!(matches!(
            sig.instantiate(it.schema, &chosen),
            Err(InstantiationError::IllTyped(_))
        ));

        // The message names the replacement chosen, where in it the trouble is, and what the
        // type says in terms of the others.
        let message = sig.describe_replacement_error(it.h, error);
        let name = |g: Generator| sig.generator_info(g).unwrap().name.clone();
        let (h, a, x, d) = (name(it.h), name(it.a), name(it.x), name(it.d));
        let (t_name, d0) = (name(generator(&t)), sig.describe(&it.d0.clone().into()));
        assert_eq!(
            message,
            format!(
                "{h} conflicts with {x}: {h} := {}: its source is {}, but the type requires {} ({} with {a} := {}, {x} := {t_name}) there",
                name(wrong),
                sig.describe(&t_d0),
                composite(&[d0, t_name.clone()]),
                composite(&[d, x.clone()]),
                sig.describe(&it.a0),
            )
        );
    }

    #[test]
    fn a_mismatch_says_what_the_type_needs_with_the_replacements_so_far() {
        let it = instance_at_placeholders();
        let mut sig = it.sig.clone();
        let t = DiagramN::try_from(it.t.clone()).unwrap();
        // A loop on `D₀ ≫ t`: its source is right and its target is not a single cell.
        let d0_t: Diagram = it.d0.attach(&t, Boundary::Target, &[]).unwrap().into();
        let around = sig
            .create_generator(d0_t.clone(), d0_t.clone(), "loop", Invertibility::Directed)
            .unwrap();
        let around = generator(&around);
        let chosen = vec![
            (it.x, it.t.clone()),
            (it.a, it.a0.clone()),
            (it.h, sig.generator_info(around).unwrap().diagram.clone()),
        ];
        let results = sig.check_replacements(it.schema, &chosen);
        let Err(error) = &results[2] else {
            panic!("expected the replacement to be refused");
        };
        assert!(matches!(
            error,
            ReplacementError::Mismatch(mismatch) if mismatch.path == [Part::Target]
        ));
        assert!(matches!(
            sig.instantiate(it.schema, &chosen),
            Err(InstantiationError::IllTyped(_))
        ));

        let message = sig.describe_replacement_error(it.h, error);
        let name = |g: Generator| sig.generator_info(g).unwrap().name.clone();
        let (h, a, x) = (name(it.h), name(it.a), name(it.x));
        assert_eq!(
            message,
            format!(
                "{h} := {}: its target is {}, but the type of {h} needs {} ({} with {a} := {}, {x} := {})",
                name(around),
                sig.describe(&d0_t),
                sig.describe(&it.d1.clone().into()),
                name(it.e),
                sig.describe(&it.a0),
                sig.describe(&it.t),
            )
        );

        // On a replacement for a placeholder that is not the one in the way it names only it.
        let message = sig.describe_replacement_error(it.a, error);
        assert!(
            message.starts_with(&format!("{a} conflicts with {h}: {h} := ")),
            "{message}"
        );
    }
}
