//! Instantiations in progress.
//!
//! Choosing the diagrams to instantiate a schema with takes a while, and the choice must not be
//! lost when the view that shows it goes away (when the attach menu replaces the signature
//! drawer, say). So the choices live apart from the views, here, and not in the project: they are
//! not part of the proof, and neither undoing nor redoing touches them.

use homotopy_common::hash::FastHashMap;
use homotopy_core::{Diagram, Generator};

use super::Signature;

/// The diagrams chosen so far to replace the placeholders of one schema, in the order they were
/// chosen.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    slots: Vec<(Generator, Diagram)>,
}

impl Draft {
    /// The diagram chosen to replace a placeholder, if any.
    #[must_use]
    pub fn get(&self, placeholder: Generator) -> Option<&Diagram> {
        self.slots
            .iter()
            .find(|(p, _)| *p == placeholder)
            .map(|(_, d)| d)
    }

    /// The substitution chosen so far.
    #[must_use]
    pub fn substitution(&self) -> &[(Generator, Diagram)] {
        &self.slots
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    fn fill(&mut self, placeholder: Generator, diagram: Diagram) {
        self.clear(placeholder);
        self.slots.push((placeholder, diagram));
    }

    fn clear(&mut self, placeholder: Generator) {
        self.slots.retain(|(p, _)| *p != placeholder);
    }
}

/// A change to the draft instantiation of a schema.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DraftEdit {
    /// Replace a placeholder of a schema with a diagram.
    Fill(Generator, Generator, Diagram),
    /// Forget the diagram chosen for a placeholder of a schema.
    Clear(Generator, Generator),
}

/// The instantiations in progress, for every schema.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Drafts {
    drafts: FastHashMap<Generator, Draft>,
}

impl Drafts {
    /// The instantiation of a schema in progress, which is empty if there is none.
    #[must_use]
    pub fn get(&self, schema: Generator) -> Draft {
        self.drafts.get(&schema).cloned().unwrap_or_default()
    }

    pub fn apply(&mut self, _signature: &Signature, edit: DraftEdit) {
        match edit {
            DraftEdit::Fill(schema, placeholder, diagram) => {
                self.drafts
                    .entry(schema)
                    .or_default()
                    .fill(placeholder, diagram);
            }
            DraftEdit::Clear(schema, placeholder) => {
                if let Some(draft) = self.drafts.get_mut(&schema) {
                    draft.clear(placeholder);
                    if draft.is_empty() {
                        self.drafts.remove(&schema);
                    }
                }
            }
        }
    }

    /// Forgets what no longer makes sense for the signature, such as the draft for a generator
    /// that was removed, or for a placeholder that the schema no longer has. Drafts are not
    /// part of the proof, so this is needed after anything that changes the signature.
    pub fn retain_valid(&mut self, signature: &Signature) {
        self.drafts.retain(|&schema, draft| {
            let parameters = signature.parameters(schema);
            draft.slots.retain(|(p, _)| parameters.contains(p));
            !draft.is_empty()
        });
    }
}

#[cfg(test)]
mod tests {
    use homotopy_core::{signature::Invertibility, Diagram};

    use super::{DraftEdit, Drafts};
    use crate::proof::{Signature, SignatureEdit, SignatureItemEdit};

    /// `x`; a placeholder `P : x → x`; `Q : x → x`; and the schema `e : P ⇒ P`.
    fn setup() -> (
        Signature,
        homotopy_core::Generator,
        homotopy_core::Generator,
        Diagram,
    ) {
        let mut sig = Signature::default();
        let x = sig.create_generator_zero("x");
        let mut cell = |s: Diagram, t: Diagram| {
            sig.create_generator(s, t, "c", Invertibility::Directed)
                .unwrap()
        };
        let p = cell(x.into(), x.into());
        let q = cell(x.into(), x.into());
        let e = cell(p.clone().into(), p.clone().into());
        let (p, e) = (p.max_generator().generator, e.max_generator().generator);
        let node = sig.find_node(p).unwrap();
        sig.update(&SignatureEdit::Edit(
            node,
            SignatureItemEdit::MakePlaceholder(true),
        ))
        .unwrap();
        (sig, p, e, q.into())
    }

    #[test]
    fn drafts_are_edited_independently_of_the_signature() {
        let (sig, p, e, q) = setup();
        let mut drafts = Drafts::default();
        assert!(drafts.get(e).is_empty());

        drafts.apply(&sig, DraftEdit::Fill(e, p, q.clone()));
        assert_eq!(drafts.get(e).get(p), Some(&q));
        assert_eq!(drafts.get(e).substitution(), &[(p, q.clone())]);

        // Filling a slot again replaces its diagram.
        let id: Diagram = q.clone().identity().into();
        drafts.apply(&sig, DraftEdit::Fill(e, p, id.clone()));
        assert_eq!(drafts.get(e).substitution(), &[(p, id)]);

        drafts.apply(&sig, DraftEdit::Clear(e, p));
        assert!(drafts.get(e).is_empty());
        assert_eq!(drafts, Drafts::default());
    }

    #[test]
    fn stale_drafts_are_forgotten() {
        let (mut sig, p, e, q) = setup();
        let mut drafts = Drafts::default();
        drafts.apply(&sig, DraftEdit::Fill(e, p, q));
        drafts.retain_valid(&sig);
        assert!(!drafts.get(e).is_empty());

        // The schema is no longer a schema in `P` once it is gone.
        sig.remove(e);
        drafts.retain_valid(&sig);
        assert_eq!(drafts, Drafts::default());
    }
}
