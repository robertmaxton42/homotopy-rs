//! The notation that the names of derived cells and of diagrams are written in.
//!
//! Everything about *which symbol* or *which shape of formula* is used in a name that the app
//! makes (see [`Structure`](super::Structure), `describe.rs` and `doc/labels.md`) is decided
//! here, and nowhere else: the code that makes names only decides *what* to say (that this cell
//! is the inverse of `f`, that this one is `p` whiskered by `T`) and asks the [`Notation`] how
//! it is written. A different convention (another "language" for names) is another
//! implementation of the trait that overrides some of its methods: the defaults are the
//! conventions of [`Standard`]. Which one is in use is [`current`].
//!
//! Every method takes and returns *formulas* (the text between dollar signs, see
//! `homotopy_common::tex`), with the names of cells already put in as formulas. Arguments are
//! not wrapped in braces: a method puts braces where its notation needs them.
//!
//! Order is diagrammatic, as for composition: a composite `f ; g` is `f` and then `g`, and a
//! cell whiskered *on the right* by `T` has `T` after it, `p ▷ T`, and one whiskered *on the
//! left* by `S` has `S` before it, `S ◁ p`. The triangle points at the whiskering cell.

/// How names are written. Each method has the conventional notation as its default.
pub trait Notation: Sync {
    /// The identity on `inner`, raised `k` times to reach the dimension that is needed:
    /// `\mathrm{id}_{f}`, `\mathrm{id}^{2}_{f}`.
    fn identity(&self, k: usize, inner: &str) -> String {
        format!(r"\mathrm{{id}}{}_{{{inner}}}", iterated(k))
    }

    /// The inverse of an invertible cell: `f^{-1}`.
    fn inverse(&self, f: &str) -> String {
        format!("{{{f}}}^{{-1}}")
    }

    /// The reverse of a cell that is only dualisable, in the only direction there is: `f^{*}`.
    fn dual(&self, f: &str) -> String {
        format!("{{{f}}}^{{*}}")
    }

    /// The reverse of a cell as drawn, when it is not known in which directions it was
    /// reflected: `\overline{f}`.
    fn reversed(&self, f: &str) -> String {
        format!(r"\overline{{{f}}}")
    }

    /// The bubble of `inner`, which creates it together with its reverse, taken up `k`
    /// dimensions: `\mathrm{bub}_{f}`.
    fn bubble(&self, k: usize, inner: &str) -> String {
        format!(r"\mathrm{{bub}}{}_{{{inner}}}", iterated(k))
    }

    /// The reflection of `f` in the given directions (numbered from 1; for the first direction
    /// alone, see [`dual`](Self::dual)): `\mathrm{R}_{1,3}(f)`.
    fn reflection(&self, directions: &[usize], f: &str) -> String {
        let directions = directions
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        format!(r"\mathrm{{R}}_{{{directions}}}({{{f}}})")
    }

    /// The reflection of `f` by the symmetry named `symmetry` (a formula): `f^{op}`.
    fn symmetry(&self, symmetry: &str, f: &str) -> String {
        format!("{{{f}}}^{{{symmetry}}}")
    }

    /// The parts of a composite, in the order of the diagram: `f ; g ; h`. A part may be
    /// [`cut`](Self::cut) where the composite was cut short.
    fn compose(&self, parts: &[String]) -> String {
        parts.join(r" \mathbin{;} ")
    }

