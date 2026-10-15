//! Definitions first: the definitions of the cell that a click lands on are among what the whole
//! search finds, and the cell after it in the signature is found.

use homotopy_core::{
    signature::{Invertibility, Signature as _},
    Diagram, DiagramN, Generator, Height, SliceIndex,
};
use homotopy_model::{
    history::Proof,
    job::run_to_end,
    proof::{cell_at, definitions_of, Action, AttachOptionsJob, DefinitionsJob, InferenceLimits},
};

const EXAMPLES: [&str; 5] = [
    include_str!("examples/associator.json"),
    include_str!("examples/braiding_half_bubbles.json"),
    include_str!("examples/equivalence_to_adjoint_equivalence.json"),
    include_str!("examples/merge_contract_RI.json"),
    include_str!("examples/symmetric_monoidal_abelianize.json"),
];

fn recorded_actions(json: &str) -> Vec<Action> {
    let (_safe, actions): (bool, Vec<Action>) = serde_json::from_str(json).unwrap();
    actions
}

/// Every point of `diagram` that is singular in its first `depth` coordinates.
fn vertices(diagram: &Diagram, depth: usize) -> Vec<Vec<SliceIndex>> {
    fn go(
        diagram: &Diagram,
        depth: usize,
        prefix: &mut Vec<SliceIndex>,
        out: &mut Vec<Vec<SliceIndex>>,
    ) {
        if depth == 0 {
            out.push(prefix.clone());
            return;
        }
        let Ok(d) = DiagramN::try_from(diagram.clone()) else {
            return;
        };
        for i in 0..d.size() {
            let height = Height::Singular(i);
            prefix.push(SliceIndex::Interior(height));
            if let Some(slice) = d.slice(height) {
                go(&slice, depth - 1, prefix, out);
            }
            prefix.pop();
        }
    }
    let mut out = vec![];
    go(diagram, depth, &mut vec![], &mut out);
    out
}

fn states(stride_of: usize) -> Vec<Proof> {
    let mut all = vec![];
    for json in EXAMPLES {
        let mut proof = Proof::default();
        let mut states = vec![];
        for action in recorded_actions(json) {
            if proof.update(&action).is_err() {
                continue;
            }
            if proof.workspace.is_some() {
                states.push(proof.clone());
            }
        }
        let stride = (states.len() / stride_of).max(1);
        all.extend(states.into_iter().step_by(stride));
    }
    all
}

/// A state of each example for every cell that has a definition in its signature, with the
/// workspace on that cell: and the generator and its definitions.
fn on_defined_cells() -> Vec<(Proof, Generator, Vec<Generator>)> {
    let mut out = vec![];
    for state in states(12) {
        for info in state.signature.iter() {
            let defs = definitions_of(&state.signature, info.generator);
            if defs.is_empty() {
                continue;
            }
            let mut on = state.clone();
            if on.update(&Action::SelectGenerator(info.generator)).is_ok() && on.workspace.is_some()
            {
                out.push((on, info.generator, defs));
            }
        }
    }
    out
}

fn full(
    state: &Proof,
    point: &[SliceIndex],
    weak_units: bool,
) -> Vec<homotopy_model::proof::AttachOption> {
    run_to_end(
        AttachOptionsJob::new(state, point, weak_units, InferenceLimits::UNLIMITED, None).unwrap(),
    )
    .options
}

fn heights(point: &[SliceIndex]) -> Vec<Height> {
    point
        .iter()
        .map(|index| match index {
            SliceIndex::Interior(height) => *height,
            SliceIndex::Boundary(_) => unreachable!(),
        })
        .collect()
}

/// The definitions that are in the signature already are found at the cell's own vertices, and
/// they are among what the whole search finds, also through weak units; `g + 1` first.
#[test]
fn the_definitions_of_a_cell_are_among_what_the_whole_search_finds() {
    let (mut clicks, mut offered, mut next_first) = (0, 0, 0);
    for (state, cell, definitions) in on_defined_cells() {
        // The cell after it comes first among the candidates, if it is one.
        if definitions.contains(&Generator::new(cell.id + 1, cell.dimension + 1)) {
            assert_eq!(
                definitions[0],
                Generator::new(cell.id + 1, cell.dimension + 1)
            );
        }
        let diagram = state.workspace.as_ref().unwrap().diagram.clone();
        for depth in 1..=diagram.dimension().min(3) {
            for point in vertices(&diagram, depth) {
                // The workspace is the cell: the click lands on it (the diagram is one cell).
                assert_eq!(cell_at(&diagram, &heights(&point)), Some(cell), "{point:?}");
                for weak_units in [false, true] {
                    clicks += 1;
                    let job =
                        DefinitionsJob::new(&state, &point, weak_units, InferenceLimits::UNLIMITED)
                            .expect("the cell has definitions");
                    assert_eq!(job.of(), cell);
                    let outcome = run_to_end(job);
                    assert!(outcome.cut_short.is_empty());
                    let whole = full(&state, &point, weak_units);
                    for option in &outcome.options {
                        assert!(whole.contains(option), "{option:?} at {point:?}");
                        assert!(definitions.contains(&option.generator));
                    }
                    offered += usize::from(!outcome.options.is_empty());
                    next_first += usize::from(
                        outcome
                            .options
                            .first()
                            .is_some_and(|option| option.generator.id == cell.id + 1),
                    );
                }
            }
        }
    }
    assert!(clicks > 40, "{clicks}");
    assert!(offered > 20, "definitions are found: {offered}");
    assert!(
        next_first > 20,
        "the generator after the cell is one: {next_first}"
    );
}

