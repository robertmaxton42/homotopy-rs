//! Doing a job elsewhere gives what doing it here gives. "Elsewhere" is a thread of its own,
//! which shares none of the hash-consed diagrams (they live in thread-locals) and is reached by
//! bytes only, as a web worker is.

use std::{
    sync::mpsc::{channel, Receiver, Sender},
    thread::{self, JoinHandle},
};

use homotopy_core::{Boundary, SliceIndex};
use homotopy_model::{
    history::{proof_from_state, Proof},
    job::{run_to_end, Progress},
    proof::{Action, ActionsJob, ActionsOutcome, AttachOptionsJob, ProofState, REPLAYING},
    remote::{Answer, Executor, Output, Remote, Step, Work},
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

fn recorded_actions(json: &str) -> Vec<Action> {
    let (_safe, actions): (bool, Vec<Action>) = serde_json::from_str(json).unwrap();
    actions
}

/// A thread running an executor, which gets requests and gives responses, and what it said
/// before its steps.
struct Elsewhere {
    requests: Option<Sender<Vec<u8>>>,
    responses: Receiver<(Vec<u8>, Vec<Step>)>,
    thread: Option<JoinHandle<()>>,
}

impl Elsewhere {
    fn new() -> Self {
        let (request_sender, request_receiver) = channel::<Vec<u8>>();
        let (response_sender, responses) = channel();
        let thread = thread::spawn(move || {
            let mut executor = Executor::default();
            for request in request_receiver {
                let mut steps = vec![];
                let response = executor.run(&request, &mut |step| steps.push(step));
                response_sender.send((response, steps)).unwrap();
            }
        });
        Self {
            requests: Some(request_sender),
            responses,
            thread: Some(thread),
        }
    }

    fn ask(&self, request: Vec<u8>) -> (Vec<u8>, Vec<Step>) {
        self.requests.as_ref().unwrap().send(request).unwrap();
        self.responses.recv().unwrap()
    }

    /// Does the work on the proof there, as `remote` would have it.
    fn work(&self, remote: &mut Remote, proof: &ProofState, work: &Work) -> (Output, Vec<Step>) {
        let mut send_signature = false;
        loop {
            let (response, steps) =
                self.ask(remote.request(proof, work.clone(), 0, send_signature));
            match remote.response(&response).unwrap() {
                Answer::Done { output, .. } => return (output, steps),
                Answer::NeedsSignature => send_signature = true,
            }
        }
    }
}

impl Drop for Elsewhere {
    fn drop(&mut self) {
        self.requests.take();
        self.thread.take().unwrap().join().unwrap();
    }
}

fn actions_output(output: Output) -> ActionsOutcome {
    match output {
        Output::Actions(outcome) => outcome,
        Output::AttachOptions(_) => panic!("expected actions"),
    }
}

/// That the outcomes are the same: the same actions, with the same proofs after them (down to the
/// numbering of the nodes of the signatures) and the same error.
fn assert_same_outcome(here: &ActionsOutcome, there: &ActionsOutcome) {
    assert_eq!(here.applied.len(), there.applied.len());
    assert_eq!(here.positions, there.positions);
    for ((here_action, here_proof), (there_action, there_proof)) in
        here.applied.iter().zip(&there.applied)
    {
        assert_eq!(here_action, there_action);
        assert_eq!(**here_proof, **there_proof);
        assert!(
            here_proof.signature.identical(&there_proof.signature),
            "the nodes of the signature are numbered as they were"
        );
    }
    assert_eq!(
        here.error.as_ref().map(ToString::to_string),
        there.error.as_ref().map(ToString::to_string)
    );
}

#[test]
fn replaying_elsewhere_is_replaying_here() {
    let elsewhere = Elsewhere::new();
    for (name, json) in EXAMPLES {
        let actions = recorded_actions(json);
        let mut remote = Remote::default();
        let work = Work::Actions {
            actions: actions.clone(),
            replay: true,
        };
        let (output, steps) = elsewhere.work(&mut remote, &ProofState::default(), &work);
        let there = actions_output(output);
        let here = run_to_end(ActionsJob::replay(Proof::default(), actions, REPLAYING));
        assert!(here.error.is_none(), "{name}");
        assert_same_outcome(&here, &there);
        // It said what it was doing before every step, and got further along.
        assert!(!steps.is_empty());
        assert_eq!(steps[0].label, "Replaying actions");
        let done: Vec<usize> = steps
            .iter()
            .map(|step| step.progress.map_or(0, |Progress { done, .. }| done))
            .collect();
        assert!(done.windows(2).all(|pair| pair[0] <= pair[1]), "{name}");
    }
}

#[test]
fn the_signature_is_not_sent_again() {
    let elsewhere = Elsewhere::new();
    let actions = recorded_actions(EXAMPLES[0].1);
    let (first, second) = actions.split_at(actions.len() / 2);
    let mut remote = Remote::default();

    // The first half, from nothing.
    let work = Work::Actions {
        actions: first.to_vec(),
        replay: true,
    };
    let (output, _) = elsewhere.work(&mut remote, &ProofState::default(), &work);
    let middle = actions_output(output);
    let middle_state: ProofState = (**middle.applied.last().unwrap().1).clone();
    assert!(middle_state.signature.iter().count() > 1);

    // The second half, from there: the executor has the signature from the first, so it is
    // not sent again, and the job gives what it does here.
    let work = Work::Actions {
        actions: second.to_vec(),
        replay: true,
    };
    let here = run_to_end(ActionsJob::replay(
        proof_from_state(middle_state.clone()),
        second.to_vec(),
        REPLAYING,
    ));
    let held = remote.request(&middle_state, work.clone(), 0, false);
    let (response, _) = elsewhere.ask(held.clone());
    let Answer::Done { output, .. } = remote.response(&response).unwrap() else {
        panic!("the executor held the signature");
    };
    assert_same_outcome(&here, &actions_output(output));
    // The request that leaves the signature out is shorter than one with it.
    let sent = remote.request(&middle_state, work.clone(), 0, true);
    assert!(
        held.len() < sent.len(),
        "({} against {})",
        held.len(),
        sent.len()
    );

    // An executor that restarted holds nothing, which the remote end is not told. It asks again
    // with the signature, and the result is the same.
    let mut remote = Remote::default();
    let restarted = Elsewhere::new();
    let (output, _) = restarted.work(&mut remote, &middle_state, &work);
    assert_same_outcome(&here, &actions_output(output));
    let other = Elsewhere::new();
    let (output, _) = other.work(&mut remote, &middle_state, &work);
    assert_same_outcome(&here, &actions_output(output));
}

#[test]
fn a_chain_elsewhere_is_a_chain_here() {
    // Everything up to the last homotopy, then the homotopy followed by merging exchanges.
    let elsewhere = Elsewhere::new();
    for (name, json) in EXAMPLES {
        let actions = recorded_actions(json);
        let Some(last) = actions
            .iter()
            .rposition(|action| matches!(action, Action::Homotopy(_)))
        else {
            continue;
        };
        let mut proof = Proof::default();
        for action in &actions[..last] {
            proof.update(action).unwrap();
        }
        let chain = vec![actions[last].clone(), Action::MergeExchanges];
        let here = run_to_end(ActionsJob::chain(proof.clone(), chain.clone()));
        let mut remote = Remote::default();
        let work = Work::Actions {
            actions: chain,
            replay: false,
        };
        let (output, steps) = elsewhere.work(&mut remote, &proof, &work);
        let there = actions_output(output);
        assert_same_outcome(&here, &there);
        assert!(!here.applied.is_empty(), "{name}");
        assert!(steps.iter().any(|step| step.label == "Merging exchanges"));
    }
}

#[test]
fn an_error_elsewhere_is_the_error_here() {
    let elsewhere = Elsewhere::new();
    let actions = vec![
        Action::CreateGeneratorZero,
        Action::ImportProof(vec![1, 2, 3].into()),
        Action::CreateGeneratorZero,
    ];
    let here = run_to_end(ActionsJob::replay(
        Proof::default(),
        actions.clone(),
        REPLAYING,
    ));
    let mut remote = Remote::default();
    let work = Work::Actions {
        actions,
        replay: true,
    };
    let (output, _) = elsewhere.work(&mut remote, &ProofState::default(), &work);
    let there = actions_output(output);
    assert_eq!(there.applied.len(), 1);
    assert!(there.error.is_some());
    assert_same_outcome(&here, &there);
}

#[test]
fn a_garbled_request_is_a_failure_not_a_crash() {
    let elsewhere = Elsewhere::new();
    let mut remote = Remote::default();
    let (response, _) = elsewhere.ask(vec![0xc1, 1, 2, 3]);
    assert!(remote.response(&response).is_err());
    assert!(remote.response(&[0xc1, 2, 3]).is_err());
}

#[test]
fn attach_options_elsewhere_are_attach_options_here() {
    let elsewhere = Elsewhere::new();
    for (name, json) in &EXAMPLES[..2] {
        let mut actions = recorded_actions(json);
        let first_attach = actions
            .iter()
            .position(|a| matches!(a, Action::Attach(_)))
            .unwrap();
        actions.truncate(first_attach);
        let mut proof = Proof::default();
        for action in &actions {
            proof.update(action).unwrap();
        }
        if proof.workspace.is_none() {
            continue;
        }
        let point = vec![SliceIndex::Boundary(Boundary::Target)];
        for weak_units in [false, true] {
            let here = run_to_end(AttachOptionsJob::new(&proof, &point, weak_units).unwrap());
            let mut remote = Remote::default();
            let work = Work::AttachOptions {
                point: point.clone(),
                weak_units,
            };
            let (output, _) = elsewhere.work(&mut remote, &proof, &work);
            let Output::AttachOptions(there) = output else {
                panic!("expected options");
            };
            assert_eq!(here, there, "{name}");
        }
    }
}

#[test]
fn what_a_job_offers_is_what_it_does() {
    let actions = recorded_actions(EXAMPLES[3].1);
    let proof = Proof::default();
    let job = ActionsJob::replay(proof.clone(), actions.clone(), REPLAYING);
    let (state, work) = job.offer().unwrap();
    assert_eq!(*state, **proof);
    match work {
        Work::Actions {
            actions: offered,
            replay,
        } => {
            assert_eq!(offered, actions);
            assert!(replay);
        }
        Work::AttachOptions { .. } => panic!("expected actions"),
    }
    // A job that has begun is not one to hand to anyone.
    let mut job = ActionsJob::chain(proof, actions);
    assert!(job.offer().is_some());
    homotopy_model::job::Job::step(&mut job);
    assert!(job.offer().is_none());
}