    /// What stands for the steps of a composite that are left out.
    fn cut(&self) -> &'static str {
        r"\cdots"
    }

    /// What stands for what was left out of a description, because of its size.
    fn ellipsis(&self) -> &'static str {
        r"\ldots"
    }

    /// A formula that is a composite or a type, put where it is one part of another formula
    /// without being read as part of it: `(f ; g)`.
    fn group(&self, formula: &str) -> String {
        format!("({formula})")
    }

    /// The cell `cell` whiskered on the left by `whisker`, which is before it: `S ◁ p`.
    fn whisker_left(&self, whisker: &str, cell: &str) -> String {
        format!(r"{whisker} \triangleleft {cell}")
    }

    /// The cell `cell` (which may be whiskered on the left already) whiskered on the right by
    /// `whisker`, which is after it: `p ▷ T`.
    fn whisker_right(&self, cell: &str, whisker: &str) -> String {
        format!(r"{cell} \triangleright {whisker}")
    }

    /// The exchange of two cells, the first being the lowest in the source, when it is not known
    /// on which side of each other they pass (they are attached to something that does not say):
    /// `\beta_{f,g}`, the same symbol as the [braid](Self::braid) with the sides given.
    fn exchange(&self, first: &str, second: &str) -> String {
        format!(r"\beta_{{{first},{second}}}")
    }

    /// The braid of two cells that are attached to nothing: `\beta_{f,g}`, or the one that
    /// passes on the other side, `\beta^{-1}_{f,g}`.
    fn braid(&self, first: &str, second: &str, other_side: bool) -> String {
        let sign = if other_side { "^{-1}" } else { "" };
        format!(r"\beta{sign}_{{{first},{second}}}")
    }

    /// The counit: `f` and its reverse cancelling.
    fn counit(&self, f: &str) -> String {
        format!(r"\varepsilon_{{{f}}}")
    }

    /// The unit: `f` and its reverse being created.
    fn unit(&self, f: &str) -> String {
        format!(r"\eta_{{{f}}}")
    }

    /// A homotopy that is not recognised.
    fn homotopy(&self) -> &'static str {
        r"\mathrm{h}"
    }

    /// A cell that is drawn as a fold of a homotopy, whose reverse is not known.
    fn fold(&self, f: &str) -> String {
        format!(r"{}({f})", self.homotopy())
    }

    /// A type, from the description of its source and of its target, with the arrow for the
    /// dimension of the diagram: `s \Rightarrow t`.
    fn arrow(&self, dimension: usize, source: &str, target: &str) -> String {
        let arrow = match dimension {
            1 => r"\to",
            2 => r"\Rightarrow",
            _ => r"\Rrightarrow",
        };
        format!("{source} {arrow} {target}")
    }
}

/// The exponent of an iterated operation, if it is iterated.
fn iterated(k: usize) -> String {
    if k > 1 {
        format!("^{{{k}}}")
    } else {
        String::new()
    }
}

/// The conventional notation: the defaults of [`Notation`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Standard;

impl Notation for Standard {}

/// The notation that names are written in.
#[must_use]
pub fn current() -> &'static dyn Notation {
    &Standard
}

#[cfg(test)]
mod tests {
    use super::{Notation, Standard};

    #[test]
    fn the_standard_notation() {
        let n = Standard;
        assert_eq!(n.identity(1, "f"), r"\mathrm{id}_{f}");
        assert_eq!(n.identity(2, "f"), r"\mathrm{id}^{2}_{f}");
        assert_eq!(n.inverse("f"), "{f}^{-1}");
        assert_eq!(n.dual("f"), "{f}^{*}");
        assert_eq!(n.reflection(&[1, 3], "f"), r"\mathrm{R}_{1,3}({f})");
        assert_eq!(n.symmetry("op", "f"), "{f}^{op}");
        assert_eq!(n.whisker_left("S", "p"), r"S \triangleleft p");
        assert_eq!(n.whisker_right("p", "T"), r"p \triangleright T");
        assert_eq!(
            n.whisker_right(&n.whisker_left("S", "p"), "T"),
            r"S \triangleleft p \triangleright T"
        );
        assert_eq!(n.group("F \\mathbin{;} G"), r"(F \mathbin{;} G)");
        assert_eq!(n.braid("a", "b", true), r"\beta^{-1}_{a,b}");
        assert_eq!(n.arrow(2, "s", "t"), r"s \Rightarrow t");
    }
}
