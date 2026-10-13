//! Performing a sequence of actions on a copy of a proof, a bit at a time.

use std::collections::VecDeque;

use homotopy_core::contraction::MergeExchanges;

use super::{Action, ProofError, ProofState};
use crate::{
    history::Proof,
    job::{Job, Progress},
    remote::Work,
};

/// What a job that performs recorded actions is called.
pub const REPLAYING: &str = "Replaying actions";

/// What a finished [`ActionsJob`] came to.
#[derive(Debug)]
pub struct ActionsOutcome {
    /// The actions that changed the proof, each with the proof as it was after that action, for
    /// adding to the history.
    pub applied: Vec<(Action, Proof)>,
    /// The error that cut the sequence short, if any. What was applied before it still stands.
    pub error: Option<ProofError>,
}

/// A sequence of actions performed one after another on a copy of a proof.
///
/// Each is performed as [`ProofState::update`](super::ProofState::update) would. The proof that
/// the copy was made from is not touched: committing the [outcome](ActionsOutcome) is up to
/// whoever runs the job, and dropping the job before it is done changes nothing.
///
/// The steps are as small as they can be: one action, except for [`Action::MergeExchanges`],
/// where it is one step of the diagram.
#[derive(Debug)]
pub struct ActionsJob {
    proof: Proof,
    pending: VecDeque<Action>,
    total: usize,
    consumed: usize,
    /// What to call the job, if it is more than the action in progress.
    title: Option<&'static str>,
    /// Whether to stop at an action that changes nothing, as the actions after it are only
    /// meant to follow one that did.
    stop_at_noop: bool,
    merging: Option<MergeExchanges>,
    /// What the action being performed is doing.
    doing: Option<&'static str>,
    applied: Vec<(Action, Proof)>,
    error: Option<ProofError>,
}

impl ActionsJob {
    /// Performs actions of which each is meant to follow the one before, such as a homotopy and
    /// the merging of exchanges that follows it: if one does nothing, the rest are skipped.
    #[must_use]
    pub fn chain(proof: Proof, actions: impl IntoIterator<Item = Action>) -> Self {
        Self::new(proof, actions, None, true)
    }

    /// Performs actions that were recorded earlier, skipping those that do nothing. `title`
    /// names what is being done.
    #[must_use]
    pub fn replay(
        proof: Proof,
        actions: impl IntoIterator<Item = Action>,
        title: &'static str,
    ) -> Self {
        Self::new(proof, actions, Some(title), false)
    }

    fn new(
        proof: Proof,
        actions: impl IntoIterator<Item = Action>,
        title: Option<&'static str>,
        stop_at_noop: bool,
    ) -> Self {
        let pending: VecDeque<_> = actions.into_iter().collect();
        Self {
            proof,
            total: pending.len(),
            pending,
            consumed: 0,
            title,
            stop_at_noop,
            merging: None,
            doing: None,
            applied: vec![],
            error: None,
        }
    }

    /// Whether the next step is an action that cannot be done in smaller steps, and may take
    /// long (see [`Action::work_label`]).
    #[must_use]
    pub fn next_step_is_heavy(&self) -> bool {
        self.merging.is_none()
            && self.pending.front().is_some_and(|action| {
                *action != Action::MergeExchanges && action.work_label().is_some()
            })
    }

    /// What it takes to do the job somewhere else (see [`remote`](crate::remote)): the proof it is
    /// made from and the work to do on it. `None` if the job has begun, or is not one that can
    /// be described like that.
    #[must_use]
    pub fn offer(&self) -> Option<(&ProofState, Work)> {
        let replay = match self.title {
            None => false,
            Some(REPLAYING) => true,
            Some(_) => return None,
        };
        let untouched = self.consumed == 0 && self.merging.is_none() && self.applied.is_empty();
        (untouched && self.stop_at_noop != replay).then(|| {
            (
                &**self.proof,
                Work::Actions {
                    actions: self.pending.iter().cloned().collect(),
                    replay,
                },
            )
        })
    }

    fn outcome(&mut self) -> ActionsOutcome {
        ActionsOutcome {
            applied: std::mem::take(&mut self.applied),
            error: self.error.take(),
        }
    }

    /// Takes note of what an action came to, returning the outcome if that is the end of it.
    fn record(
        &mut self,
        action: Action,
        result: Result<bool, ProofError>,
    ) -> Option<ActionsOutcome> {
        self.consumed += 1;
        self.doing = None;
        match result {
            Ok(true) => self.applied.push((action, self.proof.clone())),
            Ok(false) if self.stop_at_noop => return Some(self.outcome()),
            Ok(false) => {}
            Err(error) => {
                self.error = Some(error);
                return Some(self.outcome());
            }
        }
        // Done with the last action, in the step that did it: there is nothing to wait for.
        self.pending.is_empty().then(|| self.outcome())
    }
}

impl Job for ActionsJob {
    type Output = ActionsOutcome;

    fn label(&self) -> &'static str {
        self.title
            .or_else(|| {
                self.merging
                    .as_ref()
                    .and(Action::MergeExchanges.work_label())
            })
            .or_else(|| self.pending.front().and_then(Action::work_label))
            .unwrap_or("Working")
    }

    fn progress(&self) -> Option<Progress> {
        if self.title.is_some() {
            Some(Progress {
                done: self.consumed,
                total: self.total,
            })
        } else {
            self.merging.as_ref().map(|merging| {
                let (done, total) = merging.progress();
                Progress { done, total }
            })
        }
    }

    fn step(&mut self) -> Option<ActionsOutcome> {
        if let Some(merging) = &mut self.merging {
            if merging.step(&self.proof.signature) {
                return None;
            }
            let merging = self.merging.take()?;
            let changed = self.proof.finish_merge_exchanges(merging);
            return self.record(Action::MergeExchanges, Ok(changed));
        }

        let Some(action) = self.pending.pop_front() else {
            return Some(self.outcome());
        };
        self.doing = action.work_label();
        if action == Action::MergeExchanges {
            return match self.proof.start_merge_exchanges() {
                Some(merging) => {
                    self.merging = Some(merging);
                    None
                }
                None => self.record(action, Ok(false)),
            };
        }
        let result = self.proof.update(&action);
        self.record(action, result)
    }
}
