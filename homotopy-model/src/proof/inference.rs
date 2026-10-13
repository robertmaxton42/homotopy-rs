//! Inferring how to instantiate a schema so that it can be attached at a point of a diagram.
//!
//! Each placeholder is solved in turn, from the lowest dimension up, so that its type is known
//! when it is solved. Its candidates are generated from the cells of the diagram being attached
//! to: the 0-cells of that diagram for a 0-cell placeholder, and otherwise the composites of up to
//! [`MAX_CELLS`] of its cells of the placeholder's dimension, between the placeholder's source and
//! target. A substitution is kept if the instantiated schema then attaches at the given point.

use homotopy_common::hash::FastHashSet;
use homotopy_core::{
    signature::{Invertibility, Signature as _},
    substitution::substitute,
    Boundary, Diagram, DiagramN, Generator, Height,
};

use super::Signature;

/// The most cells in a composite replacing a placeholder.
const MAX_CELLS: usize = 3;
/// The most candidates considered for each placeholder.
const MAX_CANDIDATES: usize = 64;
/// The most complete substitutions tried.
const MAX_SUBSTITUTIONS: usize = 512;

/// A way of attaching an instance of a schema, found by [`Signature::infer_instances`].
#[derive(Clone, Debug)]
pub struct InferredInstance {
    /// The substitution to instantiate the schema with.
    pub substitution: Vec<(Generator, Diagram)>,
    /// The diagram of the instance, labelled with the schema's generator.
    pub diagram: DiagramN,
    /// Where to attach it.
    pub embedding: Vec<usize>,
}

struct Search<'a> {
    signature: &'a Signature,
    schema: Generator,
    schema_diagram: DiagramN,
    parameters: Vec<Generator>,
    haystack: &'a Diagram,
    boundary: Boundary,
    point: &'a [Height],
    /// The diagrams of the cells of the haystack (and their inverses where invertible).
    cells: Vec<DiagramN>,
    zero_cells: Vec<Diagram>,
    tried: usize,
    results: Vec<InferredInstance>,
}

impl Signature {
    /// The ways to instantiate a schema so that it can be attached to `haystack` at `point`,
    /// attaching to its `boundary` (as in the attach menu). Instances of generators that the
    /// schema depends on must already exist for the instance to match.
    #[must_use]
    pub fn infer_instances(
        &self,
        schema: Generator,
        haystack: &Diagram,
        boundary: Boundary,
        point: &[Height],
    ) -> Vec<InferredInstance> {
        let parameters = self.parameters(schema);
        let Some(Diagram::DiagramN(schema_diagram)) =
            self.generator_info(schema).map(|info| info.diagram.clone())
        else {
            return vec![];
        };
        if parameters.is_empty() || schema_diagram.dimension() != haystack.dimension() + 1 {
            return vec![];
        }

        let mut cells = vec![];
        let mut zero_cells = vec![];
        for g in haystack.generators().into_keys() {
            match self
                .generator_info(g)
                .map(|info| (&info.diagram, info.invertibility))
            {
                Some((Diagram::Diagram0(d), _)) => zero_cells.push((*d).into()),
                Some((Diagram::DiagramN(d), invertibility)) => {
                    cells.push(d.clone());
                    if invertibility != Invertibility::Directed {
                        cells.push(d.inverse());
                    }
                }
                None => {}
            }
        }

        let mut search = Search {
            signature: self,
            schema,
            schema_diagram,
            parameters,
            haystack,
            boundary,
            point,
            cells,
            zero_cells,
            tried: 0,
            results: vec![],
        };
        search.solve(&mut vec![]);
        search.results
    }
}

