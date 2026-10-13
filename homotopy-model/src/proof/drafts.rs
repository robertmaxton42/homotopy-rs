//! Instantiations in progress.
//!
//! Choosing the diagrams to instantiate a schema with takes a while, and the choice must not be
//! lost when the view that shows it goes away (when the attach menu replaces the signature
//! drawer, say). So the choices live apart from the views, here, and not in the project: they are
//! not part of the proof, and neither undoing nor redoing touches them.

use homotopy_common::hash::FastHashMap;
use homotopy_core::{signature::Signature as _, Diagram, Generator};

use super::Signature;

/// The diagram replacing a placeholder in a [`Draft`].
#[derive(Clone, Debug, PartialEq, Eq)]
struct Slot {
    placeholder: Generator,
    diagram: Diagram,
    /// Whether the diagram was inferred from the others rather than chosen.
    inferred: bool,
}

/// The diagrams chosen so far to replace the placeholders of one schema, in the order they were
/// chosen, and those that follow from them.
///
/// Choosing a diagram for a placeholder also fills in the placeholders that it determines (see
/// [`Signature::infer_replacements`]): choosing a cone fixes the diagram and the apex that it is
/// a cone over. Those are marked as inferred. They are recomputed whenever a diagram is chosen
/// or cleared, so they never go out of date, and they never replace something that was chosen.
/// Choosing a diagram for an inferred placeholder makes it chosen, and clearing it keeps it
/// from being inferred again, until none of the diagrams it was inferred from are left.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    slots: Vec<Slot>,
    /// The placeholders whose inferred replacement was cleared.
    declined: Vec<Generator>,
}

impl Draft {
    fn slot(&self, placeholder: Generator) -> Option<&Slot> {
        self.slots
            .iter()
            .find(|slot| slot.placeholder == placeholder)
    }

    /// The diagram that replaces a placeholder, if any.
    #[must_use]
    pub fn get(&self, placeholder: Generator) -> Option<&Diagram> {
        self.slot(placeholder).map(|slot| &slot.diagram)
    }

    /// Whether the diagram that replaces a placeholder was inferred rather than chosen.
    #[must_use]
    pub fn is_inferred(&self, placeholder: Generator) -> bool {
        self.slot(placeholder).is_some_and(|slot| slot.inferred)
    }

    /// The substitution so far, with both chosen and inferred replacements.
    #[must_use]
    pub fn substitution(&self) -> Vec<(Generator, Diagram)> {
        self.slots
            .iter()
            .map(|slot| (slot.placeholder, slot.diagram.clone()))
            .collect()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    fn chosen(&self) -> Vec<(Generator, Diagram)> {
        self.slots
            .iter()
            .filter(|slot| !slot.inferred)
            .map(|slot| (slot.placeholder, slot.diagram.clone()))
            .collect()
    }

    /// Redoes the inference after a replacement was chosen or cleared. This can be slow: see
    /// [`Signature::infer_replacements`].
    fn infer(&mut self, signature: &Signature, schema: Generator) {
        self.slots.retain(|slot| !slot.inferred);
        let chosen = self.chosen();
        if chosen.is_empty() {
            self.declined.clear();
            return;
        }
        for (placeholder, diagram) in signature.infer_replacements(schema, &chosen) {
            if !self.declined.contains(&placeholder) {
                self.slots.push(Slot {
                    placeholder,
                    diagram,
                    inferred: true,
                });
            }
        }
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

    /// Makes a change. Inferring the replacements that follow can be slow.
    pub fn apply(&mut self, signature: &Signature, edit: DraftEdit) {
        match edit {
            DraftEdit::Fill(schema, placeholder, diagram) => {
                let draft = self.drafts.entry(schema).or_default();
                draft.slots.retain(|slot| slot.placeholder != placeholder);
                draft.declined.retain(|&p| p != placeholder);
                draft.slots.push(Slot {
                    placeholder,
                    diagram,
                    inferred: false,
                });
                draft.infer(signature, schema);
            }
            DraftEdit::Clear(schema, placeholder) => {
                let Some(draft) = self.drafts.get_mut(&schema) else {
                    return;
                };
                if draft.is_inferred(placeholder) {
                    draft.declined.push(placeholder);
                }
                draft.slots.retain(|slot| slot.placeholder != placeholder);
                draft.infer(signature, schema);
                if draft.is_empty() {
                    self.drafts.remove(&schema);
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
            draft.slots.retain(|slot| {
                parameters.contains(&slot.placeholder)
                    && slot
                        .diagram
                        .generators()
                        .into_keys()
                        .all(|g| signature.generator_info(g).is_some())
            });
            draft.declined.retain(|p| parameters.contains(p));
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
        assert_eq!(drafts.get(e).substitution(), vec![(p, q.clone())]);

        // Filling a slot again replaces its diagram.
        let id: Diagram = q.clone().identity().into();
        drafts.apply(&sig, DraftEdit::Fill(e, p, id.clone()));
        assert_eq!(drafts.get(e).substitution(), vec![(p, id)]);

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
