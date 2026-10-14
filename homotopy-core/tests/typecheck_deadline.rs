//! A typecheck that runs out of the time set for the thread says so (`TypeError::TimedOut`) and
//! never says that the diagram is well typed: a diagram that checks without a limit is either
//! checked or timed out with one, and is never taken for another error.

use homotopy_common::deadline;
use homotopy_core::{
    examples,
    typecheck::{typecheck, Mode, TypeError},
    Diagram,
};

#[test]
fn a_typecheck_out_of_time_is_timed_out_and_not_ill_typed() {
    let mut timed_out = 0;
    for (sig, diagram) in [
        examples::associator(),
        examples::crossing(),
        examples::algebraic_snake(),
        examples::lips(),
    ] {
        let diagram: Diagram = diagram.into();
        assert!(typecheck(&diagram, &sig, Mode::default(), true).is_ok());
        for checks in [0, 1, 3, 10, 100, 100_000] {
            let result = {
                let _limit = deadline::after_checks(checks);
                typecheck(&diagram, &sig, Mode::default(), true)
            };
            match result {
                Ok(()) => {}
                Err(TypeError::TimedOut) => timed_out += 1,
                Err(error) => panic!("{error} after {checks} checks"),
            }
        }
        // And the limit is gone with the guard.
        assert!(typecheck(&diagram, &sig, Mode::default(), true).is_ok());
    }
    assert!(timed_out > 0);
}
