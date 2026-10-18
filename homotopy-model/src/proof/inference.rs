//! Inferring how to instantiate a schema so that it can be attached at a point of a diagram.
//!
//! Each placeholder is solved in turn, from the lowest dimension up, so that its type is known
//! when it is solved. Its candidates are generated from the cells of the diagram being attached
//! to: the 0-cells of that diagram for a 0-cell placeholder, and otherwise the composites of up to
//! [`MAX_CELLS`] of its cells of the placeholder's dimension, between the placeholder's source and
//! target. A substitution is kept if the instantiated schema then attaches at the given point.

use std::{
    cell::{OnceCell, RefCell},
    rc::Rc,
};

use homotopy_common::{
    deadline,
    hash::{FastHashMap, FastHashSet},
};
use homotopy_core::{
    signature::{Invertibility, Signature as _},
    substitution::substitute,
    Boundary, Diagram, DiagramN, Generator, Height,
};

use super::{
    completion::{seeds, Seeds},
    Signature,
};

/// The most cells in a composite replacing a placeholder.
pub(super) const MAX_CELLS: usize = 3;
/// The most candidates considered for each placeholder.
const MAX_CANDIDATES: usize = 64;
/// The most complete substitutions tried.
const MAX_SUBSTITUTIONS: usize = 512;
/// The most replacements considered, over the whole search, when completing a substitution.
pub(super) const MAX_COMPLETION_NODES: usize = 2048;
/// The most solutions found when completing a substitution before giving up: that many
/// solutions means that nothing is determined.
pub(super) const MAX_COMPLETIONS: usize = 32;

/// A way of attaching an instance of a schema, found by [`Signature::infer_instances`].
#[derive(Clone, Debug)]
pub struct InferredInstance {
    /// The substitution to instantiate the schema with.
    pub substitution: Vec<(Generator, Diagram)>,
    /// The diagram of the instance, labelled with the schema's generator.
    pub diagram: DiagramN,
    /// Where to attach it.
    pub embedding: Vec<usize>,
    /// Whether it is the inverse of the instance that is attached: the schema is invertible and
    /// it is its target that matches the diagram. `diagram` is then the inverse too.
    pub inverse: bool,
}

/// What the key of an instance found for the inverse of its schema starts with, so that they
/// come after all those found for the schema itself.
pub(super) const INVERSE_KEY: usize = u32::MAX as usize;
/// What the key of an instance found by reading the replacements off the diagram (see
/// [`seeds`]) starts with, followed by the number of the way: after the instances found by
/// searching for the replacements, and before the inverses.
pub(super) const SEEDED_KEY: usize = INVERSE_KEY - 1;

/// The cells that the replacements of placeholders are made from.
pub(super) struct Pool<'a> {
    pub(super) signature: &'a Signature,
    cells: Rc<Cells>,
}

/// What a [`Pool`] is made of, which can be kept for another pool of the same signature. The
/// cells are only made when they are wanted, which is not always.
pub(super) struct Cells {
    from: From,
    made: OnceCell<Made>,
    /// The cells that the instances in the diagram were made with, for the placeholders of a
    /// dimension higher than [`reach`](Self::reach): the diagram has no cells of that dimension
    /// to make them of (see [`Pool::reaching`]).
    beyond: OnceCell<Made>,
    /// The highest dimension of the cells that the diagram can have, or `usize::MAX` if the
    /// replacements of all the placeholders are made of the cells it has.
    reach: usize,
    /// The composites of the cells that start at a diagram, so far (see [`Composites`]): what the
    /// candidates for a placeholder are taken from, which depends on where they start and not on
    /// where they end, so that placeholders (of one schema, or of many) with the same source share
    /// the work. A placeholder that is invertible or dualisable is only replaced with composites
    /// of cells that are as much, so those are made of other cells and shared only with
    /// placeholders that are as invertible.
    composites: RefCell<FastHashMap<CompositesKey, Rc<RefCell<Composites>>>>,
    /// The placeholders that the candidates for each placeholder are in terms of.
    required: RefCell<FastHashMap<Generator, Rc<[Generator]>>>,
    /// The candidates found for a placeholder, given the replacements for those it is in terms
    /// of (and not the others, which the candidates do not depend on).
    candidates: RefCell<FastHashMap<CandidatesKey, Vec<Diagram>>>,
}

/// The diagram that composites start at, and how invertible their cells are.
type CompositesKey = (Diagram, Invertibility);

/// A placeholder with the replacements of the placeholders its candidates are in terms of.
type CandidatesKey = (Generator, Vec<(Generator, Diagram)>);

/// The most lists of candidates kept.
const MAX_CACHED_CANDIDATES: usize = 1 << 16;

/// The most diagrams that composites are kept for, so that the memory held is bounded.
const MAX_CACHED_SOURCES: usize = 4096;

/// The composites of up to [`MAX_CELLS`] cells that start at a diagram, a level (number of cells)
/// at a time as they are wanted, each once and in the order of a breadth first search.
struct Composites {
    /// Those composites with this many cells that were not found with fewer.
    levels: Vec<Rc<[DiagramN]>>,
    seen: FastHashSet<DiagramN>,
    /// What the last level can be extended to, with repeats, for the next level.
    frontier: Vec<DiagramN>,
}

impl Composites {
    fn new(source: &Diagram) -> Self {
        Self {
            levels: vec![],
            seen: FastHashSet::default(),
            frontier: vec![source.clone().identity()],
        }
    }

    /// The composites with `depth` cells that are new, making them (and those with fewer) if need
    /// be. `None` if the time limit of the thread (see [`deadline`]) is reached before they are
    /// all made, in which case nothing is kept of the level that was being made.
    fn level(&mut self, depth: usize, cells: &[&DiagramN]) -> Option<Rc<[DiagramN]>> {
        while self.levels.len() <= depth {
            let depth = self.levels.len();
            let mut level = vec![];
            let mut next = vec![];
            // A cell is only attached where the cells of its source are.
            let sources: Vec<(Diagram, Vec<Generator>)> = if depth == MAX_CELLS {
                vec![]
            } else {
                cells
                    .iter()
                    .map(|cell| {
                        let source = cell.source();
                        let generators = source.generators().into_keys().collect();
                        (source, generators)
                    })
                    .collect()
            };
            let frontier = std::mem::take(&mut self.frontier);
            let mut added = vec![];
            let mut cut = false;
            'composites: for composite in &frontier {
                if deadline::expired() {
                    cut = true;
                    break;
                }
                if !self.seen.insert(composite.clone()) {
                    continue;
                }
                added.push(composite.clone());
                level.push(composite.clone());
                if depth == MAX_CELLS {
                    continue;
                }
                let end = composite.target();
                let present = end.generators();
                for (cell, (source, generators)) in cells.iter().zip(&sources) {
                    if !generators
                        .iter()
                        .all(|generator| present.contains_key(generator))
                    {
                        continue;
                    }
                    for embedding in end.embeddings(source) {
                        if let Ok(extended) = composite.attach(cell, Boundary::Target, &embedding) {
                            next.push(extended);
                        }
                        if deadline::expired() {
                            cut = true;
                            break 'composites;
                        }
                    }
                    if deadline::expired() {
                        cut = true;
                        break 'composites;
                    }
                }
            }
            if cut {
                // What was found of the level is not all of it: as if it had not been started.
                for composite in &added {
                    self.seen.remove(composite);
                }
                self.frontier = frontier;
                return None;
            }
            self.frontier = next;
            self.levels.push(level.into());
        }
        Some(Rc::clone(&self.levels[depth]))
    }
}

impl std::fmt::Debug for Cells {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cells").finish_non_exhaustive()
    }
}

enum From {
    Generators(Vec<Generator>),
    /// The generators that the diagrams are made of.
    Diagrams(Vec<Diagram>),
}

#[derive(Default)]
struct Made {
    /// The diagrams of the cells (and their inverses where invertible).
    cells: Vec<DiagramN>,
    zero_cells: Vec<Diagram>,
}

impl Cells {
    /// The composites that start at a diagram (kept, for a bounded number of diagrams).
    fn composites_from(
        &self,
        source: &Diagram,
        required: Invertibility,
    ) -> Rc<RefCell<Composites>> {
        let mut cache = self.composites.borrow_mut();
        let key = (source.clone(), required);
        if let Some(found) = cache.get(&key) {
            return Rc::clone(found);
        }
        let fresh = Rc::new(RefCell::new(Composites::new(source)));
        if cache.len() < MAX_CACHED_SOURCES {
            cache.insert(key, Rc::clone(&fresh));
        }
        fresh
    }
}

impl<'a> Pool<'a> {
    /// The pool of the cells of the given generators.
    pub(super) fn new(
        signature: &'a Signature,
        generators: impl IntoIterator<Item = Generator>,
    ) -> Self {
        Self::reaching(signature, generators, usize::MAX)
    }

