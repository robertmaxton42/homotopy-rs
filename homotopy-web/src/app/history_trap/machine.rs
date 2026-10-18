//! The history trap as a pure state machine: events in, outputs out, no browser.
//!
//! See the parent module for the scheme. This file decides; the parent does what it says.

use homotopy_core::Direction;

/// A mouse press and a navigation within this many milliseconds of each other, in the same
/// direction, are one press seen twice (see [`Machine::mouse`]).
pub const DEDUPE_MS: f64 = 300.0;

/// How long a move of ours may take to land before we look where we are.
pub const LANDING_MS: u32 = 1000;

/// The marker kept in the `state` of the sentinel entries of the history.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The entry behind the current one: landing on it is a request to undo.
    Back,
    /// The entry that the page sits on.
    Centre,
    /// The entry ahead of the current one: landing on it is a request to redo.
    Forward,
}

/// What to do to the `state` of the entry replaced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Marking {
    /// Leave it as it is.
    Keep,
    Set(Kind),
    /// Back to no state at all.
    Clear,
}

/// Something that happened.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The setting was changed (and this is its value at the start).
    Setting(bool),
    /// A user gesture (pointer down, key down): `state` is the marker of the current entry.
    Gesture { state: Option<Kind> },
    /// The history moved (`popstate`); `state` is the marker of the entry now current, `None` if
    /// it has none. This includes the moves that we ask for ourselves.
    Popstate { state: Option<Kind>, now: f64 },
    /// A back or forward button of a mouse was released and the page saw the DOM event.
    Mouse { direction: Direction, now: f64 },
    /// The page's address was changed by the app, to this URL.
    UrlChanged(String),
    /// [`Output::Wait`] ran out; `state` is the marker of the current entry.
    TimedOut { epoch: u32, state: Option<Kind> },
}

/// Something to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    /// Undo or redo (through the usual dispatch, which refuses it while a job runs).
    Move(Direction),
    /// `history.replaceState` on the current entry, with this marking and this URL (`None`: the
    /// URL it has).
    Replace {
        marking: Marking,
        url: Option<String>,
    },
    /// `history.pushState` with this marker and the current URL.
    Push(Kind),
    /// `history.go(n)`.
    Go(i32),
    /// Send [`Event::TimedOut`] with this epoch after [`LANDING_MS`], if nothing lands first.
    Wait(u32),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Phase {
    /// The setting is off. Nothing is in the history that we need.
    Off,
    /// The setting is on, but nothing has been put in the history yet: waiting for a gesture.
    Armed,
    /// Sentinels have been pushed and the move back to the centre is on its way.
    Installing,
    /// The page sits on the centre with a sentinel on either side.
    Idle,
    /// We landed on a sentinel and have asked to return to the centre.
    Restoring,
    /// The setting went off: we asked to go back to the first sentinel and so out of the trap.
    Leaving,
}

#[derive(Copy, Clone, Debug, PartialEq)]
enum Source {
    Mouse,
    Navigation,
}

#[derive(Debug)]
pub struct Machine {
    phase: Phase,
    wanted: bool,
    gestured: bool,
    /// The last undo or redo and where it came from, to take a press that is seen twice once.
    last: Option<(Source, Direction, f64)>,
    /// The address of the project, if the app has set it: what the sentinels are given when we
    /// land on them.
    url: Option<String>,
    /// Counts the times that we start waiting for a move, so that a stale timeout does nothing.
    epoch: u32,
}

impl Machine {
    #[must_use]
    pub const fn new(setting: bool) -> Self {
        Self {
            phase: if setting { Phase::Armed } else { Phase::Off },
            wanted: setting,
            gestured: false,
            last: None,
            url: None,
            epoch: 0,
        }
    }

    /// Whether there are sentinels in the history (or they are on their way), so that the page
    /// must not push entries of its own.
    #[must_use]
    pub const fn in_place(&self) -> bool {
        matches!(
            self.phase,
            Phase::Installing | Phase::Idle | Phase::Restoring
        )
    }

    /// Whether the page sits on the centre with nothing going on.
    #[cfg(test)]
    pub const fn is_idle(&self) -> bool {
        matches!(self.phase, Phase::Idle)
    }

