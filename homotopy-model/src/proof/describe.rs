//! Names for diagrams that are not generators, built from what the diagram is made of.
//!
//! A diagram that the user constructs in the workspace and uses as an argument (a replacement
//! for a placeholder, say) has no name of its own, but it is usually something recognisable: an
//! identity, a composite, a cell with other cells beside it, an exchange of two cells. The
//! description is a formula (see `doc/labels.md`) that follows the first of these that applies:
//!
//! 1. a generator: its name;
//! 2. a cell that the app derives from a generator, as [`Structure`] names it: `f^{-1}`,
//!    `f^{*}`, `\mathrm{R}_{1,3}(f)`, `f^{op}`, `\mathrm{bub}_f`;
//! 3. an identity on any diagram: `\mathrm{id}_{f}`, and `\mathrm{id}^{k}_{f}` for `k` of them
//!    on top of each other;
//! 4. a composite, in the order of the diagram (from its source to its target), each step
//!    described by the following: `f ; g`;
//! 5. one step of the diagram that introduces a cell: the name of the cell, without what is
//!    beside it. The inverse of a cell is `f^{-1}` if it is invertible and `f^{*}` otherwise
//!    (for 1-cells; `\overline{f}` for others, as the directions it was reflected in are not
//!    known any more);
//! 6. one step that exchanges two cells, `\mathrm{ex}_{f,g}`, with `f` the one that is first
//!    (lowest) in the source. If nothing but the exchange itself fixes on which side of each
//!    other the cells are (neither cell is attached to anything), it is a braid,
//!    `\beta_{f,g}`, or `\beta^{-1}_{f,g}` if `f` passes on the other side;
//! 7. one step that cancels or creates a cell `f` with its reverse: the counit `\varepsilon_f`
//!    for `f ; f^*` becoming nothing, the unit `\eta_f` for `f^* ; f` created out of nothing
//!    (`\varepsilon_{f^*}` and `\eta_{f^*}` for the other order); any other step that does not
//!    introduce a cell is `\mathrm{h}`;
//! 8. anything else is described by its type, `s \Rightarrow t`, where the source and target are
//!    described in the same way, but only to a small depth.
//!
//! Diagrams in the workspace can be large, and a description is made every time the panel is
//! shown, so the work is bounded: at most [`MAX_STEPS`] diagrams are looked at, whatever the
//! diagram, and what is left out is shown as `\ldots`.

use homotopy_common::tex::{seal, to_math};
use homotopy_core::{
    rewrite::ConeView, signature::Signature as _, Diagram, Diagram0, DiagramN, Height, Orientation,
    Rewrite, SliceIndex,
};

use super::{Signature, Structure};

/// How many diagrams (and steps of them) one description looks at.
const MAX_STEPS: usize = 48;
/// How many times a type may contain a type.
const MAX_DEPTH: usize = 2;
/// How many steps of a composite are shown before it is cut short.
const MAX_LAYERS: usize = 6;
/// The size of the largest composite that is looked at for its last step.
const MAX_SCAN: usize = 64;
/// How many points of a step are looked at to count its cells.
const MAX_POINTS: usize = 256;
/// The largest dimension in which reflections of a generator are compared with a diagram.
const MAX_REFLECTED_DIMENSION: usize = 5;
/// How many dimensions a bubble is taken up.
const MAX_BUBBLES: usize = 3;

const ELLIPSIS: &str = r"\ldots";
const HOMOTOPY: &str = r"\mathrm{h}";

impl Signature {
    /// A short description of a diagram, as a label that can be put in other labels: its name
    /// if it is a generator, and otherwise a formula that says what it is made of (see the
    /// [module](self)).
    #[must_use]
    pub fn describe(&self, diagram: &Diagram) -> String {
        if let Some(info) = self.iter().find(|info| info.diagram == *diagram) {
            return seal(&info.name);
        }
        let math = Describer::new(self).diagram(diagram, 0);
        if math == ELLIPSIS {
            "…".to_owned()
        } else {
            format!("${math}$")
        }
    }

    /// The label of the cell drawn at a vertex of a diagram, for the generator `cell` there
    /// (with its orientation): `point` is the vertex in the projection of `diagram` to two
    /// dimensions, the heights in its outermost direction and then in the next one.
    ///
    /// A generator is named by the generator, or its reverse. A vertex where no cell of the
    /// dimension of the diagram is, but several things happen at once (two strands crossing,
    /// a cell cancelled or created), is named for the step of the diagram that it is (see the
    /// [module](self)): `\mathrm{ex}_{f,g}`, `\beta_{f,g}`, `\varepsilon_f`, `\eta_f`. If the step is
    /// not recognised, a cell drawn as a fold is `\mathrm{h}(f)`, and any other is named as it is.
    #[must_use]
    pub fn describe_point(
        &self,
        diagram: &Diagram,
        point: &[SliceIndex],
        cell: Diagram0,
    ) -> String {
        let describer = Describer::new(self);
        let homotopy =
            cell.orientation == Orientation::Zero || cell.generator.dimension < diagram.dimension();
        let math = homotopy
            .then(|| describer.step_at(diagram, point))
            .flatten()
            .unwrap_or_else(|| describer.cell(cell));
        format!("${math}$")
    }

