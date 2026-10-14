//! What decides when the project is saved in the background, and the part of saving that the page
//! does a piece at a time (see `homotopy-web/src/autosave.rs`, and "Autosave" in `CLAUDE.md`).
//!
//! Saving a project is turning it into a `.hom` file ([`serialize`](crate::serialize)), which for
//! a project of a few megabytes takes longer than the page may be blocked for. So the page does
//! not do it: a worker does, given the project the way jobs are (see [`remote`](crate::remote)).
//! Giving the worker a signature it does not hold has a cost of its own, which grows with the
//! signature, and is the part of it that is done here a diagram at a time ([`Packer`]) with the
//! pieces sent as they are made, so that no single step of it is long.
//!
//! [`Plan`] is the other half: it is told what version of the project there is, and what has been
//! done about it, and says when to save next. It knows nothing of time except what it is told, nor
//! of what a version is (the app uses the position in the history), so that it can be tested.

use homotopy_core::{serialize::Store, Diagram, Rewrite};

use crate::proof::Signature;

/// A piece of the work of a [`Packer`].
enum Task {
    /// Everything about a diagram, which comes apart into smaller tasks.
    Diagram(Diagram),
    /// A rewrite, which a diagram has two of for each of its cospans.
    Rewrite(Rewrite),
    /// The diagram itself, when what it is made of has been packed.
    Pack(Diagram),
}

/// Packs the diagrams of a signature into a [`Store`] a little at a time, handing out what has
/// been packed as it goes.
///
/// A diagram of a large project can take longer to pack in one go than the page can be blocked
/// for, so a diagram is packed in the order of what it is made of: its source, then the rewrites
/// of its cospans one at a time, and then the diagram, which has nothing left to do but find them
/// already packed.
pub struct Packer {
    store: Store,
    /// What is left to do, the next on top.
    tasks: Vec<Task>,
}

impl Packer {
    #[must_use]
    pub fn new(signature: &Signature) -> Self {
        let mut tasks = vec![];
        for info in signature.iter() {
            tasks.push(Task::Diagram(info.diagram.clone()));
            if let Some(instance) = &info.instance {
                tasks.extend(
                    instance
                        .substitution
                        .iter()
                        .map(|(_, diagram)| Task::Diagram(diagram.clone())),
                );
            }
        }
        // The first in the signature is the first done.
        tasks.reverse();
        Self {
            store: Store::default(),
            tasks,
        }
    }

    /// Does a bit of the work. Returns whether there is no more.
    pub fn step(&mut self) -> bool {
        match self.tasks.pop() {
            None => {}
            Some(Task::Rewrite(rewrite)) => {
                self.store.pack_rewrite(&rewrite);
            }
            Some(Task::Pack(diagram)) => {
                self.store.pack_diagram(&diagram);
            }
            Some(Task::Diagram(Diagram::Diagram0(point))) => {
                self.store.pack_diagram(&point.into());
            }
            Some(Task::Diagram(Diagram::DiagramN(diagram))) => {
                // In reverse, as the last pushed is the first done.
                self.tasks.push(Task::Pack(diagram.clone().into()));
                for cospan in diagram.cospans().iter().rev() {
                    self.tasks.push(Task::Rewrite(cospan.backward.clone()));
                    self.tasks.push(Task::Rewrite(cospan.forward.clone()));
                }
                self.tasks.push(Task::Diagram(diagram.source()));
            }
        }
        self.tasks.is_empty()
    }

    /// What has been packed since the last call, to be sent. What it has given away is not
    /// packed again, whatever is packed after it.
    pub fn take(&mut self) -> Store {
        self.store.take_packed()
    }

    /// The store, which has the keys of everything that was packed, for packing the rest of the
    /// project with. Whatever was not taken is still in it.
    #[must_use]
    pub fn into_store(self) -> Store {
        self.store
    }
}

