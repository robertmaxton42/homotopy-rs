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
    use homotopy_common::idx::Idx as _;
    use homotopy_model::{
        history,
        proof::{SignatureEdit, SignatureItem},
    };
    // Whatever updates the history is refused, and so is what is expensive in itself.
    let refused = [
        Action::Proof(proof::Action::CreateGeneratorZero),
        Action::Proof(proof::Action::AscendSlice(1)),
        Action::Proof(proof::Action::SelectGenerator(Generator::new(0, 0))),
        Action::Proof(proof::Action::EditSignature(SignatureEdit::NewFolder(
            homotopy_common::tree::Node::new(0),
        ))),
        Action::History(history::Action::Move(history::Direction::Linear(
            homotopy_core::Direction::Backward,
        ))),
        Action::History(history::Action::Move(history::Direction::Linear(
            homotopy_core::Direction::Forward,
        ))),
        Action::ImportActions(SerializedData(vec![])),
        Action::ExportProof,
        Action::Select(0),
        Action::SelectPoint(vec![], false, InferenceLimits::default()),
        Action::SelectCell(vec![], false, InferenceLimits::default()),
        Action::SearchEverything,
        Action::Draft(proof::DraftEdit::Infer(Generator::new(0, 0))),
    ];
    for action in refused {
        assert!(!action.allowed_while_busy(), "{action:?}");
    }
    // What only changes what is shown, or what is picked or highlighted, is allowed.
    let node = homotopy_common::tree::Node::new(0);
    for action in [
        Action::HighlightAttachment(None),
        Action::HighlightSlice(None),
        Action::Help,
        Action::Arm(None),
        Action::ClearSelections,
        Action::ExportActions,
        Action::Proof(proof::Action::EditSignature(SignatureEdit::ToggleFolder(
            node,
        ))),
        Action::Proof(proof::Action::EditSignature(
            SignatureEdit::ToggleInstances(node),
        )),
    ] {
        assert!(action.allowed_while_busy(), "{action:?}");
    }

    // Opening a folder is not a step of the history: nothing is added to it, undo does not
    // undo it, and what a task computed from the project as it was is not in its way.
    let mut state = State::default();
    state
        .update(Action::Proof(proof::Action::EditSignature(
            SignatureEdit::NewFolder(state.proof().signature.as_tree().root()),
        )))
        .unwrap();
    let tree = state.proof().signature.as_tree();
    let folder = tree
        .iter()
        .find_map(|(node, data)| {
            (node != tree.root() && matches!(data.inner(), SignatureItem::Folder(_)))
                .then_some(node)
        })
        .expect("a folder was made");
    let open = |state: &State| {
        state.proof().signature.as_tree().with(
            folder,
            |data| matches!(data.inner(), SignatureItem::Folder(info) if info.open),
        )
    };
    let (position, actions) = (state.history.position(), state.history.get_actions());
    let before = open(&state);
    assert!(
        !state
            .update(Action::Proof(proof::Action::EditSignature(
                SignatureEdit::ToggleFolder(folder),
            )))
            .unwrap(),
        "toggling is not a change to the project"
    );
    assert_ne!(open(&state), before);
    assert_eq!(state.history.position(), position);
    assert_eq!(state.history.get_actions(), actions);
    // Undoing goes back past the folder being made, not to before it was opened.
    state
        .update(Action::History(history::Action::Move(
            history::Direction::Linear(homotopy_core::Direction::Backward),
        )))
        .unwrap();
    state
        .update(Action::History(history::Action::Move(
            history::Direction::Linear(homotopy_core::Direction::Forward),
        )))
        .unwrap();
    assert_ne!(open(&state), before, "the toggle was not undone");
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

