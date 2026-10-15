//! The sheets at a position of the 2-dimensional projection of a diagram (`Depths::sheets`):
//! the singular heights of what the position carries, checked against slicing the diagram
//! directly.

use homotopy_core::{
    examples, projection::Depths, Boundary, Diagram, DiagramN, Height, SliceIndex,
};

/// All the coordinates of the first `n` dimensions of a diagram, with what is left of it there.
fn positions(diagram: &Diagram, n: usize) -> Vec<(Vec<SliceIndex>, Diagram)> {
    if n == 0 {
        return vec![(vec![], diagram.clone())];
    }
    let Diagram::DiagramN(d) = diagram else {
        return vec![];
    };
    let mut all = vec![];
    let size = d.size();
    let indices = [SliceIndex::Boundary(Boundary::Source)]
        .into_iter()
        .chain((0..2 * size + 1).map(|i| SliceIndex::Interior(Height::from(i))))
        .chain([SliceIndex::Boundary(Boundary::Target)]);
    for index in indices {
        let slice = d.slice(index).unwrap();
        for (mut rest, fibre) in positions(&slice, n - 1) {
            rest.insert(0, index);
            all.push((rest, fibre));
        }
    }
    all
}

fn check<const N: usize>(diagram: &DiagramN) -> (usize, usize) {
    let diagram: Diagram = diagram.clone().into();
    let depths = Depths::<N>::new(&diagram).unwrap();
    let (mut positions_with_sheets, mut with_several) = (0, 0);
    for (coord, fibre) in positions(&diagram, N) {
        let coord: [SliceIndex; N] = coord.try_into().unwrap();
        assert_eq!(depths.fibre(coord), Some(&fibre), "{coord:?}");
        let sheets = depths.sheets(coord, &[]);
        let expected = match &fibre {
            Diagram::Diagram0(_) => vec![],
            Diagram::DiagramN(f) => f
                .singular_slices()
                .map(|slice| slice.max_generator())
                .collect(),
        };
        assert_eq!(
            sheets.iter().map(|s| s.generator).collect::<Vec<_>>(),
            expected,
            "{coord:?}"
        );
        for (i, sheet) in sheets.iter().enumerate() {
            assert_eq!(sheet.z, i);
        }
        // The front sheet is the one that the depth analysis picks, if there is one.
        let fronts: Vec<_> = sheets.iter().filter(|s| s.front).collect();
        match depths.node_depth(coord) {
            Some(z) if z < sheets.len() => assert_eq!(fronts.len(), 1, "{coord:?}"),
            _ => assert!(fronts.is_empty(), "{coord:?}"),
        }
        if let Some(front) = fronts.first() {
            assert_eq!(Some(front.z), depths.node_depth(coord));
        }
        // Nested: the sheets of a sheet are those of its slice.
        for sheet in &sheets {
            let Diagram::DiagramN(f) = &fibre else {
                unreachable!()
            };
            let inner = f.slice(Height::Singular(sheet.z)).unwrap();
            let nested = depths.sheets(coord, &[sheet.z]);
            match inner {
                Diagram::Diagram0(_) => assert!(nested.is_empty()),
                Diagram::DiagramN(inner) => {
                    assert_eq!(nested.len(), inner.size(), "{coord:?} within {}", sheet.z);
                    assert!(nested.iter().all(|s| !s.front));
                }
            }
        }
        // Nothing there: no position, or sheets beyond the diagram.
        assert!(depths.sheets(coord, &[usize::MAX / 4]).is_empty());
        positions_with_sheets += usize::from(!sheets.is_empty());
        with_several += usize::from(sheets.len() > 1);
    }
    (positions_with_sheets, with_several)
}