    /// The label of a cell, as drawn on a wire or a surface: see
    /// [`describe_point`](Self::describe_point).
    #[must_use]
    pub fn describe_cell(&self, cell: Diagram0) -> String {
        format!("${}$", Describer::new(self).cell(cell))
    }
}

struct Describer<'a> {
    signature: &'a Signature,
    steps: usize,
}

/// The cells of a step.
enum Cells {
    None,
    One(Diagram0),
    /// More than one, or too many points to tell.
    Many,
}

/// The cells of the given dimension in a diagram: those at points of every singular height.
fn cells(diagram: &Diagram, dimension: usize) -> Cells {
    fn visit(
        diagram: &Diagram,
        dimension: usize,
        found: &mut Option<Diagram0>,
        points: &mut usize,
    ) -> bool {
        *points += 1;
        if *points > MAX_POINTS {
            return false;
        }
        match diagram {
            Diagram::Diagram0(point) => {
                if point.generator.dimension == dimension {
                    if found.is_some() {
                        return false;
                    }
                    *found = Some(*point);
                }
                true
            }
            Diagram::DiagramN(diagram) => diagram
                .singular_slices()
                .all(|slice| visit(&slice, dimension, found, points)),
        }
    }

    let mut found = None;
    if !visit(diagram, dimension, &mut found, &mut 0) {
        return Cells::Many;
    }
    found.map_or(Cells::None, Cells::One)
}

/// The cell introduced by the given step of a diagram, if there is just the one.
fn cell_at(diagram: &DiagramN, step: usize) -> Option<Diagram0> {
    let layer = DiagramN::new(
        diagram.slice(Height::Regular(step))?,
        vec![diagram.cospans().get(step)?.clone()],
    );
    match cells(&layer.slice(Height::Singular(0))?, layer.dimension()) {
        Cells::One(cell) => Some(cell),
        _ => None,
    }
}

/// The cone of a rewrite that targets the given singular height, if it is not an identity there.
fn cone_at(rewrite: &Rewrite, height: usize) -> Option<ConeView<'_>> {
    let Rewrite::RewriteN(rewrite) = rewrite else {
        return None;
    };
    rewrite
        .cone_views()
        .into_iter()
        .find(|cone| cone.target() == height)
}

/// The single cone of a rewrite, as the height that it targets and how many it maps from, if the
/// rewrite is just that.
fn sole_cone(rewrite: &Rewrite) -> Option<(usize, usize)> {
    let Rewrite::RewriteN(rewrite) = rewrite else {
        return None;
    };
    match rewrite.cone_views().as_slice() {
        [cone] => Some((cone.target(), cone.source_len())),
        _ => None,
    }
}

/// `k` as the exponent of an iterated operation, if it is more than one.
fn iterated(k: usize) -> String {
    if k > 1 {
        format!("^{{{k}}}")
    } else {
        String::new()
    }
}

impl<'a> Describer<'a> {
    const fn new(signature: &'a Signature) -> Self {
        Self {
            signature,
            steps: 0,
        }
    }

    /// Accounts for looking at another diagram, returning whether there is any budget left.
    const fn tick(&mut self) -> bool {
        self.steps += 1;
        self.steps <= MAX_STEPS
    }

    /// The description (without dollar signs) of a diagram. `depth` is how many types it is in.
    fn diagram(&mut self, diagram: &Diagram, depth: usize) -> String {
        if !self.tick() {
            return ELLIPSIS.to_owned();
        }
        if let Some(known) = self.known(diagram) {
            return known;
        }
        let diagram = match diagram {
            Diagram::Diagram0(point) => return self.cell(*point),
            Diagram::DiagramN(diagram) => diagram,
        };
        self.identity(diagram, depth)
            .or_else(|| self.steps_of(diagram, depth))
            .unwrap_or_else(|| self.type_of(diagram, depth))
    }

    /// Generators, and the cells derived from them.
    fn known(&self, diagram: &Diagram) -> Option<String> {
        if let Some(info) = self.signature.iter().find(|info| info.diagram == *diagram) {
            return Some(to_math(&info.name));
        }
        self.structure(diagram.try_into().ok()?)
    }

