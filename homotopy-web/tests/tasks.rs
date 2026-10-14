//! Tasks are what the app runs in the background, a slice at a time, and may cancel: they have
//! to give what doing the action straight away does, and leave the state alone until finished.

use homotopy_core::{signature::Invertibility, Boundary, Diagram, Generator, SliceIndex};
use homotopy_model::{
    history::UndoState,
    job::{Job, Progress},
    proof::{self, InferenceLimits, SerializedData},
    remote::{Answer, Executor, Remote},
    serialize,
};
use homotopy_web::model::{Action, Started, State, Task};

const ASSOCIATOR: &str = include_str!("../../homotopy-model/tests/examples/associator.json");
const SYMMETRIC_MONOIDAL: &str =
    include_str!("../../homotopy-model/tests/examples/symmetric_monoidal_abelianize.json");
const MERGE_CONTRACT: &str =
    include_str!("../../homotopy-model/tests/examples/merge_contract_RI.json");

fn recorded_actions(json: &str) -> Vec<proof::Action> {
    let (_safe, actions): (bool, Vec<proof::Action>) = serde_json::from_str(json).unwrap();
    actions
}

/// What of a state can be compared.
fn summary(state: &State) -> (Vec<proof::Action>, bool, bool, proof::ProofState) {
    (
        state.history.get_actions(),
        state.proof().can_undo(),
        state.proof().can_redo(),
        (***state.proof()).clone(),
    )
}

/// The state after performing the actions up to (not including) the one at `until`.
fn state_before(actions: &[proof::Action], until: usize) -> State {
    let mut state = State::default();
    for action in &actions[..until] {
        state.update(Action::Proof(action.clone())).unwrap();
    }
    state
}

/// Starts an action, and runs the task it needs (if any) one step at a time.
fn run_in_steps(state: &mut State, action: Action) {
    let mut started = state.start(action).unwrap();
    loop {
        match started {
            Started::Done(_) => return,
            Started::Running(mut task) => {
                let output = loop {
                    if let Some(output) = task.step() {
                        break output;
                    }
                };
                started = state.finish(output).unwrap();
            }
        }
    }
}

#[test]
fn doing_actions_in_steps_is_doing_them() {
    let mut straight = State::default();
    let mut in_steps = State::default();
    for action in recorded_actions(MERGE_CONTRACT) {
        straight.update(Action::Proof(action.clone())).unwrap();
        run_in_steps(&mut in_steps, Action::Proof(action));
    }
    assert_eq!(summary(&straight), summary(&in_steps));
    assert!(straight.proof().workspace.is_some());
}

#[test]
fn the_state_is_left_alone_until_a_task_is_finished_and_forgotten_if_it_is_not() {
    let actions = recorded_actions(MERGE_CONTRACT);
    let last_homotopy = actions
        .iter()
        .rposition(|a| matches!(a, proof::Action::Homotopy(_)))
        .unwrap();
    let mut state = state_before(&actions, last_homotopy);
    let before = summary(&state);

    let Started::Running(mut task) = state
        .start(Action::Proof(actions[last_homotopy].clone()))
        .unwrap()
    else {
        panic!("a homotopy is a task");
    };
    assert!(task.label() == "Expanding" || task.label() == "Contracting");
    let output = loop {
        if let Some(output) = task.step() {
            break output;
        }
        // Not touched while the task works, nor if it is dropped before it is done.
        assert_eq!(summary(&state), before);
    };
    assert_eq!(summary(&state), before);

    // The output of a task that was done is committed in one go.
    assert!(matches!(state.finish(output), Ok(Started::Done(true))));
    assert_eq!(state.history.get_actions().len(), before.0.len() + 1);
}

#[test]
fn what_the_crash_log_knows_is_only_changed_by_what_is_committed() {
    // Importing a project replaces what the crash log restarts from.
    let actions = recorded_actions(ASSOCIATOR);
    let built = state_before(&actions, actions.len());
    let data = serialize::serialize(
        built.proof().signature.clone(),
        built.proof().workspace.clone(),
        built.proof().metadata.clone(),
    );
    let import = proof::Action::ImportProof(SerializedData(data.clone()));

    let mut state = State::default();
    let log = homotopy_web::crash_log_summary();
    let Started::Running(mut task) = state.start(Action::Proof(import.clone())).unwrap() else {
        panic!("importing is a task");
    };
    assert_eq!(task.label(), "Importing project");
    let output = loop {
        if let Some(output) = task.step() {
            break output;
        }
    };
    // Done, but not committed (nor dropped: it is cancelled just as well in between).
    assert_eq!(homotopy_web::crash_log_summary(), log);
    assert!(state.proof().workspace.is_none());
    drop(output);
    assert_eq!(homotopy_web::crash_log_summary(), log);

    run_in_steps(&mut state, Action::Proof(import));
    assert_eq!(homotopy_web::crash_log_summary().1, data.len());
    assert_eq!(
        state.proof().signature.iter().count(),
        built.proof().signature.iter().count()
    );
}