#[test]
fn sheets_are_the_singular_heights_of_the_fibre() {
    let mut several = 0;
    let mut checked = 0;
    for (_, diagram) in [
        examples::associator(),
        examples::crossing(),
        examples::half_braid(),
        examples::bubble(),
        examples::algebraic_snake(),
        examples::lips(),
        examples::pants_unit(),
        examples::two_cups(),
    ] {
        let dimension = diagram.dimension();
        if dimension >= 2 {
            let (with, more) = check::<1>(&diagram);
            checked += with;
            several += more;
        }
        if dimension >= 3 {
            let (with, more) = check::<2>(&diagram);
            checked += with;
            several += more;
        }
    }
    assert!(checked > 0, "some positions have sheets");
    assert!(several > 0, "some positions have several sheets");
}

// ---------------------------------------------------------------------------------------------
// The identity of sheets, and where they end.

use homotopy_core::{
    projection::thickness_steps,
    scaffold::{Explodable, Scaffold, ScaffoldNode},
    Rewrite,
};
use petgraph::visit::EdgeRef;

/// The 3D examples (and the 4D ones, which have more to follow), seen in 2D.
fn examples_in_2d() -> Vec<(&'static str, DiagramN)> {
    vec![
        ("associator", examples::associator().1),
        ("crossing", examples::crossing().1),
        ("touching", examples::touching().1),
        ("half_braid", examples::half_braid().1),
        ("algebraic_snake", examples::algebraic_snake().1),
        ("lips", examples::lips().1),
        ("pants_unit", examples::pants_unit().1),
    ]
}

/// The scaffold with every edge, as `Depths` makes it but without leaving out the regular slices
/// of the later coordinates: each edge says whether `Depths` keeps it.
fn full_scaffold<const N: usize>(diagram: &Diagram) -> Scaffold<[SliceIndex; N], bool> {
    let mut graph = Scaffold::default();
    graph.add_node(ScaffoldNode::new(
        [Boundary::Source.into(); N],
        diagram.clone(),
    ));
    for i in 0..N {
        graph = graph
            .explode_graph(
                |_, key, si| {
                    let mut key = *key;
                    key[i] = si;
                    Some(key)
                },
                |_, _, _| Some(true),
                |_, kept, r| Some(*kept && r.is_atomic()),
            )
            .unwrap();
    }
    graph
}

/// The heights of the source of a rewrite and the heights of the target they go to.
fn images(size: usize, rewrite: &Rewrite) -> Vec<(usize, usize)> {
    let Rewrite::RewriteN(r) = rewrite else {
        return vec![];
    };
    (0..size).map(|z| (z, r.singular_image(z))).collect()
}

fn sheet_ids<const N: usize>(depths: &Depths<N>, position: [SliceIndex; N]) -> Vec<usize> {
    depths
        .sheets(position, &[])
        .iter()
        .map(|s| s.id.expect("every sheet of a position has an id").0 as usize)
        .collect()
}

/// Along every edge of the scaffold that `Depths` keeps, a height goes to the height of the same
/// sheet if the cell is the same there, and to another if it is not. (Along those it leaves out,
/// the diagonals, it does too unless the way round through the edges that are kept passes a
/// height where the cell changes, which cuts the sheet: so they are not checked here, only for
/// their cones, below.)
fn check_identity<const N: usize>(name: &str, diagram: &DiagramN) -> usize {
    let diagram: Diagram = diagram.clone().into();
    let depths = Depths::<N>::new(&diagram).unwrap();
    let graph = full_scaffold::<N>(&diagram);
    let mut edges_checked = 0;
    for edge in graph.edge_references() {
        let (from, to) = (&graph[edge.source()], &graph[edge.target()]);
        let (Diagram::DiagramN(_), Diagram::DiagramN(target)) = (&from.diagram, &to.diagram) else {
            continue;
        };
        if !edge.weight().key {
            continue;
        }
        let size = match &from.diagram {
            Diagram::DiagramN(f) => f.size(),
            Diagram::Diagram0(_) => unreachable!(),
        };
        let (ids_from, ids_to) = (sheet_ids(&depths, from.key), sheet_ids(&depths, to.key));
        assert_eq!(ids_to.len(), target.size(), "{name} {:?}", to.key);
        let (sheets_from, sheets_to) = (depths.sheets(from.key, &[]), depths.sheets(to.key, &[]));
        for (z, image) in images(size, &edge.weight().rewrite) {
            edges_checked += 1;
            let same_cell =
                sheets_from[z].generator.generator == sheets_to[image].generator.generator;
            // The same cell is the same sheet; a different one never is.
            assert_eq!(
                ids_from[z] == ids_to[image],
                same_cell,
                "{name}: height {z} of {:?} and height {image} of {:?}",
                from.key,
                to.key,
            );
        }
    }
    edges_checked
}

