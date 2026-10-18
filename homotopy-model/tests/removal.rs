//! What deleting a generator deletes: everything that mentions it, and everything that lists it
//! as an explicit dependency when it is a placeholder; not what lists an instance. And the
//! preview of it (`Signature::removal_cascade`), which is made from the same code.

use homotopy_core::{
    signature::{Invertibility, Signature as _},
    Boundary, Diagram, DiagramN, Generator,
};
use homotopy_model::{
    history::{proof_from_state, History, Proof},
    proof::{
        Action, ProofState, RemovalReason, Signature, SignatureEdit, SignatureItem,
        SignatureItemEdit,
    },
    serialize,
};

type Gens = Vec<Generator>;

fn sorted(mut gs: Gens) -> Gens {
    gs.sort_by_key(|g| (g.dimension, g.id));
    gs
}

fn generators(sig: &Signature) -> Gens {
    sorted(sig.iter().map(|info| info.generator).collect())
}

fn edit(sig: &mut Signature, g: Generator, edit: SignatureItemEdit) {
    let node = sig.find_node(g).unwrap();
    sig.update(&SignatureEdit::Edit(node, edit)).unwrap();
}

fn cell(sig: &mut Signature, source: Diagram, target: Diagram) -> (Generator, DiagramN) {
    let d = sig
        .create_generator(source, target, "g", Invertibility::Directed)
        .unwrap();
    (d.max_generator().generator, d)
}

/// `x`; `Q : x → x`; placeholders `P`, `P2 : x → x`; `u : Q ⇒ Q` that depends on `P` and
/// `w : Q ⇒ Q` that depends on `P2`, without mentioning them; `v : u ⇒ u`; `z : Q ⇒ Q` that
/// depends on nothing; `P3 : u ⇒ u`, a placeholder that mentions `u`, and `t : Q ⇒ Q` that
/// depends on `P3`; the schemas `e : P ⇒ P` and `A : e ≫ e ⇛ e`, with `A[Q/P]` (and `e[Q/P]`)
/// as instances; and `d : Q ⇒ Q` that lists the instance `e[Q/P]` as a dependency.
struct Setup {
    sig: Signature,
    p: Generator,
    p2: Generator,
    p3: Generator,
    q: Generator,
    u: Generator,
    v: Generator,
    w: Generator,
    z: Generator,
    t: Generator,
    e: Generator,
    a: Generator,
    a_instance: Generator,
    e_instance: Generator,
    d: Generator,
}

fn setup() -> Setup {
    let mut sig = Signature::default();
    let x = sig.create_generator_zero("x");
    let (q, qd) = cell(&mut sig, x.into(), x.into());
    let (p, pd) = cell(&mut sig, x.into(), x.into());
    let (p2, _) = cell(&mut sig, x.into(), x.into());
    edit(&mut sig, p, SignatureItemEdit::MakePlaceholder(true));
    edit(&mut sig, p2, SignatureItemEdit::MakePlaceholder(true));

    let (u, ud) = cell(&mut sig, qd.clone().into(), qd.clone().into());
    let (w, _) = cell(&mut sig, qd.clone().into(), qd.clone().into());
    let (z, _) = cell(&mut sig, qd.clone().into(), qd.clone().into());
    let (t, _) = cell(&mut sig, qd.clone().into(), qd.clone().into());
    let (v, _) = cell(&mut sig, ud.clone().into(), ud.clone().into());
    let (p3, _) = cell(&mut sig, ud.clone().into(), ud.into());
    edit(&mut sig, p3, SignatureItemEdit::MakePlaceholder(true));
    edit(&mut sig, u, SignatureItemEdit::SetDependsOn(vec![p]));
    edit(&mut sig, w, SignatureItemEdit::SetDependsOn(vec![p2]));
    edit(&mut sig, t, SignatureItemEdit::SetDependsOn(vec![p3]));

    let (e, ed) = cell(&mut sig, pd.clone().into(), pd.into());
    let ee = ed.attach(&ed, Boundary::Target, &[]).unwrap();
    let (a, _) = cell(&mut sig, ee.into(), ed.into());
    let a_instance = sig.instantiate(a, &[(p, qd.clone().into())]).unwrap();
    let e_instance = sig
        .iter()
        .find(|info| info.instance.as_ref().is_some_and(|i| i.schema == e))
        .unwrap()
        .generator;
    let (d, _) = cell(&mut sig, qd.clone().into(), qd.into());
    edit(
        &mut sig,
        d,
        SignatureItemEdit::SetDependsOn(vec![e_instance]),
    );
    Setup {
        sig,
        p,
        p2,
        p3,
        q,
        u,
        v,
        w,
        z,
        t,
        e,
        a,
        a_instance,
        e_instance,
        d,
    }
}

