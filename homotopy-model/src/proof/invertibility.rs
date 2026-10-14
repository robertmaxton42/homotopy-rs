//! Tests that a placeholder that is invertible or dualisable can only be replaced with a diagram
//! that is: in the kernel, in the checks of the Instantiate panel, and in what is inferred.

use homotopy_core::{
    signature::{
        Invertibility::{self, Directed, Dualisable, Invertible},
        Signature as _,
    },
    Boundary, Diagram, DiagramN, Generator,
};

use super::{
    inference::Pool, DraftEdit, Drafts, InstantiationError, ReplacementError, Signature,
    SignatureEdit, SignatureError, SignatureItemEdit, Status,
};

fn generator(d: &DiagramN) -> Generator {
    d.max_generator().generator
}

fn cell(
    sig: &mut Signature,
    source: impl Into<Diagram>,
    target: impl Into<Diagram>,
    name: &str,
    level: Invertibility,
) -> DiagramN {
    sig.create_generator(source.into(), target.into(), name, level)
        .unwrap()
}

fn make_placeholder(sig: &mut Signature, g: Generator) {
    let node = sig.find_node(g).unwrap();
    sig.update(&SignatureEdit::Edit(
        node,
        SignatureItemEdit::MakePlaceholder(true),
    ))
    .unwrap();
}

/// The name of a generator, which has its number in it.
fn name_of(sig: &Signature, g: Generator) -> String {
    sig.generator_info(g).unwrap().name.clone()
}

fn name(sig: &Signature, d: &DiagramN) -> String {
    sig.generator_info(generator(d)).unwrap().name.clone()
}

/// The composite `first ≫ second` of cells that follow each other.
fn then(first: &DiagramN, second: &DiagramN) -> DiagramN {
    first.attach(second, Boundary::Target, &[]).unwrap()
}

/// `x`; `a, b, m : x → x`; the cells `f, d0, d1, d2, g : a ⇒ b` that are directed, dualisable
/// to levels 1, 2 and 3 and invertible, `h : b ⇒ a` invertible and `c1 : a ⇒ m`, `c2 : m ⇒ b`
/// invertible; the placeholders `W, D, F : a ⇒ b` that are invertible, dualisable to level 2 and
/// directed, each with a schema `S : P ⇛ P`; a placeholder `E : W ⇛ W` and the schema
/// `T : E ⇛ E`, and the cells `ηf : f ⇛ f` and `ηg : g ⇛ g`.
struct World {
    sig: Signature,
    f: DiagramN,
    d0: DiagramN,
    d1: DiagramN,
    d2: DiagramN,
    g: DiagramN,
    h: DiagramN,
    c1: DiagramN,
    c2: DiagramN,
    w: Generator,
    d: Generator,
    free: Generator,
    s_w: Generator,
    s_d: Generator,
    s_free: Generator,
    e: Generator,
    t: Generator,
    eta_f: Diagram,
    eta_g: Diagram,
}