    /// The cell derived from a generator that the diagram is, if it is one, as named in
    /// [`Structure`]: the reflections of the generator, and its bubbles.
    fn structure(&self, diagram: &DiagramN) -> Option<String> {
        let signature = self.signature;
        let info = signature.generator_info(diagram.max_generator().generator)?;
        let Diagram::DiagramN(generator) = &info.diagram else {
            return None;
        };
        let (n, k) = (diagram.dimension(), generator.dimension());
        let invertible = info.invertibility.is_invertible();

        if k == n {
            if n > MAX_REFLECTED_DIMENSION {
                return None;
            }
            let (directions, _) = generator
                .reflections()
                .into_iter()
                .find(|(_, reflected)| reflected == diagram)?;
            let mask = directions.iter().fold(0_u64, |mask, j| mask | 1 << (j - 1));
            let structure = if invertible && directions == [n] {
                Structure::Inverse
            } else {
                signature
                    .symmetries()
                    .iter()
                    .find(|symmetry| symmetry.mask() & ((1 << n) - 1) == mask)
                    .map_or_else(
                        || Structure::Reflection(directions),
                        |symmetry| Structure::Symmetry(symmetry.name.clone()),
                    )
            };
            return structure.math(&info.name);
        }

        if invertible && k < n && n - k <= MAX_BUBBLES {
            let bubble = |mut bubble: DiagramN| {
                while bubble.dimension() < n {
                    bubble = bubble.bubble().ok()?;
                }
                Some(bubble)
            };
            let structure = if bubble(generator.clone()).as_ref() == Some(diagram) {
                Structure::Bubble(n - k)
            } else if bubble(generator.inverse()).as_ref() == Some(diagram) {
                Structure::InverseBubble(n - k)
            } else {
                return None;
            };
            return structure.math(&info.name);
        }
        None
    }

    /// An identity, which is a diagram with no steps, or with one that does nothing; or such
    /// an identity of an identity.
    fn identity(&mut self, diagram: &DiagramN, depth: usize) -> Option<String> {
        let mut identities = 0;
        let mut inner = Diagram::from(diagram.clone());
        while let Diagram::DiagramN(identity) = &inner {
            match identity.cospans() {
                [] => {}
                [cospan] if cospan.is_identity() => {}
                _ => break,
            }
            identities += 1;
            inner = identity.source();
        }
        (identities > 0).then(|| {
            let inner = self.diagram(&inner, depth);
            format!(r"\mathrm{{id}}{}_{{{inner}}}", iterated(identities))
        })
    }

    /// A diagram as a composite of its steps, or as the one step that it is.
    fn steps_of(&mut self, diagram: &DiagramN, depth: usize) -> Option<String> {
        let size = diagram.size();
        if size == 1 {
            return self.step(diagram);
        }

        let shown = if size <= MAX_LAYERS { size } else { 3 };
        let cospans = diagram.cospans();
        let layer =
            |source: Diagram, step: usize| DiagramN::new(source, vec![cospans[step].clone()]);
        let mut parts = vec![];
        for (step, source) in diagram.regular_slices().take(shown).enumerate() {
            parts.push(self.step_or_identity(&layer(source, step), depth)?);
        }
        if size > shown {
            parts.push(r"\cdots".to_owned());
            if size <= MAX_SCAN {
                let source = diagram.slice(Height::Regular(size - 1))?;
                parts.push(self.step_or_identity(&layer(source, size - 1), depth)?);
            }
        }
        Some(parts.join(r" \mathbin{;} "))
    }

    fn step_or_identity(&mut self, layer: &DiagramN, depth: usize) -> Option<String> {
        if !self.tick() {
            return Some(ELLIPSIS.to_owned());
        }
        self.identity(layer, depth).or_else(|| self.step(layer))
    }

    /// A diagram with a single step: a cell with whatever is beside it, or a homotopy.
    fn step(&self, diagram: &DiagramN) -> Option<String> {
        let singular = diagram.slice(Height::Singular(0))?;
        match cells(&singular, diagram.dimension()) {
            Cells::Many => None,
            Cells::One(cell) => Some(self.cell(cell)),
            Cells::None => Some(
                self.homotopy(diagram)
                    .unwrap_or_else(|| HOMOTOPY.to_owned()),
            ),
        }
    }

    /// A step that is an exchange of two cells, the cancelling or creation of a cell with its
    /// reverse, if it is.
    fn homotopy(&self, diagram: &DiagramN) -> Option<String> {
        let cospan = diagram.cospans().first()?;
        let (Rewrite::RewriteN(forward), Rewrite::RewriteN(backward)) =
            (&cospan.forward, &cospan.backward)
        else {
            return None;
        };
        let (forward, backward) = (forward.cone_views(), backward.cone_views());
        let ([forward], [backward]) = (forward.as_slice(), backward.as_slice()) else {
            return None;
        };
        let source = DiagramN::try_from(diagram.source()).ok()?;
        let target = DiagramN::try_from(diagram.target()).ok()?;
        self.homotopy_at(&source, &target, forward, backward)
    }