    pub fn handle(&mut self, event: Event) -> Vec<Output> {
        match event {
            Event::Setting(on) => self.setting(on),
            Event::Gesture { state } => self.gesture(state),
            Event::Popstate { state, now } => self.popstate(state, now),
            Event::Mouse { direction, now } => self.mouse(direction, now),
            Event::UrlChanged(url) => self.url_changed(url),
            Event::TimedOut { epoch, state } => self.timed_out(epoch, state),
        }
    }

    fn setting(&mut self, on: bool) -> Vec<Output> {
        if on == self.wanted {
            return vec![];
        }
        self.wanted = on;
        match (on, self.phase) {
            (true, Phase::Off) if self.gestured => self.install(),
            (true, Phase::Off) => {
                self.phase = Phase::Armed;
                vec![]
            }
            (false, Phase::Armed) => {
                self.phase = Phase::Off;
                vec![]
            }
            (false, Phase::Idle) => self.leave(),
            // Where a move is on its way, the landing sorts it out.
            _ => vec![],
        }
    }

    fn gesture(&mut self, state: Option<Kind>) -> Vec<Output> {
        self.gestured = true;
        if self.phase != Phase::Armed {
            return vec![];
        }
        if state == Some(Kind::Centre) {
            // A reload leaves the entries and their markers where they were.
            self.phase = Phase::Idle;
            vec![]
        } else {
            self.install()
        }
    }

    /// Makes `[back, centre, forward]` out of the current entry and goes to the centre.
    fn install(&mut self) -> Vec<Output> {
        self.phase = Phase::Installing;
        self.epoch += 1;
        vec![
            Output::Replace {
                marking: Marking::Set(Kind::Back),
                url: None,
            },
            Output::Push(Kind::Centre),
            Output::Push(Kind::Forward),
            Output::Go(-1),
            Output::Wait(self.epoch),
        ]
    }

    /// Goes back from the centre onto the first sentinel, which is a plain entry once it has
    /// lost its marker: one more back leaves the page, as it did before.
    fn leave(&mut self) -> Vec<Output> {
        self.phase = Phase::Leaving;
        self.epoch += 1;
        vec![Output::Go(-1), Output::Wait(self.epoch)]
    }

    /// We are on the first sentinel and out of the trap, or about to go in again.
    fn left(&mut self) -> Vec<Output> {
        if self.wanted {
            return self.install();
        }
        self.phase = Phase::Off;
        vec![Output::Replace {
            marking: Marking::Clear,
            url: self.url.clone(),
        }]
    }

    fn popstate(&mut self, state: Option<Kind>, now: f64) -> Vec<Output> {
        match (self.phase, state) {
            (Phase::Off, _) | (Phase::Armed | Phase::Restoring, None) => vec![],
            (Phase::Leaving, _) => self.left(),
            // Landed on the centre, from wherever: we are home.
            (_, Some(Kind::Centre)) => self.home(),
            // On a sentinel while one of our own moves is on its way: it is not a new request.
            (Phase::Restoring, Some(kind)) => vec![Output::Go(Self::towards_centre(kind))],
            (Phase::Armed | Phase::Installing | Phase::Idle, Some(kind)) => {
                self.sentinel(kind, now)
            }
            // Something that is not ours (an entry from before, or one made by the page): start
            // again from here.
            (Phase::Installing | Phase::Idle, None) => self.install(),
        }
    }

    fn home(&mut self) -> Vec<Output> {
        match self.phase {
            Phase::Installing | Phase::Restoring => {
                self.phase = Phase::Idle;
                if self.wanted {
                    vec![]
                } else {
                    self.leave()
                }
            }
            Phase::Armed => {
                self.phase = Phase::Idle;
                vec![]
            }
            _ => vec![],
        }
    }

    const fn towards_centre(kind: Kind) -> i32 {
        match kind {
            Kind::Back => 1,
            Kind::Forward => -1,
            Kind::Centre => 0,
        }
    }

    /// Landed on the back or forward sentinel: undo or redo, then go home.
    fn sentinel(&mut self, kind: Kind, now: f64) -> Vec<Output> {
        let direction = match kind {
            Kind::Back => Direction::Backward,
            Kind::Forward => Direction::Forward,
            Kind::Centre => return vec![],
        };
        let mut out = vec![];
        if self.fresh(Source::Navigation, direction, now) {
            out.push(Output::Move(direction));
        }
        self.phase = Phase::Restoring;
        self.epoch += 1;
        out.push(Output::Replace {
            marking: Marking::Set(kind),
            url: self.url.clone(),
        });
        out.push(Output::Go(Self::towards_centre(kind)));
        out.push(Output::Wait(self.epoch));
        out
    }

