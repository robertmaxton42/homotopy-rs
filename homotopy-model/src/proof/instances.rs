//! Placeholders and instances of the generators that mention them (axiom schemas).
//!
//! An instance `A[σ]` of a generator `A` is a new generator whose diagram is that of `A` with
//! each placeholder `P` replaced by `σ(P)`. Generators in the boundary of `A` that mention an
//! instantiated placeholder are instantiated along with it. Instances are listed under their
//! schema in the signature, and are otherwise ordinary generators: nothing ever turns a diagram
//! back into a placeholder.

use homotopy_core::{
    diagram::NewDiagramError,
    signature::Signature as _,
    substitution::{substitute, SubstitutionError},
    Diagram, DiagramN, Generator,
};
use thiserror::Error;

use super::{
    generators::{GeneratorInfo, Instance},
    Signature, SignatureItem,
};

#[derive(Debug, Error)]
pub enum InstantiationError {
    #[error("selected a generator that is not in the signature")]
    UnknownGenerator,
    #[error("{0} is not a placeholder mentioned by {1}")]
    NotAParameter(String, String),
    #[error(transparent)]
    SubstitutionError(#[from] SubstitutionError),
    #[error("the replacement for {0} does not have the same type")]
    IllTyped(String),
    #[error(transparent)]
    NewDiagramError(#[from] NewDiagramError),
}

impl Signature {
    /// The placeholders mentioned by a generator, i.e. those it can be instantiated at.
    #[must_use]
    pub fn parameters(&self, generator: Generator) -> Vec<Generator> {
        let Some(info) = self.generator_info(generator) else {
            return vec![];
        };
        let mut parameters: Vec<_> = info
            .diagram
            .generators()
            .into_keys()
            .filter(|&g| {
                g != generator && self.generator_info(g).is_some_and(|info| info.placeholder)
            })
            .collect();
        parameters.sort_by_key(|g| (g.dimension, g.id));
        parameters
    }

    /// Instantiates a generator by replacing placeholders it mentions with diagrams, returning
    /// the instance (an existing one if the same instance was made before).
    ///
    /// Each placeholder can be replaced by any diagram whose source and target are those of the
    /// placeholder after substitution. Substitution needs each height of the diagrams involved to
    /// be a single generator (see [`substitute`]).
    pub fn instantiate(
        &mut self,
        schema: Generator,
        substitution: &[(Generator, Diagram)],
    ) -> Result<Generator, InstantiationError> {
        let info = self
            .generator_info(schema)
            .cloned()
            .ok_or(InstantiationError::UnknownGenerator)?;
        let parameters = self.parameters(schema);

        // Normalise the substitution, dropping trivial entries.
        let mut substitution: Vec<(Generator, Diagram)> = substitution
            .iter()
            .filter(|(p, d)| {
                self.generator_info(*p)
                    .is_none_or(|info| info.diagram != *d)
            })
            .cloned()
            .collect();
        substitution.sort_by_key(|(p, _)| (p.dimension, p.id));
        substitution.dedup_by_key(|(p, _)| *p);
        if substitution.is_empty() {
            return Ok(schema);
        }

        for (p, _) in &substitution {
            if !parameters.contains(p) {
                return Err(InstantiationError::NotAParameter(
                    self.name_of(*p),
                    info.name,
                ));
            }
        }
        let mut replacements = substitution.clone();

        // Instantiate the generators in the boundary that mention instantiated placeholders.
        let mut dependents: Vec<Generator> = info
            .diagram
            .generators()
            .into_keys()
            .filter(|&g| {
                g != schema
                    && self.generator_info(g).is_some_and(|info| !info.placeholder)
                    && self
                        .parameters(g)
                        .iter()
                        .any(|p| substitution.iter().any(|(q, _)| q == p))
            })
            .collect();
        dependents.sort_by_key(|g| (g.dimension, g.id));
        for b in dependents {
            let parameters = self.parameters(b);
            let restricted: Vec<_> = substitution
                .iter()
                .filter(|(p, _)| parameters.contains(p))
                .cloned()
                .collect();
            let instance = self.instantiate(b, &restricted)?;
            let diagram = self.generator_info(instance).unwrap().diagram.clone();
            replacements.push((b, diagram));
        }
        let substitute = |diagram: &Diagram| {
            substitute(diagram, self, &|g| {
                replacements
                    .iter()
                    .find(|(h, _)| *h == g)
                    .map(|(_, d)| d.clone())
            })
        };

        // Each replacement must have the type of the placeholder after substitution.
        for (p, replacement) in &substitution {
            let well_typed = match (&self.generator_info(*p).unwrap().diagram, replacement) {
                (Diagram::Diagram0(_), Diagram::Diagram0(_)) => true,
                (Diagram::DiagramN(p), Diagram::DiagramN(r)) => {
                    p.dimension() == r.dimension()
                        && substitute(&p.source())? == r.source()
                        && substitute(&p.target())? == r.target()
                }
                _ => false,
            };
            if !well_typed {
                return Err(InstantiationError::IllTyped(self.name_of(*p)));
            }
        }

        let instance = Instance {
            schema,
            substitution: substitution.clone(),
        };
        if let Some(existing) = self
            .iter()
            .find(|info| info.instance.as_ref() == Some(&instance))
        {
            return Ok(existing.generator);
        }

        let Diagram::DiagramN(diagram) = &info.diagram else {
            return Err(InstantiationError::NotAParameter(
                self.name_of(substitution[0].0),
                info.name,
            ));
        };
        let generator = Generator::new(self.next_generator_id(), schema.dimension);
        let diagram = DiagramN::from_generator(
            generator,
            substitute(&diagram.source())?,
            substitute(&diagram.target())?,
        )?;

        let arguments = substitution
            .iter()
            .map(|(p, d)| format!("{}/{}", self.describe(d), self.name_of(*p)))
            .collect::<Vec<_>>()
            .join(", ");
        let node = self.find_node(schema).unwrap();
        self.insert_item_under(
            node,
            SignatureItem::Item(GeneratorInfo {
                generator,
                name: format!("{}[{arguments}]", info.name),
                diagram: diagram.into(),
                placeholder: false,
                instance: Some(instance),
                instances_open: false,
                ..info
            }),
        );
        Ok(generator)
    }

    fn name_of(&self, g: Generator) -> String {
        self.generator_info(g)
            .map_or_else(|| format!("{g:?}"), |info| info.name.clone())
    }

    /// The name of the generator with the given diagram, if any.
    fn describe(&self, diagram: &Diagram) -> String {
        self.iter()
            .find(|info| info.diagram == *diagram)
            .map_or_else(|| "…".to_owned(), |info| info.name.clone())
    }
}

#[cfg(test)]
mod tests {
    use homotopy_core::{
        signature::{Invertibility, Signature as _},
        typecheck::{typecheck, Mode},
        Boundary, Diagram, DiagramN, Generator,
    };

    use super::InstantiationError;
    use crate::proof::Signature;

    /// `x`; placeholder `P : x → x`; `Q, R : x → x`; `y`, `S : y → y`; `e : P ⇒ P`;
    /// and the schema `A : e ≫ e ⇛ e`.
    struct Setup {
        sig: Signature,
        p: Generator,
        q: Diagram,
        s: Diagram,
        e: Generator,
        a: Generator,
    }

    fn setup() -> Setup {
        let mut sig = Signature::default();
        let x = sig.create_generator_zero("x");
        let make = |sig: &mut Signature, s: Diagram, t: Diagram| {
            sig.create_generator(s, t, "g", Invertibility::Directed)
                .unwrap()
        };
        let p = make(&mut sig, x.into(), x.into());
        let q = make(&mut sig, x.into(), x.into());
        let y = sig.create_generator_zero("y");
        let s = make(&mut sig, y.into(), y.into());
        let e = make(&mut sig, p.clone().into(), p.clone().into());
        let ee = e.attach(&e, Boundary::Target, &[]).unwrap();
        let a = make(&mut sig, ee.into(), e.clone().into());
        let g = |d: &DiagramN| d.max_generator().generator;
        let (p_gen, e_gen, a_gen) = (g(&p), g(&e), g(&a));
        let node = sig.find_node(p_gen).unwrap();
        sig.update(&crate::proof::SignatureEdit::Edit(
            node,
            crate::proof::SignatureItemEdit::MakePlaceholder(true),
        ))
        .unwrap();
        Setup {
            sig,
            p: p_gen,
            q: q.into(),
            s: s.into(),
            e: e_gen,
            a: a_gen,
        }
    }

    #[test]
    fn instantiating_a_schema_instantiates_its_dependents() {
        let Setup {
            mut sig,
            p,
            q,
            e,
            a,
            ..
        } = setup();
        assert_eq!(sig.parameters(a), vec![p]);
        assert_eq!(sig.parameters(e), vec![p]);

        let instance = sig.instantiate(a, &[(p, q.clone())]).unwrap();
        let info = sig.generator_info(instance).unwrap().clone();
        assert_eq!(info.instance.as_ref().unwrap().schema, a);
        // The instance mentions neither P nor e, but an instance of e.
        let generators = info.diagram.generators();
        assert!(!generators.contains_key(&p) && !generators.contains_key(&e));
        assert!(sig.parameters(instance).is_empty());
        assert!(typecheck(&info.diagram, &sig, Mode::default(), true).is_ok());

        let e_instance = sig
            .iter()
            .find(|info| info.instance.as_ref().is_some_and(|i| i.schema == e))
            .unwrap()
            .generator;
        assert!(generators.contains_key(&e_instance));

        // Instantiating again gives the same instances.
        let count = sig.iter().count();
        assert_eq!(sig.instantiate(a, &[(p, q)]).unwrap(), instance);
        assert_eq!(sig.iter().count(), count);
    }

    #[test]
    fn ill_typed_instantiations_are_refused() {
        let Setup {
            mut sig,
            p,
            q,
            s,
            e,
            a,
        } = setup();
        // S : y → y does not have the type x → x of P.
        assert!(matches!(
            sig.instantiate(a, &[(p, s)]),
            Err(InstantiationError::IllTyped(_))
        ));
        // e is not a placeholder.
        assert!(matches!(
            sig.instantiate(a, &[(e, q)]),
            Err(InstantiationError::NotAParameter(..))
        ));
    }

    #[test]
    fn placeholders_can_be_replaced_by_composites() {
        let Setup {
            mut sig, p, q, a, ..
        } = setup();
        let Diagram::DiagramN(q) = q else {
            unreachable!()
        };
        let qq = q.attach(&q, Boundary::Target, &[]).unwrap();

        let instance = sig.instantiate(a, &[(p, qq.clone().into())]).unwrap();
        let info = sig.generator_info(instance).unwrap().clone();
        assert!(typecheck(&info.diagram, &sig, Mode::default(), true).is_ok());
        // The instance of e : P ⇒ P is e[QQ/P] : Q Q ⇒ Q Q.
        let e_instance = sig
            .iter()
            .find(|info| info.instance.as_ref().is_some_and(|i| i.schema != a))
            .unwrap();
        let Diagram::DiagramN(e_diagram) = &e_instance.diagram else {
            unreachable!()
        };
        assert_eq!(e_diagram.source(), qq.clone().into());
        assert_eq!(e_diagram.target(), qq.into());
    }
}
