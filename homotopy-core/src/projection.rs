//! Diagrams of higher dimensions can be projected into 2 dimensions to be presented in a user
//! interface.
//!
//! This module contains diagram analyses which that calculate various aspects about the
//! 2-dimensional projection of a diagram.
//!
//! In order to avoid potentially costly recomputations and accidental quadratic complexity when a
//! diagram is traversed again for every point, the analyses are performed for the entire diagram
//! at once and the results are cached for efficient random-access retrieval.
use std::{cell::OnceCell, cmp::Ordering};

use homotopy_common::{
    hash::{FastHashMap, FastHasher},
    idx::IdxVec,
};
use itertools::Itertools;
use petgraph::{
    graph::{EdgeIndex, NodeIndex},
    visit::{EdgeRef, IntoNodeReferences, Topo, Walker},
    EdgeDirection,
};

use crate::{
    common::{DimensionError, SliceIndex},
    layout::Layout,
    scaffold::{Explodable, Scaffold, ScaffoldNode},
    Boundary, Diagram, Diagram0, DiagramN, Direction, Height, Rewrite, RewriteN,
};

mod sheets;

use self::sheets::SheetData;
pub use self::sheets::{thickness_step, thickness_steps, Shading, SheetBoundary, SheetId};

type Coordinate<const N: usize> = [SliceIndex; N];

/// The most echoes that a wire has (see [`Depths::echoes`]).
pub const MAX_ECHOES: usize = 3;

#[derive(Copy, Clone, Debug)]
pub enum Homotopy {
    Complex,
    Duality,
    Identity,
    HalfBraid,
    FullBraid,
}

/// Diagram analysis that determines the generator displayed at any point in the 2-dimensional
/// projection of a diagram, as well as information about homotopies.
#[derive(Clone, Debug)]
pub struct Projection<const N: usize> {
    generators: IdxVec<NodeIndex, Diagram0>,
    front_generators: IdxVec<NodeIndex, (Diagram0, bool)>,
    homotopies: IdxVec<NodeIndex, Option<Homotopy>>,
    coord_to_node: FastHashMap<Coordinate<N>, NodeIndex>,
}

