//! Laying a diagram out elsewhere gives what laying it out here gives. "Elsewhere" is a thread of
//! its own, which shares none of the hash-consed diagrams and is reached by bytes only, as the
//! layout worker is.

use std::{
    sync::mpsc::{channel, Receiver, Sender},
    thread::{self, JoinHandle},
};

use homotopy_core::{layout::Layout, Diagram, SliceIndex};
use homotopy_model::{
    history::Proof,
    job::run_to_end,
    proof::{Action, ActionsJob, REPLAYING},
    remote::layout,
};

const EXAMPLES: [(&str, &str); 5] = [
    ("associator", include_str!("examples/associator.json")),
    (
        "braiding_half_bubbles",
        include_str!("examples/braiding_half_bubbles.json"),
    ),
    (
        "equivalence_to_adjoint_equivalence",
        include_str!("examples/equivalence_to_adjoint_equivalence.json"),
    ),
    (
        "merge_contract_RI",
        include_str!("examples/merge_contract_RI.json"),
    ),
    (
        "symmetric_monoidal_abelianize",
        include_str!("examples/symmetric_monoidal_abelianize.json"),
    ),
];

struct Elsewhere {
    requests: Option<Sender<Vec<u8>>>,
    responses: Receiver<(Vec<u8>, Vec<u32>)>,
    thread: Option<JoinHandle<()>>,
}

impl Elsewhere {
    fn new() -> Self {
        let (request_sender, request_receiver) = channel::<Vec<u8>>();
        let (response_sender, responses) = channel();
        let thread = thread::spawn(move || {
            for request in request_receiver {
                let mut pauses = vec![];
                let response = layout::run(&request, &mut |ms| pauses.push(ms));
                response_sender.send((response, pauses)).unwrap();
            }
        });
        Self {
            requests: Some(request_sender),
            responses,
            thread: Some(thread),
        }
    }

    fn ask(&self, request: Vec<u8>) -> (Vec<u8>, Vec<u32>) {
        self.requests.as_ref().unwrap().send(request).unwrap();
        self.responses.recv().unwrap()
    }
}

impl Drop for Elsewhere {
    fn drop(&mut self) {
        self.requests.take();
        self.thread.take().unwrap().join().unwrap();
    }
}

fn sorted<const N: usize>(layout: &Layout<N>) -> Vec<([SliceIndex; N], [f32; N])> {
    let mut points: Vec<_> = layout.points().map(|(k, v)| (*k, *v)).collect();
    points.sort_by(|a, b| a.0.cmp(&b.0));
    points
}

fn same_layout<const N: usize>(elsewhere: &Elsewhere, diagram: &Diagram, what: &str) {
    let here = Layout::<N>::new(diagram).unwrap();
    let (response, pauses) = elsewhere.ask(layout::request::<N>(diagram, 7));
    assert_eq!(pauses, vec![7], "the delay is passed on");
    let (there, _worked) = layout::response::<N>(&response).unwrap();
    assert_eq!(sorted(&here), sorted(&there), "{what}, N = {N}");
}

fn diagrams_of(json: &str) -> Vec<Diagram> {
    let (_safe, actions): (bool, Vec<Action>) = serde_json::from_str(json).unwrap();
    let outcome = run_to_end(ActionsJob::replay(Proof::default(), actions, REPLAYING));
    assert!(outcome.error.is_none());
    let proof = &outcome.applied.last().unwrap().1;
    let mut diagrams: Vec<Diagram> = proof
        .signature
        .iter()
        .map(|info| info.diagram.clone())
        .collect();
    if let Some(workspace) = &proof.workspace {
        diagrams.push(workspace.diagram.clone());
        diagrams.push(workspace.visible_diagram());
    }
    diagrams
}

#[test]
fn laying_out_elsewhere_is_laying_out_here() {
    let elsewhere = Elsewhere::new();
    let mut laid_out = 0;
    for (name, json) in EXAMPLES {
        for diagram in diagrams_of(json) {
            let dimension = diagram.dimension();
            same_layout::<0>(&elsewhere, &diagram, name);
            if dimension >= 1 {
                same_layout::<1>(&elsewhere, &diagram, name);
            }
            if dimension >= 2 {
                same_layout::<2>(&elsewhere, &diagram, name);
            }
            if dimension >= 3 {
                same_layout::<3>(&elsewhere, &diagram, name);
            }
            laid_out += 1;
        }
    }
    assert!(laid_out > 20);
}

#[test]
fn a_diagram_too_small_for_the_layout_is_a_failure() {
    let elsewhere = Elsewhere::new();
    let diagram = diagrams_of(EXAMPLES[0].1)
        .into_iter()
        .find(|d| d.dimension() == 0)
        .unwrap();
    let (response, _) = elsewhere.ask(layout::request::<2>(&diagram, 0));
    assert!(layout::response::<2>(&response).is_err());
}

#[test]
fn the_wrong_dimension_or_garbage_is_a_failure_not_a_crash() {
    let elsewhere = Elsewhere::new();
    let diagram = diagrams_of(EXAMPLES[0].1)
        .into_iter()
        .find(|d| d.dimension() >= 2)
        .unwrap();
    let (response, _) = elsewhere.ask(layout::request::<1>(&diagram, 0));
    assert!(layout::response::<2>(&response).is_err());
    assert!(layout::response::<1>(&response).is_ok());
    let (response, _) = elsewhere.ask(vec![0xc1, 1, 2, 3]);
    assert!(layout::response::<1>(&response).is_err());
    assert!(layout::response::<1>(&[0xc1, 2]).is_err());
}

#[test]
fn the_weight_of_a_diagram_is_bounded_by_its_limit() {
    let diagrams = diagrams_of(EXAMPLES[2].1);
    let biggest = diagrams
        .iter()
        .map(|d| homotopy_core::layout::weight(d, usize::MAX))
        .max()
        .unwrap();
    assert!(biggest > 100, "{biggest}");
    for diagram in &diagrams {
        let capped = homotopy_core::layout::weight(diagram, 50);
        let full = homotopy_core::layout::weight(diagram, usize::MAX);
        assert!(capped <= full);
        // It stops soon after the limit, rather than counting on.
        assert!(capped <= 50 || capped > 50 && capped < 50 + 64, "{capped}");
    }
}