fn world() -> World {
    let mut sig = Signature::default();
    let x: Diagram = sig.create_generator_zero("x").into();
    let [a, b, m] =
        ["a", "b", "m"].map(|name| cell(&mut sig, x.clone(), x.clone(), name, Directed));
    let f = cell(&mut sig, a.clone(), b.clone(), "f", Directed);
    let d0 = cell(&mut sig, a.clone(), b.clone(), "d0", Dualisable(0));
    let d1 = cell(&mut sig, a.clone(), b.clone(), "d1", Dualisable(1));
    let d2 = cell(&mut sig, a.clone(), b.clone(), "d2", Dualisable(2));
    let g = cell(&mut sig, a.clone(), b.clone(), "g", Invertible);
    let h = cell(&mut sig, b.clone(), a.clone(), "h", Invertible);
    let c1 = cell(&mut sig, a.clone(), m.clone(), "c1", Invertible);
    let c2 = cell(&mut sig, m, b.clone(), "c2", Invertible);

    let w = cell(&mut sig, a.clone(), b.clone(), "W", Invertible);
    let d = cell(&mut sig, a.clone(), b.clone(), "D", Dualisable(1));
    let free = cell(&mut sig, a, b, "F", Directed);
    let s_w = cell(&mut sig, w.clone(), w.clone(), "SW", Directed);
    let s_d = cell(&mut sig, d.clone(), d.clone(), "SD", Directed);
    let s_free = cell(&mut sig, free.clone(), free.clone(), "SF", Directed);
    let e = cell(&mut sig, w.clone(), w.clone(), "E", Directed);
    let t = cell(&mut sig, e.clone(), e.clone(), "T", Directed);
    let eta_f = cell(&mut sig, f.clone(), f.clone(), "eta_f", Directed);
    let eta_g = cell(&mut sig, g.clone(), g.clone(), "eta_g", Directed);

    for placeholder in [&w, &d, &free, &e] {
        make_placeholder(&mut sig, generator(placeholder));
    }
    World {
        w: generator(&w),
        d: generator(&d),
        free: generator(&free),
        s_w: generator(&s_w),
        s_d: generator(&s_d),
        s_free: generator(&s_free),
        e: generator(&e),
        t: generator(&t),
        eta_f: eta_f.into(),
        eta_g: eta_g.into(),
        sig,
        f,
        d0,
        d1,
        d2,
        g,
        h,
        c1,
        c2,
    }
}

/// Instantiating is refused for want of invertibility, with this message, and makes nothing.
fn refused(
    world: &mut World,
    schema: Generator,
    placeholder: Generator,
    value: &DiagramN,
) -> String {
    let count = world.sig.iter().count();
    let result = world
        .sig
        .instantiate(schema, &[(placeholder, value.clone().into())]);
    let Err(InstantiationError::NotInvertible(message)) = result else {
        panic!("expected the replacement to be refused for want of invertibility, got {result:?}");
    };
    assert_eq!(world.sig.iter().count(), count);
    message
}