/// How long the project has to stay as it is before it is saved, in milliseconds.
pub const IDLE_MS: f64 = 5_000.0;
/// How long the project may be changing all the time without being saved, in milliseconds.
pub const MAX_WAIT_MS: f64 = 60_000.0;
/// How long to wait after a save that failed before trying again, doubling for each failure in
/// a row up to [`MAX_RETRY_MS`].
pub const RETRY_MS: f64 = 10_000.0;
pub const MAX_RETRY_MS: f64 = 300_000.0;
/// How many versions are remembered as being safe elsewhere.
const CLEAN: usize = 8;

/// When to save the project. A version is anything with an equality that changes whenever the
/// project does (undoing gives back an old one).
#[derive(Clone, Debug)]
pub struct Plan<V> {
    enabled: bool,
    /// How long to leave the project alone before saving it, and the longest to go on waiting
    /// for that (see [`IDLE_MS`] and [`MAX_WAIT_MS`]).
    idle: f64,
    max_wait: f64,
    /// How long to wait after a failure, and the longest (see [`RETRY_MS`]).
    retry: f64,
    max_retry: f64,
    /// Versions whose project is safe elsewhere: the one that was opened, those imported and
    /// those exported. There is nothing to protect in them.
    clean: Vec<V>,
    /// The version of the last save that went through.
    saved: Option<V>,
    current: V,
    /// When the project last changed, and when it last went from nothing to protect to something.
    changed_at: f64,
    unsaved_since: f64,
    saving: Option<V>,
    failures: u32,
    failed_at: f64,
}

impl<V: Copy + Eq> Plan<V> {
    /// A plan for a project that has just been opened, or is empty.
    #[must_use]
    pub fn new(version: V, now: f64) -> Self {
        Self {
            enabled: true,
            idle: IDLE_MS,
            max_wait: MAX_WAIT_MS,
            retry: RETRY_MS,
            max_retry: MAX_RETRY_MS,
            clean: vec![version],
            saved: None,
            current: version,
            changed_at: now,
            unsaved_since: now,
            saving: None,
            failures: 0,
            failed_at: now,
        }
    }

    /// Waits for other times than the usual, to test with: the retries are in proportion.
    #[must_use]
    pub fn with_timing(mut self, idle_ms: f64, max_wait_ms: f64) -> Self {
        self.idle = idle_ms;
        self.max_wait = max_wait_ms;
        self.retry = RETRY_MS * idle_ms / IDLE_MS;
        self.max_retry = MAX_RETRY_MS * idle_ms / IDLE_MS;
        self
    }

    /// Whether saving is wanted at all.
    pub const fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    /// Whether the current version is one that something would be lost with.
    #[must_use]
    pub fn unsaved(&self) -> bool {
        self.saved != Some(self.current) && !self.clean.contains(&self.current)
    }

    /// Takes note of the version the project is at now, which the app calls whenever it may have
    /// changed (it is cheap, and does nothing if it has not).
    pub fn observe(&mut self, version: V, now: f64) {
        if version == self.current {
            return;
        }
        let was_unsaved = self.unsaved();
        self.current = version;
        self.changed_at = now;
        if self.unsaved() && !was_unsaved {
            self.unsaved_since = now;
        }
    }

    /// Takes note that the project at this version is safe elsewhere: it was opened, imported or
    /// exported.
    pub fn mark_clean(&mut self, version: V) {
        if !self.clean.contains(&version) {
            if self.clean.len() == CLEAN {
                self.clean.remove(0);
            }
            self.clean.push(version);
        }
    }

    /// How long from `now` until the project should be saved, in milliseconds, if it should be:
    /// `None` if there is nothing to save, saving is off or a save is on its way. It is for the
    /// app to decide that this is not a good time either (a job is running), and ask again.
    #[must_use]
    pub fn wait(&self, now: f64) -> Option<f64> {
        if !self.enabled || self.saving.is_some() || !self.unsaved() {
            return None;
        }
        // After the project has been left alone for a while, or has been changing for too long.
        let mut due = (self.changed_at + self.idle).min(self.unsaved_since + self.max_wait);
        if self.failures > 0 {
            let backoff =
                (self.retry * f64::from(1_u32 << (self.failures - 1).min(10))).min(self.max_retry);
            due = due.max(self.failed_at + backoff);
        }
        Some((due - now).max(0.0))
    }