impl<const N: usize> Projection<N> {
    pub fn new(
        diagram: &Diagram,
        layout: &Layout<N>,
        depths: &Depths<N>,
    ) -> Result<Self, DimensionError> {
        use Height::Singular;
        use SliceIndex::Interior;

        if diagram.dimension() < N {
            return Err(DimensionError);
        }

        // Construct the exploded graph.
        let mut graph = Scaffold::default();
        graph.add_node(ScaffoldNode::new(
            [Boundary::Source.into(); N],
            diagram.clone(),
        ));
        for i in 0..N {
            graph = graph.explode_graph(
                |_, key, si| {
                    let mut key = *key;
                    key[i] = si;
                    Some(key)
                },
                |_, _, r| (i == 0).then(|| r.direction()),
                |_, key, r| (i > 0 && r.is_atomic()).then_some(*key),
            )?;
        }

        let mut generators = IdxVec::with_capacity(graph.node_count());
        let mut homotopies = IdxVec::with_capacity(graph.node_count());
        let mut front_generators = IdxVec::with_capacity(graph.node_count());
        let mut coord_to_node =
            FastHashMap::with_capacity_and_hasher(graph.node_count(), FastHasher::default());

        for n in graph.node_indices() {
            let coord = graph[n].key;
            let diagram = &graph[n].diagram;

            let g = diagram.max_generator();
            let depth = depths.node_depth(coord);
            let front_g = match depth {
                None => diagram.max_generator(),
                Some(i) => {
                    let diagram: &DiagramN = diagram.try_into()?;
                    diagram.slice(Height::Singular(i)).unwrap().max_generator()
                }
            };
            let is_identity = {
                // Find the edges in the front layer.
                let front_layer = graph
                    .edges_directed(n, EdgeDirection::Incoming)
                    .filter(|e| {
                        depths.edge_depth(graph[e.source()].key, graph[e.target()].key) == depth
                    })
                    .map(|e| e.weight())
                    .collect_vec();

                // Split them into inputs and outputs.
                let inputs = front_layer
                    .iter()
                    .filter(|edge| edge.key == Direction::Forward)
                    .map(|edge| &edge.rewrite)
                    .collect_vec();
                let outputs = front_layer
                    .iter()
                    .filter(|edge| edge.key == Direction::Backward)
                    .map(|edge| &edge.rewrite)
                    .collect_vec();

                // The node is an identity if there is one input and one output, and both are (locally) identities.
                match (inputs.split_first(), outputs.split_first()) {
                    (Some((&input, &[])), Some((&output, &[]))) => match depth {
                        None => input.is_identity() && output.is_identity(),
                        Some(i) => {
                            let input: &RewriteN = input.try_into()?;
                            let output: &RewriteN = output.try_into()?;
                            input.cone_over_target(i).is_right()
                                && output.cone_over_target(i).is_right()
                        }
                    },
                    _ => false,
                }
            };

            let h = || -> Option<Homotopy> {
                if coord.iter().any(|x| !matches!(x, Interior(Singular(_)))) {
                    return None;
                }

                // Collect information about the incoming and outgoing strands.
                let mut inputs = 0;
                let mut input_depths = vec![];
                let mut input_rewrites = vec![];
                let mut input_coords = vec![];
                let mut outputs = 0;
                let mut output_depths = vec![];
                let mut output_rewrites = vec![];
                let mut output_coords = vec![];
                for e in graph.edges_directed(n, EdgeDirection::Incoming) {
                    let rewrite: &Rewrite = &e.weight().rewrite;
                    let source_coord = graph[e.source()].key;
                    let depth = depths.edge_depth(source_coord, coord);

                    match e.weight().key {
                        Direction::Forward => {
                            inputs += 1;
                            input_depths.push(depth);
                            input_rewrites.push(rewrite);
                            input_coords.push(source_coord);
                        }
                        Direction::Backward => {
                            outputs += 1;
                            output_depths.push(depth);
                            output_rewrites.push(rewrite);
                            output_coords.push(source_coord);
                        }
                    }
                }

                if inputs == 0 || outputs == 0 {
                    Some(Homotopy::Duality)
                } else if inputs == 1 && outputs == 1 {
                    if input_rewrites[0].is_identity() && output_rewrites[0].is_identity() {
                        Some(Homotopy::Identity)
                    } else {
                        Some(Homotopy::Complex)
                    }
                } else {
                    // Find the depth of the front layer.
                    let &min_depth = input_depths.iter().chain(&output_depths).min().unwrap();

                    // Collect the incoming and outgoing strands of the front layer.
                    let front_inputs = input_depths
                        .iter()
                        .enumerate()
                        .filter_map(|(i, &depth)| (depth == min_depth).then_some(i))
                        .collect::<Vec<_>>();
                    let front_outputs = output_depths
                        .iter()
                        .enumerate()
                        .filter_map(|(i, &depth)| (depth == min_depth).then_some(i))
                        .collect::<Vec<_>>();

                    if front_inputs.len() != 1 || front_outputs.len() != 1 {
                        return Some(Homotopy::Complex);
                    }

                    // If the front layer has one incoming strand and one outgoing strand, it's either a half braid or a full braid.
                    // To determine which, we check if the incoming strand crosses over the outgoing strand using the layout.
                    let &i = front_inputs.first().unwrap();
                    let &j = front_outputs.first().unwrap();
                    let position_node = layout[&coord][0];
                    let position_input_wire = layout[&input_coords[i]][0];
                    let position_output_wire = layout[&output_coords[j]][0];

                    if matches!(
                        (
                            position_node.partial_cmp(&position_input_wire),
                            position_node.partial_cmp(&position_output_wire)
                        ),
                        (Some(Ordering::Less), Some(Ordering::Greater))
                            | (Some(Ordering::Greater), Some(Ordering::Less))
                    ) {
                        Some(Homotopy::FullBraid)
                    } else {
                        Some(Homotopy::HalfBraid)
                    }
                }
            }();

            generators.push(g);
            front_generators.push((front_g, is_identity));
            homotopies.push(h);
            coord_to_node.insert(coord, n);
        }

        Ok(Self {
            generators,
            front_generators,
            homotopies,
            coord_to_node,
        })
    }

    #[must_use]
    pub fn generator(&self, p: Coordinate<N>) -> Diagram0 {
        self.generators[self.coord_to_node[&p]]
    }

    #[must_use]
    pub fn front_generator(&self, p: Coordinate<N>) -> (Diagram0, bool) {
        self.front_generators[self.coord_to_node[&p]]
    }

    #[must_use]
    pub fn homotopy(&self, p: Coordinate<N>) -> Option<Homotopy> {
        self.homotopies[self.coord_to_node[&p]]
    }
}

