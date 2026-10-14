//! Looking for the embeddings through a point finds those of all the embeddings that contain the
//! point, in the same order: on the diagrams of the examples (every state of every proof) and
//! every point in their first two dimensions, and some deeper.

use homotopy_core::{Boundary, Diagram, DiagramN, Height};
use homotopy_model::{
    history::Proof,
    proof::{Action, Signature},
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

/// Every point of the first `depth` dimensions of the diagram, regular heights included, and the
/// shorter points that lead to them.
fn points(diagram: &Diagram, depth: usize) -> Vec<Vec<Height>> {
    fn go(diagram: &Diagram, depth: usize, prefix: &mut Vec<Height>, out: &mut Vec<Vec<Height>>) {
        out.push(prefix.clone());
        let Diagram::DiagramN(d) = diagram else {
            return;
        };
        if depth == 0 {
            return;
        }
        for i in 0..=2 * d.size() {
            let height = Height::from(i);
            prefix.push(height);
            go(&d.slice(height).unwrap(), depth - 1, prefix, out);
            prefix.pop();
        }
    }
    let mut out = vec![];
    go(diagram, depth, &mut vec![], &mut out);
    out
}

/// Compares the two ways for the needles of the generators of the signature, in a diagram.
fn compare(signature: &Signature, haystack: &Diagram, depth: usize, checked: &mut (usize, usize)) {
    let points = points(haystack, depth);
    for info in signature.iter() {
        let Diagram::DiagramN(d) = &info.diagram else {
            continue;
        };
        for boundary in [Boundary::Source, Boundary::Target] {
            let needle = d.slice(boundary).unwrap();
            if needle.dimension() != haystack.dimension() {
                continue;
            }
            let all: Vec<_> = haystack.embeddings(&needle).collect();
            for point in &points {
                let expected: Vec<_> = all
                    .iter()
                    .filter(|e| needle.contains_point(point, e))
                    .cloned()
                    .collect();
                let found: Vec<_> = haystack.embeddings_through(&needle, point).collect();
                assert_eq!(found, expected, "{} through {point:?}", info.name);
                checked.0 += 1;
                checked.1 += found.len();
            }
        }
    }
}

#[test]
fn embeddings_through_a_point_are_the_embeddings_containing_it() {
    let mut checked = (0, 0);
    let mut nonempty = 0;
    for json in EXAMPLES {
        let mut proof = Proof::default();
        for action in recorded_actions(json) {
            if proof.update(&action).is_err() {
                continue;
            }
            let Some(workspace) = proof.workspace.as_ref() else {
                continue;
            };
            let diagram = workspace.diagram.clone();
            compare(&proof.signature, &diagram, 3, &mut checked);
            // And in the boundaries, as attaching there does.
            if let Ok(d) = DiagramN::try_from(diagram) {
                for boundary in [Boundary::Source, Boundary::Target] {
                    compare(
                        &proof.signature,
                        &d.slice(boundary).unwrap(),
                        2,
                        &mut checked,
                    );
                }
            }
            nonempty += 1;
        }
    }
    assert!(checked.0 > 1000, "{checked:?}");
    assert!(checked.1 > 100, "some embeddings are found: {checked:?}");
    assert!(nonempty > 10);
}