    /// The homotopy that maps the steps of `source` that the `forward` cone is from, and those
    /// of `target` that the `backward` cone is from, to the one step.
    fn homotopy_at(
        &self,
        source: &DiagramN,
        target: &DiagramN,
        forward: &ConeView,
        backward: &ConeView,
    ) -> Option<String> {
        if forward.target() != backward.target() {
            return None;
        }
        let pair = |diagram: &DiagramN, cone: &ConeView| {
            let start = cone.source_start();
            Some((cell_at(diagram, start)?, cell_at(diagram, start + 1)?))
        };
        let reverses = |(first, second): (Diagram0, Diagram0)| {
            first.generator == second.generator
                && matches!(
                    (first.orientation, second.orientation),
                    (Orientation::Positive, Orientation::Negative)
                        | (Orientation::Negative, Orientation::Positive)
                )
        };
        match (forward.source_len(), backward.source_len()) {
            (2, 0) => {
                let pair = pair(source, forward).filter(|pair| reverses(*pair))?;
                Some(format!(r"\varepsilon_{{{}}}", self.cell(pair.0)))
            }
            (0, 2) => {
                let pair = pair(target, backward).filter(|pair| reverses(*pair))?;
                Some(format!(r"\eta_{{{}}}", self.cell(pair.1)))
            }
            (2, 2) => {
                let (lower, upper) = pair(source, forward)?;
                // Where the cell of the other step is put among those of the step in the
                // middle: to the left of the cell of this one, or to the right.
                let [lower_slice, upper_slice] = forward.singular_slices() else {
                    return None;
                };
                let upper_in_lower = sole_cone(lower_slice);
                let lower_in_upper = sole_cone(upper_slice);
                let name = |symbol: &str, sign: &str| {
                    format!(
                        "{symbol}{sign}_{{{},{}}}",
                        self.cell(lower),
                        self.cell(upper)
                    )
                };
                Some(match (lower_in_upper, upper_in_lower) {
                    // Neither cell is attached to anything, so nothing says which side.
                    (Some((lower_at, 0)), Some((upper_at, 0))) if lower_at < upper_at => {
                        name(r"\beta", "")
                    }
                    (Some((_, 0)), Some((_, 0))) => name(r"\beta", "^{-1}"),
                    _ => name(r"\mathrm{ex}", ""),
                })
            }
            _ => None,
        }
    }

    /// The exchange, braid, cancelling or creation of cells that is a vertex of the projection of
    /// a diagram, if that is what it is: the step of the diagram at the first height of the
    /// point, and what it does at the second.
    fn step_at(&self, diagram: &Diagram, point: &[SliceIndex]) -> Option<String> {
        use SliceIndex::Interior;

        let [Interior(Height::Singular(step)), Interior(Height::Singular(height))] = point else {
            return None;
        };
        let diagram = DiagramN::try_from(diagram.clone()).ok()?;
        let cospan = diagram.cospans().get(*step)?;
        let forward = cone_at(&cospan.forward, *height)?;
        let backward = cone_at(&cospan.backward, *height)?;
        let source = DiagramN::try_from(diagram.slice(Height::Regular(*step))?).ok()?;
        let target = DiagramN::try_from(diagram.slice(Height::Regular(*step + 1))?).ok()?;
        self.homotopy_at(&source, &target, &forward, &backward)
    }

    /// The name of a cell, with its orientation.
    fn cell(&self, cell: Diagram0) -> String {
        let (name, invertible) = self.signature.generator_info(cell.generator).map_or_else(
            || (format!("{:?}", cell.generator), false),
            |info| (info.name.clone(), info.invertibility.is_invertible()),
        );
        let name = to_math(&name);
        match cell.orientation {
            Orientation::Positive => name,
            Orientation::Negative if invertible => format!("{{{name}}}^{{-1}}"),
            Orientation::Negative if cell.generator.dimension == 1 => format!("{{{name}}}^{{*}}"),
            Orientation::Negative => format!(r"\overline{{{name}}}"),
            Orientation::Zero => format!(r"{HOMOTOPY}({name})"),
        }
    }

    /// A diagram as its type: the description of its source and its target.
    fn type_of(&mut self, diagram: &DiagramN, depth: usize) -> String {
        if depth >= MAX_DEPTH || diagram.size() > MAX_SCAN {
            return ELLIPSIS.to_owned();
        }
        let arrow = match diagram.dimension() {
            1 => r"\to",
            2 => r"\Rightarrow",
            _ => r"\Rrightarrow",
        };
        let source = self.diagram(&diagram.source(), depth + 1);
        let target = self.diagram(&diagram.target(), depth + 1);
        format!("{source} {arrow} {target}")
    }
}

#[cfg(test)]
mod tests {
    use homotopy_common::tex::{pieces, Piece};
    use homotopy_core::{
        signature::{Invertibility, Symmetry},
        Bias, Boundary, Diagram, Diagram0, DiagramN, Direction, Generator, Height, Orientation,
        SliceIndex,
    };

    use crate::proof::{Signature, SignatureEdit, SignatureItemEdit};

    /// `x` and the 1-cells `f`, `g : x → x` (`f` is invertible).
    struct Fixture {
        sig: Signature,
        x: Diagram0,
        f: DiagramN,
        g: DiagramN,
    }

