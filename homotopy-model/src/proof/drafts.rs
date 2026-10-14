//! Instantiations in progress.
//!
//! Choosing the diagrams to instantiate a schema with takes a while, and the choice must not be
//! lost when the view that shows it goes away (when the attach menu replaces the signature
//! drawer, say). So the choices live apart from the views, here, and not in the project: they are
//! not part of the proof, and neither undoing nor redoing touches them.

use homotopy_common::hash::FastHashMap;
use homotopy_core::{signature::Signature as _, Diagram, Generator};
use serde::Serialize;

use super::{InferJob, Inference, ReplacementError, Signature};
use crate::job::run_to_end;

/// How a replacement came to be in a [`Draft`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Origin {
    /// The person chose it.
    Chosen,
    /// The person chose it from the ways to fill in what the types leave open.
    Picked,
    /// It follows from the others.
    Inferred,
}

/// The diagram replacing a placeholder in a [`Draft`].
#[derive(Clone, Debug, PartialEq, Eq)]
struct Slot {
    placeholder: Generator,
    diagram: Diagram,
    origin: Origin,
    /// Why the diagram cannot replace the placeholder, given the replacements chosen before it.
    error: Option<ReplacementError>,
}

/// The diagrams chosen so far to replace the placeholders of one schema, in the order they were
/// chosen, and those that follow from them.
///
/// Choosing a diagram for a placeholder also fills in the placeholders that it determines (see
/// [`Signature::infer`]): choosing a cone fixes the diagram and the apex that it is a cone over.
/// Those are marked as inferred. They are recomputed whenever a diagram is chosen or cleared, so
/// they never go out of date, and they never replace something that was chosen. Choosing a
/// diagram for an inferred placeholder makes it chosen, and clearing it keeps it from being
/// inferred again, until none of the diagrams it was inferred from are left.
///
/// A diagram that cannot replace its placeholder, going by the types alone, is kept and marked
/// with the reason (see [`Draft::error`]): it is what the person asked for, and they can see
/// what is wrong with it and what it clashes with, and fix either. It does not take part in
/// inference, and is not part of the [`substitution`](Draft::substitution). Of two choices that
/// clash the earlier one counts, and the later is marked.
///
/// When the types leave several ways to fill in the rest, the draft has them as
/// [candidates](Draft::candidates). None is ever applied unasked: [`DraftEdit::Pick`] fills in
/// the slots of one, marked as inferred like the others.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    slots: Vec<Slot>,
    /// The placeholders whose inferred replacement was cleared.
    declined: Vec<Generator>,
    /// What the last inference found, if it was finished.
    inference: Option<Inference>,
    /// What the last inference started from: the replacements chosen or picked that were well
    /// typed. A change that leaves them as they were (choosing what does not fit, say) needs no
    /// new inference.
    inferred_from: Vec<(Generator, Diagram)>,
}

impl Draft {
    fn slot(&self, placeholder: Generator) -> Option<&Slot> {
        self.slots
            .iter()
            .find(|slot| slot.placeholder == placeholder)
    }

    /// The diagram that replaces a placeholder, if any (also when it is not well typed: see
    /// [`error`](Self::error)).
    #[must_use]
    pub fn get(&self, placeholder: Generator) -> Option<&Diagram> {
        self.slot(placeholder).map(|slot| &slot.diagram)
    }

    /// Whether the diagram that replaces a placeholder was inferred (or picked from the ways to
    /// infer it) rather than chosen.
    #[must_use]
    pub fn is_inferred(&self, placeholder: Generator) -> bool {
        self.slot(placeholder)
            .is_some_and(|slot| slot.origin != Origin::Chosen)
    }

    /// Why the diagram chosen for a placeholder cannot replace it, if it cannot.
    #[must_use]
    pub fn error(&self, placeholder: Generator) -> Option<&ReplacementError> {
        self.slot(placeholder)?.error.as_ref()
    }

