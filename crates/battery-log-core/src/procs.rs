//! Which processes actually burned the CPU between two samples.
//!
//! /proc is the only source that needs no privileges. Reading every task's
//! utime+stime and diffing against the previous pass costs about a
//! millisecond and turns "18 W for two hours" into a list of names.

use std::collections::HashMap;
use std::fs;

use crate::model::ProcDelta;

/// USER_HZ. Linux fixes this at 100 on every architecture that matters, and
/// the alternative is linking libc for one sysconf call.
const TICKS_PER_SEC: f64 = 100.0;

#[derive(Default)]
pub struct ProcWatcher {
    prev: HashMap<i32, (u64, String)>,
}

impl ProcWatcher {
    /// The `n` processes that used the most CPU since the previous call.
    /// The first call only primes the counters and returns nothing.
    pub fn top(&mut self, n: usize) -> Vec<ProcDelta> {
        let mut now: HashMap<i32, (u64, String)> = HashMap::new();
        let dir = match fs::read_dir("/proc") {
            Ok(d) => d,
            Err(_) => return Vec::new(),
        };
        for entry in dir.flatten() {
            let name = entry.file_name();
            let pid: i32 = match name.to_string_lossy().parse() {
                Ok(p) => p,
                Err(_) => continue,
            };
            let stat = match fs::read_to_string(entry.path().join("stat")) {
                Ok(s) => s,
                Err(_) => continue, // the process exited between readdir and open
            };
            // comm is parenthesised and may itself contain spaces and ')', so
            // split on the *last* ')' rather than tokenising from the left.
            let (head, rest) = match stat.rsplit_once(") ") {
                Some(x) => x,
                None => continue,
            };
            let comm = head
                .split_once(" (")
                .map(|(_, c)| c)
                .unwrap_or("?")
                .to_string();
            let fields: Vec<&str> = rest.split_whitespace().collect();
            // rest starts at field 3 (state), so utime/stime are indices 11/12.
            if fields.len() < 13 {
                continue;
            }
            let utime: u64 = fields[11].parse().unwrap_or(0);
            let stime: u64 = fields[12].parse().unwrap_or(0);
            now.insert(pid, (utime + stime, comm));
        }

        let mut out: Vec<ProcDelta> = Vec::new();
        if !self.prev.is_empty() {
            let mut by_comm: HashMap<String, u64> = HashMap::new();
            for (pid, (ticks, comm)) in &now {
                let before = self.prev.get(pid).map(|(t, _)| *t).unwrap_or(0);
                // A pid that was reused reads as a huge delta; ignore anything
                // that claims more CPU than the counter could have advanced.
                let delta = ticks.saturating_sub(before);
                if delta > 0 {
                    *by_comm.entry(comm.clone()).or_insert(0) += delta;
                }
            }
            let mut ranked: Vec<(String, u64)> = by_comm.into_iter().collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1));
            out = ranked
                .into_iter()
                .take(n)
                .map(|(comm, ticks)| ProcDelta {
                    comm,
                    cpu_s: ticks as f64 / TICKS_PER_SEC,
                })
                .collect();
        }
        self.prev = now;
        out
    }
}
