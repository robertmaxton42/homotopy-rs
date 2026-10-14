//! The time limit on the plain passes of the search for what to attach (the cells of the
//! signature, their variants and reflections: `InferenceLimits::plain_budget_ms`): a search that
//! is cut short finds some of what the whole search finds, in the same order, and all of it is
//! valid; without a limit nothing changes; the checkpoints that a worker makes along the way
//! say what was not done.

use homotopy_common::deadline;
use homotopy_core::{Boundary, Diagram, Generator, Height, SliceIndex};
use homotopy_model::{
    history::Proof,
    job::{run_to_end, Job},
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

fn states() -> Vec<Proof> {
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
        let stride = (states.len() / 6).max(1);
        all.extend(states.into_iter().step_by(stride));
    }
    all
}

fn points(diagram: &Diagram) -> Vec<Vec<SliceIndex>> {
    let mut points = vec![vec![], vec![SliceIndex::Boundary(Boundary::Target)]];
    if let Diagram::DiagramN(d) = diagram {
        let size = d.size();
        for i in (0..=2 * size).step_by(1 + size / 2) {
            points.push(vec![SliceIndex::Interior(Height::from(i))]);
        }
    }
    points
}

/// A limit that is a long time away, so that what is cut is what `deadline::after_checks` cuts.
const FAR: u32 = 3_600_000;

fn limited() -> InferenceLimits {
    InferenceLimits {
        plain_budget_ms: FAR,
        ..InferenceLimits::UNLIMITED
    }
}

fn of<'a>(
    outcome: &'a AttachOutcome,
    generator: Generator,
) -> Vec<&'a homotopy_model::proof::AttachOption> {
    outcome
        .options
        .iter()
        .filter(|option| option.generator == generator)
        .collect()
}

/// Whether `some` is among `all`, in the same order.
fn in_order<T: PartialEq>(all: &[T], some: &[T]) -> bool {
    let mut rest = all.iter();
    some.iter().all(|option| rest.any(|other| other == option))
}

#[test]
fn a_cut_plain_search_finds_some_of_what_the_whole_one_does_and_all_of_it_attaches() {
    let (mut cut_searches, mut with_less, mut identical) = (0, 0, 0);
    for state in states() {
        let diagram = state.workspace.as_ref().unwrap().diagram.clone();
        let generators: Vec<Generator> = state.signature.iter().map(|i| i.generator).collect();
        for point in points(&diagram) {
            let Ok(job) =
                AttachOptionsJob::new(&state, &point, true, InferenceLimits::UNLIMITED, None)
            else {
                continue;
            };
            let whole = run_to_end(job);
            assert!(!whole.inference.plain.timed_out());
            for checks in [0, 1, 7, 40, 300] {
                let job = AttachOptionsJob::new(&state, &point, true, limited(), None).unwrap();
                let cut = {
                    let _limit = deadline::after_checks(checks);
                    run_to_end(job)
                };
                cut_searches += 1;
                for &generator in &generators {
                    let (all, some) = (of(&whole, generator), of(&cut, generator));
                    assert!(
                        in_order(&all, &some),
                        "{generator:?} at {point:?} after {checks} checks: {} of {}",
                        some.len(),
                        all.len()
                    );
                }
                // It says so when it left something out, and does not when it did not.
                if cut.options.len() < whole.options.len() {
                    with_less += 1;
                    assert!(cut.inference.plain.timed_out(), "{point:?} after {checks}");
                } else if !cut.inference.plain.timed_out() {
                    assert_eq!(cut.options, whole.options);
                    identical += 1;
                }
                // Every option attaches.
                for option in cut.options.iter().take(5) {
                    assert!(
                        state
                            .clone()
                            .update(&Action::Attach(option.clone()))
                            .is_ok(),
                        "{point:?} after {checks}"
                    );
                }
                // What is not tried or cut short is a generator of the signature.
                for name in cut
                    .inference
                    .plain
                    .not_tried
                    .iter()
                    .chain(&cut.inference.plain.cut_short)
                {
                    assert!(state.signature.iter().any(|info| &info.name == name));
                }
            }
        }
    }
    assert!(cut_searches > 200, "{cut_searches}");
    assert!(
        with_less > 20,
        "searches that were cut and found less: {with_less}"
    );
    assert!(identical > 20, "searches that were not cut: {identical}");
}

