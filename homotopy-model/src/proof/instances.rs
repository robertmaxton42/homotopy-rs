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
    /// The placeholders a generator can be instantiated at: those its diagram mentions, those
    /// it explicitly [depends on](GeneratorInfo::depends_on) and those that their types mention,
    /// and those that the generators its diagram mentions depend on.
    #[must_use]
    pub fn parameters(&self, generator: Generator) -> Vec<Generator> {
        let mut parameters = vec![];
        self.collect_parameters(generator, &mut vec![], &mut parameters);
        parameters.sort_by_key(|g| (g.dimension, g.id));
        parameters.dedup();
        parameters
    }

    /// Adds the parameters of a generator to `parameters`. `visiting` holds the generators being
    /// visited, so that generators that depend on each other do not recurse forever.
    fn collect_parameters(
        &self,
        generator: Generator,
        visiting: &mut Vec<Generator>,
        parameters: &mut Vec<Generator>,
    ) {
        let Some(info) = self.generator_info(generator) else {
            return;
        };
        if visiting.contains(&generator) {
            return;
        }
        visiting.push(generator);
        self.collect_mentioned_parameters(generator, visiting, parameters);
        for &p in &info.depends_on {
            if p != generator && self.generator_info(p).is_some_and(|info| info.placeholder) {
                parameters.push(p);
                // The type of a replacement for `p` is in terms of the placeholders `p` mentions.
                self.collect_mentioned_parameters(p, visiting, parameters);
            }
        }
        visiting.pop();
    }

    /// Adds the parameters that a generator gets from the generators its diagram mentions.
    fn collect_mentioned_parameters(
        &self,
        generator: Generator,
        visiting: &mut Vec<Generator>,
        parameters: &mut Vec<Generator>,
    ) {
        let Some(info) = self.generator_info(generator) else {
            return;
        };
        for g in info.diagram.generators().into_keys() {
            if g == generator {
                continue;
            }
            match self.generator_info(g) {
                Some(info) if info.placeholder => parameters.push(g),
                Some(info) if !info.depends_on.is_empty() => {
                    self.collect_parameters(g, visiting, parameters);
                }
                _ => {}
            }
        }
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

        let substitution = self.normalise(substitution);
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
        for (b, restricted) in self.dependents(schema, &substitution) {
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

        if let Some(existing) = self.find_instance(schema, &substitution) {
            return Ok(existing);
        }
        let instance = Instance {
            schema,
            substitution: substitution.clone(),
        };

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

        let arguments = self.describe_substitution(&substitution);
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
                depends_on: vec![],
                ..info
            }),
        );
        Ok(generator)
    }

    /// Sorts a substitution and drops its trivial entries.
    pub(crate) fn normalise(
        &self,
        substitution: &[(Generator, Diagram)],
    ) -> Vec<(Generator, Diagram)> {
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
        substitution
    }

    /// The generators mentioned by a schema, other than placeholders, that mention a placeholder
    /// replaced by a (normalised) substitution, each with the substitution restricted to it.
    pub(crate) fn dependents(
        &self,
        schema: Generator,
        substitution: &[(Generator, Diagram)],
    ) -> Vec<(Generator, Vec<(Generator, Diagram)>)> {
        let Some(info) = self.generator_info(schema) else {
            return vec![];
        };
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
        dependents
            .into_iter()
            .map(|b| {
                let parameters = self.parameters(b);
                let restricted = substitution
                    .iter()
                    .filter(|(p, _)| parameters.contains(p))
                    .cloned()
                    .collect();
                (b, restricted)
            })
            .collect()
    }

    /// The existing instance of a schema for a (normalised) substitution.
    pub(crate) fn find_instance(
        &self,
        schema: Generator,
        substitution: &[(Generator, Diagram)],
    ) -> Option<Generator> {
        self.iter()
            .find(|info| {
                info.instance
                    .as_ref()
                    .is_some_and(|i| i.schema == schema && i.substitution == substitution)
            })
            .map(|info| info.generator)
    }

    /// A short description of a substitution, like `Q/P, R/S`.
    #[must_use]
    pub fn describe_substitution(&self, substitution: &[(Generator, Diagram)]) -> String {
        substitution
            .iter()
            .map(|(p, d)| format!("{}/{}", self.describe(d), self.name_of(*p)))
            .collect::<Vec<_>>()
            .join(", ")
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

    /// Limits as right Kan extensions along `! : J → 1`, with the universal property as a schema
    /// whose mediating cell depends on the cone although its type does not mention it.
    mod limits {
        use homotopy_core::{
            signature::{Invertibility, Signature as _},
            typecheck::{typecheck, Mode},
            Boundary, Diagram, DiagramN, Generator, Height,
        };

        use crate::proof::{DraftEdit, Drafts, Signature, SignatureEdit, SignatureItemEdit};

        pub(super) struct Limits {
            pub sig: Signature,
            pub d: Generator,
            pub x: Generator,
            pub alpha: Generator,
            pub l: Generator,
            pub u: Generator,
            pub beta: Generator,
            pub d0: DiagramN,
            pub x0: DiagramN,
            /// Two different cones `α₀, α₁ : X₀ ∘ ! ⇒ D₀`.
            pub alphas: [DiagramN; 2],
        }

        fn g(d: &DiagramN) -> Generator {
            d.max_generator().generator
        }

        fn edit(sig: &mut Signature, generator: Generator, edit: SignatureItemEdit) {
            let node = sig.find_node(generator).unwrap();
            sig.update(&SignatureEdit::Edit(node, edit)).unwrap();
        }

        pub(super) fn setup() -> Limits {
            let mut sig = Signature::default();
            let one: Diagram = sig.create_generator_zero("1").into();
            let j: Diagram = sig.create_generator_zero("J").into();
            let c: Diagram = sig.create_generator_zero("C").into();
            let cell = |sig: &mut Signature, s: Diagram, t: Diagram, name: &str| {
                sig.create_generator(s, t, name, Invertibility::Directed)
                    .unwrap()
            };

            let bang = cell(&mut sig, j.clone(), one.clone(), "!");
            let d = cell(&mut sig, j.clone(), c.clone(), "D");
            let l = cell(&mut sig, one.clone(), c.clone(), "L");
            let bang_l = bang.attach(&l, Boundary::Target, &[]).unwrap();
            let pi = cell(&mut sig, bang_l.into(), d.clone().into(), "π");
            let x = cell(&mut sig, one.clone(), c.clone(), "X");
            let bang_x = bang.attach(&x, Boundary::Target, &[]).unwrap();
            let alpha = cell(&mut sig, bang_x.clone().into(), d.clone().into(), "α");
            let u = cell(&mut sig, x.clone().into(), l.clone().into(), "u");
            // β : α ⇛ (u ▷ !) ≫ π
            let factorisation = bang_x
                .identity()
                .attach(&u, Boundary::Target, &[1])
                .unwrap()
                .attach(&pi, Boundary::Target, &[0])
                .unwrap();
            let beta = cell(&mut sig, alpha.clone().into(), factorisation.into(), "β");

            for placeholder in [g(&d), g(&x), g(&alpha)] {
                edit(
                    &mut sig,
                    placeholder,
                    SignatureItemEdit::MakePlaceholder(true),
                );
            }
            edit(
                &mut sig,
                g(&l),
                SignatureItemEdit::SetDependsOn(vec![g(&d)]),
            );
            edit(
                &mut sig,
                g(&u),
                SignatureItemEdit::SetDependsOn(vec![g(&alpha)]),
            );

            // A concrete diagram and two cones over it.
            let d0 = cell(&mut sig, j, c.clone(), "D₀");
            let x0 = cell(&mut sig, one, c, "X₀");
            let bang_x0 = bang.attach(&x0, Boundary::Target, &[]).unwrap();
            let alpha0 = cell(&mut sig, bang_x0.clone().into(), d0.clone().into(), "α₀");
            let alpha1 = cell(&mut sig, bang_x0.into(), d0.clone().into(), "α₁");

            Limits {
                sig,
                d: g(&d),
                x: g(&x),
                alpha: g(&alpha),
                l: g(&l),
                u: g(&u),
                beta: g(&beta),
                d0,
                x0,
                alphas: [alpha0, alpha1],
            }
        }

        fn instances_of(sig: &Signature, schema: Generator) -> Vec<Generator> {
            sig.iter()
                .filter(|info| info.instance.as_ref().is_some_and(|i| i.schema == schema))
                .map(|info| info.generator)
                .collect()
        }

        #[test]
        fn mediating_cells_depend_on_the_cone() {
            let Limits {
                mut sig,
                d,
                x,
                alpha,
                l,
                u,
                beta,
                d0,
                x0,
                alphas,
            } = setup();
            assert_eq!(sig.parameters(l), vec![d]);
            assert_eq!(sig.parameters(u), vec![d, x, alpha]);
            assert_eq!(sig.parameters(beta), vec![d, x, alpha]);

            for alpha_i in &alphas {
                let substitution = [
                    (d, d0.clone().into()),
                    (x, x0.clone().into()),
                    (alpha, alpha_i.clone().into()),
                ];
                let instance = sig.instantiate(beta, &substitution).unwrap();
                let diagram = sig.generator_info(instance).unwrap().diagram.clone();
                assert!(typecheck(&diagram, &sig, Mode::default(), true).is_ok());
            }

            // One limit L[D₀], but a different mediating cell for each cone.
            assert_eq!(instances_of(&sig, l).len(), 1);
            assert_eq!(instances_of(&sig, u).len(), 2);
            assert_eq!(instances_of(&sig, beta).len(), 2);
        }

        #[test]
        fn factorisations_are_inferred_from_a_cone() {
            let Limits {
                mut sig,
                d,
                x,
                alpha,
                u,
                beta,
                d0,
                x0,
                alphas,
                ..
            } = setup();
            let haystack: Diagram = alphas[0].clone().into();
            let point = [Height::Singular(0), Height::Singular(0)];
            let inferred = sig.infer_instances(beta, &haystack, Boundary::Target, &point);
            let expected = vec![
                (d, Diagram::from(d0)),
                (x, x0.into()),
                (alpha, alphas[0].clone().into()),
            ];
            let found = inferred
                .into_iter()
                .find(|inferred| inferred.substitution == expected)
                .expect("the cone's own substitution is inferred");

            // Attaching it creates the mediating cell for α₀ and factors α₀ through the limit.
            let instance = sig.instantiate(beta, &found.substitution).unwrap();
            let Diagram::DiagramN(diagram) = sig.generator_info(instance).unwrap().diagram.clone()
            else {
                unreachable!()
            };
            let DiagramN { .. } = alphas[0]
                .clone()
                .identity()
                .attach(&diagram, Boundary::Target, &found.embedding)
                .unwrap();
            assert_eq!(instances_of(&sig, u).len(), 1);
        }

        #[test]
        fn a_cone_determines_its_diagram_and_apex() {
            let Limits {
                sig,
                d,
                x,
                alpha,
                u,
                d0,
                x0,
                alphas,
                ..
            } = setup();
            let cone = (alpha, Diagram::from(alphas[0].clone()));

            // The cone `α₀ : X₀ ∘ ! ⇒ D₀` can only be a cone over `D₀` with apex `X₀`, although
            // `D` could also be replaced by `X₀ ∘ !`, if `α` were not to be replaced by `α₀`.
            assert_eq!(
                sig.infer_replacements(u, std::slice::from_ref(&cone)),
                vec![(d, d0.clone().into()), (x, x0.clone().into())]
            );

            // Nothing is determined by a diagram alone, or when what is chosen contradicts.
            assert!(sig
                .infer_replacements(u, &[(d, d0.clone().into())])
                .is_empty());
            assert!(sig.infer_replacements(u, &[]).is_empty());
            let bang_x0: Diagram = alphas[0].source();
            assert!(sig
                .infer_replacements(u, &[(d, bang_x0), cone.clone()])
                .is_empty());

            // What is determined is not returned if it was chosen.
            assert_eq!(
                sig.infer_replacements(u, &[(d, d0.into()), cone]),
                vec![(x, x0.into())]
            );
        }

        #[test]
        fn inferred_replacements_instantiate() {
            let Limits {
                mut sig,
                d,
                x,
                alpha,
                u,
                d0,
                x0,
                alphas,
                ..
            } = setup();
            let mut drafts = Drafts::default();
            drafts.apply(&sig, DraftEdit::Fill(u, alpha, alphas[0].clone().into()));
            let draft = drafts.get(u);
            assert!(!draft.is_inferred(alpha));
            assert!(draft.is_inferred(d) && draft.is_inferred(x));
            assert_eq!(draft.get(d), Some(&Diagram::from(d0)));
            assert_eq!(draft.get(x), Some(&Diagram::from(x0.clone())));

            // The substitution is complete, so the instance can be made.
            let instance = sig.instantiate(u, &draft.substitution()).unwrap();
            assert!(sig.generator_info(instance).unwrap().instance.is_some());
        }

        #[test]
        fn inferred_replacements_can_be_overridden_and_cleared() {
            let Limits {
                sig,
                d,
                x,
                alpha,
                u,
                x0,
                alphas,
                ..
            } = setup();
            let mut drafts = Drafts::default();
            drafts.apply(&sig, DraftEdit::Fill(u, alpha, alphas[0].clone().into()));

            // Choosing the diagram for an inferred placeholder makes it chosen.
            drafts.apply(&sig, DraftEdit::Fill(u, x, x0.clone().into()));
            assert!(!drafts.get(u).is_inferred(x));
            assert!(drafts.get(u).is_inferred(d));

            // Clearing an inferred replacement keeps it from coming back, ...
            drafts.apply(&sig, DraftEdit::Clear(u, d));
            assert_eq!(drafts.get(u).get(d), None);
            drafts.apply(&sig, DraftEdit::Clear(u, x));
            assert_eq!(drafts.get(u).get(d), None);
            assert!(drafts.get(u).is_inferred(x));

            // ... until what it was inferred from is cleared, which clears the inferred.
            drafts.apply(&sig, DraftEdit::Clear(u, alpha));
            assert_eq!(drafts, Drafts::default());
            drafts.apply(&sig, DraftEdit::Fill(u, alpha, alphas[0].clone().into()));
            assert!(drafts.get(u).is_inferred(d));
        }

        #[test]
        fn dependencies_bring_the_placeholders_their_types_mention() {
            let Limits {
                mut sig,
                d,
                x,
                alpha,
                ..
            } = setup();
            // A cell that depends on the cone but mentions neither it nor its types.
            let cell = sig
                .create_generator(
                    sig.generator_info(d).unwrap().diagram.clone(),
                    sig.generator_info(d).unwrap().diagram.clone(),
                    "w",
                    Invertibility::Directed,
                )
                .unwrap();
            let w = g(&cell);
            edit(&mut sig, w, SignatureItemEdit::SetDependsOn(vec![alpha]));
            assert_eq!(sig.parameters(w), vec![d, x, alpha]);

            // Generators that depend on each other are not a problem.
            edit(&mut sig, alpha, SignatureItemEdit::SetDependsOn(vec![d]));
            edit(&mut sig, d, SignatureItemEdit::SetDependsOn(vec![alpha]));
            assert_eq!(sig.parameters(w), vec![d, x, alpha]);
        }
    }
}