impl Search<'_> {
    fn solve(&mut self, substitution: &mut Vec<(Generator, Diagram)>) {
        if self.tried >= MAX_SUBSTITUTIONS {
            return;
        }
        let Some(&placeholder) = self.parameters.get(substitution.len()) else {
            self.tried += 1;
            self.check(substitution);
            return;
        };
        for candidate in self.candidates(placeholder, substitution) {
            substitution.push((placeholder, candidate));
            self.solve(substitution);
            substitution.pop();
        }
    }

    /// Applies a partial substitution to a diagram mentioned by `generator`, using the existing
    /// instances of the generators it depends on. Unless `strict`, a missing instance is stood in
    /// for by the dependent generator with a substituted diagram, which is enough for a preview:
    /// the actual instance is only created when attaching.
    fn substitute(
        &self,
        generator: Generator,
        substitution: &[(Generator, Diagram)],
        diagram: &Diagram,
        strict: bool,
    ) -> Option<Diagram> {
        let substitution = self.signature.normalise(substitution);
        let mut replacements = substitution.clone();
        let mentioned = diagram.generators();
        for (b, restricted) in self.signature.dependents(generator, &substitution) {
            let restricted = self.signature.normalise(&restricted);
            if restricted.is_empty() || !mentioned.contains_key(&b) {
                continue;
            }
            let diagram = match self.signature.find_instance(b, &restricted) {
                Some(instance) => self.signature.generator_info(instance)?.diagram.clone(),
                None if strict => return None,
                None => {
                    let Diagram::DiagramN(own) = &self.signature.generator_info(b)?.diagram else {
                        return None;
                    };
                    DiagramN::from_generator(
                        b,
                        self.substitute(b, &restricted, &own.source(), false)?,
                        self.substitute(b, &restricted, &own.target(), false)?,
                    )
                    .ok()?
                    .into()
                }
            };
            replacements.push((b, diagram));
        }
        substitute(diagram, self.signature, &|g| {
            replacements
                .iter()
                .find(|(h, _)| *h == g)
                .map(|(_, d)| d.clone())
        })
        .ok()
    }

    /// The candidate replacements for a placeholder, given replacements for those before it.
    // `DiagramN` is hash-consed; its `OnceCell` caches do not affect `Hash`/`Eq`.
    #[allow(clippy::mutable_key_type)]
    fn candidates(
        &self,
        placeholder: Generator,
        substitution: &[(Generator, Diagram)],
    ) -> Vec<Diagram> {
        let Some(own) = self
            .signature
            .generator_info(placeholder)
            .map(|info| info.diagram.clone())
        else {
            return vec![];
        };
        let Diagram::DiagramN(own_n) = &own else {
            return self
                .zero_cells
                .iter()
                .filter(|&d| *d != own)
                .take(MAX_CANDIDATES)
                .cloned()
                .collect();
        };
        let (Some(source), Some(target)) = (
            self.substitute(placeholder, substitution, &own_n.source(), true),
            self.substitute(placeholder, substitution, &own_n.target(), true),
        ) else {
            return vec![];
        };

        let k = own_n.dimension();
        let cells: Vec<&DiagramN> = self.cells.iter().filter(|c| c.dimension() == k).collect();
        let mut seen = FastHashSet::default();
        let mut results = vec![];
        let mut frontier = vec![source.identity()];
        for depth in 0..=MAX_CELLS {
            let mut next = vec![];
            for composite in frontier {
                if !seen.insert(composite.clone()) {
                    continue;
                }
                if composite.target() == target && Diagram::from(composite.clone()) != own {
                    results.push(composite.clone().into());
                    if results.len() >= MAX_CANDIDATES {
                        return results;
                    }
                }
                if depth == MAX_CELLS {
                    continue;
                }
                for cell in &cells {
                    for embedding in composite.target().embeddings(&cell.source()) {
                        if let Ok(extended) = composite.attach(cell, Boundary::Target, &embedding) {
                            next.push(extended);
                        }
                    }
                }
            }
            frontier = next;
        }
        results
    }

    /// Checks whether a complete substitution lets the schema attach at the point.
    fn check(&mut self, substitution: &[(Generator, Diagram)]) {
        // The side attached along must match the diagram, so it can only use existing instances;
        // instances on the other side may be created when attaching.
        let strict_source = self.boundary == Boundary::Target;
        let source = self.schema_diagram.source();
        let target = self.schema_diagram.target();
        let (Some(source), Some(target)) = (
            self.substitute(self.schema, substitution, &source, strict_source),
            self.substitute(self.schema, substitution, &target, !strict_source),
        ) else {
            return;
        };
        let Ok(diagram) = DiagramN::from_generator(self.schema, source, target) else {
            return;
        };
        let Some(needle) = diagram.slice(self.boundary.flip()) else {
            return;
        };
        for embedding in self.haystack.embeddings(&needle) {
            if needle.contains_point(self.point, &embedding) {
                self.results.push(InferredInstance {
                    substitution: self.signature.normalise(substitution),
                    diagram: diagram.clone(),
                    embedding,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use homotopy_core::{
        signature::{Invertibility, Signature as _},
        Boundary, Diagram, DiagramN, Generator, Height,
    };

    use crate::proof::{Signature, SignatureEdit, SignatureItemEdit};

    fn generator(d: &DiagramN) -> Generator {
        d.max_generator().generator
    }

    fn make_placeholder(sig: &mut Signature, g: Generator) {
        let node = sig.find_node(g).unwrap();
        sig.update(&SignatureEdit::Edit(
            node,
            SignatureItemEdit::MakePlaceholder(true),
        ))
        .unwrap();
    }

    /// `x`; placeholder `P : x → x`; `Q, R : x → x`; the schema `A : P ⇒ P`; and `Q ≫ R`.
    fn setup() -> (
        Signature,
        Generator,
        DiagramN,
        DiagramN,
        Generator,
        DiagramN,
    ) {
        let mut sig = Signature::default();
        let x: Diagram = sig.create_generator_zero("x").into();
        let mut cell = |s: Diagram, t: Diagram| {
            sig.create_generator(s, t, "c", Invertibility::Directed)
                .unwrap()
        };
        let p = cell(x.clone(), x.clone());
        let q = cell(x.clone(), x.clone());
        let r = cell(x.clone(), x);
        let a = cell(p.clone().into(), p.clone().into());
        let qr = q.attach(&r, Boundary::Target, &[]).unwrap();
        let (p, a) = (generator(&p), generator(&a));
        make_placeholder(&mut sig, p);
        (sig, p, q, qr, a, r)
    }

    #[test]
    fn infers_single_cells_and_composites_at_the_point() {
        let (sig, p, q, qr, a, r) = setup();
        let haystack: Diagram = qr.clone().into();

        let replacements = |point: usize| {
            let mut found: Vec<Diagram> = sig
                .infer_instances(a, &haystack, Boundary::Target, &[Height::Singular(point)])
                .into_iter()
                .map(|inferred| {
                    assert_eq!(inferred.substitution.len(), 1);
                    assert_eq!(inferred.substitution[0].0, p);
                    inferred.substitution[0].1.clone()
                })
                .collect();
            found.sort_by_key(|d| format!("{d:?}"));
            found
        };

        let mut expected: Vec<Diagram> = vec![q.into(), qr.clone().into()];
        expected.sort_by_key(|d| format!("{d:?}"));
        assert_eq!(replacements(0), expected);

        let mut expected: Vec<Diagram> = vec![r.into(), qr.into()];
        expected.sort_by_key(|d| format!("{d:?}"));
        assert_eq!(replacements(1), expected);
    }

    #[test]
    fn inferred_instances_attach() {
        let (mut sig, _, _, qr, a, _) = setup();
        let haystack: Diagram = qr.clone().into();
        let inferred = sig
            .infer_instances(a, &haystack, Boundary::Target, &[Height::Singular(0)])
            .into_iter()
            .find(|inferred| inferred.substitution[0].1 == qr.clone().into())
            .unwrap();
        let instance = sig.instantiate(a, &inferred.substitution).unwrap();
        let Diagram::DiagramN(diagram) = sig.generator_info(instance).unwrap().diagram.clone()
        else {
            unreachable!()
        };
        // The instance's source is Q ≫ R, so it attaches where it was found.
        let attached = qr
            .clone()
            .identity()
            .attach(&diagram, Boundary::Target, &inferred.embedding)
            .unwrap();
        assert_eq!(attached.target(), qr.into());
    }
}