    /// Makes the project due to be saved now rather than after the wait, if it is to be saved at
    /// all: the person is leaving (the tab was hidden or is being closed), and what is not saved
    /// by the time they are gone is lost.
    pub const fn hurry(&mut self) {
        self.changed_at = f64::NEG_INFINITY;
        self.unsaved_since = f64::NEG_INFINITY;
    }

    /// Starts a save of the current version, which is returned.
    pub const fn start(&mut self) -> V {
        self.saving = Some(self.current);
        self.current
    }

    /// The version being saved, if one is.
    #[must_use]
    pub const fn saving(&self) -> Option<V> {
        self.saving
    }

    /// Takes note that the save that was started is over, and whether it went through; or, if
    /// none was started, nothing.
    pub fn finish(&mut self, ok: bool, now: f64) {
        let Some(version) = self.saving.take() else {
            return;
        };
        if ok {
            self.saved = Some(version);
            self.failures = 0;
            if self.unsaved() {
                // Changed since: the next save is due a while after this one.
                self.unsaved_since = now;
            }
        } else {
            self.failures += 1;
            self.failed_at = now;
        }
    }

    /// Gives up on the save that was started, which did not fail: it was not wanted any more.
    pub const fn abandon(&mut self) {
        self.saving = None;
    }

    /// How many saves in a row have failed.
    #[must_use]
    pub const fn failures(&self) -> u32 {
        self.failures
    }

