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

/// The merging as it was before it could be done a step at a time.
fn merge_exchanges_in_one_go(diagram: &DiagramN, sig: &SignatureBuilder) -> (DiagramN, usize) {
    use homotopy_core::{rewrite::Cospan, typecheck::typecheck_cospan};

    let regular_slices = diagram.regular_slices().collect::<Vec<_>>();
    let mut cospans: Vec<(Cospan, usize)> = Vec::new();
    for (i, cospan) in diagram.cospans().iter().enumerate() {
        if let Some((last, start)) = cospans.last_mut() {
            if last.backward.is_identity() && cospan.forward.is_identity() {
                let candidate = Cospan {
                    forward: last.forward.clone(),
                    backward: cospan.backward.clone(),
                };
                if typecheck_cospan(regular_slices[*start].clone(), candidate.clone(), sig).is_ok()
                {
                    *last = candidate;
                    continue;
                }
            }
        }
        cospans.push((cospan.clone(), i));
    }
    let merged = diagram.size() - cospans.len();
    (
        DiagramN::new(
            diagram.source(),
            cospans.into_iter().map(|(cospan, _)| cospan).collect(),
        ),
        merged,
    )
}

#[test]
fn merging_a_step_at_a_time_gives_the_same_result() {
    let (sig, fg) = setup();
    let to_parallel = contract(&sig, &fg);
    let parallel = DiagramN::try_from(to_parallel.target()).unwrap();
    let swap = concat(&[&to_parallel, &expand(&sig, &parallel, &fg)]);
    let gf = DiagramN::try_from(swap.target()).unwrap();
    let to_parallel_again = contract(&sig, &gf);
    let parallel_again = DiagramN::try_from(to_parallel_again.target()).unwrap();
    let swap_back = concat(&[&to_parallel_again, &expand(&sig, &parallel_again, &gf)]);
    let there_and_back = concat(&[&swap, &swap_back]);

    for diagram in [&fg, &to_parallel, &swap, &there_and_back] {
        let expected = merge_exchanges_in_one_go(diagram, &sig);
        assert_eq!(diagram.merge_exchanges(&sig), expected);

        let mut merging = diagram.merge_exchanges_steps();
        let mut steps = 0;
        assert_eq!(merging.progress(), (0, diagram.size()));
        while merging.step(&sig) {
            steps += 1;
            assert_eq!(merging.progress(), (steps, diagram.size()));
        }
        assert_eq!(steps, diagram.size());
        assert!(!merging.step(&sig), "stepping past the end does nothing");
        assert_eq!(merging.finish(), expected);
    }
}

#[test]
fn stopping_early_leaves_the_rest_of_the_diagram_alone() {
    let (sig, fg) = setup();
    let to_parallel = contract(&sig, &fg);
    let parallel = DiagramN::try_from(to_parallel.target()).unwrap();
    let swap = concat(&[&to_parallel, &expand(&sig, &parallel, &fg)]);

    // Before the second step is considered, there is nothing to merge yet...
    let mut merging = swap.merge_exchanges_steps();
    assert!(merging.step(&sig));
    let (unmerged, count) = merging.finish();
    assert_eq!((unmerged, count), (swap.clone(), 0));

    // ...and afterwards the pair is merged.
    let mut merging = swap.merge_exchanges_steps();
    assert!(merging.step(&sig));
    assert!(merging.step(&sig));
    assert_eq!(merging.finish().1, 1);
}