#[test]
fn attaching_one_generator_gives_the_options_for_that_generator_and_picking_one_is_not_the_project()
{
    use homotopy_web::model::{ModelError, Selectables};

    let actions = recorded_actions(ASSOCIATOR);
    let first_attach = actions
        .iter()
        .position(|a| matches!(a, proof::Action::Attach(_)))
        .unwrap();
    let point = vec![SliceIndex::Boundary(Boundary::Target)];
    let limits = InferenceLimits::default();

    // What a click finds, for all the generators.
    let mut whole = state_before(&actions, first_attach);
    run_in_steps(&mut whole, Action::SelectPoint(point.clone(), true, limits));
    let Some(Selectables::Attach(all)) = whole.options.clone() else {
        panic!("several ways to attach at the point");
    };
    let generators: std::collections::BTreeSet<_> = all
        .iter()
        .map(|option| (option.generator.id, option.generator.dimension))
        .collect();
    assert!(
        generators.len() > 1,
        "more than one generator can be attached there"
    );

    for &(id, dimension) in &generators {
        let generator = Generator::new(id, dimension);
        let expected: Vec<_> = all
            .iter()
            .filter(|option| option.generator == generator)
            .cloned()
            .collect();

        // Picking the generator is not changing the project, and the attach starts by putting it
        // down.
        let mut state = state_before(&actions, first_attach);
        let before = summary(&state);
        assert!(matches!(
            state.start(Action::Arm(Some(generator))).unwrap(),
            Started::Done(false)
        ));
        assert_eq!(state.armed, Some(generator));
        assert_eq!(summary(&state), before);
        let Started::Running(mut task) = state
            .start(Action::AttachGenerator(
                generator,
                point.clone(),
                true,
                limits,
            ))
            .unwrap()
        else {
            panic!("finding what to attach is a task");
        };
        assert_eq!(state.armed, None);
        let output = loop {
            if let Some(output) = task.step() {
                break output;
            }
        };
        state.finish(output).unwrap();
        match expected.len() {
            // (Nothing is attached for none: that is an error for the app to say.)
            0 | 1 => assert!(state.options.is_none()),
            _ => assert_eq!(state.options, Some(Selectables::Attach(expected))),
        }
    }

    // Esc, or whatever clears what is selected, puts the picked generator down.
    let mut state = state_before(&actions, first_attach);
    let generator = Generator::new(0, 0);
    state.update(Action::Arm(Some(generator))).unwrap();
    assert_eq!(state.armed, Some(generator));
    state.update(Action::ClearSelections).unwrap();
    assert_eq!(state.armed, None);
    state.update(Action::Arm(Some(generator))).unwrap();
    state.update(Action::Arm(None)).unwrap();
    assert_eq!(state.armed, None);

    // A generator that is not in the signature is nothing to attach.
    let stranger = Generator::new(9999, 1);
    assert!(matches!(
        state.start(Action::AttachGenerator(
            stranger,
            point.clone(),
            true,
            limits
        )),
        Err(ModelError::NoAttachment)
    ));

    // The search is heavy, as a click is, and refused while another is running.
    let attach = Action::AttachGenerator(generator, point, true, limits);
    assert_eq!(attach.heavy_label(), Some("Searching for attachments"));
    assert!(!attach.allowed_while_busy());
}

/// A state in which a click lands on one cell, which has a definition (one is made for it: a
/// generator one dimension up from the cell, to itself), and where more than the definition
/// attaches; with the point of the click and the cell.
fn on_a_defined_cell() -> (State, Vec<SliceIndex>, Generator) {
    use homotopy_core::{signature::Signature as _, DiagramN, Height};
    for json in [ASSOCIATOR, SYMMETRIC_MONOIDAL, MERGE_CONTRACT] {
        let actions = recorded_actions(json);
        let state = state_before(&actions, actions.len());
        let Some(workspace) = state.proof().workspace.clone() else {
            continue;
        };
        let Ok(diagram) = DiagramN::try_from(workspace.diagram.clone()) else {
            continue;
        };
        // The vertices of the first two coordinates.
        let mut points = vec![];
        for i in 0..diagram.size() {
            let Some(Diagram::DiagramN(slice)) = diagram.slice(Height::Singular(i)) else {
                continue;
            };
            for j in 0..slice.size() {
                points.push(vec![
                    SliceIndex::Interior(Height::Singular(i)),
                    SliceIndex::Interior(Height::Singular(j)),
                ]);
            }
        }
        for point in points {
            let heights: Vec<_> = point
                .iter()
                .map(|index| match index {
                    SliceIndex::Interior(height) => *height,
                    SliceIndex::Boundary(_) => unreachable!(),
                })
                .collect();
            let Some(cell) = proof::cell_at(&workspace.diagram, &heights) else {
                continue;
            };
            let mut on = state.clone();
            let info = on.proof().signature.generator_info(cell).unwrap().clone();
            on.history.amend(|proof| {
                proof
                    .signature
                    .create_generator(
                        info.diagram.clone(),
                        info.diagram.clone(),
                        "definition",
                        Invertibility::Directed,
                    )
                    .unwrap();
            });
            let limits = InferenceLimits::UNLIMITED;
            let Some(job) = proof::DefinitionsJob::new(on.proof(), &point, true, limits) else {
                continue;
            };
            let defined = homotopy_model::job::run_to_end(job).options.len();
            let all = proof::AttachOptionsJob::new(on.proof(), &point, true, limits, None)
                .map(homotopy_model::job::run_to_end)
                .map_or(0, |outcome| outcome.options.len());
            // The definition attaches there, and so does more than it (else the click would
            // attach it at once).
            if defined >= 1 && all > defined {
                return (on, point, cell);
            }
        }
    }
    panic!("an example has a cell with a definition");
}

