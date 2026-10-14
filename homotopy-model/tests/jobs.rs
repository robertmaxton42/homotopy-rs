//! Jobs give what performing the same actions straight away does, and leave what they were made
//! from alone.

use homotopy_core::{Boundary, SliceIndex};
use homotopy_model::{
    history::Proof,
    job::{run_to_end, Job},
    proof::{Action, ActionsJob, AttachOptionsJob, InferenceLimits, NoAttachment},
};

fn recorded_actions(json: &str) -> Vec<Action> {
    let (_safe, actions): (bool, Vec<Action>) = serde_json::from_str(json).unwrap();
    actions
}

/// The actions followed by a merging of exchanges (which does not change what the recorded
/// actions mean, if it comes last).
fn with_merging(mut actions: Vec<Action>) -> Vec<Action> {
    actions.push(Action::MergeExchanges);
    actions
}

/// What performing the actions one after another gives, with the proofs in between.
fn performed(actions: &[Action]) -> Vec<(Action, Proof)> {
    let mut proof = Proof::default();
    let mut applied = vec![];
    for action in actions {
        if proof.update(action).unwrap() {
            applied.push((action.clone(), proof.clone()));
        }
    }
    applied
}

fn assert_same(left: &[(Action, Proof)], right: &[(Action, Proof)]) {
    assert_eq!(left.len(), right.len());
    for ((left_action, left_proof), (right_action, right_proof)) in left.iter().zip(right) {
        assert_eq!(left_action, right_action);
        assert_eq!(**left_proof, **right_proof);
    }
}

#[test]
fn replaying_in_steps_is_performing_the_actions() {
    let actions = with_merging(recorded_actions(include_str!(
        "examples/merge_contract_RI.json"
    )));
    assert!(actions.contains(&Action::MergeExchanges));
    let expected = performed(&actions);

    // One step at a time, looking at the progress in between.
    let mut job = ActionsJob::replay(Proof::default(), actions.clone(), "Replaying actions");
    let mut steps = 0;
    let mut last_done = 0;
    let outcome = loop {
        let progress = job.progress().unwrap();
        assert_eq!(progress.total, actions.len());
        assert!(progress.done >= last_done, "progress goes forward");
        last_done = progress.done;
        steps += 1;
        if let Some(outcome) = job.step() {
            break outcome;
        }
    };
    assert!(outcome.error.is_none());
    assert_same(&outcome.applied, &expected);
    // Merging a diagram takes more than one step, so there are more steps than actions.
    assert!(steps >= actions.len() + 2);

    // And without stopping.
    let outcome = run_to_end(ActionsJob::replay(
        Proof::default(),
        actions,
        "Replaying actions",
    ));
    assert_same(&outcome.applied, &expected);
}

#[test]
fn a_chain_is_cut_short_where_an_action_does_nothing() {
    let proof = Proof::default();
    // Nothing is in the workspace, so neither does anything.
    let outcome = run_to_end(ActionsJob::chain(
        proof,
        [Action::ClearWorkspace, Action::CreateGeneratorZero],
    ));
    assert!(outcome.applied.is_empty());
    assert!(outcome.error.is_none());

    let outcome = run_to_end(ActionsJob::chain(
        Proof::default(),
        [Action::CreateGeneratorZero, Action::ClearWorkspace],
    ));
    // The first does something and the second nothing, as there is no workspace.
    assert_eq!(outcome.applied.len(), 1);
}

#[test]
fn the_proof_a_job_was_made_from_is_left_alone() {
    let actions = recorded_actions(include_str!("examples/associator.json"));
    let proof = Proof::default();
    let before = proof.clone();
    let mut job = ActionsJob::replay(proof.clone(), actions, "Replaying actions");
    // Dropped before it is done.
    for _ in 0..10 {
        assert!(job.step().is_none());
    }
    drop(job);
    assert_eq!(*proof, *before);
}

#[test]
fn an_error_keeps_what_was_done_before_it() {
    // Importing nonsense is an error, and what was done before it stands.
    let outcome = run_to_end(ActionsJob::replay(
        Proof::default(),
        [
            Action::CreateGeneratorZero,
            Action::ImportProof(vec![1, 2, 3].into()),
            Action::CreateGeneratorZero,
        ],
        "Replaying",
    ));
    assert_eq!(outcome.applied.len(), 1);
    assert!(outcome.error.is_some());
}

#[test]
fn attach_options_need_a_workspace() {
    let proof = Proof::default();
    assert_eq!(
        AttachOptionsJob::new(&proof, &[], false, InferenceLimits::default(), None).err(),
        Some(NoAttachment::NoWorkspace)
    );
}

/// Replays the actions up to the point of the first attachment, to be attached to again.
#[test]
fn attach_options_are_found_a_generator_at_a_time() {
    let mut actions = recorded_actions(include_str!("examples/associator.json"));
    let first_attach = actions
        .iter()
        .position(|a| matches!(a, Action::Attach(_)))
        .unwrap();
    actions.truncate(first_attach);
    let mut proof = Proof::default();
    for action in &actions {
        proof.update(action).unwrap();
    }
    // Something is in the workspace to attach to.
    assert!(proof.workspace.is_some());

    let point = [SliceIndex::Boundary(Boundary::Target)];
    let mut job =
        AttachOptionsJob::new(&proof, &point, true, InferenceLimits::default(), None).unwrap();
    let total = job.progress().unwrap().total;
    // At least a step for each generator.
    assert!(total >= proof.signature.iter().count());
    let mut steps = 0;
    let options = loop {
        steps += 1;
        let progress = job.progress().unwrap();
        assert!(progress.done <= progress.total);
        if let Some(outcome) = job.step() {
            break outcome.options;
        }
    };
    assert!(steps >= total);
    assert!(!options.is_empty());

    // Every option is a way that its generator can be attached.
    for option in &options {
        let mut attached = proof.clone();
        assert!(attached.update(&Action::Attach(option.clone())).unwrap());
    }
}