    /// The pool of the cells of the given generators, for a diagram whose cells have dimension up
    /// to `reach`. A placeholder of a higher dimension cannot be replaced with a cell of the
    /// diagram, as there are none: it is a placeholder that the schema depends on without
    /// the cells that are attached mentioning it (`σ[η̇]` is about a functor `Ḟ`, and depends on
    /// the transformation `η̇` of it). Its replacements are made of the cells that the instances
    /// in the diagram were made with (`p` in `η̇₍₂₎[T/Ḟ, 1/Ġ, p/η̇]`), which are the cells of that
    /// dimension that the diagram is about, of the type it needs.
    pub(super) fn reaching(
        signature: &'a Signature,
        generators: impl IntoIterator<Item = Generator>,
        reach: usize,
    ) -> Self {
        Self {
            signature,
            cells: Rc::new(Cells {
                from: From::Generators(generators.into_iter().collect()),
                made: OnceCell::new(),
                beyond: OnceCell::new(),
                reach,
                composites: RefCell::default(),
                required: RefCell::default(),
                candidates: RefCell::default(),
            }),
        }
    }

    /// The pool of the cells that the diagrams are made of.
    pub(super) fn of_diagrams(signature: &'a Signature, diagrams: Vec<Diagram>) -> Self {
        Self {
            signature,
            cells: Rc::new(Cells {
                from: From::Diagrams(diagrams),
                made: OnceCell::new(),
                beyond: OnceCell::new(),
                reach: usize::MAX,
                composites: RefCell::default(),
                required: RefCell::default(),
                candidates: RefCell::default(),
            }),
        }
    }

    /// The pool made of these cells, for a signature that has them.
    pub(super) const fn from_cells(signature: &'a Signature, cells: Rc<Cells>) -> Self {
        Self { signature, cells }
    }

    /// What the pool is made of.
    pub(super) fn cells(&self) -> Rc<Cells> {
        Rc::clone(&self.cells)
    }

    fn made(&self) -> &Made {
        self.cells.made.get_or_init(|| {
            let generators: Vec<Generator> = match &self.cells.from {
                From::Generators(generators) => generators.clone(),
                From::Diagrams(diagrams) => diagrams
                    .iter()
                    .flat_map(|d| d.generators().into_keys())
                    .collect(),
            };
            self.make(generators)
        })
    }

    /// The cells (other than placeholders) that the replacements of the instances in the diagram
    /// are made of, and those that the instances among them were made with in turn.
    fn beyond(&self) -> &Made {
        self.cells.beyond.get_or_init(|| {
            let mut todo: Vec<Generator> = match &self.cells.from {
                From::Generators(generators) => generators.clone(),
                From::Diagrams(diagrams) => diagrams
                    .iter()
                    .flat_map(|d| d.generators().into_keys())
                    .collect(),
            };
            let mut seen = FastHashSet::default();
            let mut mentioned = vec![];
            while let Some(g) = todo.pop() {
                if !seen.insert(g) {
                    continue;
                }
                let Some(info) = self.signature.generator_info(g) else {
                    continue;
                };
                if let Some(made) = &info.instance {
                    for (_, replacement) in &made.substitution {
                        for h in replacement.generators().into_keys() {
                            if self
                                .signature
                                .generator_info(h)
                                .is_some_and(|i| !i.placeholder)
                            {
                                mentioned.push(h);
                            }
                            todo.push(h);
                        }
                    }
                }
            }
            mentioned.sort_by_key(|g| (g.dimension, g.id));
            mentioned.dedup();
            self.make(mentioned)
        })
    }

    /// The cells of some generators, and their inverses where they are invertible.
    fn make(&self, generators: Vec<Generator>) -> Made {
        let mut made = Made::default();
        for g in generators {
            match self
                .signature
                .generator_info(g)
                .map(|info| (&info.diagram, info.invertibility))
            {
                Some((Diagram::Diagram0(d), _)) => made.zero_cells.push((*d).into()),
                Some((Diagram::DiagramN(d), invertibility)) => {
                    made.cells.push(d.clone());
                    if invertibility != Invertibility::Directed {
                        made.cells.push(d.inverse());
                    }
                }
                None => {}
            }
        }
        made
    }

    /// The diagram of the instance of a generator that mentions placeholders for a (normalised
    /// and not empty) substitution of them, using the existing instance. Unless `strict`, a
    /// missing instance is stood in for by the generator with a substituted diagram.
    ///
    /// An instance made with replacements that mention placeholders is instantiated by
    /// instantiating its schema with them substituted.
    pub(super) fn instance_of(
        &self,
        b: Generator,
        restricted: &[(Generator, Diagram)],
        strict: bool,
    ) -> Option<Diagram> {
        let (schema, restricted) = match &self.signature.generator_info(b)?.instance {
            Some(made) if self.signature.touches(made, restricted) => {
                let mut composed = vec![];
                for (placeholder, replacement) in &made.substitution {
                    composed.push((
                        *placeholder,
                        self.substitute(b, restricted, replacement, strict)?,
                    ));
                }
                composed.extend(self.signature.free_replacements(made, restricted));
                (made.schema, self.signature.normalise(&composed))
            }
            _ => (b, restricted.to_vec()),
        };
        if restricted.is_empty() {
            // Nothing is replaced: the schema itself.
            return Some(self.signature.generator_info(schema)?.diagram.clone());
        }
        match self.signature.find_instance(schema, &restricted) {
            Some(instance) => Some(self.signature.generator_info(instance)?.diagram.clone()),
            None if strict => None,
            None => {
                let Diagram::DiagramN(own) = &self.signature.generator_info(schema)?.diagram else {
                    return None;
                };
                Some(
                    DiagramN::from_generator(
                        schema,
                        self.substitute(schema, &restricted, &own.source(), false)?,
                        self.substitute(schema, &restricted, &own.target(), false)?,
                    )
                    .ok()?
                    .into(),
                )
            }
        }
    }

    /// Applies a partial substitution to a diagram mentioned by `generator`, using the existing
    /// instances of the generators it depends on. Unless `strict`, a missing instance is stood in
    /// for by the dependent generator with a substituted diagram, which is enough for a preview:
    /// the actual instance is only created when attaching.
    pub(super) fn substitute(
        &self,
        generator: Generator,
        substitution: &[(Generator, Diagram)],
        diagram: &Diagram,
        strict: bool,
    ) -> Option<Diagram> {
        let substitution = self.signature.normalise(substitution);
        let mut replacements = substitution.clone();
        for (b, restricted) in self
            .signature
            .dependents_in(diagram, Some(generator), &substitution)
        {
            let restricted = self.signature.normalise(&restricted);
            if restricted.is_empty() {
                continue;
            }
            replacements.push((b, self.instance_of(b, &restricted, strict)?));
        }
        substitute(diagram, self.signature, &|g| {
            replacements
                .iter()
                .find(|(h, _)| *h == g)
                .map(|(_, d)| d.clone())
        })
        .ok()
    }

