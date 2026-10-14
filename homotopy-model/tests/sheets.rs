//! Choosing a sheet (one more coordinate after those the view shows, see `Depths::sheets`) when
//! looking for what to attach at a point: the options are exactly those of the point without it
//! whose embedding contains the sheet, in the same order; and the heuristic that narrows the pool
//! of cells for instances to the sheet only finds some of what the whole pool finds.

use homotopy_core::{common::BoundaryPath, Boundary, Diagram, Height, SliceIndex};
use homotopy_model::{
    history::Proof,
    job::run_to_end,
    proof::{Action, AttachOptionsJob, AttachOutcome, InferenceLimits},
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

/// Some of the places in the first `n` coordinates of a diagram, with what is left of the diagram
/// there: boundaries, and a few heights, regular and singular.
fn clicks(diagram: &Diagram, n: usize) -> Vec<(Vec<SliceIndex>, Diagram)> {
    if n == 0 {
        return vec![(vec![], diagram.clone())];
    }
    let Diagram::DiagramN(d) = diagram else {
        return vec![];
    };
    let size = d.size();
    let mut indices = vec![
        SliceIndex::Boundary(Boundary::Source),
        SliceIndex::Boundary(Boundary::Target),
    ];
    indices.extend(
        (0..=2 * size)
            .step_by(1 + size / 2)
            .map(|i| SliceIndex::Interior(Height::from(i))),
    );
    let mut all = vec![];
    for index in indices {
        let slice = d.slice(index).unwrap();
        for (mut rest, fibre) in clicks(&slice, n - 1) {
            rest.insert(0, index);
            all.push((rest, fibre));
        }
    }
    all
}

fn options(
    state: &Proof,
    point: &[SliceIndex],
    weak_units: bool,
    limits: InferenceLimits,
) -> Option<AttachOutcome> {
    AttachOptionsJob::new(state, point, weak_units, limits, None)
        .ok()
        .map(run_to_end)
}

#[test]
fn a_sheet_keeps_the_options_whose_embedding_contains_it() {
    let (mut searches, mut sheeted, mut narrowed_down, mut kept_any) = (0, 0, 0, 0);
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
        let stride = (states.len() / 8).max(1);
        for state in states.iter().step_by(stride) {
            let workspace = state.workspace.as_ref().unwrap();
            let n = usize::from(workspace.view.dimension());
            if n == 0 {
                continue;
            }
            for weak_units in [false, true] {
                let limits = InferenceLimits {
                    plain_budget_ms: 0,
                    ..InferenceLimits::default()
                };
                for (point, fibre) in clicks(&workspace.diagram, n) {
                    let Diagram::DiagramN(fibre) = fibre else {
                        continue;
                    };
                    let Some(whole) = options(state, &point, weak_units, limits) else {
                        continue;
                    };
                    for z in 0..fibre.size().min(4) {
                        let mut with_sheet = point.clone();
                        with_sheet.push(SliceIndex::Interior(Height::Singular(z)));
                        let found = options(state, &with_sheet, weak_units, limits)
                            .unwrap_or_else(|| panic!("{with_sheet:?} has no search"));

                        // What the embedding of an option is to contain: the point, as the search
                        // takes it, after the boundary of it.
                        let (boundary_path, heights) = BoundaryPath::split(&with_sheet);
                        let expected: Vec<_> = whole
                            .options
                            .iter()
                            .filter(|option| {
                                let boundary =
                                    boundary_path.map_or(Boundary::Target, |b| b.boundary());
                                let needle = option.diagram.slice(boundary.flip()).unwrap();
                                let embedding: Vec<_> = option.embedding.iter().copied().collect();
                                needle.contains_point(&heights, &embedding)
                            })
                            .cloned()
                            .collect();
                        assert_eq!(
                            found.options, expected,
                            "at {point:?} with the sheet {z} (weak units: {weak_units})"
                        );
                        searches += 1;
                        sheeted += 1;
                        narrowed_down += usize::from(expected.len() < whole.options.len());
                        kept_any += usize::from(!expected.is_empty());
                    }
                }
            }
        }
    }
    assert!(searches > 200, "{searches}");
    assert!(
        sheeted > 0 && kept_any > 50,
        "{kept_any} of {sheeted} keep some"
    );
    assert!(
        narrowed_down > 20,
        "a sheet leaves out options: {narrowed_down} of {searches}"
    );
}

/// The point for a sheet is the point of the click and then the singular height, which a
/// boundary of the click's coordinates does not interfere with.
#[test]
fn a_sheet_after_a_boundary_is_a_point_of_the_boundary() {
    let (boundary, heights) = BoundaryPath::split(&[
        SliceIndex::Interior(Height::Regular(0)),
        SliceIndex::Boundary(Boundary::Source),
        SliceIndex::Interior(Height::Singular(2)),
    ]);
    assert_eq!(boundary, Some(BoundaryPath(Boundary::Source, 1)));
    assert_eq!(heights, vec![Height::Singular(2)]);
}