#[test]
fn replaying_actions_is_a_task_with_progress_that_adds_to_the_history() {
    let actions = recorded_actions(ASSOCIATOR);
    let mut state = State::default();
    let Started::Running(mut task) = state
        .start(Action::ImportActions(SerializedData(
            ASSOCIATOR.as_bytes().to_vec(),
        )))
        .unwrap()
    else {
        panic!("replaying is a task");
    };
    assert_eq!(task.label(), "Replaying actions");
    assert_eq!(
        task.progress(),
        Some(Progress {
            done: 0,
            total: actions.len()
        })
    );
    let output = loop {
        if let Some(output) = task.step() {
            break output;
        }
    };
    // Nothing is committed until it is done.
    assert!(state.history.get_actions().is_empty());
    assert!(matches!(state.finish(output), Ok(Started::Done(true))));

    // The same as performing the actions one by one.
    let straight = state_before(&actions, actions.len());
    assert_eq!(summary(&state), summary(&straight));
}

#[test]
fn finding_attachments_is_a_task_that_gives_the_options_when_done() {
    let actions = recorded_actions(ASSOCIATOR);
    let first_attach = actions
        .iter()
        .position(|a| matches!(a, proof::Action::Attach(_)))
        .unwrap();
    let point = vec![SliceIndex::Boundary(Boundary::Target)];

    let mut state = state_before(&actions, first_attach);
    let before = summary(&state);
    let Started::Running(mut task) = state
        .start(Action::SelectPoint(
            point.clone(),
            true,
            InferenceLimits::default(),
        ))
        .unwrap()
    else {
        panic!("finding what to attach is a task");
    };
    let total = task.progress().unwrap().total;
    assert!(total > 0);
    let mut steps = 0;
    let output = loop {
        steps += 1;
        if let Some(output) = task.step() {
            break output;
        }
    };
    assert!(steps >= total);
    assert!(state.options.is_none());
    state.finish(output).unwrap();
    // Either the options to choose from, or the only one attached.
    assert!(state.options.is_some() || state.history.get_actions().len() == before.0.len() + 1);

    let mut straight = state_before(&actions, first_attach);
    straight
        .update(Action::SelectPoint(point, true, InferenceLimits::default()))
        .unwrap();
    assert_eq!(state.options, straight.options);
    assert_eq!(summary(&state), summary(&straight));
}

#[test]
fn what_may_be_done_while_busy_does_not_touch_the_project() {
    use homotopy_model::history;
    let refused = [
        Action::Proof(proof::Action::CreateGeneratorZero),
        Action::Proof(proof::Action::AscendSlice(1)),
        Action::History(history::Action::Move(history::Direction::Linear(
            homotopy_core::Direction::Backward,
        ))),
        Action::History(history::Action::Move(history::Direction::Linear(
            homotopy_core::Direction::Forward,
        ))),
        Action::ImportActions(SerializedData(vec![])),
        Action::ExportProof,
        Action::ExportActions,
        Action::Select(0),
        Action::SelectPoint(vec![], false, InferenceLimits::default()),
        Action::ClearSelections,
        Action::Draft(proof::DraftEdit::Infer(Generator::new(0, 0))),
    ];
    for action in refused {
        assert!(!action.allowed_while_busy(), "{action:?}");
    }
    for action in [
        Action::HighlightAttachment(None),
        Action::HighlightSlice(None),
        Action::Help,
    ] {
        assert!(action.allowed_while_busy(), "{action:?}");
    }
}

