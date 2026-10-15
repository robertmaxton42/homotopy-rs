//! The identity of sheets, and where they end.
//!
//! A *sheet* at a position of the projection of a diagram (see
//! [`Depths::sheets`](super::Depths::sheets)) is one singular height of what the position
//! carries, and there is a cell there. The rewrite along an edge of the scaffold, between the
//! fibres of two neighbouring positions, maps each height at the start to one at the end (its
//! singular image). When the cell is the same at both ends it is the same sheet continuing, and
//! when it is not, something happens there (a cell is formed from the sheets that meet at it, a
//! line ends on a point). So a sheet is a class of (position, height) pairs that are joined by an
//! edge and have the same cell, made with a union-find over the edges, and the number of a class
//! is its [`SheetId`].
//!
//! This is what keeps the lines that meet at a 3-cell different sheets (they are different cells
//! from it), and makes a line that lies along a wire one sheet from one end to the other, whatever
//! the positions that it passes through do beside it.
//!
//! # Boundaries
//!
//! A *boundary* is a place where the sheets that there are change (see [`SheetBoundary`]), at a
//! point of the picture (a position with every coordinate singular, where the cells of the
//! diagram are) and a height `z` there. Count the heights that the edges into the position carry
//! into it (`arms`): where one is carried in, a sheet ends or begins there; where three or
//! more are, sheets meet at the cell (two are a sheet that passes through, with a cell on it).
//!
//! This is not "a cone with no source heights", which is not what a sheet's end is: every edge
//! of the scaffold goes from a position to one of a dimension less (a regular height becomes a
//! singular one), and a sheet that lies along a wire is not at the regions beside it, so the
//! rewrites into the wire have such a cone for it on every edge from them. That is what it is to
//! be a line, not a place where it begins, and listing them would put a boundary along both
//! sides of every wire. Sheets that end or meet along a wire (surfaces joined along a line) are
//! not boundaries of the picture either: the wire is drawn. At a point, though, there is
//! nothing else to say that the sheet is there.
//!
//! The scaffold of [`Depths`](super::Depths) leaves out the regular slices of the later
//! coordinates (the edges from a regular height to a singular one of the next position). Such
//! an edge is a sparse identity followed by an edge of the last coordinate, which is kept, so
//! it has the same cones and nothing is lost (tested against the scaffold with every edge).
//!
//! # Which sheet is in front
//!
//! The projection draws, at each position, the front-most height ([`Depths::node_depth`]),
//! which can be a different sheet at the next position even though it is drawn as one wire. A
//! sheet is *foremost* when no other sheet hides it, as seen over the whole of the diagram: it
//! is the front-most at every position that it shares with another sheet, or at more of them
//! than it is hidden at. A sheet that is alone wherever it is is foremost; two sheets that
//! overlap on only a part of their length are one foremost and one behind along the whole of
//! theirs, and not one front where it happens to be alone and the other where they meet.

use petgraph::{graph::NodeIndex, unionfind::UnionFind, visit::EdgeRef};

use super::{Coordinate, Depths};
use crate::{common::SliceIndex, rewrite::RewriteN, Diagram, Generator, Height, Rewrite};

/// A sheet of the diagram that a [`Depths`] is for: the same at every position that it passes
/// through (see the [module](self)). Only meaningful for the `Depths` that made it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SheetId(pub u32);

impl SheetId {
    /// A short name for the sheet: `S1`, `S2`, ... (the ids start at 0, the names at 1).
    #[must_use]
    pub fn label(self) -> String {
        format!("S{}", self.0 + 1)
    }
}

/// What happens to a sheet at a boundary (see the [module](self)).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum SheetBoundaryKind {
    /// A sheet ends here, at a point of the picture: one height is carried into this one. Seen
    /// from the other side, it begins.
    BeginsOrEnds,
    /// Sheets meet here, at a point of the picture: three or more of the edges into the position
    /// carry heights into the same one. Several merge into one or, seen from the other side, one
    /// splits.
    MergesOrSplits,
}