    /// Whether a diagram has the type that a replacement of the placeholder needs, given the
    /// replacements of the placeholders its type is in terms of: its dimension, and its source and
    /// target being those of the placeholder with the substitution applied. (The candidates for a
    /// placeholder are made to fit; this is for a replacement that comes from elsewhere.)
    pub(super) fn fits(
        &self,
        placeholder: Generator,
        replacement: &Diagram,
        substitution: &[(Generator, Diagram)],
    ) -> bool {
        let Some(own) = self
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
                    && self.substitute(placeholder, substitution, &own.source(), true)
                        == Some(replacement.source())
                    && self.substitute(placeholder, substitution, &own.target(), true)
                        == Some(replacement.target())
            }
            _ => false,
        }
    }

    /// The candidate replacements for a placeholder, given replacements for the placeholders
    /// its type is in terms of (and others, which make no difference). The same candidates for
    /// the same replacements of those are found once.
    // `Diagram` is hash-consed; its `OnceCell` caches do not affect `Hash`/`Eq`.
    #[allow(clippy::mutable_key_type)]
    pub(super) fn candidates(
        &self,
        placeholder: Generator,
        substitution: &[(Generator, Diagram)],
    ) -> Vec<Diagram> {
        let required = {
            let mut required = self.cells.required.borrow_mut();
            Rc::clone(required.entry(placeholder).or_insert_with(|| {
                self.signature
                    .parameters(placeholder)
                    .into_iter()
                    .filter(|&q| q != placeholder)
                    .collect()
            }))
        };
        let mut key: Vec<(Generator, Diagram)> = substitution
            .iter()
            .filter(|(g, _)| required.contains(g))
            .cloned()
            .collect();
        key.sort_by_key(|(g, _)| (g.dimension, g.id));
        let key = (placeholder, key);
        if let Some(found) = self.cells.candidates.borrow().get(&key) {
            return found.clone();
        }
        let found = self.find_candidates(placeholder, substitution);
        // A list that was cut short by the time limit is not the list: it is not remembered.
        if deadline::expired() {
            return found;
        }
        let mut cache = self.cells.candidates.borrow_mut();
        if cache.len() < MAX_CACHED_CANDIDATES {
            cache.insert(key, found.clone());
        }
        found
    }

    // `DiagramN` is hash-consed; its `OnceCell` caches do not affect `Hash`/`Eq`.
    #[allow(clippy::mutable_key_type)]
    fn find_candidates(
        &self,
        placeholder: Generator,
        substitution: &[(Generator, Diagram)],
    ) -> Vec<Diagram> {
        let Some(own) = self
            .signature
            .generator_info(placeholder)
            .map(|info| info.diagram.clone())
        else {
            return vec![];
        };
        let Diagram::DiagramN(own_n) = &own else {
            return self
                .made()
                .zero_cells
                .iter()
                .filter(|&d| *d != own)
                .take(MAX_CANDIDATES)
                .cloned()
                .collect();
        };
        let (Some(source), Some(target)) = (
            self.substitute(placeholder, substitution, &own_n.source(), true),
            self.substitute(placeholder, substitution, &own_n.target(), true),
        ) else {
            return vec![];
        };

        let k = own_n.dimension();
        // What replaces an invertible placeholder is invertible (see
        // `Signature::check_invertibility`): a composite of cells is as invertible as the weakest
        // of them, so cells that are not are left out, which is also less to search.
        let required = self
            .signature
            .generator_info(placeholder)
            .map_or(Invertibility::Directed, |info| info.invertibility);
        let made = if k > self.cells.reach {
            self.beyond()
        } else {
            self.made()
        };
        let cells: Vec<&DiagramN> = made
            .cells
            .iter()
            .filter(|c| c.dimension() == k)
            .filter(|c| {
                self.signature
                    .check_invertibility(placeholder, &Diagram::from((*c).clone()))
                    .is_ok()
            })
            .collect();
        let composites = self.cells.composites_from(&source, required);
        let mut results = vec![];
        for depth in 0..=MAX_CELLS {
            let Some(level) = composites.borrow_mut().level(depth, &cells) else {
                // Out of time: what there is so far.
                return results;
            };
            for composite in level.iter() {
                if composite.target() == target && Diagram::from(composite.clone()) != own {
                    results.push(composite.clone().into());
                    if results.len() >= MAX_CANDIDATES {
                        return results;
                    }
                }
            }
        }
        results
    }
}

impl Signature {
    /// The ways to instantiate a schema so that it can be attached to `haystack` at `point`,
    /// attaching to its `boundary` (as in the attach menu). Instances of generators that the
    /// schema depends on must already exist for the instance to match.
    ///
    /// This is [`InferSearch`] run to the end, which the attach options do a step at a time.
    #[must_use]
    pub fn infer_instances(
        &self,
        schema: Generator,
        haystack: &Diagram,
        boundary: Boundary,
        point: &[Height],
    ) -> Vec<InferredInstance> {
        let cells = Pool::reaching(
            self,
            haystack.generators().into_keys(),
            haystack.dimension(),
        )
        .cells();
        let Some(mut search) =
            InferSearch::new(self, cells, schema, haystack, boundary, point, true)
        else {
            return vec![];
        };
        while !search.step() {}
        search.finish()
    }
}

impl Signature {
    /// The replacements of the placeholders of a schema that are determined by the replacements
    /// `chosen` for some of them. Each replacement chosen has a type in terms of other
    /// placeholders, e.g. a cone `α₀ : X₀ ∘ ! ⇒ D₀` for `α : X ∘ ! ⇒ D`: so `X` and `D` must be
    /// replaced with `X₀` and `D₀`.
    ///
    /// A placeholder is determined if every way of replacing the rest of the placeholders so that
    /// the types of the replacements chosen are right agrees on it. The types are matched
    /// structurally, which takes no search for the placeholders they mention: see the
    /// [module](super::completion) on how, and on what is left to a bounded search, like
    /// [`infer_instances`](Self::infer_instances). If that has to stop early, only what matching
    /// forces is returned. Placeholders that the types do not mention are left alone, and
    /// placeholders that were replaced already are never returned.
    #[must_use]
    pub fn infer_replacements(
        &self,
        schema: Generator,
        chosen: &[(Generator, Diagram)],
    ) -> Vec<(Generator, Diagram)> {
        self.infer(schema, chosen).determined
    }
}

/// The search for the ways to instantiate a schema so that it attaches at a point (see
/// [`Signature::infer_instances`]), as a machine that does a little at a time: one list of
/// candidates for a placeholder, or one complete substitution checked, in each
/// [`step`](Self::step).
///
/// The placeholders are solved one after another, each from the candidates for it given the
/// replacements chosen for those its type mentions (see [`Pool::candidates`]): not strictly from
/// the lowest dimension up, but a placeholder as soon as the placeholders its type is in terms
/// of are done, so that a replacement that cannot be completed is found out before the
/// replacements for unrelated placeholders are chosen (two functors with a transformation each
/// take the product of what each pair has in the one order, and the sum in the other).
/// What is found is the same, and in the same order: the instances are sorted by the numbers of
/// the candidates chosen, placeholder by placeholder from the lowest dimension. Only when the
/// search stops early, at [`MAX_SUBSTITUTIONS`], does it find different instances.
pub(super) struct InferSearch {
    signature: Signature,
    cells: Rc<Cells>,
    schema: Generator,
    schema_diagram: DiagramN,
    parameters: Vec<Generator>,
    /// The order the placeholders are solved in, as indices into `parameters`.
    order: Vec<usize>,
    haystack: Diagram,
    /// The generators of the haystack, for ruling out needles that have others.
    present: FastHashSet<Generator>,
    boundary: Boundary,
    point: Vec<Height>,
    started: bool,
    done: bool,
    /// The searches to make, one after the other (see [`Plan`]), and the one that is going.
    plans: Vec<Plan>,
    plan: usize,
    /// What has been found, so that a way of reading replacements off the diagram that overlaps
    /// another does not find it twice.
    #[allow(clippy::type_complexity)]
    seen: FastHashSet<(Vec<(Generator, Diagram)>, bool, Vec<usize>)>,
    /// The candidates for the placeholder solved at each depth, and how many have been taken.
    stack: Vec<Candidates>,
    /// What was chosen at each depth up to the last frame.
    chosen: Vec<Chosen>,
    tried: usize,
    /// Each instance with what it was found for, to sort by.
    results: Vec<(Vec<usize>, InferredInstance)>,
}

/// A search for the replacements of the placeholders, in one or both directions.
struct Plan {
    /// What reading the diagram's instances tells of the replacements (see [`seeds`]), with the
    /// number of the way: the placeholders that have a replacement here are replaced with just
    /// that, and the rest are searched for as usual. `None` to search for all of them.
    seed: Option<(usize, Vec<Option<Diagram>>)>,
    /// Whether the schema is to be attached, and whether its inverse is.
    forward: bool,
    inverse: bool,
}

struct Candidates {
    list: Vec<Diagram>,
    next: usize,
}

struct Chosen {
    parameter: usize,
    candidate: usize,
    replacement: Diagram,
}

impl std::fmt::Debug for InferSearch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InferSearch")
            .field("schema", &self.schema)
            .field("depth", &self.stack.len())
            .field("tried", &self.tried)
            .finish_non_exhaustive()
    }
}