#[test]
fn a_click_on_a_cell_offers_its_definitions_first_and_everything_else_on_asking() {
    use homotopy_web::model::{Definitions, Selectables};
    let (state, point, cell) = on_a_defined_cell();
    let limits = InferenceLimits::UNLIMITED;
    let before = summary(&state);

    // What a click that looks at everything finds.
    let mut whole = state.clone();
    run_in_steps(&mut whole, Action::SelectPoint(point.clone(), true, limits));
    let all = match whole.options.clone() {
        Some(Selectables::Attach(all)) => all,
        _ => panic!("several ways to attach at the point"),
    };

    // The click with definitions first: a task of its own, that leaves the project alone, and
    // offers the definitions, without having searched for the rest.
    let mut clicked = state.clone();
    let Started::Running(task) = clicked
        .start(Action::SelectCell(point.clone(), true, limits))
        .unwrap()
    else {
        panic!("looking for definitions is a task");
    };
    assert!(matches!(task, Task::Definitions { .. }));
    assert_eq!(task.label(), "Looking for definitions");
    assert_eq!(summary(&clicked), before);
    let mut task = task;
    let output = loop {
        if let Some(output) = task.step() {
            break output;
        }
    };
    assert!(matches!(
        clicked.finish(output).unwrap(),
        Started::Done(true)
    ));
    let Some(Selectables::Attach(defined)) = clicked.options.clone() else {
        panic!("the definitions are offered");
    };
    let Some(Definitions {
        of,
        count,
        more: Some(_),
        ..
    }) = clicked.definitions.clone()
    else {
        panic!("they are marked, with the rest of the search to do");
    };
    assert_eq!(of, cell);
    assert_eq!(count, defined.len());
    assert!(count >= 1);
    assert_eq!(summary(&clicked), before);
    // Each is among what the whole search finds (and is for a generator that defines the cell).
    let definitions = proof::definitions_of(&clicked.proof().signature, cell);
    for option in &defined {
        assert!(all.contains(option), "{option:?}");
        assert!(definitions.contains(&option.generator));
    }
    assert!(all.len() > defined.len(), "there is more to search for");

    // Searching for everything keeps them at the top and adds what the whole search finds.
    let mut more = clicked.clone();
    let Started::Running(task @ Task::AttachRest { .. }) =
        more.start(Action::SearchEverything).unwrap()
    else {
        panic!("the rest of the search is a task");
    };
    assert_eq!(summary(&more), before);
    let mut task = task;
    let output = loop {
        if let Some(output) = task.step() {
            break output;
        }
    };
    more.finish(output).unwrap();
    let Some(Selectables::Attach(listed)) = more.options.clone() else {
        panic!("all the options are listed");
    };
    assert_eq!(&listed[..count], &defined[..]);
    assert_eq!(listed.len(), all.len());
    for option in &all {
        assert!(listed.contains(option));
    }
    let done = more.definitions.clone().unwrap();
    assert_eq!((done.count, done.more), (count, None));
    // (And there is nothing more to search for.)
    assert!(matches!(
        more.start(Action::SearchEverything).unwrap(),
        Started::Done(false)
    ));

    // Esc puts it all away.
    clicked.update(Action::ClearSelections).unwrap();
    assert!(clicked.options.is_none() && clicked.definitions.is_none());

    // A click that is not on one cell is searched for as it always was.
    let mut elsewhere = state.clone();
    let regular = vec![SliceIndex::Interior(homotopy_core::Height::Regular(0))];
    assert!(matches!(
        elsewhere
            .start(Action::SelectCell(regular.clone(), false, limits))
            .unwrap(),
        Started::Running(Task::AttachOptions(_))
    ));
    assert!(matches!(
        elsewhere
            .start(Action::SelectCell(vec![], false, limits))
            .unwrap(),
        Started::Running(Task::AttachOptions(_))
    ));
    // The same as the click: whatever it finds.
    let mut same = state.clone();
    run_in_steps(
        &mut same,
        Action::SelectCell(regular.clone(), false, limits),
    );
    let mut plain = state.clone();
    run_in_steps(&mut plain, Action::SelectPoint(regular, false, limits));
    assert_eq!(same.options, plain.options);
    assert!(same.definitions.is_none());

    // Looking for definitions is heavy, and refused while another job runs.
    let click = Action::SelectCell(point, true, limits);
    assert_eq!(click.heavy_label(), Some("Searching for attachments"));
    assert!(!click.allowed_while_busy());
}

