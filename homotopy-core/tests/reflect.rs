use homotopy_core::{
    signature::{GeneratorInfo, Invertibility, Signature, Symmetry},
    typecheck::{typecheck, Mode, TypeError},
    Diagram, Diagram0, DiagramN, Direction, Generator, Orientation,
};
use Direction::{Backward as B, Forward as F};

struct Info(Diagram, Invertibility);

impl GeneratorInfo for Info {
    fn diagram(&self) -> &Diagram {
        &self.0
    }

    fn invertibility(&self) -> Invertibility {
        self.1
    }
}

/// A signature in which every generator has a chosen invertibility.
#[derive(Default)]
struct Sig(Vec<(Generator, Info)>, Vec<Symmetry>);

impl Signature for Sig {
    type Info = Info;

    fn generators(&self) -> impl Iterator<Item = Generator> {
        self.0.iter().map(|(g, _)| *g)
    }

    fn generator_info(&self, g: Generator) -> Option<&Info> {
        self.0.iter().find(|(h, _)| *h == g).map(|(_, info)| info)
    }

    fn symmetries(&self) -> &[Symmetry] {
        &self.1
    }
}

impl Sig {
    fn add_zero(&mut self) -> Diagram {
        let g = Generator::new(self.0.len(), 0);
        let d: Diagram = Diagram0::from(g).into();
        self.0.push((g, Info(d.clone(), Invertibility::Directed)));
        d
    }

    fn add(&mut self, source: &Diagram, target: &Diagram, inv: Invertibility) -> Diagram {
        let g = Generator::new(self.0.len(), source.dimension() + 1);
        let d: Diagram = DiagramN::from_generator(g, source.clone(), target.clone())
            .unwrap()
            .into();
        self.0.push((g, Info(d.clone(), inv)));
        d
    }
}

/// Builds `x`, `C, D : x → x`, `F, G : C ⇒ D` and `η : F ⇛ G`.
fn setup(objects: Invertibility, morphisms: Invertibility) -> (Sig, Diagram, Diagram) {
    let mut sig = Sig::default();
    let x = sig.add_zero();
    let c = sig.add(&x, &x, objects);
    let d = sig.add(&x, &x, objects);
    let f = sig.add(&c, &d, morphisms);
    let g = sig.add(&c, &d, morphisms);
    let eta = sig.add(&f, &g, Invertibility::Directed);
    (sig, f, eta)
}

fn check(sig: &Sig, diagram: &Diagram) -> Result<(), TypeError> {
    typecheck(diagram, sig, Mode::default(), true)
}

#[test]
fn reflect_is_involution() {
    let (_, f, eta) = setup(Invertibility::Directed, Invertibility::Directed);
    for dirs in [[F, B], [B, F], [B, B]] {
        assert_eq!(f.reflect(&dirs, 0).reflect(&dirs, 0), f);
    }
    for dirs in [[B, F, F], [F, B, F], [F, F, B], [B, F, B]] {
        assert_eq!(eta.reflect(&dirs, 0).reflect(&dirs, 0), eta);
    }
}

/// Reflecting a generator in any of its normal directions flips its orientation, including
/// the outermost one (previously the outer reflection left nested points untouched).
#[test]
fn reflect_orientation_is_parity_of_reflected_directions() {
    let (_, f, eta) = setup(Invertibility::Directed, Invertibility::Directed);
    let orientation = |d: Diagram| d.max_generator().orientation;
    assert_eq!(orientation(f.reflect(&[B, F], 0)), Orientation::Negative);
    assert_eq!(orientation(f.reflect(&[F, B], 0)), Orientation::Negative);
    assert_eq!(orientation(f.reflect(&[B, B], 0)), Orientation::Positive);
    assert_eq!(
        orientation(eta.reflect(&[B, F, F], 0)),
        Orientation::Negative
    );
    assert_eq!(
        orientation(eta.reflect(&[B, F, B], 0)),
        Orientation::Positive
    );
}

#[test]
fn reflected_directed_generator_is_rejected() {
    let (sig, f, _) = setup(Invertibility::Directed, Invertibility::Directed);
    assert!(check(&sig, &f).is_ok());
    // F⁻¹ : D ⇒ C must not typecheck for a directed F.
    assert!(matches!(
        check(&sig, &f.reflect(&[B, F], 0)),
        Err(TypeError::Directed)
    ));
}

#[test]
fn rotations_of_directed_cells_need_dual_objects() {
    let (sig, f, eta) = setup(Invertibility::Dualisable(0), Invertibility::Dualisable(0));
    // The mate of F : C ⇒ D is a rotation, so it typechecks even if F were directed.
    assert!(check(&sig, &f.reflect(&[B, B], 0)).is_ok());
    // Rotating η in the (x₁, x₃)-plane gives the 2-cell-reversing `op` : G* ⇛ F*.
    assert!(check(&sig, &eta.reflect(&[B, F, B], 0)).is_ok());
    // Reflecting η in x₁ alone is orientation-reversing and η is directed.
    assert!(matches!(
        check(&sig, &eta.reflect(&[F, F, B], 0)),
        Err(TypeError::Directed)
    ));
}

#[test]
fn reflections_are_numbered_from_the_inside() {
    let (_, f, _) = setup(Invertibility::Directed, Invertibility::Directed);
    let diagram = DiagramN::try_from(f.clone()).unwrap();
    let reflections = diagram.reflections();
    assert_eq!(reflections.len(), 3);
    for (directions, reflected) in reflections {
        let expected = match directions[..] {
            [1] => f.reflect(&[F, B], 0),
            [2] => f.reflect(&[B, F], 0),
            [1, 2] => f.reflect(&[B, B], 0),
            _ => unreachable!(),
        };
        assert_eq!(Diagram::from(reflected), expected);
    }
    // Reflecting in the outermost direction alone is the inverse.
    assert_eq!(f.reflect(&[B, F], 0), diagram.inverse().into());
}

