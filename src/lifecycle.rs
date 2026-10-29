use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Default)]
struct State {
    draining: bool,
    active: usize,
    drain_started: Option<Instant>,
}

pub struct Lifecycle {
    state: Mutex<State>,
    changed: event_listener::Event,
}

impl Default for Lifecycle {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::default()),
            changed: event_listener::Event::new(),
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
        self.changed.notify(usize::MAX);
        true
    }

    pub fn accepting_calls(&self) -> bool {
        !self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .draining
    }

    pub async fn draining(&self) {
        loop {
            let listener = self.changed.listen();
            if self
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .draining
            {
                return;
            }
            listener.await;
        }
    }

    pub async fn drain(&self) {
        self.start_drain();
        loop {
            let listener = self.changed.listen();
            let active = self.state.lock().unwrap_or_else(|e| e.into_inner()).active;
            if active == 0 {
                return;
            }
            listener.await;
        }
    }

    pub async fn drain_until(&self, deadline: Instant) -> DrainOutcome {
        use futures::FutureExt as _;
        let mut drain = Box::pin(self.drain().fuse());
        let mut timeout = Box::pin(async_io::Timer::at(deadline).fuse());
        futures::select! {
            _ = drain => DrainOutcome::Complete,
            _ = timeout => {
                let active = self.state.lock().unwrap_or_else(|e| e.into_inner()).active;
                if active == 0 {
                    DrainOutcome::Complete
                } else {
                    DrainOutcome::TimedOut { active }
                }
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
            self.0.changed.notify(usize::MAX);
        }
    }
}