    /// Whether any diagram chosen cannot replace its placeholder.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.slots.iter().any(|slot| slot.error.is_some())
    }

    /// The substitution so far, with both chosen and inferred replacements, without those that
    /// are not well typed.
    #[must_use]
    pub fn substitution(&self) -> Vec<(Generator, Diagram)> {
        self.slots
            .iter()
            .filter(|slot| slot.error.is_none())
            .map(|slot| (slot.placeholder, slot.diagram.clone()))
            .collect()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// What the last inference found, if there was a replacement chosen and the inference was
    /// not cancelled.
    #[must_use]
    pub const fn inference(&self) -> Option<&Inference> {
        self.inference.as_ref()
    }

    /// Whether there are replacements to infer from that nothing has been inferred from, which is
    /// the case if the inference was cancelled.
    #[must_use]
    pub fn needs_inference(&self) -> bool {
        self.inference.is_none() && !self.constraints().is_empty()
    }

    /// The ways to fill in the placeholders that are neither chosen nor inferred, when there are
    /// several, as an inference found them (see [`Inference::candidates`]).
    #[must_use]
    pub fn candidates(&self) -> &[Vec<(Generator, Diagram)>] {
        self.inference
            .as_ref()
            .map_or(&[], |inference| &inference.candidates)
    }

    /// The replacements that inference starts from: those chosen or picked that are well typed.
    fn constraints(&self) -> Vec<(Generator, Diagram)> {
        self.slots
            .iter()
            .filter(|slot| slot.origin != Origin::Inferred && slot.error.is_none())
            .map(|slot| (slot.placeholder, slot.diagram.clone()))
            .collect()
    }

    /// Checks the replacements chosen against each other, in the order they were chosen, and then
    /// those picked. Picks that no longer fit are forgotten.
    fn revalidate(&mut self, signature: &Signature, schema: Generator) {
        self.slots.retain(|slot| slot.origin != Origin::Inferred);
        let mut order: Vec<usize> = (0..self.slots.len())
            .filter(|&i| self.slots[i].origin == Origin::Chosen)
            .collect();
        order.extend((0..self.slots.len()).filter(|&i| self.slots[i].origin == Origin::Picked));
        let entries: Vec<_> = order
            .iter()
            .map(|&i| (self.slots[i].placeholder, self.slots[i].diagram.clone()))
            .collect();
        for (&i, result) in order
            .iter()
            .zip(signature.check_replacements(schema, &entries))
        {
            self.slots[i].error = result.err();
        }
        self.slots
            .retain(|slot| slot.origin != Origin::Picked || slot.error.is_none());
    }

    /// Begins the inference after a replacement was chosen or cleared: the inferred replacements
    /// are forgotten, and the job that finds them again is returned, if there is anything to
    /// infer from.
    fn infer(&mut self, signature: &Signature, schema: Generator) -> Option<InferJob> {
        self.revalidate(signature, schema);
        // Picks only count while there is a replacement chosen that they were picked for.
        let chosen = self
            .slots
            .iter()
            .any(|slot| slot.origin == Origin::Chosen && slot.error.is_none());
        if !chosen {
            self.declined.clear();
            self.slots.retain(|slot| slot.origin == Origin::Chosen);
            self.inference = None;
            self.inferred_from.clear();
            return None;
        }
        let constraints = self.constraints();
        if self.inferred_from == constraints {
            if let Some(inference) = self.inference.take() {
                // Nothing that it depends on changed.
                self.finished(inference);
                return None;
            }
        }
        self.inference = None;
        self.inferred_from = constraints;
        Some(InferJob::new(
            signature.clone(),
            schema,
            &self.inferred_from,
        ))
    }

    /// Takes in what an inference found.
    fn finished(&mut self, inference: Inference) {
        for (placeholder, diagram) in &inference.determined {
            if !self.declined.contains(placeholder) && self.slot(*placeholder).is_none() {
                self.slots.push(Slot {
                    placeholder: *placeholder,
                    diagram: diagram.clone(),
                    origin: Origin::Inferred,
                    error: None,
                });
            }
        }
        self.inference = Some(inference);
    }
}

/// A change to the draft instantiation of a schema.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum DraftEdit {
    /// Replace a placeholder of a schema with a diagram.
    Fill(Generator, Generator, Diagram),
    /// Forget the diagram chosen for a placeholder of a schema.
    Clear(Generator, Generator),
    /// Fill in the placeholders of one of the ways to fill in the rest that the last inference
    /// found, by its position in [`Draft::candidates`].
    Pick(Generator, usize),
    /// Infer again what follows from the replacements chosen for a schema, such as after the
    /// inference was cancelled.
    Infer(Generator),
}

impl DraftEdit {
    /// The schema whose draft is changed.
    #[must_use]
    pub const fn schema(&self) -> Generator {
        match self {
            Self::Fill(schema, ..)
            | Self::Clear(schema, _)
            | Self::Pick(schema, _)
            | Self::Infer(schema) => *schema,
        }
    }
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