    /// A generator with exactly the given name (the signature numbers them otherwise).
    fn cell(
        sig: &mut Signature,
        name: &str,
        source: Diagram,
        target: Diagram,
        invertibility: Invertibility,
    ) -> DiagramN {
        let diagram = sig
            .create_generator(source, target, name, invertibility)
            .unwrap();
        rename(sig, diagram.max_generator().generator, name);
        diagram
    }

    fn rename(sig: &mut Signature, generator: Generator, name: &str) {
        let node = sig.find_node(generator).unwrap();
        sig.update(&SignatureEdit::Edit(
            node,
            SignatureItemEdit::Rename(name.to_owned()),
        ))
        .unwrap();
    }

    fn fixture() -> Fixture {
        let mut sig = Signature::default();
        let x = sig.create_generator_zero("x");
        rename(&mut sig, x.generator, "x");
        let (source, target) = (Diagram::from(x), Diagram::from(x));
        let f = cell(
            &mut sig,
            "f",
            source.clone(),
            target.clone(),
            Invertibility::Invertible,
        );
        let g = cell(&mut sig, "g", source, target, Invertibility::Directed);
        Fixture { sig, x, f, g }
    }

    fn weak(diagram: &DiagramN) -> DiagramN {
        Diagram::from(diagram.clone()).weak_identity()
    }

    fn describe(sig: &Signature, diagram: impl Into<Diagram>) -> String {
        sig.describe(&diagram.into())
    }

    fn single_formula(label: &str) -> bool {
        matches!(pieces(label).as_slice(), [Piece::Math(_)])
    }

    #[test]
    fn generators_are_named_by_themselves() {
        let Fixture { sig, x, f, .. } = fixture();
        assert_eq!(describe(&sig, f), "f");
        assert_eq!(describe(&sig, x), "x");
    }

    #[test]
    fn identities_are_named_for_what_they_are_on() {
        let Fixture { sig, x, f, g } = fixture();
        assert_eq!(describe(&sig, weak(&f)), r"$\mathrm{id}_{\text{f}}$");
        assert_eq!(
            describe(&sig, f.clone().identity()),
            describe(&sig, weak(&f))
        );
        assert_eq!(
            describe(&sig, Diagram::from(x).identity()),
            r"$\mathrm{id}_{\text{x}}$"
        );
        // An identity of an identity is a raised one, as the attach menu names it.
        let twice = Diagram::from(weak(&f)).weak_identity();
        assert_eq!(describe(&sig, twice), r"$\mathrm{id}^{2}_{\text{f}}$");
        // Identities on anything, not only on generators.
        let fg = f.attach(&g, Boundary::Target, &[]).unwrap();
        assert_eq!(
            describe(&sig, fg.identity()),
            r"$\mathrm{id}_{\text{f} \mathbin{;} \text{g}}$"
        );
    }

    #[test]
    fn cells_derived_from_generators_are_named_like_the_attach_menu() {
        let Fixture { mut sig, f, g, .. } = fixture();
        assert_eq!(describe(&sig, f.inverse()), r"${\text{f}}^{-1}$");
        assert_eq!(
            describe(&sig, weak(&f.inverse())),
            r"$\mathrm{id}_{{\text{f}}^{-1}}$"
        );
        // The reverse of a directed 1-cell is its dual, or the cell of a symmetry.
        assert_eq!(describe(&sig, g.inverse()), r"${\text{g}}^{*}$");
        sig.update(&SignatureEdit::AddSymmetry(Symmetry {
            name: "op".to_owned(),
            directions: vec![1],
        }))
        .unwrap();
        assert_eq!(describe(&sig, g.inverse()), r"${\text{g}}^{\text{op}}$");
        // The bubbles of an invertible cell.
        assert_eq!(
            describe(&sig, f.bubble().unwrap()),
            r"$\mathrm{bub}_{\text{f}}$"
        );
        assert_eq!(
            describe(&sig, f.inverse().bubble().unwrap()),
            r"$\mathrm{bub}_{{\text{f}}^{-1}}$"
        );
    }

    #[test]
    fn composites_are_in_the_order_of_the_diagram() {
        let Fixture { sig, f, g, .. } = fixture();
        let fg = f.attach(&g, Boundary::Target, &[]).unwrap();
        assert_eq!(
            describe(&sig, fg.clone()),
            r"$\text{f} \mathbin{;} \text{g}$"
        );
        let gf = g.attach(&f, Boundary::Target, &[]).unwrap();
        assert_eq!(describe(&sig, gf), r"$\text{g} \mathbin{;} \text{f}$");
        let fgg = fg.attach(&g, Boundary::Target, &[]).unwrap();
        assert_eq!(
            describe(&sig, fgg),
            r"$\text{f} \mathbin{;} \text{g} \mathbin{;} \text{g}$"
        );
        // Long ones are cut short.
        let mut long = g.clone();
        for _ in 0..99 {
            long = long.attach(&g, Boundary::Target, &[]).unwrap();
        }
        let long = describe(&sig, long);
        assert!(long.contains(r"\cdots"), "{long}");
        assert!(long.len() < 100, "{long}");
        assert!(single_formula(&long));
    }

