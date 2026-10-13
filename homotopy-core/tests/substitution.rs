use homotopy_core::{
    examples,
    signature::SignatureBuilder,
    substitution::{substitute, SubstitutionError},
    Boundary, Diagram, DiagramN, Generator,
};

fn generator(d: &DiagramN) -> Generator {
    d.max_generator().generator
}

#[test]
fn substituting_nothing_is_the_identity() {
    let cases: Vec<(&str, (SignatureBuilder, DiagramN))> = vec![
        ("associator", examples::associator()),
        ("two_monoid", examples::two_monoid()),
        ("two_endomorphism", examples::two_endomorphism()),
        ("scalar", examples::scalar()),
        ("two_beads", examples::two_beads()),
        ("three_beads", examples::three_beads()),
        ("stacks", examples::stacks()),
        ("matchsticks", examples::matchsticks()),
        ("bead_series", examples::bead_series(3)),
        ("iterated_endomorphism", examples::iterated_endomorphism(3)),
        ("monoid_unit", examples::monoid_unit()),
    ];
    for (name, (sig, diagram)) in cases {
        let diagram: Diagram = diagram.into();
        match substitute(&diagram, &sig, &|_| None) {
            Ok(result) => assert_eq!(result, diagram, "{name}"),
            Err(SubstitutionError::NotAtomic { .. }) => println!("{name}: not atomic"),
            Err(e) => panic!("{name}: {e}"),
        }
    }
}

#[test]
fn composite_replacement_at_the_top() {
    let mut sig = SignatureBuilder::default();
    let x = sig.add_zero();
    let p = sig.add(x, x).unwrap();
    let q = sig.add(x, x).unwrap();
    let r = sig.add(x, x).unwrap();
    let pp = p.attach(&p, Boundary::Target, &[]).unwrap();
    let qr = q.attach(&r, Boundary::Target, &[]).unwrap();
    let expected = qr.attach(&qr, Boundary::Target, &[]).unwrap();

    let p = generator(&p);
    let result = substitute(&pp.into(), &sig, &|g| (g == p).then(|| qr.clone().into())).unwrap();
    assert_eq!(result, expected.into());
}

#[test]
fn embeddings_shift_past_replaced_cells() {
    // Wires P, a : x → x and g : a ⇒ a, acting on a to the right of P.
    let mut sig = SignatureBuilder::default();
    let x = sig.add_zero();
    let p = sig.add(x, x).unwrap();
    let a = sig.add(x, x).unwrap();
    let q = sig.add(x, x).unwrap();
    let r = sig.add(x, x).unwrap();
    let g = sig.add(a.clone(), a.clone()).unwrap();
    let pa = p.attach(&a, Boundary::Target, &[]).unwrap();
    let diagram = pa
        .clone()
        .identity()
        .attach(&g, Boundary::Target, &[1])
        .unwrap();

    // Replacing P by Q R moves g one position to the right.
    let qr = q.attach(&r, Boundary::Target, &[]).unwrap();
    let qra = qr.attach(&a, Boundary::Target, &[]).unwrap();
    let expected = qra.identity().attach(&g, Boundary::Target, &[2]).unwrap();

    let p = generator(&p);
    let result = substitute(&diagram.into(), &sig, &|h| {
        (h == p).then(|| qr.clone().into())
    })
    .unwrap();
    assert_eq!(result, expected.into());

    // Replacing P by the identity on x moves g one position to the left.
    let expected = a.identity().attach(&g, Boundary::Target, &[0]).unwrap();
    let id: Diagram = Diagram::from(x).identity().into();
    let diagram = pa
        .clone()
        .identity()
        .attach(&g, Boundary::Target, &[1])
        .unwrap();
    let result = substitute(&diagram.into(), &sig, &|h| (h == p).then(|| id.clone())).unwrap();
    assert_eq!(result, expected.into());
}

