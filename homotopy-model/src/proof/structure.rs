//! Default names for the cells that the app provides itself, rather than the user: identities,
//! inverses, bubbles and reflections of generators. They are shown in the attach menu, as
//! labels (see [`homotopy_common::tex`]) with the conventional symbol for each: `\mathrm{id}_f`
//! for the identity on `f`, `f^{-1}` for its inverse, and so on. The name of the generator is
//! put into the formula whatever it is, whether text or a formula itself. The symbols and the
//! shape of each formula are the [notation](super::notation)'s.

use homotopy_common::tex::{seal, to_math};

use super::notation::{self, Notation};

/// A cell that the app derives from a generator (or from a schema, with a substitution).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Structure {
    /// The identity on the generator, taken the given number of times to reach the dimension
    /// that is needed: `\mathrm{id}_f`, `\mathrm{id}^2_f`.
    Identity(usize),
    /// The identity on the inverse of the generator.
    InverseIdentity(usize),
    /// The inverse of the generator: `f^{-1}`.
    Inverse,
    /// The bubble of the generator, which creates it together with its reverse, iterated the
    /// given number of times: `\mathrm{bub}_f`, `\mathrm{bub}^2_f`.
    Bubble(usize),
    /// The bubble of the inverse of the generator.
    InverseBubble(usize),
    /// The reflection of the generator in the given directions (see `doc/duals.md`), numbered
    /// from 1: the dual `f^*` for the first direction, and `\mathrm{R}_{1,3}(f)` otherwise.
    Reflection(Vec<usize>),
    /// The reflection of the generator by the symmetry with the given name: `f^{\mathrm{op}}`.
    Symmetry(String),
    /// An instance of a schema, with a description of the substitution: `A [Q/P]`.
    Instance(String),
}

impl Structure {
    /// The label of this cell for the generator with the given name.
    #[must_use]
    pub fn label(&self, name: &str) -> String {
        match self {
            Self::Instance(description) => format!("{} [{}]", seal(name), seal(description)),
            _ => format!("${}$", self.math(name).unwrap_or_default()),
        }
    }

    /// The formula (without dollar signs) for this cell, for the generator with the given name:
    /// everything but an [instance](Self::Instance), which is a label of its own. It is written
    /// in the [current notation](notation::current).
    #[must_use]
    pub fn math(&self, name: &str) -> Option<String> {
        self.math_in(notation::current(), name)
    }

    /// As [`math`](Self::math), in the given notation.
    #[must_use]
    pub fn math_in(&self, notation: &dyn Notation, name: &str) -> Option<String> {
        let f = to_math(name);
        Some(match self {
            Self::Identity(k) => notation.identity(*k, &f),
            Self::InverseIdentity(k) => notation.identity(*k, &notation.inverse(&f)),
            Self::Inverse => notation.inverse(&f),
            Self::Bubble(k) => notation.bubble(*k, &f),
            Self::InverseBubble(k) => notation.bubble(*k, &notation.inverse(&f)),
            Self::Reflection(directions) if directions.as_slice() == [1] => notation.dual(&f),
            Self::Reflection(directions) => notation.reflection(directions, &f),
            Self::Symmetry(symmetry) => notation.symmetry(&to_math(symmetry), &f),
            Self::Instance(_) => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use homotopy_common::tex::{pieces, Piece};

    use super::Structure;

    #[test]
    fn labels_follow_the_conventional_symbols() {
        use Structure::{
            Bubble, Identity, Instance, Inverse, InverseBubble, InverseIdentity, Reflection,
            Symmetry,
        };
        assert_eq!(Identity(1).label("f"), r"$\mathrm{id}_{\text{f}}$");
        assert_eq!(
            Identity(2).label("$\\alpha$"),
            r"$\mathrm{id}^{2}_{{\alpha}}$"
        );
        assert_eq!(
            InverseIdentity(1).label("f"),
            r"$\mathrm{id}_{{\text{f}}^{-1}}$"
        );
        assert_eq!(Inverse.label("f"), r"${\text{f}}^{-1}$");
        assert_eq!(Bubble(1).label("f"), r"$\mathrm{bub}_{\text{f}}$");
        assert_eq!(
            InverseBubble(3).label("f"),
            r"$\mathrm{bub}^{3}_{{\text{f}}^{-1}}$"
        );
        assert_eq!(Reflection(vec![1]).label("f"), r"${\text{f}}^{*}$");
        assert_eq!(
            Reflection(vec![1, 3]).label("f"),
            r"$\mathrm{R}_{1,3}({\text{f}})$"
        );
        assert_eq!(
            Symmetry("op".to_owned()).label("f"),
            r"${\text{f}}^{\text{op}}$"
        );
        assert_eq!(Instance("Q/P".to_owned()).label("A"), "A [Q/P]");
    }

    #[test]
    fn labels_are_one_formula_whatever_the_name() {
        // Names that try to break out of the formula, or leave off their last dollar sign.
        for name in ["a $x", "$", "}{", "$x$ $y", "\\", "\\$", "<b>", "100%"] {
            for structure in [
                Structure::Identity(1),
                Structure::InverseIdentity(2),
                Structure::Inverse,
                Structure::Bubble(1),
                Structure::InverseBubble(1),
                Structure::Reflection(vec![1]),
                Structure::Reflection(vec![2, 3]),
                Structure::Symmetry(name.to_owned()),
            ] {
                let label = structure.label(name);
                let pieces = pieces(&label);
                assert!(
                    matches!(pieces.as_slice(), [Piece::Math(_)]),
                    "{label:?} for {name:?}"
                );
            }
        }
        // Instances are labels, which cannot leak into each other.
        let label = Structure::Instance("a $y".to_owned()).label("b $x");
        assert!(pieces(&label).iter().all(|p| !matches!(p, Piece::Math(_))));
    }
}