/// A place where a sheet ends, or where sheets meet.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SheetBoundary<const N: usize> {
    /// The neighbour that carries the sheet in (for a junction, the first that does): the sheet
    /// runs along the edge from here to `to`.
    pub from: Coordinate<N>,
    /// The position that the sheet ends at, or that the sheets meet at.
    pub to: Coordinate<N>,
    /// The singular height at `to` that this is a boundary of.
    pub z: usize,
    /// The sheet at `to` at that height.
    pub sheet: SheetId,
    pub kind: SheetBoundaryKind,
    /// How many heights are carried into that one: 1 for an end, 3 or more for a junction.
    pub branches: usize,
}

#[derive(Clone, Debug)]
pub(super) struct SheetData<const N: usize> {
    /// Where the heights of each node start in `ids`.
    offsets: Vec<usize>,
    sizes: Vec<usize>,
    /// The sheet of each (node, height).
    ids: Vec<SheetId>,
    /// How many heights the edges into each (node, height) carry into it.
    carried: Vec<usize>,
    /// Whether each sheet is foremost.
    foremost: Vec<bool>,
    boundaries: Vec<SheetBoundary<N>>,
    /// For each node, the boundaries that are at it.
    boundaries_to: Vec<Vec<u32>>,
    /// For each sheet, the (node, height) pairs it is at, in the order of the nodes.
    places: Vec<Vec<(NodeIndex, usize)>>,
}

/// The dimension of the part of the picture that a position is of: the number of its coordinates
/// that are not singular (a wire has one, a point none, a region two; the ends of a coordinate
/// are of the regular heights next to them).
fn stratum_dimension<const N: usize>(position: &Coordinate<N>) -> usize {
    position
        .iter()
        .filter(|index| !matches!(index, SliceIndex::Interior(Height::Singular(_))))
        .count()
}

/// Calls `f` with each singular height of the source of the rewrite (of `size` heights) and the
/// height of the target it goes to.
fn for_each_image(size: usize, rewrite: &RewriteN, mut f: impl FnMut(usize, usize)) {
    // A height that is not in a cone goes to the height `shift` further on, where `shift` is
    // one less than the heights of the source for each cone before it.
    let (mut z, mut shift) = (0_usize, 0_isize);
    for cone in rewrite.cone_views() {
        while z < cone.source_start() {
            f(z, z.saturating_add_signed(shift));
            z += 1;
        }
        for covered in z..z + cone.source_len() {
            f(covered, cone.target());
        }
        z += cone.source_len();
        shift += 1 - cone.source_len() as isize;
    }
    while z < size {
        f(z, z.saturating_add_signed(shift));
        z += 1;
    }
}

/// The edges of the scaffold that have a rewrite between fibres that have sheets.
fn edges<const N: usize>(
    depths: &Depths<N>,
) -> impl Iterator<Item = (NodeIndex, NodeIndex, &RewriteN)> {
    depths
        .graph
        .edge_references()
        .filter_map(|edge| match &edge.weight().rewrite {
            Rewrite::RewriteN(rewrite) => Some((edge.source(), edge.target(), rewrite)),
            Rewrite::Rewrite0(_) => None,
        })
}