    /// A mouse's button as the DOM reports it. A mouse that reports it and also navigates must
    /// not undo twice, so a press that the other source saw within [`DEDUPE_MS`] is dropped.
    /// Presses from the same source are never dropped: two quick clicks are two undos.
    fn mouse(&mut self, direction: Direction, now: f64) -> Vec<Output> {
        if self.wanted && self.fresh(Source::Mouse, direction, now) {
            vec![Output::Move(direction)]
        } else {
            vec![]
        }
    }

    /// Notes a move and says whether to act on it.
    fn fresh(&mut self, source: Source, direction: Direction, now: f64) -> bool {
        let duplicate = matches!(
            self.last,
            Some((last, d, then))
                if last != source && d == direction && (0.0..DEDUPE_MS).contains(&(now - then))
        );
        if duplicate {
            // Forget it, so that the one press is not matched with a third.
            self.last = None;
        } else {
            self.last = Some((source, direction, now));
        }
        !duplicate
    }

    fn url_changed(&mut self, url: String) -> Vec<Output> {
        self.url = Some(url.clone());
        let marking = if self.phase == Phase::Idle {
            Marking::Set(Kind::Centre)
        } else {
            Marking::Keep
        };
        vec![Output::Replace {
            marking,
            url: Some(url),
        }]
    }