#[test]
fn sheets_have_the_same_identity_along_every_edge() {
    let mut checked = 0;
    for (name, diagram) in examples_in_2d() {
        checked += check_identity::<2>(name, &diagram);
        checked += check_identity::<1>(name, &diagram);
    }
    for (name, diagram) in [
        ("monoid_unit", examples::monoid_unit().1),
        ("matchsticks", examples::matchsticks().1),
        ("stacks", examples::stacks().1),
        ("bubble", examples::bubble().1),
        ("two_cups", examples::two_cups().1),
    ] {
        checked += check_identity::<1>(name, &diagram);
    }
    assert!(checked > 100, "{checked} heights checked");
}

#[test]
fn every_sheet_has_an_id_and_the_sheets_inside_one_have_none() {
    fn check<const N: usize>(diagram: &DiagramN) -> usize {
        let diagram: Diagram = diagram.clone().into();
        let depths = Depths::<N>::new(&diagram).unwrap();
        let mut seen = vec![false; depths.sheet_count()];
        for (coord, _) in positions(&diagram, N) {
            let coord: [SliceIndex; N] = coord.try_into().unwrap();
            let sheets = depths.sheets(coord, &[]);
            assert_eq!(depths.thickness(coord), sheets.len());
            for sheet in &sheets {
                let id = sheet.id.expect("a sheet has an id");
                assert!((id.0 as usize) < depths.sheet_count());
                seen[id.0 as usize] = true;
                assert_eq!(id.label(), format!("S{}", id.0 + 1));
            }
            for z in 0..sheets.len() {
                assert!(depths
                    .sheets(coord, &[z])
                    .iter()
                    .all(|s| s.id.is_none() && !s.foremost));
            }
        }
        // Every id is of a sheet somewhere.
        assert!(seen.iter().all(|&s| s));
        depths.sheet_count()
    }
    for (name, diagram) in examples_in_2d() {
        let count = check::<2>(&diagram);
        assert!(count >= 1, "{name}");
        check::<1>(&diagram);
    }
}

#[test]
fn the_number_of_sheets_is_what_the_examples_have() {
    let count = |diagram: &DiagramN| {
        let diagram: Diagram = diagram.clone().into();
        Depths::<2>::new(&diagram).unwrap().sheet_count()
    };
    // Two strands that cross, or touch, or half cross, are two sheets, however long they are.
    assert_eq!(count(&examples::crossing().1), 2);
    assert_eq!(count(&examples::touching().1), 2);
    assert_eq!(count(&examples::half_braid().1), 2);
    // A diagram of the dimensions of the view has none.
    let flat: Diagram = examples::two_monoid().1.into();
    let depths = Depths::<2>::new(&flat).unwrap();
    assert_eq!(depths.sheet_count(), 0);
    assert!(depths.sheet_boundaries().is_empty());
    // More sheets where more is going on than there are positions with several.
    assert!(count(&examples::associator().1) > 2);
}