/// Diagram analysis that finds the depth of cells in the 2-dimensional projection of a diagram.
#[derive(Debug, Clone)]
pub struct Depths<const N: usize> {
    graph: Scaffold<Coordinate<N>>,
    node_depths: IdxVec<NodeIndex, Option<usize>>,
    edge_depths: IdxVec<EdgeIndex, Option<usize>>,
    coord_to_node: FastHashMap<Coordinate<N>, NodeIndex>,
    /// The identity of sheets and where they end, made when first asked for (see [`sheets`]).
    sheet_data: OnceCell<SheetData<N>>,
}

impl<const N: usize> Depths<N> {
    pub fn new(diagram: &Diagram) -> Result<Self, DimensionError> {
        let mut graph = Scaffold::default();
        graph.add_node(ScaffoldNode::new(
            [Boundary::Source.into(); N],
            diagram.clone(),
        ));
        for i in 0..N {
            graph = graph.explode_graph(
                |_, key, si| {
                    let mut key = *key;
                    key[i] = si;
                    Some(key)
                },
                |_, _, _| Some(()),
                |_, (), r| r.is_atomic().then_some(()),
            )?;
        }

        let mut node_depths = IdxVec::splat(None, graph.node_count());
        let mut edge_depths = IdxVec::splat(None, graph.edge_count());

        let coord_to_node = graph
            .node_references()
            .map(|(n, node)| (node.key, n))
            .collect();

        for node in Topo::new(&graph).iter(&graph) {
            for edge in graph.edges_directed(node, EdgeDirection::Incoming) {
                if let Rewrite::RewriteN(r) = &edge.weight().rewrite {
                    edge_depths[edge.id()] =
                        node_depths[edge.source()].map(|d| r.singular_image(d));

                    let target_depth = r.targets().next();
                    node_depths[node] = min_defined(
                        min_defined(node_depths[node], edge_depths[edge.id()]),
                        target_depth,
                    );
                }
            }
        }

        Ok(Self {
            graph,
            node_depths,
            edge_depths,
            coord_to_node,
            sheet_data: OnceCell::new(),
        })
    }

    fn sheet_data(&self) -> &SheetData<N> {
        self.sheet_data.get_or_init(|| SheetData::build(self))
    }

    #[must_use]
    pub fn node_depth(&self, coord: Coordinate<N>) -> Option<usize> {
        let &n = self.coord_to_node.get(&coord)?;
        self.node_depths[n]
    }

    #[must_use]
    pub fn edge_depth(&self, from: Coordinate<N>, to: Coordinate<N>) -> Option<usize> {
        let &from = self.coord_to_node.get(&from)?;
        let &to = self.coord_to_node.get(&to)?;
        let e = self
            .graph
            .edges_directed(from, EdgeDirection::Outgoing)
            .find(|&e| e.target() == to)?;
        self.edge_depths[e.id()]
    }

    /// The diagram that is carried by the position `coord` of the projection: what is left of the
    /// diagram when its first `N` coordinates are fixed (a diagram of `N` dimensions fewer). The
    /// projection shows one point of it, at the front ([`node_depth`](Self::node_depth)), and its
    /// other points are hidden behind that one: they are the sheets (see
    /// [`sheets`](Self::sheets)) of the position.
    #[must_use]
    pub fn fibre(&self, coord: Coordinate<N>) -> Option<&Diagram> {
        let &n = self.coord_to_node.get(&coord)?;
        Some(&self.graph[n].diagram)
    }