#[test]
fn the_heavy_actions_are_labelled_and_the_light_ones_are_not() {
    let homotopy = recorded_actions(MERGE_CONTRACT)
        .into_iter()
        .find(|a| matches!(a, proof::Action::Homotopy(_)))
        .unwrap();
    assert!(Action::Proof(homotopy).heavy_label().is_some());
    assert!(Action::Proof(proof::Action::MergeExchanges)
        .heavy_label()
        .is_some());
    assert!(
        Action::SelectPoint(vec![], false, InferenceLimits::default())
            .heavy_label()
            .is_some()
    );
    assert!(Action::ImportActions(SerializedData(vec![]))
        .heavy_label()
        .is_some());
    // Cheap, and often repeated quickly, such as while typing or stepping through slices.
    for action in [
        proof::Action::CreateGeneratorZero,
        proof::Action::AscendSlice(1),
        proof::Action::SwitchSlice(homotopy_core::Direction::Forward),
        proof::Action::EditSignature(proof::SignatureEdit::Edit(
            homotopy_common::tree::Node::default(),
            proof::SignatureItemEdit::Rename("x".into()),
        )),
    ] {
        assert!(Action::Proof(action).heavy_label().is_none());
    }
}

/// Does the task as a worker would: on a thread of its own that shares none of the diagrams with
/// this one, reached by bytes only. Returns `None` if the task is not one for a worker.
fn done_by_a_worker(state: &State, task: &Task) -> Option<homotopy_web::model::TaskOutput> {
    let (proof, work) = task.offer(&**state.proof())?;
    let mut remote = Remote::default();
    let request = remote.request(proof, work, 0, false);
    let response = std::thread::spawn(move || Executor::default().run(&request, &mut |_| {}))
        .join()
        .unwrap();
    match remote.response(&response).unwrap() {
        Answer::Done { output, .. } => Some(task.output_of(output).unwrap()),
        Answer::NeedsSignature => panic!("a new worker holds nothing, and was sent everything"),
    }
}

#[test]
fn a_task_done_by_a_worker_is_committed_as_if_done_here() {
    for json in [MERGE_CONTRACT, SYMMETRIC_MONOIDAL] {
        let actions = recorded_actions(json);
        let heavy = actions
            .iter()
            .rposition(|a| a.work_label().is_some() && *a != proof::Action::MergeExchanges)
            .unwrap();
        let state = state_before(&actions, heavy);

        let mut here = state.clone();
        here.update(Action::Proof(actions[heavy].clone())).unwrap();

        let mut there = state.clone();
        let Started::Running(task) = there.start(Action::Proof(actions[heavy].clone())).unwrap()
        else {
            panic!("a heavy action is a task");
        };
        let output = done_by_a_worker(&there, &task).expect("heavy actions are for the worker");
        // The state is as it was until the output is committed.
        assert_eq!(summary(&there), summary(&state));
        assert!(matches!(there.finish(output), Ok(Started::Done(true))));
        assert_eq!(summary(&there), summary(&here));
    }
}

#[test]
fn only_what_is_heavy_is_for_a_worker() {
    let actions = recorded_actions(MERGE_CONTRACT);
    // An action that does not take long is done in place.
    let mut state = State::default();
    let Started::Running(task) = state
        .start(Action::Proof(proof::Action::CreateGeneratorZero))
        .unwrap()
    else {
        panic!("actions are tasks");
    };
    assert!(task.offer(&**state.proof()).is_none());

    // A homotopy is not, and neither is replaying actions.
    let heavy = actions
        .iter()
        .rposition(|a| matches!(a, proof::Action::Homotopy(_)))
        .unwrap();
    let mut state = state_before(&actions, heavy);
    let Started::Running(task) = state.start(Action::Proof(actions[heavy].clone())).unwrap() else {
        panic!("a homotopy is a task");
    };
    assert!(task.offer(&**state.proof()).is_some());

    let mut state = State::default();
    let Started::Running(task) = state
        .start(Action::ImportActions(SerializedData(
            MERGE_CONTRACT.as_bytes().to_vec(),
        )))
        .unwrap()
    else {
        panic!("replaying is a task");
    };
    assert!(task.offer(&**state.proof()).is_some());

    // Importing a small project takes about as long as sending it would.
    let mut state = State::default();
    let Started::Running(task) = state
        .start(Action::Proof(proof::Action::ImportProof(SerializedData(
            vec![0; 1000],
        ))))
        .unwrap()
    else {
        panic!("importing is a task");
    };
    assert!(task.offer(&**state.proof()).is_none());
}