fn remove(sig: &Signature, g: Generator) -> Signature {
    let mut after = sig.clone();
    let node = after.find_node(g).unwrap();
    after.update(&SignatureEdit::Remove(node)).unwrap();
    after
}

/// The generators that disappear when `g` is deleted.
fn vanished(sig: &Signature, g: Generator) -> Gens {
    let after = generators(&remove(sig, g));
    generators(sig)
        .into_iter()
        .filter(|h| !after.contains(h))
        .collect()
}

#[test]
fn deleting_a_placeholder_deletes_what_lists_it_as_a_dependency() {
    let s = setup();
    assert_eq!(s.sig.parameters(s.u), vec![s.p]);
    let gone = vanished(&s.sig, s.p);
    // `u` does not mention `P`, but depends on it, and `v` mentions `u`; the schemas mention
    // it and the instances are under them.
    for g in [s.p, s.u, s.v, s.e, s.a, s.a_instance, s.e_instance] {
        assert!(gone.contains(&g), "{g:?} should go with P");
    }
    // Other things stay. (`d` lists an instance, which goes: that is no reason for it to.)
    for g in [s.q, s.p2, s.w, s.z, s.d] {
        assert!(!gone.contains(&g), "{g:?} should stay");
    }
    // The instance that `d` listed is gone, and is not left in the list.
    let after = remove(&s.sig, s.p);
    assert!(after.generator_info(s.d).unwrap().depends_on.is_empty());
}

#[test]
fn the_cascade_through_dependencies_is_transitive() {
    let s = setup();
    let gone = vanished(&s.sig, s.p);
    // `P3` mentions `u`, which goes because it lists `P`; `t` lists `P3` and nothing else.
    assert!(gone.contains(&s.p3));
    assert!(gone.contains(&s.t));
    // Deleting `P3` alone takes `t` and nothing more.
    assert_eq!(vanished(&s.sig, s.p3), sorted(vec![s.p3, s.t]));
    // The preview says the same, in the order it finds them, without the generator itself.
    let cascade = s.sig.removal_cascade(s.p);
    assert!(!cascade.contains(&s.p));
    assert_eq!(
        sorted(cascade),
        sorted(gone.into_iter().filter(|g| *g != s.p).collect())
    );
}

#[test]
fn deleting_an_ordinary_generator_does_not_follow_dependencies_it_is_not_in() {
    let s = setup();
    for g in [s.z, s.w, s.t, s.d] {
        assert_eq!(vanished(&s.sig, g), vec![g], "{g:?} alone");
    }
    // `w` depends on `P2`, so deleting `P2` takes it (and nothing else).
    assert_eq!(vanished(&s.sig, s.p2), sorted(vec![s.p2, s.w]));
}

#[test]
fn deleting_an_instance_does_not_delete_what_lists_it() {
    let s = setup();
    assert_eq!(
        s.sig.generator_info(s.d).unwrap().depends_on,
        vec![s.e_instance]
    );
    // `A[Q/P]` mentions `e[Q/P]`, so it goes; `d` only lists it.
    assert_eq!(
        vanished(&s.sig, s.e_instance),
        sorted(vec![s.e_instance, s.a_instance])
    );
    assert_eq!(s.sig.removal_cascade(s.e_instance), vec![s.a_instance]);
    let after = remove(&s.sig, s.e_instance);
    let d = after.generator_info(s.d).expect("d stays");
    // The entry is not left to name some generator that is later given the number.
    assert!(d.depends_on.is_empty());
    // Nor does the placeholder lose anything.
    assert!(after.generator_info(s.p).is_some());
    assert!(after.generator_info(s.u).is_some());
}

#[test]
fn an_instance_that_is_a_placeholder_does_not_cascade_either() {
    let mut s = setup();
    // Marking the instance a placeholder does not make it one that can be depended on.
    edit(
        &mut s.sig,
        s.e_instance,
        SignatureItemEdit::MakePlaceholder(true),
    );
    assert_eq!(
        vanished(&s.sig, s.e_instance),
        sorted(vec![s.e_instance, s.a_instance])
    );
}

#[test]
fn deleting_a_schema_deletes_its_instances_and_what_mentions_them() {
    let s = setup();
    assert_eq!(vanished(&s.sig, s.a), sorted(vec![s.a, s.a_instance]));
    let cascade = s.sig.removal_cascade(s.e);
    assert!(cascade.contains(&s.a) && cascade.contains(&s.e_instance));
    assert!(cascade.contains(&s.a_instance));
    assert!(!cascade.contains(&s.e));
}