    /// Forgets what was saved, as it is gone from where it was put (all of the saves were
    /// deleted), so that the project is saved again if it is worth it.
    pub const fn forget_saved(&mut self) {
        self.saved = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_nobody_changed_is_never_saved() {
        let mut plan = Plan::new(0_u32, 0.0);
        assert_eq!(plan.wait(1e9), None);
        // Neither is one that was only opened or imported, whatever the history said.
        plan.observe(1, 100.0);
        plan.mark_clean(1);
        assert_eq!(plan.wait(1e9), None);
        // Going back to where it was opened is not a change worth saving either.
        plan.observe(0, 200.0);
        assert_eq!(plan.wait(1e9), None);
    }

    #[test]
    fn it_waits_for_the_project_to_be_left_alone() {
        let mut plan = Plan::new(0_u32, 0.0);
        plan.observe(1, 1_000.0);
        assert_eq!(plan.wait(1_000.0), Some(IDLE_MS));
        assert_eq!(plan.wait(3_000.0), Some(IDLE_MS - 2_000.0));
        // Changing it again starts the wait again.
        plan.observe(2, 3_500.0);
        assert_eq!(plan.wait(3_500.0), Some(IDLE_MS));
        assert_eq!(plan.wait(3_500.0 + IDLE_MS), Some(0.0));
        assert_eq!(plan.wait(1e9), Some(0.0));
    }

    #[test]
    fn it_does_not_wait_for_ever_for_a_project_that_keeps_changing() {
        let mut plan = Plan::new(0_u32, 0.0);
        let mut now = 0.0;
        // A change every two seconds never leaves it alone for five.
        for version in 1..=100 {
            now += 2_000.0;
            plan.observe(version, now);
        }
        assert_eq!(now, 200_000.0);
        assert_eq!(plan.wait(now), Some(0.0));
        let mut plan = Plan::new(0_u32, 0.0);
        for version in 1..=10 {
            plan.observe(version, 2_000.0 * f64::from(version));
        }
        // Unsaved since the first change, at 2s; the last was at 20s.
        assert_eq!(plan.wait(20_000.0), Some(IDLE_MS));
        let mut plan = Plan::new(0_u32, 0.0);
        for version in 1..=40 {
            plan.observe(version, 2_000.0 * f64::from(version));
        }
        assert_eq!(plan.wait(80_000.0), Some(0.0));
        assert_eq!(plan.wait(60_000.0), Some(2_000.0 + MAX_WAIT_MS - 60_000.0));
    }

    #[test]
    fn a_save_covers_the_version_it_started_with() {
        let mut plan = Plan::new(0_u32, 0.0);
        plan.observe(1, 0.0);
        assert_eq!(plan.start(), 1);
        // Nothing more is started while it is on its way.
        assert_eq!(plan.wait(1e9), None);
        // The project changes meanwhile.
        plan.observe(2, 10_000.0);
        plan.finish(true, 12_000.0);
        assert!(plan.unsaved());
        assert_eq!(plan.wait(12_000.0), Some(IDLE_MS - 2_000.0));
        assert_eq!(plan.start(), 2);
        plan.finish(true, 20_000.0);
        assert!(!plan.unsaved());
        assert_eq!(plan.wait(1e9), None);
        // Undoing to what was saved needs no save; undoing to something else does.
        plan.observe(1, 30_000.0);
        assert!(plan.unsaved());
        plan.observe(2, 31_000.0);
        assert_eq!(plan.wait(1e9), None);
    }

    #[test]
    fn leaving_makes_it_due_at_once_but_not_before_a_retry() {
        let mut plan = Plan::new(0_u32, 0.0);
        plan.observe(1, 0.0);
        assert_eq!(plan.wait(100.0), Some(IDLE_MS - 100.0));
        plan.hurry();
        assert_eq!(plan.wait(100.0), Some(0.0));
        // A save that failed is not tried again at once, however much of a hurry.
        plan.start();
        plan.finish(false, 200.0);
        plan.hurry();
        assert_eq!(plan.wait(200.0), Some(RETRY_MS));
        // Nothing to save, nothing due.
        let mut plan = Plan::new(0_u32, 0.0);
        plan.hurry();
        assert_eq!(plan.wait(1e9), None);
        // Changes after it count as usual.
        plan.observe(1, 1_000.0);
        assert_eq!(plan.wait(1_000.0), Some(IDLE_MS));
    }

    #[test]
    fn a_failed_save_is_tried_again_later_and_less_often() {
        let mut plan = Plan::new(0_u32, 0.0);
        plan.observe(1, 0.0);
        plan.start();
        plan.finish(false, 6_000.0);
        assert_eq!(plan.failures(), 1);
        assert_eq!(plan.wait(6_000.0), Some(RETRY_MS));
        plan.start();
        plan.finish(false, 20_000.0);
        assert_eq!(plan.wait(20_000.0), Some(2.0 * RETRY_MS));
        for _ in 0..20 {
            plan.start();
            plan.finish(false, 1_000_000.0);
        }
        assert_eq!(plan.wait(1_000_000.0), Some(MAX_RETRY_MS));
        // One that goes through ends it.
        plan.start();
        plan.finish(true, 1_000_000.0);
        assert_eq!(plan.failures(), 0);
        assert_eq!(plan.wait(1e9), None);
    }

    #[test]
    fn exporting_and_turning_it_off() {
        let mut plan = Plan::new(0_u32, 0.0);
        plan.observe(1, 0.0);
        // Exported before the wait is over: there is nothing to protect.
        plan.mark_clean(1);
        assert_eq!(plan.wait(1e9), None);
        plan.observe(2, 100.0);
        assert!(plan.wait(1e9).is_some());
        plan.set_enabled(false);
        assert_eq!(plan.wait(1e9), None);
        plan.set_enabled(true);
        assert!(plan.wait(1e9).is_some());
        // A save that is not wanted any more is just dropped.
        plan.start();
        plan.abandon();
        assert!(plan.wait(1e9).is_some());
    }

    #[test]
    fn only_the_last_versions_are_remembered_as_clean() {
        let mut plan = Plan::new(0_u32, 0.0);
        for version in 1..=20 {
            plan.mark_clean(version);
        }
        plan.observe(20, 0.0);
        assert!(!plan.unsaved());
        plan.observe(1, 0.0);
        assert!(plan.unsaved());
    }
}