#[test]
fn no_limit_is_the_search_it_was() {
    for state in states().iter().step_by(3) {
        let diagram = state.workspace.as_ref().unwrap().diagram.clone();
        for point in points(&diagram) {
            let (Ok(a), Ok(b)) = (
                AttachOptionsJob::new(state, &point, true, InferenceLimits::UNLIMITED, None),
                AttachOptionsJob::new(state, &point, true, limited(), None),
            ) else {
                continue;
            };
            let (a, b) = (run_to_end(a), run_to_end(b));
            assert_eq!(a.options, b.options);
            assert!(!b.inference.plain.timed_out());
            assert_eq!(b.inference.plain.budget_ms, FAR);
        }
    }
}

/// A search for one generator obeys the limit too, and what it finds is a prefix of what the
/// search for it finds.
#[test]
fn a_search_for_one_generator_obeys_the_limit_too() {
    let mut cut_any = false;
    for state in states().iter().step_by(2) {
        let diagram = state.workspace.as_ref().unwrap().diagram.clone();
        let generators: Vec<Generator> = state.signature.iter().map(|i| i.generator).collect();
        for point in points(&diagram) {
            for &generator in &generators {
                let Ok(job) = AttachOptionsJob::new(
                    state,
                    &point,
                    true,
                    InferenceLimits::UNLIMITED,
                    Some(generator),
                ) else {
                    continue;
                };
                let whole = run_to_end(job);
                for checks in [0, 1, 4] {
                    let job =
                        AttachOptionsJob::new(state, &point, true, limited(), Some(generator))
                            .unwrap();
                    let cut = {
                        let _limit = deadline::after_checks(checks);
                        run_to_end(job)
                    };
                    let (all, some): (Vec<_>, Vec<_>) =
                        (whole.options.iter().collect(), cut.options.iter().collect());
                    assert!(
                        in_order(&all, &some),
                        "{generator:?} at {point:?} after {checks}: {} of {}: {:?}",
                        cut.options.len(),
                        whole.options.len(),
                        cut.inference.plain,
                    );
                    if cut.options.len() < whole.options.len() {
                        cut_any = true;
                        assert!(cut.inference.plain.timed_out());
                    }
                }
            }
        }
    }
    assert!(cut_any, "some search for one generator was cut");
}

/// The first checkpoint is made before anything is done, so that whoever runs the search where
/// it can be ended knows that there is a limit, and says that nothing was looked at; later ones
/// say what has been found, which the whole search finds as well, and what is left.
#[test]
fn checkpoints_in_the_plain_passes_say_what_was_not_done() {
    let state = states()
        .into_iter()
        .max_by_key(|state| state.signature.iter().count())
        .unwrap();
    let mut job = AttachOptionsJob::new(&state, &[], true, limited(), None).unwrap();
    assert_eq!(
        job.time_allowed(),
        Some(std::time::Duration::from_millis(u64::from(FAR)))
    );
    let mut checkpoints = vec![];
    let outcome = loop {
        if let Some(checkpoint) = job.checkpoint() {
            checkpoints.push(checkpoint);
        }
        if let Some(outcome) = job.step() {
            break outcome;
        }
    };
    assert!(checkpoints.len() > 2, "{}", checkpoints.len());
    let first = &checkpoints[0];
    assert!(first.options.is_empty());
    assert!(first.inference.forced);
    assert!(!first.inference.plain.cut_short.is_empty());
    for checkpoint in &checkpoints {
        assert!(checkpoint
            .options
            .iter()
            .all(|o| outcome.options.contains(o)));
        assert!(
            checkpoint.inference.plain.timed_out()
                || checkpoint.options.len() == outcome.options.len()
        );
    }
    // As it goes there is less that was not tried.
    let left = |c: &AttachOutcome| c.inference.plain.not_tried.len();
    assert!(left(&checkpoints[0]) >= left(checkpoints.last().unwrap()));
    assert!(!outcome.inference.plain.timed_out() && !outcome.inference.forced);
}
