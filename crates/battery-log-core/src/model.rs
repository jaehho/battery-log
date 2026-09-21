//! What one sample is, and what the dashboard is handed.

use serde::{Deserialize, Serialize};

/// Why a sample was taken. The boundary kinds are what make standby drain
/// exact: `PreSleep` is written while logind holds the machine awake for us,
/// `Resume` immediately after it comes back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Tick,
    PreSleep,
    Resume,
    Start,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Tick => "tick",
            Kind::PreSleep => "presleep",
            Kind::Resume => "resume",
            Kind::Start => "start",
        }
    }
}

/// One row of the log: the pack, plus the context that explains it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Sample {
    pub ts: i64,
    pub kind: String,
    pub boot_id: String,
    pub status: String,
    pub capacity: Option<i64>,
    pub energy_uwh: Option<i64>,
    pub full_uwh: Option<i64>,
    pub design_uwh: Option<i64>,
    pub power_uw: Option<i64>,
    pub voltage_uv: Option<i64>,
    pub cycles: Option<i64>,
    pub ac: i64,
    pub profile: String,
    pub model: String,

    // --- context: what the machine was doing ---
    /// Backlight, 0-100. None when there is no internal panel.
    pub backlight: Option<f64>,
    /// Displays that are connected, enabled and not DPMS-off.
    pub displays_on: i64,
    /// ...of which are not the internal panel.
    pub external_displays: i64,
    /// CPU time that was not idle, as a percentage of one interval.
    pub cpu_busy: Option<f64>,
    /// Share of CPU time spent in a deep C-state (C2 and below).
    pub deep_idle: Option<f64>,
    pub load1: Option<f64>,
    pub temp_c: Option<f64>,
    pub wifi: Option<i64>,
    pub bluetooth: Option<i64>,

    // --- time and suspend accounting ---
    /// Wall seconds since the previous sample.
    pub elapsed_s: Option<i64>,
    /// Of those, how many the machine spent suspended. Derived from the gap
    /// between CLOCK_BOOTTIME (counts suspend) and CLOCK_MONOTONIC (does not),
    /// so it is exact even when nothing was there to observe the sleep.
    pub slept_s: Option<i64>,
    /// Kernel counter: microseconds the platform has ever spent in real
    /// hardware sleep. Cumulative, so only its delta means anything -- and on
    /// this firmware the delta is not always trustworthy, hence the next field.
    pub hw_sleep_us: Option<i64>,
    /// Microseconds of hardware sleep in the *most recent* suspend, read at
    /// resume. Preferred over differencing `hw_sleep_us`: the running total
    /// has been observed to jump by exactly 2^32 us across a 71-minute
    /// suspend, which would read as 101% residency.
    pub last_hw_sleep_us: Option<i64>,
    pub suspend_ok: Option<i64>,
    pub suspend_fail: Option<i64>,
    pub source: String,
}

/// CPU burned by one process between two samples.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcDelta {
    pub comm: String,
    pub cpu_s: f64,
}