impl InferSearch {
    /// The search to instantiate a schema, with the cells of the pool shared with other searches
    /// in the same diagram (which share the work of making the composites in them). `None` if
    /// there is nothing to search: the schema has no placeholders, or is not a cell of the
    /// dimension to attach.
    pub(super) fn new(
        signature: &Signature,
        cells: Rc<Cells>,
        schema: Generator,
        haystack: &Diagram,
        boundary: Boundary,
        point: &[Height],
        seeding: bool,
    ) -> Option<Self> {
        let parameters = signature.parameters(schema);
        let Some(Diagram::DiagramN(schema_diagram)) = signature
            .generator_info(schema)
            .map(|info| info.diagram.clone())
        else {
            return None;
        };
        if parameters.is_empty() || schema_diagram.dimension() != haystack.dimension() + 1 {
            return None;
        }

        // The placeholders that the type of each is in terms of.
        let required: Vec<Vec<usize>> = parameters
            .iter()
            .map(|&p| {
                signature
                    .parameters(p)
                    .into_iter()
                    .filter(|&q| q != p)
                    .filter_map(|q| parameters.iter().position(|&r| r == q))
                    .collect()
            })
            .collect();
        // A placeholder whose type is in terms of others is solved as soon as they are, so that a
        // choice that leaves it without replacements is found out before the replacements for
        // unrelated placeholders are chosen; otherwise the lowest dimension first.
        let mut order: Vec<usize> = vec![];
        while order.len() < parameters.len() {
            let ready = |i: &usize| required[*i].iter().all(|j| order.contains(j));
            let unsolved = || (0..parameters.len()).filter(|i| !order.contains(i));
            let next = unsolved()
                .filter(ready)
                .find(|&i| !required[i].is_empty())
                .or_else(|| unsolved().find(ready))
                // Placeholders whose types are in terms of each other.
                .or_else(|| unsolved().next())?;
            order.push(next);
        }

        let present: FastHashSet<Generator> = haystack.generators().into_keys().collect();
        // The cell is attached along its source on a target boundary and along its target on a
        // source one, and an invertible schema also as its inverse, which is the other way round.
        let invertible = signature
            .generator_info(schema)
            .is_some_and(|info| info.invertibility.is_invertible());
        let mut plans = vec![];
        let mut unread = Plan {
            seed: None,
            forward: false,
            inverse: false,
        };
        for inverse in [false, true] {
            if inverse && !invertible {
                break;
            }
            let along_source = (boundary == Boundary::Target) != inverse;
            let side: Diagram = if along_source {
                schema_diagram.source()
            } else {
                schema_diagram.target()
            };
            let read = if seeding {
                seeds(signature, &parameters, schema, &side, &present)
            } else {
                Seeds::Free
            };
            match read {
                Seeds::Free => {
                    unread.forward |= !inverse;
                    unread.inverse |= inverse;
                }
                Seeds::Impossible => {}
                Seeds::Ways(ways) => {
                    plans.extend(ways.into_iter().enumerate().map(|(number, way)| Plan {
                        seed: Some((number, way)),
                        forward: !inverse,
                        inverse,
                    }));
                }
            }
        }
        if unread.forward || unread.inverse {
            plans.push(unread);
        }

        Some(Self {
            signature: signature.clone(),
            cells,
            schema,
            schema_diagram,
            parameters,
            order,
            haystack: haystack.clone(),
            present,
            boundary,
            point: point.to_vec(),
            started: false,
            done: false,
            plans,
            plan: 0,
            seen: FastHashSet::default(),
            stack: vec![],
            chosen: vec![],
            tried: 0,
            results: vec![],
        })
    }

    /// Does a little of the search: whether it is over.
    pub(super) fn step(&mut self) -> bool {
        if self.done {
            return true;
        }
        if !self.started {
            self.started = true;
            return self.begin_plan();
        }
        let depth = self.stack.len();
        let Some(frame) = self.stack.last_mut() else {
            self.plan += 1;
            return self.begin_plan();
        };
        if frame.next >= frame.list.len() {
            self.stack.pop();
            return false;
        }
        if self.tried >= MAX_SUBSTITUTIONS {
            self.stack.clear();
            return false;
        }
        let candidate = frame.next;
        let replacement = frame.list[candidate].clone();
        frame.next += 1;
        self.chosen.truncate(depth - 1);
        self.chosen.push(Chosen {
            parameter: self.order[depth - 1],
            candidate,
            replacement,
        });
        if depth == self.order.len() {
            self.tried += 1;
            self.check();
        } else {
            self.descend();
        }
        false
    }

    /// Starts the search of the plan that is next, if there is one: whether the whole search is
    /// over.
    fn begin_plan(&mut self) -> bool {
        self.stack.clear();
        self.chosen.clear();
        self.tried = 0;
        if self.plan >= self.plans.len() {
            self.done = true;
            return true;
        }
        self.descend();
        false
    }

    /// Finds the candidates for the next placeholder, given those chosen.
    fn descend(&mut self) {
        let parameter = self.order[self.chosen.len()];
        let placeholder = self.parameters[parameter];
        let substitution: Vec<_> = self
            .chosen
            .iter()
            .map(|chosen| {
                (
                    self.parameters[chosen.parameter],
                    chosen.replacement.clone(),
                )
            })
            .collect();
        let pool = Pool::from_cells(&self.signature, Rc::clone(&self.cells));
        let read = self.plans[self.plan]
            .seed
            .as_ref()
            .and_then(|(_, way)| way[parameter].as_ref());
        let list = match read {
            // Read off the diagram: what it is is known, and whether it fits what the others
            // chosen make of the placeholder's type is not.
            Some(replacement) if pool.fits(placeholder, replacement, &substitution) => {
                vec![replacement.clone()]
            }
            Some(_) => vec![],
            None => pool.candidates(placeholder, &substitution),
        };
        self.stack.push(Candidates { list, next: 0 });
    }

    /// The instances found since the last time this was asked, each with the numbers of the
    /// candidates chosen for the placeholders (from the lowest dimension), which is what they are
    /// sorted by in the end: in the order they were found. Taking them as the search goes on
    /// does not change what [`finish`](Self::finish) gives for the rest.
    pub(super) fn take_results(&mut self) -> Vec<(Vec<usize>, InferredInstance)> {
        std::mem::take(&mut self.results)
    }

    /// The instances found and not taken, in the order that choosing the candidates for the
    /// placeholders from the lowest dimension would find them.
    pub(super) fn finish(mut self) -> Vec<InferredInstance> {
        self.results.sort_by(|(a, _), (b, _)| a.cmp(b));
        self.results.into_iter().map(|(_, found)| found).collect()
    }