fn check_preview(sig: &Signature) {
    for g in generators(sig) {
        let gone = vanished(sig, g);
        let mut preview = sig.removal_cascade(g);
        preview.push(g);
        assert_eq!(sorted(preview), gone, "{g:?}");
        // And the nodes it is made of are the ones the deletion detaches.
        let node = sig.find_node(g).unwrap();
        let nodes = sig.removal_nodes(node);
        assert_eq!(nodes.first(), Some(&node));
        let from_nodes = sorted(
            nodes
                .iter()
                .filter_map(|&n| sig.find_generator(n))
                .collect(),
        );
        assert_eq!(from_nodes, gone, "{g:?}");
        check_reasons(sig, node);
    }
}

/// The reasons are the nodes of the deletion, in its order, and each reason is true: a node
/// that goes with another is under it, or its diagram mentions a generator that goes before it,
/// or it is an instance whose substitution does, or it lists a placeholder that goes before it.
fn check_reasons(sig: &Signature, node: homotopy_common::tree::Node) {
    let reasons = sig.removal_reasons(node);
    let nodes = sig.removal_nodes(node);
    assert_eq!(reasons.iter().map(|r| r.0).collect::<Vec<_>>(), nodes);
    assert_eq!(reasons.first(), Some(&(node, RemovalReason::Itself)));
    let mut before: Vec<homotopy_common::tree::Node> = vec![];
    let mut generators_before: Vec<Generator> = vec![];
    for &(n, why) in &reasons {
        let info = sig.find_generator(n).and_then(|g| sig.generator_info(g));
        match why {
            RemovalReason::Itself => assert_eq!(n, node),
            RemovalReason::Within(root) => {
                assert!(before.contains(&root) && root != n);
                assert!(sig.as_tree().descendents_of(root).any(|d| d == n));
            }
            RemovalReason::Mentions(g) => {
                assert!(generators_before.contains(&g));
                assert!(info.unwrap().diagram.generators().contains_key(&g));
            }
            RemovalReason::Substitutes(g) => {
                assert!(generators_before.contains(&g));
                let made = info.unwrap().instance.as_ref().unwrap();
                assert!(made
                    .substitution
                    .iter()
                    .any(|(p, d)| *p == g || d.generators().contains_key(&g)));
            }
            RemovalReason::DependsOn(g) => {
                assert!(generators_before.contains(&g));
                assert!(info.unwrap().depends_on.contains(&g));
                assert!(sig.generator_info(g).unwrap().placeholder);
            }
        }
        before.push(n);
        generators_before.extend(sig.find_generator(n));
    }
    // The preview with reasons is the same, in pieces or not.
    assert_eq!(sig.removal_preview_reasons(node, usize::MAX), Some(reasons));
}

#[test]
fn every_generator_that_goes_says_why() {
    let s = setup();
    let why = |g: Generator, of: Generator| {
        s.sig
            .removal_cascade_reasons(of)
            .into_iter()
            .find(|r| r.0 == g)
            .map(|r| r.1)
    };
    let node = |g| s.sig.find_node(g).unwrap();
    // `u` lists `P` and does not mention it; `v` mentions `u`; the schema mentions `P`.
    assert_eq!(why(s.u, s.p), Some(RemovalReason::DependsOn(s.p)));
    assert_eq!(why(s.v, s.p), Some(RemovalReason::Mentions(s.u)));
    assert_eq!(why(s.e, s.p), Some(RemovalReason::Mentions(s.p)));
    // `P3` mentions `u`; `t` lists `P3`.
    assert_eq!(why(s.p3, s.p), Some(RemovalReason::Mentions(s.u)));
    assert_eq!(why(s.t, s.p), Some(RemovalReason::DependsOn(s.p3)));
    // The instance `e[Q/P]` is under `e`, and `A[Q/P]` is under `A`.
    assert_eq!(
        why(s.e_instance, s.p),
        Some(RemovalReason::Within(node(s.e)))
    );
    assert_eq!(
        why(s.a_instance, s.p),
        Some(RemovalReason::Within(node(s.a)))
    );
    // Deleting the instance takes what mentions it, and says so.
    assert_eq!(
        why(s.a_instance, s.e_instance),
        Some(RemovalReason::Mentions(s.e_instance))
    );
    // What the deletion does not take has no reason.
    assert_eq!(why(s.z, s.p), None);
    assert!(s.sig.removal_cascade_reasons(s.z).is_empty());
}

#[test]
fn the_preview_is_what_the_deletion_removes() {
    check_preview(&setup().sig);
}