    /// Whether a diagram can be chosen for a placeholder of a schema, given what has been chosen
    /// for the others, going by the types alone: see [`Signature::check_replacement`]. This is
    /// cheap. Inferred replacements are not in the way, as choosing a diagram for one overrides it.
    ///
    /// # Errors
    ///
    /// Why not: see [`ReplacementError`].
    pub fn check(
        &self,
        signature: &Signature,
        schema: Generator,
        placeholder: Generator,
        diagram: &Diagram,
    ) -> Result<(), ReplacementError> {
        let chosen = self
            .drafts
            .get(&schema)
            .map_or_else(Vec::new, Draft::constraints);
        signature.check_replacement(schema, &chosen, placeholder, diagram)
    }

    /// Makes a change, and finds what follows from it. See [`start`](Self::start) to do the
    /// second part a step at a time.
    pub fn apply(&mut self, signature: &Signature, edit: DraftEdit) {
        let schema = edit.schema();
        if let Some(job) = self.start(signature, edit) {
            self.finish(schema, run_to_end(job));
        }
    }

    /// Makes a change, which is cheap: the replacements are checked against each other, and what
    /// was inferred before is forgotten. If there is something to infer from, returns the job
    /// that finds what follows, which [`finish`](Self::finish) then takes in. If the job is
    /// dropped instead, the replacements chosen are kept and nothing is inferred.
    pub fn start(&mut self, signature: &Signature, edit: DraftEdit) -> Option<InferJob> {
        let schema = edit.schema();
        match edit {
            DraftEdit::Fill(schema, placeholder, diagram) => {
                let draft = self.drafts.entry(schema).or_default();
                draft.slots.retain(|slot| slot.placeholder != placeholder);
                draft.declined.retain(|&p| p != placeholder);
                draft.slots.push(Slot {
                    placeholder,
                    diagram,
                    origin: Origin::Chosen,
                    error: None,
                });
            }
            DraftEdit::Clear(schema, placeholder) => {
                let draft = self.drafts.get_mut(&schema)?;
                if draft.is_inferred(placeholder) {
                    draft.declined.push(placeholder);
                }
                draft.slots.retain(|slot| slot.placeholder != placeholder);
            }
            DraftEdit::Pick(schema, index) => {
                let draft = self.drafts.get_mut(&schema)?;
                let candidate = draft.candidates().get(index)?.clone();
                for (placeholder, diagram) in candidate {
                    if draft.slot(placeholder).is_none() {
                        draft.slots.push(Slot {
                            placeholder,
                            diagram,
                            origin: Origin::Picked,
                            error: None,
                        });
                    }
                }
            }
            DraftEdit::Infer(_) => {
                if let Some(draft) = self.drafts.get_mut(&schema) {
                    draft.inference = None;
                }
            }
        }
        let draft = self.drafts.get_mut(&schema)?;
        let job = draft.infer(signature, schema);
        if draft.is_empty() {
            self.drafts.remove(&schema);
        }
        job
    }

