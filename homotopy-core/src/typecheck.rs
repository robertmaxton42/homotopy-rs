use std::{
    cell::RefCell,
    convert::{Into, TryInto},
    rc::Rc,
};

use homotopy_common::{
    hash::{FastHashMap, FastHashSet},
    idx::IdxVec,
};
use itertools::Itertools;
use petgraph::{graph::NodeIndex, visit::EdgeRef};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    collapse::Collapsible,
    common::{Direction, Generator, Height, Label, SingularHeight},
    diagram::{Diagram, DiagramN},
    manifold::is_manifold,
    rewrite::{Cone, Cospan, Rewrite, RewriteN},
    scaffold::{Explodable, Scaffold},
    signature::{GeneratorInfo, Invertibility, Signature},
    Diagram0, Orientation, Rewrite0, SliceIndex,
};

type Point = Vec<SingularHeight>;

#[derive(Debug, Error)]
pub enum TypeError {
    #[error("diagram contains an unknown generator: {0:?}")]
    UnknownGenerator(Generator),

    #[error("diagram is ill-typed")]
    IllTyped,

    #[error("directed generator appears in non-positive orientation")]
    Directed,

    #[error("dualisable generator exhibits illegal non-manifold behaviour")]
    Dualisable,

    /// The time limit set for the thread (see [`homotopy_common::deadline`]) was reached before
    /// the diagram was checked. It says nothing about whether the diagram is well typed, only
    /// that this was not found out: whoever sets a limit takes it as "not known to be".
    #[error("the time limit was reached while typechecking")]
    TimedOut,
}

thread_local! {
    static RESTRICT_CACHE: RefCell<FastHashMap<(Rewrite, Embedding), Rewrite>> = RefCell::new(FastHashMap::default());
}

#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Serialize, Deserialize)]
pub struct Mode {
    pub directed: bool,
    pub dualisable: bool,
    pub simplices: bool,
    pub generator: Option<Generator>,
}

impl Default for Mode {
    fn default() -> Self {
        Self {
            directed: true,
            dualisable: true,
            simplices: true,
            generator: None,
        }
    }
}

pub fn typecheck(
    diagram: &Diagram,
    signature: &impl Signature,
    mode: Mode,
    recursive: bool,
) -> Result<(), TypeError> {
    if !check_dimension(diagram.clone()) {
        return Err(TypeError::IllTyped);
    }

    typecheck_worker(diagram, signature, mode, recursive)
}

fn typecheck_worker(
    diagram: &Diagram,
    signature: &impl Signature,
    mode: Mode,
    recursive: bool,
) -> Result<(), TypeError> {
    let diagram = match diagram {
        Diagram::Diagram0(d) => {
            return if d.generator.dimension == 0 && d.orientation == Orientation::Positive {
                Ok(())
            } else {
                Err(TypeError::IllTyped)
            }
        }
        Diagram::DiagramN(d) => d,
    };

    if recursive {
        typecheck_worker(&diagram.source(), signature, mode, recursive)?;
    }

    for (cospan, (regular0, regular1)) in
        std::iter::zip(diagram.cospans(), diagram.regular_slices().tuple_windows())
    {
        if recursive {
            typecheck_worker(&regular1, signature, mode, recursive)?;
        }

        let target_embeddings = target_points(&[cospan.forward.clone(), cospan.backward.clone()])
            .into_iter()
            .map(|(point, target)| (Embedding::from_point(&point), target));

        for (target_embedding, target) in target_embeddings {
            if homotopy_common::deadline::expired() {
                return Err(TypeError::TimedOut);
            }
            if mode.generator.is_some_and(|g| target.generator != g) {
                continue;
            }
            let info = signature
                .generator_info(target.generator)
                .ok_or(TypeError::UnknownGenerator(target.generator))?;

            // With reflection symmetries, a directed generator may appear negatively oriented,
            // which is checked against its neighbourhood below. It can never fold back on itself.
            let directed = mode.directed && info.invertibility() == Invertibility::Directed;
            let symmetric = directed && !signature.symmetries().is_empty();
            if directed
                && (target.orientation == Orientation::Zero
                    || (!symmetric && target.orientation != Orientation::Positive))
            {
                return Err(TypeError::Directed);
            }

            let source = restrict_diagram(&regular0, &target_embedding.preimage(&cospan.forward));

            let forward = restrict_rewrite(&cospan.forward, &target_embedding);
            let backward = restrict_rewrite(&cospan.backward, &target_embedding);
            let restricted = DiagramN::new(source, vec![Cospan { forward, backward }]);

            if let Invertibility::Dualisable(k) = info.invertibility() {
                if mode.dualisable
                    && diagram.dimension() > target.generator.dimension + k + 1
                    && !is_manifold(restricted.slice(Height::Singular(0)).unwrap())
                {
                    return Err(TypeError::Dualisable);
                }
            }

            if mode.simplices {
                // (Each collapse is not interrupted, but there is a look at the clock between.)
                let observed = collapse_simplicies(restricted.clone(), false);
                if homotopy_common::deadline::expired() {
                    return Err(TypeError::TimedOut);
                }
                if observed != collapse_simplicies(info.diagram().clone(), false) {
                    return Err(TypeError::IllTyped);
                }
            }

            if symmetric && !is_allowed_reflection(restricted, target.generator, info, signature) {
                // (It says no when it runs out of time, which is not that it is not allowed.)
                return Err(if homotopy_common::deadline::expired() {
                    TypeError::TimedOut
                } else {
                    TypeError::Directed
                });
            }
        }
    }

    RESTRICT_CACHE.with_borrow_mut(FastHashMap::clear);

    Ok(())
}