    /// The sheets at the position `coord`: the singular heights `z` of its [fibre](Self::fibre)
    /// (the cospans of the first coordinate after the `N` that the projection is of), in order,
    /// and for each the cell that is there. The sheet that the projection draws is the one with
    /// [`Sheet::front`] set, if any (the front-most: see [`node_depth`](Self::node_depth)).
    ///
    /// The sheets nest: the fibre of a sheet `z` (its slice at the singular height `z`) has
    /// sheets of its own, for the next coordinate. They are those of `within = [z]`, and so on
    /// deeper; `within` is empty for the sheets of the position itself. Empty if there is no such
    /// position or sheet, or no more coordinates. Cheap: it slices the fibre along `within` and
    /// looks at one slice per height.
    #[must_use]
    pub fn sheets(&self, coord: Coordinate<N>, within: &[usize]) -> Vec<Sheet> {
        let Some(Diagram::DiagramN(fibre)) = self.fibre(coord) else {
            return vec![];
        };
        let mut fibre = fibre.clone();
        for &z in within {
            match fibre.slice(Height::Singular(z)) {
                Some(Diagram::DiagramN(slice)) => fibre = slice,
                _ => return vec![],
            }
        }
        let top = within.is_empty();
        let front = if top { self.node_depth(coord) } else { None };
        // The identity of sheets is for the first coordinate after the position's.
        let node = top
            .then(|| self.coord_to_node.get(&coord).copied())
            .flatten();
        let data = node.map(|_| self.sheet_data());
        let merges = |z: usize| node.zip(data).map_or(0, |(n, data)| data.merged(n, z));
        fibre
            .singular_slices()
            .enumerate()
            .map(|(z, slice)| {
                let id = node.zip(data).and_then(|(n, data)| data.id(n, z));
                Sheet {
                    z,
                    generator: slice.max_generator(),
                    front: front == Some(z),
                    id,
                    foremost: id.zip(data).is_some_and(|(id, data)| data.foremost(id)),
                    merges: merges(z),
                }
            })
            .collect()
    }

    /// The number of sheets there are in the diagram: the number of distinct [`SheetId`]s, which
    /// are `0..sheet_count()`. A sheet that comes back to the same position (a U-turn in the
    /// direction that the projection is along) is one sheet, at two heights.
    #[must_use]
    pub fn sheet_count(&self) -> usize {
        self.sheet_data().count()
    }

    /// The places where distinct sheets meet: the cones of the rewrites along the edges of the
    /// scaffold that take two or more heights that are different sheets to one (see
    /// [`SheetBoundary`]). Empty for a diagram that has only the dimensions of the projection, and
    /// for one with a single sheet.
    #[must_use]
    pub fn sheet_boundaries(&self) -> &[SheetBoundary<N>] {
        self.sheet_data().boundaries()
    }

    /// The meetings of sheets at the position `coord`: its heights that several distinct sheets
    /// of its neighbours are merged into.
    #[must_use]
    pub fn sheet_boundaries_at(&self, coord: Coordinate<N>) -> Vec<&SheetBoundary<N>> {
        match self.coord_to_node.get(&coord) {
            Some(&n) => self.sheet_data().boundaries_to(n).collect(),
            None => vec![],
        }
    }

    /// The sheets at the position `coord` by their [`SheetId`], in the order of their heights
    /// (the same as the first coordinate after the position's in [`sheets`](Self::sheets), but
    /// without making a [`Sheet`] of each).
    #[must_use]
    pub fn sheet_ids_at(&self, coord: Coordinate<N>) -> &[SheetId] {
        self.coord_to_node
            .get(&coord)
            .map_or(&[], |&n| self.sheet_data().ids_at(n))
    }

    /// Everywhere the sheet `id` is: each position that it passes through, with its height there
    /// (twice, if it comes back to a position). Made from the lists that are kept for the
    /// sheets, so it costs what it returns.
    #[must_use]
    pub fn sheet_places(&self, id: SheetId) -> Vec<(Coordinate<N>, usize)> {
        self.sheet_data()
            .places(id)
            .iter()
            .map(|&(n, z)| (self.graph[n].key, z))
            .collect()
    }

    /// How many distinct sheets meet at the position `coord`, if some do: the most that are merged
    /// into one of its heights, two or more (see [`SheetBoundary`]). 0 if no sheets meet there.
    #[must_use]
    pub fn sheets_meeting_at(&self, coord: Coordinate<N>) -> usize {
        self.sheet_boundaries_at(coord)
            .iter()
            .map(|b| b.branches)
            .max()
            .unwrap_or(0)
    }