    /// Takes in what the job returned by [`start`](Self::start) found.
    pub fn finish(&mut self, schema: Generator, inference: Inference) {
        if let Some(draft) = self.drafts.get_mut(&schema) {
            draft.finished(inference);
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
    use crate::{
        job::Job,
        proof::{
            completion::tests::square, ReplacementError, Signature, SignatureEdit,
            SignatureItemEdit, Status,
        },
    };

    /// `x`; a placeholder `P : x → x`; `Q, R : x → x`; and the schema `e : P ⇒ P`.
    fn setup() -> (
        Signature,
        homotopy_core::Generator,
        homotopy_core::Generator,
        Diagram,
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
        let r = cell(x.into(), x.into());
        let e = cell(p.clone().into(), p.clone().into());
        let (p, e) = (p.max_generator().generator, e.max_generator().generator);
        let node = sig.find_node(p).unwrap();
        sig.update(&SignatureEdit::Edit(
            node,
            SignatureItemEdit::MakePlaceholder(true),
        ))
        .unwrap();
        (sig, p, e, q.into(), r.into())
    }

    #[test]
    fn drafts_are_edited_independently_of_the_signature() {
        let (sig, p, e, q, r) = setup();
        let mut drafts = Drafts::default();
        assert!(drafts.get(e).is_empty());

        drafts.apply(&sig, DraftEdit::Fill(e, p, q.clone()));
        assert_eq!(drafts.get(e).get(p), Some(&q));
        assert_eq!(drafts.get(e).substitution(), vec![(p, q.clone())]);

        // Filling a slot again replaces its diagram.
        drafts.apply(&sig, DraftEdit::Fill(e, p, r.clone()));
        assert_eq!(drafts.get(e).substitution(), vec![(p, r)]);

        drafts.apply(&sig, DraftEdit::Clear(e, p));
        assert!(drafts.get(e).is_empty());
        assert_eq!(drafts, Drafts::default());
    }

    #[test]
    fn a_diagram_that_does_not_fit_is_kept_and_marked() {
        let (sig, p, e, q, _) = setup();
        let mut drafts = Drafts::default();
        let id: Diagram = q.identity().into();
        drafts.apply(&sig, DraftEdit::Fill(e, p, id.clone()));
        let draft = drafts.get(e);
        // It is there to be seen, marked with why, and is not used.
        assert_eq!(draft.get(p), Some(&id));
        assert!(matches!(
            draft.error(p),
            Some(ReplacementError::Dimension { .. })
        ));
        assert!(draft.has_errors());
        assert!(draft.substitution().is_empty());
        assert!(draft.inference().is_none() && !draft.needs_inference());

        drafts.apply(&sig, DraftEdit::Clear(e, p));
        assert_eq!(drafts, Drafts::default());
    }

    #[test]
    fn a_later_choice_that_clashes_is_marked_and_the_earlier_one_counts() {
        let sq = square();
        let sig = &sq.sig;
        let mut drafts = Drafts::default();
        // `η := p` fills in `F := T` and `G := U`.
        drafts.apply(sig, DraftEdit::Fill(sq.schema, sq.eta, sq.p.clone()));
        let draft = drafts.get(sq.schema);
        assert_eq!(draft.get(sq.f), Some(&sq.t));
        assert!(draft.is_inferred(sq.f) && draft.is_inferred(sq.g));

        // `F := U` is chosen over the inferred `T`, and is wrong because of `η`.
        let job = drafts.start(sig, DraftEdit::Fill(sq.schema, sq.f, sq.u.clone()));
        assert!(
            job.is_none(),
            "it takes no part in the inference, which stands"
        );
        let draft = drafts.get(sq.schema);
        assert_eq!(draft.get(sq.f), Some(&sq.u));
        assert!(!draft.is_inferred(sq.f));
        assert!(matches!(
            draft.error(sq.f),
            Some(ReplacementError::Conflict { .. })
        ));
        assert_eq!(draft.error(sq.eta), None);
        assert!(!draft
            .substitution()
            .iter()
            .any(|(placeholder, _)| *placeholder == sq.f));
        assert_eq!(draft.get(sq.g), Some(&sq.u));

        // Without `η`, `F := U` is fine.
        drafts.apply(sig, DraftEdit::Clear(sq.schema, sq.eta));
        assert!(!drafts.get(sq.schema).has_errors());
        assert_eq!(
            drafts.get(sq.schema).substitution(),
            vec![(sq.f, sq.u.clone())]
        );
    }

    #[test]
    fn what_was_inferred_stays_while_more_is_chosen() {
        let sq = square();
        let sig = &sq.sig;
        let mut drafts = Drafts::default();
        let inferred = |drafts: &Drafts| {
            let draft = drafts.get(sq.schema);
            [sq.f, sq.g, sq.f2, sq.g2, sq.mu, sq.nu].map(|p| draft.get(p).cloned())
        };
        // `η := p` determines the ends of `η`.
        drafts.apply(sig, DraftEdit::Fill(sq.schema, sq.eta, sq.p.clone()));
        assert_eq!(
            inferred(&drafts),
            [
                Some(sq.t.clone()),
                Some(sq.u.clone()),
                None,
                None,
                None,
                None
            ]
        );
        // `η' := z` determines those of `η'`, and what `η := p` determined is still there.
        drafts.apply(sig, DraftEdit::Fill(sq.schema, sq.eta2, sq.z.clone()));
        assert_eq!(
            inferred(&drafts),
            [
                Some(sq.t.clone()),
                Some(sq.u.clone()),
                Some(sq.u.clone()),
                Some(sq.t.clone()),
                None,
                None
            ]
        );
        // A choice that does not fit is marked and changes nothing else.
        let before = inferred(&drafts);
        drafts.apply(sig, DraftEdit::Fill(sq.schema, sq.nu, sq.p.clone()));
        let draft = drafts.get(sq.schema);
        assert!(draft.error(sq.nu).is_some());
        assert_eq!(inferred(&drafts)[..5], before[..5]);
        drafts.apply(sig, DraftEdit::Clear(sq.schema, sq.nu));
        // And the square, which has them all, determines the rest.
        drafts.apply(sig, DraftEdit::Fill(sq.schema, sq.h, sq.m.clone()));
        let draft = drafts.get(sq.schema);
        assert_eq!(draft.get(sq.mu), Some(&sq.p));
        assert_eq!(draft.get(sq.nu), Some(&sq.z));
        assert!(draft.candidates().is_empty() && !draft.has_errors());
        assert_eq!(
            draft.inference().map(|i| &i.status),
            Some(&Status::Complete)
        );
    }

    #[test]
    fn the_ways_to_fill_in_the_rest_are_offered_and_never_applied() {
        let sq = square();
        let sig = &sq.sig;
        let mut drafts = Drafts::default();
        drafts.apply(sig, DraftEdit::Fill(sq.schema, sq.h, sq.ki.clone()));
        let draft = drafts.get(sq.schema);
        // The corners follow; the cuts do not, and nothing of them is filled in.
        assert_eq!(draft.get(sq.f), Some(&sq.t));
        assert!(draft.get(sq.eta).is_none() && draft.get(sq.mu).is_none());
        assert_eq!(draft.candidates().len(), 9);

        // Picking the first fills in its cells as inferred, and the rest follows from them.
        drafts.apply(sig, DraftEdit::Pick(sq.schema, 0));
        let draft = drafts.get(sq.schema);
        assert!(draft.candidates().is_empty());
        for placeholder in [sq.eta, sq.eta2, sq.mu, sq.nu, sq.g] {
            assert!(draft.is_inferred(placeholder));
        }
        assert_eq!(draft.get(sq.mu), Some(&sq.l));
        assert!(!draft.has_errors());

        // A pick can be overridden, and cleared, like any other inferred replacement.
        drafts.apply(sig, DraftEdit::Clear(sq.schema, sq.mu));
        assert!(drafts.get(sq.schema).get(sq.mu).is_none());
        drafts.apply(sig, DraftEdit::Fill(sq.schema, sq.eta, sq.p.clone()));
        let draft = drafts.get(sq.schema);
        assert!(!draft.is_inferred(sq.eta));
        // The other picks stay, so that nothing is left open; `μ` was cleared and stays so.
        assert!(draft.candidates().is_empty());
        assert!(draft.get(sq.mu).is_none());

        // Clearing what was chosen clears what followed, picks included.
        drafts.apply(sig, DraftEdit::Clear(sq.schema, sq.eta));
        drafts.apply(sig, DraftEdit::Clear(sq.schema, sq.h));
        assert_eq!(drafts, Drafts::default());
    }

    #[test]
    fn cancelling_the_inference_keeps_what_was_chosen_and_infers_nothing() {
        let sq = square();
        let sig = &sq.sig;
        let mut drafts = Drafts::default();
        drafts.apply(sig, DraftEdit::Fill(sq.schema, sq.eta, sq.p.clone()));
        assert!(drafts.get(sq.schema).get(sq.f).is_some());

        // The job is started and dropped after a step.
        let mut job = drafts
            .start(sig, DraftEdit::Fill(sq.schema, sq.h, sq.ki.clone()))
            .expect("there is something to infer from");
        assert!(job.step().is_none());
        drop(job);
        let draft = drafts.get(sq.schema);
        assert_eq!(draft.get(sq.h), Some(&sq.ki));
        assert_eq!(draft.get(sq.eta), Some(&sq.p));
        assert!(draft.get(sq.f).is_none(), "nothing is inferred");
        assert!(draft.needs_inference());

        // And it can be asked for again.
        drafts.apply(sig, DraftEdit::Infer(sq.schema));
        let draft = drafts.get(sq.schema);
        assert_eq!(draft.get(sq.f), Some(&sq.t));
        assert!(!draft.needs_inference());
    }

    #[test]
    fn stale_drafts_are_forgotten() {
        let (mut sig, p, e, q, _) = setup();
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
