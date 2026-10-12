use homotopy_core::{signature::SignatureBuilder, Boundary, DiagramN, Direction, Height};

/// Wires `a, b : x → x` with `f : a ⇒ a` and `g : b ⇒ b`, and the 2-diagram `f ▷ b ≫ a ◁ g`.
fn setup() -> (SignatureBuilder, DiagramN) {
    let mut sig = SignatureBuilder::default();
    let x = sig.add_zero();
    let a = sig.add(x, x).unwrap();
    let b = sig.add(x, x).unwrap();
    let f = sig.add(a.clone(), a.clone()).unwrap();
    let g = sig.add(b.clone(), b.clone()).unwrap();
    let ab = a.attach(&b, Boundary::Target, &[]).unwrap();
    let fg = ab
        .identity()
        .attach(&f, Boundary::Target, &[0])
        .unwrap()
        .attach(&g, Boundary::Target, &[1])
        .unwrap();
    (sig, fg)
}

/// Contracts the two heights of `diagram` into one (`f ◫ g`), as a single 3D step.
fn contract(sig: &SignatureBuilder, diagram: &DiagramN) -> DiagramN {
    diagram
        .clone()
        .identity()
        .contract(
            Boundary::Target.into(),
            &mut [],
            0,
            Direction::Forward,
            1,
            None,
            sig,
        )
        .unwrap()
}

/// Expands `f ◫ g` into the sequential order other than `not`, as a single 3D step.
fn expand(sig: &SignatureBuilder, parallel: &DiagramN, not: &DiagramN) -> DiagramN {
    [Direction::Forward, Direction::Backward]
        .into_iter()
        .flat_map(|direction| {
            (0..2).filter_map(move |h| {
                parallel
                    .clone()
                    .identity()
                    .expand(
                        Boundary::Target.into(),
                        &mut [],
                        [Height::Singular(0), Height::Singular(h)],
                        direction,
                        sig,
                    )
                    .ok()
            })
        })
        .find(|e| DiagramN::try_from(e.target()).unwrap() != *not)
        .unwrap()
}

fn concat(diagrams: &[&DiagramN]) -> DiagramN {
    DiagramN::new(
        diagrams[0].source(),
        diagrams.iter().flat_map(|d| d.cospans().to_vec()).collect(),
    )
}

#[test]
fn wrench_merges_into_single_exchange() {
    let (sig, fg) = setup();
    let to_parallel = contract(&sig, &fg);
    let parallel = DiagramN::try_from(to_parallel.target()).unwrap();
    let from_parallel = expand(&sig, &parallel, &fg);
    let wrench = concat(&[&to_parallel, &from_parallel]);
    assert_eq!(wrench.size(), 2);

    let (merged, count) = wrench.merge_exchanges(&sig);
    assert_eq!(count, 1);
    assert_eq!(merged.size(), 1);
    assert_eq!(merged.source(), wrench.source());
    assert_eq!(merged.target(), wrench.target());

    // The same as contracting the wrench one dimension up.
    assert_eq!(
        merged,
        DiagramN::try_from(contract(&sig, &wrench).target()).unwrap()
    );
}

#[test]
fn consecutive_wrenches_merge_separately() {
    let (sig, fg) = setup();
    let to_parallel = contract(&sig, &fg);
    let parallel = DiagramN::try_from(to_parallel.target()).unwrap();
    let swap = concat(&[&to_parallel, &expand(&sig, &parallel, &fg)]);
    let gf = DiagramN::try_from(swap.target()).unwrap();
    let to_parallel_again = contract(&sig, &gf);
    let parallel_again = DiagramN::try_from(to_parallel_again.target()).unwrap();
    let swap_back = concat(&[&to_parallel_again, &expand(&sig, &parallel_again, &gf)]);
    let there_and_back = concat(&[&swap, &swap_back]);
    assert_eq!(there_and_back.size(), 4);

    let (merged, count) = there_and_back.merge_exchanges(&sig);
    assert_eq!(count, 2);
    assert_eq!(merged.size(), 2);
    assert_eq!(merged.target(), there_and_back.target());
}

#[test]
fn generator_steps_are_left_alone() {
    let (sig, fg) = setup();
    let (merged, count) = fg.merge_exchanges(&sig);
    assert_eq!(count, 0);
    assert_eq!(merged, fg);

    // A contraction on its own has nothing to merge with.
    let to_parallel = contract(&sig, &fg);
    assert_eq!(to_parallel.merge_exchanges(&sig).1, 0);
}