impl<const N: usize> SheetData<N> {
    #[allow(clippy::cognitive_complexity)]
    pub(super) fn build(depths: &Depths<N>) -> Self {
        let graph = &depths.graph;

        let mut offsets = Vec::with_capacity(graph.node_count());
        let mut sizes = Vec::with_capacity(graph.node_count());
        let mut total = 0;
        for n in graph.node_indices() {
            let size = fibre_size(&graph[n].diagram);
            offsets.push(total);
            sizes.push(size);
            total += size;
        }

        // The cell at each height.
        let generators: Vec<Generator> = graph
            .node_indices()
            .flat_map(|n| match &graph[n].diagram {
                Diagram::DiagramN(d) => d
                    .singular_slices()
                    .map(|slice| slice.max_generator().generator)
                    .collect(),
                Diagram::Diagram0(_) => vec![],
            })
            .collect();

        // How many heights are carried into each, and from where (the first).
        let mut carried = vec![0_usize; total];
        let mut carried_from = vec![NodeIndex::end(); total];
        // Every edge carries each height of its source to one of its target.
        let mut union = UnionFind::new(total);
        if total > 0 {
            for (from, to, rewrite) in edges(depths) {
                for_each_image(sizes[from.index()], rewrite, |z, image| {
                    if image >= sizes[to.index()] {
                        return;
                    }
                    let (source, target) = (offsets[from.index()] + z, offsets[to.index()] + image);
                    carried[target] += 1;
                    if carried_from[target] == NodeIndex::end() {
                        carried_from[target] = from;
                    }
                    // It is the same sheet when it is the same cell.
                    if generators[source] == generators[target] {
                        union.union(source, target);
                    }
                });
            }
        }

        // Number the classes in the order of their first height.
        let labels = union.into_labeling();
        let mut numbers = vec![u32::MAX; total];
        let mut count = 0;
        let ids: Vec<SheetId> = labels
            .iter()
            .map(|&class| {
                if numbers[class] == u32::MAX {
                    numbers[class] = count;
                    count += 1;
                }
                SheetId(numbers[class])
            })
            .collect();

        // Which sheets are foremost: see the module.
        let mut visible = vec![0_u32; count as usize];
        let mut hidden = vec![0_u32; count as usize];
        for n in graph.node_indices() {
            let (start, size) = (offsets[n.index()], sizes[n.index()]);
            let Some(front) = depths.node_depths[n].filter(|&d| size >= 2 && d < size) else {
                continue;
            };
            for z in 0..size {
                let id = ids[start + z].0 as usize;
                if z == front {
                    visible[id] += 1;
                } else {
                    hidden[id] += 1;
                }
            }
        }
        let foremost = visible
            .iter()
            .zip(&hidden)
            .map(|(&v, &h)| h == 0 || v > h)
            .collect();

        let mut places = vec![vec![]; count as usize];
        for n in graph.node_indices() {
            for z in 0..sizes[n.index()] {
                places[ids[offsets[n.index()] + z].0 as usize].push((n, z));
            }
        }

        let mut boundaries = vec![];
        let mut boundaries_to = vec![vec![]; graph.node_count()];
        for n in graph.node_indices() {
            for z in 0..sizes[n.index()] {
                let at = offsets[n.index()] + z;
                if stratum_dimension(&graph[n].key) != 0 {
                    continue;
                }
                let kind = match carried[at] {
                    1 => SheetBoundaryKind::BeginsOrEnds,
                    3.. => SheetBoundaryKind::MergesOrSplits,
                    _ => continue,
                };
                let (from, branches) = (carried_from[at], carried[at]);
                boundaries_to[n.index()].push(boundaries.len() as u32);
                boundaries.push(SheetBoundary {
                    from: graph[from].key,
                    to: graph[n].key,
                    z,
                    sheet: ids[at],
                    kind,
                    branches,
                });
            }
        }

        Self {
            offsets,
            sizes,
            ids,
            carried,
            foremost,
            boundaries,
            boundaries_to,
            places,
        }
    }

    pub(super) fn places(&self, id: SheetId) -> &[(NodeIndex, usize)] {
        self.places.get(id.0 as usize).map_or(&[], Vec::as_slice)
    }

    /// The sheets at a node, by height.
    pub(super) fn ids_at(&self, node: NodeIndex) -> &[SheetId] {
        let start = self.offsets[node.index()];
        &self.ids[start..start + self.sizes[node.index()]]
    }

    pub(super) fn id(&self, node: NodeIndex, z: usize) -> Option<SheetId> {
        (z < self.sizes[node.index()]).then(|| self.ids[self.offsets[node.index()] + z])
    }

    pub(super) fn arms(&self, node: NodeIndex, z: usize) -> usize {
        if z < self.sizes[node.index()] {
            self.carried[self.offsets[node.index()] + z]
        } else {
            0
        }
    }

    pub(super) const fn count(&self) -> usize {
        self.foremost.len()
    }

    pub(super) fn foremost(&self, id: SheetId) -> bool {
        self.foremost.get(id.0 as usize).copied().unwrap_or(false)
    }

    pub(super) fn boundaries(&self) -> &[SheetBoundary<N>] {
        &self.boundaries
    }

