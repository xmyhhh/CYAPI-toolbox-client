use std::sync::{Mutex, MutexGuard, PoisonError};

pub const MESSAGE: &str = "操作已取消";

/// Whether a running installation may be stopped.
#[derive(Default)]
struct State {
    /// The user asked to stop the task.
    cancelled: bool,
    /// An installation is running, so only its requests can be stopped.
    installing: bool,
}

impl State {
    fn begin(&mut self) {
        self.cancelled = false;
        self.installing = true;
    }
    fn end(&mut self) {
        self.installing = false;
    }
    fn cancel(&mut self) {
        self.cancelled = true;
    }
    fn is_cancelled(&self) -> bool {
        self.installing && self.cancelled
    }
}

static STATE: Mutex<State> = Mutex::new(State {
    cancelled: false,
    installing: false,
});

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The user asked to stop the running task.
pub fn cancel() {
    state().cancel();
}

/// An installation starts, so an earlier cancellation does not apply to it.
pub fn begin() {
    state().begin();
}

/// The installation is over, so a catalog refresh is never interrupted.
pub fn end() {
    state().end();
}

/// Only a running installation can be stopped.
pub fn is_cancelled() -> bool {
    state().is_cancelled()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancellation_only_stops_a_running_installation() {
        let mut state = State::default();
        state.begin();
        assert!(!state.is_cancelled(), "a new installation is never cancelled");
        state.cancel();
        assert!(state.is_cancelled(), "the running installation stops");
        state.end();
        assert!(
            !state.is_cancelled(),
            "other requests keep working after the installation"
        );
        state.begin();
        assert!(
            !state.is_cancelled(),
            "an earlier cancellation does not apply to the next installation"
        );
        state.end();
    }
}