    fn timed_out(&mut self, epoch: u32, state: Option<Kind>) -> Vec<Output> {
        if epoch != self.epoch {
            return vec![];
        }
        match self.phase {
            Phase::Installing | Phase::Restoring if state == Some(Kind::Centre) => {
                self.phase = Phase::Idle;
                if self.wanted {
                    vec![]
                } else {
                    self.leave()
                }
            }
            // The move never landed, or landed somewhere we do not know: start again here.
            Phase::Installing | Phase::Restoring if self.wanted => self.install(),
            Phase::Installing | Phase::Restoring | Phase::Leaving => self.left(),
            _ => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use Direction::{Backward, Forward};

    use super::*;

    fn pop(m: &mut Machine, state: Option<Kind>, now: f64) -> Vec<Output> {
        m.handle(Event::Popstate { state, now })
    }

    fn installed() -> Machine {
        let mut m = Machine::new(true);
        m.handle(Event::Gesture { state: None });
        pop(&mut m, Some(Kind::Centre), 0.0);
        assert!(m.is_idle());
        m
    }

    #[test]
    fn nothing_is_touched_before_a_gesture() {
        let mut m = Machine::new(true);
        assert!(!m.in_place());
        assert_eq!(pop(&mut m, None, 0.0), vec![]);
        assert_eq!(
            m.handle(Event::Gesture { state: None }),
            vec![
                Output::Replace {
                    marking: Marking::Set(Kind::Back),
                    url: None
                },
                Output::Push(Kind::Centre),
                Output::Push(Kind::Forward),
                Output::Go(-1),
                Output::Wait(1),
            ]
        );
        assert!(m.in_place());
        // Later gestures do nothing.
        assert_eq!(m.handle(Event::Gesture { state: None }), vec![]);
    }

    #[test]
    fn a_setting_that_starts_off_installs_nothing() {
        let mut m = Machine::new(false);
        assert_eq!(m.handle(Event::Gesture { state: None }), vec![]);
        assert!(!m.in_place());
        assert_eq!(pop(&mut m, Some(Kind::Back), 0.0), vec![]);
        // And turning it on after a gesture installs at once.
        assert_eq!(m.handle(Event::Setting(true)).len(), 5);
        assert!(m.in_place());
    }

    #[test]
    fn turning_it_on_before_any_gesture_waits_for_one() {
        let mut m = Machine::new(false);
        assert_eq!(m.handle(Event::Setting(true)), vec![]);
        assert_eq!(m.handle(Event::Gesture { state: None }).len(), 5);
    }

    #[test]
    fn a_page_that_comes_back_on_its_centre_keeps_the_entries_it_has() {
        let mut m = Machine::new(true);
        assert_eq!(
            m.handle(Event::Gesture {
                state: Some(Kind::Centre)
            }),
            vec![]
        );
        assert!(m.is_idle());
    }

    #[test]
    fn landing_on_the_back_sentinel_undoes_once_and_goes_home() {
        let mut m = installed();
        let out = pop(&mut m, Some(Kind::Back), 1000.0);
        assert_eq!(
            out[..3],
            [
                Output::Move(Backward),
                Output::Replace {
                    marking: Marking::Set(Kind::Back),
                    url: None
                },
                Output::Go(1)
            ]
        );
        // Our own move home is not a request.
        assert_eq!(pop(&mut m, Some(Kind::Centre), 1010.0), vec![]);
        assert!(m.is_idle());
    }

    #[test]
    fn landing_on_the_forward_sentinel_redoes_and_goes_back() {
        let mut m = installed();
        let out = pop(&mut m, Some(Kind::Forward), 1000.0);
        assert_eq!(out[0], Output::Move(Forward));
        assert_eq!(out[2], Output::Go(-1));
        pop(&mut m, Some(Kind::Centre), 1010.0);
        assert!(m.is_idle());
    }

    #[test]
    fn nothing_is_pushed_while_handling_a_popstate() {
        let mut m = installed();
        for state in [Some(Kind::Back), Some(Kind::Forward)] {
            let out = pop(&mut m, state, 5000.0);
            assert!(!out.iter().any(|o| matches!(o, Output::Push(_))));
            pop(&mut m, Some(Kind::Centre), 5001.0);
        }
    }

    #[test]
    fn repeated_presses_undo_repeatedly() {
        let mut m = installed();
        for i in 0..4 {
            let t = f64::from(i) * 50.0;
            assert_eq!(pop(&mut m, Some(Kind::Back), t)[0], Output::Move(Backward));
            pop(&mut m, Some(Kind::Centre), t + 10.0);
        }
    }

    #[test]
    fn a_second_arrival_while_going_home_is_not_another_undo() {
        let mut m = installed();
        pop(&mut m, Some(Kind::Back), 0.0);
        assert_eq!(pop(&mut m, Some(Kind::Back), 5.0), vec![Output::Go(1)]);
        pop(&mut m, Some(Kind::Centre), 10.0);
        assert!(m.is_idle());
    }

    #[test]
    fn a_mouse_that_sends_both_undoes_once_in_either_order() {
        // DOM first.
        let mut m = installed();
        assert_eq!(
            m.handle(Event::Mouse {
                direction: Backward,
                now: 1000.0
            }),
            vec![Output::Move(Backward)]
        );
        let out = pop(&mut m, Some(Kind::Back), 1050.0);
        assert!(!out.contains(&Output::Move(Backward)));
        assert!(out.contains(&Output::Go(1)));
        pop(&mut m, Some(Kind::Centre), 1060.0);
        // Navigation first.
        let out = pop(&mut m, Some(Kind::Back), 2000.0);
        assert!(out.contains(&Output::Move(Backward)));
        pop(&mut m, Some(Kind::Centre), 2010.0);
        assert_eq!(
            m.handle(Event::Mouse {
                direction: Backward,
                now: 2040.0
            }),
            vec![]
        );
        // The pair is forgotten: the next DOM press is a new one.
        assert_eq!(
            m.handle(Event::Mouse {
                direction: Backward,
                now: 2100.0
            }),
            vec![Output::Move(Backward)]
        );
    }

    #[test]
    fn the_other_direction_or_a_later_press_is_not_a_duplicate() {
        let mut m = installed();
        m.handle(Event::Mouse {
            direction: Backward,
            now: 1000.0,
        });
        let out = pop(&mut m, Some(Kind::Forward), 1050.0);
        assert!(out.contains(&Output::Move(Forward)));
        pop(&mut m, Some(Kind::Centre), 1060.0);
        m.handle(Event::Mouse {
            direction: Backward,
            now: 5000.0,
        });
        let out = pop(&mut m, Some(Kind::Back), 5000.0 + DEDUPE_MS + 1.0);
        assert!(out.contains(&Output::Move(Backward)));
    }

    #[test]
    fn two_quick_presses_of_one_source_are_two_undos() {
        let mut m = installed();
        for t in [0.0, 100.0] {
            assert_eq!(
                m.handle(Event::Mouse {
                    direction: Backward,
                    now: t
                }),
                vec![Output::Move(Backward)]
            );
        }
        let mut m = installed();
        assert!(pop(&mut m, Some(Kind::Back), 0.0).contains(&Output::Move(Backward)));
        pop(&mut m, Some(Kind::Centre), 10.0);
        assert!(pop(&mut m, Some(Kind::Back), 100.0).contains(&Output::Move(Backward)));
    }

    #[test]
    fn the_mouse_does_nothing_while_the_setting_is_off() {
        let mut m = Machine::new(false);
        assert_eq!(
            m.handle(Event::Mouse {
                direction: Backward,
                now: 0.0
            }),
            vec![]
        );
    }

    #[test]
    fn the_address_changing_keeps_the_marker_on_the_centre() {
        let mut m = installed();
        assert_eq!(
            m.handle(Event::UrlChanged("https://x/p/a".into())),
            vec![Output::Replace {
                marking: Marking::Set(Kind::Centre),
                url: Some("https://x/p/a".into())
            }]
        );
        // The sentinels get the address when we land on them.
        let out = pop(&mut m, Some(Kind::Back), 0.0);
        assert!(out.contains(&Output::Replace {
            marking: Marking::Set(Kind::Back),
            url: Some("https://x/p/a".into())
        }));
    }

    #[test]
    fn the_address_changing_before_install_leaves_the_state_alone() {
        let mut m = Machine::new(true);
        assert_eq!(
            m.handle(Event::UrlChanged("u".into())),
            vec![Output::Replace {
                marking: Marking::Keep,
                url: Some("u".into())
            }]
        );
    }

    #[test]
    fn turning_it_off_steps_out_of_the_trap_once() {
        let mut m = installed();
        m.handle(Event::UrlChanged("u".into()));
        assert_eq!(m.handle(Event::Setting(false))[0], Output::Go(-1));
        // Landing on the first sentinel is not an undo; it loses its marker and gets the address.
        assert_eq!(
            pop(&mut m, Some(Kind::Back), 0.0),
            vec![Output::Replace {
                marking: Marking::Clear,
                url: Some("u".into())
            }]
        );
        assert!(!m.in_place());
        // From now on nothing is handled.
        assert_eq!(pop(&mut m, Some(Kind::Back), 10.0), vec![]);
        assert_eq!(
            m.handle(Event::Mouse {
                direction: Backward,
                now: 0.0
            }),
            vec![]
        );
    }

    #[test]
    fn turning_it_off_and_on_again_installs_again() {
        let mut m = installed();
        m.handle(Event::Setting(false));
        // Back on before the move lands: install where it lands.
        m.handle(Event::Setting(true));
        let out = pop(&mut m, Some(Kind::Back), 0.0);
        assert_eq!(out.len(), 5);
        assert!(matches!(out[1], Output::Push(Kind::Centre)));
    }

    #[test]
    fn turning_it_off_in_the_middle_of_a_move_leaves_after_it_lands() {
        let mut m = installed();
        pop(&mut m, Some(Kind::Back), 0.0);
        assert_eq!(m.handle(Event::Setting(false)), vec![]);
        let out = pop(&mut m, Some(Kind::Centre), 5.0);
        assert_eq!(out[0], Output::Go(-1));
        assert_eq!(
            pop(&mut m, Some(Kind::Back), 6.0)[0],
            Output::Replace {
                marking: Marking::Clear,
                url: None
            }
        );
    }

    #[test]
    fn an_entry_that_is_not_ours_makes_the_trap_start_again() {
        let mut m = installed();
        let out = pop(&mut m, None, 0.0);
        assert_eq!(out.len(), 5);
        assert!(matches!(
            out[0],
            Output::Replace {
                marking: Marking::Set(Kind::Back),
                ..
            }
        ));
    }

    #[test]
    fn a_move_that_never_lands_is_looked_into() {
        let mut m = installed();
        let wait = pop(&mut m, Some(Kind::Back), 0.0)
            .into_iter()
            .find_map(|o| match o {
                Output::Wait(e) => Some(e),
                _ => None,
            })
            .unwrap();
        // A stale timeout does nothing.
        assert_eq!(
            m.handle(Event::TimedOut {
                epoch: wait - 1,
                state: None
            }),
            vec![]
        );
        // Still on the sentinel: start again from here.
        assert_eq!(
            m.handle(Event::TimedOut {
                epoch: wait,
                state: Some(Kind::Back)
            })
            .len(),
            5
        );
        // On the centre after all: home.
        let mut m = installed();
        let wait = pop(&mut m, Some(Kind::Forward), 0.0)
            .into_iter()
            .find_map(|o| match o {
                Output::Wait(e) => Some(e),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            m.handle(Event::TimedOut {
                epoch: wait,
                state: Some(Kind::Centre)
            }),
            vec![]
        );
        assert!(m.is_idle());
    }
}