/// A state and a point where several ways to attach are found (at least `most`, if `exactly` is
/// not set; exactly that many if it is), and the options the whole search finds there, in order.
fn where_options_are_found(
    wanted: impl Fn(usize) -> bool,
) -> (State, Vec<SliceIndex>, Vec<proof::AttachOption>) {
    use homotopy_core::DiagramN;
    for json in [ASSOCIATOR, SYMMETRIC_MONOIDAL, MERGE_CONTRACT] {
        let actions = recorded_actions(json);
        let mut state = State::default();
        for (i, action) in actions.iter().enumerate() {
            if state.update(Action::Proof(action.clone())).is_err() {
                continue;
            }
            if i % 4 != 0 || state.proof().workspace.is_none() {
                continue;
            }
            let diagram = state.proof().workspace.as_ref().unwrap().diagram.clone();
            let mut points = vec![
                vec![],
                vec![SliceIndex::Boundary(Boundary::Source)],
                vec![SliceIndex::Boundary(Boundary::Target)],
            ];
            if let Ok(d) = DiagramN::try_from(diagram) {
                for h in 0..=2 * d.size() {
                    points.push(vec![SliceIndex::Interior(homotopy_core::Height::from(h))]);
                }
            }
            for point in points {
                let limits = InferenceLimits::UNLIMITED;
                let Ok(job) =
                    proof::AttachOptionsJob::new(state.proof(), &point, true, limits, None)
                else {
                    continue;
                };
                let options = homotopy_model::job::run_to_end(job).options;
                if wanted(options.len()) {
                    return (state, point, options);
                }
            }
        }
    }
    panic!("an example has a point with the wanted number of options");
}

/// Starts a click that looks for everything, and steps the search until options have been found
/// (and put in the list) while it is not over: the task, which has not been finished.
fn click_until_something_is_listed(
    state: &mut State,
    point: &[SliceIndex],
) -> (Task, Vec<proof::AttachOption>) {
    let Started::Running(mut task) = state
        .start(Action::SelectPoint(
            point.to_vec(),
            true,
            InferenceLimits::UNLIMITED,
        ))
        .unwrap()
    else {
        panic!("finding what to attach is a task");
    };
    let mut shown: Vec<proof::AttachOption> = vec![];
    loop {
        assert!(
            task.step().is_none(),
            "the search was over before it listed anything"
        );
        if state.found(task.take_found()) {
            let Some(Selectables::Attach(now)) = state.options.clone() else {
                panic!("the options found are listed");
            };
            // What is listed is only added to.
            let mut rest = now.iter();
            assert!(shown.iter().all(|option| rest.any(|other| other == option)));
            shown = now;
            return (task, shown);
        }
    }
}

use homotopy_web::model::{Listing, Selectables};