/// Two strands at different depths that overlap on only a part of their length (a crossing): the
/// one that is in front where they overlap is in front along the whole of its length, and the
/// other is behind along the whole of its, though it is the front-most at the positions where it
/// is alone, which is what the depth of a position says.
#[test]
fn of_two_strands_that_overlap_in_part_one_is_in_front_throughout() {
    for diagram in [examples::crossing().1, examples::half_braid().1] {
        let diagram: Diagram = diagram.into();
        let depths = Depths::<2>::new(&diagram).unwrap();
        let mut foremost = std::collections::BTreeMap::new();
        let (mut together, mut alone_but_not_foremost) = (0, 0);
        for (coord, _) in positions(&diagram, 2) {
            let coord: [SliceIndex; 2] = coord.try_into().unwrap();
            let sheets = depths.sheets(coord, &[]);
            for sheet in &sheets {
                let id = sheet.id.unwrap();
                // One answer for the whole sheet.
                assert_eq!(
                    *foremost.entry(id).or_insert(sheet.foremost),
                    sheet.foremost
                );
            }
            if sheets.len() > 1 {
                together += 1;
                // Where they overlap, the one that is drawn is the foremost.
                let drawn: Vec<_> = sheets.iter().filter(|s| s.front).collect();
                assert_eq!(drawn.len(), 1);
                assert!(drawn[0].foremost, "{coord:?}");
            } else if let [only] = sheets.as_slice() {
                // Alone, it is what is drawn, whether or not it is the sheet that is in front.
                assert!(only.front);
                alone_but_not_foremost += usize::from(!only.foremost);
            }
        }
        assert!(together > 0, "the strands overlap somewhere");
        assert_eq!(foremost.len(), 2);
        assert_eq!(foremost.values().filter(|&&f| f).count(), 1, "{foremost:?}");
        // The old notion flips along the sheet that is behind; the new one does not.
        assert!(alone_but_not_foremost > 0);
    }
}

/// A meeting is at a point of the picture, at a height where two or more distinct sheets of a
/// neighbour that carries them in are merged, and the sheets it lists are the ones that are.
#[test]
fn meetings_are_of_distinct_sheets_at_the_points_of_the_picture() {
    fn check<const N: usize>(diagram: &DiagramN) -> usize {
        let diagram: Diagram = diagram.clone().into();
        let depths = Depths::<N>::new(&diagram).unwrap();
        let mut meetings = 0;
        for b in depths.sheet_boundaries() {
            let at = depths.sheets(b.to, &[]);
            assert_eq!(at[b.z].id, Some(b.sheet));
            assert_eq!(at[b.z].merges, b.branches);
            assert!(depths.sheet_boundaries_at(b.to).contains(&b));
            assert_eq!(depths.sheets_meeting_at(b.to), b.branches);
            assert_ne!(b.from, b.to);
            // At a point: every coordinate is a singular height.
            assert!(b
                .to
                .iter()
                .all(|index| matches!(index, SliceIndex::Interior(Height::Singular(_)))));
            // Several distinct sheets are merged, and the neighbour has at least two heights.
            assert!(b.branches >= 2 && b.branches <= depths.sheet_count());
            assert!(sheet_ids(&depths, b.from).len() >= 2);
            meetings += 1;
        }
        // Nothing is made of positions that are not there.
        assert!(depths
            .sheet_boundaries_at([SliceIndex::Boundary(Boundary::Source); N])
            .iter()
            .all(|b| b.to == [SliceIndex::Boundary(Boundary::Source); N]));
        meetings
    }
    let mut meetings = 0;
    for diagram in [
        examples::matchsticks().1,
        examples::stacks().1,
        examples::monoid_unit().1,
        examples::two_monoid().1,
        examples::bubble().1,
    ] {
        meetings += check::<1>(&diagram);
    }
    for (_, diagram) in examples_in_2d() {
        meetings += check::<2>(&diagram);
    }
    assert!(meetings > 0, "some sheets meet");
    // Strands that only cross do not meet: the vertex where they cross has a height for each, and
    // a point of a wire where nothing happens has the heights of its neighbours one to one.
    for diagram in [
        examples::crossing().1,
        examples::touching().1,
        examples::half_braid().1,
    ] {
        assert_eq!(check::<2>(&diagram), 0);
    }
}