    /// `a`, `b : x → x`, `α : a ⇒ a`, `β : b ⇒ b`; `a ; b` with `α` and `β` attached to it,
    /// the 3-diagram exchanging them (not yet merged), and the diagram with both at once.
    struct Exchange {
        sig: Signature,
        alpha_beta: DiagramN,
        wrench: DiagramN,
        parallel: DiagramN,
        to_parallel: DiagramN,
    }

    /// The 3-diagram that exchanges the cells of `diagram`, which has two steps, through
    /// the diagram with both in one step, with a contraction followed by an expansion; and the
    /// ways to do that.
    fn exchanges(
        sig: &Signature,
        diagram: &DiagramN,
        bias: Option<Bias>,
    ) -> (DiagramN, Vec<DiagramN>) {
        let to_parallel = diagram
            .clone()
            .identity()
            .contract(
                Boundary::Target.into(),
                &mut [],
                0,
                Direction::Forward,
                1,
                bias,
                sig,
            )
            .unwrap();
        let parallel = DiagramN::try_from(to_parallel.target()).unwrap();
        let mut wrenches = vec![];
        for direction in [Direction::Forward, Direction::Backward] {
            for h in 0..2 {
                let Ok(from_parallel) = parallel.clone().identity().expand(
                    Boundary::Target.into(),
                    &mut [],
                    [Height::Singular(0), Height::Singular(h)],
                    direction,
                    sig,
                ) else {
                    continue;
                };
                if DiagramN::try_from(from_parallel.target()).unwrap() == *diagram {
                    continue;
                }
                wrenches.push(DiagramN::new(
                    to_parallel.source(),
                    to_parallel
                        .cospans()
                        .iter()
                        .chain(from_parallel.cospans())
                        .cloned()
                        .collect(),
                ));
            }
        }
        (to_parallel, wrenches)
    }

    fn exchange() -> Exchange {
        let Fixture { mut sig, x, .. } = fixture();
        let directed = Invertibility::Directed;
        let (a, b) = (
            cell(&mut sig, "a", x.into(), x.into(), directed),
            cell(&mut sig, "b", x.into(), x.into(), directed),
        );
        let (a_a, b_b) = (Diagram::from(a.clone()), Diagram::from(b.clone()));
        let alpha = cell(&mut sig, "α", a_a.clone(), a_a, directed);
        let beta = cell(&mut sig, "β", b_b.clone(), b_b, directed);
        let ab = a.attach(&b, Boundary::Target, &[]).unwrap();
        let alpha_beta = ab
            .identity()
            .attach(&alpha, Boundary::Target, &[0])
            .unwrap()
            .attach(&beta, Boundary::Target, &[1])
            .unwrap();
        let (to_parallel, wrenches) = exchanges(&sig, &alpha_beta, None);
        let parallel = DiagramN::try_from(to_parallel.target()).unwrap();
        Exchange {
            sig,
            alpha_beta,
            wrench: wrenches.into_iter().next().unwrap(),
            parallel,
            to_parallel,
        }
    }

    #[test]
    fn whiskered_cells_are_named_by_the_cell() {
        let Exchange {
            sig, alpha_beta, ..
        } = exchange();
        // The first step of the diagram is the cell α, beside a wire.
        let first = DiagramN::new(alpha_beta.source(), vec![alpha_beta.cospans()[0].clone()]);
        assert_eq!(describe(&sig, first), r"$\text{α}$");
        assert_eq!(
            describe(&sig, alpha_beta),
            r"$\text{α} \mathbin{;} \text{β}$"
        );
    }

    #[test]
    fn exchanges_are_named_for_the_cells_in_the_order_of_the_source() {
        let Exchange { sig, wrench, .. } = exchange();
        // The steps are a contraction and an expansion.
        assert_eq!(
            describe(&sig, wrench.clone()),
            r"$\mathrm{h} \mathbin{;} \mathrm{h}$"
        );
        let (merged, merges) = wrench.merge_exchanges(&sig);
        assert_eq!(merges, 1);
        assert_eq!(describe(&sig, merged), r"$\mathrm{ex}_{\text{α},\text{β}}$");
    }

    #[test]
    fn homotopies_that_are_not_recognised_are_just_homotopies() {
        let Exchange {
            sig, to_parallel, ..
        } = exchange();
        assert_eq!(describe(&sig, to_parallel), r"$\mathrm{h}$");
    }

    #[test]
    fn what_is_not_recognised_is_described_by_its_type() {
        let Exchange { sig, parallel, .. } = exchange();
        // Two cells in the same step.
        assert_eq!(
            describe(&sig, parallel),
            r"$\text{a} \mathbin{;} \text{b} \Rightarrow \text{a} \mathbin{;} \text{b}$"
        );
    }

