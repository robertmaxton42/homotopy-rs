//! Inferring instances of schemas on the user's tangent categories project (13MB, not in the
//! repository). Run with `HOMOTOPY_TANGENT_FIXTURE=<path to the .hom>`, in release; without it the
//! tests do nothing.
//!
//! The project's workspace is `start`, a 5-dimensional diagram; the picture shows its target (a
//! 4-dimensional diagram) as the row on top, with `T` on its left.

use homotopy_core::{Boundary, Diagram, Generator, Height, SliceIndex};
use homotopy_model::{
    history::Proof,
    job::Job,
    proof::{Action, AttachOption, AttachOptionsJob, InferenceLimits},
};

fn fixture() -> Option<Proof> {
    let path = std::env::var("HOMOTOPY_TANGENT_FIXTURE").ok()?;
    let bytes = std::fs::read(path).unwrap();
    let mut state = Proof::default();
    state.update(&Action::ImportProof(bytes.into())).unwrap();
    Some(state)
}

/// The one generator whose name contains all of the strings (and is not an instance).
fn containing(state: &Proof, parts: &[&str]) -> Generator {
    let found: Vec<_> = state
        .signature
        .iter()
        .filter(|info| info.instance.is_none())
        .filter(|info| parts.iter().all(|part| info.name.contains(part)))
        .collect();
    assert_eq!(found.len(), 1, "{parts:?}");
    found[0].generator
}

fn diagram_of(state: &Proof, generator: Generator) -> Diagram {
    state
        .signature
        .iter()
        .find(|info| info.generator == generator)
        .unwrap()
        .diagram
        .clone()
}

fn search(
    state: &Proof,
    point: &[SliceIndex],
    only: Generator,
    generator_ms: u32,
) -> homotopy_model::proof::AttachOutcome {
    let limits = InferenceLimits {
        generator_ms,
        ..InferenceLimits::default()
    };
    let mut job = AttachOptionsJob::new(state, point, false, limits, Some(only)).unwrap();
    loop {
        if let Some(outcome) = job.step() {
            return outcome;
        }
    }
}

/// The top row of the picture, `x` from the left.
fn top(x: Height) -> Vec<SliceIndex> {
    vec![
        SliceIndex::Boundary(Boundary::Target),
        SliceIndex::Interior(x),
    ]
}

fn attach(state: &Proof, option: &AttachOption) -> Diagram {
    let mut state = state.clone();
    state.update(&Action::Attach(option.clone())).unwrap();
    state.workspace.as_ref().unwrap().diagram.clone()
}

/// "Dragging the delta-on-objects schema onto the top-left corner fails completely, while
/// dragging the specific instance for T works fine." The instance for `T` attaches there as an
/// inverse (the target of the cell is what the diagram has), and the inference only looked for the
/// schema's source, so that of all the instances only the trivial one, `Ẋ := 1_𝒞`, whose source is
/// there as it is, was found.
#[test]
fn the_instance_for_t_of_delta_on_objects_is_found_at_the_top_left() {
    let Some(state) = fixture() else {
        return;
    };
    let schema = containing(&state, &["triangleleft\\delta=\\delta"]);
    let t = diagram_of(&state, containing(&state, &["$T$"]));
    let replaced_with_t =
        |substitution: &[(Generator, Diagram)]| substitution.len() == 1 && substitution[0].1 == t;
    let hand_made = state
        .signature
        .iter()
        .find(|info| {
            info.instance
                .as_ref()
                .is_some_and(|made| made.schema == schema && replaced_with_t(&made.substitution))
        })
        .map(|info| info.generator)
        .expect("the instance for T was made");
    for x in [Height::Regular(0), Height::Singular(0), Height::Regular(1)] {
        let point = top(x);
        // What the instance made by hand does there: only its inverse attaches.
        let plain = search(&state, &point, hand_made, 2_000);
        assert_eq!(plain.options.len(), 1, "{x:?}");
        let by_hand = &plain.options[0];
        assert!(by_hand.substitution.is_none());

        let outcome = search(&state, &point, schema, 0);
        assert!(outcome.inference.cut_short.is_empty());
        let inferred: Vec<_> = outcome
            .options
            .iter()
            .filter(|option| option.substitution.is_some())
            .collect();
        // The trivial instance, forwards and backwards, and the one for `T`, backwards.
        assert!(inferred.iter().any(|option| !option.inverse));
        let for_t = inferred
            .iter()
            .find(|option| {
                option.inverse
                    && option
                        .substitution
                        .as_ref()
                        .is_some_and(|s| replaced_with_t(s))
            })
            .unwrap_or_else(|| panic!("no T at {x:?}: {:?}", outcome.options));
        // Attaching it is attaching the instance made by hand.
        assert_eq!(by_hand.embedding, for_t.embedding);
        assert_eq!(attach(&state, for_t), attach(&state, by_hand));
    }
}