#[test]
fn the_preview_is_what_the_deletion_removes_on_the_examples() {
    for json in [
        include_str!("examples/associator.json"),
        include_str!("examples/braiding_half_bubbles.json"),
        include_str!("examples/symmetric_monoidal_abelianize.json"),
        include_str!("examples/equivalence_to_adjoint_equivalence.json"),
        include_str!("examples/merge_contract_RI.json"),
    ] {
        let (_safe, actions): (bool, Vec<Action>) = serde_json::from_str(json).unwrap();
        let mut proof: Proof = Default::default();
        for a in &actions {
            proof.update(a).unwrap();
        }
        check_preview(&proof.signature);
    }
}

/// Needs the project, which is large: `HOMOTOPY_TANGENT_FIXTURE=<.hom> cargo test -p
/// homotopy-model --test removal --release -- --nocapture`.
#[test]
fn the_preview_is_what_the_deletion_removes_on_the_fixture() {
    let Ok(path) = std::env::var("HOMOTOPY_TANGENT_FIXTURE") else {
        return;
    };
    let ((sig, _), _) = serialize::deserialize(&std::fs::read(path).unwrap()).unwrap();
    let start = std::time::Instant::now();
    let mut biggest = 0;
    for g in generators(&sig) {
        biggest = biggest.max(sig.removal_cascade(g).len());
    }
    eprintln!(
        "{} generators, previews of all in {:?}, the largest cascade has {biggest}",
        generators(&sig).len(),
        start.elapsed()
    );
    check_preview(&sig);
}

#[test]
fn deleting_a_folder_deletes_what_depends_on_its_contents() {
    let mut s = setup();
    let root = s.sig.as_tree().root();
    s.sig.update(&SignatureEdit::NewFolder(root)).unwrap();
    let folder = s
        .sig
        .as_tree()
        .iter()
        .find_map(|(n, d)| matches!(d.inner(), SignatureItem::Folder(_)).then_some(n))
        .filter(|n| *n != root)
        .or_else(|| {
            s.sig
                .as_tree()
                .iter()
                .filter(|(n, d)| *n != root && matches!(d.inner(), SignatureItem::Folder(_)))
                .map(|(n, _)| n)
                .next()
        })
        .unwrap();
    let node = s.sig.find_node(s.p).unwrap();
    s.sig
        .update(&SignatureEdit::MoveInto(node, folder))
        .unwrap();
    let preview = s.sig.removal_generators(folder);
    s.sig.update(&SignatureEdit::Remove(folder)).unwrap();
    assert!(preview.contains(&s.p) && preview.contains(&s.u) && preview.contains(&s.v));
    assert!(s.sig.generator_info(s.u).is_none() && s.sig.generator_info(s.p).is_none());
    assert!(s.sig.generator_info(s.z).is_some());
}

#[test]
fn undoing_a_deletion_restores_everything_and_the_workspace_is_cleared() {
    let s = setup();
    let mut state = ProofState {
        signature: s.sig.clone(),
        ..Default::default()
    };
    // `u` depends on `P` without mentioning it, so the workspace holds a generator that goes.
    state.update(&Action::SelectGenerator(s.u)).unwrap();
    assert!(state.workspace.is_some());

    let mut history = History::default();
    history.add(Action::Nothing, proof_from_state(state));
    let before = history.proof().clone();
    let node = s.sig.find_node(s.p).unwrap();
    let action = Action::EditSignature(SignatureEdit::Remove(node));
    assert!(before.resets_panzoom(&action));
    let mut after = before.clone();
    after.update(&action).unwrap();
    history.add(action, after);

    assert!(history.proof().workspace.is_none());
    assert!(history.proof().signature.generator_info(s.u).is_none());
    assert!(history.proof().signature.generator_info(s.p).is_none());

    history.undo().unwrap();
    assert_eq!(history.proof().signature, before.signature);
    assert!(history.proof().signature.generator_info(s.u).is_some());
    assert!(history.proof().workspace.is_some());
    assert_eq!(
        history
            .proof()
            .signature
            .generator_info(s.d)
            .unwrap()
            .depends_on,
        vec![s.e_instance]
    );
    // And it can be done again.
    history.redo().unwrap();
    assert!(history.proof().signature.generator_info(s.v).is_none());
}

#[test]
fn a_bounded_preview_finishes_in_pieces_and_is_the_whole_answer() {
    let s = setup();
    let node = s.sig.find_node(s.p).unwrap();
    // (The generators of a diagram are looked at once per thread; this thread has not.)
    let mut asks = 0;
    let nodes = loop {
        asks += 1;
        assert!(asks < 1000, "the preview must end");
        if let Some(nodes) = s.sig.removal_preview(node, 3) {
            break nodes;
        }
    };
    assert!(asks > 1, "a small budget needs more than one ask");
    assert_eq!(nodes, s.sig.removal_nodes(node));
    // Once it is known, one ask does it, even with nothing left to look at.
    assert_eq!(s.sig.removal_preview(node, 0), Some(nodes));
}
