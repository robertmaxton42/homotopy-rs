//! The identity of sheets, and where they end.
//!
//! A *sheet* at a position of the projection of a diagram (see
//! [`Depths::sheets`](super::Depths::sheets)) is one singular height of what the position
//! carries. The rewrite along an edge of the scaffold, between the fibres of two neighbouring
//! positions, maps each height at the start to one at the end (its singular image), and when
//! that is the sheet continuing it is the same sheet at both ends. So a sheet is a class of
//! (position, height) pairs, made with a union-find over the edges, and the number of a class is
//! its [`SheetId`].
//!
//! *Where it is not one sheet*: the heights that an edge carries into one height of its target
//! are `e`, counted over all the edges into that (position, height). One carried in is the end of
//! a sheet, two are a sheet passing through (straight, or turning a corner, or a cap: a sheet
//! that comes back to the same position at another height), and three or more are a junction,
//! where sheets meet: such as the point of a 3-cell where four lines meet, or a line splitting
//! in two. A junction is not joined to the sheets that meet at it (it is a sheet of its own, a
//! point), because then every line that meets another would be the same sheet as it.
//!
//! # Boundaries
//!
//! The *boundaries* of sheets are the ends (`e` is 1) and the junctions (`e` is 3 or more).
//! Every edge of the scaffold goes from a position to one of a dimension less (a regular height
//! becomes a singular one), and a sheet that lies along a wire is not at the regions beside it:
//! the rewrites into the wire have a cone with no source heights for it, which is what it is to
//! be a wire, not a place where it begins. So those are not looked at: a sheet only ends at a
//! position of a lower dimension than the highest it reaches (a line is of dimension one in
//! the picture and its ends are points; a surface of two, and its ends are wires). Where the
//! sheet comes from is the position of the one edge that carries it.
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
use crate::{common::SliceIndex, rewrite::RewriteN, Diagram, Height, Rewrite};

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

/// What happens to a sheet at a boundary.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum SheetBoundaryKind {
    /// A sheet ends here: only one of the edges into the position carries it, and it is of a
    /// lower dimension than the sheet reaches elsewhere. Seen from the other side, it begins.
    BeginsOrEnds,
    /// Sheets meet here: three or more of the edges into the position carry heights into the
    /// same one. Several merge into one or, seen from the other side, one splits.
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
    /// Whether each sheet is foremost.
    foremost: Vec<bool>,
    boundaries: Vec<SheetBoundary<N>>,
    /// For each node, the boundaries that are at it.
    boundaries_to: Vec<Vec<u32>>,
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

        // How many heights are carried into each, and from where (the first).
        let mut carried = vec![0_usize; total];
        let mut carried_from = vec![NodeIndex::end(); total];
        // Every edge carries each height of its source to one of its target.
        let mut carries = vec![];
        if total > 0 {
            for (from, to, rewrite) in edges(depths) {
                for_each_image(sizes[from.index()], rewrite, |z, image| {
                    if image < sizes[to.index()] {
                        let target = offsets[to.index()] + image;
                        carried[target] += 1;
                        if carried_from[target] == NodeIndex::end() {
                            carried_from[target] = from;
                        }
                        carries.push((offsets[from.index()] + z, target));
                    }
                });
            }
        }

        // Join what is carried along, but not into a junction.
        let mut union = UnionFind::new(total);
        for &(source, target) in &carries {
            if carried[target] <= 2 {
                union.union(source, target);
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

        // How far each sheet reaches in the picture, as the highest dimension of a position that
        // it passes through.
        let mut reach = vec![0_usize; count as usize];
        for n in graph.node_indices() {
            let dimension = stratum_dimension(&graph[n].key);
            for z in 0..sizes[n.index()] {
                let id = ids[offsets[n.index()] + z].0 as usize;
                reach[id] = reach[id].max(dimension);
            }
        }

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

        let mut boundaries = vec![];
        let mut boundaries_to = vec![vec![]; graph.node_count()];
        for n in graph.node_indices() {
            for z in 0..sizes[n.index()] {
                let at = offsets[n.index()] + z;
                let kind = match carried[at] {
                    1 if stratum_dimension(&graph[n].key) < reach[ids[at].0 as usize] => {
                        SheetBoundaryKind::BeginsOrEnds
                    }
                    3.. => SheetBoundaryKind::MergesOrSplits,
                    _ => continue,
                };
                boundaries_to[n.index()].push(boundaries.len() as u32);
                boundaries.push(SheetBoundary {
                    from: graph[carried_from[at]].key,
                    to: graph[n].key,
                    z,
                    sheet: ids[at],
                    kind,
                    branches: carried[at],
                });
            }
        }

        Self {
            offsets,
            sizes,
            ids,
            foremost,
            boundaries,
            boundaries_to,
        }
    }

    pub(super) fn id(&self, node: NodeIndex, z: usize) -> Option<SheetId> {
        (z < self.sizes[node.index()]).then(|| self.ids[self.offsets[node.index()] + z])
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

/// How thick each surface is, in a few steps.
///
/// `counts` are the numbers of sheets that pass through the surfaces, and the result is, for
/// each, a step from 0 (as few as the thinnest surface has, which is not shaded) to `levels - 1`
/// (as many as the thickest). With a range that fits in the steps they are the differences from
/// the minimum; with a wider range they are that difference scaled to fit, rounded, so that the
/// thickest is the last step and anything above the minimum is at least the first.
///
/// # Panics
///
/// If `levels` is 0.
#[must_use]
pub fn thickness_steps(counts: &[usize], levels: usize) -> Vec<usize> {
    assert!(levels > 0, "there is at least one step");
    let (Some(&min), Some(&max)) = (counts.iter().min(), counts.iter().max()) else {
        return vec![];
    };
    let top = levels - 1;
    counts
        .iter()
        .map(|&count| {
            let above = count - min;
            let range = max - min;
            if above == 0 || top == 0 {
                0
            } else if range <= top {
                above
            } else {
                ((above * top + range / 2) / range).max(1)
            }
        })
        .collect()
}