pub fn typecheck_cospan(
    source: Diagram,
    cospan: Cospan,
    signature: &impl Signature,
) -> Result<(), TypeError> {
    let diagram = DiagramN::new(source, vec![cospan]);

    typecheck(&diagram.target(), signature, Mode::default(), false)?;

    // should never return `IllTyped` but may return `NonManifold`
    typecheck(&diagram.into(), signature, Mode::default(), false)
}

fn target_points(rewrites: &[Rewrite]) -> Vec<(Point, Diagram0)> {
    if rewrites.is_empty() {
        return vec![];
    }

    if rewrites[0].dimension() == 0 {
        let target = rewrites.iter().find_map(|r| match r {
            Rewrite::Rewrite0(r) => r.target(),
            Rewrite::RewriteN(_) => panic!(),
        });

        match target {
            Some(target) => {
                return vec![(vec![], target)];
            }
            None => return vec![],
        }
    }

    assert!(rewrites
        .iter()
        .all(|r| r.dimension() == rewrites[0].dimension()));

    let mut target_rewrites: FastHashMap<usize, Vec<Rewrite>> = Default::default();

    for rewrite in rewrites {
        let rewrite: RewriteN = rewrite.clone().try_into().unwrap();
        for target_height in rewrite.targets() {
            let target_rewrites_at_height = target_rewrites.entry(target_height).or_default();

            for source_height in rewrite.singular_preimage(target_height) {
                target_rewrites_at_height.push(rewrite.slice(source_height));
            }

            let cone = rewrite.cone_over_target(target_height).unwrap_left();
            let cone_target = cone.target();

            target_rewrites_at_height.push(cone_target.forward.clone());
            target_rewrites_at_height.push(cone_target.backward.clone());
        }
    }

    let mut targets = Vec::new();

    for (target_height, rewrite_slices) in &target_rewrites {
        targets.extend(target_points(rewrite_slices).into_iter().map(|mut p| {
            p.0.insert(0, *target_height);
            p
        }));
    }

    targets
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Embedding {
    Regular(usize, Rc<Self>),
    Singular(usize, Vec<Rc<Self>>),
    Zero,
}

impl Embedding {
    /// Construct an embedding which contains precisely one singular point.
    fn from_point(point: &[SingularHeight]) -> Self {
        let mut embedding = Self::Zero;

        for &height in point.iter().rev() {
            embedding = Self::Singular(height, vec![Rc::new(embedding)]);
        }

        embedding
    }

    fn preimage(&self, rewrite: &Rewrite) -> Self {
        match self {
            Self::Zero => Self::Zero,
            Self::Regular(height, slice) => {
                let rewrite: &RewriteN = rewrite.try_into().unwrap();
                let preimage_height = rewrite.regular_image(*height);
                Self::Regular(preimage_height, slice.clone())
            }
            Self::Singular(height, slices) => {
                let rewrite: &RewriteN = rewrite.try_into().unwrap();
                let preimage_height = rewrite.regular_image(*height);
                let preimage_slices: Vec<_> = slices
                    .iter()
                    .enumerate()
                    .flat_map(|(target_height, slice)| {
                        rewrite
                            .singular_preimage(target_height + height)
                            .map(|source_height| {
                                Rc::new(slice.preimage(&rewrite.slice(source_height)))
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect();

                if preimage_slices.is_empty() {
                    let cospan = &rewrite.cone_over_target(*height).unwrap_left().target();
                    Self::Regular(
                        preimage_height,
                        Rc::new(slices[0].preimage(&cospan.forward)),
                    )
                } else {
                    Self::Singular(preimage_height, preimage_slices)
                }
            }
        }
    }
}

fn restrict_diagram(diagram: &Diagram, embedding: &Embedding) -> Diagram {
    match embedding {
        Embedding::Zero => {
            assert_eq!(diagram.dimension(), 0);
            diagram.clone()
        }
        Embedding::Regular(height, slice) => {
            let diagram: DiagramN = diagram.clone().try_into().unwrap();
            restrict_diagram(&diagram.slice(Height::Regular(*height)).unwrap(), slice)
                .identity()
                .into()
        }
        Embedding::Singular(height, slices) => {
            let diagram: &DiagramN = diagram.try_into().unwrap();
            assert!(diagram.size() >= height + slices.len());
            let source = restrict_diagram(
                &diagram.slice(Height::Regular(*height)).unwrap(),
                &slices[0].preimage(&diagram.cospans()[*height].forward),
            );
            let cospans = diagram.cospans()[*height..*height + slices.len()]
                .iter()
                .enumerate()
                .map(|(i, cospan)| Cospan {
                    forward: restrict_rewrite(&cospan.forward, &slices[i]),
                    backward: restrict_rewrite(&cospan.backward, &slices[i]),
                })
                .collect();
            DiagramN::new(source, cospans).into()
        }
    }
}

/// Restrict a rewrite to the preimage over the a subdiagram of the target.
fn restrict_rewrite(rewrite: &Rewrite, embedding: &Embedding) -> Rewrite {
    if rewrite.is_identity() {
        return rewrite.clone();
    }

    let cached = RESTRICT_CACHE
        .with_borrow(|cache| cache.get(&(rewrite.clone(), embedding.clone())).cloned());

    if let Some(cached) = cached {
        return cached;
    }

    match embedding {
        Embedding::Zero => {
            assert_eq!(rewrite.dimension(), 0);
            rewrite.clone()
        }
        Embedding::Regular(_, _) => Rewrite::identity(rewrite.dimension()),
        Embedding::Singular(height, slices) => {
            let rewrite: &RewriteN = rewrite.try_into().unwrap();
            let mut restricted_cones: Vec<Cone> = Vec::new();

            for target_height in rewrite.targets() {
                if target_height < *height {
                    continue;
                }
                if target_height >= height + slices.len() {
                    break;
                }

                let embedding_slice = &slices[target_height - *height];

                // TODO: This is quite ugly
                let cone = rewrite.cone_over_target(target_height).unwrap_left();

                let restricted_regular_slices: Vec<_> = cone
                    .regular_slices()
                    .iter()
                    .map(|cone_slice| restrict_rewrite(cone_slice, embedding_slice))
                    .collect();

                let restricted_singular_slices: Vec<_> = cone
                    .singular_slices()
                    .iter()
                    .map(|cone_slice| restrict_rewrite(cone_slice, embedding_slice))
                    .collect();

                let restricted_source: Vec<_> = cone
                    .source()
                    .iter()
                    .enumerate()
                    .map(|(i, cospan)| {
                        let embedding = embedding_slice.preimage(&cone.singular_slices()[i]);
                        let forward = restrict_rewrite(&cospan.forward, &embedding);
                        let backward = restrict_rewrite(&cospan.backward, &embedding);
                        Cospan { forward, backward }
                    })
                    .collect();

                let restricted_target = {
                    let slice = embedding_slice;
                    let forward = restrict_rewrite(&cone.target().forward, slice);
                    let backward = restrict_rewrite(&cone.target().backward, slice);
                    Cospan { forward, backward }
                };

                restricted_cones.push(Cone::new(
                    cone.index - rewrite.regular_image(*height),
                    restricted_source,
                    restricted_target,
                    restricted_regular_slices,
                    restricted_singular_slices,
                ));
            }

            let restricted_rewrite: Rewrite =
                RewriteN::new(rewrite.dimension(), restricted_cones).into();

            RESTRICT_CACHE.with_borrow_mut(|cache| {
                cache.insert(
                    (rewrite.clone().into(), embedding.clone()),
                    restricted_rewrite.clone(),
                )
            });

            restricted_rewrite
        }
    }
}

// `DiagramN` is hash-consed; its `OnceCell` caches do not affect `Hash`/`Eq`.
#[allow(clippy::mutable_key_type)]
fn check_dimension(diagram: Diagram) -> bool {
    fn worker(
        diagram: Diagram,
        max_dimension: usize,
        checked: &mut FastHashMap<DiagramN, usize>,
    ) -> bool {
        match diagram {
            Diagram::Diagram0(d) => d.generator.dimension <= max_dimension,
            Diagram::DiagramN(diagram) => {
                if checked
                    .get(&diagram)
                    .is_some_and(|checked| *checked <= max_dimension)
                {
                    return true;
                }

                if !diagram
                    .slices()
                    .enumerate()
                    .all(|(i, slice)| worker(slice, max_dimension + i % 2, checked))
                {
                    return false;
                }

                checked.insert(diagram, max_dimension);
                true
            }
        }
    }

    // We cache the smallest dimension at which we have checked a diagram. Whenever we check a
    // diagram at a smaller dimension we can short circuit.
    let mut checked: FastHashMap<DiagramN, usize> = FastHashMap::default();
    worker(diagram, 0, &mut checked)
}

type Simplex = Vec<NodeIndex>; // An n-simplex is a list of n + 1 vertices.
/// Checks that the neighbourhood of a point of a directed generator is a reflection of the
/// generator that the signature's symmetries allow (see [`allowed_reflections`]).
fn is_allowed_reflection(
    neighbourhood: DiagramN,
    generator: Generator,
    info: &impl GeneratorInfo,
    signature: &impl Signature,
) -> bool {
    let Diagram::DiagramN(diagram) = info.diagram() else {
        return true;
    };
    let observed = collapse_simplicies(neighbourhood, true);
    allowed_reflections(generator, diagram, signature)
        .into_iter()
        .any(|mask| {
            // Each of these can take long, and there are many (see `deadline`): out of time is no.
            if homotopy_common::deadline::expired() {
                return false;
            }
            let reflected = diagram.reflect(&Direction::from_mask(mask, diagram.dimension()), 0);
            collapse_simplicies(reflected, true) == observed
        })
}

/// The reflections (as masks of directions, see [`Symmetry::mask`](crate::signature::Symmetry::mask)) in which a directed
/// generator may appear.
///
/// These are composites of a reflection given by a symmetry of the signature, restricted to the
/// directions of the generator, with a rotation: an orientation-preserving reflection which may
/// only reverse those cells in the boundary of the generator that are invertible or dualisable,
/// as in a mate. Rotations through cells that are merely symmetric are not allowed, as they
/// cannot be built from cups and caps.
fn allowed_reflections(
    generator: Generator,
    diagram: &DiagramN,
    signature: &impl Signature,
) -> Vec<u64> {
    let n = generator.dimension;
    let full = (1_u64 << n) - 1;

    // The group generated by the symmetries, restricted to the directions of the generator.
    let mut group = vec![0];
    for symmetry in signature.symmetries() {
        let mask = symmetry.mask() & full;
        if !group.contains(&mask) {
            let translated = group.iter().map(|g| g ^ mask).collect_vec();
            group.extend(translated);
        }
    }

    let directed_boundary_dimensions = Diagram::from(diagram.clone())
        .generators()
        .into_keys()
        .filter(|&g| {
            g != generator
                && g.dimension > 0
                && signature
                    .generator_info(g)
                    .is_some_and(|info| info.invertibility() == Invertibility::Directed)
        })
        .map(|g| g.dimension)
        .collect::<FastHashSet<_>>();
    let is_rotation = |mask: u64| {
        mask.count_ones() & 1 == 0
            && directed_boundary_dimensions
                .iter()
                .all(|&j| (mask & ((1 << j) - 1)).count_ones() & 1 == 0)
    };

    (0..=full)
        .filter(|&mask| group.iter().any(|&g| is_rotation(mask ^ g)))
        .collect()
}

// An n-simplex is a list of (n + 1 choose 2) edges, labelled by the position in the generator
// they come from and, when the simplices are oriented, by the orientation of their target.
type LabelledSimplex = Vec<(Option<Label>, Option<Orientation>)>;

fn collapse_simplicies(
    diagram: impl Into<Diagram>,
    oriented: bool,
) -> FastHashSet<LabelledSimplex> {
    let diagram: Diagram = diagram.into();
    let dimension = diagram.dimension();

    // Construct the fully exploded scaffold of the diagram.
    let mut scaffold: Scaffold<Vec<Height>> = Scaffold::default();
    scaffold.add_node(diagram.into());
    for _ in 0..dimension {
        scaffold = scaffold
            .explode_graph(
                |_, key, si| match si {
                    SliceIndex::Boundary(_) => None,
                    SliceIndex::Interior(h) => Some([key.as_slice(), &[h]].concat()),
                },
                |_, _, _| Some(()),
                |_, (), _| Some(()),
            )
            .unwrap();
    }

    let stratum = |coord: &[Height]| {
        coord.iter().fold(0, |stratum, h| {
            stratum
                + match h {
                    Height::Regular(_) => 0,
                    Height::Singular(_) => 1,
                }
        })
    };

    // Extract the neighbourhoods of each point.
    let mut neighbourhoods: IdxVec<NodeIndex, Vec<Simplex>> =
        IdxVec::splat(vec![], scaffold.node_count());
    for n in scaffold
        .node_indices()
        .sorted_by_cached_key(|n| stratum(&scaffold[*n].key))
    {
        let mut neighbourhood = vec![vec![n]];
        for e in scaffold.edges_directed(n, petgraph::Direction::Incoming) {
            for simplex in &neighbourhoods[e.source()] {
                neighbourhood.push([simplex.as_slice(), &[n]].concat());
            }
        }
        neighbourhoods[n].extend(neighbourhood);
    }

    // Find the central point.
    let central = scaffold
        .node_indices()
        .filter(|n| stratum(&scaffold[*n].key) == dimension)
        .exactly_one()
        .unwrap();

    let label = |a, b| {
        let e = scaffold.find_edge(a, b).unwrap();
        let r: &Rewrite0 = (&scaffold[e].rewrite).try_into().unwrap();
        let orientation = r.target().filter(|_| oriented).map(|t| t.orientation);
        (r.label().cloned(), orientation)
    };

    let (_, union_find) = scaffold.collapse();
    neighbourhoods[central]
        .iter()
        .map(|simplex| {
            // Collapse the simplex.
            let mut simplex = simplex.clone();
            simplex.dedup_by(|x, y| union_find.equiv(*x, *y));

            // Collect the labels of the collapsed simplex.
            let mut labels = vec![];
            let n = simplex.len();
            for k in 1..n {
                for i in 0..n - k {
                    labels.push(label(simplex[i], simplex[i + k]));
                }
            }

            labels
        })
        .collect()
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{signature::SignatureBuilder, Boundary};

    #[test]
    fn associativity() {
        let mut sig = SignatureBuilder::default();

        let x = sig.add_zero();
        let f = sig.add(x, x).unwrap();
        let ff = f.attach(&f, Boundary::Target, &[]).unwrap();
        let m = sig.add(ff, f).unwrap();
        let left = m.attach(&m, Boundary::Source, &[0]).unwrap();
        let right = m.attach(&m, Boundary::Source, &[1]).unwrap();
        let a = sig.add(left, right).unwrap();

        typecheck(&a.into(), &sig, Mode::default(), true).unwrap();
    }
}