/// The regular slices that `Depths` leaves out of the scaffold have the cones of the edges that
/// it keeps: where one has a height with nothing to come from, so does one that is kept.
#[test]
fn leaving_out_regular_slices_loses_no_unit_cone() {
    fn check<const N: usize>(diagram: &DiagramN) -> usize {
        let diagram: Diagram = diagram.clone().into();
        let graph = full_scaffold::<N>(&diagram);
        // The heights that nothing is carried to, by an edge that is kept.
        let mut unit_kept = std::collections::HashSet::new();
        let mut unit_dropped = vec![];
        for edge in graph.edge_references() {
            let Rewrite::RewriteN(r) = &edge.weight().rewrite else {
                continue;
            };
            for cone in r.cone_views() {
                if cone.source_len() == 0 {
                    let id = (edge.target(), cone.target());
                    if edge.weight().key {
                        unit_kept.insert(id);
                    } else {
                        unit_dropped.push(id);
                    }
                }
            }
        }
        for id in &unit_dropped {
            assert!(
                unit_kept.contains(id),
                "a unit cone at height {} of {:?} is only on edges that are left out",
                id.1,
                graph[id.0].key
            );
        }
        unit_dropped.len()
    }
    let mut dropped = 0;
    for (_, diagram) in examples_in_2d() {
        dropped += check::<2>(&diagram);
        dropped += check::<1>(&diagram);
    }
    assert!(dropped > 0, "some edges that are left out have unit cones");
}