#[test]
fn options_are_listed_as_a_search_finds_them_in_the_place_they_will_have() {
    let (state, point, whole) = where_options_are_found(|n| n >= 3);
    let mut state = state;
    let before = summary(&state);
    assert_eq!(state.listing, Listing::Complete);

    let (mut task, mut shown) = click_until_something_is_listed(&mut state, &point);
    // Listed, in the list that is going to be, while the search goes on; the project is not touched.
    assert_eq!(state.listing, Listing::Searching);
    assert!(state.is_searching());
    assert!(shown.len() <= whole.len());
    assert_eq!(summary(&state), before);

    // The rest arrives without moving what is listed, and the list is the search's in the end.
    let output = loop {
        if let Some(output) = task.step() {
            break output;
        }
        if state.found(task.take_found()) {
            let Some(Selectables::Attach(now)) = state.options.clone() else {
                panic!("listed");
            };
            let mut rest = now.iter();
            assert!(shown.iter().all(|option| rest.any(|other| other == option)));
            shown = now;
        }
        assert_eq!(summary(&state), before);
    };
    state.found(task.take_found());
    assert_eq!(shown, whole);
    assert_eq!(state.options, Some(Selectables::Attach(whole.clone())));
    state.finish(output).unwrap();
    assert_eq!(state.listing, Listing::Complete);
    assert_eq!(state.options, Some(Selectables::Attach(whole)));
    assert_eq!(summary(&state), before);
}

#[test]
fn picking_an_option_while_the_search_goes_on_ends_it_and_attaches_exactly_that_option() {
    let (state, point, whole) = where_options_are_found(|n| n >= 3);
    let mut state = state;
    let before = summary(&state);

    let (task, shown) = click_until_something_is_listed(&mut state, &point);
    // It is allowed while the search runs (the app ends the search first).
    let picked = shown[0].clone();
    assert!(whole.contains(&picked));
    let pick = Action::PickStreamed(picked.clone());
    assert!(pick.allowed_while_busy());
    assert!(Action::StopSearch.allowed_while_busy());
    // The numbers are those of the final list, which is not known: the keys are refused.
    assert!(!Action::Select(0).allowed_while_busy());
    // Cancelling the search: nothing of it was committed.
    assert!(task.is_attach_search());
    drop(task);
    assert_eq!(summary(&state), before);

    run_in_steps(&mut state, pick);

    // What is attached is that option, as clicking it in a finished list would.
    let mut straight = state_before_clicking(&point, &before);
    straight
        .update(Action::Proof(proof::Action::Attach(picked.clone())))
        .unwrap();
    assert_eq!(summary(&state), summary(&straight));
    assert_eq!(
        state.history.get_actions().last(),
        Some(&proof::Action::Attach(picked))
    );
    assert_eq!(state.history.get_actions().len(), before.0.len() + 1);
    // And the list is put away.
    assert!(state.options.is_none());
    assert_eq!(state.listing, Listing::Complete);
    assert!(!state.is_searching());
}

/// The state with the actions of the summary done again (the project as it was).
fn state_before_clicking(
    _point: &[SliceIndex],
    before: &(Vec<proof::Action>, bool, bool, proof::ProofState),
) -> State {
    let mut state = State::default();
    for action in &before.0 {
        state.update(Action::Proof(action.clone())).unwrap();
    }
    assert_eq!(summary(&state), *before);
    state
}

#[test]
fn stopping_a_search_keeps_the_options_it_found_and_leaves_the_project_alone() {
    let (state, point, whole) = where_options_are_found(|n| n >= 3);
    let mut state = state;
    let before = summary(&state);

    let (task, shown) = click_until_something_is_listed(&mut state, &point);
    drop(task);
    assert_eq!(state.stopped(), Some(shown.len()));
    // They are still listed, marked as what a search that was stopped had found.
    assert_eq!(state.options, Some(Selectables::Attach(shown.clone())));
    assert_eq!(state.listing, Listing::Stopped);
    assert_eq!(summary(&state), before);
    // Each is among what the whole search finds, in its order.
    let mut rest = whole.iter();
    assert!(shown.iter().all(|option| rest.any(|other| other == option)));
    // They are listed as any list is: picking one attaches it, and the keys index them.
    assert!(matches!(Action::Select(0).allowed_while_busy(), false));
    let mut picked = state.clone();
    picked.update(Action::Select(0)).unwrap();
    assert_eq!(
        picked.history.get_actions().last(),
        Some(&proof::Action::Attach(shown[0].clone()))
    );
    // Esc puts them away.
    state.update(Action::ClearSelections).unwrap();
    assert!(state.options.is_none());
    assert_eq!(state.listing, Listing::Complete);
    assert_eq!(summary(&state), before);

    // A search that had found nothing leaves the list as it was.
    let mut quiet = State::default();
    assert_eq!(quiet.stopped(), None);
}