    pub(super) fn boundaries_to(&self, node: NodeIndex) -> impl Iterator<Item = &SheetBoundary<N>> {
        self.boundaries_to[node.index()]
            .iter()
            .map(|&i| &self.boundaries[i as usize])
    }
}

/// The number of singular heights of a fibre: its sheets.
fn fibre_size(fibre: &Diagram) -> usize {
    match fibre {
        Diagram::Diagram0(_) => 0,
        Diagram::DiagramN(d) => d.size(),
    }
}

/// How surfaces are shaded by the sheets behind them: the thickness of each, in a few steps from
/// the thinnest surface drawn.
///
/// A surface of the picture is a triangle of the simplicial complex, and it belongs to the
/// position that is its first corner ([`Depths::surface_position`]): a region, where the cells of
/// the projection are in the two coordinates of the view, with its own fibre and so its own
/// number of sheets. That number is the thickness of the triangle (not the fewest of its three
/// corners, which are mostly on the wires and vertices around the region, and those have fewer
/// sheets than the region next to them where sheets end on the wire). It is the same position
/// that hovering over the triangle lists the sheets of, so that a surface shaded for `k` is one
/// that the hover says has `k` sheets.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Shading {
    min: usize,
    max: usize,
    levels: usize,
}

impl Shading {
    /// The shading of these surfaces (the visible triangles of a diagram), in `levels` steps.
    ///
    /// # Panics
    ///
    /// If `levels` is 0.
    pub fn new<const N: usize>(
        depths: &Depths<N>,
        surfaces: impl IntoIterator<Item = [Coordinate<N>; 3]>,
        levels: usize,
    ) -> Self {
        assert!(levels > 0, "there is at least one step");
        let (min, max) = surfaces
            .into_iter()
            .map(|corners| depths.surface_thickness(&corners))
            .fold(None, |range: Option<(usize, usize)>, count| {
                Some(range.map_or((count, count), |(lo, hi)| (lo.min(count), hi.max(count))))
            })
            .unwrap_or_default();
        Self { min, max, levels }
    }

    /// Whether every surface is as thin as every other, so that none is shaded.
    #[must_use]
    pub const fn is_flat(&self) -> bool {
        self.max <= self.min
    }

    /// The step of the surface with these corners, from 0 (as thin as the thinnest) up.
    #[must_use]
    pub fn step<const N: usize>(&self, depths: &Depths<N>, corners: &[Coordinate<N>; 3]) -> usize {
        if self.is_flat() {
            0
        } else {
            thickness_step(
                depths.surface_thickness(corners),
                self.min,
                self.max,
                self.levels,
            )
        }
    }
}

/// How thick one surface is, in a few steps.
///
/// `count` sheets pass through it, and the thinnest and thickest surfaces of the diagram have
/// `min` and `max`. The step is from 0 (as few as the
/// thinnest, which is not shaded) to `levels - 1` (as many as the thickest). With a range that
/// fits in the steps they are the differences from the minimum; with a wider range that
/// difference is scaled to fit and rounded, so that the thickest is the last step and anything
/// above the minimum is at least the first.
///
/// # Panics
///
/// If `levels` is 0.
#[must_use]
pub fn thickness_step(count: usize, min: usize, max: usize, levels: usize) -> usize {
    assert!(levels > 0, "there is at least one step");
    let top = levels - 1;
    let above = count.saturating_sub(min);
    let range = max.saturating_sub(min);
    if above == 0 || top == 0 {
        0
    } else if range <= top {
        above
    } else {
        ((above * top + range / 2) / range).clamp(1, top)
    }
}

/// [`thickness_step`] of each of `counts`, with the thinnest and thickest of them for `min` and
/// `max`.
///
/// # Panics
///
/// If `levels` is 0.
#[must_use]
pub fn thickness_steps(counts: &[usize], levels: usize) -> Vec<usize> {
    let (Some(&min), Some(&max)) = (counts.iter().min(), counts.iter().max()) else {
        assert!(levels > 0, "there is at least one step");
        return vec![];
    };
    counts
        .iter()
        .map(|&count| thickness_step(count, min, max, levels))
        .collect()
}
