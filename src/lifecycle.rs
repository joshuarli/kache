use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Default)]
struct State {
    draining: bool,
    active: usize,
    drain_started: Option<Instant>,
}

pub struct Lifecycle {
    state: Mutex<State>,
    changed: Condvar,
}

impl Default for Lifecycle {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LifecycleSnapshot {
    pub active: usize,
    pub draining: bool,
    pub draining_for: Option<Duration>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DrainOutcome {
    Complete,
    TimedOut { active: usize },
}

impl Lifecycle {
    pub fn snapshot(&self) -> LifecycleSnapshot {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        LifecycleSnapshot {
            active: state.active,
            draining: state.draining,
            draining_for: state.drain_started.map(|started| started.elapsed()),
        }
    }

    pub fn begin(self: &Arc<Self>) -> Option<RequestGuard> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.draining {
            return None;
        }
        state.active += 1;
        Some(RequestGuard(Arc::clone(self)))
    }

    pub fn start_drain(&self) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.draining {
            return false;
        }
        state.draining = true;
        state.drain_started = Some(Instant::now());
        self.changed.notify_all();
        true
    }

    pub fn accepting_calls(&self) -> bool {
        !self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .draining
    }

    pub fn drain_until(&self, deadline: Instant) -> DrainOutcome {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if state.active == 0 {
                return DrainOutcome::Complete;
            }
            let now = Instant::now();
            if now >= deadline {
                return DrainOutcome::TimedOut {
                    active: state.active,
                };
            }
            let remaining = deadline - now;
            let (guard, timed_out) = self
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(|e| e.into_inner());
            state = guard;
            if state.active == 0 {
                return DrainOutcome::Complete;
            }
            if timed_out.timed_out() && Instant::now() >= deadline {
                return DrainOutcome::TimedOut {
                    active: state.active,
                };
            }
        }
    }
}

#[must_use = "keep the guard alive until the operation and its reply finish"]
pub struct RequestGuard(Arc<Lifecycle>);

impl Drop for RequestGuard {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        state.active = state.active.saturating_sub(1);
        if state.draining && state.active == 0 {
            self.0.changed.notify_all();
        }
    }
}

#[derive(Debug)]
pub(crate) struct Semaphore {
    state: Mutex<usize>,
}

#[derive(Debug)]
pub(crate) struct SemaphoreGuard {
    sem: Arc<Semaphore>,
}

impl Semaphore {
    pub(crate) fn new(permits: usize) -> Self {
        Self {
            state: Mutex::new(permits),
        }
    }

    pub(crate) fn try_acquire(self: &Arc<Self>) -> Option<SemaphoreGuard> {
        let mut permits = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if *permits == 0 {
            return None;
        }
        *permits -= 1;
        Some(SemaphoreGuard {
            sem: Arc::clone(self),
        })
    }
}

impl Drop for SemaphoreGuard {
    fn drop(&mut self) {
        if let Ok(mut permits) = self.sem.state.lock() {
            *permits += 1;
        }
    }
}
