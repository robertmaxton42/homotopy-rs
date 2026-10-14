//! Searching for the ways to attach one generator finds what the whole search finds for it, in
//! the same order: on the diagrams of the examples (states of every proof), at points of them
//! and of their boundaries, also through weak units.

use homotopy_common::hash::FastHashSet;
use homotopy_core::{Boundary, Diagram, DiagramN, Generator, Height, SliceIndex};
use homotopy_model::{
    history::Proof,
    job::Job,
    proof::{can_attach, Action, AttachOptionsJob, InferenceLimits, NoAttachment},
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

/// Some points of the diagram: the whole of it, singular and regular heights in its first two
/// dimensions, and its boundaries.
fn points(diagram: &Diagram) -> Vec<Vec<SliceIndex>> {
    let mut points: Vec<Vec<SliceIndex>> = vec![
        vec![],
        vec![SliceIndex::Boundary(Boundary::Source)],
        vec![SliceIndex::Boundary(Boundary::Target)],
    ];
    if let Ok(d) = DiagramN::try_from(diagram.clone()) {
        let size = d.size();
        for i in (0..=2 * size).step_by(1 + size / 2) {
            let height = SliceIndex::Interior(Height::from(i));
            points.push(vec![height]);
            points.push(vec![height, SliceIndex::Boundary(Boundary::Target)]);
            if let Some(Diagram::DiagramN(slice)) = d.slice(Height::from(i)) {
                points.push(vec![
                    height,
                    SliceIndex::Interior(Height::Singular(slice.size() / 2)),
                ]);
            }
        }
    }
    points
}

fn run(mut job: AttachOptionsJob) -> (homotopy_model::proof::AttachOutcome, usize) {
    let mut steps = 0;
    let mut last_done = 0;
    loop {
        let progress = job.progress().unwrap();
        assert!(progress.done <= progress.total, "{progress:?}");
        assert!(progress.done >= last_done);
        last_done = progress.done;
        steps += 1;
        if let Some(outcome) = job.step() {
            return (outcome, steps);
        }
    }
}

#[test]
fn one_generator_is_what_the_whole_search_finds_for_it() {
    let (mut searches, mut nonempty, mut split_up) = (0, 0, 0);
    let (mut rejected, mut kept) = (0, 0);
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
        for state in states.iter().step_by(stride) {
            let diagram = state.workspace.as_ref().unwrap().diagram.clone();
            let generators: Vec<Generator> = state.signature.iter().map(|i| i.generator).collect();
            for weak_units in [false, true] {
                let mut with_options = FastHashSet::default();
                for point in points(&diagram) {
                    let limits = InferenceLimits {
                        plain_budget_ms: 0,
                        ..InferenceLimits::default()
                    };
                    let Ok(whole) = AttachOptionsJob::new(state, &point, weak_units, limits, None)
                    else {
                        assert!(matches!(
                            AttachOptionsJob::new(state, &point, weak_units, limits, None),
                            Err(NoAttachment::NoBoundary)
                        ));
                        continue;
                    };
                    let (whole, _) = run(whole);
                    let mut found = 0;
                    for &generator in &generators {
                        let job = AttachOptionsJob::new(
                            state,
                            &point,
                            weak_units,
                            limits,
                            Some(generator),
                        )
                        .unwrap();
                        let (only, steps) = run(job);
                        let expected: Vec<_> = whole
                            .options
                            .iter()
                            .filter(|option| option.generator == generator)
                            .cloned()
                            .collect();
                        assert_eq!(only.options, expected, "{generator:?} at {point:?}");
                        found += only.options.len();
                        if !only.options.is_empty() {
                            with_options.insert(generator);
                        }
                        searches += 1;
                        nonempty += usize::from(!expected.is_empty());
                        split_up += usize::from(steps > 3);
                    }
                    assert_eq!(
                        found,
                        whole.options.len(),
                        "every option is for a generator"
                    );
                }
                // A generator that has a way to attach somewhere is never ruled out beforehand.
                for &generator in &generators {
                    let verdict = can_attach(
                        &state.signature,
                        Some(&diagram),
                        generator,
                        weak_units,
                        InferenceLimits {
                            plain_budget_ms: 0,
                            ..InferenceLimits::default()
                        },
                    );
                    if with_options.contains(&generator) {
                        assert_eq!(verdict, Ok(()), "{generator:?} (weak units: {weak_units})");
                        kept += 1;
                    } else if verdict.is_err() {
                        rejected += 1;
                    }
                }
            }
        }
    }
    assert!(
        rejected > 20 && kept > 20,
        "{rejected} rejected, {kept} kept"
    );
    assert!(searches > 1000, "{searches}");
    assert!(nonempty > 50, "some options are found: {nonempty}");
    assert!(
        split_up > 20,
        "the search for a generator is in pieces: {split_up}"
    );
}

#[test]
fn a_generator_that_is_not_there_is_not_searched_for() {
    let mut proof = Proof::default();
    for action in recorded_actions(EXAMPLES[0]) {
        proof.update(&action).ok();
    }
    let stranger = Generator::new(9999, 1);
    assert_eq!(
        AttachOptionsJob::new(
            &proof,
            &[],
            false,
            InferenceLimits {
                plain_budget_ms: 0,
                ..InferenceLimits::default()
            },
            Some(stranger)
        )
        .err(),
        Some(NoAttachment::UnknownGenerator)
    );
}
