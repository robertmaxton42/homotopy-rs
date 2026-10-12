use homotopy_core::{
    signature::{GeneratorInfo, Invertibility, Signature},
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
struct Sig(Vec<(Generator, Info)>);

impl Signature for Sig {
    type Info = Info;

    fn generators(&self) -> impl Iterator<Item = Generator> {
        self.0.iter().map(|(g, _)| *g)
    }

    fn generator_info(&self, g: Generator) -> Option<&Info> {
        self.0.iter().find(|(h, _)| *h == g).map(|(_, info)| info)
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
