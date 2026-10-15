//! Shading surfaces by the sheets behind them agrees with the hover: a region that the hover
//! says has `k` sheets is shaded for `k`, and what is drawn is made of the surfaces of one step
//! each.

use std::collections::{BTreeSet, HashSet};

use homotopy_core::{
    complex::{make_complex, Simplex},
    layout::Layout,
    projection::{thickness_step, Depths, Projection, Shading},
    Diagram,
};
use homotopy_graphics::svg::render::GraphicElement;
use homotopy_model::{history::Proof, proof::Action, sheets::Stack};

const STEPS: usize = 4;

/// Checks the shading of the 2D view of a diagram against the sheets that the hover lists (the
/// `Stack` at the first corner of each surface, which is what hovering over it reads), and that
/// the elements drawn are the surfaces of one generator and step each.
fn check(name: &str, diagram: &Diagram, layout: &Layout<2>) -> usize {
    let depths = Depths::<2>::new(diagram).unwrap();
    let projection = Projection::new(diagram, layout, &depths).unwrap();
    let complex = make_complex::<2>(diagram);
    let surfaces: Vec<_> = complex
        .iter()
        .filter_map(|(simplex, visible)| match simplex {
            Simplex::Surface(corners) if *visible => Some(*corners),
            _ => None,
        })
        .collect();
    let shading = Shading::new(&depths, surfaces.iter().copied(), STEPS);
    // What the hover lists, from the stack that it makes for the surface under the pointer.
    let listed: Vec<usize> = surfaces
        .iter()
        .map(|corners| {
            Stack::new(&depths, &[corners[0]], None).map_or(0, |stack| stack.entries.len())
        })
        .collect();
    let (min, max) = (
        listed.iter().copied().min().unwrap_or(0),
        listed.iter().copied().max().unwrap_or(0),
    );
    // (The shading used to read the fewest of the three corners.)
    let misread = surfaces
        .iter()
        .zip(&listed)
        .filter(|(corners, &count)| {
            corners
                .iter()
                .map(|corner| depths.thickness(*corner))
                .min()
                .unwrap_or(0)
                != count
        })
        .count();
    let mut steps = BTreeSet::new();
    let mut expected = HashSet::new();
    for (corners, &count) in surfaces.iter().zip(&listed) {
        let step = shading.step(&depths, corners);
        assert_eq!(
            step,
            thickness_step(count, min, max, STEPS),
            "{name}: {corners:?}"
        );
        steps.insert((count, step));
        expected.insert((projection.generator(corners[0]), step));
    }
    // The elements: one per generator and step.
    let elements =
        GraphicElement::build_shaded(&complex, layout, &projection, &depths, |corners| {
            shading.step(&depths, corners)
        });
    let drawn: HashSet<_> = elements
        .iter()
        .filter_map(|(element, step)| match element {
            GraphicElement::Surface(generator, _) => Some((*generator, *step)),
            _ => None,
        })
        .collect();
    assert_eq!(drawn, expected, "{name}");
    eprintln!(
        "{name}: {} surfaces ({misread} that the corners misread), hover counts {min}..={max}, \
         (sheets, step) {steps:?}",
        surfaces.len()
    );
    surfaces.len()
}

#[test]
fn the_examples_are_shaded_for_the_sheets_the_hover_lists() {
    let mut checked = 0;
    for (name, diagram) in [
        ("associator", homotopy_core::examples::associator().1),
        ("crossing", homotopy_core::examples::crossing().1),
        ("half_braid", homotopy_core::examples::half_braid().1),
        (
            "algebraic_snake",
            homotopy_core::examples::algebraic_snake().1,
        ),
        ("lips", homotopy_core::examples::lips().1),
        ("pants_unit", homotopy_core::examples::pants_unit().1),
    ] {
        let diagram: Diagram = diagram.into();
        let layout = Layout::<2>::new(&diagram).unwrap();
        checked += check(name, &diagram, &layout);
    }
    assert!(checked > 100);
}

/// The tangent categories project (8.5MB, not in the repository): `start` in the 2D view. Run with
/// `HOMOTOPY_TANGENT_FIXTURE=<path to the .hom>`, in release.
#[test]
fn start_of_the_tangent_categories_project_is_shaded_for_the_sheets_the_hover_lists() {
    let Ok(path) = std::env::var("HOMOTOPY_TANGENT_FIXTURE") else {
        return;
    };
    let bytes = std::fs::read(path).unwrap();
    let mut state = Proof::default();
    state.update(&Action::ImportProof(bytes.into())).unwrap();
    let start = state
        .signature
        .iter()
        .find(|info| info.name == "start")
        .map(|info| info.generator)
        .unwrap();
    state.update(&Action::SelectGenerator(start)).unwrap();
    let diagram = state.workspace.as_ref().unwrap().diagram.clone();
    assert_eq!(diagram.dimension(), 5);
    let layout = Layout::<2>::new(&diagram).unwrap();
    let t = std::time::Instant::now();
    let surfaces = check("start", &diagram, &layout);
    eprintln!("start checked in {:?}", t.elapsed());
    assert!(surfaces > 20);
}
