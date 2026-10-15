//! The options of a search for attachments can be had while the search goes on, a few at a time
//! (`AttachOptionsJob::take_found`, and in a worker the options sent ahead of the answer): put
//! together in their places (`Streamed`) they are exactly the options of the outcome, in the
//! same order, and what is there is never moved while more arrives.

use std::{sync::mpsc::channel, thread};

use homotopy_core::{
    signature::Invertibility, Boundary, Diagram, DiagramN, Generator, Height, SliceIndex,
};
use homotopy_model::{
    history::Proof,
    job::{run_to_end, Job},
    proof::{
        Action, AttachOption, AttachOptionsJob, AttachOutcome, InferenceLimits, ProofState,
        Signature, SignatureEdit, SignatureItemEdit, Streamed, Workspace,
    },
    remote::{Answer, Executor, Output, Remote, Work},
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

/// That `before` is still there in `after`, in the same order, with only more between.
fn is_subsequence(before: &[AttachOption], after: &[AttachOption]) -> bool {
    let mut rest = after.iter();
    before
        .iter()
        .all(|option| rest.any(|other| other == option))
}

/// Runs the job a step at a time, taking what it found after each step (every `every` steps), and
/// returns the outcome and what was taken, put together. Checks on the way that what has been
/// shown is not moved.
fn streamed(mut job: AttachOptionsJob, every: usize) -> (AttachOutcome, Streamed, usize) {
    let mut streamed = Streamed::default();
    let mut shown: Vec<AttachOption> = vec![];
    let mut arrivals = 0;
    let mut steps = 0;
    let outcome = loop {
        steps += 1;
        let output = job.step();
        if output.is_some() || steps % every == 0 {
            let found = job.take_found();
            if !found.is_empty() {
                arrivals += 1;
            }
            streamed.extend(found);
            let now = streamed.options();
            assert!(
                is_subsequence(&shown, &now),
                "options that were shown moved"
            );
            assert_eq!(now.len(), streamed.len());
            shown = now;
        }
        if let Some(outcome) = output {
            break outcome;
        }
    };
    // Nothing is left to take once the job is over.
    assert!(job.take_found().is_empty());
    (outcome, streamed, arrivals)
}

fn states_of(json: &str, wanted: usize) -> Vec<ProofState> {
    let mut proof = Proof::default();
    let mut states = vec![];
    for action in recorded_actions(json) {
        if proof.update(&action).is_err() {
            continue;
        }
        if proof.workspace.is_some() {
            states.push((**proof).clone());
        }
    }
    let stride = (states.len() / wanted).max(1);
    states.into_iter().step_by(stride).collect()
}

const NO_LIMIT: InferenceLimits = InferenceLimits {
    generator_ms: 0,
    ..InferenceLimits::DEFAULT
};

#[test]
fn options_taken_as_they_are_found_are_the_options_in_the_same_order() {
    let (mut searches, mut with_arrivals, mut with_options) = (0, 0, 0);
    for json in EXAMPLES {
        for state in states_of(json, 4) {
            let diagram = state.workspace.as_ref().unwrap().diagram.clone();
            for weak_units in [false, true] {
                for point in points(&diagram) {
                    // (Taking after every step, and now and then.)
                    for every in [1, 3] {
                        let Ok(job) =
                            AttachOptionsJob::new(&state, &point, weak_units, NO_LIMIT, None)
                        else {
                            continue;
                        };
                        let (outcome, streamed, arrivals) = streamed(job, every);
                        assert_eq!(streamed.options(), outcome.options, "{point:?}");
                        assert_eq!(streamed.len(), outcome.options.len());
                        searches += 1;
                        with_options += usize::from(!outcome.options.is_empty());
                        with_arrivals += usize::from(arrivals > 1);
                    }
                }
            }
        }
    }
    assert!(searches > 100, "{searches}");
    assert!(with_options > 20, "{with_options}");
    // Some of the searches found their options in more than one go.
    assert!(with_arrivals > 5, "{with_arrivals}");
}

/// The generators are searched cheapest first and the options put back in the order of the
/// generators: what is found late can be placed before what was found early.
#[test]
fn what_is_found_late_can_come_before_what_was_found_early() {
    let mut late_before_early = 0;
    for json in EXAMPLES {
        for state in states_of(json, 4) {
            let diagram = state.workspace.as_ref().unwrap().diagram.clone();
            for point in points(&diagram) {
                let Ok(mut job) = AttachOptionsJob::new(&state, &point, true, NO_LIMIT, None)
                else {
                    continue;
                };
                let mut order = vec![];
                loop {
                    let output = job.step();
                    order.extend(job.take_found().into_iter().map(|found| found.slot));
                    if output.is_some() {
                        break;
                    }
                }
                // Not in the order of the final list, because the search is not in it.
                if order.windows(2).any(|pair| pair[1] < pair[0]) {
                    late_before_early += 1;
                }
            }
        }
    }
    assert!(late_before_early > 0);
}

/// A search that is given a short time for each generator: what it finds is still what is
/// put together from what it said as it went.
#[test]
fn a_search_cut_short_streams_what_it_finds() {
    let state = states_of(EXAMPLES[2], 1).pop().unwrap();
    let limits = InferenceLimits {
        generator_ms: 1,
        ..InferenceLimits::DEFAULT
    };
    let job = AttachOptionsJob::new(&state, &[], true, limits, None).unwrap();
    let (outcome, streamed, _) = streamed(job, 1);
    assert_eq!(streamed.options(), outcome.options);
}

/// What a worker says, in the order it says it.
enum Said {
    Options(Vec<u8>),
    Checkpoint(Vec<u8>),
}

/// Does the work on a thread of its own, streaming: the bytes of everything it says, and its
/// answer.
fn elsewhere(request: Vec<u8>) -> (Vec<u8>, Vec<Said>) {
    let (sender, receiver) = channel();
    let answer = thread::spawn(move || {
        let (options, checkpoints) = (sender.clone(), sender);
        let answer = Executor::default().run_streaming(
            &request,
            &mut |_| {},
            &mut |bytes| checkpoints.send(Said::Checkpoint(bytes)).unwrap(),
            Some(&mut |bytes| options.send(Said::Options(bytes)).unwrap()),
        );
        answer
    })
    .join()
    .unwrap();
    (answer, receiver.try_iter().collect())
}

#[test]
fn options_sent_ahead_by_a_worker_are_the_options_of_the_answer() {
    let (mut asked, mut sent_ahead, mut checked) = (0, 0, 0);
    for json in &EXAMPLES[..3] {
        for state in states_of(json, 3) {
            let diagram = state.workspace.as_ref().unwrap().diagram.clone();
            for point in points(&diagram).into_iter().step_by(2) {
                let limits = InferenceLimits {
                    // (A limit, so that there are checkpoints, that nothing reaches.)
                    generator_ms: 600_000,
                    ..InferenceLimits::DEFAULT
                };
                let Ok(job) = AttachOptionsJob::new(&state, &point, true, limits, None) else {
                    continue;
                };
                let here = run_to_end(job);
                let work = Work::AttachOptions {
                    point: point.clone(),
                    weak_units: true,
                    limits,
                    only: None,
                };
                let mut remote = Remote::default();
                let (answer, said) = elsewhere(remote.request(&state, work.clone(), 0, false));

                // The options as they come, in their places.
                let mut shown: Vec<AttachOption> = vec![];
                let mut so_far = Streamed::default();
                let mut arrivals = 0;
                for (i, event) in said.iter().enumerate() {
                    match event {
                        Said::Options(bytes) => {
                            let found = remote.options(bytes).unwrap();
                            assert!(!found.is_empty());
                            arrivals += 1;
                            so_far.extend(found);
                            let now = so_far.options();
                            assert!(is_subsequence(&shown, &now));
                            shown = now;
                        }
                        Said::Checkpoint(bytes) => {
                            // What the worker would have come to if it had been ended here: the
                            // options sent ahead of it and the report.
                            let mut again = Remote::default();
                            again.request(&state, work.clone(), 0, false);
                            for earlier in &said[..i] {
                                if let Said::Options(bytes) = earlier {
                                    again.options(bytes).unwrap();
                                }
                            }
                            let Answer::Done {
                                output: Output::AttachOptions(partial),
                                ..
                            } = again.response(bytes).unwrap()
                            else {
                                panic!("a checkpoint is an answer");
                            };
                            assert!(partial.inference.forced);
                            assert_eq!(partial.options, shown);
                            checked += 1;
                        }
                    }
                }
                let Answer::Done {
                    output: Output::AttachOptions(there),
                    ..
                } = remote.response(&answer).unwrap()
                else {
                    panic!("expected options");
                };
                assert_eq!(here.options, there.options, "{point:?}");
                assert_eq!(
                    here.inference.plain.cut_short,
                    there.inference.plain.cut_short
                );
                assert_eq!(here.inference.inferred, there.inference.inferred);
                assert_eq!(shown, there.options);
                asked += 1;
                sent_ahead += usize::from(arrivals > 0);
            }
        }
    }
    assert!(asked > 20, "{asked}");
    assert!(sent_ahead > 5, "{sent_ahead}");
    assert!(checked > asked, "{checked}");
}

/// An answer that says the options were sent ahead is not one to use if they did not all get
/// here.
#[test]
fn an_answer_that_refers_to_options_that_did_not_arrive_is_not_used() {
    let state = states_of(EXAMPLES[0], 1).pop().unwrap();
    let diagram = state.workspace.as_ref().unwrap().diagram.clone();
    for point in points(&diagram) {
        let limits = NO_LIMIT;
        let Ok(job) = AttachOptionsJob::new(&state, &point, true, limits, None) else {
            continue;
        };
        if run_to_end(job).options.is_empty() {
            continue;
        }
        let work = Work::AttachOptions {
            point,
            weak_units: true,
            limits,
            only: None,
        };
        let mut remote = Remote::default();
        let (answer, said) = elsewhere(remote.request(&state, work, 0, false));
        let mut deltas = said.iter().filter_map(|said| match said {
            Said::Options(bytes) => Some(bytes),
            Said::Checkpoint(_) => None,
        });
        // The first never gets here.
        deltas.next().expect("something was found");
        for bytes in deltas {
            remote.options(bytes).unwrap();
        }
        assert!(remote.response(&answer).is_err());
        return;
    }
    panic!("an example has a point with options");
}

/// Not streaming, a worker answers with all of the options, as it always has.
#[test]
fn without_a_stream_the_answer_has_all_the_options() {
    let state = states_of(EXAMPLES[0], 1).pop().unwrap();
    let work = Work::AttachOptions {
        point: vec![],
        weak_units: true,
        limits: NO_LIMIT,
        only: None,
    };
    let mut remote = Remote::default();
    let request = remote.request(&state, work, 0, false);
    let answer = thread::spawn(move || Executor::default().run(&request, &mut |_| {}))
        .join()
        .unwrap();
    assert!(matches!(
        remote.response(&answer).unwrap(),
        Answer::Done {
            output: Output::AttachOptions(_),
            ..
        }
    ));
}

/// The tangent categories project (not in the repository), at one point, with a short time for
/// each generator. Run with `HOMOTOPY_TANGENT_FIXTURE=<path to the .hom>`, in release.
#[test]
fn the_tangent_categories_stream_is_the_search() {
    use std::time::Instant;
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
    // (The whole diagram: the most to find.)
    let point = vec![];
    let limits = InferenceLimits {
        generator_ms: 150,
        ..InferenceLimits::DEFAULT
    };
    let t = Instant::now();
    let job = AttachOptionsJob::new(&state, &point, false, limits, None).unwrap();
    let (outcome, streamed, arrivals) = streamed(job, 1);
    eprintln!(
        "{} options in {} arrivals, {:?}",
        outcome.options.len(),
        arrivals,
        t.elapsed()
    );
    assert_eq!(streamed.options(), outcome.options);
    assert!(!outcome.options.is_empty());
}

// Instances of schemas, and every way that a search for attachments is started.

fn node_of(sig: &Signature, generator: Generator) -> homotopy_common::tree::Node {
    sig.as_tree()
        .iter()
        .find(|(_, item)| {
            matches!(item.inner(), homotopy_model::proof::SignatureItem::Item(info) if info.generator == generator)
        })
        .map(|(node, _)| node)
        .unwrap()
}

fn make_placeholder(sig: &mut Signature, generator: Generator) {
    let node = node_of(sig, generator);
    sig.update(&SignatureEdit::Edit(
        node,
        SignatureItemEdit::MakePlaceholder(true),
    ))
    .unwrap();
}

/// Two independent placeholders `P, P' : x -> x` with a 2-cell each, `e : P => P` and
/// `e' : P' => P'`; the schema `S : e ==> e` which depends on `e'`; and a diagram with cells to
/// replace them with: the identity on `q ; q ; q`, or, `big`, one with a great deal of choice.
/// With the schema, which the search for instances is for.
fn schema_state(big: bool) -> (ProofState, Generator) {
    let mut sig = Signature::default();
    let x: Diagram = sig.create_generator_zero("x").into();
    let cell = |sig: &mut Signature, s: Diagram, t: Diagram| {
        sig.create_generator(s, t, "c", Invertibility::Directed)
            .unwrap()
    };
    let p = cell(&mut sig, x.clone(), x.clone());
    let p2 = cell(&mut sig, x.clone(), x.clone());
    let q = cell(&mut sig, x.clone(), x.clone());
    let e = cell(&mut sig, p.clone().into(), p.clone().into());
    let e2 = cell(&mut sig, p2.clone().into(), p2.clone().into());
    let u = cell(&mut sig, q.clone().into(), q.clone().into());
    let s = cell(&mut sig, e.clone().into(), e.clone().into());
    let g = |d: &DiagramN| d.max_generator().generator;
    for d in [&p, &p2, &e, &e2] {
        make_placeholder(&mut sig, g(d));
    }
    let node = node_of(&sig, g(&s));
    sig.update(&SignatureEdit::Edit(
        node,
        SignatureItemEdit::SetDependsOn(vec![g(&e2)]),
    ))
    .unwrap();
    let haystack: Diagram = if big {
        let mut d = q.identity();
        for _ in 0..3 {
            d = d.attach(&u, Boundary::Target, &[]).unwrap();
        }
        d.into()
    } else {
        q.attach(&q, Boundary::Target, &[])
            .unwrap()
            .attach(&q, Boundary::Target, &[])
            .unwrap()
            .identity()
            .into()
    };
    (
        ProofState {
            signature: sig,
            workspace: Some(Workspace::new(haystack)),
            ..ProofState::default()
        },
        g(&s),
    )
}

fn instances(options: &[AttachOption]) -> usize {
    options
        .iter()
        .filter(|option| option.substitution.is_some())
        .count()
}

/// The instances of a schema come as they are found, in more than one go and before the schema
/// is done with, in the places they have in the end, in the search for everything at a point
/// (a click) and in the search for the schema alone (a drop, an armed click).
#[test]
fn instances_are_streamed_as_they_are_found_for_a_click_and_for_one_generator() {
    for big in [false, true] {
        let (state, schema) = schema_state(big);
        for only in [None, Some(schema)] {
            for every in [1, 2] {
                let job = AttachOptionsJob::new(&state, &[], false, NO_LIMIT, only).unwrap();
                let (outcome, streamed, _) = streamed(job, every);
                assert_eq!(streamed.options(), outcome.options, "big {big}, {only:?}");
                assert!(instances(&outcome.options) > 1, "big {big}");
            }
            // One at a time: the number of arrivals that bring instances is more than the
            // schemas there are (one), which it would not be if they came when it was done.
            let mut job = AttachOptionsJob::new(&state, &[], false, NO_LIMIT, only).unwrap();
            let mut arrivals = 0;
            loop {
                let output = job.step();
                arrivals += usize::from(
                    job.take_found()
                        .iter()
                        .any(|found| found.option.substitution.is_some()),
                );
                if output.is_some() {
                    break;
                }
            }
            assert!(arrivals > 1, "big {big}, {only:?}: {arrivals}");
        }
    }
}

/// The same through a worker: the instances are sent ahead as they are found, in more than one
/// message, and put together in their places they are the answer, for a click and for one
/// generator.
#[test]
fn instances_sent_ahead_by_a_worker_are_the_instances_of_the_answer() {
    for big in [false, true] {
        let (state, schema) = schema_state(big);
        for only in [None, Some(schema)] {
            let here =
                run_to_end(AttachOptionsJob::new(&state, &[], false, NO_LIMIT, only).unwrap());
            let work = Work::AttachOptions {
                point: vec![],
                weak_units: false,
                limits: NO_LIMIT,
                only,
            };
            let mut remote = Remote::default();
            let (answer, said) = elsewhere(remote.request(&state, work, 0, false));
            let mut so_far = Streamed::default();
            let mut shown: Vec<AttachOption> = vec![];
            let mut with_instances = 0;
            for event in &said {
                if let Said::Options(bytes) = event {
                    let found = remote.options(bytes).unwrap();
                    with_instances += usize::from(
                        found
                            .iter()
                            .any(|found| found.option.substitution.is_some()),
                    );
                    so_far.extend(found);
                    let now = so_far.options();
                    assert!(is_subsequence(&shown, &now), "what was shown moved");
                    shown = now;
                }
            }
            assert!(with_instances > 1, "big {big}, {only:?}: {with_instances}");
            let Answer::Done {
                output: Output::AttachOptions(there),
                ..
            } = remote.response(&answer).unwrap()
            else {
                panic!("expected options");
            };
            assert_eq!(here.options, there.options);
            assert_eq!(shown, there.options);
        }
    }
}
