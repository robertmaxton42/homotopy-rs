//! Computations that can take long enough to need a progress indicator and a way out.
//!
//! A [`Job`] is done a step at a time, so whoever drives it can do something else in between
//! (such as letting a web page respond to input, show how far along the job is, or drop it).
//! Jobs are transactional: they work on their own copy of whatever they compute from, and
//! produce an output for the driver to commit in one go. Dropping a job before it is done
//! therefore changes nothing, and a driver may run one anywhere it can: on the thread that
//! asked for it a slice at a time, or on another thread, as long as the job's input and output
//! make it there.

/// How far along a [`Job`] is.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
}

pub trait Job {
    type Output;

    /// What the job is doing now, as a gerund phrase such as "Merging exchanges".
    fn label(&self) -> &'static str;

    /// How far along the job is, if that is known.
    fn progress(&self) -> Option<Progress>;

    /// Does a bit of the work. Returns the output once the job is done, and must not be called
    /// again after that.
    ///
    /// The steps are the unit of responsiveness: the driver can only stop, show progress or
    /// handle other input between them, so a step should be as small as the job can make it.
    fn step(&mut self) -> Option<Self::Output>;

    /// The most time that one step may take if the job keeps its limits, if it has any, for
    /// whoever runs the job somewhere that it can be ended from outside (a worker) to notice that
    /// a step is taking much longer than that and to end it (see [`checkpoint`](Self::checkpoint)).
    /// A job that stops by itself within the limit never meets it.
    fn step_limit(&self) -> Option<std::time::Duration> {
        None
    }

    /// What the job would come to if it were stopped now, if it has something new to say about
    /// that since the last time it was asked. Asked before each step by whoever runs the job
    /// somewhere it can be ended from outside (a worker), so that if that happens the work is
    /// not all lost: it is only for jobs whose steps can take long and whose output can be
    /// partial.
    fn checkpoint(&mut self) -> Option<Self::Output> {
        None
    }
}

/// Runs a job to the end without interruption.
pub fn run_to_end<J: Job>(mut job: J) -> J::Output {
    loop {
        if let Some(output) = job.step() {
            return output;
        }
    }
}