    /// Two cells `s`, `t : id x ⇒ id x` that are attached to nothing, and the exchanges of them.
    #[test]
    fn exchanges_of_cells_attached_to_nothing_are_braids() {
        let Fixture { mut sig, x, .. } = fixture();
        let id = Diagram::from(x).identity();
        let directed = Invertibility::Directed;
        let s = cell(
            &mut sig,
            "s",
            id.clone().into(),
            id.clone().into(),
            directed,
        );
        let t = cell(&mut sig, "t", id.clone().into(), id.into(), directed);
        let st = s.attach(&t, Boundary::Target, &[]).unwrap();
        let mut names = vec![];
        let wrenches = [Bias::Lower, Bias::Higher]
            .into_iter()
            .flat_map(|bias| exchanges(&sig, &st, Some(bias)).1);
        for wrench in wrenches {
            let (merged, merges) = wrench.merge_exchanges(&sig);
            assert_eq!(merges, 1);
            names.push(describe(&sig, merged));
        }
        names.sort();
        names.dedup();
        // There are two ways for the cells to pass each other.
        assert_eq!(
            names,
            vec![
                r"$\beta^{-1}_{\text{s},\text{t}}$".to_owned(),
                r"$\beta_{\text{s},\text{t}}$".to_owned()
            ]
        );
    }

    /// `f` and its reverse cancelling or being created, in the steps of a diagram.
    #[test]
    fn cancelling_and_creating_a_cell_with_its_reverse() {
        let Fixture { sig, f, g, .. } = fixture();
        let contract = |diagram: DiagramN, boundary: Boundary, height| {
            diagram
                .identity()
                .contract(
                    boundary.into(),
                    &mut [],
                    height,
                    Direction::Forward,
                    1,
                    None,
                    &sig,
                )
                .unwrap()
        };
        let f_f_inverse = f.attach(&f.inverse(), Boundary::Target, &[]).unwrap();
        let inverse_f = f.inverse().attach(&f, Boundary::Target, &[]).unwrap();

        let cap = contract(f_f_inverse.clone(), Boundary::Target, 0);
        assert_eq!(describe(&sig, cap), r"$\varepsilon_{\text{f}}$");
        let cap = contract(inverse_f.clone(), Boundary::Target, 0);
        assert_eq!(describe(&sig, cap), r"$\varepsilon_{{\text{f}}^{-1}}$");

        // The cups are the bubbles of the cell.
        let cup = contract(f_f_inverse.clone(), Boundary::Source, 0);
        assert_eq!(describe(&sig, cup), r"$\mathrm{bub}_{\text{f}}$");

        // Beside a wire they are the unit and counit of the cell.
        let wire = g.attach(&f_f_inverse, Boundary::Target, &[]).unwrap();
        let cap = contract(wire.clone(), Boundary::Target, 1);
        assert_eq!(describe(&sig, cap), r"$\varepsilon_{\text{f}}$");
        let cup = contract(wire, Boundary::Source, 1);
        assert_eq!(describe(&sig, cup), r"$\eta_{{\text{f}}^{-1}}$");
        let wire = g.attach(&inverse_f, Boundary::Target, &[]).unwrap();
        let cup = contract(wire, Boundary::Source, 1);
        assert_eq!(describe(&sig, cup), r"$\eta_{\text{f}}$");
    }

    #[test]
    fn points_of_a_diagram_are_named() {
        let Fixture { sig, f, .. } = fixture();
        let f_f_inverse = f.attach(&f.inverse(), Boundary::Target, &[]).unwrap();
        let cap: Diagram = f_f_inverse
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
            .into();
        let generator = f.max_generator().generator;
        let cell = |orientation| Diagram0 {
            generator,
            orientation,
        };
        let at = |step, height| {
            [
                SliceIndex::Interior(Height::Singular(step)),
                SliceIndex::Interior(Height::Singular(height)),
            ]
        };
        let point = at(0, 0);
        // On wires and surfaces, the cell is named as drawn.
        assert_eq!(
            sig.describe_cell(cell(Orientation::Positive)),
            r"$\text{f}$"
        );
        assert_eq!(
            sig.describe_cell(cell(Orientation::Negative)),
            r"${\text{f}}^{-1}$"
        );
        // Where no wire is, a step of the diagram is.
        assert_eq!(
            sig.describe_point(&cap, &at(0, 1), cell(Orientation::Positive)),
            r"$\text{f}$"
        );
        assert_eq!(
            sig.describe_point(&cap, &point, cell(Orientation::Zero)),
            r"$\varepsilon_{\text{f}}$"
        );
        // Anywhere else, or without knowing, the cell is just a homotopy.
        assert_eq!(
            sig.describe_point(&cap, &at(0, 1), cell(Orientation::Zero)),
            r"$\mathrm{h}(\text{f})$"
        );
        assert_eq!(
            sig.describe_point(&cap, &[], cell(Orientation::Zero)),
            r"$\mathrm{h}(\text{f})$"
        );
    }