#[test]
fn thickness_is_stepped_from_the_thinnest() {
    assert!(thickness_steps(&[], 4).is_empty());
    // All as thick as each other: nothing is shaded.
    assert_eq!(thickness_steps(&[3, 3, 3], 4), vec![0, 0, 0]);
    // Within the steps: the difference from the thinnest.
    assert_eq!(thickness_steps(&[1, 2, 4, 1], 4), vec![0, 1, 3, 0]);
    assert_eq!(thickness_steps(&[2, 3], 4), vec![0, 1]);
    // Wider than the steps: scaled so that the thickest is the last, and anything thicker than
    // the thinnest is at least the first.
    let wide: Vec<usize> = (0..=40).collect();
    let steps = thickness_steps(&wide, 4);
    assert_eq!((steps[0], steps[40]), (0, 3));
    assert!(steps[1..].iter().all(|&s| s >= 1));
    assert!(steps.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(thickness_steps(&[1, 100], 4), vec![0, 3]);
    assert_eq!(thickness_steps(&[1, 2, 100], 4), vec![0, 1, 3]);
    // One step is no shading at all.
    assert_eq!(thickness_steps(&[1, 5, 9], 1), vec![0, 0, 0]);
}

// ---------------------------------------------------------------------------------------------
// Shading agrees with the sheets that the hover lists.

use homotopy_core::{
    complex::{make_complex, Simplex},
    projection::{thickness_step, Shading},
};

/// The visible surfaces of a diagram in 2D, with the number of sheets that the hover lists for
/// each (`Depths::sheets` at the position of the surface), and the step that it is shaded.
fn shaded_surfaces(diagram: &DiagramN) -> (Vec<(usize, usize, usize)>, usize) {
    let diagram: Diagram = diagram.clone().into();
    let depths = Depths::<2>::new(&diagram).unwrap();
    let surfaces: Vec<_> = make_complex::<2>(&diagram)
        .into_iter()
        .filter_map(|(simplex, visible)| match simplex {
            Simplex::Surface(corners) if visible => Some(corners),
            _ => None,
        })
        .collect();
    let shading = Shading::new(&depths, surfaces.iter().copied(), 4);
    // What the hover says, worked out without `Shading` (and without `surface_thickness`).
    let listed: Vec<usize> = surfaces
        .iter()
        .map(|corners| depths.sheets(corners[0], &[]).len())
        .collect();
    let (min, max) = (
        listed.iter().copied().min().unwrap_or(0),
        listed.iter().copied().max().unwrap_or(0),
    );
    let mut old_min_of_corners_differs = 0;
    let rows = surfaces
        .iter()
        .zip(&listed)
        .map(|(corners, &count)| {
            // The surface is a region: no singular height in the two coordinates of the view.
            assert!(
                corners[0]
                    .iter()
                    .all(|si| !matches!(si, SliceIndex::Interior(Height::Singular(_)))),
                "the first corner of {corners:?} is a region"
            );
            let step = shading.step(&depths, corners);
            assert_eq!(step, thickness_step(count, min, max, 4), "{corners:?}");
            let fewest = corners
                .iter()
                .map(|corner| depths.thickness(*corner))
                .min()
                .unwrap();
            old_min_of_corners_differs += usize::from(fewest != count);
            (count, step, fewest)
        })
        .collect();
    (rows, old_min_of_corners_differs)
}

#[test]
fn a_surface_is_shaded_for_the_sheets_that_the_hover_lists_at_its_region() {
    let mut surfaces = 0;
    let mut shaded = 0;
    let mut min_of_corners_would_differ = 0;
    for (name, diagram) in examples_in_2d() {
        let (rows, differs) = shaded_surfaces(&diagram);
        surfaces += rows.len();
        shaded += rows.iter().filter(|(_, step, _)| *step > 0).count();
        min_of_corners_would_differ += differs;
        // Surfaces with more sheets are never shaded less than those with fewer.
        let mut by_count: Vec<(usize, usize)> = rows.iter().map(|r| (r.0, r.1)).collect();
        by_count.sort_unstable();
        by_count.dedup();
        for pair in by_count.windows(2) {
            assert!(pair[0].1 <= pair[1].1, "{name}: {pair:?}");
            // and the same count is always the same step
            assert!(pair[0].0 != pair[1].0, "{name}: {pair:?}");
        }
    }
    assert!(surfaces > 100, "{surfaces} surfaces");
    assert!(shaded > 0, "some surfaces are shaded");
    // (The old rule read the fewest of the three corners, and went wrong on these.)
    assert!(
        min_of_corners_would_differ > 0,
        "some surface has a corner with fewer sheets than its region"
    );
}

// ---------------------------------------------------------------------------------------------
// Where sheets are, where they meet, and the echoes of the wires behind.

use homotopy_core::projection::{SheetId, MAX_ECHOES};

#[test]
fn the_places_of_a_sheet_are_the_positions_that_have_it() {
    fn check<const N: usize>(name: &str, diagram: &DiagramN) -> usize {
        let diagram: Diagram = diagram.clone().into();
        let depths = Depths::<N>::new(&diagram).unwrap();
        let mut heights = 0;
        let mut by_listing = vec![vec![]; depths.sheet_count()];
        for (coord, _) in positions(&diagram, N) {
            let coord: [SliceIndex; N] = coord.try_into().unwrap();
            let ids = depths.sheet_ids_at(coord);
            // The same sheets as `sheets` lists, in the same order.
            assert_eq!(ids, sheet_ids_as_ids(&depths, coord).as_slice(), "{name}");
            for (z, id) in ids.iter().enumerate() {
                by_listing[id.0 as usize].push((coord, z));
                heights += 1;
            }
        }
        let mut listed = 0;
        for (i, expected) in by_listing.iter().enumerate() {
            let mut places = depths.sheet_places(SheetId(i as u32));
            let mut expected = expected.clone();
            places.sort_unstable();
            expected.sort_unstable();
            assert_eq!(places, expected, "{name}: sheet {i}");
            listed += places.len();
        }
        assert_eq!(listed, heights, "{name}");
        // No such sheet.
        assert!(depths.sheet_places(SheetId(u32::MAX)).is_empty());
        heights
    }
    fn sheet_ids_as_ids<const N: usize>(
        depths: &Depths<N>,
        coord: [SliceIndex; N],
    ) -> Vec<SheetId> {
        depths
            .sheets(coord, &[])
            .iter()
            .map(|s| s.id.unwrap())
            .collect()
    }
    let mut checked = 0;
    for (name, diagram) in examples_in_2d() {
        checked += check::<2>(name, &diagram);
        checked += check::<1>(name, &diagram);
    }
    assert!(checked > 100, "{checked} heights");
}

/// The layer of a wire, worked out from `sheets`, the way a person hovering over it sees it: the
/// height that is marked as drawn at the position beside the point.
fn drawn_height<const N: usize>(depths: &Depths<N>, position: [SliceIndex; N]) -> Option<usize> {
    depths
        .sheets(position, &[])
        .iter()
        .find(|s| s.front)
        .map(|s| s.z)
}

/// Echoes are only on the wires at a point where sheets meet, and by how many layers behind the
/// front-most of the wires there they are; the wires of the front layer have none.
fn check_echoes(name: &str, diagram: &DiagramN) -> (usize, usize) {
    let diagram: Diagram = diagram.clone().into();
    let depths = Depths::<2>::new(&diagram).unwrap();
    let wires: Vec<_> = make_complex::<2>(&diagram)
        .into_iter()
        .filter_map(|(simplex, _)| match simplex {
            Simplex::Wire(wire) => Some(wire),
            _ => None,
        })
        .collect();
    let (mut with_echoes, mut meeting_points) = (0, 0);
    for wire in &wires {
        let echoes = depths.echoes(*wire);
        assert!(echoes <= MAX_ECHOES, "{name}");
        let (vertex, beside) = if depths.sheets_meeting_at(wire[1]) > 0 {
            (wire[1], wire[0])
        } else {
            (wire[0], wire[1])
        };
        if depths.sheets_meeting_at(vertex) == 0 {
            assert_eq!(echoes, 0, "{name}: {wire:?} is not at a meeting");
            continue;
        }
        // The layers that the wires at the point are drawn at, as the hover marks them.
        let mut layers: Vec<usize> = wires
            .iter()
            .filter(|other| other.contains(&vertex))
            .filter_map(|other| {
                let beside = if other[0] == vertex {
                    other[1]
                } else {
                    other[0]
                };
                drawn_height(&depths, beside)
            })
            .collect();
        layers.sort_unstable();
        layers.dedup();
        let expected = drawn_height(&depths, beside).map_or(0, |layer| {
            layers
                .iter()
                .filter(|&&other| other < layer)
                .count()
                .min(MAX_ECHOES)
        });
        assert_eq!(echoes, expected, "{name}: {wire:?}");
        if layers
            .first()
            .is_some_and(|&front| drawn_height(&depths, beside) == Some(front))
        {
            assert_eq!(echoes, 0, "{name}: a wire of the front layer has none");
        }
        with_echoes += usize::from(echoes > 0);
        meeting_points += 1;
    }
    (with_echoes, meeting_points)
}

#[test]
fn only_wires_behind_the_front_wires_at_a_meeting_have_echoes() {
    let (mut with_echoes, mut at_meetings) = (0, 0);
    for (name, diagram) in examples_in_2d() {
        let (echoes, wires) = check_echoes(name, &diagram);
        with_echoes += echoes;
        at_meetings += wires;
    }
    assert!(at_meetings > 10, "{at_meetings} wires at meetings");
    assert!(with_echoes > 0, "some wire is behind");
    assert!(with_echoes < at_meetings, "and some are not");
}

#[test]
fn the_associator_has_one_vertex_where_five_sheets_meet_and_one_wire_behind() {
    let diagram: Diagram = examples::associator().1.into();
    let depths = Depths::<2>::new(&diagram).unwrap();
    let vertex = [
        SliceIndex::Interior(Height::Singular(0)),
        SliceIndex::Interior(Height::Singular(0)),
    ];
    assert_eq!(depths.sheets_meeting_at(vertex), 5);
    let behind: Vec<_> = make_complex::<2>(&diagram)
        .into_iter()
        .filter_map(|(simplex, _)| match simplex {
            Simplex::Wire(wire) if depths.echoes(wire) > 0 => Some((wire, depths.echoes(wire))),
            _ => None,
        })
        .collect();
    assert_eq!(behind.len(), 1, "{behind:?}");
    assert_eq!(behind[0].1, 1);
    assert_eq!(behind[0].0[1], vertex);
    // Nowhere else do sheets meet.
    assert_eq!(depths.sheets_meeting_at([Boundary::Source.into(); 2]), 0);
}