#[test]
fn dependent_generators_are_replaced_with_their_instances() {
    // P, a : x → x, f : P ⇒ P and g : a ⇒ a; Q, R : x → x and f' : Q R ⇒ Q R.
    let mut sig = SignatureBuilder::default();
    let x = sig.add_zero();
    let p = sig.add(x, x).unwrap();
    let a = sig.add(x, x).unwrap();
    let q = sig.add(x, x).unwrap();
    let r = sig.add(x, x).unwrap();
    let f = sig.add(p.clone(), p.clone()).unwrap();
    let g = sig.add(a.clone(), a.clone()).unwrap();
    let qr = q.attach(&r, Boundary::Target, &[]).unwrap();
    let f_instance = sig.add(qr.clone(), qr.clone()).unwrap();

    // f ▷ a ≫ P ◁ g  ↦  f' ▷ a ≫ Q R ◁ g
    let pa = p.attach(&a, Boundary::Target, &[]).unwrap();
    let diagram = pa
        .identity()
        .attach(&f, Boundary::Target, &[0])
        .unwrap()
        .attach(&g, Boundary::Target, &[1])
        .unwrap();
    let qra = qr.attach(&a, Boundary::Target, &[]).unwrap();
    let expected = qra
        .identity()
        .attach(&f_instance, Boundary::Target, &[0])
        .unwrap()
        .attach(&g, Boundary::Target, &[2])
        .unwrap();

    let (p, f) = (generator(&p), generator(&f));
    let replacement = |h: Generator| {
        if h == p {
            Some(qr.clone().into())
        } else if h == f {
            Some(f_instance.clone().into())
        } else {
            None
        }
    };
    let result = substitute(&diagram.into(), &sig, &replacement).unwrap();
    assert_eq!(result, expected.into());
}

#[test]
fn reflected_occurrences_get_reflected_replacements() {
    let mut sig = SignatureBuilder::default();
    let x = sig.add_zero();
    let p = sig.add(x, x).unwrap();
    let q = sig.add(x, x).unwrap();
    let r = sig.add(x, x).unwrap();
    let qr = q.attach(&r, Boundary::Target, &[]).unwrap();

    let p_inverse = p.inverse();
    let pid = generator(&p);
    let result = substitute(&p_inverse.into(), &sig, &|h| {
        (h == pid).then(|| qr.clone().into())
    })
    .unwrap();
    assert_eq!(result, qr.inverse().into());
}

#[test]
fn contracted_heights_are_kept_or_separated() {
    use homotopy_core::{
        typecheck::{typecheck, Mode},
        Direction,
    };

    // f ◫ g: two cells contracted into a single height.
    let mut sig = SignatureBuilder::default();
    let x = sig.add_zero();
    let a = sig.add(x, x).unwrap();
    let b = sig.add(x, x).unwrap();
    let f = sig.add(a.clone(), a.clone()).unwrap();
    let g = sig.add(b.clone(), b.clone()).unwrap();
    let f2 = sig.add(a.clone(), a.clone()).unwrap();
    let ab = a.attach(&b, Boundary::Target, &[]).unwrap();
    let fg = ab
        .identity()
        .attach(&f, Boundary::Target, &[0])
        .unwrap()
        .attach(&g, Boundary::Target, &[1])
        .unwrap();
    let parallel = fg
        .identity()
        .contract(
            Boundary::Target.into(),
            &mut [],
            0,
            Direction::Forward,
            1,
            None,
            &sig,
        )
        .unwrap()
        .target();
    assert_eq!(DiagramN::try_from(parallel.clone()).unwrap().size(), 1);

    // Untouched, the contracted height stays as it is.
    assert_eq!(substitute(&parallel, &sig, &|_| None).unwrap(), parallel);

    // Replacing f, it is separated first.
    let (f, f2_gen) = (generator(&f), generator(&f2));
    let result = substitute(&parallel, &sig, &|h| (h == f).then(|| f2.clone().into())).unwrap();
    let result_n = DiagramN::try_from(result.clone()).unwrap();
    assert_eq!(result_n.size(), 2);
    let generators = result.generators();
    assert!(generators.contains_key(&f2_gen) && !generators.contains_key(&f));
    assert!(typecheck(&result, &sig, Mode::default(), true).is_ok());
}
