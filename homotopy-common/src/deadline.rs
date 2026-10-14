//! A time limit that long searches look at as they go, so that one step of a search cannot run
//! far past the time it was given.
//!
//! A job that works in small steps can be stopped between them, but a search is not always made
//! of small steps (finding the embeddings of a diagram in another is one call, however long it
//! takes), and measuring the time between steps only finds out afterwards. So the code that
//! loops over what can be large [`expired`]-checks a limit that whoever called it has set for the
//! thread with [`until`], and stops early, returning what it has.
//!
//! This is only for searches whose results may be incomplete without being wrong: stopping early
//! can only make them find less. Whoever sets a limit has to look at [`Limit::expired`] after the
//! work, and treat what came back as possibly incomplete (and not remember it as if it were the
//! whole answer) if it did. Without a limit, nothing changes and the check is a thread-local read.

use std::cell::Cell;

use instant::Instant;

/// How many checks go by between looks at the clock, so that checking is cheap.
const TICKS_PER_LOOK: u32 = 4;

#[derive(Copy, Clone, Debug)]
struct State {
    until: Option<Instant>,
    /// For tests: the number of checks left before the limit is over, whatever the time.
    checks_left: Option<u64>,
    expired: bool,
    ticks: u32,
}

const NONE: State = State {
    until: None,
    checks_left: None,
    expired: false,
    ticks: 0,
};

thread_local! {
    static STATE: Cell<State> = const { Cell::new(NONE) };
}

/// The limit set for this thread, until it is dropped, when the one before it is back.
#[derive(Debug)]
#[must_use = "the limit ends when this is dropped"]
pub struct Limit {
    previous: State,
}

/// Limits the work that checks [`expired`] on this thread to what can be done by `deadline`.
/// Within another limit, whichever is sooner counts.
pub fn until(deadline: Instant) -> Limit {
    set(|state| {
        state.until = Some(state.until.map_or(deadline, |other| other.min(deadline)));
    })
}

/// Limits the work to a number of checks of [`expired`], whatever time they take, so that a test
/// can stop a search at a place that does not depend on how fast the machine is.
pub fn after_checks(checks: u64) -> Limit {
    set(|state| {
        state.checks_left = Some(state.checks_left.map_or(checks, |other| other.min(checks)));
    })
}

fn set(change: impl FnOnce(&mut State)) -> Limit {
    STATE.with(|cell| {
        let previous = cell.get();
        let mut state = previous;
        change(&mut state);
        state.expired = false;
        cell.set(state);
        Limit { previous }
    })
}

impl Limit {
    /// Whether the limit was reached, or any limit within which this one is, while it was set.
    #[must_use]
    #[allow(clippy::unused_self)] // The question is about the limit in force, which this is.
    pub fn expired(&self) -> bool {
        expired()
    }
}

impl Drop for Limit {
    fn drop(&mut self) {
        STATE.with(|cell| cell.set(self.previous));
    }
}

/// Whether the limit set for this thread is reached, in which case the search that asks is to
/// stop and return what it has. Never, if there is no limit. Stays true until the limit is
/// dropped.
#[must_use]
pub fn expired() -> bool {
    STATE.with(|cell| {
        let mut state = cell.get();
        if state.expired {
            return true;
        }
        if state.until.is_none() && state.checks_left.is_none() {
            return false;
        }
        if let Some(left) = state.checks_left {
            if left == 0 {
                state.expired = true;
            } else {
                state.checks_left = Some(left - 1);
            }
        }
        if let Some(until) = state.until {
            state.ticks = state.ticks.wrapping_add(1);
            if state.ticks % TICKS_PER_LOOK == 0 && Instant::now() >= until {
                state.expired = true;
            }
        }
        cell.set(state);
        state.expired
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn without_a_limit_nothing_expires() {
        for _ in 0..1000 {
            assert!(!expired());
        }
    }

    #[test]
    fn a_limit_in_checks_expires_and_stays_expired() {
        let limit = after_checks(3);
        assert!(!expired() && !expired() && !expired());
        assert!(expired() && expired());
        assert!(limit.expired());
        drop(limit);
        assert!(!expired());
    }

    #[test]
    fn a_time_limit_expires_with_the_time() {
        let limit = until(Instant::now() + Duration::from_millis(20));
        assert!(!limit.expired());
        std::thread::sleep(Duration::from_millis(40));
        // The clock is looked at every few checks.
        let mut checks = 0;
        while !expired() {
            checks += 1;
            assert!(checks < 100);
        }
        assert!(limit.expired());
    }

    #[test]
    fn limits_nest_and_the_sooner_counts() {
        let outer = after_checks(100);
        {
            let inner = after_checks(1);
            assert!(!expired());
            assert!(expired() && inner.expired());
        }
        // The outer limit was not used up by the inner one's expiry.
        assert!(!outer.expired());
        assert!(!expired());
    }
}
