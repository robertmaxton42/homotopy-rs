//! Where distinct sheets meet in the 2D view of the tangent categories project (13MB, not in the
//! repository): at a few vertices and not at every point of every wire, and nowhere in a diagram
//! with one sheet. Run with `HOMOTOPY_TANGENT_FIXTURE=<path to the .hom>`; without it the tests
//! do nothing.

use homotopy_core::{
    complex::{make_complex, Simplex},
    projection::Depths,
    Diagram, Height, SliceIndex,
};
use homotopy_model::{history::Proof, proof::Action};

fn fixture() -> Option<Proof> {
    let path = std::env::var("HOMOTOPY_TANGENT_FIXTURE").ok()?;
    let bytes = std::fs::read(path).unwrap();
    let mut state = Proof::default();
    state.update(&Action::ImportProof(bytes.into())).unwrap();
    Some(state)
}

fn vertex(first: usize, second: usize) -> [SliceIndex; 2] {
    [
        SliceIndex::Interior(Height::Singular(first)),
        SliceIndex::Interior(Height::Singular(second)),
    ]
}

/// The positions where sheets meet, in order, and the number of wires that have echoes.
fn meetings(diagram: &Diagram) -> (Vec<[SliceIndex; 2]>, usize) {
    let depths = Depths::<2>::new(diagram).unwrap();
    let mut places: Vec<_> = depths.sheet_boundaries().iter().map(|b| b.to).collect();
    places.dedup();
    let echoes = make_complex::<2>(diagram)
        .into_iter()
        .filter(|(simplex, _)| match simplex {
            Simplex::Wire(wire) => depths.echoes(*wire) > 0,
            _ => false,
        })
        .count();
    (places, echoes)
}

/// The 5-dimensional diagram in the workspace has one vertex that is drawn as a dot, with sheets
/// meeting at it, and many points of wires at the heights where it and other things happen, where
/// the sheets beside a wire end on it (the wire at the left of the diagram) or pass through, and
/// where nothing is to be marked.
#[test]
fn the_workspace_has_a_meeting_at_its_one_dot_only() {
    let Some(state) = fixture() else {
        return;
    };
    let diagram = &state.workspace.as_ref().unwrap().diagram;
    assert_eq!(diagram.dimension(), 5);
    let (places, _) = meetings(diagram);
    assert_eq!(places, vec![vertex(0, 0)]);
    assert!(
        points_of_the_picture(diagram) > 10,
        "many points have nothing marked"
    );
}

/// The target of the definition of `T(Tp)₍₂₎ → (T²p)₍₂₎`: a diagram of a project like the one a
/// person has open, with a green, a black and a purple dot and two blue ones, each of which is a
/// vertex where sheets meet, and wires that pass at their heights (the long white one at the left)
/// where none do.
#[test]
fn a_composite_has_meetings_at_its_dots_and_nowhere_else() {
    let Some(state) = fixture() else {
        return;
    };
    let info = state
        .signature
        .iter()
        .find(|info| info.name.starts_with("$T{(Tp)}_{(2)}") && info.name.ends_with(" def"))
        .unwrap();
    let Diagram::DiagramN(definition) = &info.diagram else {
        panic!("a cell");
    };
    let target = definition.target();
    assert_eq!(target.dimension(), 5);
    let (places, _) = meetings(&target);
    let dots = [(0, 2), (1, 1), (2, 0), (5, 0), (6, 1)].map(|(a, b)| vertex(a, b));
    assert_eq!(places, dots.to_vec());
    assert!(points_of_the_picture(&target) > places.len() + 10);
}

/// How many points of the picture there are: positions with every coordinate a singular height.
fn points_of_the_picture(diagram: &Diagram) -> usize {
    make_complex::<2>(diagram)
        .into_iter()
        .filter_map(|(simplex, _)| match simplex {
            Simplex::Point([point]) => Some(point),
            _ => None,
        })
        .filter(|point| {
            point
                .iter()
                .all(|index| matches!(index, SliceIndex::Interior(Height::Singular(_))))
        })
        .count()
}

/// `p` and `ε` have one sheet (a cell of the third coordinate, with a line and a vertex in it),
/// so nothing is marked, and nor is a wire echoed.
#[test]
fn p_and_epsilon_have_no_meetings_and_no_echoes() {
    let Some(state) = fixture() else {
        return;
    };
    let mut seen = 0;
    for info in state.signature.iter() {
        if info.name != "$p$" && info.name != r"$\varepsilon$" {
            continue;
        }
        let (places, echoes) = meetings(&info.diagram);
        assert!(places.is_empty(), "{}: {places:?}", info.name);
        assert_eq!(echoes, 0, "{}", info.name);
        seen += 1;
    }
    assert_eq!(seen, 2);
}

/// A generator is one cell, which is at one point of the picture, so the sheets that meet are
/// at most there; and wires that are echoed are all at it.
#[test]
fn a_generator_has_meetings_at_its_vertex_at_most() {
    let Some(state) = fixture() else {
        return;
    };
    let mut with = 0;
    for info in state.signature.iter() {
        if info.diagram.dimension() < 2 {
            continue;
        }
        let (places, _) = meetings(&info.diagram);
        assert!(
            places.iter().all(|place| *place == vertex(0, 0)),
            "{}",
            info.name
        );
        with += usize::from(!places.is_empty());
    }
    assert!(with > 10, "{with} generators have a meeting");
}