/// Wherever a click lands on one cell, a definition made for that cell (a generator from the cell
/// to itself, which is attached wherever the cell is) is offered, and what is offered is among
/// what the whole search finds; and clicks that do not land on one cell have none.
#[test]
fn a_definition_is_offered_wherever_a_click_lands_on_its_cell() {
    let (mut single, mut other) = (0, 0);
    for state in states(8) {
        // (A point is relative to the slice the workspace is at.)
        if !state.workspace.as_ref().unwrap().path.is_empty() {
            continue;
        }
        let diagram = state.workspace.as_ref().unwrap().diagram.clone();
        for depth in 1..=diagram.dimension().min(2) {
            for point in vertices(&diagram, depth) {
                let Some(cell) = cell_at(&diagram, &heights(&point)) else {
                    assert!(
                        DefinitionsJob::new(&state, &point, false, InferenceLimits::UNLIMITED)
                            .is_none_or(|job| definitions_of(&state.signature, job.of()).is_empty())
                    );
                    other += 1;
                    continue;
                };
                let mut with = state.clone();
                let info = with.signature.generator_info(cell).unwrap().clone();
                let Ok(made) = with.signature.create_generator(
                    info.diagram.clone(),
                    info.diagram.clone(),
                    "definition",
                    Invertibility::Directed,
                ) else {
                    continue;
                };
                let made = made.max_generator().generator;
                for weak_units in [false, true] {
                    let job =
                        DefinitionsJob::new(&with, &point, weak_units, InferenceLimits::UNLIMITED)
                            .expect("it has a definition now");
                    let outcome = run_to_end(job);
                    let whole = full(&with, &point, weak_units);
                    assert!(
                        outcome
                            .options
                            .iter()
                            .any(|option| option.generator == made),
                        "the definition of {cell:?} attaches at {point:?}"
                    );
                    for option in &outcome.options {
                        assert!(whole.contains(option), "{option:?} at {point:?}");
                    }
                    // They come in the order that the whole search has them.
                    let positions: Vec<_> = outcome
                        .options
                        .iter()
                        .map(|option| whole.iter().position(|other| other == option).unwrap())
                        .collect();
                    assert!(
                        positions.windows(2).all(|pair| pair[0] < pair[1]),
                        "{positions:?}"
                    );
                    single += 1;
                }
            }
        }
    }
    assert!(single > 20, "{single}");
    assert!(other > 20, "{other}");
}

/// A point on a wire or a region, or none at all, is not one cell.
#[test]
fn only_a_vertex_in_every_coordinate_lands_on_a_cell() {
    let mut checked = 0;
    for state in states(8) {
        let diagram = state.workspace.as_ref().unwrap().diagram.clone();
        assert_eq!(cell_at(&diagram, &[]), None);
        let Ok(d) = DiagramN::try_from(diagram.clone()) else {
            continue;
        };
        for i in 0..=2 * d.size() {
            let height = Height::from(i);
            if matches!(height, Height::Regular(_)) {
                assert_eq!(cell_at(&diagram, &[height]), None);
                checked += 1;
            }
        }
    }
    assert!(checked > 10);
}

/// The tangent categories project (7.4MB, not in the repository): `start` is a 5-cell with its
/// unfolding right after it. Run with `HOMOTOPY_TANGENT_FIXTURE=<path to the .hom>`, in release.
#[test]
fn the_unfolding_of_start_is_offered_at_once() {
    use std::time::Instant;
    let Ok(path) = std::env::var("HOMOTOPY_TANGENT_FIXTURE") else {
        return;
    };
    let bytes = std::fs::read(path).unwrap();
    let mut state = Proof::default();
    let t = Instant::now();
    state.update(&Action::ImportProof(bytes.into())).unwrap();
    eprintln!("import {:?}", t.elapsed());
    let start = state
        .signature
        .iter()
        .find(|info| info.name == "start")
        .map(|info| info.generator)
        .unwrap();
    assert_eq!((start.id, start.dimension), (62, 5));
    let t = Instant::now();
    let defs = definitions_of(&state.signature, start);
    eprintln!("definitions_of {:?}: {defs:?}", t.elapsed());
    state.update(&Action::SelectGenerator(start)).unwrap();
    let diagram = state.workspace.as_ref().unwrap().diagram.clone();
    let point = vec![
        SliceIndex::Interior(Height::Singular(0)),
        SliceIndex::Interior(Height::Singular(0)),
    ];
    let heights = [Height::Singular(0), Height::Singular(0)];
    assert_eq!(cell_at(&diagram, &heights), Some(start));
    let limits = InferenceLimits::DEFAULT;
    let t = Instant::now();
    let job = DefinitionsJob::new(&state, &point, false, limits).unwrap();
    let made = t.elapsed();
    let outcome = run_to_end(job);
    eprintln!(
        "definitions: made in {made:?}, all in {:?}: {} options {:?}",
        t.elapsed(),
        outcome.options.len(),
        outcome
            .options
            .iter()
            .map(|o| (o.generator, o.label.clone()))
            .collect::<Vec<_>>()
    );
    assert!(outcome
        .options
        .iter()
        .any(|o| o.generator == Generator::new(63, 6)));
    assert!(t.elapsed().as_secs() < 5);
    if std::env::var("HOMOTOPY_FIXTURE_FULL").is_ok() {
        let t = Instant::now();
        let whole =
            run_to_end(AttachOptionsJob::new(&state, &point, false, limits, None).unwrap()).options;
        eprintln!("full search {:?}, {} options", t.elapsed(), whole.len());
        for o in &outcome.options {
            assert!(whole.contains(o));
        }
    }
}