    /// How many echoes to draw along a wire, which is a segment of the picture between a point
    /// where sheets meet (see [`sheets_meeting_at`](Self::sheets_meeting_at)) and the position
    /// beside it: 0 for a wire that is not at such a point, or that is on no sheet, or on the
    /// front-most of the sheets that the wires at the point are on, and else one for each step
    /// that it is behind, at most [`MAX_ECHOES`].
    ///
    /// The sheet that a wire is on is the one that is drawn at the position beside the point
    /// ([`node_depth`](Self::node_depth): the height `z` that hovering over the wire marks as
    /// drawn). A step is a different `z` among the wires at the point: the wires of the first
    /// have none, those of the next have one, and so on, so that the echoes say which wires go
    /// into which layer.
    #[must_use]
    pub fn echoes(&self, wire: [Coordinate<N>; 2]) -> usize {
        let [first, second] = wire;
        let (Some(&first), Some(&second)) = (
            self.coord_to_node.get(&first),
            self.coord_to_node.get(&second),
        ) else {
            return 0;
        };
        let meets = |n: NodeIndex| self.sheet_data().boundaries_to(n).next().is_some();
        let vertex = if meets(second) {
            second
        } else if meets(first) {
            first
        } else {
            return 0;
        };
        let beside = if vertex == second { first } else { second };
        let Some(layer) = self.node_depths[beside] else {
            return 0;
        };
        let mut layers: Vec<usize> = self
            .graph
            .edges_directed(vertex, EdgeDirection::Incoming)
            .filter_map(|e| self.node_depths[e.source()])
            .collect();
        layers.sort_unstable();
        layers.dedup();
        let behind = layers.iter().take_while(|&&other| other < layer).count();
        behind.min(MAX_ECHOES)
    }

    /// How many sheets pass through the position `coord`, counted by their heights (a sheet
    /// that passes twice counts twice): the thickness of the diagram there. 0 if there is no
    /// such position or no more coordinates.
    #[must_use]
    pub fn thickness(&self, coord: Coordinate<N>) -> usize {
        match self.fibre(coord) {
            Some(Diagram::DiagramN(fibre)) => fibre.size(),
            _ => 0,
        }
    }

    /// The position of the projection that a triangle of a surface belongs to, and so whose sheets
    /// are the ones behind it: its first corner, which is the one that is a region (in the
    /// coordinates of the view, no singular height) for the triangles that a cube of the mesh is
    /// cut into. Hovering over the surface lists the sheets of this position.
    #[must_use]
    pub const fn surface_position(corners: &[Coordinate<N>; 3]) -> Coordinate<N> {
        corners[0]
    }

    /// How many sheets pass through a triangle of a surface: those of its
    /// [position](Self::surface_position).
    #[must_use]
    pub fn surface_thickness(&self, corners: &[Coordinate<N>; 3]) -> usize {
        self.thickness(Self::surface_position(corners))
    }

    #[must_use]
    pub fn edges_above(&self, depth: usize, to: Coordinate<N>) -> Vec<Coordinate<N>> {
        let to = match self.coord_to_node.get(&to) {
            Some(to) => *to,
            None => return vec![],
        };

        self.graph
            .edges_directed(to, EdgeDirection::Incoming)
            .filter_map(|e| match self.edge_depths[e.id()] {
                Some(d) if d < depth => self.graph.node_weight(e.source()).map(|node| node.key),
                _ => None,
            })
            .collect()
    }
}

/// A sheet at a position of the projection of a diagram: one singular height of the diagram
/// that is carried by the position (see [`Depths::sheets`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Sheet {
    /// The singular height, in the first coordinate that the projection is not of.
    pub z: usize,
    /// The cell of greatest dimension at that height.
    pub generator: Diagram0,
    /// Whether this is the sheet that the projection draws at the position, the front-most
    /// there ([`Depths::node_depth`]). Which sheet that is can change along a wire.
    pub front: bool,
    /// Which sheet this is, the same at every position it passes through (see [`SheetId`]).
    /// `None` for the sheets *inside* a sheet (`within` of [`Depths::sheets`] not empty): only
    /// the identity of the sheets of the first coordinate after the position's is followed.
    pub id: Option<SheetId>,
    /// How many distinct sheets of the neighbouring positions the edges into this position merge
    /// into this one, where sheets meet (see the `sheets` module): 2 or more at a point of the
    /// picture where they do, and 0 everywhere else, and for sheets inside a sheet.
    pub merges: usize,
    /// Whether the sheet is in front along its whole length, which is not what [`front`](Self::front)
    /// says: it is not hidden by another sheet, or is hidden at fewer of the positions that it
    /// shares with others than it is the front-most at (see the `sheets` module).
    pub foremost: bool,
}

fn min_defined<T>(a: Option<T>, b: Option<T>) -> Option<T>
where
    T: Ord,
{
    match (a, b) {
        (None, None) => None,
        (None, Some(b)) => Some(b),
        (Some(a), None) => Some(a),
        (Some(a), Some(b)) => Some(std::cmp::min(a, b)),
    }
}