#[test]
fn one_option_is_attached_at_once_only_when_the_search_ends_with_it() {
    let (state, point, whole) = where_options_are_found(|n| n == 1);
    let mut state = state;
    let before = summary(&state);
    let (mut task, shown) = click_until_something_is_listed(&mut state, &point);
    // It is listed when it is found, and not attached.
    assert_eq!(shown, whole);
    assert_eq!(state.listing, Listing::Searching);
    assert_eq!(summary(&state), before);
    let output = loop {
        if let Some(output) = task.step() {
            break output;
        }
    };
    // At the end of the search it is the only one: attached.
    let mut started = state.finish(output).unwrap();
    while let Started::Running(mut task) = started {
        let output = loop {
            if let Some(output) = task.step() {
                break output;
            }
        };
        started = state.finish(output).unwrap();
    }
    assert_eq!(state.history.get_actions().len(), before.0.len() + 1);
    assert!(state.options.is_none());
    assert_eq!(state.listing, Listing::Complete);
}

#[test]
fn the_rest_of_what_attaches_is_listed_below_the_definitions_as_it_is_found() {
    let (state, point, _cell) = on_a_defined_cell();
    let limits = InferenceLimits::UNLIMITED;
    let mut clicked = state.clone();
    run_in_steps(
        &mut clicked,
        Action::SelectCell(point.clone(), true, limits),
    );
    let Some(Selectables::Attach(defined)) = clicked.options.clone() else {
        panic!("the definitions are offered");
    };
    let count = defined.len();
    let before = summary(&clicked);

    let mut more = clicked.clone();
    let Started::Running(mut task @ Task::AttachRest { .. }) =
        more.start(Action::SearchEverything).unwrap()
    else {
        panic!("the rest of the search is a task");
    };
    // The definitions stay as they are while it searches.
    assert_eq!(more.listing, Listing::Searching);
    assert_eq!(more.options, Some(Selectables::Attach(defined.clone())));
    let mut arrivals = 0;
    let output = loop {
        let output = task.step();
        if more.found(task.take_found()) {
            arrivals += 1;
            let Some(Selectables::Attach(now)) = more.options.clone() else {
                panic!("listed");
            };
            assert_eq!(&now[..count], &defined[..], "the definitions stay on top");
            assert!(more.definitions.as_ref().unwrap().more.is_some());
            // What is below is not a definition again.
            assert!(now[count..].iter().all(|option| !defined.contains(option)));
            assert_eq!(summary(&more), before);
            if arrivals == 1 {
                // Stopping here keeps both, and the way to search again.
                let mut stopped = more.clone();
                assert!(stopped.stopped().unwrap() >= 1);
                assert_eq!(stopped.listing, Listing::Stopped);
                assert_eq!(&stopped_options(&stopped)[..count], &defined[..]);
                assert!(stopped.definitions.as_ref().unwrap().more.is_some());
            }
        }
        if let Some(output) = output {
            break output;
        }
    };
    assert!(arrivals >= 1);
    let streamed = stopped_options(&more);
    more.finish(output).unwrap();
    // The end of the search gives the same list as the one that was built as it went.
    assert_eq!(stopped_options(&more), streamed);
    assert_eq!(more.listing, Listing::Complete);
    assert_eq!(more.definitions.as_ref().unwrap().more, None);

    // The same as searching without streaming it.
    let mut plain = clicked.clone();
    run_in_steps(&mut plain, Action::SearchEverything);
    assert_eq!(plain.options, more.options);
}

fn stopped_options(state: &State) -> Vec<proof::AttachOption> {
    match &state.options {
        Some(Selectables::Attach(options)) => options.clone(),
        _ => panic!("options are listed"),
    }
}