/// A k-dualisable 1-cell admits iterated bubbles (units and counits, their units and counits,
/// and so on) up to ambient dimension 1 + k + 1, i.e. it is (k + 1)-times dualisable.
#[test]
fn dualisable_k_allows_k_plus_one_levels_of_bubbles() {
    for (invertibility, levels) in [
        (Invertibility::Directed, 0),
        (Invertibility::Dualisable(0), 1),
        (Invertibility::Dualisable(1), 2),
        (Invertibility::Dualisable(2), 3),
        (Invertibility::Invertible, 4),
    ] {
        let mut sig = Sig::default();
        let x = sig.add_zero();
        let mut bubble = DiagramN::try_from(sig.add(&x, &x, invertibility)).unwrap();
        for level in 1..=4 {
            bubble = bubble.bubble().unwrap();
            assert_eq!(
                check(&sig, &bubble.clone().into()).is_ok(),
                level <= levels,
                "{invertibility} at level {level}"
            );
        }
    }
}

fn with_symmetry(mut sig: Sig, directions: &[usize]) -> Sig {
    sig.1.push(Symmetry {
        name: "op".to_owned(),
        directions: directions.to_vec(),
    });
    sig
}

/// With the symmetry `{1}`, directed cells may be reflected in x₁ (covariantly: `α ↦ α^op :
/// F^op ⇛ G^op`), but not inverted, and not rotated through cells that are merely symmetric.
#[test]
fn covariant_op_symmetry() {
    let (sig, f, eta) = setup(Invertibility::Directed, Invertibility::Directed);
    let sig = with_symmetry(sig, &[1]);
    assert!(check(&sig, &f).is_ok());
    assert!(
        check(&sig, &f.reflect(&[F, B], 0)).is_ok(),
        "F^op : C^op ⇒ D^op"
    );
    assert!(check(&sig, &f.reflect(&[B, F], 0)).is_err(), "F⁻¹ : D ⇒ C");
    assert!(
        check(&sig, &f.reflect(&[B, B], 0)).is_err(),
        "mate D^op ⇒ C^op"
    );
    assert!(
        check(&sig, &eta.reflect(&[F, F, B], 0)).is_ok(),
        "η^op : F^op ⇛ G^op"
    );
    assert!(
        check(&sig, &eta.reflect(&[B, F, B], 0)).is_err(),
        "G^op ⇛ F^op"
    );
    assert!(
        check(&sig, &eta.reflect(&[B, F, F], 0)).is_err(),
        "η⁻¹ : G ⇛ F"
    );
}

/// With the symmetry `{1, 3}` (`op` on **Cat**, which reverses 2-cells), α : F ⇒ G becomes
/// α^op : G^op ⇒ F^op.
#[test]
fn contravariant_op_symmetry() {
    let (sig, f, eta) = setup(Invertibility::Directed, Invertibility::Directed);
    let sig = with_symmetry(sig, &[1, 3]);
    assert!(
        check(&sig, &f.reflect(&[F, B], 0)).is_ok(),
        "F^op : C^op ⇒ D^op"
    );
    assert!(
        check(&sig, &eta.reflect(&[B, F, B], 0)).is_ok(),
        "η^op : G^op ⇛ F^op"
    );
    assert!(
        check(&sig, &eta.reflect(&[F, F, B], 0)).is_err(),
        "F^op ⇛ G^op"
    );
    assert!(
        check(&sig, &eta.reflect(&[B, F, F], 0)).is_err(),
        "η⁻¹ : G ⇛ F"
    );
}

/// Rotations (mates) of directed cells remain available through dualisable cells.
#[test]
fn symmetry_keeps_mates_through_dualisable_cells() {
    let (sig, f, _) = setup(Invertibility::Dualisable(0), Invertibility::Directed);
    let sig = with_symmetry(sig, &[1]);
    assert!(check(&sig, &f.reflect(&[B, B], 0)).is_ok(), "mate D* ⇒ C*");
    assert!(
        check(&sig, &f.reflect(&[F, B], 0)).is_ok(),
        "F^op : C* ⇒ D*"
    );
    // The mate of F^op is the reverse of F, which is then derivable.
    assert!(check(&sig, &f.reflect(&[B, F], 0)).is_ok(), "D ⇒ C");
}

/// Declaring a symmetry must not reject ordinary composite diagrams, where generators occur in
/// larger neighbourhoods, nor their reflections.
#[test]
fn symmetry_accepts_composites() {
    use homotopy_core::Boundary;

    let mut sig = Sig::default();
    let x = sig.add_zero();
    let c = DiagramN::try_from(sig.add(&x, &x, Invertibility::Directed)).unwrap();
    let cc = c.attach(&c, Boundary::Target, &[]).unwrap();
    let m = DiagramN::try_from(sig.add(&cc.into(), &c.into(), Invertibility::Directed)).unwrap();
    let left = m.attach(&m, Boundary::Source, &[0]).unwrap();
    let right = m.attach(&m, Boundary::Source, &[1]).unwrap();
    let a = sig.add(
        &left.clone().into(),
        &right.clone().into(),
        Invertibility::Directed,
    );
    let sig = with_symmetry(sig, &[1]);

    for diagram in [Diagram::from(left), right.into(), a.clone()] {
        assert!(check(&sig, &diagram).is_ok());
        let n = diagram.dimension();
        let mut op = vec![F; n];
        op[n - 1] = B;
        assert!(check(&sig, &diagram.reflect(&op, 0)).is_ok());
    }
    // a⁻¹ is still rejected.
    assert!(check(&sig, &a.reflect(&[B, F, F], 0)).is_err());
}