    /// Checks whether a complete substitution lets the schema attach at the point.
    fn check(&mut self) {
        let mut chosen: Vec<&Chosen> = self.chosen.iter().collect();
        chosen.sort_by_key(|chosen| chosen.parameter);
        let substitution: Vec<(Generator, Diagram)> = chosen
            .iter()
            .map(|chosen| {
                (
                    self.parameters[chosen.parameter],
                    chosen.replacement.clone(),
                )
            })
            .collect();
        let mut key = vec![0; self.parameters.len()];
        for chosen in &chosen {
            key[chosen.parameter] = chosen.candidate;
        }

        let pool = Pool::from_cells(&self.signature, Rc::clone(&self.cells));
        let source = self.schema_diagram.source();
        let target = self.schema_diagram.target();
        // The cell is attached along its source on a target boundary and along its target on a
        // source one, and an invertible schema also as its inverse, which is the other way round.
        // (The inverse of an instance is an instance of the inverse: they are the same cell
        // read backwards.)
        let (forward, backward, way) = {
            let plan = &self.plans[self.plan];
            (
                plan.forward,
                plan.inverse,
                plan.seed.as_ref().map(|(number, _)| *number),
            )
        };
        let normalised = self.signature.normalise(&substitution);
        for (inverse, wanted) in [(false, forward), (true, backward)] {
            if !wanted {
                continue;
            }
            // The side attached along must match the diagram, so it can only use existing
            // instances; instances on the other side may be created when attaching.
            let along_source = (self.boundary == Boundary::Target) != inverse;
            let (Some(source), Some(target)) = (
                pool.substitute(self.schema, &substitution, &source, along_source),
                pool.substitute(self.schema, &substitution, &target, !along_source),
            ) else {
                continue;
            };
            // The side attached along is what is looked for in the haystack, and cells that it
            // does not have are not in it.
            let needle_side = if along_source { &source } else { &target };
            if !needle_side
                .generators()
                .keys()
                .all(|generator| self.present.contains(generator))
            {
                continue;
            }
            let Ok(diagram) = DiagramN::from_generator(self.schema, source, target) else {
                continue;
            };
            let diagram = if inverse { diagram.inverse() } else { diagram };
            let Some(needle) = diagram.slice(self.boundary.flip()) else {
                continue;
            };
            let mut key = key.clone();
            if let Some(number) = way {
                key.splice(0..0, [SEEDED_KEY, number]);
            }
            if inverse {
                key.insert(0, INVERSE_KEY);
            }
            for embedding in self.haystack.embeddings_through(&needle, &self.point) {
                if way.is_some()
                    && !self
                        .seen
                        .insert((normalised.clone(), inverse, embedding.clone()))
                {
                    continue;
                }
                self.results.push((
                    key.clone(),
                    InferredInstance {
                        substitution: normalised.clone(),
                        diagram: diagram.clone(),
                        embedding,
                        inverse,
                    },
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use homotopy_common::{deadline, hash::FastHashSet};
    use homotopy_core::{
        signature::{Invertibility, Signature as _},
        Boundary, Diagram, DiagramN, Generator, Height,
    };

    use super::{Cells, InferSearch, Pool, MAX_CANDIDATES, MAX_CELLS, MAX_SUBSTITUTIONS};
    use crate::{
        job::{run_to_end, Job},
        proof::{
            AttachOptionsJob, AttachOutcome, InferenceLimits, ProofState, Signature, SignatureEdit,
            SignatureItemEdit, Workspace,
        },
    };

    /// The candidates for a placeholder as they were found before the composites were kept: a
    /// search from the source of the placeholder of its own, which stops at enough candidates.
    fn reference_candidates(
        pool: &Pool<'_>,
        placeholder: Generator,
        substitution: &[(Generator, Diagram)],
    ) -> Vec<Diagram> {
        let own = pool
            .signature
            .generator_info(placeholder)
            .unwrap()
            .diagram
            .clone();
        let Diagram::DiagramN(own_n) = &own else {
            return pool.candidates(placeholder, substitution);
        };
        let (Some(source), Some(target)) = (
            pool.substitute(placeholder, substitution, &own_n.source(), true),
            pool.substitute(placeholder, substitution, &own_n.target(), true),
        ) else {
            return vec![];
        };
        let cells: Vec<DiagramN> = pool
            .made()
            .cells
            .iter()
            .filter(|cell| cell.dimension() == own_n.dimension())
            .cloned()
            .collect();
        let cells: Vec<&DiagramN> = cells.iter().collect();
        let mut seen = FastHashSet::default();
        let mut results = vec![];
        let mut frontier = vec![source.identity()];
        for depth in 0..=MAX_CELLS {
            let mut next = vec![];
            for composite in frontier {
                if !seen.insert(composite.clone()) {
                    continue;
                }
                if composite.target() == target && Diagram::from(composite.clone()) != own {
                    results.push(composite.clone().into());
                    if results.len() >= MAX_CANDIDATES {
                        return results;
                    }
                }
                if depth == MAX_CELLS {
                    continue;
                }
                for cell in &cells {
                    for embedding in composite.target().embeddings(&cell.source()) {
                        if let Ok(extended) = composite.attach(cell, Boundary::Target, &embedding) {
                            next.push(extended);
                        }
                    }
                }
            }
            frontier = next;
        }
        results
    }

    /// The search as it was before it was made a step at a time, the placeholders from the
    /// lowest dimension up, and every embedding looked at.
    fn reference_infer(
        sig: &Signature,
        schema: Generator,
        haystack: &Diagram,
        boundary: Boundary,
        point: &[Height],
    ) -> Vec<(Vec<(Generator, Diagram)>, Vec<usize>)> {
        struct Ref<'a> {
            pool: Pool<'a>,
            schema: Generator,
            schema_diagram: DiagramN,
            parameters: Vec<Generator>,
            haystack: &'a Diagram,
            boundary: Boundary,
            point: &'a [Height],
            tried: usize,
            results: Vec<(Vec<(Generator, Diagram)>, Vec<usize>)>,
        }
        impl Ref<'_> {
            fn solve(&mut self, substitution: &mut Vec<(Generator, Diagram)>) {
                if self.tried >= MAX_SUBSTITUTIONS {
                    return;
                }
                let Some(&placeholder) = self.parameters.get(substitution.len()) else {
                    self.tried += 1;
                    self.check(substitution);
                    return;
                };
                for candidate in reference_candidates(&self.pool, placeholder, substitution) {
                    substitution.push((placeholder, candidate));
                    self.solve(substitution);
                    substitution.pop();
                }
            }

            fn check(&mut self, substitution: &[(Generator, Diagram)]) {
                let strict_source = self.boundary == Boundary::Target;
                let (source, target) = (self.schema_diagram.source(), self.schema_diagram.target());
                let (Some(source), Some(target)) = (
                    self.pool
                        .substitute(self.schema, substitution, &source, strict_source),
                    self.pool
                        .substitute(self.schema, substitution, &target, !strict_source),
                ) else {
                    return;
                };
                let Ok(diagram) = DiagramN::from_generator(self.schema, source, target) else {
                    return;
                };
                let Some(needle) = diagram.slice(self.boundary.flip()) else {
                    return;
                };
                for embedding in self.haystack.embeddings(&needle) {
                    if needle.contains_point(self.point, &embedding) {
                        self.results
                            .push((self.pool.signature.normalise(substitution), embedding));
                    }
                }
            }
        }

        let parameters = sig.parameters(schema);
        let Some(Diagram::DiagramN(schema_diagram)) =
            sig.generator_info(schema).map(|info| info.diagram.clone())
        else {
            return vec![];
        };
        if parameters.is_empty() || schema_diagram.dimension() != haystack.dimension() + 1 {
            return vec![];
        }
        let mut search = Ref {
            pool: Pool::new(sig, haystack.generators().into_keys()),
            schema,
            schema_diagram,
            parameters,
            haystack,
            boundary,
            point,
            tried: 0,
            results: vec![],
        };
        search.solve(&mut vec![]);
        search.results
    }

    fn found(
        sig: &Signature,
        schema: Generator,
        haystack: &Diagram,
        boundary: Boundary,
        point: &[Height],
    ) -> Vec<(Vec<(Generator, Diagram)>, Vec<usize>)> {
        sig.infer_instances(schema, haystack, boundary, point)
            .into_iter()
            .map(|inferred| (inferred.substitution, inferred.embedding))
            .collect()
    }

    /// Two independent placeholders `P, P' : x → x` with a 2-cell each, `e : P ⇒ P` and
    /// `e' : P' ⇒ P'`; the schema `S : e ⇛ e` which depends on `e'` and so on `P'`; and a diagram
    /// with cells to replace them with.
    fn independent() -> (Signature, Generator, DiagramN, DiagramN) {
        let mut sig = Signature::default();
        let x: Diagram = sig.create_generator_zero("x").into();
        let cell = |sig: &mut Signature, s: Diagram, t: Diagram| {
            sig.create_generator(s, t, "c", Invertibility::Directed)
                .unwrap()
        };
        let p = cell(&mut sig, x.clone(), x.clone());
        let p2 = cell(&mut sig, x.clone(), x.clone());
        let q = cell(&mut sig, x.clone(), x.clone());
        let e = cell(&mut sig, p.clone().into(), p.clone().into());
        let e2 = cell(&mut sig, p2.clone().into(), p2.clone().into());
        let u = cell(&mut sig, q.clone().into(), q.clone().into());
        let s = cell(&mut sig, e.clone().into(), e.clone().into());
        let g = |d: &DiagramN| d.max_generator().generator;
        for d in [&p, &p2, &e, &e2] {
            make_placeholder(&mut sig, g(d));
        }
        let node = sig.find_node(g(&s)).unwrap();
        sig.update(&SignatureEdit::Edit(
            node,
            SignatureItemEdit::SetDependsOn(vec![g(&e2)]),
        ))
        .unwrap();
        (sig, g(&s), q, u)
    }

    /// The identity on `q ≫ q ≫ q`.
    fn small_haystack(q: &DiagramN) -> Diagram {
        q.attach(q, Boundary::Target, &[])
            .unwrap()
            .attach(q, Boundary::Target, &[])
            .unwrap()
            .identity()
            .into()
    }

    #[test]
    fn the_search_finds_what_the_search_from_the_lowest_dimension_up_finds() {
        let (sig, s, q, _) = independent();
        let haystack = small_haystack(&q);
        assert_eq!(sig.parameters(s).len(), 4);
        // `e` is solved right after `P` and before `P'`.
        let cells = Pool::new(&sig, haystack.generators().into_keys()).cells();
        let search =
            InferSearch::new(&sig, cells, s, &haystack, Boundary::Target, &[], true).unwrap();
        assert_eq!(search.order, vec![0, 2, 1, 3]);
        let mut total = 0;
        for point in [
            vec![],
            vec![Height::Singular(0)],
            vec![Height::Regular(0)],
            vec![Height::Regular(0), Height::Singular(1)],
            vec![Height::Regular(0), Height::Singular(2)],
        ] {
            let expected = reference_infer(&sig, s, &haystack, Boundary::Target, &point);
            let got = found(&sig, s, &haystack, Boundary::Target, &point);
            assert_eq!(got.len(), expected.len(), "{point:?}");
            assert!(got == expected, "{point:?} (the order is different)");
            total += expected.len();
        }
        assert!(total > 0, "some instances are inferred");
    }

    /// The schema `S` of `independent`, with the identity on `q ≫ q ≫ q` (small, and the search
    /// finds instances), or, with the cell `u : q ⇒ q` as well, a diagram that the search for
    /// the placeholders has a great deal of choice in (and runs into its limit).
    fn attach_job(big: bool, limits: InferenceLimits) -> AttachOptionsJob {
        AttachOptionsJob::new(&schema_proof(big), &[], false, limits, None).unwrap()
    }

    fn schema_proof(big: bool) -> ProofState {
        let (sig, _, q, u) = independent();
        let haystack = if big {
            let mut d = q.identity();
            for _ in 0..3 {
                d = d.attach(&u, Boundary::Target, &[]).unwrap();
            }
            d.into()
        } else {
            small_haystack(&q)
        };
        ProofState {
            signature: sig,
            workspace: Some(Workspace::new(haystack)),
            ..ProofState::default()
        }
    }

    /// The search for the schema alone gives what the whole search gives for it, in pieces.
    #[test]
    fn a_schema_searched_for_alone_is_what_the_whole_search_finds_for_it() {
        for big in [false, true] {
            let proof = schema_proof(big);
            let limits = InferenceLimits::UNLIMITED;
            let whole =
                run_to_end(AttachOptionsJob::new(&proof, &[], false, limits, None).unwrap());
            let mut instances = 0;
            for info in proof.signature.iter() {
                let mut job =
                    AttachOptionsJob::new(&proof, &[], false, limits, Some(info.generator))
                        .unwrap();
                let (mut steps, mut last_done) = (0, 0);
                let only = loop {
                    let progress = job.progress().unwrap();
                    assert!(progress.done <= progress.total && progress.done >= last_done);
                    last_done = progress.done;
                    steps += 1;
                    if let Some(outcome) = job.step() {
                        break outcome;
                    }
                };
                let expected: Vec<_> = whole
                    .options
                    .iter()
                    .filter(|option| option.generator == info.generator)
                    .cloned()
                    .collect();
                assert_eq!(only.options, expected, "{}", info.name);
                let found = only
                    .options
                    .iter()
                    .filter(|option| option.substitution.is_some())
                    .count();
                if found > 0 {
                    // Searching for the schema is more than a step: the pieces and the inference.
                    assert!(steps > 3, "{steps}");
                }
                instances += found;
            }
            assert!(instances > 0, "instances are inferred");
        }
    }

    /// The instances of a schema are options as soon as the search finds them, and not when the
    /// schema is done with (a schema can have no end, and the options found are what there is to
    /// show meanwhile): each in the place that it has in the end, which is where the search that
    /// is not taken from as it goes puts it (the order of [`Signature::infer_instances`]).
    #[test]
    fn instances_are_listed_as_they_are_found_in_the_places_they_will_have() {
        use crate::proof::Streamed;

        let mut several_arrivals = 0;
        for big in [false, true] {
            let proof = schema_proof(big);
            let mut job =
                AttachOptionsJob::new(&proof, &[], false, InferenceLimits::UNLIMITED, None)
                    .unwrap();
            let mut streamed = Streamed::default();
            let mut shown: Vec<crate::proof::AttachOption> = vec![];
            let (mut arrivals, mut before_the_end) = (0, 0);
            let outcome = loop {
                let output = job.step();
                let found = job.take_found();
                if found
                    .iter()
                    .any(|found| found.option.substitution.is_some())
                {
                    arrivals += 1;
                    // (Not in the step that ends the search: it has found them before.)
                    if output.is_none() {
                        assert_eq!(job.label(), "Inferring instances");
                        before_the_end += 1;
                    }
                }
                streamed.extend(found);
                // What is listed is only added to.
                let now = streamed.options();
                let mut rest = now.iter();
                assert!(shown.iter().all(|option| rest.any(|other| other == option)));
                shown = now;
                if let Some(outcome) = output {
                    break outcome;
                }
            };
            assert_eq!(streamed.options(), outcome.options);
            assert!(job.take_found().is_empty());

            // The instances, in the order the whole search of the schema gives them.
            let haystack = proof.workspace.as_ref().unwrap().diagram.clone();
            let schema = proof
                .signature
                .iter()
                .find(|info| {
                    info.generator.dimension == haystack.dimension() + 1
                        && !proof.signature.parameters(info.generator).is_empty()
                })
                .map(|info| info.generator)
                .unwrap();
            let expected: Vec<_> = proof
                .signature
                .infer_instances(schema, &haystack, Boundary::Target, &[])
                .into_iter()
                .map(|inferred| (inferred.substitution, inferred.embedding))
                .collect();
            let listed: Vec<_> = outcome
                .options
                .iter()
                .filter_map(|option| {
                    Some((
                        option.substitution.clone()?,
                        option.embedding.iter().copied().collect::<Vec<_>>(),
                    ))
                })
                .collect();
            assert!(!expected.is_empty(), "instances are found (big: {big})");
            assert!(
                listed == expected,
                "the order is that of the search (big: {big})"
            );
            several_arrivals += usize::from(arrivals > 1 && before_the_end > 0);
        }
        assert!(
            several_arrivals > 0,
            "they arrive before their schema is done"
        );
    }

    /// The heuristic of narrowing the pool of cells for the replacements of placeholders to
    /// those near a sheet: all that it finds attaches, it says that it was in effect, only the
    /// instances are narrowed, and without a sheet it does nothing.
    ///
    /// (What it finds is not always among what the search of the whole pool finds: that search
    /// stops at [`MAX_SUBSTITUTIONS`] tried, and a smaller pool has fewer substitutions to try
    /// before that and can reach ones that the whole pool does not. The guarantee is that it is
    /// valid, and not that it is a subset.)
    #[test]
    fn narrowing_to_a_sheet_only_finds_valid_instances() {
        use homotopy_core::SliceIndex;

        use crate::proof::{Action, Narrowed};

        let (mut sig, _, q, u) = independent();
        let w = sig
            .create_generator(
                q.clone().into(),
                q.clone().into(),
                "w",
                Invertibility::Directed,
            )
            .unwrap();
        let mut haystack = q.clone().identity();
        for cell in [&u, &w] {
            haystack = haystack.attach(cell, Boundary::Target, &[]).unwrap();
        }
        let size = haystack.size();
        let mut proof = ProofState {
            signature: sig,
            workspace: Some(Workspace::new(haystack.into())),
            ..ProofState::default()
        };
        // A view of one dimension: the second coordinate of a point is then a sheet.
        proof.workspace.as_mut().unwrap().view.dimension = 1;

        let search = |point: &[SliceIndex], narrow_to_sheet: bool| {
            let limits = InferenceLimits {
                narrow_to_sheet,
                ..InferenceLimits::UNLIMITED
            };
            run_to_end(AttachOptionsJob::new(&proof, point, false, limits, None).unwrap())
        };
        let others = |outcome: &AttachOutcome| {
            outcome
                .options
                .iter()
                .filter(|option| option.substitution.is_none())
                .cloned()
                .collect::<Vec<_>>()
        };

        let (mut narrower, mut found) = (0, 0);
        for height in 0..=2 * size {
            let click = [SliceIndex::Interior(Height::from(height))];
            let sheet = [click[0], SliceIndex::Interior(Height::Singular(0))];
            let whole = search(&sheet, false);
            let near = search(&sheet, true);
            assert_eq!(whole.inference.narrowed, None);
            let Some(Narrowed { kept, of }) = near.inference.narrowed else {
                panic!("{height}: not narrowed");
            };
            assert!(kept <= of && kept > 0, "{kept} of {of}");
            narrower += usize::from(kept < of);
            for option in near.options.iter().filter(|o| o.substitution.is_some()) {
                assert!(
                    proof
                        .clone()
                        .update(&Action::Attach(option.clone()))
                        .is_ok(),
                    "{height}: does not attach"
                );
                found += 1;
            }
            // Only the instances are narrowed: everything else is as it was.
            assert_eq!(others(&near), others(&whole), "{height}");

            // Without a sheet the setting does nothing.
            let (plain, setting) = (search(&click, false), search(&click, true));
            assert_eq!(setting.inference.narrowed, None);
            assert_eq!(setting.options, plain.options);
        }
        assert!(
            narrower > 0,
            "the pool is smaller where the cells are elsewhere"
        );
        assert!(found > 0, "instances are found");
    }

    /// The limit is for one generator and nothing else: it is the same whether the schema is
    /// searched for alone or with others, and without one there is none.
    #[test]
    fn the_time_limit_is_for_a_generator_and_is_the_same_for_all() {
        let limits = InferenceLimits {
            generator_ms: 4000,
            ..InferenceLimits::UNLIMITED
        };
        assert_eq!(
            limits.generator(),
            Some(std::time::Duration::from_millis(4000))
        );
        assert_eq!(InferenceLimits::UNLIMITED.generator(), None);
        let proof = schema_proof(false);
        for only in [None, Some(proof.signature.iter().last().unwrap().generator)] {
            let job = AttachOptionsJob::new(&proof, &[], false, limits, only).unwrap();
            assert_eq!(job.step_limit(), limits.generator());
            let Some(crate::remote::Work::AttachOptions { limits: asked, .. }) = job.offer() else {
                panic!("the search can be offered");
            };
            assert_eq!(asked.generator_ms, 4000);
        }
    }

    #[test]
    fn inference_can_be_turned_off_and_limited() {
        let instances = |outcome: &AttachOutcome| {
            outcome
                .options
                .iter()
                .filter(|option| option.substitution.is_some())
                .count()
        };
        let all = run_to_end(attach_job(false, InferenceLimits::UNLIMITED));
        assert!(instances(&all) > 0, "instances are inferred");
        assert!(!all.inference.timed_out() && all.inference.too_high.is_none());

        // Off: only the options that are not instances, and the same ones.
        let plain = run_to_end(attach_job(
            false,
            InferenceLimits {
                enabled: false,
                ..InferenceLimits::UNLIMITED
            },
        ));
        assert_eq!(instances(&plain), 0);
        assert!(!plain.inference.timed_out() && plain.inference.too_high.is_none());
        let without_instances: Vec<_> = all
            .options
            .iter()
            .filter(|option| option.substitution.is_none())
            .cloned()
            .collect();
        assert_eq!(plain.options, without_instances);

        // The diagram has more dimensions than the limit lets instances be inferred for.
        let high = run_to_end(attach_job(
            false,
            InferenceLimits {
                max_dimension: 1,
                ..InferenceLimits::UNLIMITED
            },
        ));
        assert_eq!(instances(&high), 0);
        assert_eq!(high.inference.too_high, Some(2));
        assert_eq!(high.options, plain.options);
        // At the limit it is done.
        let at_limit = run_to_end(attach_job(
            false,
            InferenceLimits {
                max_dimension: 2,
                ..InferenceLimits::UNLIMITED
            },
        ));
        assert_eq!(at_limit.options, all.options);
        assert_eq!(at_limit.inference.too_high, None);
    }

    #[test]
    fn inference_stops_when_its_time_is_used_up() {
        let all = run_to_end(attach_job(true, InferenceLimits::UNLIMITED));
        assert!(!all.inference.timed_out());
        assert!(all
            .options
            .iter()
            .any(|option| option.substitution.is_some()));

        // A millisecond is used up by the search for the schema.
        let limited = run_to_end(attach_job(
            true,
            InferenceLimits {
                generator_ms: 1,
                ..InferenceLimits::UNLIMITED
            },
        ));
        assert!(limited.inference.timed_out(), "{:?}", limited.inference);
        assert_eq!(limited.inference.budget_ms, 1);
        // Every schema is gone through (given up on or not), as the time is each one's own: none
        // is left for lack of time.
        assert!(limited.inference.not_tried.is_empty());
        assert!(!limited.inference.targeted);
        // What it found is among what there is, and the options that are not instances are all
        // there.
        assert!(limited.options.len() < all.options.len());
        assert!(limited
            .options
            .iter()
            .all(|option| all.options.contains(option)));
        let plain = |outcome: &AttachOutcome| {
            outcome
                .options
                .iter()
                .filter(|option| option.substitution.is_none())
                .cloned()
                .collect::<Vec<_>>()
        };
        // (The plain passes have the same millisecond, which they almost always keep.)
        if !limited.inference.plain.timed_out() {
            assert_eq!(plain(&limited), plain(&all));
        }
    }

    /// A search whose time runs out in the middle of a step finds some of the instances, and no
    /// instance that is not one; and what it left behind is not taken for the whole: a search
    /// that shares its cells with it afterwards finds everything.
    #[test]
    fn a_search_cut_by_the_clock_is_incomplete_not_wrong() {
        let (sig, s, q, _) = independent();
        let haystack = small_haystack(&q);
        let all = found(&sig, s, &haystack, Boundary::Target, &[]);
        assert!(all.len() > 5, "{}", all.len());

        let cells = Pool::new(&sig, haystack.generators().into_keys()).cells();
        let (mut cut_short, mut complete) = (0, 0);
        for checks in (0..24).chain([30, 50, 80, 120, 200, 400, 100_000]) {
            let search = |cells: &Rc<Cells>| {
                InferSearch::new(
                    &sig,
                    Rc::clone(cells),
                    s,
                    &haystack,
                    Boundary::Target,
                    &[],
                    true,
                )
                .unwrap()
            };
            let limit = deadline::after_checks(checks);
            let mut cut = search(&cells);
            while !cut.step() {}
            let expired = limit.expired();
            drop(limit);
            let some: Vec<_> = cut
                .finish()
                .into_iter()
                .map(|inferred| (inferred.substitution, inferred.embedding))
                .collect();
            assert!(
                some.iter().all(|instance| all.contains(instance)),
                "{checks} checks: an instance that is not one"
            );
            if expired {
                cut_short += 1;
            } else {
                complete += 1;
                assert_eq!(some, all, "{checks} checks: a search that was not cut");
            }

            // The candidates and composites it made are still good.
            let mut again = search(&cells);
            while !again.step() {}
            let again: Vec<_> = again
                .finish()
                .into_iter()
                .map(|inferred| (inferred.substitution, inferred.embedding))
                .collect();
            assert_eq!(again, all, "{checks} checks: what was left behind");
        }
        assert!(cut_short > 20 && complete > 0, "{cut_short} / {complete}");
    }

    /// The checkpoints of the job that a worker keeps in case it has to be ended are the options
    /// that there are by then, and say what was not finished.
    #[test]
    fn checkpoints_are_what_the_search_has_by_then() {
        let mut job = attach_job(false, InferenceLimits::UNLIMITED);
        let mut checkpoints = vec![];
        let outcome = loop {
            if let Some(checkpoint) = job.checkpoint() {
                checkpoints.push(checkpoint);
            }
            if let Some(outcome) = job.step() {
                break outcome;
            }
        };
        assert!(!outcome.inference.forced, "{:?}", outcome.inference);
        assert!(!outcome.options.is_empty());
        // One as the instances are begun, and one for each schema that is done.
        assert!(checkpoints.len() >= 2, "{}", checkpoints.len());
        for checkpoint in &checkpoints {
            assert!(checkpoint.inference.forced);
            assert!(checkpoint
                .options
                .iter()
                .all(|option| outcome.options.contains(option)));
        }
        // The first has the options that are not instances, and says that the first schema is
        // not done; the last has everything, as nothing was left.
        let first = &checkpoints[0];
        assert!(first
            .options
            .iter()
            .all(|option| option.substitution.is_none()));
        assert_eq!(first.inference.cut_short.len(), 1);
        let last = checkpoints.last().unwrap();
        assert_eq!(last.options, outcome.options);
        assert!(!last.inference.timed_out(), "{:?}", last.inference);
    }

    /// What an executor says it has when it is asked for a search: answers that do for the
    /// request, which are the checkpoints of the job, and the last of them is the whole of what
    /// there was before the schema being searched.
    #[test]
    fn an_executor_says_what_it_has_as_it_goes() {
        use crate::remote::{Answer, Executor, Output, Remote, Work};

        let (sig, _, q, _) = independent();
        let proof = ProofState {
            signature: sig,
            workspace: Some(Workspace::new(small_haystack(&q))),
            ..ProofState::default()
        };
        let work = Work::AttachOptions {
            point: vec![],
            weak_units: false,
            limits: InferenceLimits::UNLIMITED,
            only: None,
        };
        let mut remote = Remote::default();
        let request = remote.request(&proof, work, 0, false);
        let mut checkpoints = vec![];
        let response =
            Executor::default()
                .run_with_checkpoints(&request, &mut |_| {}, &mut |bytes| checkpoints.push(bytes));
        let outcome_of = |remote: &mut Remote, bytes: &[u8]| match remote.response(bytes) {
            Ok(Answer::Done {
                output: Output::AttachOptions(outcome),
                ..
            }) => outcome,
            other => panic!("{other:?}"),
        };
        let whole = outcome_of(&mut remote, &response);
        assert!(!whole.inference.forced);
        assert!(whole.options.iter().any(|o| o.substitution.is_some()));
        // (How many depends on how long they take to make.)
        assert!(!checkpoints.is_empty());
        let mut instances_before = 0;
        for bytes in &checkpoints {
            let partial = outcome_of(&mut remote, bytes);
            assert!(partial.inference.forced);
            assert!(partial.options.iter().all(|o| whole.options.contains(o)));
            let instances = partial
                .options
                .iter()
                .filter(|o| o.substitution.is_some())
                .count();
            assert!(instances >= instances_before, "checkpoints only grow");
            instances_before = instances;
        }
    }

    fn generator(d: &DiagramN) -> Generator {
        d.max_generator().generator
    }

    fn make_placeholder(sig: &mut Signature, g: Generator) {
        let node = sig.find_node(g).unwrap();
        sig.update(&SignatureEdit::Edit(
            node,
            SignatureItemEdit::MakePlaceholder(true),
        ))
        .unwrap();
    }

    /// `x`; placeholder `P : x → x`; `Q, R : x → x`; the schema `A : P ⇒ P`; and `Q ≫ R`.
    fn setup() -> (
        Signature,
        Generator,
        DiagramN,
        DiagramN,
        Generator,
        DiagramN,
    ) {
        let mut sig = Signature::default();
        let x: Diagram = sig.create_generator_zero("x").into();
        let mut cell = |s: Diagram, t: Diagram| {
            sig.create_generator(s, t, "c", Invertibility::Directed)
                .unwrap()
        };
        let p = cell(x.clone(), x.clone());
        let q = cell(x.clone(), x.clone());
        let r = cell(x.clone(), x);
        let a = cell(p.clone().into(), p.clone().into());
        let qr = q.attach(&r, Boundary::Target, &[]).unwrap();
        let (p, a) = (generator(&p), generator(&a));
        make_placeholder(&mut sig, p);
        (sig, p, q, qr, a, r)
    }

    #[test]
    fn infers_single_cells_and_composites_at_the_point() {
        let (sig, p, q, qr, a, r) = setup();
        let haystack: Diagram = qr.clone().into();

        let replacements = |point: usize| {
            let mut found: Vec<Diagram> = sig
                .infer_instances(a, &haystack, Boundary::Target, &[Height::Singular(point)])
                .into_iter()
                .map(|inferred| {
                    assert_eq!(inferred.substitution.len(), 1);
                    assert_eq!(inferred.substitution[0].0, p);
                    inferred.substitution[0].1.clone()
                })
                .collect();
            found.sort_by_key(|d| format!("{d:?}"));
            found
        };

        let mut expected: Vec<Diagram> = vec![q.into(), qr.clone().into()];
        expected.sort_by_key(|d| format!("{d:?}"));
        assert_eq!(replacements(0), expected);

        let mut expected: Vec<Diagram> = vec![r.into(), qr.into()];
        expected.sort_by_key(|d| format!("{d:?}"));
        assert_eq!(replacements(1), expected);
    }

    #[test]
    fn inferred_instances_attach() {
        let (mut sig, _, _, qr, a, _) = setup();
        let haystack: Diagram = qr.clone().into();
        let inferred = sig
            .infer_instances(a, &haystack, Boundary::Target, &[Height::Singular(0)])
            .into_iter()
            .find(|inferred| inferred.substitution[0].1 == qr.clone().into())
            .unwrap();
        let instance = sig.instantiate(a, &inferred.substitution).unwrap();
        let Diagram::DiagramN(diagram) = sig.generator_info(instance).unwrap().diagram.clone()
        else {
            unreachable!()
        };
        // The instance's source is Q ≫ R, so it attaches where it was found.
        let attached = qr
            .clone()
            .identity()
            .attach(&diagram, Boundary::Target, &inferred.embedding)
            .unwrap();
        assert_eq!(attached.target(), qr.into());
    }

    /// `x`; placeholder `P : x → x`; `Q, R : x → x`; and the invertible (or not) schema
    /// `A : P ⇒ R`, which says that `P` is `R` (all that `A` mentions is `P`).
    fn equivalence(invertibility: Invertibility) -> (Signature, Generator, Generator) {
        let mut sig = Signature::default();
        let x: Diagram = sig.create_generator_zero("x").into();
        let mut cell = |s: Diagram, t: Diagram, invertibility| {
            sig.create_generator(s, t, "c", invertibility).unwrap()
        };
        let p = cell(x.clone(), x.clone(), Invertibility::Directed);
        let r = cell(x.clone(), x, Invertibility::Directed);
        let a = cell(p.clone().into(), r.clone().into(), invertibility);
        let (p, a) = (generator(&p), generator(&a));
        make_placeholder(&mut sig, p);
        (sig, p, a)
    }

    /// An invertible schema is attached as its inverse as well, along its target: an instance
    /// is found where the diagram has what the instance ends at, as with a plain cell. (The
    /// instance `T` of "`Ẋ ◁ δ = δ ▷ (Ẋ, Ẋ)`" was only found by dragging the instance made by
    /// hand, whose inverse is what attaches there.)
    #[test]
    fn an_invertible_schema_is_found_by_its_target_too() {
        let (mut sig, p, a) = equivalence(Invertibility::Invertible);
        let r = sig.generator_info(a).unwrap().diagram.clone();
        let Diagram::DiagramN(r) = r else {
            unreachable!()
        };
        // `A : P ⇒ R`: a diagram with `R` in it has no `A` to attach, but `A⁻¹ : R ⇒ P`, for
        // every replacement of `P`.
        let haystack: Diagram = r.target().into();
        let Diagram::DiagramN(haystack_n) = &haystack else {
            unreachable!()
        };
        assert_eq!(haystack_n.dimension(), 1);
        let found = sig.infer_instances(a, &haystack, Boundary::Target, &[]);
        let forward: Vec<_> = found.iter().filter(|f| !f.inverse).collect();
        let backward: Vec<_> = found.iter().filter(|f| f.inverse).collect();
        // Backwards, every replacement of `P` has `R` to start from, and `P := R` is among them.
        assert!(!backward.is_empty(), "{found:?}");
        assert!(backward
            .iter()
            .any(|f| f.substitution == vec![(p, haystack.clone())]));
        assert!(backward.len() >= forward.len());
        for f in &found {
            // The cell found is the instance or its inverse, from where it was found.
            let instance = sig.instantiate(a, &f.substitution).unwrap();
            let Diagram::DiagramN(instance) = sig.generator_info(instance).unwrap().diagram.clone()
            else {
                unreachable!()
            };
            let cell = if f.inverse {
                instance.inverse()
            } else {
                instance
            };
            let attached = haystack_n
                .clone()
                .identity()
                .attach(&cell, Boundary::Target, &f.embedding)
                .unwrap();
            assert_eq!(attached.source(), haystack, "{f:?}");
            assert_eq!(
                f.diagram.slice(Boundary::Source),
                cell.slice(Boundary::Source)
            );
            // (Not `P := P`, which is no instance.)
            assert!(f
                .substitution
                .iter()
                .all(|(g, d)| *g != p || *d != sig.generator_info(p).unwrap().diagram));
        }
        // A directed schema has no inverse to find.
        let (sig, _, a) = equivalence(Invertibility::Directed);
        let r = sig.generator_info(a).unwrap().diagram.clone();
        let Diagram::DiagramN(r) = r else {
            unreachable!()
        };
        let haystack: Diagram = r.target().into();
        let found = sig.infer_instances(a, &haystack, Boundary::Target, &[]);
        assert!(found.iter().all(|f| !f.inverse));
    }

    /// `x`; placeholder `P : x → x`; `Q : x → x`; the schema `D : P ⇒ P`; the schema `S : D ⇛ D`
    /// (which has a cell that depends on `P` to attach along); and the instance `D[Q⁴/P]`, made
    /// by hand, which is the diagram to attach to.
    fn read_off() -> (Signature, Generator, Diagram) {
        let mut sig = Signature::default();
        let x: Diagram = sig.create_generator_zero("x").into();
        let mut cell = |s: Diagram, t: Diagram| {
            sig.create_generator(s, t, "c", Invertibility::Directed)
                .unwrap()
        };
        let p = cell(x.clone(), x.clone());
        let q = cell(x.clone(), x);
        let d = cell(p.clone().into(), p.clone().into());
        let s = cell(d.clone().into(), d.clone().into());
        let (p, d, s) = (generator(&p), generator(&d), generator(&s));
        make_placeholder(&mut sig, p);
        let mut q4 = q.clone();
        for _ in 0..3 {
            q4 = q4.attach(&q, Boundary::Target, &[]).unwrap();
        }
        let instance = sig.instantiate(d, &[(p, q4.into())]).unwrap();
        let haystack = sig.generator_info(instance).unwrap().diagram.clone();
        (sig, s, haystack)
    }

    /// The replacement for `P` is not made of at most three cells of the diagram (`Q⁴`), but the
    /// diagram has an instance of `D` that says what `P` was replaced with when it was made, and
    /// `S` can only attach where its `D[σ]` is. Reading it off finds it; searching does not.
    #[test]
    fn a_cell_in_the_diagram_says_what_the_replacement_is() {
        let (sig, s, haystack) = read_off();
        let search = |seeding: bool| {
            let cells = Pool::new(&sig, haystack.generators().into_keys()).cells();
            let mut search =
                InferSearch::new(&sig, cells, s, &haystack, Boundary::Target, &[], seeding)
                    .unwrap();
            while !search.step() {}
            search.finish()
        };
        let searched = search(false);
        let read = search(true);
        assert!(searched.is_empty(), "{searched:?}");
        assert_eq!(read.len(), 1, "{read:?}");
        assert_eq!(read[0].substitution.len(), 1);
        // It is a well typed instance, and it attaches: the instance of `S` made with it has
        // the instance of `D` in the diagram as its source.
        let mut sig = sig;
        let instance = sig.instantiate(s, &read[0].substitution).unwrap();
        let Diagram::DiagramN(instance) = sig.generator_info(instance).unwrap().diagram.clone()
        else {
            unreachable!()
        };
        let Diagram::DiagramN(haystack) = haystack else {
            unreachable!()
        };
        let attached = haystack
            .clone()
            .identity()
            .attach(&instance, Boundary::Target, &read[0].embedding)
            .unwrap();
        assert_eq!(attached.source(), haystack.into());
    }

    /// A diagram with no instance of what `S` attaches along has no instance of `S` to find,
    /// which is known at once: nothing is searched for.
    #[test]
    fn no_cell_in_the_diagram_to_read_means_nothing_to_find() {
        let (sig, s, _) = read_off();
        let d = sig
            .iter()
            .find(|info| {
                info.instance.is_none() && info.diagram.dimension() == 2 && !info.placeholder
            })
            .unwrap()
            .generator;
        // `D` itself, with `P` not replaced, is a cell of the diagram: but not an instance.
        let Diagram::DiagramN(d) = sig.generator_info(d).unwrap().diagram.clone() else {
            unreachable!()
        };
        let haystack: Diagram = d.into();
        let cells = Pool::new(&sig, haystack.generators().into_keys()).cells();
        let mut search =
            InferSearch::new(&sig, cells, s, &haystack, Boundary::Target, &[], true).unwrap();
        assert!(search.step(), "nothing to search");
        assert!(search.finish().is_empty());
    }
}