fn accepted(world: &mut World, schema: Generator, placeholder: Generator, value: &DiagramN) {
    let result = world
        .sig
        .instantiate(schema, &[(placeholder, value.clone().into())]);
    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn an_invertible_placeholder_is_not_replaced_with_a_directed_cell() {
    let mut w = world();
    let (s, p, f) = (w.s_w, w.w, w.f.clone());
    let message = refused(&mut w, s, p, &f);
    let (nw, nf) = (name_of(&w.sig, p), name(&w.sig, &f));
    assert_eq!(
        message,
        format!("{nw} must be invertible, but {nf} is not ({nf} is directed)")
    );
    // Nor with a cell that is only dualisable, whatever its level.
    for cell in [w.d0.clone(), w.d1.clone(), w.d2.clone()] {
        let message = refused(&mut w, s, p, &cell);
        assert!(
            message.starts_with(&format!("{nw} must be invertible, but ")),
            "{message}"
        );
        assert!(message.contains("is dualisable (level"), "{message}");
    }
    // Nor with a composite that has a directed cell in it.
    let composite = then(&then(&w.f, &w.h), &w.g);
    let message = refused(&mut w, s, p, &composite);
    assert!(message.contains(&format!("{nf} is directed")), "{message}");
    // The weakest cell of a composite is the one that is named.
    let composite = then(&then(&w.d2, &w.h), &w.d1);
    let message = refused(&mut w, s, p, &composite);
    let nd1 = name(&w.sig, &w.d1);
    assert!(
        message.contains(&format!("{nd1} is dualisable (level 2)")),
        "{message}"
    );
}

#[test]
fn an_invertible_placeholder_is_replaced_with_invertible_diagrams() {
    let mut w = world();
    let (s, p) = (w.s_w, w.w);
    // An invertible cell.
    let g = w.g.clone();
    accepted(&mut w, s, p, &g);
    // The inverse of an invertible cell.
    let h_inverse = w.h.inverse();
    accepted(&mut w, s, p, &h_inverse);
    // A composite of invertible cells.
    let composite = then(&w.c1, &w.c2);
    accepted(&mut w, s, p, &composite);
    // A composite with an invertible cell, the inverse of another and an identity.
    let back_and_forth = then(&then(&w.g, &w.h), &w.g);
    accepted(&mut w, s, p, &back_and_forth);
    // Four instances, each made once.
    assert_eq!(
        w.sig
            .iter()
            .filter(|info| info.instance.as_ref().is_some_and(|i| i.schema == s))
            .count(),
        4
    );
}

#[test]
fn a_dualisable_placeholder_is_replaced_with_cells_that_are_dualisable_enough() {
    let mut w = world();
    let (s, p) = (w.s_d, w.d);
    // `D` is dualisable to level 2.
    let (f, d0) = (w.f.clone(), w.d0.clone());
    let message = refused(&mut w, s, p, &f);
    let (nd, nf, nd0) = (name_of(&w.sig, p), name(&w.sig, &f), name(&w.sig, &d0));
    assert_eq!(
        message,
        format!("{nd} must be dualisable (level 2), but {nf} is not ({nf} is directed)")
    );
    let message = refused(&mut w, s, p, &d0);
    assert_eq!(
        message,
        format!(
            "{nd} must be dualisable (level 2), but {nd0} is not ({nd0} is dualisable (level 1))"
        )
    );
    for ok in [w.d1.clone(), w.d2.clone(), w.g.clone(), w.h.inverse()] {
        accepted(&mut w, s, p, &ok);
    }
    // Dualisable cells compose, to the weakest level of their cells.
    let composite = then(&then(&w.d1, &w.h), &w.d2);
    accepted(&mut w, s, p, &composite);
    let composite = then(&then(&w.d0, &w.h), &w.d2);
    let message = refused(&mut w, s, p, &composite);
    assert!(
        message.contains(&format!("{nd0} is dualisable (level 1)")),
        "{message}"
    );
}

#[test]
fn a_directed_placeholder_is_replaced_with_anything() {
    let mut w = world();
    let (s, p) = (w.s_free, w.free);
    for ok in [w.f.clone(), w.d0.clone(), w.g.clone(), w.h.inverse()] {
        accepted(&mut w, s, p, &ok);
    }
}

#[test]
fn an_instance_has_the_invertibility_of_its_schema() {
    // The schema says what the instance is. A schema that is invertible (a claim about every
    // replacement, which the placeholders it mentions being invertible makes sound) has
    // invertible instances, and a directed one has directed instances.
    let mut w = world();
    let (s, p) = (w.s_w, w.w);
    let g = w.g.clone();
    let instance = w.sig.instantiate(s, &[(p, g.into())]).unwrap();
    assert_eq!(
        w.sig.generator_info(instance).unwrap().invertibility,
        Directed
    );
    let node = w.sig.find_node(s).unwrap();
    w.sig
        .update(&SignatureEdit::Edit(
            node,
            SignatureItemEdit::MakeInvertible(Invertible),
        ))
        .unwrap();
    let h_inverse = w.h.inverse();
    let instance = w.sig.instantiate(s, &[(p, h_inverse.into())]).unwrap();
    assert_eq!(
        w.sig.generator_info(instance).unwrap().invertibility,
        Invertible
    );
}

#[test]
fn a_cup_of_a_cell_is_as_invertible_as_its_unit() {
    // The cup created from nothing by a cell `c : x → x` is a 2-cell that is not an
    // equivalence unless `c` is one: it is the unit of the duality of `c`, which is
    // `k` times dualisable if `c` is `k + 1` times.
    let cases = [
        // (c, the placeholder, whether the cup can replace it)
        (Directed, Invertible, false),
        (Dualisable(0), Invertible, false),
        (Dualisable(0), Dualisable(0), false),
        (Dualisable(1), Dualisable(0), true),
        (Dualisable(1), Dualisable(1), false),
        (Dualisable(2), Dualisable(1), true),
        (Dualisable(2), Invertible, false),
        (Invertible, Invertible, true),
        (Invertible, Dualisable(3), true),
        (Dualisable(0), Directed, true),
    ];
    for (level, required, ok) in cases {
        let mut sig = Signature::default();
        let x = sig.create_generator_zero("x");
        let c = cell(&mut sig, x, x, "c", level);
        let cup = c.bubble().unwrap();
        let p = cell(&mut sig, cup.source(), cup.target(), "P", required);
        make_placeholder(&mut sig, generator(&p));
        let result = sig.check_invertibility(generator(&p), &cup.clone().into());
        assert_eq!(result.is_ok(), ok, "{level:?} {required:?}: {result:?}");
        if let Err(shortfall) = result {
            assert!(shortfall.fold);
            let message = sig.describe_shortfall(&shortfall);
            let nc = name(&sig, &c);
            assert!(
                message.contains(&format!("it folds {nc} back on itself")),
                "{message}"
            );
        }
    }
}

#[test]
fn the_panel_marks_a_replacement_that_is_not_invertible_as_soon_as_it_is_chosen() {
    let w = world();
    let mut drafts = Drafts::default();
    drafts.apply(&w.sig, DraftEdit::Fill(w.s_w, w.w, w.f.clone().into()));
    let draft = drafts.get(w.s_w);
    let error = draft.error(w.w).expect("the replacement is marked");
    assert!(matches!(
        error,
        ReplacementError::NotInvertible { because, .. } if *because == w.w
    ));
    let (nw, nf) = (name_of(&w.sig, w.w), name(&w.sig, &w.f));
    assert_eq!(
        w.sig.describe_replacement_error(w.w, error),
        format!("{nw} must be invertible, but {nf} is not ({nf} is directed)")
    );
    // It is kept to be seen, and takes no part in anything.
    assert_eq!(draft.get(w.w), Some(&w.f.clone().into()));
    assert!(draft.substitution().is_empty());

    // A replacement that is invertible is not marked.
    drafts.apply(&w.sig, DraftEdit::Fill(w.s_w, w.w, w.g.clone().into()));
    assert!(drafts.get(w.s_w).error(w.w).is_none());

    // `check_replacement` says the same without a draft, for every kind.
    for (schema, placeholder, value, ok) in [
        (w.s_w, w.w, &w.h.inverse(), true),
        (w.s_w, w.w, &w.d2, false),
        (w.s_d, w.d, &w.d0, false),
        (w.s_d, w.d, &w.d1, true),
        (w.s_free, w.free, &w.f, true),
    ] {
        let value: Diagram = value.clone().into();
        let result = w.sig.check_replacement(schema, &[], placeholder, &value);
        assert_eq!(result.is_ok(), ok, "{result:?}");
    }
}

#[test]
fn a_replacement_that_makes_another_placeholder_not_invertible_is_marked() {
    // `E : W ⇛ W` replaced with `ηf : f ⇛ f` makes `W` be `f`.
    let w = world();
    let mut drafts = Drafts::default();
    drafts.apply(&w.sig, DraftEdit::Fill(w.t, w.e, w.eta_f.clone()));
    let draft = drafts.get(w.t);
    let error = draft.error(w.e).expect("the replacement is marked");
    let ReplacementError::NotInvertible { because, shortfall } = error else {
        panic!("expected an error about invertibility, got {error:?}");
    };
    assert_eq!((*because, shortfall.placeholder), (w.e, w.w));
    let message = w.sig.describe_replacement_error(w.e, error);
    let (nw, ne, nf) = (
        name_of(&w.sig, w.w),
        name_of(&w.sig, w.e),
        name(&w.sig, &w.f),
    );
    assert_eq!(
        message,
        format!("{ne} needs {nw} to be {nf}, but {nw} must be invertible ({nf} is directed)")
    );
    assert!(draft.substitution().is_empty());
    assert_eq!(
        w.sig.infer(w.t, &[(w.e, w.eta_f.clone())]).status,
        Status::Inconsistent(error.clone())
    );

    // With `ηg : g ⇛ g`, `W` is `g`, which is fine and is inferred.
    drafts.apply(&w.sig, DraftEdit::Fill(w.t, w.e, w.eta_g.clone()));
    let draft = drafts.get(w.t);
    assert!(draft.error(w.e).is_none());
    assert_eq!(draft.get(w.w), Some(&w.g.clone().into()));
    assert!(draft.is_inferred(w.w));
}

#[test]
fn the_cells_a_placeholder_is_searched_for_among_are_as_invertible_as_it_is() {
    let w = world();
    let generators: Vec<Generator> = w.sig.iter().map(|info| info.generator).collect();
    let pool = Pool::new(&w.sig, generators);
    let has =
        |candidates: &[Diagram], cell: &DiagramN| candidates.contains(&Diagram::from(cell.clone()));

    let candidates = pool.candidates(w.w, &[]);
    assert!(has(&candidates, &w.g) && has(&candidates, &w.h.inverse()));
    assert!(!has(&candidates, &w.f) && !has(&candidates, &w.d2));
    for candidate in &candidates {
        assert_eq!(w.sig.check_invertibility(w.w, candidate), Ok(()));
    }
    // A composite of invertible cells is a candidate too.
    assert!(has(&candidates, &then(&w.c1, &w.c2)));

    // Dualisable to level 2: the cells that are, and the invertible ones.
    let candidates = pool.candidates(w.d, &[]);
    assert!(has(&candidates, &w.d1) && has(&candidates, &w.d2) && has(&candidates, &w.g));
    assert!(!has(&candidates, &w.f) && !has(&candidates, &w.d0));

    // A directed placeholder has all of them.
    let candidates = pool.candidates(w.free, &[]);
    assert!(has(&candidates, &w.f) && has(&candidates, &w.g) && has(&candidates, &w.d0));
}

#[test]
fn attaching_does_not_offer_a_directed_cell_for_an_invertible_placeholder() {
    let w = world();
    // `f ≫ h ≫ g : a ⇒ b` has the directed `f` and the invertible `h` and `g` in it.
    let haystack: Diagram = then(&then(&w.f, &w.h), &w.g).into();
    let replacements = |schema| -> Vec<Diagram> {
        w.sig
            .infer_instances(schema, &haystack, Boundary::Target, &[])
            .into_iter()
            .map(|instance| instance.substitution[0].1.clone())
            .collect()
    };

    // The schema of the directed placeholder can be attached with each of the cells.
    let for_free = replacements(w.s_free);
    assert!(for_free.contains(&w.f.clone().into()));
    assert!(for_free.contains(&w.g.clone().into()));

    // That of the invertible one with `g` but never with `f`, or anything with `f` in it.
    let for_w = replacements(w.s_w);
    assert!(for_w.contains(&w.g.clone().into()));
    for replacement in &for_w {
        assert_eq!(w.sig.check_invertibility(w.w, replacement), Ok(()));
        assert!(!replacement.generators().contains_key(&generator(&w.f)));
    }
}

/// `x`; `a : x → x`; `f, g : a ⇒ a` directed and invertible; placeholders `U, V : a ⇒ a` and
/// `H : U ≫ V ⇛ U ≫ V`; the schema `K : H ⇛ H`; and `ζ : g ≫ f ⇛ g ≫ f`.
/// Returns `K`, `H`, `U`, `V` and `ζ`.
fn splits(
    u: Invertibility,
    v: Invertibility,
) -> (
    Signature,
    Generator,
    Generator,
    Generator,
    Generator,
    Diagram,
) {
    let mut sig = Signature::default();
    let x: Diagram = sig.create_generator_zero("x").into();
    let a = cell(&mut sig, x.clone(), x, "a", Directed);
    let f = cell(&mut sig, a.clone(), a.clone(), "f", Directed);
    let g = cell(&mut sig, a.clone(), a.clone(), "g", Invertible);
    let pu = cell(&mut sig, a.clone(), a.clone(), "U", u);
    let pv = cell(&mut sig, a.clone(), a, "V", v);
    let uv = then(&pu, &pv);
    let h = cell(&mut sig, uv.clone(), uv, "H", Directed);
    let k = cell(&mut sig, h.clone(), h.clone(), "K", Directed);
    let gf = then(&g, &f);
    let zeta = cell(&mut sig, gf.clone(), gf, "zeta", Directed);
    for p in [&pu, &pv, &h] {
        make_placeholder(&mut sig, generator(p));
    }
    (
        sig,
        generator(&k),
        generator(&h),
        generator(&pu),
        generator(&pv),
        zeta.into(),
    )
}

#[test]
fn the_ways_to_fill_in_the_rest_are_the_ones_that_are_invertible_enough() {
    // `ζ : g ≫ f ⇛ g ≫ f` for `H : U ≫ V ⇛ U ≫ V` cuts `g ≫ f` into `U` and `V` three ways:
    // all of it either side, or one cell each.
    let infer = |u, v| {
        let (sig, k, h, pu, pv, zeta) = splits(u, v);
        (sig.infer(k, &[(h, zeta)]), pu, pv)
    };
    let (inference, ..) = infer(Directed, Directed);
    assert_eq!(inference.candidates.len(), 3);
    assert_eq!(inference.status, Status::Complete);

    // `U` invertible: not all of `g ≫ f` for it, as `f` is directed. With the identity for `U`
    // and `g ≫ f` for `V`, or `g` for `U` and `f` for `V`.
    let (inference, pu, pv) = infer(Invertible, Directed);
    assert_eq!(inference.candidates.len(), 2, "{inference:?}");
    assert_eq!(inference.status, Status::Complete);
    for candidate in &inference.candidates {
        let of = |p| &candidate.iter().find(|(q, _)| *q == p).unwrap().1;
        let (Diagram::DiagramN(u), Diagram::DiagramN(v)) = (of(pu), of(pv)) else {
            panic!("expected cells");
        };
        assert_eq!(u.size() + v.size(), 2);
        assert!(u.size() < 2, "U would have the directed f in it");
    }

    // Both invertible: `f` cannot be in either, so there is no way.
    let (inference, ..) = infer(Invertible, Invertible);
    assert_eq!(inference.status, Status::NoFilling, "{inference:?}");
    assert!(inference.candidates.is_empty() && inference.determined.is_empty());
}

#[test]
fn a_placeholder_cannot_be_made_more_invertible_than_its_replacements() {
    let mut w = world();
    let node = w.sig.find_node(w.free).unwrap();
    let make = |level| SignatureEdit::Edit(node, SignatureItemEdit::MakeInvertible(level));

    // Instantiated with an invertible cell: it can be made invertible.
    let (s, p, g, f) = (w.s_free, w.free, w.g.clone(), w.f.clone());
    accepted(&mut w, s, p, &g);
    w.sig.update(&make(Invertible)).unwrap();
    w.sig.update(&make(Directed)).unwrap();

    // Not with a directed one in an instance.
    accepted(&mut w, s, p, &f);
    let Err(error) = w.sig.update(&make(Invertible)) else {
        panic!("expected the edit to be refused");
    };
    assert!(
        matches!(error, SignatureError::InvertibilityOfReplaced { .. }),
        "{error}"
    );
    assert_eq!(
        w.sig.generator_info(p).unwrap().invertibility,
        Directed,
        "the signature is as it was"
    );
    // Dualisable is no better for a directed cell, and lower levels are free.
    assert!(w.sig.update(&make(Dualisable(0))).is_err());
    assert!(w.sig.update(&make(Directed)).is_ok());
}