    fn vertex(step: usize, height: usize) -> [SliceIndex; 2] {
        [
            SliceIndex::Interior(Height::Singular(step)),
            SliceIndex::Interior(Height::Singular(height)),
        ]
    }

    #[test]
    fn crossings_in_projections_are_named_for_the_step() {
        let Exchange { sig, wrench, .. } = exchange();
        let (merged, _) = wrench.merge_exchanges(&sig);
        let merged = Diagram::from(merged);
        // The vertex is drawn as one of the two cells; its dimension is below that of the
        // diagram, so it is the step that is named.
        let alpha = sig.iter().find(|info| info.name == "α").unwrap().generator;
        let beta = sig.iter().find(|info| info.name == "β").unwrap().generator;
        let drawn = |generator| Diagram0 {
            generator,
            orientation: Orientation::Positive,
        };
        for generator in [alpha, beta] {
            assert_eq!(
                sig.describe_point(&merged, &vertex(0, 0), drawn(generator)),
                r"$\mathrm{ex}_{\text{α},\text{β}}$"
            );
        }
        // Not anywhere else: where nothing happens (there is no height 1), it is the cell.
        assert_eq!(
            sig.describe_point(&merged, &vertex(0, 1), drawn(alpha)),
            r"$\text{α}$"
        );
        // And a wire or surface is named by its cell.
        assert_eq!(sig.describe_cell(drawn(alpha)), r"$\text{α}$");
    }

    #[test]
    fn a_cell_of_the_dimension_of_the_diagram_is_not_a_step() {
        let Exchange {
            mut sig, wrench, ..
        } = exchange();
        let (merged, _) = wrench.merge_exchanges(&sig);
        // A 3-cell from one order of the cells to the other, whose cones look like the exchange.
        let directed = Invertibility::Directed;
        let gamma = cell(&mut sig, "γ", merged.source(), merged.target(), directed);
        let drawn = Diagram0 {
            generator: gamma.max_generator().generator,
            orientation: Orientation::Positive,
        };
        assert_eq!(
            sig.describe_point(&gamma.into(), &vertex(0, 0), drawn),
            r"$\text{γ}$"
        );
    }

    #[test]
    fn braids_in_projections_are_named_with_the_side_they_pass_on() {
        let Fixture { mut sig, x, .. } = fixture();
        let id = Diagram::from(x).identity();
        let directed = Invertibility::Directed;
        let s = cell(
            &mut sig,
            "s",
            id.clone().into(),
            id.clone().into(),
            directed,
        );
        let t = cell(&mut sig, "t", id.clone().into(), id.into(), directed);
        let st = s.attach(&t, Boundary::Target, &[]).unwrap();
        let wrenches = [Bias::Lower, Bias::Higher]
            .into_iter()
            .flat_map(|bias| exchanges(&sig, &st, Some(bias)).1);
        let mut points = vec![];
        for wrench in wrenches {
            let (merged, _) = wrench.merge_exchanges(&sig);
            let drawn = Diagram0 {
                generator: s.max_generator().generator,
                orientation: Orientation::Positive,
            };
            let merged = Diagram::from(merged);
            let point = sig.describe_point(&merged, &vertex(0, 0), drawn);
            assert_eq!(point, sig.describe(&merged));
            points.push(point);
        }
        points.sort();
        points.dedup();
        assert_eq!(
            points,
            vec![
                r"$\beta^{-1}_{\text{s},\text{t}}$".to_owned(),
                r"$\beta_{\text{s},\text{t}}$".to_owned()
            ]
        );
    }

    #[test]
    fn descriptions_are_single_formulas_whatever_the_names() {
        let Fixture { mut sig, f, g, .. } = fixture();
        for (generator, name) in [(&f, "a $x"), (&g, "$"), (&f, "}{")] {
            rename(&mut sig, generator.max_generator().generator, name);
            let fg = f.attach(&g, Boundary::Target, &[]).unwrap();
            for diagram in [
                f.inverse().into(),
                weak(&f).into(),
                fg.clone().into(),
                fg.identity().into(),
                f.bubble().unwrap().into(),
            ] {
                let label = sig.describe(&diagram);
                assert!(single_formula(&label), "{label:?} for {name:?}");
            }
        }
    }

    #[test]
    fn big_diagrams_are_described_in_bounded_time() {
        let Fixture { sig, f, g, .. } = fixture();
        let mut long = f;
        for _ in 0..600 {
            long = long.attach(&g, Boundary::Target, &[]).unwrap();
        }
        let long = long.identity();
        let start = std::time::Instant::now();
        let label = sig.describe(&long.into());
        assert!(start.elapsed() < std::time::Duration::from_millis(500));
        assert!(label.len() < 200, "{label}");
    }
}