fn node_of(sig: &proof::Signature, generator: Generator) -> homotopy_common::tree::Node {
    sig.as_tree()
        .iter()
        .find(|(_, item)| {
            matches!(item.inner(), proof::SignatureItem::Item(info) if info.generator == generator)
        })
        .map(|(node, _)| node)
        .unwrap()
}

/// A schema `s` in the placeholder `η : F ⇒ G` of `F, G : x → x`, and `p : T ⇒ U`.
fn schema_state() -> (State, Generator, Generator, Diagram) {
    use homotopy_core::signature::Signature as _;
    use homotopy_model::proof::{SignatureEdit, SignatureItemEdit};

    let mut sig = proof::Signature::default();
    let x: Diagram = sig.create_generator_zero("x").into();
    let mut cell = |sig: &mut proof::Signature, s: Diagram, t: Diagram, name: &str| {
        sig.create_generator(s, t, name, Invertibility::Directed)
            .unwrap()
    };
    let f = cell(&mut sig, x.clone(), x.clone(), "F");
    let g = cell(&mut sig, x.clone(), x.clone(), "G");
    let t = cell(&mut sig, x.clone(), x.clone(), "T");
    let u = cell(&mut sig, x.clone(), x.clone(), "U");
    let ends = [f.clone(), g.clone()].map(|d| d.max_generator().generator);
    let eta = cell(&mut sig, f.into(), g.into(), "eta");
    let p = cell(&mut sig, t.into(), u.into(), "p");
    let s = cell(&mut sig, x.clone(), x, "s");
    let (eta, s) = (eta.max_generator().generator, s.max_generator().generator);
    let node = node_of(&sig, eta);
    sig.update(&SignatureEdit::Edit(
        node,
        SignatureItemEdit::MakePlaceholder(true),
    ))
    .unwrap();
    let node = node_of(&sig, s);
    sig.update(&SignatureEdit::Edit(
        node,
        SignatureItemEdit::SetDependsOn(vec![eta]),
    ))
    .unwrap();
    // `F` and `G` are mentioned by the placeholder, so they are placeholders as well.
    for generator in ends {
        let node = node_of(&sig, generator);
        sig.update(&SignatureEdit::Edit(
            node,
            SignatureItemEdit::MakePlaceholder(true),
        ))
        .unwrap();
    }
    let data = serialize::serialize(sig, None, Default::default());
    let mut state = State::default();
    state
        .update(Action::Proof(proof::Action::ImportProof(SerializedData(
            data,
        ))))
        .unwrap();
    (state, s, eta, p.into())
}

#[test]
fn inferring_what_a_replacement_determines_is_a_task_that_can_be_dropped() {
    let (mut state, schema, eta, p) = schema_state();
    let before = summary(&state);
    let edit = proof::DraftEdit::Fill(schema, eta, p.clone());
    assert!(Action::Draft(edit.clone()).heavy_label().is_some());

    // What is chosen is kept at once, and what follows comes with the task.
    let Started::Running(mut task) = state.start(Action::Draft(edit.clone())).unwrap() else {
        panic!("inferring is a task");
    };
    assert_eq!(task.label(), "Inferring replacements");
    assert!(!task.next_step_may_block());
    assert!(
        task.offer(state.proof()).is_none(),
        "it is done a step at a time, here"
    );
    assert_eq!(state.drafts.get(schema).get(eta), Some(&p));
    assert!(state.drafts.get(schema).needs_inference());
    assert!(task.step().is_none() || state.drafts.get(schema).needs_inference());
    // Cancelling is dropping it: the choice stays, nothing is inferred, the project is as it was.
    drop(task);
    let draft = state.drafts.get(schema);
    assert_eq!(draft.substitution(), vec![(eta, p.clone())]);
    assert!(draft.needs_inference());
    assert_eq!(summary(&state), before);

    // Asking again finishes it.
    run_in_steps(&mut state, Action::Draft(proof::DraftEdit::Infer(schema)));
    let draft = state.drafts.get(schema);
    assert_eq!(
        draft.substitution().len(),
        3,
        "the ends of `p` are inferred"
    );
    assert!(!draft.needs_inference());
    assert_eq!(
        summary(&state),
        before,
        "choosing is not changing the project"
    );

    // And doing it in one go is the same.
    let mut other = state.clone();
    other.drafts = Default::default();
    other.update(Action::Draft(edit)).unwrap();
    assert_eq!(other.drafts.get(schema), draft);
}
