//! A per-file watchdog for the corpus runs: a hang becomes a failure with a
//! name instead of a run that never ends.
//!
//! Each worker publishes the file it starts into its slot and clears it when
//! done. A thread wakes every 5 s, prints `still running: <path> (N s)` for a
//! file past 10 s and, past the limit (`<env var>` seconds, default 60), prints
//! every in-flight file as `<path>: timeout after N s`, the summary so far and
//! exits the process with status 1.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const WAKE: Duration = Duration::from_secs(5);
const SLOW: Duration = Duration::from_secs(10);

#[derive(Default)]
struct State {
    slots: Vec<Option<(String, Instant)>>,
    summary: String,
}

/// Handle to the watchdog thread; cloning shares the slots.
#[derive(Clone)]
pub struct Watchdog {
    state: Arc<Mutex<State>>,
}

impl Watchdog {
    /// Start the thread with room for `workers` slots; the limit is read from
    /// `env_var` (seconds).
    pub fn start(env_var: &str, workers: usize) -> Self {
        let limit = std::env::var(env_var)
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map_or(Duration::from_secs(60), Duration::from_secs);
        let state = Arc::new(Mutex::new(State {
            slots: vec![None; workers.max(1)],
            summary: String::new(),
        }));
        let watched = Arc::clone(&state);
        std::thread::spawn(move || loop {
            std::thread::sleep(WAKE);
            let state = watched.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            let late: Vec<(&str, Duration)> = state
                .slots
                .iter()
                .flatten()
                .map(|(path, started)| (path.as_str(), now - *started))
                .filter(|(_, took)| *took >= SLOW)
                .collect();
            if late.iter().any(|(_, took)| *took >= limit) {
                for (path, took) in &late {
                    eprintln!("{path}: timeout after {} s", took.as_secs());
                }
                eprintln!("{}", state.summary);
                std::process::exit(1);
            }
            for (path, took) in late {
                eprintln!("still running: {path} ({} s)", took.as_secs());
            }
        });
        Watchdog { state }
    }

    /// Worker `worker` starts on `path`.
    pub fn begin(&self, worker: usize, path: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.slots[worker] = Some((path.to_string(), Instant::now()));
    }

    /// Worker `worker` is done with its file.
    pub fn end(&self, worker: usize) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.slots[worker] = None;
    }

    /// What a timeout prints after the files: the counts so far.
    pub fn summary(&self, summary: String) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.summary = summary;
    }
}
